//! The prose of the CLI's help: the tool's `about` and the common tasks its
//! help lists, each noun's `about`, each leaf command's one-line `about`, the
//! `long_about` that `--help` shows in its place, the help of its flags and
//! arguments, the examples both `-h` and `--help` end with, and the output
//! conventions the help of every command that prints GraphQL closes on.
//!
//! The clap attributes on `Args`, `Commands`, `QueryCommands`, and
//! `SchemaCommands` reference these rather than carrying doc comments, so the
//! text lives in one place, which `skill` builds its reference for each
//! command from without drifting from `--help`.
//!
//! Everything is hard-wrapped, since clap only wraps help text with its
//! `wrap_help` feature: prose at 80 columns, and flag and argument help at 55,
//! as clap indents it by up to 25 in `-h` (10 in `--help`). Each `long_about`
//! opens with its `about`, as `--help` shows it instead of the `about`, not
//! beside it, followed by a paragraph that sums up what the command does,
//! which the skill shows under the `about` without the details after it, so it
//! has to make sense without them. Each examples block is an `Examples:` line
//! followed by commands indented two spaces, which continue onto lines indented
//! four when they end in `\`; the tests below parse every one of them against
//! the CLI, and every command in `COMMON_TASKS` too. Commands are not wrapped,
//! so they can be copied whole.
//!
//! The README is checked by the same tests: every command in its `bash` blocks
//! parses, it invokes every command there is, and it lists `COMMON_TASKS` as
//! they are, so its examples cannot drift from the CLI either. Parsing only
//! shows a command is well formed; `tests/examples.rs` runs every one, here
//! and in the README, against `tests/fixtures`, and snapshots what it does.
//!
//! What the prose says a command does is true by test, not by care: the
//! `// Tested by:` comment above each constant that makes a claim names the
//! tests that check it, `module::test`, which the tests below check exist, and
//! every `long_about` has one. The facts code decides are not written out but
//! checked against it, such as the exit codes against `Error`.

/// The tool's one-line description, above the command list of its help.
pub const ABOUT: &str = "Utilities for processing GraphQL query and schema documents";

/// Each noun's `about`, which the top-level command list shows. It names the
/// noun's verbs, so that list shows every command there is.
pub const QUERY_ABOUT: &str = "Operate on a query document (normalize, focus, strip)";
pub const SCHEMA_ABOUT: &str =
    "Operate on a schema document (format, focus, prune, sort, split, subgraph)";

/// What the top-level help ends with, before the output conventions: what
/// the tool is for, as the command that does each thing, and where to read
/// more. Each intent is indented two spaces, with its command whole on the
/// next line, indented four, so it can be copied as it is.
//
// Tested by: every command parses in `docs::every_task_parses` and runs in
// `examples::every_common_task_runs`; the footer's in
// `docs::every_command_the_tasks_point_at_parses`.
pub const COMMON_TASKS: &str = "\
Common tasks:
  Canonicalize a query for diffing
    graphql-document-utils query normalize -q query.graphql
  Keep only the parts of a query that reach a type or field
    graphql-document-utils query focus -s schema.graphql -q query.graphql User.email
  Remove a type or field, and everything that references it, from a query
    graphql-document-utils query strip -s schema.graphql -q query.graphql Profile
  Cut a schema down to what a query uses
    graphql-document-utils schema prune -s schema.graphql -q query.graphql
  Extract a type and everything it depends on from a schema
    graphql-document-utils schema focus -s schema.graphql User
  Sort and format a schema
    graphql-document-utils schema sort -s schema.graphql
  Split a supergraph into the schemas of its subgraphs
    graphql-document-utils schema split -s supergraph.graphql -o subgraphs
  Extract one subgraph's schema from a supergraph
    graphql-document-utils schema subgraph -s supergraph.graphql reviews

Run 'graphql-document-utils <noun> <verb> --help' for details, or
'graphql-document-utils skill' for a complete guide for agents.";

/// What every command's help ends with: the shape of stdout, where
/// diagnostics go, and what the exit codes mean. `command` in `main` attaches
/// it to each command, after its examples or common tasks.
//
// Tested by:
// - one newline, or nothing: `cli::a_document_ends_in_exactly_one_newline`,
//   `cli::an_empty_result_prints_nothing`
// - stderr: `cli::a_repeated_definition_warns_and_the_first_is_used`,
//   `cli::a_target_that_matches_nothing_gets_a_note`, `cli::bad_input_exits_1`
// - exit codes: `docs::the_output_conventions_give_the_exit_codes`, which
//   checks them against `Error`, and `error::usage_errors_exit_as_clap_does`
pub const OUTPUT_CONVENTIONS: &str = "\
Output is a GraphQL document on stdout ending in one newline, or nothing at all
when nothing survives. Errors, warnings, and notes go to stderr.

Exit codes: 0 on success, including a valid target that matches nothing; 1 for
a bad input (unreadable, invalid, unknown target) or a failed write; 2 for a
usage error.";

/// The help of the flag for the document a command is named for, `-q` on each
/// `query` command and `-s` on each `schema` command, which reads stdin when
/// omitted. One wording, so the rule reads the same wherever it applies. The
/// flags hide clap's `[default: -]`, which says the same less plainly.
//
// Tested by: `docs::every_stdin_flag_says_so_in_one_wording`,
// `cli::a_schema_piped_in_is_read_as_if_from_its_file`,
// `cli::prune_reads_either_document_from_stdin`
pub const QUERY_FROM_STDIN: &str = "Query file. Reads stdin when omitted.";
pub const SCHEMA_FROM_STDIN: &str = "Schema file. Reads stdin when omitted.";

/// The `-s` of `query focus` and `query strip`, a path that never reads stdin,
/// as the schema is not what they transform but what they resolve against.
//
// Tested by: `docs::the_other_document_is_a_required_flag`,
// `cli::missing_required_arguments_are_a_usage_error`
pub const QUERY_SCHEMA: &str = "\
Schema file the query is written against. Required,
since a query alone does not say what type each field
returns.";

pub const QUERY_NORMALIZE_ABOUT: &str = "Format and sort a query into a canonical form";

// Tested by, where `graphql_normalize` is the library's tests:
// - same text whatever the order, and each order it describes:
//   `graphql_normalize::orders_operations_by_type_then_name_then_fragments_by_name`,
//   `graphql_normalize::orders_fields_then_spreads_then_inline_fragments`,
//   `graphql_normalize::sorts_arguments_and_directives_by_name_regardless_of_case`,
//   and the library README's doc test (variable definitions)
// - input object fields: `graphql_normalize::normalizes_field_argument_values_like_directive_arguments`
// - lists: `graphql_normalize::preserves_enum_and_string_list_order`
// - no schema: `cli::a_document_ends_in_exactly_one_newline`
// - comments: `graphql_normalize::drops_comments`,
//   `cli::comments_are_dropped_and_descriptions_kept`
// - blank: `cli::an_empty_result_prints_nothing`,
//   `cli::an_empty_strip_pipes_into_normalize`
pub const QUERY_NORMALIZE_LONG_ABOUT: &str = "\
Format and sort a query into a canonical form

Layout does not change the output. Neither does the order of definitions,
selections, arguments, and directives with different names. Use the output to
diff, hash, or deduplicate queries. No schema is needed.

Operations come first, queries then mutations then subscriptions, each sorted by
name, followed by fragments sorted by name. In a selection set, fields are
sorted by field name (not alias), followed by fragment spreads, then inline
fragments by type condition. Arguments, directives, and variable definitions
are sorted by name, and input object fields by key. Names compare without
regard to case, apart from input object fields, which sort as written, so
uppercase first. Two that compare the same, such as one field selected under
two aliases, keep their order. List values keep their order too, since it can
be meaningful.

Nothing is added or removed, but `#` comments are dropped. A blank query passes
through as empty, so `query focus` or `query strip` output that nothing
survived can be piped in.";

// Tested by: `cli::a_document_ends_in_exactly_one_newline`
pub const QUERY_NORMALIZE_MINIFY: &str = "\
Print the query on a single line with no unnecessary
whitespace";

pub const QUERY_NORMALIZE_EXAMPLES: &str = "\
Examples:
  graphql-document-utils query normalize -q query.graphql
  cat query.graphql | graphql-document-utils query normalize -m";

pub const QUERY_FOCUS_ABOUT: &str =
    "Keep only the parts of a query that reach the given types or fields";

// Tested by:
// - paths and whole matches: `query_focus::keeps_every_path_to_the_target`,
//   `query_focus::keeps_path_and_subtree_for_a_type_target`, and the output
//   parses again in `cli::a_second_pass_changes_nothing`
// - fragments: `query_focus::prunes_fragments_and_drops_unreached_ones`
// - targets: `query_focus::keeps_only_the_targeted_field`,
//   `query_focus::rejects_unknown_targets`
// - variables and operations: `query_focus::drops_variables_that_are_no_longer_referenced`,
//   `query_focus::drops_operations_that_reach_nothing`
// - subtypes: `query_focus::resolves_fields_and_interfaces_added_by_extensions`,
//   `query_focus::a_union_target_matches_its_members`,
//   `query_focus::matches_interface_field_on_implementing_type`,
//   `query_focus::a_field_target_on_an_implementor_matches_it_selected_on_the_interface`
// - input types: `query_focus::notes_an_input_type_target`
// - `__typename`: `query_focus::never_matches_fields_the_schema_does_not_define`
// - unknown targets: `query_target::rejects_an_unknown_type_naming_the_schema`,
//   `query_target::rejects_an_unknown_field_with_a_suggestion`
// - nothing reached: `query_focus::notes_a_valid_target_that_nothing_reaches`,
//   `cli::a_target_that_matches_nothing_gets_a_note`
// - blank: `query_focus::passes_an_empty_document_through`
// - supergraphs: `query_target::a_supergraph_resolves_targets_against_its_api_schema`
pub const QUERY_FOCUS_LONG_ABOUT: &str = "\
Keep only the parts of a query that reach the given types or fields

Every path from an operation root to a selection matching a target stays,
plus the whole selection at the match. The output stays a valid query.
Fragments stay fragments, trimmed in place.

A target is a type (`Profile`) or a field on a type (`User.email`), named as
the schema names them. Fragments, variable definitions, and operations that no
longer reach a target are dropped.

Unlike `schema focus`, which takes types only and reduces a schema to them and
everything they depend on, this reduces a query to the paths reaching its
targets.

A type target matches every selection that returns the type. Subtypes match
too: an interface or union target matches its implementors or members, and a
field target matches across the interface hierarchy in both directions, so
`Person.name` matches `name` selected on a `User` that implements `Person`, and
`User.name` matches `name` selected on the `Person` interface.

Only output selections match. No selection returns an input type, so an input
type target matches nothing here; `query strip` removes the arguments typed
with one. Fields the schema does not define, such as `__typename`, are never
matched against the targets.

Every target must be a type or field the schema knows, or the command fails,
with a did-you-mean when a name is close. A valid target that reaches nothing
is not an error: the output is empty, with a note on stderr. A blank query
passes through as empty.

A supergraph schema is read as the API schema its clients see, so the types of
the specs it links, such as `join__Graph`, and whatever is `@inaccessible` are
unknown names.";

pub const QUERY_FOCUS_TARGETS: &str = "Types (`MyType`) or fields (`MyType.field`) to focus on";

pub const QUERY_FOCUS_EXAMPLES: &str = "\
Examples:
  graphql-document-utils query focus -s schema.graphql -q query.graphql Profile
  graphql-document-utils query focus -s schema.graphql -q query.graphql User.email Query.company
  cat query.graphql | graphql-document-utils query focus -s schema.graphql Node";

pub const QUERY_STRIP_ABOUT: &str =
    "Remove the given types or fields, and every reference to them, from a query";

// Tested by:
// - subtypes: `query_strip::strips_implementors_of_a_targeted_interface`,
//   `query_strip::strips_every_member_of_a_targeted_union`,
//   `query_strip::strips_fragments_in_place_and_drops_emptied_ones`
// - cascade: `query_strip::cascades_removal_through_emptied_parents`,
//   `query_strip::an_emptied_inline_fragment_goes_and_its_parent_stays`,
//   `query_strip::drops_operations_left_with_nothing`
// - fragments and variables: `query_strip::drops_fragments_defined_on_a_targeted_type`,
//   `query_strip::drops_fragments_no_surviving_operation_spreads`,
//   `query_strip::drops_variables_left_unreferenced_by_the_removal`
// - input types: `query_strip::removes_arguments_and_variables_typed_with_a_targeted_input`,
//   `query_strip::strips_input_object_literals_level_by_level`
// - required inputs: `query_strip::removes_the_field_when_a_required_argument_is_stripped`,
//   `query_strip::removes_a_nullable_argument_on_its_own`,
//   `query_strip::removes_an_argument_with_a_schema_default_on_its_own`,
//   `query_strip::drops_a_directive_that_loses_a_required_argument`
// - lists: `query_strip::drops_only_the_list_elements_that_cannot_stay`,
//   `query_strip::removes_a_list_emptied_by_the_strip`
// - defaults: `query_strip::strips_variable_default_values_level_by_level`,
//   `query_strip::removes_a_variable_whose_default_loses_a_required_input_field`
// - `__typename`: `query_strip::never_matches_fields_the_schema_does_not_define`,
//   `query_strip::strips_inputs_of_fields_the_schema_does_not_define`
// - unknown targets: `query_strip::rejects_unknown_targets`
// - nothing matched: `query_strip::notes_nothing_matched_even_when_unused_fragments_are_dropped`,
//   `cli::a_target_that_matches_nothing_gets_a_note`
// - everything stripped: `query_strip::returns_empty_when_everything_is_stripped`
// - blank: `query_strip::passes_an_empty_document_through`
// - supergraphs: `query_target::a_supergraph_resolves_targets_against_its_api_schema`
pub const QUERY_STRIP_LONG_ABOUT: &str = "\
Remove the given types or fields, and every reference to them, from a query

Targets and their matching are the same as in `query focus`, subtypes
included. Stripping an interface or union strips every selection
returning one of its implementors or members. That can leave nothing.

Removal cascades: a selection set left empty takes its parent field or inline
fragment with it, and an operation left with nothing is dropped. Fragments on a
stripped type are dropped, the rest are reduced in place, and fragments and
variable definitions left unused go too.

Unlike `query focus`, input types are stripped as well: arguments and input
object fields typed with a stripped type are removed, along with the variable
definitions behind them. A required input (non-null, with no default) is never
removed on its own, so the output stays a valid query: a field that would lose
a required argument is removed instead, a directive that would lose one is
dropped, and an object value that would lose a required field is removed by
the same rules. A list element that cannot stay is dropped from its list, and a
list emptied that way is removed by the same rules. Variable default values
are stripped the same way, and a variable whose default loses a required input
field is removed.

Fields the schema does not define, such as `__typename`, are never matched
against the targets, but one is removed if it passes a variable that was
removed.

Every target must be a type or field the schema knows, or the command fails,
with a did-you-mean when a name is close. A valid target that matches nothing
is not an error: the query comes out unchanged apart from dropping unused
fragments, with a note on stderr. If everything is stripped, the output is
empty. A blank query passes through as empty. A supergraph schema is read as
its API schema, as in `query focus`.";

pub const QUERY_STRIP_TARGETS: &str = "Types (`MyType`) or fields (`MyType.field`) to strip";

pub const QUERY_STRIP_EXAMPLES: &str = "\
Examples:
  graphql-document-utils query strip -s schema.graphql -q query.graphql Profile
  graphql-document-utils query strip -s schema.graphql -q query.graphql SearchFilter User.email
  cat query.graphql \\
    | graphql-document-utils query strip -s schema.graphql SearchFilter \\
    | graphql-document-utils query focus -s schema.graphql Profile \\
    | graphql-document-utils query normalize";

pub const SCHEMA_FORMAT_ABOUT: &str = "Reformat a schema, leaving its contents unchanged";

// Tested by:
// - layout and order: `cli::format_keeps_the_source_order`,
//   `cli::a_document_ends_in_exactly_one_newline`
// - contents: `input::formatting_keeps_every_definition_as_written`
// - repeats: `cli::only_the_commands_that_lay_a_document_out_keep_repeats`
// - comments: `cli::comments_are_dropped_and_descriptions_kept`
// - blank: `cli::a_blank_schema_passes_through_every_schema_command`
pub const SCHEMA_FORMAT_LONG_ABOUT: &str = "\
Reformat a schema, leaving its contents unchanged

Every definition prints in a standard layout, in source order. To also sort
them, use `schema sort`.

A name defined more than once keeps every definition, as written. Descriptions
are kept, but `#` comments are dropped. A blank schema passes through as empty.";

pub const SCHEMA_FORMAT_EXAMPLES: &str = "\
Examples:
  graphql-document-utils schema format -s schema.graphql
  cat schema.graphql | graphql-document-utils schema format";

pub const SCHEMA_SORT_ABOUT: &str =
    "Sort schema definitions by category, then alphabetically by name";

// Tested by:
// - categories: `sort::test_sort_with_schema_definition`,
//   `sort::test_sort_with_directives`, `sort::test_sort_mixed_extension_kinds`,
//   `sort::test_sort_compares_names_as_written`
// - layout: `cli::a_document_ends_in_exactly_one_newline`
// - kinds: `sort::test_sort_mixed_definitions`
// - extensions: `sort::test_sort_extensions_by_extended_type_name`,
//   `sort::test_sort_extensions_of_one_type_keep_source_order`
// - names as written: `sort::test_sort_compares_names_as_written`
// - members, repeats, and descriptions: `sort::test_sort_keeps_members_and_repeats_as_written`,
//   `cli::only_the_commands_that_lay_a_document_out_keep_repeats`,
//   `cli::comments_are_dropped_and_descriptions_kept`
// - blank: `sort::test_sort_empty_schema`
pub const SCHEMA_SORT_LONG_ABOUT: &str = "\
Sort schema definitions by category, then alphabetically by name

The categories, in order, are the schema definition, directives, types, and
type extensions. The output is already formatted as `schema format` formats
it.

All type kinds (object, interface, union, enum, input, scalar) are interleaved
within types. Extensions are sorted by the name of the type they extend, and
several extensions of one type keep their source order. Names compare as
written, so uppercase sorts before lowercase. Fields, enum values, and every
other member of a definition keep their source order.

A name defined more than once keeps every definition. Descriptions are kept,
but `#` comments are dropped. A blank schema passes through as empty.";

pub const SCHEMA_SORT_EXAMPLES: &str = "\
Examples:
  graphql-document-utils schema sort -s schema.graphql
  graphql-document-utils schema focus -s schema.graphql User | graphql-document-utils schema sort";

pub const SCHEMA_FOCUS_ABOUT: &str =
    "Reduce a schema to the given types and everything they depend on";

// Tested by:
// - reachable types: `focus::test_focus_nested_types`,
//   `focus::test_focus_walks_union_members`, `focus::test_focus_interface`
// - what they need: `focus::test_focus_keeps_argument_input_types`,
//   `focus::test_focus_keeps_implemented_interfaces_without_their_implementors`,
//   `focus::test_focus_keeps_used_directive_definitions`
// - extensions and `schema`: `focus::test_focus_keeps_extensions_of_kept_types_whole`,
//   `focus::test_focus_trims_schema_definition_to_surviving_roots`
// - defines what it names: every test through `focused`, which runs
//   `util::assert_self_contained`, bar `focus::test_focus_on_a_type_defined_only_by_extensions`
// - query root: `focus::test_focus_keeps_a_query_root_a_given_type_reaches`,
//   `focus::test_focus_query_operation`
// - targets: `focus::rejects_field_targets`,
//   `focus::rejects_an_unknown_type_with_a_suggestion`,
//   `focus::suggests_the_right_case`,
//   `focus::rejects_a_built_in_scalar_the_schema_does_not_define`
// - blank: `focus::passes_a_blank_schema_through`
// - supergraphs: `focus::test_focus_on_a_supergraph_keeps_its_machinery_and_a_query_root`,
//   `focus::test_focus_on_a_supergraph_keeps_a_query_root_it_reaches_whole`,
//   `focus::test_focus_on_a_supergraph_adds_what_the_query_root_field_returns`,
//   each checked by `focused` with `util::assert_valid_supergraph`
pub const SCHEMA_FOCUS_LONG_ABOUT: &str = "\
Reduce a schema to the given types and everything they depend on

Each given type stays, with every type it reaches through field types, union
members, and interface implementors. So does what the kept types need to stand
alone: argument and input types, implemented interfaces, and directive
definitions.

An interface kept only because a type implements it brings in none of its
other implementors. Extensions of a kept type are kept whole and in place, and
a `schema` block keeps only the root types that survive.

The output defines every type it names that the schema defines rather than
only extends, but it is a part of the schema rather than a schema to serve: it
has a query root type only if one of the given types is or reaches it. To cut a
schema down to what a query uses, use `schema prune`.

Targets are type names only, as the schema defines them, extensions included.
Unlike `query focus`, which reduces a query to the paths reaching types or
fields, this takes no fields: a `Type.field` fails with a pointer to that
command. Every type must be one the schema defines, or the command fails, with
a did-you-mean when a name is close; a built-in scalar such as `String` is
rejected, as it has no definition to keep. A blank schema passes through as
empty.

A supergraph stays a supergraph that `schema subgraph` and `schema split` can
read: it keeps its `schema` block and the definitions of the specs it links,
and the join directives of what it keeps. When the given types do not reach
the query root, it is added with one field, and what that field returns.";

pub const SCHEMA_FOCUS_TYPES: &str = "Types to keep, along with all of their descendants";

pub const SCHEMA_FOCUS_EXAMPLES: &str = "\
Examples:
  graphql-document-utils schema focus -s schema.graphql User Company
  cat schema.graphql | graphql-document-utils schema focus SearchResult";

pub const SCHEMA_PRUNE_ABOUT: &str =
    "Remove the types and fields a query does not use from a schema";

// Tested by:
// - types and fields: `prune::keeps_argument_input_types_recursively`,
//   `prune::prunes_fields`, `prune::prunes_interface_fields`
// - valid for the query: every test through `pruned`, which runs
//   `assert_supports_query`
// - implementors and members: `prune::prunes_interface_fields`,
//   `prune::keeps_union_members_the_query_enters`
// - what they need: `prune::keeps_argument_input_types_recursively`,
//   `prune::keeps_only_directive_definitions_in_use`
// - emptied types: `prune::keeps_types_named_by_kept_fields`,
//   `prune::keeps_one_field_of_types_the_query_enters_without_selecting_any`
// - query root: `prune::keeps_the_query_root_of_a_mutation_only_document`
// - several queries: `examples::every_example_runs` (`cat queries/*.graphql`)
// - blank: `prune::passes_a_blank_schema_through`,
//   `cli::a_blank_query_prunes_to_nothing_with_a_note`
// - supergraphs: `prune::keeps_the_fields_keys_requires_and_provides_name`,
//   `prune::keeps_nested_key_fields`,
//   `prune::drops_the_join_directives_of_what_it_removes`,
//   `prune::keeps_a_field_of_what_a_kept_field_returns_in_each_graph_resolving_it`,
//   each checked by `pruned` with `util::assert_valid_supergraph`
pub const SCHEMA_PRUNE_LONG_ABOUT: &str = "\
Remove the types and fields a query does not use from a schema

Every type the query never enters is removed. Object and interface types keep
only the fields the query selects on them or on their interfaces. The query
stays valid against the output.

An interface the query enters keeps every implementor, since any of them can
be returned in its place, and a union keeps only the members the query enters.
What the kept definitions need comes too: argument and input types, the
interfaces they implement, and the definitions of directives the query
applies. A type left with nothing selected is removed, along with every
reference to it, unless a kept field returns it, as when the query selects
only `__typename` on it; such a type keeps one field. The query root type is
always kept, so a query of only mutations still gets one.

To prune for several queries at once, pass them as one document, such as by
piping them all to `-q -`. A blank schema passes through as empty, and a blank
query gives an empty output with a note on stderr.

A supergraph stays a supergraph, as with `schema focus`. The fields a kept
type's keys and a kept field's `@requires` and `@provides` name are kept too,
since the router fetches them to resolve what the query selects. A subgraph
resolving a kept field keeps a field of the type it returns, so it stays a
valid schema. So each subgraph `schema split` extracts from the output has the
part of it the query uses.";

/// A path, not the document `schema prune` transforms, so it has no default and
/// reads stdin only when given `-`.
//
// Tested by: `cli::missing_required_arguments_are_a_usage_error`,
// `cli::prune_reads_either_document_from_stdin`,
// `cli::prune_cannot_read_both_documents_from_stdin`
pub const SCHEMA_PRUNE_QUERY: &str = "\
Query file whose selections decide what the schema
keeps. Required; `-q -` reads it from stdin, and -s
must then name a file.";

pub const SCHEMA_PRUNE_EXAMPLES: &str = "\
Examples:
  graphql-document-utils schema prune -s schema.graphql -q query.graphql
  cat queries/*.graphql | graphql-document-utils schema prune -s schema.graphql -q -";

pub const SCHEMA_SPLIT_ABOUT: &str = "Write each subgraph of a supergraph to a file of its own";

// Tested by:
// - files and notes: `subgraph::split_writes_each_subgraph_to_a_file_of_its_own`,
//   `examples::every_example_runs`
// - replaced, and no other touched: `subgraph::split_replaces_its_own_files_and_no_other`,
//   `cli::split_writes_only_inside_its_directory`
// - no types: `subgraph::split_skips_a_subgraph_with_no_types_with_a_note`
// - names: `subgraph::split_refuses_a_name_that_is_not_a_plain_file_name_and_writes_nothing`,
//   `subgraph::split_refuses_names_that_differ_only_in_case`
// - failures: `subgraph::a_schema_that_is_not_a_supergraph_is_an_error`,
//   `subgraph::a_federation_1_supergraph_is_an_error`,
//   `subgraph::split_into_a_directory_it_cannot_make_is_an_error`
// - blank: `subgraph::split_of_a_blank_schema_writes_nothing`
pub const SCHEMA_SPLIT_LONG_ABOUT: &str = "\
Write each subgraph of a supergraph to a file of its own

Each subgraph is extracted as `schema subgraph` extracts it and written to
`<name>.graphql` in the `-o` directory, which is created if missing. Nothing
goes to stdout; a note on stderr names each file written.

A file of one of those names is replaced, and no other file is touched. A
subgraph with no types gets no file, with a note. The names become file names,
so each must be only letters, digits, `_`, and `-`, and no two may differ only
in case, or the command fails before writing anything; `schema subgraph`
prints a subgraph of any name.

The schema must be a Federation 2 supergraph, or the command fails, as does a
file it cannot write. A blank schema writes nothing.";

// Tested by: `subgraph::split_writes_each_subgraph_to_a_file_of_its_own`
pub const SCHEMA_SPLIT_OUTPUT: &str = "\
Directory to write `<name>.graphql` files to, created
if missing. Required.";

pub const SCHEMA_SPLIT_EXAMPLES: &str = "\
Examples:
  graphql-document-utils schema split -s supergraph.graphql -o subgraphs
  graphql-document-utils schema prune -s supergraph.graphql -q supergraph-query.graphql | graphql-document-utils schema split -o subgraphs";

pub const SCHEMA_SUBGRAPH_ABOUT: &str = "Extract one subgraph's schema from a supergraph";

// Tested by:
// - extraction: `subgraph::extracts_the_subgraphs_the_supergraph_was_composed_from`,
//   `subgraph::a_type_a_graph_extends_is_an_extension_in_it`,
//   `subgraph::a_field_takes_the_type_its_graph_gave_it`,
//   `subgraph::a_join_directive_becomes_the_directive_it_records_in_the_graphs_it_names`,
//   `subgraph::a_carried_directive_is_applied_and_imported_under_its_federation_name`
// - `@link`: `subgraph::extracts_the_subgraphs_the_supergraph_was_composed_from`,
//   `subgraph::a_subgraph_using_no_federation_directive_imports_none`
// - names: `subgraph::an_unknown_subgraph_suggests_one_and_lists_them_all`
// - rebuilt: `subgraph::extracts_the_subgraphs_the_supergraph_was_composed_from`
//   (`@shareable`), `subgraph::a_field_takes_the_type_its_graph_gave_it`,
//   `subgraph::an_unused_external_field_goes_and_what_it_empties_with_it`
// - `extend schema`: `subgraph::a_subgraph_without_root_types_extends_the_schema`
// - warnings: `subgraph::a_linked_directive_extraction_does_not_carry_is_a_warning`,
//   `supergraph::context_arguments_are_not_followed_and_say_so`
// - failures: `subgraph::a_schema_that_is_not_a_supergraph_is_an_error`,
//   `subgraph::a_federation_1_supergraph_is_an_error`
// - no types: `subgraph::a_subgraph_with_no_types_prints_nothing_with_a_note`
// - blank: `subgraph::a_blank_schema_passes_through`
pub const SCHEMA_SUBGRAPH_LONG_ABOUT: &str = "\
Extract one subgraph's schema from a supergraph

The subgraph is printed as a schema of its own: the types, fields, and members
it has, as it typed them, with the federation directives it applies (`@key`,
`@external`, `@requires`, `@provides`, `@override`, and the rest) read back
from the supergraph's join directives, under an `@link` importing those it
uses. For a supergraph composed from subgraphs, that is what each of them
said, apart from what a supergraph does not record.

The name is the one the supergraph's `@join__graph(name:)` gives, as `schema
split` names its files. A name no subgraph has fails, with a did-you-mean when
one is close, and the names there are.

What a supergraph does not record is rebuilt where it can be. `@shareable` goes
on each field more than one subgraph resolves, or on its type when all of its
fields are. `@inaccessible`, `@tag`, and the other directives composition
keeps go on the element in every subgraph that has it. An `@external` field no
key, `@requires`, or `@provides` names is dropped, with what that empties. The
`@link` is at the lowest federation version that defines what the subgraph
uses, and a subgraph with no root types opens with `extend schema`.

What it cannot rebuild is lost: how a subgraph named its root types, arguments
it declared differently than composition merged them, and directives
composition did not keep. A directive of another spec the supergraph links is
dropped, with a warning, and a field taking `@fromContext` arguments gets one
too, since what those select is not followed.

The schema must be a Federation 2 supergraph, or the command fails. A subgraph
left with no types prints nothing, with a note on stderr. A blank schema
passes through as empty.";

pub const SCHEMA_SUBGRAPH_NAME: &str = "\
Name of the subgraph, as the supergraph's
`@join__graph(name:)` gives it";

pub const SCHEMA_SUBGRAPH_EXAMPLES: &str = "\
Examples:
  graphql-document-utils schema subgraph -s supergraph.graphql reviews
  cat supergraph.graphql | graphql-document-utils schema subgraph accounts";

pub const SKILL_ABOUT: &str = "Print a complete guide to this tool for agents, as an Agent Skill";

// Tested by: `skill::frontmatter_names_the_binary`,
// `skill::every_command_and_flag_is_in_the_skill`,
// `skill::every_example_is_in_the_skill`, and `skill::snapshot`, whose
// `skill` checks the version stamp
pub const SKILL_LONG_ABOUT: &str = "\
Print a complete guide to this tool for agents, as an Agent Skill

The guide is Markdown with YAML frontmatter: how the commands read and write
documents, what a target matches, what each command does, with its flags and
examples, and what its results and exit codes mean. An agent can read it as it
is printed. Saved as `SKILL.md` in a directory of its own under a skills
directory, it loads whenever a task calls for this tool. It describes the
version that printed it, so print it again after upgrading.";

pub const SKILL_EXAMPLES: &str = "\
Examples:
  graphql-document-utils skill
  graphql-document-utils skill > ~/.claude/skills/graphql-document-utils/SKILL.md";

#[cfg(test)]
pub(crate) mod extract;

/// Shared with the tests of `skill`, which is built from the same commands.
#[cfg(test)]
pub(crate) mod tests {
    use super::extract::{self, invocations};
    use crate::{error::Error, input::Kind, Args};
    use clap::{CommandFactory, Parser};

    /// The top-level help's common tasks, as they are attached to the
    /// command: each intent with its command, and the lines after the table.
    fn tasks() -> (Vec<(String, String)>, String) {
        let after_help = Args::command().get_after_help().unwrap().to_string();
        extract::tasks(&after_help)
    }

    /// Parses every invocation of the binary in `example` as the CLI's own
    /// command line, panicking with clap's error on one that does not parse,
    /// and returns their arguments. Asking for help parses, though clap
    /// reports it as an error, as it prints the help rather than running.
    fn parse(example: &str) -> Vec<Vec<&str>> {
        let invocations = invocations(example);
        for args in &invocations {
            let argv = std::iter::once(env!("CARGO_BIN_NAME")).chain(args.iter().copied());
            match Args::try_parse_from(argv) {
                Err(error) if error.kind() != clap::error::ErrorKind::DisplayHelp => {
                    panic!("{example}\n{error}")
                }
                _ => {}
            }
        }
        invocations
    }

    /// `extract` names the binary itself, for the integration test, which is
    /// not compiled with `CARGO_BIN_NAME`.
    #[test]
    fn extract_names_the_binary() {
        assert_eq!(extract::BIN, env!("CARGO_BIN_NAME"));
    }

    /// Every leaf command, as the words that name it (`["query", "focus"]`),
    /// alongside its clap definition.
    pub(crate) fn leaves() -> Vec<(Vec<String>, clap::Command)> {
        fn walk(
            path: Vec<String>,
            command: &clap::Command,
            leaves: &mut Vec<(Vec<String>, clap::Command)>,
        ) {
            if !command.has_subcommands() {
                leaves.push((path, command.clone()));
                return;
            }
            for sub in command.get_subcommands() {
                let mut path = path.clone();
                path.push(sub.get_name().to_string());
                walk(path, sub, leaves);
            }
        }
        let mut leaves = Vec::new();
        walk(Vec::new(), &Args::command(), &mut leaves);
        leaves
    }

    /// A leaf's help as `-h` and `--help` print it, apart from the usage line,
    /// which names only the leaf when it is rendered on its own.
    fn helps(command: &mut clap::Command) -> [String; 2] {
        [
            command.render_help().to_string(),
            command.render_long_help().to_string(),
        ]
    }

    #[test]
    fn every_leaf_has_an_about_a_long_about_and_examples() {
        let leaves = leaves();
        assert!(!leaves.is_empty());
        for (path, command) in leaves {
            let about = command.get_about().map(|about| about.to_string());
            let about = about.unwrap_or_else(|| panic!("{path:?} has an about"));
            assert!(!about.contains('\n'), "{path:?} about is one line");

            let long_about = command.get_long_about().map(|about| about.to_string());
            let long_about = long_about.unwrap_or_else(|| panic!("{path:?} has a long_about"));
            assert!(
                long_about.starts_with(&format!("{about}\n\n")),
                "{path:?} long_about opens with its about:\n{long_about}"
            );

            let examples = command.get_after_help().map(|help| help.to_string());
            let examples = examples.unwrap_or_else(|| panic!("{path:?} has examples"));
            let count = extract::examples(&examples).len();
            assert!((2..=4).contains(&count), "{path:?} has {count} examples");
        }
    }

    /// Every example parses as the CLI's own command line, so none can go
    /// stale as flags change, and each invokes the command it is listed under.
    #[test]
    fn every_example_parses() {
        for (path, command) in leaves() {
            let examples = command.get_after_help().unwrap().to_string();
            for example in extract::examples(&examples) {
                let invocations = parse(&example);
                assert!(
                    invocations
                        .iter()
                        .any(|args| args.iter().take(path.len()).eq(&path)),
                    "{example}\ninvokes {path:?}"
                );
            }
        }
    }

    /// Every common task's command parses the same way, and is a single
    /// invocation of a leaf command, so it does the task on its own.
    #[test]
    fn every_task_parses() {
        let (tasks, _) = tasks();
        assert!(!tasks.is_empty());
        let leaves = leaves();
        for (intent, command) in tasks {
            let invocations = parse(&command);
            let [args] = invocations.as_slice() else {
                panic!("{intent}: {command}\nis one invocation");
            };
            assert!(
                leaves
                    .iter()
                    .any(|(path, _)| args.iter().take(path.len()).eq(path)),
                "{intent}: {command}\ninvokes a leaf command"
            );
        }
    }

    /// The README, whose commands are checked against the CLI like the help's.
    const README: &str = include_str!("../README.md");

    /// The commands in the README's `bash` blocks that invoke the binary.
    fn readme_commands() -> Vec<String> {
        extract::readme_commands(README)
    }

    /// Every command the README shows parses as the CLI's own command line.
    #[test]
    fn every_readme_command_parses() {
        let commands = readme_commands();
        assert!(!commands.is_empty(), "the README has commands");
        for command in commands {
            parse(&command);
        }
    }

    /// The README shows every command, so a new one cannot go undocumented.
    #[test]
    fn the_readme_invokes_every_command() {
        let commands = readme_commands();
        let invocations: Vec<_> = commands.iter().flat_map(|c| invocations(c)).collect();
        for (path, _) in leaves() {
            assert!(
                invocations
                    .iter()
                    .any(|args| args.iter().take(path.len()).eq(&path)),
                "the README invokes {path:?}"
            );
        }
    }

    /// The README lists the help's common tasks, in one `bash` block, as the
    /// help does: each intent, as a comment, above its command.
    #[test]
    fn the_readme_lists_the_common_tasks() {
        let (tasks, _) = tasks();
        let block: String = tasks
            .iter()
            .map(|(intent, command)| format!("# {intent}\n{command}\n"))
            .collect();
        assert!(
            README.contains(&format!("```bash\n{block}```\n")),
            "the README has the common tasks:\n{block}"
        );
    }

    /// The commands the common tasks' footer quotes parse too, such as
    /// `skill`. One naming `<noun> <verb>` parses with each leaf in its place.
    #[test]
    fn every_command_the_tasks_point_at_parses() {
        let (_, footer) = tasks();
        let quoted: Vec<_> = footer.split('\'').skip(1).step_by(2).collect();
        assert!(!quoted.is_empty(), "{footer}");
        for command in quoted {
            let command = command.replace('\n', " ");
            if command.contains("<noun> <verb>") {
                for (path, _) in leaves() {
                    parse(&command.replace("<noun> <verb>", &path.join(" ")));
                }
            } else {
                parse(&command);
            }
        }
    }

    /// Each noun's `about` ends by listing its verbs, in the order its help
    /// lists them, so the top-level command list names every command.
    #[test]
    fn every_noun_names_its_verbs() {
        let args = Args::command();
        let nouns: Vec<_> = args
            .get_subcommands()
            .filter(|noun| noun.has_subcommands())
            .collect();
        assert!(!nouns.is_empty());
        for noun in nouns {
            let verbs: Vec<_> = noun.get_subcommands().map(|verb| verb.get_name()).collect();
            let about = noun.get_about().map(ToString::to_string);
            let about = about.unwrap_or_else(|| panic!("`{}` has an about", noun.get_name()));
            assert!(
                about.ends_with(&format!(" ({})", verbs.join(", "))),
                "`{}` about names {verbs:?}: {about}",
                noun.get_name()
            );
        }
    }

    /// The exit codes the help gives are the ones the errors exit with.
    /// `error` checks clap's usage errors exit as its own do.
    #[test]
    fn the_output_conventions_give_the_exit_codes() {
        let words: Vec<_> = super::OUTPUT_CONVENTIONS.split_whitespace().collect();
        let conventions = words.join(" ");
        for error in Error::one_of_each() {
            let meaning = if error.is_usage() {
                "for a usage error"
            } else {
                "for a bad input"
            };
            let claim = format!("{} {meaning}", error.status());
            assert!(conventions.contains(&claim), "{error:?}: {claim}");
        }
    }

    #[test]
    fn prose_is_wrapped_at_80_columns() {
        for (path, command) in leaves() {
            let long_about = command.get_long_about().unwrap().to_string();
            for line in long_about.lines() {
                assert!(line.chars().count() <= 80, "{path:?}: {line}");
            }
        }
        let (tasks, footer) = tasks();
        let intents = tasks.iter().map(|(intent, _)| intent.as_str());
        let prose = intents
            .chain(footer.lines())
            .chain(super::OUTPUT_CONVENTIONS.lines());
        for line in prose {
            assert!(line.chars().count() <= 80, "{line}");
        }
    }

    /// Flag and argument help fits 80 columns once clap indents it, which it
    /// does by a different amount in `-h` and `--help`.
    #[test]
    fn flag_help_is_wrapped_at_80_columns() {
        for (path, mut command) in leaves() {
            for help in helps(&mut command) {
                let flags = help
                    .lines()
                    .skip_while(|line| !matches!(*line, "Arguments:" | "Options:"))
                    .take_while(|line| *line != "Examples:");
                for line in flags {
                    assert!(line.chars().count() <= 80, "{path:?}: {line}");
                }
            }
        }
    }

    /// The short flag `arg` is given with, as `-q`.
    fn flag(arg: &clap::Arg) -> Option<String> {
        arg.get_short().map(|short| format!("-{short}"))
    }

    /// The kind of document the argument `id` names.
    fn kind(id: &str) -> Kind {
        match id {
            "query" => Kind::Query,
            "schema" => Kind::Schema,
            _ => panic!("`{id}` names no document"),
        }
    }

    /// Besides the document it is named for, a command reads at most one
    /// other, from a required flag that has no default: `query focus` and
    /// `query strip` the schema, and `schema prune` the query.
    #[test]
    fn the_other_document_is_a_required_flag() {
        let mut others = Vec::new();
        for (path, command) in leaves() {
            for arg in command.get_arguments() {
                let id = arg.get_id().as_str();
                if matches!(id, "query" | "schema") && id != path[0] {
                    assert!(arg.is_required_set(), "{path:?} --{id} is required");
                    assert!(arg.get_default_values().is_empty(), "{path:?} --{id}");
                    assert_eq!(flag(arg), Some(kind(id).flag().to_string()), "{path:?}");
                    others.push(format!("{} {}", path.join(" "), kind(id).flag()));
                }
            }
        }
        assert_eq!(
            others,
            ["query focus -s", "query strip -s", "schema prune -q"]
        );
    }

    /// On every command, the flag for the document it is named for reads
    /// stdin when omitted, and no other flag does. Each says so in the one
    /// wording for its document, in place of clap's `[default: -]`.
    #[test]
    fn every_stdin_flag_says_so_in_one_wording() {
        for (path, mut command) in leaves() {
            // A command outside the nouns, like `skill`, reads no document.
            let noun = path[0].as_str();
            let wording = match noun {
                "query" => Some(super::QUERY_FROM_STDIN),
                "schema" => Some(super::SCHEMA_FROM_STDIN),
                _ => None,
            };
            for arg in command.get_arguments() {
                let id = arg.get_id().as_str();
                let reads_stdin = arg
                    .get_default_values()
                    .iter()
                    .map(|value| value.to_str())
                    .eq([Some("-")]);
                assert_eq!(reads_stdin, id == noun, "{path:?} --{id} reads stdin");
                if reads_stdin {
                    let help = arg.get_help().map(ToString::to_string);
                    assert_eq!(help.as_deref(), wording, "{path:?} --{id}");
                    assert_eq!(flag(arg), Some(kind(id).flag().to_string()), "{path:?}");
                }
            }
            for help in helps(&mut command) {
                assert!(!help.contains("[default: -]"), "{path:?}:\n{help}");
            }
        }
    }

    /// The source of each module a `Tested by:` comment can name a test in,
    /// by the name it is given there: this crate's modules, its integration
    /// tests, and the library's tests as `graphql_normalize`.
    const TEST_SOURCES: [(&str, &str); 18] = [
        ("cli", include_str!("../tests/cli.rs")),
        ("docs", include_str!("docs.rs")),
        ("error", include_str!("error.rs")),
        ("examples", include_str!("../tests/examples.rs")),
        ("focus", include_str!("focus.rs")),
        (
            "graphql_normalize",
            include_str!("../graphql-normalize-lib/src/lib.rs"),
        ),
        ("input", include_str!("input.rs")),
        ("main", include_str!("main.rs")),
        ("prune", include_str!("prune.rs")),
        ("query_focus", include_str!("query_focus.rs")),
        ("query_strip", include_str!("query_strip.rs")),
        ("query_target", include_str!("query_target.rs")),
        ("skill", include_str!("skill.rs")),
        ("sort", include_str!("sort.rs")),
        ("subgraph", include_str!("subgraph.rs")),
        ("supergraph", include_str!("supergraph.rs")),
        ("util", include_str!("util.rs")),
        ("extract", include_str!("docs/extract.rs")),
    ];

    /// The `Tested by:` comments of `source`, each with the line of the item
    /// it is above: a run of `//` lines, not doc comments, that has one.
    fn claim_comments(source: &str) -> Vec<(String, &str)> {
        let mut comments = Vec::new();
        let mut comment = String::new();
        for line in source.lines() {
            let line = line.trim_start();
            if line.starts_with("//") && !line.starts_with("///") && !line.starts_with("//!") {
                comment.push_str(line);
                comment.push('\n');
            } else if line.starts_with("///") || line.starts_with("#[") {
                // Doc comments and attributes may sit between a comment and
                // its item.
            } else {
                if comment.contains("Tested by") {
                    comments.push((std::mem::take(&mut comment), line));
                }
                comment.clear();
            }
        }
        comments
    }

    /// Every test a claim's comment names, as `module::name`, exists, so a
    /// renamed or removed test cannot leave a claim pointing at nothing.
    #[test]
    fn every_test_a_claim_names_exists() {
        let mut named = 0;
        for source in [include_str!("docs.rs"), include_str!("skill.rs")] {
            for (comment, item) in claim_comments(source) {
                let names: Vec<_> = comment
                    .split('`')
                    .skip(1)
                    .step_by(2)
                    .filter_map(|name| name.split_once("::"))
                    .collect();
                assert!(!names.is_empty(), "the comment on `{item}` names a test");
                for (module, name) in names {
                    let (_, test) = TEST_SOURCES
                        .iter()
                        .find(|(source, _)| *source == module)
                        .unwrap_or_else(|| panic!("`{module}::{name}`: no module `{module}`"));
                    assert!(
                        test.contains(&format!("fn {name}(")),
                        "`{module}::{name}`, named on `{item}`, is a function in `{module}`"
                    );
                    named += 1;
                }
            }
        }
        assert!(named > 100, "{named}");
    }

    /// Every `long_about`, the prose that states how a command behaves, names
    /// the tests that check what it says.
    #[test]
    fn every_long_about_names_its_tests() {
        let source = include_str!("docs.rs");
        let commented: Vec<_> = claim_comments(source)
            .into_iter()
            .map(|(_, item)| item)
            .collect();
        let long_abouts: Vec<_> = source
            .lines()
            .filter(|line| line.starts_with("pub const ") && line.contains("_LONG_ABOUT: "))
            .collect();
        assert_eq!(long_abouts.len(), leaves().len(), "{long_abouts:?}");
        for long_about in long_abouts {
            assert!(commented.contains(&long_about), "{long_about}");
        }
    }

    /// The Rust version the README and CONTRIBUTING say the tool builds with
    /// is the one both crates declare.
    #[test]
    fn the_stated_rust_version_is_the_declared_one() {
        let declared = |manifest: &str| {
            let line = manifest
                .lines()
                .find(|line| line.starts_with("rust-version = "))
                .unwrap_or_else(|| panic!("a `rust-version`:\n{manifest}"));
            line.trim_start_matches("rust-version = ")
                .trim_matches('"')
                .to_string()
        };
        let version = declared(include_str!("../Cargo.toml"));
        let library = declared(include_str!("../graphql-normalize-lib/Cargo.toml"));
        assert_eq!(library, version, "both crates declare one version");
        let stated = format!("Rust {version} or later");
        assert!(include_str!("../README.md").contains(&stated), "{stated}");
        assert!(
            include_str!("../CONTRIBUTING.md").contains(&stated),
            "{stated}"
        );
    }
}
