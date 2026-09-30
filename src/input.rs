use crate::error::Error;
use clap_stdin::{FileOrStdin, StdinError};
use graphql_parser::{query, schema, Pos};
use std::{
    collections::HashMap,
    fmt, fs, io,
    io::{IsTerminal, Read},
    path::{Path, PathBuf},
};

/// Which kind of document an input holds, as error messages name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Schema,
    Query,
}

impl Kind {
    /// The short flag each command reads this kind of document from.
    pub fn flag(self) -> &'static str {
        match self {
            Kind::Schema => "-s",
            Kind::Query => "-q",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Schema => "schema",
            Kind::Query => "query",
        })
    }
}

/// Where an input was read from, printed as `'path'` or `(stdin)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    File(PathBuf),
    Stdin,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::File(path) => write!(f, "'{}'", path.display()),
            Origin::Stdin => f.write_str("(stdin)"),
        }
    }
}

/// A document's text along with where it came from, so a failure to read or
/// parse it is reported against the file or stream the user has to fix.
#[derive(Debug)]
pub struct Input {
    origin: Origin,
    text: String,
}

impl Input {
    /// Reads a document from a file path.
    pub fn from_path(path: &Path, kind: Kind) -> Result<Self, Error> {
        let origin = Origin::File(path.to_path_buf());
        match fs::read_to_string(path) {
            Ok(text) => Ok(Input { origin, text }),
            Err(source) => Err(Error::Read {
                kind,
                origin,
                source,
            }),
        }
    }

    /// Reads a document from a file path or stdin.
    ///
    /// Reads through clap-stdin rather than `FileOrStdin::contents`, whose
    /// errors label a missing file as a stdin failure and drop its path.
    /// Going through its reader keeps its guard against reading stdin twice.
    ///
    /// clap-stdin reads stdin even when it is a terminal, which would wait on
    /// the keyboard forever when the flag was left off by mistake, so that
    /// fails as `Error::NoInput` instead. An idle pipe cannot be told apart
    /// from a slow one, so it is read as usual.
    pub fn read(input: FileOrStdin, kind: Kind) -> Result<Self, Error> {
        let origin = if input.is_stdin() {
            if io::stdin().is_terminal() {
                return Err(Error::NoInput { kind });
            }
            Origin::Stdin
        } else {
            Origin::File(PathBuf::from(input.filename()))
        };

        let mut text = String::new();
        let read = input
            .into_reader()
            .map_err(|e| match e {
                StdinError::StdIn(e) => e,
                other => io::Error::other(other.to_string()),
            })
            .and_then(|mut reader| reader.read_to_string(&mut text));

        match read {
            Ok(_) => Ok(Input { origin, text }),
            Err(source) => Err(Error::Read {
                kind,
                origin,
                source,
            }),
        }
    }

    /// A document given directly, as tests do, reported as if from stdin.
    #[cfg(test)]
    pub fn inline(text: &str) -> Self {
        Input {
            origin: Origin::Stdin,
            text: text.to_string(),
        }
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the document is blank: nothing but what GraphQL ignores between
    /// tokens (spaces, tabs, line breaks, commas, comments, a byte order mark),
    /// so it holds no definitions at all.
    ///
    /// graphql-parser rejects a blank document, since the grammar asks for at
    /// least one definition. But it is what `query focus`/`strip` emit when
    /// nothing survives, and an empty file is a natural stand-in for "nothing",
    /// so every command reads it as no document rather than a parse error, and
    /// decides for itself what that means. `parse_schema`/`parse_query` return
    /// `None` for one, so no caller can forget to.
    ///
    /// The characters are the ones graphql-parser itself skips, so a document
    /// is blank exactly when parsing it would find nothing but its end.
    pub fn is_blank(&self) -> bool {
        let mut in_comment = false;
        self.text.chars().all(|c| match c {
            '\n' | '\r' => {
                in_comment = false;
                true
            }
            _ if in_comment => true,
            '#' => {
                in_comment = true;
                true
            }
            ' ' | '\t' | ',' | '\u{feff}' => true,
            _ => false,
        })
    }

    /// Parses the document as a schema, or `None` when it is blank, for a
    /// command that looks its definitions up by name.
    ///
    /// A document that defines a name more than once is invalid GraphQL, but
    /// it does not have to stop a command: every later definition of a type,
    /// directive, or the schema definition is dropped, so the first is the
    /// only one any command sees, and each repeated name adds a warning. See
    /// `drop_duplicates`. Extensions are not definitions, so any number of
    /// `extend` blocks may follow a type, or stand in for one.
    pub fn parse_schema(
        &self,
        warnings: &mut Vec<String>,
    ) -> Result<Option<schema::Document<'_, String>>, Error> {
        let mut document = self.parse_schema_as_written()?;
        if let Some(document) = &mut document {
            self.drop_duplicates(&mut document.definitions, warnings, |def| match def {
                schema::Definition::SchemaDefinition(sd) => {
                    Some(("`schema`".to_string(), sd.position))
                }
                schema::Definition::TypeDefinition(td) => {
                    let (name, position) = type_definition_name_and_position(td);
                    Some((format!("type `{name}`"), position))
                }
                schema::Definition::DirectiveDefinition(dd) => {
                    Some((format!("directive `@{}`", dd.name), dd.position))
                }
                schema::Definition::TypeExtension(_) => None,
            });
        }
        Ok(document)
    }

    /// Parses the document as a schema, or `None` when it is blank, keeping
    /// every definition as written, for a command that only lays it out
    /// (`schema format`, `sort`) and so never has to pick one of a repeated
    /// name.
    pub fn parse_schema_as_written(&self) -> Result<Option<schema::Document<'_, String>>, Error> {
        if self.is_blank() {
            return Ok(None);
        }
        schema::parse_schema(&self.text)
            .map(Some)
            .map_err(|e| self.parse_error(Kind::Schema, e))
    }

    /// Parses the document as a query, or `None` when it is blank.
    ///
    /// As with `parse_schema`, every later fragment or named operation of a
    /// name already defined is dropped, with a warning for each repeated name.
    /// Operation names share one namespace whatever the operation type, so a
    /// query and a mutation of one name are repeats too. Anonymous operations
    /// have no name to repeat.
    pub fn parse_query(
        &self,
        warnings: &mut Vec<String>,
    ) -> Result<Option<query::Document<'_, String>>, Error> {
        if self.is_blank() {
            return Ok(None);
        }
        let mut document =
            query::parse_query(&self.text).map_err(|e| self.parse_error(Kind::Query, e))?;
        self.drop_duplicates(&mut document.definitions, warnings, |def| match def {
            query::Definition::Fragment(fragment) => {
                Some((format!("fragment `{}`", fragment.name), fragment.position))
            }
            query::Definition::Operation(op) => match op {
                query::OperationDefinition::SelectionSet(_) => None,
                query::OperationDefinition::Query(op) => op.name.as_ref().zip(Some(op.position)),
                query::OperationDefinition::Mutation(op) => op.name.as_ref().zip(Some(op.position)),
                query::OperationDefinition::Subscription(op) => {
                    op.name.as_ref().zip(Some(op.position))
                }
            }
            .map(|(name, position)| (format!("operation `{name}`"), position)),
        });
        Ok(Some(document))
    }

    /// Keeps only the first of `definitions` under each name `name_of` gives,
    /// and adds a warning to `warnings` for each name that had more.
    ///
    /// `name_of` returns a definition's name as the warning prints it, which
    /// includes its kind (``type `User` ``), so names of different kinds never
    /// collide, and where it starts; `None` for one that has no name to repeat.
    ///
    /// The first definition wins, as it does wherever a command indexes types
    /// by name (`util::merged_type_definitions`), and the rest are dropped
    /// here, once, so no command meets a repeat at all. Left in, commands would
    /// disagree: the schema commands resolve a type to its first definition
    /// but print every definition of a type they keep, and the query commands
    /// index fragments with the last one winning. The positions say which
    /// definitions to fix.
    fn drop_duplicates<T>(
        &self,
        definitions: &mut Vec<T>,
        warnings: &mut Vec<String>,
        name_of: impl Fn(&T) -> Option<(String, Pos)>,
    ) {
        // Every name in first-seen order, with where each definition of it
        // starts, so the warnings come out in source order.
        let mut names: Vec<(String, Vec<Pos>)> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        definitions.retain(|def| {
            let Some((name, position)) = name_of(def) else {
                return true;
            };
            match index.get(&name) {
                Some(&i) => {
                    names[i].1.push(position);
                    false
                }
                None => {
                    index.insert(name.clone(), names.len());
                    names.push((name, vec![position]));
                    true
                }
            }
        });

        for (name, positions) in names {
            if positions.len() > 1 {
                let positions: Vec<String> = positions.iter().map(Pos::to_string).collect();
                warnings.push(format!(
                    "{name} is defined more than once in {} (at {}); only the first is used",
                    self.origin,
                    positions.join(", ")
                ));
            }
        }
    }

    /// Reports a parser error, from graphql-parser or a crate built on it such
    /// as graphql-normalize, against this input.
    pub fn parse_error(&self, kind: Kind, error: impl fmt::Display) -> Error {
        Error::parse(kind, self.origin.clone(), error)
    }
}

fn type_definition_name_and_position<'r>(
    td: &'r schema::TypeDefinition<'_, String>,
) -> (&'r String, Pos) {
    match td {
        schema::TypeDefinition::Scalar(t) => (&t.name, t.position),
        schema::TypeDefinition::Object(t) => (&t.name, t.position),
        schema::TypeDefinition::Interface(t) => (&t.name, t.position),
        schema::TypeDefinition::Union(t) => (&t.name, t.position),
        schema::TypeDefinition::Enum(t) => (&t.name, t.position),
        schema::TypeDefinition::InputObject(t) => (&t.name, t.position),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use std::str::FromStr;

    /// A scratch directory unique to one test, removed before use.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "graphql-document-utils-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_error(result: Result<Input, Error>) -> String {
        result.expect_err("expected the read to fail").to_string()
    }

    #[test]
    fn missing_file_names_its_path() {
        let path = scratch("missing").join("nope.graphql");

        assert_eq!(
            read_error(Input::from_path(&path, Kind::Schema)),
            format!(
                "cannot read schema '{}': No such file or directory",
                path.display()
            )
        );

        // clap-stdin labels this as a stdin error; it must still name the file.
        let arg = FileOrStdin::from_str(path.to_str().unwrap()).unwrap();
        assert_eq!(
            read_error(Input::read(arg, Kind::Query)),
            format!(
                "cannot read query '{}': No such file or directory",
                path.display()
            )
        );
    }

    #[test]
    fn directory_is_a_read_error() {
        let dir = scratch("directory");

        assert_eq!(
            read_error(Input::from_path(&dir, Kind::Schema)),
            format!("cannot read schema '{}': Is a directory", dir.display())
        );

        let arg = FileOrStdin::from_str(dir.to_str().unwrap()).unwrap();
        assert_eq!(
            read_error(Input::read(arg, Kind::Query)),
            format!("cannot read query '{}': Is a directory", dir.display())
        );
    }

    #[test]
    fn non_utf8_is_a_read_error() {
        let path = scratch("non-utf8").join("binary.graphql");
        fs::write(&path, b"{ a \xff }").unwrap();

        assert_eq!(
            read_error(Input::from_path(&path, Kind::Schema)),
            format!(
                "cannot read schema '{}': stream did not contain valid UTF-8",
                path.display()
            )
        );

        let arg = FileOrStdin::from_str(path.to_str().unwrap()).unwrap();
        assert_eq!(
            read_error(Input::read(arg, Kind::Query)),
            format!(
                "cannot read query '{}': stream did not contain valid UTF-8",
                path.display()
            )
        );
    }

    #[test]
    fn file_reads_whole_and_parses() {
        let path = scratch("ok").join("schema.graphql");
        fs::write(&path, "type Query { a: Int }\n").unwrap();

        let arg = FileOrStdin::from_str(path.to_str().unwrap()).unwrap();
        let input = Input::read(arg, Kind::Schema).unwrap();

        assert_eq!(input.text(), "type Query { a: Int }\n");
        assert!(input.parse_schema(&mut Vec::new()).unwrap().is_some());
    }

    #[test]
    fn blank_documents_parse_as_none() {
        for text in [
            "",
            "\n",
            "  \t\r\n  \n",
            ", ,\n",
            "\u{feff}\n",
            "# nothing here\n",
            "  # a comment, then a blank line\n\n# and another",
            "# a comment ending in a lone carriage return\r",
        ] {
            let input = Input::inline(text);
            assert!(input.is_blank(), "{text:?}");
            assert!(
                input.parse_schema(&mut Vec::new()).unwrap().is_none(),
                "{text:?}"
            );
            assert!(
                input.parse_schema_as_written().unwrap().is_none(),
                "{text:?}"
            );
            assert!(
                input.parse_query(&mut Vec::new()).unwrap().is_none(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_document_with_any_token_is_not_blank() {
        for text in [
            "{ a }",
            "  # a comment\n{ a }\n",
            // A line break ends a comment, whichever kind it is.
            "# a comment\r{ a }",
        ] {
            let input = Input::inline(text);
            assert!(!input.is_blank(), "{text:?}");
            assert!(
                input.parse_query(&mut Vec::new()).unwrap().is_some(),
                "{text:?}"
            );
        }

        // Only what GraphQL ignores counts, not every kind of whitespace, so a
        // form feed is left to the parser to reject.
        let input = Input::inline("\u{c}");
        assert!(!input.is_blank());
        assert!(input.parse_schema(&mut Vec::new()).is_err());
    }

    fn parse_error(result: Result<impl fmt::Debug, Error>) -> String {
        result.expect_err("expected the parse to fail").to_string()
    }

    #[test]
    fn syntax_error_reports_position_and_message() {
        assert_eq!(
            parse_error(Input::inline("{ a b(x: 1 }").parse_query(&mut Vec::new())),
            "failed to parse query (stdin) at 1:12: unexpected `}`; expected Name, : or )"
        );
        assert_eq!(
            parse_error(Input::inline("type Query { a: Int\n").parse_schema(&mut Vec::new())),
            "failed to parse schema (stdin) at 2:1: unexpected end of input; expected }"
        );
    }

    #[test]
    fn parser_limits_are_parse_errors() {
        assert_eq!(
            parse_error(
                Input::inline("{ a(x: 99999999999999999999) }").parse_query(&mut Vec::new())
            ),
            "failed to parse query (stdin) at 1:8: number too large to fit in target type"
        );

        let deep = format!("{{{} b {}}}", "a { ".repeat(60), "} ".repeat(60));
        let message = parse_error(Input::inline(&deep).parse_query(&mut Vec::new()));
        assert!(
            message.starts_with("failed to parse query (stdin) at 1:")
                && message.ends_with("recursion limit exceeded"),
            "{message}"
        );
    }

    /// Parses `text` as a schema, returning it printed with the warnings.
    fn schema_with_warnings(text: &str) -> (String, Vec<String>) {
        let mut warnings = Vec::new();
        let input = Input::inline(text);
        let document = input.parse_schema(&mut warnings).unwrap();
        (document.unwrap().to_string(), warnings)
    }

    /// Parses `text` as a query, returning it printed with the warnings.
    fn query_with_warnings(text: &str) -> (String, Vec<String>) {
        let mut warnings = Vec::new();
        let input = Input::inline(text);
        let document = input.parse_query(&mut warnings).unwrap();
        (document.unwrap().to_string(), warnings)
    }

    #[test]
    fn keeps_the_first_definition_of_a_repeated_type() {
        // A repeat of a different kind is still a repeat: a name is one type.
        let schema = indoc! {"
            type User {
              id: ID
            }

            type Query {
              user: User
            }

            type User {
              email: String
            }

            scalar User
        "};

        assert_eq!(
            schema_with_warnings(schema),
            (
                indoc! {"
                    type User {
                      id: ID
                    }

                    type Query {
                      user: User
                    }
                "}
                .to_string(),
                vec![
                    "type `User` is defined more than once in (stdin) (at 1:1, 9:1, 13:1); only the first is used"
                        .to_string()
                ]
            )
        );
    }

    #[test]
    fn keeps_the_first_of_a_repeated_directive_or_schema_definition() {
        let schema = indoc! {"
            schema {
              query: Query
            }

            directive @tag(name: String) on FIELD_DEFINITION

            type Query {
              a: Int
            }

            directive @tag(label: Int) on OBJECT

            schema {
              query: Other
            }
        "};

        assert_eq!(
            schema_with_warnings(schema),
            (
                indoc! {"
                    schema {
                      query: Query
                    }

                    directive @tag(name: String) on FIELD_DEFINITION

                    type Query {
                      a: Int
                    }
                "}
                .to_string(),
                vec![
                    "`schema` is defined more than once in (stdin) (at 1:1, 13:1); only the first is used"
                        .to_string(),
                    "directive `@tag` is defined more than once in (stdin) (at 5:1, 11:1); only the first is used"
                        .to_string(),
                ]
            )
        );
    }

    #[test]
    fn a_type_and_a_directive_may_share_a_name() {
        let schema = "directive @key on OBJECT

scalar key
";

        assert_eq!(schema_with_warnings(schema), (schema.to_string(), vec![]));
    }

    #[test]
    fn extensions_are_not_repeats() {
        // `Query` is only ever extended, as in a federation subgraph, and
        // `User` is extended both before and after its definition.
        let schema = indoc! {"
            extend type Query {
              me: User
            }

            extend type Query {
              bots: [User]
            }

            extend type User {
              avatar: String
            }

            type User {
              id: ID
            }

            extend type User {
              name: String
            }
        "};

        assert_eq!(schema_with_warnings(schema), (schema.to_string(), vec![]));
    }

    #[test]
    fn formatting_keeps_every_definition_as_written() {
        let schema = "scalar Date

scalar Date
";

        assert_eq!(
            Input::inline(schema)
                .parse_schema_as_written()
                .unwrap()
                .unwrap()
                .to_string(),
            schema
        );
    }

    #[test]
    fn keeps_the_first_definition_of_a_repeated_fragment() {
        let query = indoc! {"
            query GetUser {
              user {
                ...UserFields
              }
            }

            fragment UserFields on User {
              id
            }

            fragment UserFields on User {
              name
            }
        "};

        assert_eq!(
            query_with_warnings(query),
            (
                indoc! {"
                    query GetUser {
                      user {
                        ...UserFields
                      }
                    }

                    fragment UserFields on User {
                      id
                    }
                "}
                .to_string(),
                vec![
                    "fragment `UserFields` is defined more than once in (stdin) (at 7:1, 11:1); only the first is used"
                        .to_string()
                ]
            )
        );
    }

    #[test]
    fn keeps_the_first_operation_of_a_repeated_name_whatever_its_type() {
        // Operation names share one namespace, and a fragment's name is not in
        // it. Anonymous operations have no name to repeat.
        let query = indoc! {"
            query Save {
              a
            }

            mutation Save {
              b
            }

            fragment Save on Query {
              c
            }

            {
              d
            }

            {
              e
            }
        "};

        assert_eq!(
            query_with_warnings(query),
            (
                indoc! {"
                    query Save {
                      a
                    }

                    fragment Save on Query {
                      c
                    }

                    {
                      d
                    }

                    {
                      e
                    }
                "}
                .to_string(),
                vec![
                    "operation `Save` is defined more than once in (stdin) (at 1:1, 5:1); only the first is used"
                        .to_string()
                ]
            )
        );
    }
}
