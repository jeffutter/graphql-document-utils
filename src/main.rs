mod docs;
mod error;
mod focus;
mod input;
mod prune;
mod query_focus;
mod query_strip;
mod query_target;
mod skill;
mod sort;
mod subgraph;
mod supergraph;
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

/// The command line. Help text comes from `docs`, as it does for every
/// command below.
#[derive(Parser, Debug)]
#[command(version, about = docs::ABOUT, after_help = docs::COMMON_TASKS)]
struct Args {
    /// `None` for the tool run on its own, which prints its help.
    #[command(subcommand)]
    cmd: Option<Commands>,
}

/// The nouns, and `skill`. Each noun takes an optional verb, as a noun run on
/// its own prints its help, like the tool run on its own does.
#[derive(Subcommand, Debug)]
enum Commands {
    #[command(about = docs::QUERY_ABOUT)]
    Query {
        #[command(subcommand)]
        verb: Option<QueryCommands>,
    },

    #[command(about = docs::SCHEMA_ABOUT)]
    Schema {
        #[command(subcommand)]
        verb: Option<SchemaCommands>,
    },

    #[command(
        about = docs::SKILL_ABOUT,
        long_about = docs::SKILL_LONG_ABOUT,
        after_help = docs::SKILL_EXAMPLES,
    )]
    Skill,
}

/// The leaf commands and their arguments take their help text from `docs`,
/// where it can be read on its own, rather than from doc comments.
#[derive(Subcommand, Debug)]
enum QueryCommands {
    #[command(
        about = docs::QUERY_NORMALIZE_ABOUT,
        long_about = docs::QUERY_NORMALIZE_LONG_ABOUT,
        after_help = docs::QUERY_NORMALIZE_EXAMPLES,
    )]
    Normalize {
        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::QUERY_FROM_STDIN,
        )]
        query: FileOrStdin,

        #[arg(short, long, default_value_t = false, help = docs::QUERY_NORMALIZE_MINIFY)]
        minify: bool,
    },
    #[command(
        about = docs::QUERY_FOCUS_ABOUT,
        long_about = docs::QUERY_FOCUS_LONG_ABOUT,
        after_help = docs::QUERY_FOCUS_EXAMPLES,
    )]
    Focus {
        #[arg(short, long, help = docs::QUERY_SCHEMA)]
        schema: PathBuf,

        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::QUERY_FROM_STDIN,
        )]
        query: FileOrStdin,

        #[arg(
            required = true, num_args = 1.., value_name = "TYPE|TYPE.FIELD",
            help = docs::QUERY_FOCUS_TARGETS,
        )]
        targets: Vec<String>,
    },
    #[command(
        about = docs::QUERY_STRIP_ABOUT,
        long_about = docs::QUERY_STRIP_LONG_ABOUT,
        after_help = docs::QUERY_STRIP_EXAMPLES,
    )]
    Strip {
        #[arg(short, long, help = docs::QUERY_SCHEMA)]
        schema: PathBuf,

        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::QUERY_FROM_STDIN,
        )]
        query: FileOrStdin,

        #[arg(
            required = true, num_args = 1.., value_name = "TYPE|TYPE.FIELD",
            help = docs::QUERY_STRIP_TARGETS,
        )]
        targets: Vec<String>,
    },
}

/// The leaf commands and their arguments take their help text from `docs`,
/// where it can be read on its own, rather than from doc comments.
#[derive(Subcommand, Debug)]
enum SchemaCommands {
    #[command(
        about = docs::SCHEMA_FORMAT_ABOUT,
        long_about = docs::SCHEMA_FORMAT_LONG_ABOUT,
        after_help = docs::SCHEMA_FORMAT_EXAMPLES,
    )]
    Format {
        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::SCHEMA_FROM_STDIN,
        )]
        schema: FileOrStdin,
    },
    #[command(
        about = docs::SCHEMA_FOCUS_ABOUT,
        long_about = docs::SCHEMA_FOCUS_LONG_ABOUT,
        after_help = docs::SCHEMA_FOCUS_EXAMPLES,
    )]
    Focus {
        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::SCHEMA_FROM_STDIN,
        )]
        schema: FileOrStdin,

        #[arg(
            required = true, num_args = 1.., value_name = "TYPE",
            help = docs::SCHEMA_FOCUS_TYPES,
        )]
        types: Vec<String>,
    },
    #[command(
        about = docs::SCHEMA_PRUNE_ABOUT,
        long_about = docs::SCHEMA_PRUNE_LONG_ABOUT,
        after_help = docs::SCHEMA_PRUNE_EXAMPLES,
    )]
    Prune {
        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::SCHEMA_FROM_STDIN,
        )]
        schema: FileOrStdin,

        #[arg(short, long, help = docs::SCHEMA_PRUNE_QUERY)]
        query: FileOrStdin,
    },
    #[command(
        about = docs::SCHEMA_SORT_ABOUT,
        long_about = docs::SCHEMA_SORT_LONG_ABOUT,
        after_help = docs::SCHEMA_SORT_EXAMPLES,
    )]
    Sort {
        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::SCHEMA_FROM_STDIN,
        )]
        schema: FileOrStdin,
    },
    #[command(
        about = docs::SCHEMA_SPLIT_ABOUT,
        long_about = docs::SCHEMA_SPLIT_LONG_ABOUT,
        after_help = docs::SCHEMA_SPLIT_EXAMPLES,
    )]
    Split {
        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::SCHEMA_FROM_STDIN,
        )]
        schema: FileOrStdin,

        #[arg(short, long, value_name = "DIR", help = docs::SCHEMA_SPLIT_OUTPUT)]
        output: PathBuf,
    },
    #[command(
        about = docs::SCHEMA_SUBGRAPH_ABOUT,
        long_about = docs::SCHEMA_SUBGRAPH_LONG_ABOUT,
        after_help = docs::SCHEMA_SUBGRAPH_EXAMPLES,
    )]
    Subgraph {
        #[arg(
            short, long, default_value = "-", hide_default_value = true,
            help = docs::SCHEMA_FROM_STDIN,
        )]
        schema: FileOrStdin,

        #[arg(help = docs::SCHEMA_SUBGRAPH_NAME)]
        name: String,
    },
}

fn main() -> ExitCode {
    let mut command = command();
    let args = Args::from_arg_matches(&command.get_matches_mut())
        .unwrap_or_else(|e| e.format(&mut command).exit());

    let mut warnings = Vec::new();
    let result = run(args, &mut command, &mut warnings);

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

/// The clap command for `Args`, with `docs::OUTPUT_CONVENTIONS` ending the help
/// of it and of every noun and command below one, after the examples a leaf
/// has, or the common tasks at the top. Applied here rather than per variant,
/// since clap does not pass `after_help` down to subcommands, so a new
/// subcommand gets it without having to remember to. Both go in `after_help`,
/// which `-h` shows as well as `--help`.
///
/// A command outside the nouns, `skill`, does not get them: it prints a guide
/// to the tool rather than a GraphQL document, and reads nothing.
fn command() -> clap::Command {
    fn with_conventions(command: clap::Command) -> clap::Command {
        let after_help = match command.get_after_help() {
            Some(own) => format!("{own}\n\n{}", docs::OUTPUT_CONVENTIONS),
            None => docs::OUTPUT_CONVENTIONS.to_string(),
        };
        command.after_help(after_help)
    }
    fn with_conventions_throughout(command: clap::Command) -> clap::Command {
        with_conventions(command).mut_subcommands(with_conventions_throughout)
    }
    with_conventions(Args::command()).mut_subcommands(|command| {
        if command.has_subcommands() {
            with_conventions_throughout(command)
        } else {
            command
        }
    })
}

/// Runs the command `args` names, returning what it produced for `main` to
/// print. Warnings about the input documents, such as a name defined twice,
/// are added to `warnings` as each is parsed, so they reach stderr even when
/// the command then fails.
///
/// The tool or a noun run without a subcommand is asking what it can do, not
/// making a mistake, so it produces the help `-h` would print, taken from
/// `command`, the one `main` parsed with. As output it goes to stdout, exit 0,
/// through `emit` like any document, rather than clap's usage error on stderr.
fn run(
    args: Args,
    command: &mut clap::Command,
    warnings: &mut Vec<String>,
) -> Result<Output, Error> {
    let output = match args.cmd {
        None => help(command, &[]).into(),
        Some(Commands::Query { verb: None }) => help(command, &["query"]).into(),
        Some(Commands::Schema { verb: None }) => help(command, &["schema"]).into(),
        Some(Commands::Skill) => skill::render().into(),
        Some(Commands::Query {
            verb: Some(query_commands),
        }) => match query_commands {
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
        Some(Commands::Schema {
            verb: Some(schema_commands),
        }) => match schema_commands {
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
            SchemaCommands::Split { schema, output } => {
                let schema = Input::read(schema, Kind::Schema)?;
                subgraph::split(&schema, &output, warnings)?
            }
            SchemaCommands::Subgraph { schema, name } => {
                let schema = Input::read(schema, Kind::Schema)?;
                subgraph::process(&schema, &name, warnings)?
            }
        },
    };

    Ok(output)
}

/// The help `-h` prints for the command `path` names under `command`, the root
/// when `path` is empty. `--help` prints the same for the root and the nouns,
/// which have no long help.
fn help(command: &mut clap::Command, path: &[&str]) -> String {
    // Parsing gives only the root, and the subcommand it enters, a usage line
    // naming the full command (`graphql-document-utils query`); building
    // gives every subcommand one, as `-h` would show.
    command.build();
    let command = path.iter().fold(command, |command, name| {
        command
            .find_subcommand_mut(name)
            .expect("`path` names subcommands")
    });
    command.render_help().to_string()
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

#[cfg(test)]
mod tests {
    /// clap's own checks of the command definition, such as conflicting flag
    /// names, which otherwise surface only when the affected command runs.
    #[test]
    fn command_is_well_formed() {
        super::command().debug_assert();
    }
}
