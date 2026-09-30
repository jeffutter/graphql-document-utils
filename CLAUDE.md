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
- `main.rs`: CLI interface using clap with subcommands for query and schema
  operations, and `Output` (document plus stderr notes), which every command's
  result goes through `emit` as. `main` prints the warnings `run` collects
  first, then the output or the error. It parses arguments through `command()`,
  which attaches `OUTPUT_CONVENTIONS` as `after_help` to every command
  recursively, since clap does not inherit `after_help`
- `input.rs`: `Input`, a document's text plus its origin (path or stdin), which
  reads it, decides whether it is blank, and parses it as a schema or query,
  dropping repeated definitions with a warning
- `error.rs`: `Error`, the errors every command reports (read and parse
  failures, unknown targets, no document to read, a failed write to stdout),
  printed by `main` as one `error: ...` line with the status
  `Error::exit_code` picks
- `focus.rs`: Schema focusing logic using petgraph for dependency graph traversal
- `query_focus.rs`: Query focusing logic that keeps only the paths reaching given types/fields
- `query_strip.rs`: Query stripping logic that removes given types/fields and their references
- `query_target.rs`: Shared target validation and matching (`Matcher`) used by
  both query commands, `parse_schema` for the schema they resolve against, and
  the did-you-mean suggestions `schema focus` shares
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

# Every command reads the document it is named for from stdin when its flag is omitted, so they pipe
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

### Input
- One rule for the whole CLI: the document named by the noun reads stdin when
  its flag is omitted. Every `query` subcommand takes `-q`/`--query`, and every
  `schema` subcommand `-s`/`--schema`, as a `clap_stdin::FileOrStdin` defaulting
  to `-`, so the commands compose as filters
- The other document is a plain required flag: `--schema` on `query focus`/`strip`
  stays a path, and `schema prune` takes `-q` as a required `FileOrStdin`, so
  `-q -` reads the query from stdin when `-s` names a file
- Stdin holds one document (`clap-stdin` allows one stdin read per process), so
  `schema prune` with both flags on `-`, which `-q -` alone is, fails with
  `Error::StdinTwice`, exit 2, before either is read, rather than reading the
  schema and then failing on the query, or first waiting on an idle pipe
- A blank document is no document, one rule for every command: `Input::is_blank`
  holds when the text is only what GraphQL ignores between tokens (spaces, tabs,
  line breaks, commas, comments, a BOM; the characters graphql-parser skips),
  and `Input::parse_schema`/`parse_schema_as_written`/`parse_query` return
  `Ok(None)` for it, so every caller has to decide what that means. graphql-parser would reject it, but
  `query focus`/`strip` emit one when nothing survives, so it must pipe on
- The document a command transforms passes through blank as empty, with no
  note, since whatever emitted it already said why: `query normalize` (which
  branches on `is_blank`, as it normalizes text rather than a parsed document),
  `query focus`/`strip` (a blank query), `schema format`, `sort`, `focus`, and
  `prune` (a blank schema, whatever the query)
- `schema prune` with a blank query and a real schema emits an empty schema
  plus `note: the query is empty; output is empty`, not the smallest valid
  schema a mutations-only query gets: a blank query is no query, and a root
  field made up for it would hide that nothing reached the prune. Both documents
  are parsed before either is found blank, so a syntax error in one is still
  reported beside a blank other
- A query command's `--schema` is not transformed but resolved against, so
  `query_target::parse_schema` fails a blank one with `Error::BlankSchema`,
  exit 1 (``error: schema 'x.graphql' is empty; pass the schema the query is
  written against with -s``), before targets are checked and whether or not the
  query is blank. Read as a schema with no definitions it would report unknown
  types, but a built-in scalar target like `String` would validate and quietly
  match nothing
- `main` reads every document into an `input::Input` (`Input::read` for
  `FileOrStdin`, `Input::from_path` for the query commands' `--schema` path)
  and hands it to the command's
  `process()`, which parses it through `Input::parse_schema`/`parse_query`.
  `Input::read` goes through clap-stdin's reader rather than `contents()`,
  whose errors label a missing file as a stdin failure without its path, and
  keeps clap-stdin's guard against reading stdin twice
- `Input::read` refuses to read stdin when it is a terminal, since clap-stdin
  0.6 would wait on the keyboard forever when the flag was left off:
  `Error::NoInput`, exit 2, `error: no query given; pass -q FILE or pipe one on
  stdin`. The message names the document and its flag through `Kind::flag`
  (`-q` for a query, `-s` for a schema), so any `FileOrStdin` read through
  `Input::read` gets the check. An idle open pipe cannot be detected and is
  read as usual, as filter tools do
- `query focus`/`strip` take the query as a `read_query` closure, which
  `process()` calls only after every target has validated, since reading may
  block on stdin. clap-stdin 0.6's `FileOrStdin` does not read at parse time,
  only through `into_reader`, so a bad target fails before stdin is touched,
  and before the terminal check, so it wins over `Error::NoInput`
- `schema focus` takes its schema as a `read_schema` closure the same way. A
  root's form (a path, or a `Type.field`) needs no schema, so `focus::process`
  checks it before calling `read_schema` and a schema file passed positionally
  fails at once. Whether a root names a type needs the schema, so that check
  comes after the read, and with no schema piped in `Error::NoInput` wins over
  an unknown type
- Targets are required (`required = true, num_args = 1..`), so omitting them
  is a clap usage error, exit 2. The positional renders as `<TYPE|TYPE.FIELD>...`
  on the query commands and `<TYPE>...` on `schema focus`, in both `--help` and
  the usage line

### Errors
- Every `process()` returns a `Result` with `error::Error`; nothing panics on
  bad input. `main` returns `ExitCode`, prints `error: {error}` to stderr, and
  exits with `Error::exit_code`: 2 for a usage error, matching clap's own
  (`NoInput` and `StdinTwice`, where the command line needs fixing), 1 for
  everything else, where an input does
- Stdout is only ever a GraphQL document. `query focus`/`strip` and `schema
  prune` return an `Output` whose notes `main`'s `emit` prints to stderr as
  `note: ...`, exit 0; the other commands return `String`, which converts into
  an `Output` with no notes, so every command prints through `emit`
- Warnings are a third kind of diagnostic, for input the tool can work around
  but GraphQL deems invalid. They are found while parsing, before a command can
  fail, so they do not ride on `Output`: `main` hands `run` a
  `warnings: &mut Vec<String>` that reaches each `process()` and on into
  `Input::parse_schema`/`parse_query`, and prints them as `warning: ...` before
  the output or the error, exit code unchanged. A warning can explain the error
  after it (a target naming a field only a dropped duplicate has). Each
  document is parsed once per command, so each warning prints once
- A name defined more than once is a warning, not an error. `Input::parse_schema`
  keeps the first definition of each type (of any kind), directive, and `schema`
  block, and `Input::parse_query` the first of each fragment and named
  operation (one namespace across query, mutation, and subscription), dropping
  the rest before any command sees them, so every command agrees on first-wins
  and none prints a repeat: ``warning: type `User` is defined more than once in
  'schema.graphql' (at 3:1, 7:1); only the first is used``. Extensions are not
  definitions, so a base plus `extend` blocks, or `extend` blocks alone, never
  warn. Anonymous operations have no name and are not checked. `schema format`
  and `sort` parse through `Input::parse_schema_as_written` instead, and
  `query normalize` works on text, so all three keep repeats as written and
  say nothing, since they only lay a document out
- `emit` owns the shape of stdout, so no command has to: it trims the
  document's trailing whitespace and prints it with exactly one newline, or
  prints nothing at all when that leaves it empty. graphql-parser's `Display`
  ends in a newline and `minify_query` does not; neither matters. Every
  command's `-h`/`--help` states this, where diagnostics go, and the exit codes,
  from the one `OUTPUT_CONVENTIONS` constant `command()` applies to the whole
  tree, so a new subcommand gets it without a per-variant attribute
- Nothing prints with `println!`/`eprintln!`, which panic when the stream is
  closed. `emit` writes the document with `writeln!` on a locked stdout and
  flushes. Rust ignores SIGPIPE, so a reader that stops early (`| head -1`)
  shows up as `io::ErrorKind::BrokenPipe`, which `emit` treats as success,
  exit 0 with nothing on stderr, like other Unix filters. Any other write
  failure is `Error::Write`, exit 1. Diagnostics go through `diagnose`, which
  writes `label: message` to stderr and drops the line if stderr is gone
- `Error::Read` covers missing files, directories, and non-UTF-8 input, naming
  the document kind and origin (`'path'` or `(stdin)`), with std's
  ` (os error N)` suffix dropped:
  `error: cannot read schema 'nope.graphql': No such file or directory`
- `Error::Parse` covers syntax errors and graphql-parser's limits (integers
  past `i64`, nesting past its recursion limit of 50). graphql-parser's errors
  are opaque strings, so `Error::parse` recovers the position and joins the
  message lines with `; `, lowercased, with token kinds (`[Punctuator]`)
  dropped: ``error: failed to parse query (stdin) at 1:12: unexpected `}`;
  expected Name, : or )``. `normalize`'s boxed error goes through the same path
  via `Input::parse_error`
- Unknown targets fail with a suggestion, checked against the schema alone
  (with extensions merged) before the query is parsed, and even when the query
  is blank: `Error::UnknownType` (``unknown type `Usr` in 'schema.graphql'; did
  you mean `User`?``), `UnknownField` (`` `User` has no field `nmae`; did you
  mean `name`? ``), `NoFields` (a `Type.field` on an input, enum, or scalar),
  `InvalidTarget` (not `Type` or `Type.field`), and for `schema focus`,
  `FieldInSchemaFocus` and `BuiltInScalar`. A target that looks like a path
  (`query_target::looks_like_path`: contains `/`, or ends in `.graphql`/`.gql`)
  is most likely a document passed positionally, so `Error::PathAsTarget` wins
  over all of these, did-you-mean included, and names the right flag:
  `error: unknown target 'query.graphql'; pass query files with -q: -q
  query.graphql`, or for `schema focus`, `pass the schema file with -s: -s ...`.
  `Matcher::new` applies it only once a target has failed, since `Query.graphql`
  could be a real field; `schema focus` checks it first, as no type name can
  look like a path. `query_target::suggest` prefers a
  case-insensitive match (`user` -> `User`), else the closest by `strsim::jaro`
  above 0.8, ties broken alphabetically
- A valid target that matches nothing is not an error, so pipelines keep
  working: exit 0 with the empty or unchanged document plus a note. A blank
  query passed through gets no note (see Input)

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
- Every root must be a type the schema defines (extensions included), else exit
  1: an unknown one gets a suggestion, `Type.field` gets told to use
  `query focus`, and an undefined built-in scalar is rejected since there is no
  definition to keep. So a valid root always yields non-empty output, and the
  only empty output is from a blank schema, which passes through with its roots
  checked only for form, as there is nothing to look them up in
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
- `Matcher::new` validates targets (see Errors) and needs only the schema. A type
  is known if defined or a built-in scalar; a `Type.field` if the type or any
  subtype declares the field, since field targets match across the hierarchy
  (`Node.name`, `SearchResult.title` on a union)
- Notes targets that reach nothing: an input object type gets
  ``note: `SearchFilter` is an input type; `query focus` only matches output
  selections``, the rest are listed in ``note: no selection reaches `Profile`;
  output is empty`` (the suffix only when it is). With several targets and some
  output, each is re-run alone via `Matcher::only`, since one may be reached
  only inside another's match, which the walk keeps whole without descending

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
- Notes targets that match nothing: ``note: nothing in the query matches
  `Profile`; nothing was stripped`` (the suffix when no target matched). A target
  matched nothing when stripping it alone gives the same output as stripping no
  targets (`Matcher::only(None)`), which still drops unused fragments, so the
  output is compared against that rather than the input

### Prune Feature
- Analyzes GraphQL queries to determine which types and fields are actually used
- Handles fragments, inline fragments, and interface implementations
- Preserves schema structure while removing unused elements
- Returns an `Output`: a blank query yields an empty schema with a note, a blank
  schema an empty one without (see Input)
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
  directives). A name defined twice keeps its first definition, though commands
  never pass one, as `Input::parse_schema` drops repeats first. `Matcher`,
  focus, prune, and `retain_with_dependencies` all index types through it, so
  no command sees a type without its extensions
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
Tests are located in each module using `#[cfg(test)]`. `tests/cli.rs` covers
only what needs the binary (clap exit codes, which document each command reads
from stdin, failing before stdin is read, refusing a terminal stdin, blank
documents passing from one command to the next, and stdout's exact bytes: one
trailing newline, zero bytes for an empty result, and a second pass through
the binary changing nothing), every command's help ending with the output
conventions, a reader closing stdout early (a normalized query far larger than
a pipe buffer, of which the test reads a few bytes) exiting 0 with empty stderr,
and, on Linux only, a write to `/dev/full` failing with `Error::Write`. Commands
that should not read stdin run with an open stdin pipe or a pseudo-terminal, so
a read hangs and a timeout catches it; ones that should get a written, closed
pipe. The pseudo-terminal
tests skip with a stderr note where none can be opened (the macOS sandbox
agents run in denies them), except under `CI`, where they fail instead.
Tests use:
- `indoc`: For clean multi-line string literals in tests
- `pretty_assertions`: For better test failure output
- `rustix` (Unix only): Opens the pseudo-terminal in `tests/cli.rs`