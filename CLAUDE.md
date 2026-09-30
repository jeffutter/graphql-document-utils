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
- `util.rs`: Shared utilities for GraphQL type manipulation, including
  `retain_with_dependencies` (the dependency closure schema commands share) and
  the `assert_self_contained` test helper

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
at a time from `main`. Dry run by default; add `--execute` to actually release.
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
graphql-document-utils schema focus --schema schema.graphql User Company

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
- Uses petgraph to build a dependency graph of GraphQL types. Every type the schema
  defines gets a node up front, whatever its kind, so any of them can be a root,
  including a scalar, enum, or input nothing references
- Performs DFS traversal from specified root types to find all descendants (field
  types, union members, interface implementors)
- Supports interfaces, unions, and nested type relationships
- Hands the descendants to `util::retain_with_dependencies`, which adds what they
  need to stand alone (argument and input types, implemented interfaces, directive
  definitions) and trims `schema {}` to surviving roots. These dependencies are not
  walked as roots, so an interface kept only via `implements` brings no implementors
- Builds the graph from `util::merged_type_definitions`, so extension fields, union
  members, and `implements` clauses are walked, and a type defined only by an
  extension can be a root. Kept types keep their extensions whole and in place
- Tests run every non-empty output through `util::assert_self_contained`

### Query Focus Feature
- Requires a schema, since a query alone does not say what type each field returns
- Retains every path from an operation root to a matching type or field, keeping the
  full sub-selection at the match so the output stays a valid query
- Matches subtypes: an interface/union target matches its implementors/members, and
  `Type.field` targets match in both directions across the interface hierarchy
- Prunes fragment definitions in place rather than inlining them; fragments spread
  inside a match are kept whole, fragments reaching nothing are dropped
- Drops unreferenced variable definitions and operations that reach no target
- `Matcher` resolves types through `util::merged_type_definitions`, so fields,
  arguments, and interfaces added by `extend` blocks are known like any other

### Query Strip Feature
- The complement of query focus, and shares its target matching via
  `query_target::Matcher`, so both agree on what a target is (including the
  subtype rules) and differ only in what they do with a match
- Removal cascades: an emptied selection set takes its parent field, inline
  fragment, or whole operation with it
- Strips input positions too: arguments and input object fields typed with a
  stripped type, and the variable definitions behind them
- Drops fragments defined on a stripped type outright; reduces the rest in place
  and recomputes reachability against the surviving tree
- Never matches fields the schema does not define (e.g. `__typename`) against the
  targets, nor anything below them until a type condition names a type, since
  what they return is unknown. Their inputs are still stripped, all counted as
  required, so a removed variable takes the unknown field that passes it, and
  spreads of fragments that did not survive go too
- Never removes a required input on its own, so the output stays valid. An input
  is required when it is non-null with no schema default; one the schema does not
  describe (an unknown argument, or any argument of an undeclared directive such as
  `@include`) counts as required. A required field argument that has to go takes
  its field with it and the removal cascades; a required directive argument takes
  the directive application; a required input field inside an object literal takes
  the literal, and its position then decides by the same rule. Optional inputs go
  on their own. A list element that cannot stay is dropped and the rest keep
  their order; a list emptied that way cannot stay either, and its position
  decides by the same rule. A list written as `[]` is left alone
- Strips variable default values the same way, against the variable's declared
  type. A default that only loses optional input fields keeps the rest. One that
  loses a required input field cannot stay, and dropping it would change what
  callers get when they omit the variable, so the variable is removed as if its
  type had been stripped and its usages cascade by the rules above. Variables are
  tracked by name across operations because fragments are shared

### Prune Feature
- Analyzes GraphQL queries to determine which types and fields are actually used
- Handles fragments, inline fragments, and interface implementations
- Preserves schema structure while removing unused elements
- Trims object/interface fields to what the query selects, then hands the entered
  types to `util::retain_with_dependencies`, which keeps everything the trimmed
  definitions name (field, argument and input types, implemented interfaces,
  union members, directive definitions) so the output never names a dropped type
- Keeps objects implementing an entered interface; trims unions to entered
  members; keeps directive definitions the query applies
- A definition trimmed down to empty is removed: `retain_with_dependencies`
  drops any object/interface left with no fields and union left with no members,
  then every field returning it, union member naming it, and `implements` clause
  naming it, cascading until nothing more empties. Definitions already empty in
  the source are left as written, which is why `schema focus` (identity trim) is
  unaffected
- Types a kept field returns are never removed, since the query and every kept
  implementor must stay valid against the output. `complete_used_fields` runs
  two rules to a fixpoint before trimming. First, every field kept on an entered
  type, or on an interface one implements (directly or transitively), enters its
  return type. That covers covariant narrowing: an implementor keeps every field
  its interfaces keep, and may declare it with a subtype (`result: User` for the
  interface's `result: SearchResult`, an object for an interface, under any
  wrappers), which the query may never enter. Entering a union member also
  keeps it in the union, as the narrowing requires. Second, an entered type left
  empty (only `__typename` selected, or entered only by the first rule) is
  treated as though the query had selected the least it needs: one field
  (leaf-typed, no arguments preferred) or a union's first member. Interfaces go
  before objects, so implementors inherit the interface's field instead of
  needing their own
- Prune tests assert, through `pruned`, that every kept implementor still has
  every field of the interfaces it keeps, returning a subtype of the interface
  field's type (`assert_implementors_complete`)
- Selecting into a type enters it, even with only `__typename`, so
  `node { __typename }` enters `Node` and keeps its implementors
- Always keeps the query root type (`RootTypes.query` from
  `util::detect_root_types`: `schema { query: X }`, else `Query`), since a schema
  without one is invalid. `process` enters it before `complete_used_fields`, so a
  query document of only mutations or subscriptions keeps it in its smallest
  valid form, and the type its one field returns in turn. A root defined only by
  `extend type Query` stays an `extend` block. A schema with no query root object
  type gets nothing added. `schema focus` does not do this, since its output is a
  subset view, not a servable schema
- Trims `schema {}` to the root types that survive, dropping it when none do;
  the query root always survives, so `query:` is always kept
- Decides usage against types with extensions merged in; `retain_with_dependencies`
  then emits each `extend` block trimmed to its own surviving members, dropping
  blocks left empty. A base definition trimmed to empty gives way to its first
  surviving extension, emitted as the definition with the base's description,
  interfaces, and directives (absorbing later extensions until it has members)
- Tests run every output through `util::assert_self_contained`,
  `util::assert_no_empty_definitions`, `assert_supports_query` (every type,
  field, and variable type the query names is defined in the output), and,
  when the input schema has a query root, `assert_has_query_root`. A test whose
  output extends a type defined elsewhere skips `pruned` for its
  self-containment check and runs the rest directly

### Type Extensions
- `util::merged_type_definitions` is the one place `extend` blocks are folded into
  their base types (fields, interfaces, union members, enum values, input fields,
  directives). `Matcher`, focus, prune, and `retain_with_dependencies` all index
  types through it, so no command sees a type without its extensions
- An extension whose base type is missing stands in as the definition (federation
  subgraphs often only `extend type Query`); one of a different kind than its base
  is invalid and ignored
- Schema output keeps the source layout rather than collapsing extensions into
  their base: `retain_with_dependencies` trims the merged type, then projects the
  result back onto the base definition and each extension by member name

### Sort Feature
- Sorts schema definitions by category (schema, directives, types, type extensions)
- Within each category, sorts alphabetically by name; all type kinds are
  interleaved within types, and extensions are keyed by the type they extend
- The sort is stable, so several extensions of one type keep their source order
- Fields and other members of a definition keep their source order
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