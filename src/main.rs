mod focus;
mod prune;
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
        #[clap(short, default_value = "-")]
        path: FileOrStdin,
        #[clap(short, long, default_value_t = false)]
        minify: bool,
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
}

fn main() {
    let args = Args::parse();

    match args.cmd {
        Commands::Query(query_commands) => match query_commands {
            QueryCommands::Normalize { path, minify } => {
                let query_content: String = path.contents().expect("Unable to read input");
                let normalized = normalize(&query_content).expect("Could not normalize");

                if minify {
                    let minified =
                        graphql_parser::minify_query(normalized).expect("Could not minify");

                    println!("{}", minified);
                } else {
                    println!("{}", normalized);
                }
            }
        },
        Commands::Schema(schema_commands) => match schema_commands {
            SchemaCommands::Format { schema } => {
                let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
                let schema_doc =
                    parse_schema::<String>(&schema_str).expect("Failed to parse schema");
                println!("{}", schema_doc);
            }
            SchemaCommands::Focus { schema, types } => {
                let schema_str = fs::read_to_string(&schema).expect("Failed to read schema file");
                let types: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
                let focused = focus::process(&schema_str, &types);

                println!("{}", focused);
            }
            SchemaCommands::Prune { schema, query } => {
                let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
                let query_str = fs::read_to_string(query).expect("Failed to read query file");
                let pruned = prune::process(&schema_str, &query_str);

                println!("{}", pruned);
            }
        },
    }
}
