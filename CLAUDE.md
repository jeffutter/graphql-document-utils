# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

GraphQL Document Utilities is a Rust CLI tool that provides utilities to process GraphQL queries and schema documents. The main features include:

- **Normalization**: Formats and sorts GraphQL queries for better readability
- **Pruning**: Removes unused types and fields from schemas based on queries
- **Focus**: Extracts only descendants of specified types from a schema
- **Query Focus**: Strips a query down to the paths needed to reach given types or fields
- **Query Strip**: Removes given types or fields, and every reference to them, from a query
- **Sort**: Sorts all schema definitions alphabetically by category and name

## Architecture

The project uses a workspace structure with two main components:

### Main Binary (`src/`)
- `main.rs`: CLI interface using clap with subcommands for query and schema operations
- `focus.rs`: Schema focusing logic using petgraph for dependency graph traversal
- `query_focus.rs`: Query focusing logic that keeps only the paths reaching given types/fields
- `query_strip.rs`: Query stripping logic that removes given types/fields and their references
- `query_target.rs`: Shared target matching (`Matcher`) used by both query commands
- `prune.rs`: Schema pruning logic that removes unused types and fields based on query analysis
- `sort.rs`: Schema sorting logic that organizes definitions by category and name
- `util.rs`: Shared utilities for GraphQL type manipulation

### Library (`graphql-normalize-lib/`)
- Separate crate for query normalization functionality
- Can be used as a standalone library

## Development Commands

### Build and Run
```bash
cargo build
cargo run -- <subcommand> <args>
```

### Testing
```bash
cargo test                    # Run all tests
cargo test focus              # Run focus-specific tests
cargo test query_focus        # Run query focus-specific tests
cargo test query_strip        # Run query strip-specific tests
cargo test prune              # Run prune-specific tests
```

### Format and Lint
```bash
cargo fmt                     # Format code
cargo clippy                  # Run linter
```

### Release
Uses `cargo-release`; the two crates are versioned independently and released one
at a time from `main`. Dry run by default — add `--execute` to actually release.
```bash
cargo release patch                       # binary crate, tags vX.Y.Z (triggers the CD workflow)
cargo release patch -p graphql-normalize  # library crate, tags graphql-normalize-vX.Y.Z, publishes to crates.io
```
Config lives in `[workspace.metadata.release]` / `[package.metadata.release]` in
each `Cargo.toml`. The binary crate has `publish = false` (distributed as GitHub
release assets) and a bare `v{{version}}` tag, since `.github/workflows/cd.yml`
only builds tags matching `[v]?X.Y.Z`.

## CLI Usage Examples

```bash
# Normalize a GraphQL query
graphql-document-utils query normalize --query query.graphql

# Focus a query on the paths reaching a type or field
graphql-document-utils query focus --schema schema.graphql --query query.graphql Profile User.name

# Strip a type or field, and every reference to it, out of a query
graphql-document-utils query strip --schema schema.graphql --query query.graphql Profile User.name

# Every query subcommand reads stdin when its query argument is omitted, so they pipe
cat query.graphql \
  | graphql-document-utils query strip --schema schema.graphql SearchFilter \
  | graphql-document-utils query focus --schema schema.graphql Profile \
  | graphql-document-utils query normalize --minify

# Focus schema on specific types
graphql-document-utils schema focus --schema schema.graphql --type User Company

# Prune unused types and fields
graphql-document-utils schema prune --schema schema.graphql --query query.graphql

# Format schema
graphql-document-utils schema format --schema schema.graphql

# Sort schema definitions alphabetically
graphql-document-utils schema sort --schema schema.graphql
```

### Query Input
- Every `query` subcommand takes its query on `-q`/`--query` as a
  `clap_stdin::FileOrStdin` defaulting to `-`, so omitting the flag reads stdin and
  the commands compose as filters
- `--schema` stays a plain path: `clap-stdin` allows only one stdin read per process
- A blank query document is passed through as empty rather than parsed, since
  `focus` and `strip` both emit one when nothing survives

## Key Implementation Details

### Focus Feature
- Uses petgraph to build a dependency graph of GraphQL types
- Performs DFS traversal from specified root types to find all descendants
- Supports interfaces, unions, and nested type relationships

### Query Focus Feature
- Requires a schema, since a query alone does not say what type each field returns
- Retains every path from an operation root to a matching type or field, keeping the
  full sub-selection at the match so the output stays a valid query
- Matches subtypes: an interface/union target matches its implementors/members, and
  `Type.field` targets match in both directions across the interface hierarchy
- Prunes fragment definitions in place rather than inlining them; fragments spread
  inside a match are kept whole, fragments reaching nothing are dropped
- Drops unreferenced variable definitions and operations that reach no target

### Query Strip Feature
- The complement of query focus, and shares its target matching via
  `query_target::Matcher`, so both agree on what a target is (including the
  subtype rules) and differ only in what they do with a match
- Removal cascades: an emptied selection set takes its parent field, inline
  fragment, or whole operation with it
- Strips input positions too — arguments typed with a stripped type, the variable
  definitions behind them, and directives that depended on those variables
- Drops fragments defined on a stripped type outright; reduces the rest in place
  and recomputes reachability against the surviving tree
- Leaves fields the schema does not define (e.g. `__typename`) untouched, since
  their sub-selections cannot be resolved
- Does not verify that the result still satisfies required arguments

### Prune Feature
- Analyzes GraphQL queries to determine which types and fields are actually used
- Handles fragments, inline fragments, and interface implementations
- Preserves schema structure while removing unused elements

### Sort Feature
- Sorts schema definitions by category (schema, directives, types, type extensions)
- Within each category, sorts alphabetically by name
- Uses an index-based approach to avoid lifetime issues with the graphql-parser crate

### Dependencies
- `graphql-parser`: Core GraphQL parsing functionality
- `petgraph`: Graph data structure for focus feature
- `clap`: CLI argument parsing
- `clap-stdin`: Support for reading from stdin or files

## Testing
Tests are located in each module using `#[cfg(test)]` and use:
- `indoc`: For clean multi-line string literals in tests
- `pretty_assertions`: For better test failure output