# GraphQL Document Utilities

A powerful Rust CLI tool that provides utilities to process, analyze, and optimize GraphQL queries and schema documents. This tool helps developers maintain clean, efficient GraphQL codebases by offering normalization, pruning, focusing, and sorting capabilities.

## Features

### Query Operations
- **Normalization**: Formats and sorts GraphQL queries for better readability and consistency
- **Minification**: Compact query representation for production use

### Schema Operations
- **Pruning**: Intelligently removes unused types and fields from schemas based on query analysis
- **Focus**: Extracts only descendants of specified types, creating focused schema subsets
- **Format**: Pretty-prints GraphQL schemas with consistent formatting
- **Sort**: Organizes schema definitions alphabetically by category and name

## Architecture

This project uses a Rust workspace structure with two main components:

### Main Binary (`src/`)
- `main.rs`: CLI interface using clap with subcommands for query and schema operations
- `focus.rs`: Schema focusing logic using petgraph for dependency graph traversal
- `prune.rs`: Schema pruning logic that removes unused types and fields based on query analysis
- `sort.rs`: Schema sorting logic that organizes definitions by category and name
- `util.rs`: Shared utilities for GraphQL type manipulation

### Library (`graphql-normalize-lib/`)
- Separate crate for query normalization functionality
- Can be used as a standalone library in other Rust projects

## Installation

### Pre-built Binaries

Download pre-built binaries from the [GitHub releases page](https://github.com/jeffutter/graphql-document-utils/releases).

### From Source

Clone the repository and build locally:

```bash
git clone <repository-url>
cd graphql-document-utils
cargo build --release
```

The binary will be available at `target/release/graphql-document-utils`.

### Development Setup

1. Ensure you have Rust installed (1.70.0 or later recommended)
2. Clone the repository
3. Run `cargo build` to build the project
4. Run `cargo test` to run the test suite

## Usage

### Query Commands

#### Normalize a GraphQL Query

Format and sort a GraphQL query for better readability:

```bash
graphql-document-utils query normalize --path query.graphql
```

With minification:

```bash
graphql-document-utils query normalize --path query.graphql --minify
```

**Example:**
```graphql
# Input (query.graphql)
query GetUser($id: ID!) {
  user(id: $id) {
    name
    email
    posts {
      title
      content
    }
  }
}

# Output (normalized)
query GetUser($id: ID!) {
  user(id: $id) {
    email
    name
    posts {
      content
      title
    }
  }
}
```

### Schema Commands

#### Format a Schema

Pretty-print a GraphQL schema with consistent formatting:

```bash
graphql-document-utils schema format --schema schema.graphql
```

#### Focus on Specific Types

Extract only the descendants of specified root types, creating a focused subset of your schema:

```bash
graphql-document-utils schema focus --schema schema.graphql --type User
```

Multiple types:

```bash
graphql-document-utils schema focus --schema schema.graphql --type User Company Post
```

**Example:**
```graphql
# Input schema with User, Company, Post, Comment types
# Focus on User type
graphql-document-utils schema focus --schema schema.graphql --type User

# Output: Only User and its dependent types (Profile, Post, etc.)
```

#### Prune Unused Types and Fields

Remove unused types and fields from a schema based on actual query usage:

```bash
graphql-document-utils schema prune --schema schema.graphql --query query.graphql
```

**Example:**
```graphql
# Schema has User type with name, email, phone, address fields
# Query only uses name and email
# Result: User type will only contain name and email fields
```

#### Sort Schema Definitions

Organize schema definitions alphabetically by category and name:

```bash
graphql-document-utils schema sort --schema schema.graphql
```

**Categories sorted in order:**
1. Schema definitions
2. Directive definitions
3. Type definitions (alphabetically)
4. Type extensions (alphabetically)

### Input/Output Options

All commands support reading from stdin and writing to stdout:

```bash
# Read from stdin, write to stdout
cat schema.graphql | graphql-document-utils schema format

# Read from file, write to stdout
graphql-document-utils schema format --schema schema.graphql > formatted.graphql

# Specify output file
graphql-document-utils schema format --schema schema.graphql --output formatted.graphql
```

## Development

### Building

```bash
cargo build                    # Debug build
cargo build --release          # Release build
```

### Testing

```bash
cargo test                     # Run all tests
cargo test focus               # Run focus-specific tests
cargo test prune               # Run prune-specific tests
```

### Code Quality

```bash
cargo fmt                      # Format code
cargo clippy                   # Run linter
```

## Key Implementation Details

### Focus Feature
- Uses `petgraph` to build a dependency graph of GraphQL types
- Performs depth-first search (DFS) traversal from specified root types
- Handles complex relationships including interfaces, unions, and nested types
- Preserves schema validity by including all necessary dependencies

### Prune Feature
- Analyzes GraphQL queries to determine actual type and field usage
- Supports fragments, inline fragments, and interface implementations
- Maintains schema structure while removing unused elements
- Handles complex scenarios like union types and interface implementations

### Sort Feature
- Categorizes schema definitions (schema, directives, types, extensions)
- Sorts alphabetically within each category
- Uses index-based approach to work efficiently with the `graphql-parser` crate
- Preserves comments and formatting where possible

### Dependencies

- **`graphql-parser`**: Core GraphQL parsing and AST manipulation
- **`petgraph`**: Graph data structures for type dependency analysis
- **`clap`**: Command-line argument parsing with derive macros
- **`clap-stdin`**: Seamless stdin/file input handling

## Examples

### Complete Workflow Example

```bash
# 1. Start with a large schema and queries
# 2. Focus on specific types you care about
graphql-document-utils schema focus --schema large-schema.graphql --type User Product > focused-schema.graphql

# 3. Prune unused fields based on your actual queries
graphql-document-utils schema prune --schema focused-schema.graphql --query app-queries.graphql > pruned-schema.graphql

# 4. Sort and format the final schema
graphql-document-utils schema sort --schema pruned-schema.graphql | graphql-document-utils schema format > final-schema.graphql
```

### Library Usage

The normalization functionality can be used as a library:

```rust
use graphql_normalize::normalize_query;

let normalized = normalize_query(query_string, false)?;
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests for new functionality
5. Ensure `cargo test`, `cargo fmt`, and `cargo clippy` all pass
6. Submit a pull request

## License

[License information here]

## Author

Jeffery Utter <jeff@jeffutter.com>
