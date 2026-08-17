# GraphQL Document Utilities

A powerful Rust CLI tool that provides utilities to process, analyze, and optimize GraphQL queries and schema documents. This tool helps developers maintain clean, efficient GraphQL codebases by offering normalization, pruning, focusing, and sorting capabilities.

## Features

### Query Operations
- **Normalization**: Formats and sorts GraphQL queries for better readability and consistency
- **Minification**: Compact query representation for production use
- **Focus**: Strips a query down to just the selections needed to reach given types or fields
- **Strip**: Removes every reference to given types or fields from a query, keeping the rest

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
- `query_focus.rs`: Query focusing logic that keeps only the paths reaching given types or fields
- `query_strip.rs`: Query stripping logic that removes given types or fields and their references
- `query_target.rs`: Shared target matching for `query focus` and `query strip`
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

### Releasing

Releases are cut with [cargo-release](https://github.com/crate-ci/cargo-release)
(included in the Nix dev shell, or `cargo install cargo-release`). The two crates
are versioned independently, so release one at a time from `main`:

```bash
cargo release patch                                    # the binary: tags vX.Y.Z
cargo release patch -p graphql-normalize               # the library: tags graphql-normalize-vX.Y.Z
```

Both commands are a dry run until you add `--execute`. They run the test suite,
bump the version, commit, tag, and push; pushing a `vX.Y.Z` tag is what triggers
the CD workflow to build and attach the release binaries. `graphql-normalize`
also publishes to crates.io; the binary crate does not (flip `publish` under
`[package.metadata.release]` in `Cargo.toml` to change that).

## Usage

### Query Commands

#### Normalize a GraphQL Query

Format and sort a GraphQL query for better readability:

```bash
graphql-document-utils query normalize --query query.graphql
```

With minification:

```bash
graphql-document-utils query normalize --query query.graphql --minify
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

#### Focus a Query on Specific Types or Fields

Strip a query down to just the selections needed to reach the given targets. Each
target is either a type name (`Profile`) or a field on a type (`User.name`). A
schema is required, since resolving a query's field types depends on it.

```bash
graphql-document-utils query focus --schema schema.graphql --query query.graphql Profile
```

Omit `--query` to read the query from stdin:

```bash
cat query.graphql | graphql-document-utils query focus --schema schema.graphql Profile
```

Multiple targets, mixing types and fields:

```bash
graphql-document-utils query focus --schema schema.graphql --query query.graphql Profile Company.name
```

**Example:**
```graphql
# Input (query.graphql)
query GetUser($id: ID!, $first: Int) {
  user(id: $id) {
    name
    profile {
      email
      avatar {
        url
      }
    }
  }
  company {
    employees(first: $first) {
      name
    }
  }
}

# Output for target `Profile`
query GetUser($id: ID!) {
  user(id: $id) {
    profile {
      email
      avatar {
        url
      }
    }
  }
}
```

The result stays rooted at the same entrypoint operations as the original, and:

- **All paths are kept.** If a target is reachable more than one way, every route
  to it survives.
- **The match keeps its full sub-selection**, so the output is a valid query.
- **Subtypes count.** Targeting an interface or union matches its implementors and
  members, and targeting `Person.name` also matches a `name` selected inside
  `... on User`.
- **Fragments are pruned**, not inlined. Fragments that reach nothing are dropped;
  fragments spread inside a match are kept verbatim.
- **Unreferenced variable definitions and whole operations are dropped.** If no
  operation reaches a target, the output is empty.

#### Strip Types or Fields Out of a Query

The complement of `focus`: instead of keeping only what reaches the targets,
remove the targets and everything that references them, keeping the rest of the
query. Targets take the same form (`Profile`, `User.name`), and a schema is
required for the same reason.

```bash
graphql-document-utils query strip --schema schema.graphql --query query.graphql Profile
```

Omit `--query` to read the query from stdin:

```bash
cat query.graphql | graphql-document-utils query strip --schema schema.graphql Profile
```

Multiple targets, mixing types and fields:

```bash
graphql-document-utils query strip --schema schema.graphql --query query.graphql Profile Company.name
```

**Example:**
```graphql
# Input (query.graphql)
query GetUser($id: ID!, $filter: SearchFilter) {
  user(id: $id) {
    name
    profile {
      email
    }
  }
  search(filter: $filter) {
    id
  }
}

# Output for target `SearchFilter`
query GetUser($id: ID!) {
  user(id: $id) {
    name
    profile {
      email
    }
  }
  search {
    id
  }
}
```

Specifically:

- **Removal cascades.** A field or inline fragment whose selection set is emptied
  by the removal cannot be printed, so it is dropped too, and so on up the tree.
- **Subtypes count**, exactly as in `focus`. Stripping an interface or union also
  strips its implementors and members, and `Person.name` strips a `name` selected
  on `User` as well as the other way round.
- **Input positions are stripped too.** Arguments typed with a stripped type are
  deleted, along with the variable definitions that fed them, and any directive
  that depended on one of those variables.
- **Fragments are reduced in place.** A fragment defined on a stripped type is
  dropped whatever it selects; one that empties out, or that no surviving
  operation spreads any more, is dropped as well.
- **Operations left with nothing are dropped.** If everything is stripped, the
  output is empty; if nothing matches, the query comes back unchanged.

Note that `strip` does not check whether the result still satisfies the schema's
required arguments — removing an argument that was declared non-null will produce
a query the server rejects.

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

Every `query` subcommand reads its query from stdin when the query argument is
omitted, and always writes to stdout, so they compose as filters:

```bash
# Read the query from stdin
cat query.graphql | graphql-document-utils query focus --schema schema.graphql Profile

# Chain them together
cat query.graphql \
  | graphql-document-utils query strip --schema schema.graphql SearchFilter \
  | graphql-document-utils query focus --schema schema.graphql Profile \
  | graphql-document-utils query normalize --minify
```

`-q`/`--query` still takes a file, and `-` names stdin explicitly. Only the query
comes from stdin; `--schema` is always a file, since one stream cannot serve both.

An empty document passes straight through. `focus` and `strip` produce one when
nothing survives, so a pipeline that strips everything ends quietly instead of
failing to parse downstream.

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
cargo test query_focus         # Run query focus-specific tests
cargo test query_strip         # Run query strip-specific tests
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

### Query Focus Feature
- Resolves each query selection against the schema to know the type it lands on
- Retains every path from an operation root to a matching type or field
- Prunes fragment definitions in place rather than inlining them, tracking whether
  each fragment was kept whole or reduced
- Drops variable definitions the surviving selections no longer reference

### Query Strip Feature
- Shares its target matching with `query focus` via `query_target::Matcher`, so
  the two commands agree on what a target is and differ only in what they do with
  a match
- Cascades removal upward: an emptied selection set takes its parent field,
  inline fragment, or whole operation with it
- Strips input-position references too, deleting arguments typed with a stripped
  type and the variable definitions behind them
- Recomputes fragment reachability against the surviving tree, so a fragment
  reduced on a branch that was later dropped is not emitted

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
