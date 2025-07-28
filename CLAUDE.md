# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

GraphQL Document Utilities is a Rust CLI tool that provides utilities to process GraphQL queries and schema documents. The main features include:

- **Normalization**: Formats and sorts GraphQL queries for better readability
- **Pruning**: Removes unused types and fields from schemas based on queries
- **Focus**: Extracts only descendants of specified types from a schema
- **Sort**: Sorts all schema definitions alphabetically by category and name

## Architecture

The project uses a workspace structure with two main components:

### Main Binary (`src/`)
- `main.rs`: CLI interface using clap with subcommands for query and schema operations
- `focus.rs`: Schema focusing logic using petgraph for dependency graph traversal
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
cargo test prune              # Run prune-specific tests
```

### Format and Lint
```bash
cargo fmt                     # Format code
cargo clippy                  # Run linter
```

## CLI Usage Examples

```bash
# Normalize a GraphQL query
graphql-document-utils query normalize --path query.graphql

# Focus schema on specific types
graphql-document-utils schema focus --schema schema.graphql --type User Company

# Prune unused types and fields
graphql-document-utils schema prune --schema schema.graphql --query query.graphql

# Format schema
graphql-document-utils schema format --schema schema.graphql

# Sort schema definitions alphabetically
graphql-document-utils schema sort --schema schema.graphql
```

## Key Implementation Details

### Focus Feature
- Uses petgraph to build a dependency graph of GraphQL types
- Performs DFS traversal from specified root types to find all descendants
- Supports interfaces, unions, and nested type relationships

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