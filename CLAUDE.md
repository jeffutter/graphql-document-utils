# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

GraphQL Document Utilities is a Rust workspace: the `graphql-document-utils`
CLI, which rewrites GraphQL query and schema documents, and `graphql-normalize`,
the library crate behind `query normalize`. Setup, the checks CI runs, where the
docs live, the module map, and releasing are in CONTRIBUTING.md, imported here:

@CONTRIBUTING.md

This file does not document how to use the tool. For current usage run
`cargo run -- skill`, which prints the guide agents get, built from the code,
or `cargo run -- <noun> <verb> --help`. What follows is what working on the code
needs beyond CONTRIBUTING.md: how the user-facing prose stays in step with the
code, and the design behind each module.

## User-facing prose

The CLI's help text lives in `src/docs.rs` as constants. The clap attributes in
`main.rs` reference them, and `skill.rs` builds the skill's reference for each
command from the same constants by walking `Args::command()`, so `--help` and
the skill cannot disagree. The README repeats commands, not rules, and tests
parse and run every command in it. The details:

- Each leaf's `long_about` opens with its `about`, since `--help` shows it in
  place of the `about`, then a paragraph summing up the command, which the skill
  shows after the `about` without the details that follow, so it must stand
  alone. Examples are an `Examples:` line, then commands indented two spaces
  (four on a `\` continuation). Prose is hard-wrapped to fit 80 columns, and
  commands are never wrapped. The flag for the document a command is named for
  uses the shared `QUERY_FROM_STDIN` or `SCHEMA_FROM_STDIN` wording and hides
  clap's `[default: -]`
- `skill.rs`'s `render` is YAML frontmatter (`name` is `CARGO_BIN_NAME`;
  `description` lists intents, as it is all an agent sees before loading the
  skill), a stamp of `CARGO_PKG_VERSION` saying when to regenerate it, the
  hand-written `STEPS` (how to run any command, as numbered steps), then a
  reference
  per verb: a synopsis spelling out every argument clap defines (optional ones
  bracketed, where clap's usage says `[OPTIONS]`), the `about` run into the
  `long_about`'s summary, and the examples, then `results()`: the hand-written
  `SUCCESS` bullets, and `exit_codes()`, a bullet per failing status built
  from `Error::one_of_each()` (grouped by `Error::status`, each error by its
  `Error::summary`) plus clap's own usage errors (`CLAP_ERRORS`) under the
  status clap exits with. A new `Error` variant has to be added to
  `one_of_each` (a test's exhaustive match fails until it is) and so appears in
  the list. `wrap` never breaks inside a backticked code span
- `docs.rs`'s tests check that every leaf has an about, long_about, and
  examples; that every example, common task, and command the tasks' footer
  quotes (such as `skill`) parses with `Args::try_parse_from`, through the
  `parse` helper; that each noun's `about` lists its verbs in order; that prose
  and rendered flag help fit 80 columns; and that every stdin flag uses the
  shared wording. They check `README.md` too: every command in its `bash`
  fences that invokes the binary (in any pipeline stage, `\` continuations
  joined) parses, the README invokes every leaf, and it holds `COMMON_TASKS` as
  one `bash` fence of `# intent` lines, each above its command
- `skill.rs`'s tests check that every leaf and long flag appears, that the
  frontmatter parses as YAML with the right `name`, that prose fits 80 columns,
  that no argument shows a default the synopsis would omit, and an `insta`
  snapshot of the whole output
  (`src/snapshots/graphql_document_utils__skill__tests__snapshot.snap`), with
  the version replaced by `[version]` so a release's version bump does not
  break it. They also check that the README's Usage steps are `STEPS`'s
  and its "Results and exit codes" section is `results()`, word for word, so
  those parts of the README are copied from `cargo run -- skill`, not edited
- `tests/examples.rs` runs every example of every leaf (walked from the
  binary's `-h`), every common task, and every other README command against
  `tests/fixtures`, and snapshots each with `insta`
  (`tests/snapshots/examples__*.snap`): each stage's exit code, stderr, and the
  last stage's stdout. Every one must exit 0 and print something. Pipelines run
  stage by stage, `cat` reads fixtures (a `*` globbed in sorted order), and a
  `>` ends the arguments, so the output is captured and nothing is written. It
  shares `src/docs/extract.rs` (included by `#[path]`) with `docs`' tests. A
  stage with `-o DIR` (`schema split`) gets a fresh `tempfile` directory in
  place of `DIR`, with that path cut from its stderr, and the transcript shows
  each file written (`wrote DIR/name:`, sorted) in place of an empty stdout,
  so "prints something" means prints or writes something and the fixtures
  directory is never written to. A
  stdout equal to the skill snapshot is written as a pointer to it; after a
  skill change accept that snapshot first. It also checks the README: a
  `graphql` fence introduced by a fixture's name (``this query,
  `query.graphql`:``) must be that fixture byte for byte, and a `graphql` fence
  right after a one-command `bash` fence must be exactly that command's stdout
- Behavioral claims are tied to tests. Each constant in `docs.rs` and
  `skill.rs` that states behavior has a `// Tested by:` comment above it,
  naming its tests as `module::test` (`cli` and `examples` for the integration
  tests, `graphql_normalize` for the library's).
  `docs::every_test_a_claim_names_exists` checks each named test is a function
  in that module's source, and `docs::every_long_about_names_its_tests` that no
  `long_about` lacks one. When
  adding a claim, name its test, or add one; when changing behavior, the
  snapshots show which examples change, and the comments which prose to reread.
  Facts the code decides are generated or asserted instead: the exit codes
  (`OUTPUT_CONVENTIONS` against `Error`, the skill's list from it), which flag
  reads stdin and which document is the other one, and the Rust version the
  README and CONTRIBUTING state against both crates' `rust-version`
- What no test checks is prose outside those constants: the README's
  per-command paragraphs and CONTRIBUTING. Read the snapshot diffs as the help
  a user will get

## CLI shell (`main.rs`)
- `main` parses through `command()`, which appends `docs::OUTPUT_CONVENTIONS`
  to the top level's `after_help` (after its common tasks) and to every noun's
  and verb's, recursively (after a leaf's examples). clap does not inherit
  `after_help`, so doing it here means a new subcommand gets it without a
  per-variant attribute. `skill`, the one command outside the nouns, is left
  out, as it prints Markdown rather than GraphQL
- `Args::cmd` is an `Option<Commands>` and each noun's verb an `Option` too, so
  the tool or a noun run on its own reaches `run`, which returns that command's
  `-h` help as its `Output`: stdout, exit 0, since asking what a command does is
  not a mistake. An unknown verb is still clap's usage error, exit 2
- Every command's result is an `Output` (document plus stderr notes) printed
  through `emit`, `skill::render()` included. `main` prints the warnings `run`
  collects first, then the output or the error

## Input
- The document a command is named for (`-q` on every `query` verb, `-s` on
  every `schema` verb) is a `clap_stdin::FileOrStdin` defaulting to `-`, so the
  commands compose as filters. The other document is a plain required flag:
  `--schema` on `query focus`/`strip` is a `PathBuf`, since it is resolved
  against rather than transformed, and `schema prune`'s `-q` is a required
  `FileOrStdin`, so `-q -` reads the query from stdin when `-s` names a file
- `main` reads every document into an `input::Input` (`Input::read` for a
  `FileOrStdin`, `Input::from_path` for a path) and hands it to the command's
  `process()`, which parses it through `Input::parse_schema`/`parse_query`.
  `Input::read` goes through clap-stdin's reader rather than `contents()`,
  whose errors label a missing file as a stdin failure without its path, and so
  keeps clap-stdin's guard against reading stdin twice
- `Input::read` refuses to read stdin when it is a terminal, since clap-stdin
  0.6 would wait on the keyboard forever when the flag was left off:
  `Error::NoInput`, exit 2, naming the document and its flag through
  `Kind::flag`. An idle open pipe cannot be detected and is read as usual, as
  filter tools do
- Stdin holds one document (clap-stdin allows one stdin read per process), so
  `schema prune` with both flags on `-`, which `-q -` alone is, fails with
  `Error::StdinTwice`, exit 2, before either is read, rather than reading the
  schema and then failing on the query, or first waiting on an idle pipe
- What can be checked without a document is checked before reading it, since
  reading may block on stdin. clap-stdin 0.6's `FileOrStdin` reads only
  through `into_reader`, not at parse time, so `query focus`/`strip` take the
  query as a `read_query` closure that `process()` calls only once every target
  has validated: a bad target fails before stdin is touched, and before the
  terminal check, so it wins over `Error::NoInput`. `schema focus` takes a
  `read_schema` closure the same way, but only a root's form (a path, or a
  `Type.field`) needs no schema, so a schema file passed positionally fails at
  once while an unknown type is found after the read, and with no schema piped
  in `Error::NoInput` wins over it
- A blank document is no document, one rule for every command:
  `Input::is_blank` holds when the text is only what graphql-parser skips
  between tokens (spaces, tabs, line breaks, commas, comments, a BOM), and
  `Input::parse_schema`/`parse_schema_as_written`/`parse_query` return
  `Ok(None)` for it, so no caller can forget to decide what that means.
  graphql-parser would reject it, but `query focus`/`strip` emit one when
  nothing survives, so it must pipe on
- The document a command transforms passes through blank as empty, with no
  note, since whatever emitted it already said why. `query normalize` branches
  on `is_blank` itself, as it normalizes text rather than a parsed document
- `schema prune` with a blank query and a real schema emits an empty schema
  plus a note, not the smallest valid schema a mutations-only query gets: a
  blank query is no query, and a root field made up for it would hide that
  nothing reached the prune. Both documents are parsed before either is found
  blank, so a syntax error in one is still reported beside a blank other
- A query command's `--schema` is not transformed but resolved against, so
  `query_target::parse_schema` fails a blank one with `Error::BlankSchema`,
  exit 1, before targets are checked and whether or not the query is blank.
  Read as a schema with no definitions it would report unknown types, but a
  built-in scalar target like `String` would validate and quietly match nothing
- The positional renders as `<TYPE|TYPE.FIELD>...` on the query commands and
  `<TYPE>...` on `schema focus` (`value_name`), since focus means a different
  thing under each noun

## Errors
- Every `process()` returns a `Result` with `error::Error`; nothing panics on
  bad input. `main` returns `ExitCode`, prints `error: {error}` to stderr, and
  exits with `Error::exit_code`: 2 for a usage error, matching clap's own
  (`NoInput` and `StdinTwice`, where the command line needs fixing), 1 for
  everything else, where an input does. `Error::is_usage` and `Error::status`
  decide it, and `Error::summary` names each variant's kind of failure for
  the skill's exit-code list; `Error::one_of_each` lists every variant for it
  and the tests that check the help against the code
- Stdout is only ever a GraphQL document, apart from help, `--version`, and
  `skill`. `schema split` is the one command that writes files, and prints
  nothing to stdout: a note per file written. `query focus`/`strip` and `schema prune` return an `Output` whose
  notes `emit` prints to stderr as `note: ...`; the other commands return
  `String`, which converts into an `Output` with no notes
- A valid target that matches nothing is not an error, so pipelines keep
  working: exit 0 with the empty or unchanged document plus a note. A blank
  query passed through gets no note (see Input)
- Warnings are a third kind of diagnostic, for input the tool can work around
  but GraphQL deems invalid. They are found while parsing, before a command can
  fail, so they do not ride on `Output`: `main` hands `run` a
  `warnings: &mut Vec<String>` that reaches each `process()` and on into
  `Input::parse_schema`/`parse_query`, and prints them as `warning: ...` before
  the output or the error, exit code unchanged. A warning can explain the error
  after it (a target naming a field only a dropped duplicate has). Each
  document is parsed once per command, so each warning prints once
- A name defined more than once is a warning, not an error. `Input::parse_schema`
  keeps the first definition of each type (of any kind), directive, and
  `schema` block, and `Input::parse_query` the first of each fragment and named
  operation (one namespace across query, mutation, and subscription), dropping
  the rest before any command sees them. Left in, commands would disagree: the
  schema commands resolve a type to its first definition but print every
  definition of a type they keep, and the query commands index fragments with
  the last one winning. Extensions are not definitions, so a base plus `extend`
  blocks, or `extend` blocks alone, never warn. Anonymous operations have no
  name and are not checked. `schema format` and `sort` parse through
  `Input::parse_schema_as_written` instead, and `query normalize` works on
  text, so all three keep repeats as written and say nothing, since they only
  lay a document out
- `emit` owns the shape of stdout, so no command has to: it trims the
  document's trailing whitespace and prints it with exactly one newline, or
  prints nothing at all when that leaves it empty. graphql-parser's `Display`
  ends in a newline and `minify_query` does not; neither matters
- Nothing prints with `println!`/`eprintln!`, which panic when the stream is
  closed. `emit` writes the document with `writeln!` on a locked stdout and
  flushes. Rust ignores SIGPIPE, so a reader that stops early (`| head -1`)
  shows up as `io::ErrorKind::BrokenPipe`, which `emit` treats as success,
  exit 0 with nothing on stderr, like other Unix filters. Any other write
  failure is `Error::Write`, exit 1. Diagnostics go through `diagnose`, which
  writes `label: message` to stderr and drops the line if stderr is gone
- `Error::Read` covers missing files, directories, and non-UTF-8 input, naming
  the document kind and origin (`'path'` or `(stdin)`), with std's
  ` (os error N)` suffix dropped
- `Error::Parse` covers syntax errors and graphql-parser's limits (integers
  past `i64`, nesting past its recursion limit of 50). graphql-parser's errors
  are opaque strings, so `Error::parse` recovers the position and joins the
  message lines with `; `, lowercased, with token kinds (`[Punctuator]`)
  dropped. `normalize`'s boxed error goes through the same path via
  `Input::parse_error`
- Targets are checked against the schema alone (with extensions merged) before
  the query is parsed, and even when the query is blank: `Error::UnknownType`,
  `UnknownField`, `NoFields` (a `Type.field` on an input, enum, or scalar),
  `InvalidTarget` (not `Type` or `Type.field`), and for `schema focus`,
  `FieldInSchemaFocus` and `BuiltInScalar`. A target that looks like a path
  (`query_target::looks_like_path`: contains `/`, or ends in `.graphql`/`.gql`)
  is most likely a document passed positionally, so `Error::PathAsTarget` wins
  over all of these, did-you-mean included, and names the right flag.
  `Matcher::new` applies it only once a target has failed, since
  `Query.graphql` could be a real field; `schema focus` checks it first, as no
  type name can look like a path
- `query_target::suggest` gives the did-you-mean: a case-insensitive match
  (`user` -> `User`) wins outright, else the closest by `strsim::jaro` above
  0.8, ties broken alphabetically. clap settles for 0.7, but a schema has far
  more names than a CLI has subcommands, and at 0.7 unrelated ones get
  suggested

## Key Implementation Details

### Focus Feature
- Uses petgraph to build a dependency graph of GraphQL types. Every type the schema
  defines gets a node up front, whatever its kind, so any of them can be a root,
  including a scalar, enum, or input nothing references
- Performs DFS traversal from the given roots to find all descendants (field
  types, input field types, union members, interface implementors)
- Hands the descendants to `util::retain_with_dependencies`, which adds what they
  need to stand alone (argument and input types, implemented interfaces, directive
  definitions) and trims `schema {}` to surviving roots. These dependencies are not
  walked as roots, so an interface kept only via `implements` brings no implementors
- Builds the graph from `util::merged_type_definitions`, so extension fields, union
  members, and `implements` clauses are walked, and a type defined only by an
  extension can be a root. Kept types keep their extensions whole and in place
- Every root must be a type the schema defines (extensions included), else exit
  1, and an undefined built-in scalar is rejected since there is no definition
  to keep. So a valid root always yields non-empty output, and the only empty
  output is from a blank schema, which passes through with its roots checked
  only for form, as there is nothing to look them up in
- Tests run every non-empty output through `util::assert_self_contained`

### Query Focus Feature
- Retains every path from an operation root to a matching type or field, keeping the
  full sub-selection at the match so the output stays a valid query
- Prunes fragment definitions in place rather than inlining them; fragments spread
  inside a match are kept whole, fragments reaching nothing are dropped
- Drops unreferenced variable definitions and operations that reach no target
- `Matcher` (in `query_target`) resolves types through
  `util::merged_type_definitions`, so fields, arguments, and interfaces added by
  `extend` blocks are known like any other. It matches subtypes: an
  interface/union target matches its implementors/members, and `Type.field`
  targets match in both directions across the interface hierarchy
- `Matcher::new` validates targets (see Errors) and needs only the schema. A type
  is known if defined or a built-in scalar; a `Type.field` if the type or any
  subtype declares the field, since field targets match across the hierarchy
  (`Node.name`, `SearchResult.title` on a union)
- Notes targets that reach nothing: an input object type gets a note of its own
  saying `query focus` only matches output selections, and the rest are listed
  in one note, which says the output is empty when it is. With several targets
  and some output, each is re-run alone via `Matcher::only`, since one may be
  reached only inside another's match, which the walk keeps whole without
  descending

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
- Notes targets that match nothing, and says nothing was stripped when no target
  matched. A target matched nothing when stripping it alone gives the same
  output as stripping no targets (`Matcher::only(None)`), which still drops
  unused fragments, so the output is compared against that rather than the input

### Prune Feature
- Returns an `Output`: a blank query yields an empty schema with a note, a blank
  schema an empty one without (see Input)
- Trims object/interface fields to what the query selects, then hands the entered
  types to `util::retain_with_dependencies`, which keeps everything the trimmed
  definitions name (field, argument and input types, implemented interfaces,
  union members, directive definitions) so the output never names a dropped type
- Keeps objects implementing an entered interface; trims unions to entered
  members; keeps directive definitions the query applies, and the types of the
  query's variables
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
  (leaf-typed, no arguments preferred) or a union's first object member. Unions
  go first, then interfaces (those implementing fewer interfaces first), then
  objects, so implementors inherit an interface's field instead of needing
  their own
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

### Supergraphs
- `supergraph.rs` is the one place that knows federation. `Supergraph::detect`
  finds a supergraph by its `schema` definition applying `@link` to the join
  spec (`https://specs.apollo.dev/join/vX.Y`), with no flag, and records each
  linked feature's name, version, `as:` rename, and `import:`s. The machinery
  is what those features define: `@link` and `link__*`, and per feature its
  `@prefix`, `@prefix__*`, `prefix__*`, and imported names
  (`is_machinery_type`, `is_machinery_directive`). Everything is written from
  the public link and join specs; no Apollo code is used or copied, and the
  `apollo-federation` crate (Elastic-2.0) is not a dependency
- Field sets (`key:`, `requires:`, `provides:`) are strings holding
  selections. `parse_field_set` wraps one in `{ }` and parses it as a query,
  so nested selections and inline fragments work. `field_sets` parses every
  one up front, and one that does not parse is `Error::MalformedSupergraph`
- `Supergraph::untracked` warns about what is recorded but not followed: a
  `@join__field(contextArguments:)` (`@fromContext`), whose selections may
  then be missing. Focus, prune, and extraction all print it
- `util::retain_with_dependencies` detects a supergraph itself, so focus and
  prune get the same rules without a parameter: every machinery definition is
  kept whole, and so are the `schema` block's directives; the type a
  `@join__field(type:)` names is a dependency; and `drop_dangling_relations`
  brings a trimmed type's join directives in line with it, dropping a
  `@join__implements`/`@join__unionMember` naming what it no longer has, and a
  `@join__type` for a graph none of its remaining fields or members are in
  (never the last, since a type with none is in every graph). The query root
  is exempt and keeps every `@join__type`, since every subgraph has a query
  root (if only for `_service`) and composition joins it to every graph
- `schema prune` adds two rules to `complete_used_fields`' fixpoint (see
  Prune Feature), before its last one since they may leave nothing empty.
  First, the router fetches what the join directives name, so each entered
  type uses its keys for every graph, and each kept field the fields its
  `requires` names on its type and its `provides` on the type it returns, fed
  through `collect_used_fields` as a query selection would be, entering what
  they reach. The type a kept field has in a graph (`@join__field(type:)`) is
  entered too. Second (`complete_graphs`), each subgraph must stay valid on
  its own: a kept field a graph resolves returns a type that graph must keep
  something of, so a type keeping no field of that graph selects the smallest
  it has (`util::smallest_field`), and a union no member of it, its first
- `schema focus` on a supergraph keeps the query root even when no root
  reaches it, since a router needs one: trimmed to the one field
  `util::smallest_field` picks (leaf-typed, no arguments preferred, the same
  choice prune's last rule makes), and walks from what that field returns
- `query_target::parse_schema` hands the query commands
  `Supergraph::api_schema`: the machinery, the `schema` block's `@link`s, and
  every machinery directive application are gone, and every `@inaccessible`
  type, field, argument, enum value, and input field is hidden. So targets,
  matching, and did-you-mean all see what clients see, and `join__Graph` is an
  unknown type
- Tests run every supergraph output of focus and prune through
  `util::assert_valid_supergraph`: self-contained, no empty definitions, a
  query root, every field-set name resolving, and every `@join__implements`/
  `@join__unionMember` matching the type. Whether it also composes needs
  Apollo's composition, a manual check (see CONTRIBUTING)

### Subgraph Extraction (`schema subgraph`, `schema split`)
- `subgraph.rs` projects the supergraph onto one graph, reading the join spec
  (v0.2 to v0.5, Federation 2). A Federation 1 supergraph (join v0.1, or
  `@core`) is `Error::MalformedSupergraph`, a schema that is not a supergraph
  `Error::NotASupergraph`, and a blank one passes through (only the name's
  form is checked, when splitting)
- A type is in a graph when it has `@join__type(graph:)` for it; a
  non-machinery type with none is in every graph. `key:` becomes `@key`
  (`resolvable: false` kept), `extension: true` an `extend type`, and
  `isInterfaceObject` an object with `@interfaceObject`. A field is in the
  graphs its `@join__field`s name, or every graph of its type when it has none
  (one with no `graph:` puts it in none). `external`, `requires`, `provides`,
  `override`/`overrideLabel`, and `type:` become `@external`, `@requires`,
  `@provides`, `@override(from:, label:)`, and the field's type there.
  `@join__implements`, `@join__unionMember`, and `@join__enumValue` give each
  graph its interfaces, members, and values; a type with none keeps all that
  are in the graph. `@join__directive(graphs:, name:, args:)` becomes
  `@name(args)` in those graphs
- What a supergraph does not record is rebuilt: `@shareable` goes on a field
  more than one graph resolves non-externally, or on the type when all its
  fields are; the directives composition carries (`CARRIED`: `@tag`,
  `@inaccessible`, `@authenticated`, and the rest) are renamed back from any
  `as:` and go on the element in every graph that has it. An `@external` field
  no key, `@requires`, or `@provides` of the graph names is dropped, and the
  cascade (`cascade`) removes what that empties. Any other directive of a
  linked spec is dropped with a warning
- The output opens with `@link(url: ".../federation/v2.N", import: [...])`
  importing exactly the federation directives it uses, at the lowest version
  defining all of them (`FEDERATION`: 2.3 for `@interfaceObject` or a key on
  an interface, 2.5 `@authenticated`/`@requiresScopes`, 2.6 `@policy`, 2.7 an
  `@override` label, 2.9 `@cost`/`@listSize`). The roots are those the graph
  keeps, named as the supergraph names them. With no roots it is
  `extend schema`, which graphql-parser can neither print nor parse, so it is
  printed as a `scalar` carrying the same directives and renamed. That output,
  like Apollo's own Federation 2 subgraphs that open with `extend schema`,
  cannot be read back by this tool
- Lost, as in any extraction: the root type names a subgraph used, argument
  differences between subgraphs, and directives composition did not keep
- `schema subgraph` fails an unknown name with `Error::UnknownSubgraph`, a
  did-you-mean from `query_target::suggest`, and every name there is. A graph
  left with no types prints nothing, with a note
- `schema split` writes `<dir>/<name>.graphql` per graph in `join__Graph`
  order, creating `dir`, replacing those files and touching no other, with a
  note per file (and per graph skipped as empty). Names come from the input
  and become paths, so before anything is extracted or written each must be a
  plain file name (`[A-Za-z0-9_-]`, which rules out `../` and absolute
  paths), and no two may differ only in case, as they would collide on a
  case-insensitive filesystem; either is `Error::MalformedSupergraph`. A write
  failure is `Error::WriteFile`, naming the path (`Write` stays about stdout)

### Sort Feature
- Sorts definitions by the key (category, name): schema definition, directives,
  types, type extensions, with all type kinds interleaved and extensions keyed
  by the type they extend. The sort is stable, so several extensions of one
  type keep their source order. Members of a definition are not reordered
- Sorts indices and clones definitions into place, to avoid lifetime issues
  with the graphql-parser crate

## Testing
- Tests live in each module under `#[cfg(test)]`, against its `process()`.
  `tests/cli.rs` covers only what needs the binary, and claims about every
  command at once; its module doc lists what. `tests/examples.rs` runs the
  documented commands (see User-facing prose)
- Commands that should not read stdin run with an open stdin pipe or a
  pseudo-terminal, so a read hangs and a timeout catches it; ones that should
  get a written, closed pipe. The pseudo-terminal tests skip with a stderr note
  where none can be opened, as in the macOS sandbox agents run in, and fail
  instead under `CI`, so a sandboxed pass has not run them. The `/dev/full`
  write-failure test runs on Linux only
- Test-only crates: `indoc` for multi-line literals, `insta` for the skill
  and example snapshots, `tempfile` for the directories `schema split` writes
  to, `serde_norway` to parse the skill's frontmatter (the maintained fork
  of the deprecated `serde_yaml`), and `rustix` (Unix only) to open the
  pseudo-terminal. `pretty_assertions` is used only by tests too
