use crate::input::{Kind, Origin};
use graphql_parser::Pos;
use std::{fmt, io, process::ExitCode};

/// A problem with what the user gave the tool, or with where its output goes,
/// as opposed to a bug in it.
///
/// `main` prints these as one `error: ...` line on stderr and exits with
/// `exit_code`. Each names the document and where it came from, so the message
/// says which input to fix.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A document was to be read from stdin, but stdin is a terminal: neither
    /// its flag nor a pipe gave one, and reading would wait on the keyboard
    /// forever. A usage error, since the command line is what needs fixing.
    #[error("no {kind} given; pass {} FILE or pipe one on stdin", .kind.flag())]
    NoInput { kind: Kind },

    /// Both documents were to be read from stdin, which holds only one. A
    /// usage error, like `NoInput`.
    #[error("cannot read both the schema and the query from stdin; pass one of them as a file with -s FILE or -q FILE")]
    StdinTwice,

    /// A document could not be read: a missing file, a directory, or bytes
    /// that are not UTF-8.
    #[error("cannot read {kind} {origin}: {}", io_message(.source))]
    Read {
        kind: Kind,
        origin: Origin,
        source: io::Error,
    },

    /// A document is not valid GraphQL, or exceeds one of the parser's limits
    /// (integers past `i64`, nesting past its recursion limit).
    #[error("failed to parse {kind} {origin}{}: {message}", at(.position))]
    Parse {
        kind: Kind,
        origin: Origin,
        position: Option<Pos>,
        message: String,
    },

    /// A query command's schema is blank (see `Input::is_blank`). Blank
    /// documents pass through every command as empty, but this one is not the
    /// document being transformed: the query is resolved against it, and
    /// against nothing no target is known and no field resolves.
    #[error("schema {origin} is empty; pass the schema the query is written against with -s")]
    BlankSchema { origin: Origin },

    /// A target is not in the form `Type` or `Type.field`.
    #[error("invalid target `{target}`; expected `Type` or `Type.field`")]
    InvalidTarget { target: String },

    /// A target that is not a type or field but looks like a file path, most
    /// likely a document passed positionally instead of with its flag. `kind`
    /// is the document the command reads that way, which picks the flag.
    #[error("unknown target '{target}'; {}", pass_with_flag(*.kind, .target))]
    PathAsTarget { target: String, kind: Kind },

    /// A target names a type the schema does not define.
    #[error("unknown type `{name}` in {origin}{}", did_you_mean(.suggestion))]
    UnknownType {
        name: String,
        origin: Origin,
        suggestion: Option<String>,
    },

    /// A `Type.field` target names a field that neither the type nor any of
    /// its subtypes declares.
    #[error("`{type_name}` has no field `{field}`{}", did_you_mean(.suggestion))]
    UnknownField {
        type_name: String,
        field: String,
        suggestion: Option<String>,
    },

    /// A `Type.field` target names a type with no selectable fields, such as
    /// an input object or an enum.
    #[error("`{type_name}` is {kind}, which has no fields to select (got `{target}`)")]
    NoFields {
        type_name: String,
        kind: &'static str,
        target: String,
    },

    /// `schema focus` was given a `Type.field` target.
    #[error("`schema focus` takes type names; for fields use `query focus` (got `{target}`)")]
    FieldInSchemaFocus { target: String },

    /// `schema focus` was given a built-in scalar the schema does not define,
    /// so there is no definition to keep.
    #[error("`{name}` is a built-in scalar, which {origin} does not define; `schema focus` takes types the schema defines")]
    BuiltInScalar { name: String, origin: Origin },

    /// The document could not be written to stdout, as when it is a file on
    /// a full disk. A reader closing the pipe early is not an error; `emit`
    /// treats that as success.
    #[error("cannot write output: {}", io_message(.source))]
    Write { source: io::Error },
}

impl Error {
    /// Whether the command line is what needs fixing, as with clap's own
    /// errors, rather than an input or where the output goes.
    pub fn is_usage(&self) -> bool {
        matches!(self, Error::NoInput { .. } | Error::StdinTwice)
    }

    /// The status the process exits with: 2 for a usage error, the same as
    /// clap's own, and 1 for everything else.
    pub fn status(&self) -> u8 {
        if self.is_usage() {
            2
        } else {
            1
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.status())
    }

    /// What went wrong, in a few words, for the skill's list of what each exit
    /// code means. Variants that read the same to a user share one.
    pub fn summary(&self) -> &'static str {
        match self {
            Error::NoInput { .. } => "no document to read (no flag, and stdin a terminal)",
            Error::StdinTwice => "both of `schema prune`'s documents on stdin",
            Error::Read { .. } => "an unreadable file",
            Error::Parse { .. } => "a parse error (with line and column)",
            Error::BlankSchema { .. } => "a blank schema for `query focus` or `query strip`",
            Error::InvalidTarget { .. } | Error::PathAsTarget { .. } => {
                "a malformed target (or a file passed as one)"
            }
            Error::UnknownType { .. } | Error::UnknownField { .. } | Error::NoFields { .. } => {
                "an unknown type or field"
            }
            Error::FieldInSchemaFocus { .. } | Error::BuiltInScalar { .. } => {
                "a field or built-in scalar given to `schema focus`"
            }
            Error::Write { .. } => "a failed write to stdout (as on a full disk)",
        }
    }

    /// One error of each variant, in the order they are declared, so what the
    /// exit codes mean can be listed from the errors themselves. A test checks
    /// no variant is missing.
    pub fn one_of_each() -> Vec<Error> {
        let origin = || Origin::Stdin;
        let target = || String::new();
        let source = || io::Error::other("");
        vec![
            Error::NoInput { kind: Kind::Query },
            Error::StdinTwice,
            Error::Read {
                kind: Kind::Query,
                origin: origin(),
                source: source(),
            },
            Error::Parse {
                kind: Kind::Query,
                origin: origin(),
                position: None,
                message: String::new(),
            },
            Error::BlankSchema { origin: origin() },
            Error::InvalidTarget { target: target() },
            Error::PathAsTarget {
                target: target(),
                kind: Kind::Query,
            },
            Error::UnknownType {
                name: target(),
                origin: origin(),
                suggestion: None,
            },
            Error::UnknownField {
                type_name: target(),
                field: target(),
                suggestion: None,
            },
            Error::NoFields {
                type_name: target(),
                kind: "",
                target: target(),
            },
            Error::FieldInSchemaFocus { target: target() },
            Error::BuiltInScalar {
                name: target(),
                origin: origin(),
            },
            Error::Write { source: source() },
        ]
    }

    /// Builds a `Parse` error from one of graphql-parser's.
    ///
    /// Its errors are opaque strings, so the position and message are
    /// recovered from the text, which looks like
    /// "query parse error: Parse error at 1:12\nUnexpected `}[Punctuator]`\nExpected Name\n".
    /// The lines become one `; `-separated message in Rust's lowercase error
    /// style, with the token kinds the parser appends dropped. Text in any
    /// other shape is kept whole, with no position.
    pub fn parse(kind: Kind, origin: Origin, error: impl fmt::Display) -> Self {
        let text = error.to_string();
        let located = text.split_once("Parse error at ").and_then(|(_, rest)| {
            let (position, rest) = rest.split_once('\n').unwrap_or((rest, ""));
            let (line, column) = position.split_once(':')?;
            let position = Pos {
                line: line.parse().ok()?,
                column: column.parse().ok()?,
            };
            Some((position, rest))
        });
        let (position, rest) = match located {
            Some((position, rest)) => (Some(position), rest),
            None => (None, text.as_str()),
        };

        let message = rest
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let line = TOKEN_KINDS.iter().fold(line.to_string(), |line, kind| {
                    line.replace(&format!("[{kind}]`"), "`")
                });
                let mut chars = line.chars();
                match chars.next() {
                    Some(first) => first.to_lowercase().chain(chars).collect(),
                    None => line,
                }
            })
            .collect::<Vec<String>>()
            .join("; ");

        Error::Parse {
            kind,
            origin,
            position,
            message,
        }
    }
}

/// graphql-parser's token kinds, which it prints after each token it quotes in
/// an error (`` `}[Punctuator]` ``).
const TOKEN_KINDS: [&str; 6] = [
    "Punctuator",
    "Name",
    "IntValue",
    "FloatValue",
    "StringValue",
    "BlockString",
];

/// `"; did you mean `Name`?"`, or nothing without a suggestion.
fn did_you_mean(suggestion: &Option<String>) -> String {
    suggestion
        .as_ref()
        .map(|s| format!("; did you mean `{s}`?"))
        .unwrap_or_default()
}

/// Where the document `path` belongs on the command line. The query commands
/// read their query from `-q`; `schema focus` reads a single schema from `-s`.
fn pass_with_flag(kind: Kind, path: &str) -> String {
    let flag = kind.flag();
    match kind {
        Kind::Query => format!("pass query files with {flag}: {flag} {path}"),
        Kind::Schema => format!("pass the schema file with {flag}: {flag} {path}"),
    }
}

/// `" at line:column"`, or nothing when the position is unknown.
fn at(position: &Option<Pos>) -> String {
    position.map(|p| format!(" at {p}")).unwrap_or_default()
}

/// The error's message without the " (os error 2)" suffix std appends, which
/// only repeats the message as a number.
fn io_message(error: &io::Error) -> String {
    let message = error.to_string();
    match message.rsplit_once(" (os error ") {
        Some((message, _)) if error.raw_os_error().is_some() => message.to_string(),
        _ => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    fn file(path: &str) -> Origin {
        Origin::File(PathBuf::from(path))
    }

    #[test]
    fn no_input_names_the_flag_for_the_document() {
        assert_eq!(
            Error::NoInput { kind: Kind::Query }.to_string(),
            "no query given; pass -q FILE or pipe one on stdin"
        );
        assert_eq!(
            Error::NoInput { kind: Kind::Schema }.to_string(),
            "no schema given; pass -s FILE or pipe one on stdin"
        );
    }

    /// `one_of_each` has every variant. The match has no wildcard, so a new
    /// variant fails to compile here until it is given a place, and `one_of_each`
    /// must then hold it for the places to be all there.
    #[test]
    fn one_of_each_has_every_variant() {
        let places: Vec<usize> = Error::one_of_each()
            .iter()
            .map(|error| match error {
                Error::NoInput { .. } => 0,
                Error::StdinTwice => 1,
                Error::Read { .. } => 2,
                Error::Parse { .. } => 3,
                Error::BlankSchema { .. } => 4,
                Error::InvalidTarget { .. } => 5,
                Error::PathAsTarget { .. } => 6,
                Error::UnknownType { .. } => 7,
                Error::UnknownField { .. } => 8,
                Error::NoFields { .. } => 9,
                Error::FieldInSchemaFocus { .. } => 10,
                Error::BuiltInScalar { .. } => 11,
                Error::Write { .. } => 12,
            })
            .collect();
        assert_eq!(places, (0..=12).collect::<Vec<_>>());
    }

    /// A usage error exits as clap's own do, which the skill and the help say.
    #[test]
    fn usage_errors_exit_as_clap_does() {
        let clap = <crate::Args as clap::Parser>::try_parse_from(["bin", "--unknown"]);
        let clap = clap.expect_err("an unknown flag is an error").exit_code();
        for error in Error::one_of_each() {
            assert_eq!(
                i32::from(error.status()) == clap,
                error.is_usage(),
                "{error:?}"
            );
        }
    }

    #[test]
    fn usage_errors_exit_2_and_input_errors_exit_1() {
        assert_eq!(
            Error::NoInput { kind: Kind::Query }.exit_code(),
            ExitCode::from(2)
        );
        assert_eq!(Error::StdinTwice.exit_code(), ExitCode::from(2));
        let read = Error::Read {
            kind: Kind::Query,
            origin: Origin::Stdin,
            source: io::Error::from_raw_os_error(2),
        };
        assert_eq!(read.exit_code(), ExitCode::FAILURE);
    }

    #[test]
    fn read_names_the_document_and_drops_the_os_error_code() {
        let error = Error::Read {
            kind: Kind::Schema,
            origin: file("nope.graphql"),
            source: io::Error::from_raw_os_error(2),
        };

        assert_eq!(
            error.to_string(),
            "cannot read schema 'nope.graphql': No such file or directory"
        );
    }

    #[test]
    fn read_keeps_messages_without_an_os_error_code() {
        let error = Error::Read {
            kind: Kind::Query,
            origin: Origin::Stdin,
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            ),
        };

        assert_eq!(
            error.to_string(),
            "cannot read query (stdin): stream did not contain valid UTF-8"
        );
    }

    #[test]
    fn write_drops_the_os_error_code() {
        let error = Error::Write {
            source: io::Error::from_raw_os_error(28),
        };

        assert_eq!(error.exit_code(), ExitCode::FAILURE);
        assert_eq!(
            error.to_string(),
            "cannot write output: No space left on device"
        );
    }

    #[test]
    fn parse_extracts_the_position_and_cleans_the_message() {
        let raw = "query parse error: Parse error at 1:12\nUnexpected `}[Punctuator]`\nExpected Name, : or )\n";
        let error = Error::parse(Kind::Query, Origin::Stdin, raw);

        let Error::Parse { position, .. } = &error else {
            panic!("expected a parse error, got {error:?}");
        };
        assert_eq!(
            *position,
            Some(Pos {
                line: 1,
                column: 12
            })
        );
        assert_eq!(
            error.to_string(),
            "failed to parse query (stdin) at 1:12: unexpected `}`; expected Name, : or )"
        );
    }

    #[test]
    fn parse_keeps_unrecognized_text_whole_without_a_position() {
        let error = Error::parse(Kind::Schema, file("s.graphql"), "something else went wrong");

        assert_eq!(
            error.to_string(),
            "failed to parse schema 's.graphql': something else went wrong"
        );
    }
}
