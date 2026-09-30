# Contributing

For what the tool does and how to use it, see the [README](README.md), or run
`cargo run -- skill`. This file covers working on it.

## Development

You need Rust 1.85 or later, or Nix: `nix develop` gives a shell with the
toolchain, `cargo-release`, and `cargo-watch`.

```bash
cargo build                                # debug build
cargo build --release                      # target/release/graphql-document-utils
cargo run -- query normalize -q query.graphql
```

Before sending a change, run what CI runs:

```bash
cargo test --workspace                     # unit, CLI, and doc tests
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --workspace
```

A subset of the tests can be run by name, such as `cargo test query_strip`.

## Where the documentation lives

Usage is documented in four places, which tests keep in step with each other
and with what the tool does:

- `src/docs.rs` holds all of the CLI's help prose as constants: each command's
  one-line `about`, its `long_about`, its flag help, and its examples, plus the
  top-level "Common tasks" list. The clap attributes in `src/main.rs`
  reference these, so write help text there rather than in doc comments. Its
  tests parse every example and common task as a real command line.
- `src/skill.rs` builds what `graphql-document-utils skill` prints, from the
  same constants plus a few hand-written sections. Its exit-code list is built
  from `Error` in `src/error.rs`, so a new error appears in it. An `insta`
  snapshot of its output shows every change in review: accept one with
  `cargo insta review`, or `INSTA_UPDATE=always cargo test`.
- `README.md`. The tests in `src/docs.rs` parse every command in its `bash`
  blocks, check it shows every command, and check its "Common tasks" block
  matches the help's. Its Usage bullets and its "Results and exit codes"
  section are the skill's, word for word, so copy them from
  `cargo run -- skill` rather than editing them. It has no Rust snippets,
  since a binary crate runs no doc tests; it links to the library's README
  instead.
- `graphql-normalize-lib/README.md` is the library's crate documentation, so
  its Rust example runs as a doc test.

`tests/examples.rs` runs every example in the help, every common task, and
every command in the README against the files in `tests/fixtures`, one
pipeline stage at a time, and snapshots each one's exit codes, stderr, and
stdout under `tests/snapshots`. A redirect is not followed; its output is
captured instead. The README's sample schema and query must be those fixtures,
and each output it shows below a command must be what that command prints, so
a change in behavior fails the build until the README shows it. When a
snapshot changes, read the diff as the output a user will now get.

Each help constant and hand-written skill section that says how the tool
behaves has a `// Tested by:` comment naming the tests that check it, as
`module::test`; a test checks those tests exist. When you add or change a
claim, point it at a test that checks it, adding one if none does. The
README's per-command paragraphs are the one prose no test checks; reread them
when behavior changes.

## Architecture

The repository is a Cargo workspace of two crates, versioned independently.

The binary, in `src/`:

- `main.rs`: the clap CLI, and `emit`, which prints every command's output.
- `docs.rs`: the help prose (see above).
- `skill.rs`: the `skill` command's output.
- `input.rs`: reads a document from a file or stdin and parses it, dropping
  repeated definitions with a warning.
- `error.rs`: the errors every command reports, and their exit codes.
- `query_target.rs`: target validation and matching, shared by `query focus`
  and `query strip`, so both agree on what a target matches, and the
  did-you-mean suggestions `schema focus` uses too.
- `query_focus.rs`, `query_strip.rs`: the query commands.
- `focus.rs`: `schema focus`, a depth-first walk over a `petgraph` graph of
  the schema's types.
- `prune.rs`: `schema prune`, which finds the types and fields a query uses,
  then completes them until the result is valid.
- `sort.rs`: `schema sort`, a stable sort of definitions by category and name.
- `util.rs`: shared schema utilities, notably `merged_type_definitions`, the one
  place `extend` blocks are folded into their base types, and
  `retain_with_dependencies`, which adds what a set of kept types needs to
  stand alone and emits each base definition and extension trimmed to its own
  surviving members.

The library, in `graphql-normalize-lib/`, is `graphql-normalize` on crates.io,
the normalization behind `query normalize`. `query normalize --minify` is the
binary's, through `graphql_parser::minify_query`.

[CLAUDE.md](CLAUDE.md) has detailed design notes on each module.

## Releasing

Releases are cut with [cargo-release](https://github.com/crate-ci/cargo-release)
(in the Nix dev shell, or `cargo install cargo-release`), one crate at a time
from `main`:

```bash
cargo release patch                        # the binary: tags vX.Y.Z
cargo release patch -p graphql-normalize   # the library: tags graphql-normalize-vX.Y.Z
```

Both are a dry run until you add `--execute`. They run the test suite, bump the
version, commit, tag, and push. Pushing a `vX.Y.Z` tag triggers the CD
workflow, which builds the release binaries and attaches them to a GitHub
release. `graphql-normalize` also publishes to crates.io and updates its
`CHANGELOG.md`; the binary crate is not published (`publish = false` under
`[package.metadata.release]` in `Cargo.toml`).

The binary's tag is a bare `vX.Y.Z` because `.github/workflows/cd.yml` builds
only tags matching `[v]?X.Y.Z`, and the library's is prefixed so it does not
trigger that build. The settings live in `[workspace.metadata.release]` and in
each crate's `[package.metadata.release]`.
