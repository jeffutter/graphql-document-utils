use crate::{
    error::Error,
    input::{Input, Kind, Origin},
    util,
};
use graphql_parser::schema::{
    Definition as SchemaDef, DirectiveDefinition, Document as SchemaDoc, Field as SchemaField,
    InputValue, TypeDefinition,
};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

/// Parses the schema a query command resolves its query against.
///
/// Unlike the query, a blank schema does not pass through: it is not the
/// document being transformed but what the query is resolved against, and
/// against nothing no field resolves. Read as a schema with no definitions,
/// most targets would fail as unknown types, but a built-in scalar such as
/// `String` would still validate and quietly match nothing. So a blank schema
/// fails with `Error::BlankSchema` instead, before the targets are checked and
/// whether or not the query is blank.
///
/// A name the schema defines more than once adds a warning to `warnings`, and
/// only its first definition is kept (see `Input::parse_schema`).
pub fn parse_schema<'s>(
    schema: &'s Input,
    warnings: &mut Vec<String>,
) -> Result<SchemaDoc<'s, String>, Error> {
    schema
        .parse_schema(warnings)?
        .ok_or_else(|| Error::BlankSchema {
            origin: schema.origin().clone(),
        })
}

/// A target: either a bare type name (`MyType`), or a field qualified by the
/// type it is selected on (`MyType.field`).
#[derive(Clone)]
enum Target<'t> {
    Type(&'t str),
    Field { type_name: &'t str, name: &'t str },
}

impl<'t> Target<'t> {
    fn parse(target: &'t str) -> Result<Self, Error> {
        match target.split_once('.') {
            None if !target.is_empty() => Ok(Target::Type(target)),
            Some((type_name, name))
                if !type_name.is_empty() && !name.is_empty() && !name.contains('.') =>
            {
                Ok(Target::Field { type_name, name })
            }
            _ => Err(Error::InvalidTarget {
                target: target.to_string(),
            }),
        }
    }
}

/// Decides whether a query selection hits one of the targets, resolving field
/// and argument types against the schema on the way.
///
/// Matching is subtype-aware in both directions: a target naming an interface
/// or union matches its implementors and members, and `Person.name` matches a
/// `name` selected on `User` as readily as `User.name` matches a `name`
/// selected on the `Person` interface. Shared by `query focus` and
/// `query strip`, which agree on what a target *is* and differ only in what
/// they do with a match.
///
/// Types are resolved with their `extend` blocks merged in, so a field or
/// interface an extension adds is known like any other.
///
/// Cloning is cheap: the schema indexes are shared, so a command can match
/// each target on its own (see `only`) without rebuilding them.
#[derive(Clone)]
pub struct Matcher<'s, 't> {
    type_map: Rc<HashMap<String, TypeDefinition<'s, String>>>,
    directives: Rc<HashMap<String, &'s DirectiveDefinition<'s, String>>>,
    /// Each target as given, alongside its parsed form.
    targets: Vec<(&'t str, Target<'t>)>,
}

impl<'s, 't> Matcher<'s, 't> {
    /// Builds a matcher for `targets`, failing on the first one the schema
    /// does not define. `origin` is where the schema came from, which an
    /// unknown type is reported against.
    ///
    /// Needs only the schema, so targets can be checked before the query is
    /// read or parsed.
    pub fn new(
        schema: &'s SchemaDoc<'s, String>,
        origin: &Origin,
        targets: &'t [&'t str],
    ) -> Result<Self, Error> {
        let mut matcher = Matcher {
            type_map: Rc::new(util::merged_type_definitions(schema)),
            directives: Rc::new(
                schema
                    .definitions
                    .iter()
                    .filter_map(|def| match def {
                        SchemaDef::DirectiveDefinition(directive) => {
                            Some((directive.name.clone(), directive))
                        }
                        _ => None,
                    })
                    .collect(),
            ),
            targets: Vec::new(),
        };
        for target in targets {
            // A path is checked only once the target has failed, since
            // `Query.graphql` could be a real field. It then wins over the
            // failure, which would have read `query.graphql` as the field
            // `graphql` on a type `query`.
            let parsed = matcher.validate(target, origin).map_err(|error| {
                if looks_like_path(target) {
                    Error::PathAsTarget {
                        target: target.to_string(),
                        kind: Kind::Query,
                    }
                } else {
                    error
                }
            })?;
            matcher.targets.push((target, parsed));
        }
        Ok(matcher)
    }

    /// Parses `target` and checks that the schema knows what it names.
    ///
    /// A type is known if the schema defines it, with extensions merged in, or
    /// if it is a built-in scalar: `String` is a fine target for a query even
    /// though no schema writes it down. A field is known if the type or any of
    /// its subtypes declares it, since field targets match across the
    /// hierarchy: `Node.name` reaches `name` on every `Node` that has one, and
    /// `SearchResult.title` reaches `title` on the union's members.
    fn validate(&self, target: &'t str, origin: &Origin) -> Result<Target<'t>, Error> {
        let parsed = Target::parse(target)?;
        let type_name = match parsed {
            Target::Type(type_name) | Target::Field { type_name, .. } => type_name,
        };

        let definition = self.type_map.get(type_name);
        if definition.is_none() && !util::BUILT_IN_SCALARS.contains(&type_name) {
            return Err(unknown_type(
                type_name,
                origin,
                self.type_map
                    .keys()
                    .map(String::as_str)
                    .chain(util::BUILT_IN_SCALARS),
            ));
        }

        let Target::Field { name, .. } = parsed else {
            return Ok(parsed);
        };
        let kind = match definition {
            Some(
                TypeDefinition::Object(_) | TypeDefinition::Interface(_) | TypeDefinition::Union(_),
            ) => None,
            Some(TypeDefinition::Enum(_)) => Some("an enum"),
            Some(TypeDefinition::InputObject(_)) => Some("an input object type"),
            Some(TypeDefinition::Scalar(_)) | None => Some("a scalar"),
        };
        if let Some(kind) = kind {
            return Err(Error::NoFields {
                type_name: type_name.to_string(),
                kind,
                target: target.to_string(),
            });
        }

        let fields: HashSet<&str> = self
            .type_map
            .iter()
            .filter(|(sub, _)| self.is_subtype_of(sub, type_name))
            .filter_map(|(_, definition)| util::type_fields(definition))
            .flatten()
            .map(|field| field.name.as_str())
            .collect();
        if fields.contains(name) {
            return Ok(parsed);
        }
        Err(Error::UnknownField {
            type_name: type_name.to_string(),
            field: name.to_string(),
            suggestion: suggest(name, fields),
        })
    }

    /// The targets, as given.
    pub fn targets(&self) -> impl Iterator<Item = &'t str> + '_ {
        self.targets.iter().map(|(target, _)| *target)
    }

    /// A matcher over the same schema that matches only `target`, one of
    /// `targets()`, or nothing at all when `None`. Lets a command tell which
    /// targets matched: running it once per target is the only way to see a
    /// match that the full run never visited because it sat inside another
    /// target's match.
    pub fn only(&self, target: Option<&str>) -> Self {
        Matcher {
            targets: self
                .targets
                .iter()
                .filter(|(given, _)| Some(*given) == target)
                .cloned()
                .collect(),
            ..self.clone()
        }
    }

    pub fn field_def(&self, parent_type: &str, name: &str) -> Option<&SchemaField<'s, String>> {
        self.type_map
            .get(parent_type)
            .and_then(util::type_fields)
            .and_then(|fields| fields.iter().find(|field| field.name == *name))
    }

    /// The name of the type a field lands on, with list and non-null wrappers
    /// stripped. `None` when the schema does not define the field, which is the
    /// case for meta fields like `__typename`.
    pub fn field_type(&self, parent_type: &str, name: &str) -> Option<&str> {
        self.field_def(parent_type, name)
            .and_then(|def| util::named_type(&def.field_type))
            .map(String::as_str)
    }

    /// The arguments a field declares. `None` when the schema does not define
    /// the field.
    pub fn field_arguments(
        &self,
        parent_type: &str,
        field_name: &str,
    ) -> Option<&[InputValue<'s, String>]> {
        self.field_def(parent_type, field_name)
            .map(|def| def.arguments.as_slice())
    }

    /// The fields an input object type declares. `None` when `type_name` is not
    /// an input object the schema defines.
    pub fn input_fields(&self, type_name: &str) -> Option<&[InputValue<'s, String>]> {
        match self.type_map.get(type_name) {
            Some(TypeDefinition::InputObject(input)) => Some(&input.fields),
            _ => None,
        }
    }

    /// The arguments a directive declares. `None` when the schema does not
    /// define the directive, which is usually the case for the built-in
    /// `@include` and `@skip`.
    pub fn directive_arguments(&self, name: &str) -> Option<&[InputValue<'s, String>]> {
        self.directives
            .get(name)
            .map(|directive| directive.arguments.as_slice())
    }

    /// True if arriving at `type_name` means arriving at one of the targeted
    /// types. Subtypes count: reaching a `User` is reaching a `Person` when
    /// `User implements Person`.
    pub fn is_type_target(&self, type_name: &str) -> bool {
        self.targets.iter().any(|(_, target)| match target {
            Target::Type(target) => self.is_subtype_of(type_name, target),
            Target::Field { .. } => false,
        })
    }

    /// True if `name` selected on `parent_type` is one of the targeted fields.
    pub fn is_field_target(&self, parent_type: &str, name: &str) -> bool {
        self.targets.iter().any(|(_, target)| match target {
            Target::Field {
                type_name: target_type,
                name: target_name,
            } => {
                *target_name == name
                    && (self.is_subtype_of(parent_type, target_type)
                        || self.is_subtype_of(target_type, parent_type))
            }
            Target::Type(_) => false,
        })
    }

    /// True if `sub` is `sup`, implements the interface `sup`, or is a member of
    /// the union `sup`.
    fn is_subtype_of(&self, sub: &str, sup: &str) -> bool {
        if sub == sup {
            return true;
        }
        match self.type_map.get(sup) {
            Some(TypeDefinition::Union(union_type)) => {
                union_type.types.iter().any(|member| member == sub)
            }
            Some(TypeDefinition::Interface(_)) => self.implements(sub, sup, &mut HashSet::new()),
            _ => false,
        }
    }

    /// Walks the interface hierarchy, since interfaces may themselves implement
    /// interfaces. `seen` breaks cycles in invalid schemas.
    fn implements(&self, type_name: &str, interface: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(type_name.to_string()) {
            return false;
        }
        let interfaces = match self.type_map.get(type_name) {
            Some(TypeDefinition::Object(obj)) => &obj.implements_interfaces,
            Some(TypeDefinition::Interface(iface)) => &iface.implements_interfaces,
            _ => return false,
        };
        interfaces
            .iter()
            .any(|i| i == interface || self.implements(i, interface, seen))
    }
}

/// True if `target` looks like a file path rather than `Type` or `Type.field`:
/// it contains a `/`, or ends in a GraphQL file extension. Passing a document
/// positionally is an easy slip, and the query commands would otherwise go on
/// to wait on stdin for the query that was never piped.
pub fn looks_like_path(target: &str) -> bool {
    target.contains('/') || target.ends_with(".graphql") || target.ends_with(".gql")
}

/// The error for a type name the schema does not define, suggesting the
/// closest of the `known` names.
pub fn unknown_type<'a>(
    name: &str,
    origin: &Origin,
    known: impl IntoIterator<Item = &'a str>,
) -> Error {
    Error::UnknownType {
        name: name.to_string(),
        origin: origin.clone(),
        suggestion: suggest(name, known),
    }
}

/// The candidate `name` was most likely meant to be, if any is close enough.
///
/// A candidate differing only in case wins outright, since that is the
/// likeliest slip (`user` for `User`). Otherwise the closest by Jaro
/// similarity wins, the measure clap uses for its own suggestions, if it is
/// above 0.8. clap settles for 0.7, but a schema has far more names to choose
/// from than a CLI has subcommands, and at 0.7 unrelated ones get suggested
/// (`Nothing` for `String`). One wrong letter in a four-letter name still
/// scores 0.83. Ties go to the alphabetically first, so the suggestion does not
/// depend on hash order.
fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let name = name.to_lowercase();
    let mut best: Option<(f64, &str)> = None;
    for candidate in candidates {
        let lowered = candidate.to_lowercase();
        let score = if lowered == name {
            f64::INFINITY
        } else {
            strsim::jaro(&name, &lowered)
        };
        let better = match best {
            None => score > 0.8,
            Some((best_score, best)) => {
                score > best_score || (score == best_score && candidate < best)
            }
        };
        if better {
            best = Some((score, candidate));
        }
    }
    best.map(|(_, candidate)| candidate.to_string())
}

/// Targets as a readable list: `` `A` ``, `` `A` or `B` ``, `` `A`, `B` or `C` ``.
pub fn quoted_list(targets: &[&str]) -> String {
    let quoted: Vec<String> = targets.iter().map(|t| format!("`{t}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    const SCHEMA: &str = indoc! {"
        type Query {
          node: Node
          search(filter: SearchFilter): [SearchResult]
        }

        interface Node {
          id: ID!
        }

        type User implements Node {
          id: ID!
          name: String
        }

        type Post implements Node {
          id: ID!
          title: String
        }

        union SearchResult = User | Post

        input SearchFilter {
          term: String
        }

        enum Role {
          ADMIN
        }

        extend type User {
          email: String
        }

        extend type Query {
          viewer: Viewer
        }

        type Viewer {
          id: ID!
        }

        extend union SearchResult = Comment

        type Comment {
          body: String
        }
    "};

    /// The error `Matcher::new` reports for `targets`, against a schema read
    /// from `schema.graphql`.
    fn error(targets: &[&str]) -> String {
        let schema = graphql_parser::parse_schema::<String>(SCHEMA).unwrap();
        let origin = Origin::File(PathBuf::from("schema.graphql"));
        match Matcher::new(&schema, &origin, targets) {
            Ok(_) => panic!("expected {targets:?} to be rejected"),
            Err(error) => error.to_string(),
        }
    }

    fn accepts(targets: &[&str]) {
        let schema = graphql_parser::parse_schema::<String>(SCHEMA).unwrap();
        if let Err(error) = Matcher::new(&schema, &Origin::Stdin, targets) {
            panic!("expected {targets:?} to be accepted, got: {error}");
        }
    }

    #[test]
    fn accepts_defined_types_and_fields() {
        accepts(&[
            "User",
            "Node",
            "SearchResult",
            "SearchFilter",
            "Role",
            "User.name",
        ]);
    }

    #[test]
    fn accepts_built_in_scalars() {
        accepts(&["String", "ID"]);
    }

    #[test]
    fn accepts_types_and_fields_added_by_extensions() {
        accepts(&[
            "User.email",
            "Query.viewer",
            "Viewer.id",
            "SearchResult.body",
        ]);
    }

    /// Field targets match across the hierarchy, so a field any subtype
    /// declares is a field of the target's type.
    #[test]
    fn accepts_fields_declared_only_by_subtypes() {
        accepts(&["Node.name", "Node.title", "SearchResult.title"]);
    }

    #[test]
    fn rejects_an_unknown_type_naming_the_schema() {
        assert_eq!(
            error(&["Usr"]),
            "unknown type `Usr` in 'schema.graphql'; did you mean `User`?"
        );
        assert_eq!(
            error(&["Nothing"]),
            "unknown type `Nothing` in 'schema.graphql'"
        );
        assert_eq!(
            error(&["Usr.name"]),
            "unknown type `Usr` in 'schema.graphql'; did you mean `User`?"
        );
    }

    #[test]
    fn suggests_a_case_insensitive_match_first() {
        assert_eq!(
            error(&["user"]),
            "unknown type `user` in 'schema.graphql'; did you mean `User`?"
        );
        assert_eq!(
            error(&["string"]),
            "unknown type `string` in 'schema.graphql'; did you mean `String`?"
        );
        assert_eq!(
            error(&["User.Name"]),
            "`User` has no field `Name`; did you mean `name`?"
        );
    }

    #[test]
    fn rejects_an_unknown_field_with_a_suggestion() {
        assert_eq!(
            error(&["User.nmae"]),
            "`User` has no field `nmae`; did you mean `name`?"
        );
        assert_eq!(
            error(&["Node.titel"]),
            "`Node` has no field `titel`; did you mean `title`?"
        );
        assert_eq!(error(&["User.zzz"]), "`User` has no field `zzz`");
    }

    #[test]
    fn rejects_fields_of_types_without_selectable_fields() {
        assert_eq!(
            error(&["SearchFilter.term"]),
            "`SearchFilter` is an input object type, which has no fields to select (got `SearchFilter.term`)"
        );
        assert_eq!(
            error(&["Role.ADMIN"]),
            "`Role` is an enum, which has no fields to select (got `Role.ADMIN`)"
        );
        assert_eq!(
            error(&["String.length"]),
            "`String` is a scalar, which has no fields to select (got `String.length`)"
        );
    }

    #[test]
    fn rejects_malformed_targets() {
        for target in ["", ".name", "User.", "User.name.first"] {
            assert_eq!(
                error(&[target]),
                format!("invalid target `{target}`; expected `Type` or `Type.field`")
            );
        }
    }

    /// A query file passed positionally reads as `Type.field`, and would
    /// otherwise fail as an unknown type, or an unknown field with a
    /// suggestion, without saying what went wrong.
    #[test]
    fn rejects_a_path_with_a_hint_to_pass_it_with_its_flag() {
        for target in [
            "query.graphql",
            "user.graphql",
            "User.graphql",
            "queries/user.gql",
            "./query",
            "Node.gql",
        ] {
            assert_eq!(
                error(&[target]),
                format!("unknown target '{target}'; pass query files with -q: -q {target}")
            );
        }
    }

    /// The hint is only for targets that fail: one that merely looks like a
    /// path but names a real field is still a field.
    #[test]
    fn accepts_a_field_that_looks_like_a_path() {
        let schema =
            graphql_parser::parse_schema::<String>("type Query { graphql: String }").unwrap();
        assert!(Matcher::new(&schema, &Origin::Stdin, &["Query.graphql"]).is_ok());
    }

    #[test]
    fn breaks_suggestion_ties_alphabetically() {
        assert_eq!(suggest("Usxr", ["Usbr", "Usar"]).as_deref(), Some("Usar"));
        assert_eq!(suggest("Usxr", ["Usar", "Usbr"]).as_deref(), Some("Usar"));
    }

    #[test]
    fn lists_targets_readably() {
        assert_eq!(quoted_list(&["A"]), "`A`");
        assert_eq!(quoted_list(&["A", "B"]), "`A` or `B`");
        assert_eq!(quoted_list(&["A", "B", "C"]), "`A`, `B` or `C`");
    }
}
