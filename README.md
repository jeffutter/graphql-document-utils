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
git clone https://github.com/jeffutter/graphql-document-utils.git
cd graphql-document-utils
cargo build --release
```

The binary will be available at `target/release/graphql-document-utils`.

### Development Setup

1. Ensure you have Rust installed (1.74 or later, the minimum `clap` requires)
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
- **Type extensions count.** Fields, arguments, and interfaces an `extend` block
  adds are resolved like the base type's, even when the base type is defined in
  another document (as in a federation subgraph that only writes
  `extend type Query`).

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
- **Type extensions count**, exactly as in `focus`.
- **Input positions are stripped too.** Arguments typed with a stripped type are
  deleted, along with the variable definitions that fed them. Object literals are
  stripped field by field, so `filter: { term: "x", status: ACTIVE }` loses just
  `status` when `Status` is stripped, if `status` is optional.
- **Required inputs are never removed on their own**, so the output stays a
  valid query. An input is required when it is non-null with no schema default.
  A field that would lose a required argument is removed instead (it cannot be
  called without it), and the removal cascades from there. A directive that
  would lose a required argument is dropped. A required input field that has to
  go takes its object literal, and the literal's own position then decides by
  the same rule. A list element that cannot stay is dropped and the rest keep
  their order, so `filters: [{ status: ACTIVE }, { term: "x" }]` becomes
  `filters: [{ term: "x" }]` when `Status` is stripped and `status` is required.
  A list emptied that way goes too, by the same rule, while one written as `[]`
  is left alone. Inputs the schema does not describe, including the arguments
  of the undeclared built-ins `@include` and `@skip`, count as required.
- **Variable defaults are stripped too**, against the variable's declared type,
  so `$f: SearchFilter = { term: "x", sort: DATE }` loses `sort` when `Sort` is
  stripped. A default that loses a required input field cannot stay, and
  dropping just the default would change what callers get when they omit the
  variable (a non-null one would become required, a nullable one null). So the
  variable is removed instead, exactly as if its type had been stripped, and
  every usage goes by the rules above.
- **Fields the schema does not define**, such as `__typename`, are never matched
  against the targets, and neither is anything they select until a type
  condition names a type again. Their arguments and directives are still
  stripped, so a field the schema does not know goes if it passes a removed
  variable, and no removed variable or fragment is left referenced.
- **Fragments are reduced in place.** A fragment defined on a stripped type is
  dropped whatever it selects; one that empties out, or that no surviving
  operation spreads any more, is dropped as well.
- **Operations left with nothing are dropped.** If everything is stripped, the
  output is empty; if nothing matches, the query comes back unchanged.

### Schema Commands

#### Format a Schema

Pretty-print a GraphQL schema with consistent formatting:

```bash
graphql-document-utils schema format --schema schema.graphql
```

#### Focus on Specific Types

Extract only the descendants of specified root types, creating a focused subset of your schema.
The types are positional arguments, not a flag:

```bash
graphql-document-utils schema focus --schema schema.graphql User
```

Multiple types:

```bash
graphql-document-utils schema focus --schema schema.graphql User Company Post
```

**Example:**
```graphql
# Input schema with User, Company, Post, Comment types
# Focus on User type
graphql-document-utils schema focus --schema schema.graphql User

# Output: Only User and its dependent types (Profile, Post, etc.)
```

The output is self-contained: every type and directive it names is defined in
it.

- **Descendants of each given type are kept whole**, recursively: the types its
  fields return, a union's members, and every type implementing an interface it
  reaches.
- **Any type the schema defines can be focused on**, including a scalar, enum,
  or input type that nothing references.
- **Dependencies of what is kept are kept too**, but not as roots: argument
  types and the input objects, scalars, and enums they reach, the interfaces a
  kept type implements, and the definitions of directives a kept definition
  uses. An interface kept only because a type implements it does not bring its
  other implementors.
- **The query root type is always kept**, since a schema without one is
  invalid. When the query never uses it, as in a document of only mutations or
  subscriptions, it keeps its smallest valid form (one field, chosen as below),
  and whatever that field returns keeps its smallest form too. The root is
  `schema { query: ... }` when given, otherwise `Query`, and one defined only by
  `extend type Query`, as in a federation subgraph, stays an `extend` block. A
  schema with no query root type is left without one.
- **The `schema` definition keeps only the root operation types that survive**,
  so it always keeps its `query` entry, and is dropped when none do.
- **`extend` blocks are part of their type.** Their fields, union members, and
  `implements` clauses are walked like the base definition's, and a kept type
  keeps its extensions whole and in place. An extension whose base type the
  document does not define stands in as the definition.

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

The output is self-contained: every type and directive it names is defined in
it.

- **Objects and interfaces keep only the fields the query selects**, on the type
  itself or on an interface it implements. Every object implementing an
  interface the query selects from is kept with that interface's fields, so
  every kept implementor still satisfies the interfaces it keeps.
- **Everything a kept field depends on is kept**: its return type, argument
  types, input objects reached through those arguments (whole and recursively),
  scalars, enums, and the interfaces a kept type implements.
- **Unions keep only the members the query selects through.**
- **Directive definitions are kept only when used**, by a kept definition or by
  the query itself.
- **The query root type is always kept**, since a schema without one is
  invalid. When the query never uses it, as in a document of only mutations or
  subscriptions, it keeps its smallest valid form (one field, chosen as below),
  and whatever that field returns keeps its smallest form too. The root is
  `schema { query: ... }` when given, otherwise `Query`, and one defined only by
  `extend type Query`, as in a federation subgraph, stays an `extend` block. A
  schema with no query root type is left without one.
- **The `schema` definition keeps only the root operation types that survive**,
  so it always keeps its `query` entry, and is dropped when none do.
- **A definition pruned down to empty is removed**, along with every reference
  to it. An interface none of whose fields the query selects is dropped, and
  kept types stop implementing it. A union left with no members, or an object
  left with no fields, is dropped too, and so are fields returning it and union
  memberships naming it, which can empty further types in turn. Definitions
  that were already empty in the source are left as written.
- **A type a kept field returns is never removed**, since the query or an
  implementor would no longer be valid against the output. That includes the
  narrower type an implementor declares for an interface field, as in
  `type Holder implements HasResult { result: User }` for the interface's
  `result: SearchResult`, which is kept (and stays a member of `SearchResult`)
  even when the query never enters `User`. When the query selects nothing on
  such a type, or nothing but `__typename`, as in `search { __typename }` or
  `... on Node { __typename }`, it keeps its smallest valid form: an object or
  interface keeps one field (preferring one that returns a scalar or enum and
  takes no arguments), and a union keeps its first member, which keeps one
  field of its own.
- **`extend` blocks are pruned like the type they extend.** Their fields,
  interfaces, and union members count as the type's, each block keeps only its
  own share of what survives, and a block left with nothing is dropped. When the
  base definition is left with no fields, the first surviving extension takes
  its place as the definition, with the base's description, interfaces, and
  directives, so `type Query` followed by `extend type Query { bots: [Bot] }`
  comes out as `type Query { bots: [Bot] }`.

#### Sort Schema Definitions

Organize schema definitions alphabetically by category and name:

```bash
graphql-document-utils schema sort --schema schema.graphql
```

**Categories sorted in order:**
1. Schema definition
2. Directive definitions (alphabetically by name)
3. Type definitions (alphabetically by name, with all kinds - object,
   interface, union, enum, input, scalar - interleaved rather than grouped)
4. Type extensions (alphabetically by the name of the type they extend;
   several extensions of one type keep their source order)

Fields, arguments, enum values, and union members keep their source order.

### Input/Output Options

Every command reads the document it is named for from stdin when that
document's flag is omitted: `query` subcommands read the query
(`-q`/`--query`), and `schema` subcommands read the schema (`-s`/`--schema`).
Output always goes to stdout, so the commands compose as filters:

```bash
# Read the query from stdin
cat query.graphql | graphql-document-utils query focus --schema schema.graphql Profile

# Read the schema from stdin
cat schema.graphql | graphql-document-utils schema sort

# Chain them together
cat query.graphql \
  | graphql-document-utils query strip --schema schema.graphql SearchFilter \
  | graphql-document-utils query focus --schema schema.graphql Profile \
  | graphql-document-utils query normalize --minify
```

Each flag still takes a file, and `-` names stdin explicitly. Stdin holds one
document, so a command that needs two takes the other as a file: `query focus`
and `query strip` always take `--schema` as a file, and `schema prune` always
needs `--query`. `schema prune --query -` reads the query from stdin instead,
which needs `--schema FILE`, since the schema would otherwise read stdin too:

```bash
cat query.graphql | graphql-document-utils schema prune --schema schema.graphql --query -
```

With nothing piped in, a command that would read stdin fails with a usage error
(exit 2) instead of waiting on the terminal.

Stdout is only ever a GraphQL document, ending in exactly one newline, or
nothing at all when nothing survives, so an empty result redirected to a file
leaves it zero bytes (`[ -s out.graphql ]` fails). Errors, warnings, and notes such as a target that matched
nothing, go to stderr. The exit code is 0 on success, including a valid target
that matches nothing; 1 for a bad input (unreadable, invalid, or an unknown
target); 2 for a usage error.

An empty document (nothing but whitespace, commas, and comments) passes
straight through every command as an empty document. `focus` and `strip`
produce one when nothing survives, so a pipeline that strips everything ends
quietly instead of failing to parse downstream. `schema prune` given an empty
query prints an empty schema and says so on stderr. The one exception is the
`--schema` of `query focus` and `query strip`: the query is resolved against
it, so an empty one is an error (exit 1).

A name defined more than once (a type, directive, `schema` block, fragment, or
named operation) is invalid GraphQL, but not an error here. `focus`, `strip`,
and `prune` use the first definition, leave the rest out of their output, and
print a warning naming the document and where each definition starts:

```
warning: type `User` is defined more than once in 'schema.graphql' (at 3:1, 7:1); only the first is used
```

`extend` blocks are not repeats. `schema format`, `schema sort`, and
`query normalize` only lay a document out, so they keep every definition as
written and say nothing.

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
- Merges `extend` blocks into their base types (`util::merged_type_definitions`)
  before building the graph, so what an extension adds is walked too
- Performs depth-first search (DFS) traversal from specified root types
- Handles complex relationships including interfaces, unions, and nested types
- Preserves schema validity by including all necessary dependencies (argument
  and input types, implemented interfaces, directive definitions, the trimmed
  `schema` definition) through `util::retain_with_dependencies`

### Query Focus Feature
- Resolves each query selection against the schema to know the type it lands on,
  with `extend` blocks merged into their base types
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
- Never removes a required input on its own: a field losing a required argument
  is removed instead, and a directive losing one is dropped
- Drops only the list elements that cannot be kept, in order; a list emptied
  that way is removed by the same rules, while a `[]` written in the query stays
- Strips variable default values too; a variable whose default loses a required
  input field is removed along with its usages, as if its type were stripped
- Cleans removed variables and fragments out of fields the schema does not
  define, without matching them against the targets
- Recomputes fragment reachability against the surviving tree, so a fragment
  reduced on a branch that was later dropped is not emitted

### Prune Feature
- Analyzes GraphQL queries to determine actual type and field usage
- Supports fragments, inline fragments, and interface implementations
- Maintains schema structure while removing unused elements
- Handles complex scenarios like union types and interface implementations
- Keeps every dependency of what survives (argument and input types, scalars,
  enums, implemented interfaces, union members, directive definitions) through
  `util::retain_with_dependencies`, so the output never names a dropped type
- Decides usage against each type with its `extend` blocks merged in, then emits
  the base definition and each extension trimmed to its own surviving members
- Removes any definition trimmed down to empty, and every reference to it,
  except types a kept field returns (including the narrower type an implementor
  declares for an interface field), which keep their smallest valid form so the
  query and every kept implementor stay valid against the output
- Promotes the first surviving extension to the definition when the base is
  trimmed to empty
- Always keeps the query root type, in its smallest valid form when the query
  never uses it, so a query document of only mutations or subscriptions still
  prunes to a valid schema

### Sort Feature
- Categorizes schema definitions (schema, directives, types, extensions)
- Sorts alphabetically by name within each category, interleaving all type
  kinds within types and keying each extension by the type it extends
- Uses a stable sort, so several extensions of one type keep their source order
- Leaves the members of each definition (fields, enum values, union members) in
  source order
- Uses index-based approach to work efficiently with the `graphql-parser` crate

### Dependencies

- **`graphql-parser`**: Core GraphQL parsing and AST manipulation
- **`petgraph`**: Graph data structures for type dependency analysis
- **`clap`**: Command-line argument parsing with derive macros
- **`clap-stdin`**: Seamless stdin/file input handling

## Examples

### Complete Workflow Example

```bash
# Start with a large schema and the queries your app sends, then:
# 1. Prune the types and fields those queries do not use
# 2. Focus on the types you care about, along with everything they depend on
# 3. Sort the result (sort output is already formatted, so no `schema format` step is needed)
graphql-document-utils schema prune --schema large-schema.graphql --query app-queries.graphql \
  | graphql-document-utils schema focus User Product \
  | graphql-document-utils schema sort > final-schema.graphql
```

Prune before focus: focusing on `User Product` first drops `Query`, leaving prune
nothing to match the queries against. Sort can go first or last, since prune and
focus both keep definitions in source order.

### Library Usage

The normalization functionality can be used as a library:

```rust
use graphql_normalize::normalize;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `normalize(&str) -> Result<String, Box<dyn std::error::Error>>`
    let normalized = normalize("query { user { name email } }")?;
    print!("{normalized}");
    Ok(())
}
```

The library only normalizes. Minification (`--minify`) is done by the CLI with
`graphql_parser::minify_query`.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests for new functionality
5. Ensure `cargo test`, `cargo fmt`, and `cargo clippy` all pass
6. Submit a pull request

## License

MIT. See [LICENSE](LICENSE).

## Author

Jeffery Utter <jeff@jeffutter.com>
