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
        #[arg(num_args = 1..)]
        targets: Vec<String>,
    },
    /// Remove the given targets, and every reference to them, from a query
    ///
    /// The complement of `focus`: selections reaching a matching type
    /// (`MyType`) or field (`MyType.field`) are dropped, and the rest of the
    /// query is kept intact.
    Strip {
        /// Schema the query is written against, used to resolve field types
        #[arg(short, long)]
        schema: PathBuf,

        /// Query to read. Defaults to stdin, so `strip` can be piped into.
        #[arg(short, long, default_value = "-")]
        query: FileOrStdin,

        /// Types (`MyType`) or fields (`MyType.field`) to strip
        #[arg(num_args = 1..)]
        targets: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SchemaCommands {
    /// Reformat a schema, leaving its contents unchanged
    Format {
        /// Schema to read
        #[arg(short, long)]
        schema: PathBuf,
    },
    /// Reduce a schema to the given types and everything they depend on
    Focus {
        /// Schema to read
        #[arg(short, long)]
        schema: PathBuf,

        /// Types to keep, along with all of their descendants
        #[arg(num_args = 1..)]
        types: Vec<String>,
    },
    /// Remove the types and fields a query does not use from a schema
    Prune {
        /// Schema to read
        #[arg(short, long)]
        schema: PathBuf,

        /// Query whose usage decides what the schema keeps
        #[arg(short, long)]
        query: PathBuf,
    },
    /// Sort schema definitions by category, then alphabetically by name
    Sort {
        /// Schema to read
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
