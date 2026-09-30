mod error;
mod focus;
mod input;
mod prune;
mod query_focus;
mod query_strip;
mod query_target;
mod sort;
mod util;

use std::{
    fmt,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_stdin::FileOrStdin;
use error::Error;
use graphql_normalize::normalize;
use input::{Input, Kind};

/// What every command's `--help` ends with: the shape of stdout, where
/// diagnostics go, and what the exit codes mean. `command` attaches it to each
/// command, since clap does not pass `after_help` down to subcommands.
///
/// Hard-wrapped, since clap only wraps help text with its `wrap_help` feature.
const OUTPUT_CONVENTIONS: &str = "\
Output is a GraphQL document on stdout ending in one newline, or nothing at all
when nothing survives. Errors, warnings, and notes go to stderr.

Exit codes: 0 on success, including a valid target that matches nothing; 1 for
a bad input (unreadable, invalid, unknown target); 2 for a usage error.";

/// Utilities for processing GraphQL query and schema documents.
#[derive(Parser, Debug)]
#[clap(version)]
struct Args {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Operate on a query document
    #[command(subcommand)]
    Query(QueryCommands),

    /// Operate on a schema document
    #[command(subcommand)]
    Schema(SchemaCommands),
}

#[derive(Subcommand, Debug)]
enum QueryCommands {
    /// Format and sort a query into a canonical form
    Normalize {
        /// Query to read. Defaults to stdin, so `normalize` can be piped into.
        #[arg(short, long, default_value = "-")]
        query: FileOrStdin,

        /// Print the query on a single line with no unnecessary whitespace
        #[arg(short, long, default_value_t = false)]
        minify: bool,
    },
    /// Strip a query down to the selections needed to reach the given targets
    ///
    /// Every path from an operation root to a matching type (`MyType`) or field
    /// (`MyType.field`) is kept, along with the full selection at the match.
    Focus {
        /// Schema the query is written against, used to resolve field types
        #[arg(short, long)]
        schema: PathBuf,

        /// Query to read. Defaults to stdin, so `focus` can be piped into.
        #[arg(short, long, default_value = "-")]
        query: FileOrStdin,

        /// Types (`MyType`) or fields (`MyType.field`) to focus on
        #[arg(required = true, num_args = 1.., value_name = "TYPE|TYPE.FIELD")]
        targets: Vec<String>,
    },
    /// Remove the given targets, and every reference to them, from a query
    ///
    /// The complement of `focus`: selections reaching a matching type
    /// (`MyType`) or field (`MyType.field`) are dropped, and the rest of the
    /// query is kept intact.
    ///
    /// Arguments and input fields typed with a matching type are removed too,
    /// but a required one (non-null, no default) is never removed on its own,
    /// so the output stays a valid query. A field that would lose a required
    /// argument is removed instead, and a directive that would lose one is
    /// dropped. A list element that cannot be kept is dropped from its list,
    /// and a list emptied that way is removed by the same rule. Variable
    /// default values are stripped the same way, and a variable whose default
    /// loses a required input field is removed.
    ///
    /// Fields the schema does not define, like `__typename`, are never matched
    /// against the targets, but they are removed if they pass a variable that
    /// was removed.
    Strip {
        /// Schema the query is written against, used to resolve field types
        #[arg(short, long)]
        schema: PathBuf,

        /// Query to read. Defaults to stdin, so `strip` can be piped into.
        #[arg(short, long, default_value = "-")]
        query: FileOrStdin,

        /// Types (`MyType`) or fields (`MyType.field`) to strip
        #[arg(required = true, num_args = 1.., value_name = "TYPE|TYPE.FIELD")]
        targets: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SchemaCommands {
    /// Reformat a schema, leaving its contents unchanged
    Format {
        /// Schema to read. Defaults to stdin, so `format` can be piped into.
        #[arg(short, long, default_value = "-")]
        schema: FileOrStdin,
    },
    /// Reduce a schema to the given types and everything they depend on
    Focus {
        /// Schema to read. Defaults to stdin, so `focus` can be piped into.
        #[arg(short, long, default_value = "-")]
        schema: FileOrStdin,

        /// Types to keep, along with all of their descendants
        #[arg(required = true, num_args = 1.., value_name = "TYPE")]
        types: Vec<String>,
    },
    /// Remove the types and fields a query does not use from a schema
    Prune {
        /// Schema to read. Defaults to stdin, so `prune` can be piped into.
        #[arg(short, long, default_value = "-")]
        schema: FileOrStdin,

        /// Query whose usage decides what the schema keeps. `-` reads it from
        /// stdin, in which case the schema has to be a file.
        #[arg(short, long)]
        query: FileOrStdin,
    },
    /// Sort schema definitions by category, then alphabetically by name
    ///
    /// Categories come in this order: the schema definition, directives, types,
    /// then extensions. All type kinds (object, interface, union, enum, input,
    /// scalar) are interleaved alphabetically within types. Extensions are
    /// sorted by the name of the type they extend, and several extensions of
    /// one type keep their source order. Fields keep their source order.
    Sort {
        /// Schema to read. Defaults to stdin, so `sort` can be piped into.
        #[arg(short, long, default_value = "-")]
        schema: FileOrStdin,
    },
}

fn main() -> ExitCode {
    let mut command = command();
    let args = Args::from_arg_matches(&command.get_matches_mut())
        .unwrap_or_else(|e| e.format(&mut command).exit());

    let mut warnings = Vec::new();
    let result = run(args, &mut warnings);

    // Warnings come first, and on failure too, since a problem they describe
    // can be what the command failed on, as when a target names a field only
    // an ignored duplicate definition has.
    for warning in warnings {
        diagnose("warning", warning);
    }
    match result.and_then(emit) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            diagnose("error", &error);
            error.exit_code()
        }
    }
}

/// The clap command for `Args`, with `OUTPUT_CONVENTIONS` after the help of it
/// and every subcommand below it. Applied here rather than per variant, so a
/// new subcommand gets it without having to remember to.
fn command() -> clap::Command {
    fn with_conventions(mut command: clap::Command) -> clap::Command {
        let names: Vec<String> = command
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_owned())
            .collect();
        for name in names {
            command = command.mut_subcommand(name, with_conventions);
        }
        command.after_help(OUTPUT_CONVENTIONS)
    }
    with_conventions(Args::command())
}

/// Runs the command `args` names, returning what it produced for `main` to
/// print. Warnings about the input documents, such as a name defined twice,
/// are added to `warnings` as each is parsed, so they reach stderr even when
/// the command then fails.
fn run(args: Args, warnings: &mut Vec<String>) -> Result<Output, Error> {
    let output = match args.cmd {
        Commands::Query(query_commands) => match query_commands {
            QueryCommands::Normalize { query, minify } => {
                let query = Input::read(query, Kind::Query)?;

                // `focus` and `strip` emit an empty document when nothing is
                // left; passing it straight through keeps them pipeable into
                // `normalize` even in that case.
                let document = if query.is_blank() {
                    String::new()
                } else {
                    let normalized =
                        normalize(query.text()).map_err(|e| query.parse_error(Kind::Query, e))?;
                    if minify {
                        // Minifying only tokenizes, and `normalize` just
                        // printed this from a parsed document, so it cannot
                        // fail.
                        graphql_parser::minify_query(normalized)
                            .expect("normalized query should tokenize")
                    } else {
                        normalized
                    }
                };
                document.into()
            }
            QueryCommands::Focus {
                schema,
                query,
                targets,
            } => {
                let schema = Input::from_path(&schema, Kind::Schema)?;
                let targets: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
                query_focus::process(
                    &schema,
                    &targets,
                    || Input::read(query, Kind::Query),
                    warnings,
                )?
            }
            QueryCommands::Strip {
                schema,
                query,
                targets,
            } => {
                let schema = Input::from_path(&schema, Kind::Schema)?;
                let targets: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
                query_strip::process(
                    &schema,
                    &targets,
                    || Input::read(query, Kind::Query),
                    warnings,
                )?
            }
        },
        Commands::Schema(schema_commands) => match schema_commands {
            SchemaCommands::Format { schema } => {
                let schema = Input::read(schema, Kind::Schema)?;
                // A blank schema has nothing to format, so it passes through
                // as empty. Formatting leaves the contents unchanged, so a
                // name defined twice keeps both.
                let formatted = schema.parse_schema_as_written()?.map(|doc| doc.to_string());
                formatted.unwrap_or_default().into()
            }
            SchemaCommands::Focus { schema, types } => {
                let types: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
                focus::process(&types, || Input::read(schema, Kind::Schema), warnings)?.into()
            }
            SchemaCommands::Prune { schema, query } => {
                // Stdin can be read only once, and `-s` defaults to it, so
                // `-q -` on its own asks for both. Caught before either is
                // read, so it cannot first wait on a pipe that never closes.
                if schema.is_stdin() && query.is_stdin() {
                    return Err(Error::StdinTwice);
                }
                let schema = Input::read(schema, Kind::Schema)?;
                let query = Input::read(query, Kind::Query)?;
                prune::process(&schema, &query, warnings)?
            }
            SchemaCommands::Sort { schema } => {
                let schema = Input::read(schema, Kind::Schema)?;
                sort::process(&schema)?.into()
            }
        },
    };

    Ok(output)
}

/// What a command produces: the document for stdout, and notes for stderr,
/// such as about targets that were valid but matched nothing.
///
/// A target that matches nothing is not an error, since a pipeline may
/// legitimately narrow a query to nothing, but an empty or unchanged document
/// on its own looks the same as success. The notes are returned rather than
/// printed so stdout stays a pure GraphQL document and `process()` stays
/// testable. A command with nothing to note returns its document as a
/// `String`, which converts into one with no notes.
///
/// Warnings about the input, such as a name defined twice, are not carried
/// here but added to the `warnings` a command is given, since they are found
/// while parsing and still matter when the command goes on to fail.
#[derive(Debug, Default)]
pub struct Output {
    pub document: String,
    pub notes: Vec<String>,
}

impl From<String> for Output {
    fn from(document: String) -> Self {
        Output {
            document,
            notes: Vec::new(),
        }
    }
}

/// Prints a command's document to stdout and its notes to stderr, so stdout
/// stays a pure GraphQL document for the next command in a pipe.
///
/// The document ends in exactly one newline, or is nothing at all when empty,
/// so a script can test stdout for empty. Commands do not agree on their own:
/// graphql-parser's `Display` ends in a newline and `minify_query` does not.
/// Trailing whitespace is never part of a GraphQL token, so trimming it here
/// cannot change the document.
///
/// A reader that stops early (`| head`) closes the pipe, and the rest of the
/// document has nowhere to go. That is how a filter is used, not a failure, so
/// like other Unix filters this stops quietly and succeeds. Rust ignores
/// SIGPIPE, which is why the write returns `BrokenPipe` rather than killing the
/// process, and why `println!`, which panics on it, is not used. Any other
/// failure to write is `Error::Write`.
fn emit(output: Output) -> Result<(), Error> {
    let document = output.document.trim_end();
    let written = if document.is_empty() {
        Ok(())
    } else {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{document}").and_then(|()| stdout.flush())
    };
    for note in output.notes {
        diagnose("note", note);
    }
    match written {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(Error::Write { source: e }),
        _ => Ok(()),
    }
}

/// Prints one `label: message` line to stderr. Unlike `eprintln!`, it does not
/// panic when stderr cannot be written, as when its reader has gone: there is
/// nowhere left to report that, so the line is dropped.
fn diagnose(label: &str, message: impl fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "{label}: {message}");
}
