//! What `skill` prints: a guide to the tool for agents, as an Agent Skill,
//! YAML frontmatter followed by Markdown. An agent can read it as it is
//! printed, and a user can save it to a skills directory so it loads whenever
//! a task calls for the tool.
//!
//! What the commands share, which no one command's help says, is written out
//! here. The reference for each command is built from its clap definition
//! instead: a synopsis of the arguments clap defines, and the `about`, summary,
//! and examples its `--help` shows from `docs`. So a new command or flag
//! appears in both, and the two cannot disagree. Everything else a command's
//! `--help` says, the skill leaves to it, to stay short enough to read whole.

use std::{collections::BTreeMap, fmt::Write};

use clap::{ArgAction, CommandFactory};

use crate::{error::Error, Args};

/// When the skill applies, which is all an agent sees of it before loading
/// it, so it lists what someone would ask for rather than what the tool has.
/// One line, which YAML reads as a plain string as long as it has no `: `.
const DESCRIPTION: &str = "Transform GraphQL documents from the command line. Use to canonicalize or diff GraphQL queries, cut a query down to the parts that reach a type or field, remove a type or field and everything that references it from a query, prune a schema to what a set of queries uses, extract a type and its dependencies from a schema, or sort and format a schema.";

/// How to run any command, in the order an agent does it. The README's usage
/// lists the same steps.
//
// Tested by:
// - stdout and stderr: `cli::no_command_changes_a_file`, every example's stdout
//   in `examples::every_example_runs`, `cli::bad_input_exits_1`,
//   `cli::a_target_that_matches_nothing_gets_a_note`
// - stdin: `docs::every_stdin_flag_says_so_in_one_wording`,
//   `cli::a_schema_piped_in_is_read_as_if_from_its_file`,
//   `cli::a_command_waits_on_an_open_stdin`
// - the other document: `docs::the_other_document_is_a_required_flag`,
//   `cli::prune_reads_either_document_from_stdin`,
//   `cli::prune_cannot_read_both_documents_from_stdin`
// - files as targets: `cli::a_query_file_given_as_a_target_fails_without_reading_stdin`,
//   `cli::a_schema_file_given_as_a_type_fails_without_reading_stdin`
// - names: `query_target::accepts_types_and_fields_added_by_extensions`,
//   `focus::test_focus_on_a_type_defined_only_by_extensions`,
//   `focus::suggests_the_right_case`,
//   `query_target::suggests_a_case_insensitive_match_first`,
//   `query_target::rejects_an_unknown_field_with_a_suggestion`,
//   `focus::rejects_field_targets`
// - subtypes: `query_focus::a_union_target_matches_its_members`,
//   `query_strip::strips_implementors_of_a_targeted_interface`,
//   `query_focus::matches_interface_field_on_implementing_type`,
//   `query_focus::a_field_target_on_an_implementor_matches_it_selected_on_the_interface`
// - `__typename`: `query_focus::never_matches_fields_the_schema_does_not_define`,
//   `query_strip::never_matches_fields_the_schema_does_not_define`
// - inputs: `query_focus::notes_an_input_type_target`,
//   `query_strip::removes_arguments_and_variables_typed_with_a_targeted_input`
// - cascades: `query_strip::cascades_removal_through_emptied_parents`,
//   `query_strip::an_emptied_inline_fragment_goes_and_its_parent_stays`,
//   `query_strip::drops_operations_left_with_nothing`,
//   `query_strip::removes_the_field_when_a_required_argument_is_stripped`,
//   `query_strip::drops_a_directive_that_loses_a_required_argument`
// - comments and repeats: `cli::comments_are_dropped_and_descriptions_kept`,
//   `cli::a_repeated_definition_warns_and_the_first_is_used`,
//   `cli::only_the_commands_that_lay_a_document_out_keep_repeats`
const STEPS: &str = "\
## Steps

1. Pick the command for the task from the per-command sections below. For
   every rule of one command, run `graphql-document-utils <noun> <verb> --help`.
2. Pass the documents.
   - Pass the query to a `query` command with `-q FILE`.
   - Pass the schema to a `schema` command with `-s FILE`.
   - Omit the `-q` of a `query` command, or the `-s` of a `schema` command,
     to read that document from stdin, as in a pipeline.
   - Outside a pipeline, pass every document with its flag. A command waits
     forever on a stdin pipe that never closes.
   - Pass `query focus` and `query strip` the schema too, with `-s FILE`.
   - Pass `schema prune` the query too, with `-q FILE`. To pipe the query in,
     pass `-q -` and the schema with `-s FILE`.
3. Pass targets to `query focus`, `query strip`, and `schema focus` as
   positional arguments.
   - Pass `query focus` and `query strip` types (`Profile`) or fields on a
     type (`User.email`).
   - Pass `schema focus` types. To target a field, use `query focus`.
   - Spell each name as the schema does, case included. Names from `extend`
     blocks count.
   - Pass files only with `-q` and `-s`. A positional argument is a name.
   - An unknown name exits 1, with a did-you-mean when a name is close.
   - An interface or union target also matches its implementors or members.
   - A field target matches across interfaces both ways. When `User`
     implements `Person`, `Person.name` matches `name` on `User`, and
     `User.name` matches `name` on `Person`.
   - Fields the schema does not define, such as `__typename`, match no target.
   - `query focus` matches output selections only. An input type target
     matches nothing.
   - `query strip` also removes arguments and input fields of a stripped type,
     and their variables.
   - `query strip` removes a field or inline fragment whose selection set it
     empties, and an operation it empties.
   - `query strip` removes a field or directive whose required input it
     removes.
4. Redirect stdout to a file to save the result.
5. Read the result.
   - Stdout holds only the GraphQL document.
   - Stderr holds `error:`, `warning:`, and `note:` lines.
   - Look up the exit code under Results and exit codes below.
   - On exit 1 or 2, fix what the `error:` line names, then rerun.
   - The output drops `#` comments and keeps `\"descriptions\"`.
   - A name defined more than once gets a `warning:`, and the command uses
     its first definition. `query normalize`, `schema format`, and
     `schema sort` keep every definition.";

/// What the results of a command that succeeds are. What each other exit code
/// means is listed after this from the errors themselves, by `exit_codes`.
//
// Tested by:
// - a note: `cli::a_target_that_matches_nothing_gets_a_note`,
//   `query_focus::notes_a_valid_target_that_nothing_reaches`,
//   `query_strip::notes_nothing_matched_even_when_unused_fragments_are_dropped`
// - zero bytes: `cli::an_empty_result_prints_nothing`
// - blank: `input::blank_documents_parse_as_none`,
//   `cli::a_blank_schema_passes_through_every_schema_command`,
//   `cli::an_empty_focus_pipes_into_format`,
//   `cli::an_empty_strip_pipes_into_normalize`
const SUCCESS: &str = "\
- `0`: success. A valid target that matches nothing adds a `note:` on stderr.
  When no target matches, `query focus` prints nothing, and `query strip`
  prints the query minus its unused fragments.
- Empty output is zero bytes. A blank document to transform passes through as
  empty, so the next command in a pipeline still runs. Blank means only
  whitespace, commas, and comments.";

/// The usage errors clap reports on its own, before any command runs.
const CLAP_ERRORS: [&str; 2] = ["an unknown command or flag", "a missing flag or targets"];

/// A bullet for each status the tool exits with on failure, listing what
/// fails with it: every `Error`, by its summary, and clap's own usage errors
/// under the status clap exits with. So the list cannot drift from the code.
fn exit_codes() -> String {
    let clap = Args::command()
        .try_get_matches_from([env!("CARGO_BIN_NAME"), "--unknown"])
        .expect_err("an unknown flag is an error")
        .exit_code();
    let errors = Error::one_of_each();
    let mut statuses: BTreeMap<i32, (bool, Vec<&str>)> = BTreeMap::new();
    for error in &errors {
        let (usage, summaries) = statuses.entry(error.status().into()).or_default();
        *usage |= error.is_usage();
        if !summaries.contains(&error.summary()) {
            summaries.push(error.summary());
        }
    }
    let (usage, summaries) = statuses.entry(clap).or_default();
    *usage = true;
    summaries.splice(0..0, CLAP_ERRORS);

    let mut bullets = Vec::new();
    for (status, (usage, summaries)) in statuses {
        let meaning = if usage {
            "usage error"
        } else {
            "bad input, or output that cannot be written"
        };
        let (last, rest) = summaries.split_last().expect("a status has an error");
        let list = match rest {
            [] => last.to_string(),
            [first] => format!("{first} or {last}"),
            _ => format!("{}, or {last}", rest.join(", ")),
        };
        let bullet = wrap(&format!("`{status}`: {meaning}: {list}."), 78);
        bullets.push(format!("- {}", bullet.replace('\n', "\n  ")));
    }
    bullets.join("\n")
}

/// The results section, under its heading in the skill, and in the README.
fn results() -> String {
    format!("{SUCCESS}\n{}", exit_codes())
}

/// The skill, frontmatter and all. The binary is named as `CARGO_BIN_NAME`
/// rather than as it was invoked, since the skill is saved and read later,
/// when it should name the command on `PATH`, and the version is stamped on it
/// so an agent can compare it with `--version` and notice a stale copy.
///
/// Each noun gets a section with a reference for each of its commands. A
/// command outside the nouns (`skill` itself) has none, as it does nothing
/// with GraphQL; the header says how to run it.
pub fn render() -> String {
    let bin = env!("CARGO_BIN_NAME");
    let version = env!("CARGO_PKG_VERSION");
    let stamp = wrap(
        &format!(
            "Generated by {bin} {version}. If this guide is a saved file and \
             `{bin} --version` prints another version, overwrite the file with \
             the output of `{bin} skill`."
        ),
        80,
    );
    let intro = wrap(
        &format!(
            "Use `{bin}` to transform a GraphQL query or schema. Each command prints \
             the transformed document to stdout. Run a command as \
             `{bin} <noun> <verb>`, where the noun is `query` or `schema`."
        ),
        80,
    );
    let mut skill = format!(
        "\
---
name: {bin}
description: {DESCRIPTION}
---

# {bin}

{stamp}

{intro}

{STEPS}
"
    );
    let args = Args::command();
    for noun in args.get_subcommands().filter(|noun| noun.has_subcommands()) {
        write!(skill, "\n## `{}` commands\n", noun.get_name()).unwrap();
        for verb in noun.get_subcommands() {
            let name = format!("{} {}", noun.get_name(), verb.get_name());
            reference(&mut skill, &name, verb);
        }
    }
    write!(skill, "\n## Results and exit codes\n\n{}\n", results()).unwrap();
    skill
}

/// Appends the reference for the command `name` names: a synopsis of its
/// arguments, its `about` run into the summary its `long_about` opens with
/// after the `about`, and its examples, as a code block.
///
/// The synopsis spells out every argument, where clap's usage line folds the
/// optional flags into `[OPTIONS]`: flags first, in the order they are
/// defined, then positionals, each as clap writes it in `--help`, and in
/// brackets unless required. Defaults are not shown; clap hides every one
/// there is, the `-` a stdin flag defaults to, which the guide says in words.
fn reference(skill: &mut String, name: &str, command: &clap::Command) {
    // clap can write an argument only once its command is built, which adds
    // the `--help` flag every command has, and which the synopsis leaves out.
    let mut command = command.clone();
    command.build();
    let mut synopsis = format!("{} {name}", env!("CARGO_BIN_NAME"));
    let mut arguments: Vec<_> = command
        .get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .filter(|arg| !matches!(arg.get_action(), ArgAction::Help | ArgAction::Version))
        .collect();
    arguments.sort_by_key(|arg| arg.is_positional());
    for arg in arguments {
        if arg.is_required_set() {
            write!(synopsis, " {arg}").unwrap();
        } else {
            write!(synopsis, " [{arg}]").unwrap();
        }
    }

    let about = command.get_about().expect("every command has an about");
    let long_about = command
        .get_long_about()
        .expect("every command has a long_about")
        .to_string();
    let summary = long_about
        .split("\n\n")
        .nth(1)
        .expect("a long_about has a summary after its about");
    let description = wrap(&format!("{about}. {summary}"), 80);

    let examples = command
        .get_after_help()
        .expect("every command has examples")
        .to_string();
    let examples = examples
        .strip_prefix("Examples:\n")
        .expect("examples start with `Examples:`")
        .lines()
        // Indented two spaces, and four when continued; two more makes them
        // a Markdown code block.
        .map(|example| format!("  {example}\n"))
        .collect::<String>();

    write!(skill, "\n`{synopsis}`\n\n{description}\n\n{examples}").unwrap();
}

/// `text` rewrapped into lines of at most `columns`, broken between words but
/// never inside a `code span`, which Markdown would render with the break as a
/// space. A word too long for a line gets one to itself.
fn wrap(text: &str, columns: usize) -> String {
    let mut words: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match words.last_mut() {
            Some(open) if open.matches('`').count() % 2 == 1 => {
                open.push(' ');
                open.push_str(word);
            }
            _ => words.push(word.to_string()),
        }
    }
    let mut wrapped = String::new();
    let mut width = 0;
    for word in &words {
        let length = word.chars().count();
        if width > 0 && width + 1 + length > columns {
            wrapped.push('\n');
            width = 0;
        } else if width > 0 {
            wrapped.push(' ');
            width += 1;
        }
        wrapped.push_str(word);
        width += length;
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use crate::docs::tests::leaves;

    /// The skill with the version it is stamped with made generic, so a
    /// release, which bumps the version before running the tests, does not
    /// change the snapshot.
    fn skill() -> String {
        let stamp = format!("{} {}", env!("CARGO_BIN_NAME"), env!("CARGO_PKG_VERSION"));
        let generic = format!("{} [version]", env!("CARGO_BIN_NAME"));
        let skill = super::render();
        assert!(skill.contains(&format!("Generated by {stamp}.")), "{skill}");
        skill.replace(&stamp, &generic)
    }

    /// Every change to the skill shows up in review as a change to the
    /// snapshot. `cargo insta review` accepts one.
    #[test]
    fn snapshot() {
        insta::assert_snapshot!(skill());
    }

    /// The skill names every command and every long flag, so none is left for
    /// an agent to find only through `--help`.
    #[test]
    fn every_command_and_flag_is_in_the_skill() {
        let skill = super::render();
        for (path, command) in leaves() {
            let name = path.join(" ");
            let invocation = format!("{} {name}", env!("CARGO_BIN_NAME"));
            assert!(skill.contains(&invocation), "{invocation}");
            for arg in command.get_arguments() {
                if let Some(long) = arg.get_long() {
                    assert!(skill.contains(&format!("--{long}")), "{name} --{long}");
                }
            }
        }
    }

    /// Every example a `query` or `schema` command's help ends with is in the
    /// skill, as a code block, so the skill shows each command as its help
    /// does.
    #[test]
    fn every_example_is_in_the_skill() {
        let skill = super::render();
        let mut examples = 0;
        for (path, command) in leaves() {
            if !matches!(path[0].as_str(), "query" | "schema") {
                continue;
            }
            let after_help = command.get_after_help().unwrap().to_string();
            for line in after_help.lines().skip(1) {
                assert!(skill.contains(&format!("\n  {line}\n")), "{path:?}: {line}");
                examples += 1;
            }
        }
        assert!(examples > 7, "{examples}");
    }

    /// No argument shows a default in `--help`, since the synopsis, which
    /// otherwise spells out what `--help` does, would leave it out.
    #[test]
    fn every_default_is_hidden() {
        for (path, command) in leaves() {
            for arg in command.get_arguments() {
                let shown = arg.get_action().takes_values()
                    && !arg.get_default_values().is_empty()
                    && !arg.is_hide_default_value_set();
                assert!(!shown, "{path:?} {} shows a default", arg.get_id());
            }
        }
    }

    /// The frontmatter is YAML that names the skill after the binary, as a
    /// skill's directory is, and describes when it applies.
    #[test]
    fn frontmatter_names_the_binary() {
        let skill = super::render();
        let (frontmatter, _) = skill
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .unwrap_or_else(|| panic!("frontmatter between `---` lines:\n{skill}"));
        let frontmatter: serde_norway::Value = serde_norway::from_str(frontmatter)
            .unwrap_or_else(|error| panic!("{error}:\n{frontmatter}"));
        assert_eq!(
            frontmatter["name"].as_str(),
            Some(env!("CARGO_BIN_NAME")),
            "{frontmatter:?}"
        );
        let description = frontmatter["description"].as_str();
        assert_eq!(description, Some(super::DESCRIPTION), "{frontmatter:?}");
    }

    /// Prose fits 80 columns like the help it is built from. Commands are not
    /// wrapped, so they can be copied whole: the synopses, the examples,
    /// indented as a code block, and the one in the frontmatter's
    /// description.
    #[test]
    fn prose_is_wrapped_at_80_columns() {
        let skill = super::render();
        let (_, body) = skill.split_once("\n---\n").unwrap();
        let synopsis = format!("`{} ", env!("CARGO_BIN_NAME"));
        for line in body.lines() {
            if !line.starts_with("    ") && !line.starts_with(&synopsis) {
                assert!(line.chars().count() <= 80, "{line}");
            }
        }
    }

    /// The README's usage lists the steps as the skill does.
    #[test]
    fn the_readme_has_the_steps() {
        let readme = include_str!("../README.md");
        let (_, steps) = super::STEPS.split_once("\n\n").unwrap();
        assert!(readme.contains(&format!("\n\n{steps}\n\n")), "{steps}");
    }

    /// The README's results section is the skill's, word for word, so both
    /// list what each exit code means from the errors themselves.
    #[test]
    fn the_readme_has_the_results() {
        let readme = include_str!("../README.md");
        let section = format!("### Results and exit codes\n\n{}\n\n", super::results());
        assert!(readme.contains(&section), "the README has:\n{section}");
    }

    #[test]
    fn wrap_keeps_code_spans_whole() {
        let text = format!("{} `query strip` and more", ["word"; 14].join(" "));
        assert_eq!(
            super::wrap(&text, 80),
            format!("{}\n`query strip` and more", ["word"; 14].join(" "))
        );
    }

    #[test]
    fn wrap_breaks_between_words() {
        let text = ["word"; 30].join("\n");
        let wrapped = super::wrap(&text, 80);
        let lines: Vec<_> = wrapped.lines().collect();
        assert_eq!(lines.len(), 2, "{wrapped}");
        assert_eq!(lines[0].len(), 79, "{wrapped}");
        assert_eq!(wrapped.split_whitespace().count(), 30, "{wrapped}");
    }
}
