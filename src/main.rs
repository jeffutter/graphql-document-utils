mod focus;
mod prune;
mod query_focus;
mod query_strip;
mod query_target;
mod sort;
mod util;

use std::{fs, path::PathBuf};

use clap::{Parser, Subcommand};
use clap_stdin::FileOrStdin;
use graphql_normalize::normalize;
use graphql_parser::parse_schema;

#[derive(Parser, Debug)]
#[clap(version)]
struct Args {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(subcommand)]
    Query(QueryCommands),
    #[command(subcommand)]
    Schema(SchemaCommands),
}

#[derive(Subcommand, Debug)]
enum QueryCommands {
    Normalize {
        /// Query to read. Defaults to stdin, so `normalize` can be piped into.
        #[arg(short, long, default_value = "-")]
        query: FileOrStdin,

        #[arg(short, long, default_value_t = false)]
        minify: bool,
    },
    /// Strip a query down to the selections needed to reach the given types
    /// (`MyType`) or fields (`MyType.field`).
    Focus {
        #[arg(short, long)]
        schema: PathBuf,

        /// Query to read. Defaults to stdin, so `focus` can be piped into.
        #[arg(short, long, default_value = "-")]
        query: FileOrStdin,

        #[arg(num_args = 1..)]
        targets: Vec<String>,
    },
    /// Remove every reference to the given types (`MyType`) or fields
    /// (`MyType.field`) from a query, keeping the rest intact.
    Strip {
        #[arg(short, long)]
        schema: PathBuf,

        /// Query to read. Defaults to stdin, so `strip` can be piped into.
        #[arg(short, long, default_value = "-")]
        query: FileOrStdin,

        #[arg(num_args = 1..)]
        targets: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SchemaCommands {
    Format {
        #[arg(short, long)]
        schema: PathBuf,
    },
    Focus {
        #[arg(short, long)]
        schema: PathBuf,

        #[arg(num_args = 1..)]
        types: Vec<String>,
    },
    Prune {
        #[arg(short, long)]
        schema: PathBuf,

        #[arg(short, long)]
        query: PathBuf,
    },
    Sort {
        #[arg(short, long)]
        schema: PathBuf,
    },
}

fn main() {
    let args = Args::parse();

    match args.cmd {
        Commands::Query(query_commands) => match query_commands {
            QueryCommands::Normalize { query, minify } => {
                let query_str: String = query.contents().expect("Failed to read query");

                // `focus` and `strip` emit an empty document when nothing is
                // left; passing it straight through keeps them pipeable into
                // `normalize` even in that case.
                if query_str.trim().is_empty() {
                    println!();
                    return;
                }

                let normalized = normalize(&query_str).expect("Could not normalize");

                if minify {
                    let minified =
                        graphql_parser::minify_query(normalized).expect("Could not minify");

                    println!("{minified}");
                } else {
                    println!("{normalized}");
                }
            }
            QueryCommands::Focus {
                schema,
                query,
                targets,
            } => {
                let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
                let query_str: String = query.contents().expect("Failed to read query");
                let targets: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
                let focused = query_focus::process(&schema_str, &query_str, &targets);

                println!("{focused}");
            }
            QueryCommands::Strip {
                schema,
                query,
                targets,
            } => {
                let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
                let query_str: String = query.contents().expect("Failed to read query");
                let targets: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
                let stripped = query_strip::process(&schema_str, &query_str, &targets);

                println!("{stripped}");
            }
        },
        Commands::Schema(schema_commands) => match schema_commands {
            SchemaCommands::Format { schema } => {
                let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
                let schema_doc =
                    parse_schema::<String>(&schema_str).expect("Failed to parse schema");
                println!("{schema_doc}");
            }
            SchemaCommands::Focus { schema, types } => {
                let schema_str = fs::read_to_string(&schema).expect("Failed to read schema file");
                let types: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
                let focused = focus::process(&schema_str, &types);

                println!("{focused}");
            }
            SchemaCommands::Prune { schema, query } => {
                let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
                let query_str = fs::read_to_string(query).expect("Failed to read query file");
                let pruned = prune::process(&schema_str, &query_str);

                println!("{pruned}");
            }
            SchemaCommands::Sort { schema } => {
                let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
                let sorted = sort::process(&schema_str);

                println!("{sorted}");
            }
        },
    }
}
