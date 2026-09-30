//! End-to-end checks of what the binary itself does with its command line:
//! exit codes from clap, which document each command reads from stdin, failing
//! before stdin is read, refusing to read stdin from a terminal, blank
//! documents passing from one command to the next in a pipe, warnings printed
//! beside a result or ahead of an error, the exact bytes of stdout: one
//! trailing newline, or nothing for an empty result, what happens when stdout
//! stops taking them, the output conventions every command's help ends with,
//! the help a bare invocation prints, and the skill `skill` prints. Also the
//! claims the help and the skill make of every command at once, which one
//! command's `process()` cannot show: that no command changes a file, which
//! ones keep repeated definitions, that comments are dropped and descriptions
//! kept, and what each kind of failure exits with. Everything else is tested
//! against each module's `process()`.

use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// How long a command gets before it counts as hung.
const TIMEOUT: Duration = Duration::from_secs(10);

const SCHEMA: &str = "type Query { user: User }\ntype User { id: ID name: String }\n";

const QUERY: &str = "{ user { name } }\n";

/// A supergraph of the subgraphs `a` and `b`, for `schema subgraph` and
/// `schema split`.
const SUPERGRAPH: &str = r#"schema @link(url: "https://specs.apollo.dev/link/v1.0") @link(url: "https://specs.apollo.dev/join/v0.5", for: EXECUTION) { query: Query }
enum join__Graph { A @join__graph(name: "a", url: "http://a") B @join__graph(name: "b", url: "http://b") }
type Query @join__type(graph: A) @join__type(graph: B) { user: User @join__field(graph: A) users: [User] @join__field(graph: B) }
type User @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") { id: ID! name: String @join__field(graph: A) }
"#;

/// A file in a scratch directory unique to one test.
fn scratch_file(test: &str, name: &str, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "graphql-document-utils-cli-{}-{test}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn schema_file(test: &str) -> PathBuf {
    scratch_file(test, "schema.graphql", SCHEMA)
}

fn query_file(test: &str) -> PathBuf {
    scratch_file(test, "query.graphql", QUERY)
}

/// Runs the binary with `input` piped to its stdin, which is then closed.
fn run_with_input(args: &[&str], input: &str) -> Output {
    run(args, Stdio::piped(), Some(input))
}

/// Runs the binary with a stdin pipe that is never written to or closed, as an
/// idle pipe would be, so a command that reads stdin never finishes.
fn run_with_open_stdin(args: &[&str]) -> Output {
    run(args, Stdio::piped(), None)
}

/// Runs the binary with a pseudo-terminal as its stdin, as when it is run from
/// a shell with nothing piped in. Nothing is ever typed, so a command that
/// reads stdin never finishes.
///
/// Returns `None`, skipping the test, where no pseudo-terminal can be opened,
/// as in a macOS sandbox that denies them. CI always can, so it fails there
/// instead of letting the test quietly stop running.
#[cfg(unix)]
fn run_with_terminal_stdin(args: &[&str]) -> Option<Output> {
    use rustix::{
        fs::{self, Mode, OFlags},
        pty::{self, OpenptFlags},
    };

    // NOCTTY on both ends, so the test process does not take the terminal as
    // its controlling terminal.
    let controller = match pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY) {
        Ok(controller) => controller,
        Err(e) if std::env::var_os("CI").is_none() => {
            eprintln!("skipping: cannot open a pseudo-terminal: {e}");
            return None;
        }
        Err(e) => panic!("cannot open a pseudo-terminal: {e}"),
    };
    pty::grantpt(&controller).unwrap();
    pty::unlockpt(&controller).unwrap();
    let name = pty::ptsname(&controller, Vec::new()).unwrap();
    let terminal = fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY,
        Mode::empty(),
    )
    .unwrap();

    // The controller stays open until the command exits, since closing it
    // hangs up the terminal and would end a read instead of leaving it waiting.
    let output = run(args, Stdio::from(terminal), None);
    drop(controller);
    Some(output)
}

/// Runs the binary with the given stdin. A piped one has `input` written to it
/// and is closed, or without `input` is kept open and unwritten. Panics if the
/// command is still running after `TIMEOUT`.
fn run(args: &[&str], stdin: Stdio, input: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphql-document-utils"))
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take();
    // The documents here are far smaller than a pipe's buffer, so this never
    // blocks on the command reading them. A command that exits without reading
    // breaks the pipe, which is ignored so the test reports what it printed.
    if let Some(input) = input {
        let mut pipe = stdin.take().expect("input needs a piped stdin");
        let _ = pipe.write_all(input.as_bytes());
    }

    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > TIMEOUT {
            child.kill().unwrap();
            panic!("`{}` hung waiting on stdin", args.join(" "));
        }
        thread::sleep(Duration::from_millis(10));
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Asserts the command succeeded and returns what it printed.
fn success(output: Output) -> String {
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    stdout(&output)
}

#[test]
fn a_schema_piped_in_is_read_as_if_from_its_file() {
    let schema = schema_file("schema-stdin");
    let schema = schema.to_str().unwrap();

    for args in [
        &["schema", "format"][..],
        &["schema", "sort"][..],
        &["schema", "focus", "User"][..],
    ] {
        let from_file = success(run_with_open_stdin(&[args, &["-s", schema]].concat()));
        assert!(!from_file.trim().is_empty(), "{args:?}");
        assert_eq!(success(run_with_input(args, SCHEMA)), from_file, "{args:?}");
        assert_eq!(
            success(run_with_input(&[args, &["-s", "-"]].concat(), SCHEMA)),
            from_file,
            "{args:?}"
        );
    }
}

/// `schema prune` reads either of its documents from stdin, whichever is not
/// given as a file.
#[test]
fn prune_reads_either_document_from_stdin() {
    let schema = schema_file("prune-stdin");
    let schema = schema.to_str().unwrap();
    let query = query_file("prune-stdin");
    let query = query.to_str().unwrap();

    let from_files = success(run_with_open_stdin(&[
        "schema", "prune", "-s", schema, "-q", query,
    ]));
    assert_eq!(
        from_files,
        "type Query {\n  user: User\n}\n\ntype User {\n  name: String\n}\n"
    );

    assert_eq!(
        success(run_with_input(
            &["schema", "prune", "-s", schema, "-q", "-"],
            QUERY
        )),
        from_files
    );
    assert_eq!(
        success(run_with_input(&["schema", "prune", "-q", query], SCHEMA)),
        from_files
    );
}

/// Stdin holds one document, and `-s` defaults to it, so `-q -` alone asks for
/// two. That fails before anything is read, rather than first waiting on a
/// pipe that may never close.
#[test]
fn prune_cannot_read_both_documents_from_stdin() {
    for args in [
        &["schema", "prune", "-q", "-"][..],
        &["schema", "prune", "-s", "-", "-q", "-"][..],
    ] {
        let output = run_with_open_stdin(args);
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert_eq!(
            stderr(&output),
            "error: cannot read both the schema and the query from stdin; pass one of them as a file with -s FILE or -q FILE\n"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn missing_required_arguments_are_a_usage_error() {
    let schema = schema_file("missing-arguments");
    let schema = schema.to_str().unwrap();

    for args in [
        &["query", "focus", "User"][..],
        &["query", "strip", "User"][..],
        &["query", "focus", "-s", schema][..],
        &["query", "strip", "-s", schema][..],
        &["schema", "focus", "-s", schema][..],
        &["schema", "prune", "-s", schema][..],
        &["schema", "prune"][..],
    ] {
        let output = run_with_open_stdin(args);
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("required arguments were not provided"),
            "{}",
            stderr(&output)
        );
    }
}

/// The query passed positionally is taken as a target. It has to fail on
/// that before the command waits on stdin for the query it thinks is coming.
#[test]
fn a_query_file_given_as_a_target_fails_without_reading_stdin() {
    let schema = schema_file("query-as-target");
    let schema = schema.to_str().unwrap();

    for command in ["focus", "strip"] {
        let output =
            run_with_open_stdin(&["query", command, "-s", schema, "query.graphql", "User"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "error: unknown target 'query.graphql'; pass query files with -q: -q query.graphql\n"
        );
        assert!(output.stdout.is_empty());
    }
}

/// A schema file passed positionally is taken as a type. `schema focus` has
/// to fail on that before it waits on stdin for the schema.
#[test]
fn a_schema_file_given_as_a_type_fails_without_reading_stdin() {
    let output = run_with_open_stdin(&["schema", "focus", "schema.graphql", "User"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "error: unknown target 'schema.graphql'; pass the schema file with -s: -s schema.graphql\n"
    );
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn a_query_from_a_terminal_is_a_usage_error() {
    let schema = schema_file("terminal-query");
    let schema = schema.to_str().unwrap();

    for args in [
        &["query", "normalize"][..],
        &["query", "normalize", "-q", "-"][..],
        &["query", "focus", "-s", schema, "User"][..],
        &["query", "strip", "-s", schema, "User.name"][..],
        &["schema", "prune", "-s", schema, "-q", "-"][..],
    ] {
        let Some(output) = run_with_terminal_stdin(args) else {
            return;
        };
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert_eq!(
            stderr(&output),
            "error: no query given; pass -q FILE or pipe one on stdin\n"
        );
        assert!(output.stdout.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn a_schema_from_a_terminal_is_a_usage_error() {
    let query = query_file("terminal-schema");
    let query = query.to_str().unwrap();

    for args in [
        &["schema", "format"][..],
        &["schema", "sort", "-s", "-"][..],
        &["schema", "focus", "User"][..],
        &["schema", "prune", "-q", query][..],
        &["schema", "subgraph", "a"][..],
        &["schema", "split", "-o", "never-made"][..],
    ] {
        let Some(output) = run_with_terminal_stdin(args) else {
            return;
        };
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert_eq!(
            stderr(&output),
            "error: no schema given; pass -s FILE or pipe one on stdin\n"
        );
        assert!(output.stdout.is_empty());
    }
}

/// Targets are checked before the query is read, so a bad one is reported
/// even when there is no query to read.
#[cfg(unix)]
#[test]
fn a_bad_target_wins_over_a_query_from_a_terminal() {
    let schema = schema_file("terminal-bad-target");
    let schema = schema.to_str().unwrap();

    let Some(output) = run_with_terminal_stdin(&["query", "focus", "-s", schema, "Usr"]) else {
        return;
    };
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("error: unknown type `Usr` in"),
        "{}",
        stderr(&output)
    );
}

/// A blank document is what a command emits when nothing survives, so every
/// command that transforms a schema passes one through as empty, from a file
/// or from stdin, rather than failing to parse it.
#[test]
fn a_blank_schema_passes_through_every_schema_command() {
    let blank = scratch_file("blank-schema", "schema.graphql", "\n# nothing\n");
    let blank = blank.to_str().unwrap();
    let query = query_file("blank-schema");
    let query = query.to_str().unwrap();
    let out = PathBuf::from(blank).with_file_name("out");
    let out = out.to_str().unwrap();

    for args in [
        &["schema", "format"][..],
        &["schema", "sort"][..],
        &["schema", "focus", "User"][..],
        &["schema", "prune", "-q", query][..],
        &["schema", "subgraph", "a"][..],
        &["schema", "split", "-o", out][..],
    ] {
        for output in [
            run_with_open_stdin(&[args, &["-s", blank]].concat()),
            run_with_input(args, ""),
        ] {
            assert_eq!(stderr(&output), "", "{args:?}");
            assert_eq!(success(output), "", "{args:?}");
        }
    }
    // `schema split` wrote nothing, not even its directory.
    assert!(!PathBuf::from(out).exists());
}

/// `schema focus` emits nothing only for a blank schema, which the next
/// command in the pipe then has to accept.
#[test]
fn an_empty_focus_pipes_into_format() {
    let focused = success(run_with_input(&["schema", "focus", "User"], ""));
    assert_eq!(focused, "");
    let output = run_with_input(&["schema", "format"], &focused);
    assert_eq!(stderr(&output), "");
    assert_eq!(success(output), "");
}

/// A query `query strip` removed everything from prunes the schema to nothing,
/// with a note, since the schema itself was not empty.
#[test]
fn a_blank_query_prunes_to_nothing_with_a_note() {
    let schema = schema_file("blank-prune-query");
    let schema = schema.to_str().unwrap();

    let stripped = success(run_with_input(
        &["query", "strip", "-s", schema, "User"],
        QUERY,
    ));
    let output = run_with_input(&["schema", "prune", "-s", schema, "-q", "-"], &stripped);
    assert_eq!(
        stderr(&output),
        "note: the query is empty; output is empty\n"
    );
    assert_eq!(success(output), "");
}

/// A name defined twice is invalid GraphQL, but not worth failing over: each
/// document's repeats get one warning on stderr, the command uses the first
/// definition and succeeds, and stdout holds only the document.
#[test]
fn a_repeated_definition_warns_and_the_first_is_used() {
    let schema = scratch_file(
        "repeated-definition",
        "schema.graphql",
        "type Query { user: User }\ntype User { id: ID }\ntype User { name: String }\n",
    );
    let schema = schema.to_str().unwrap();
    let query = "{ user { ...F } }\nfragment F on User { id }\nfragment F on User { name }\n";

    let output = run_with_input(&["schema", "prune", "-s", schema, "-q", "-"], query);
    assert_eq!(
        stderr(&output),
        format!(
            "warning: type `User` is defined more than once in '{schema}' (at 2:1, 3:1); only the first is used\n\
             warning: fragment `F` is defined more than once in (stdin) (at 2:1, 3:1); only the first is used\n"
        )
    );
    assert_eq!(
        success(output),
        "type Query {\n  user: User\n}\n\ntype User {\n  id: ID\n}\n"
    );

    // Formatting only lays the schema out, so it keeps both, and says nothing.
    let output = run(&["schema", "format", "-s", schema], Stdio::null(), None);
    assert_eq!(stderr(&output), "");
    assert_eq!(success(output).matches("type User").count(), 2);
}

/// A warning is printed even when the command then fails, and before the
/// error, since the repeat can be what the error is about.
#[test]
fn a_warning_comes_before_the_error_it_explains() {
    let schema = scratch_file(
        "repeated-definition-error",
        "schema.graphql",
        "type Query { user: User }\ntype User { id: ID }\ntype User { email: String }\n",
    );
    let schema = schema.to_str().unwrap();

    let output = run_with_input(&["query", "strip", "-s", schema, "User.email"], QUERY);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        format!(
            "warning: type `User` is defined more than once in '{schema}' (at 2:1, 3:1); only the first is used\n\
             error: `User` has no field `email`\n"
        )
    );
}

/// The query commands resolve the query against their schema, so a blank one
/// is the wrong file, not a document to pass through, even with a blank query.
#[test]
fn a_blank_schema_fails_the_query_commands() {
    let blank = scratch_file("blank-query-schema", "schema.graphql", "");
    let blank = blank.to_str().unwrap();

    for command in ["focus", "strip"] {
        for input in [QUERY, ""] {
            let output = run_with_input(&["query", command, "-s", blank, "User"], input);
            assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
            assert_eq!(
                stderr(&output),
                format!("error: schema '{blank}' is empty; pass the schema the query is written against with -s\n")
            );
            assert!(output.stdout.is_empty());
        }
    }
}

/// Every command ends a document in exactly one newline, whatever the code
/// that printed it ends with: graphql-parser's `Display` ends in one, and
/// `minify_query` does not.
#[test]
fn a_document_ends_in_exactly_one_newline() {
    let schema = schema_file("one-newline");
    let schema = schema.to_str().unwrap();
    let query = query_file("one-newline");
    let query = query.to_str().unwrap();
    let formatted_query = "{\n  user {\n    name\n  }\n}\n";
    let formatted_schema =
        "type Query {\n  user: User\n}\n\ntype User {\n  id: ID\n  name: String\n}\n";

    for (args, input, expected) in [
        (&["query", "normalize"][..], QUERY, formatted_query),
        (
            &["query", "normalize", "--minify"][..],
            QUERY,
            "{user{name}}\n",
        ),
        (
            &["query", "focus", "-s", schema, "User"][..],
            QUERY,
            formatted_query,
        ),
        (
            &["query", "strip", "-s", schema, "User.id"][..],
            "{ user { id name } }",
            formatted_query,
        ),
        (&["schema", "format"][..], SCHEMA, formatted_schema),
        (&["schema", "sort"][..], SCHEMA, formatted_schema),
        (
            &["schema", "focus", "User"][..],
            SCHEMA,
            "type User {\n  id: ID\n  name: String\n}\n",
        ),
        (
            &["schema", "prune", "-q", query][..],
            SCHEMA,
            "type Query {\n  user: User\n}\n\ntype User {\n  name: String\n}\n",
        ),
    ] {
        assert_eq!(success(run_with_input(args, input)), expected, "{args:?}");
    }
}

/// A result with nothing left in it is zero bytes, not a blank line, so a
/// script can test stdout for empty.
#[test]
fn an_empty_result_prints_nothing() {
    let schema = schema_file("empty-result");
    let schema = schema.to_str().unwrap();

    for (args, input) in [
        (
            &["query", "focus", "-s", schema, "User"][..],
            "{ __typename }",
        ),
        (&["query", "strip", "-s", schema, "User"][..], QUERY),
        (&["query", "strip", "-s", schema, "User.name"][..], QUERY),
        (&["query", "normalize"][..], "# nothing\n"),
        (&["query", "normalize", "--minify"][..], "# nothing\n"),
    ] {
        assert_eq!(success(run_with_input(args, input)), "", "{args:?}");
    }
}

/// Running a command on its own output changes nothing, byte for byte, so a
/// pipeline can apply it again safely.
#[test]
fn a_second_pass_changes_nothing() {
    let schema = schema_file("second-pass");
    let schema = schema.to_str().unwrap();
    let query = "query B($id: ID) { user { name id } }\nquery A { user { id } }\nfragment F on User { name }\n";

    for (args, input) in [
        (&["query", "normalize"][..], query),
        (&["query", "normalize", "--minify"][..], query),
        (&["query", "focus", "-s", schema, "User.name"][..], query),
        (&["query", "strip", "-s", schema, "User.id"][..], query),
        (&["schema", "format"][..], SCHEMA),
        (&["schema", "sort"][..], SCHEMA),
    ] {
        let once = success(run_with_input(args, input));
        assert!(!once.is_empty(), "{args:?}");
        assert_eq!(success(run_with_input(args, &once)), once, "{args:?}");
    }
}

/// Every command's help, top-level, group, or leaf, and short or long, ends
/// with what stdout holds, where diagnostics go, and what the exit codes mean.
#[test]
fn every_help_states_the_output_conventions() {
    for command in [
        &[][..],
        &["query"][..],
        &["query", "normalize"][..],
        &["query", "focus"][..],
        &["query", "strip"][..],
        &["schema"][..],
        &["schema", "format"][..],
        &["schema", "focus"][..],
        &["schema", "prune"][..],
        &["schema", "sort"][..],
        &["schema", "split"][..],
        &["schema", "subgraph"][..],
    ] {
        for flag in ["-h", "--help"] {
            let help = success(run_with_open_stdin(&[command, &[flag]].concat()));
            for convention in [
                "Output is a GraphQL document on stdout ending in one newline",
                "Errors, warnings, and notes go to stderr.",
                "Exit codes: 0 on success",
            ] {
                assert!(help.contains(convention), "{command:?} {flag}:\n{help}");
            }
            assert!(
                help.ends_with("\n\nExit codes: 0 on success, including a valid target that matches nothing; 1 for\na bad input (unreadable, invalid, unknown target) or a failed write; 2 for a\nusage error.\n"),
                "{command:?} {flag} ends with the conventions:\n{help}"
            );
        }
    }
}

/// The tool or a noun run on its own is asking what it can do, so it prints
/// the help `-h` would, to stdout, with nothing on stderr, and exits 0, without
/// reading the stdin it is given.
#[test]
fn a_bare_invocation_prints_the_help() {
    for command in [&[][..], &["query"][..], &["schema"][..]] {
        let help = success(run_with_open_stdin(&[command, &["-h"]].concat()));
        let output = run_with_open_stdin(command);
        assert_eq!(stderr(&output), "", "{command:?}");
        assert_eq!(success(output), help, "{command:?}");
    }

    let help = success(run_with_open_stdin(&[]));
    assert!(help.contains("\nCommon tasks:\n"), "{help}");
    for noun in ["query", "schema"] {
        let verbs = success(run_with_open_stdin(&[noun]));
        assert!(verbs.contains("\nCommands:\n"), "{noun}:\n{verbs}");
    }
}

/// `skill` prints the skill through the same path as any document: to stdout,
/// ending in one newline, with nothing on stderr, and without reading the
/// stdin it is given. What it says is tested in `skill`. Its help leaves out
/// the output conventions, which describe the commands that print GraphQL.
#[test]
fn skill_prints_the_skill() {
    let output = run_with_open_stdin(&["skill"]);
    assert_eq!(stderr(&output), "");
    let skill = success(output);
    assert!(
        skill.starts_with("---\nname: graphql-document-utils\n"),
        "{skill}"
    );
    assert!(skill.ends_with("stdin.\n"), "{skill}");

    for flag in ["-h", "--help"] {
        let help = success(run_with_open_stdin(&["skill", flag]));
        assert!(help.contains("Agent Skill"), "{flag}:\n{help}");
        assert!(
            !help.contains("Output is a GraphQL document"),
            "{flag}:\n{help}"
        );
    }
}

/// Anything else clap cannot parse is a usage error too.
#[test]
fn an_unknown_command_or_flag_is_a_usage_error() {
    for (args, message) in [
        (&["frob"][..], "unrecognized subcommand"),
        (&["schema", "format", "--frob"][..], "unexpected argument"),
    ] {
        let output = run_with_open_stdin(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            stderr(&output).contains(message),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

/// An input that is wrong, rather than the command line, exits 1 with one
/// `error:` line and nothing on stdout.
#[test]
fn bad_input_exits_1() {
    let schema = schema_file("bad-input");
    let schema = schema.to_str().unwrap();

    for (args, input, error) in [
        (
            &["schema", "format", "-s", "nope.graphql"][..],
            "",
            "error: cannot read schema 'nope.graphql': No such file or directory\n",
        ),
        (
            &["schema", "format"][..],
            "type Query {",
            "error: failed to parse schema (stdin) at 1:13: unexpected end of input; expected Name\n",
        ),
        (
            &["query", "focus", "-s", schema, "User."][..],
            QUERY,
            "error: invalid target `User.`; expected `Type` or `Type.field`\n",
        ),
        (
            &["query", "strip", "-s", schema, "Usr"][..],
            QUERY,
            &format!("error: unknown type `Usr` in '{schema}'; did you mean `User`?\n"),
        ),
        (
            &["schema", "subgraph", "a"][..],
            SCHEMA,
            "error: schema (stdin) is not a supergraph; `schema subgraph` and `schema split` take one composed by Apollo Federation 2, whose `schema` links the join spec with `@link`\n",
        ),
        (
            &["schema", "subgraph", "A"][..],
            SUPERGRAPH,
            "error: unknown subgraph `A` in (stdin); did you mean `a`? Its subgraphs are `a` and `b`\n",
        ),
    ] {
        let output = run_with_input(args, input);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert_eq!(stderr(&output), error, "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

/// Only descriptions are part of a document; `#` comments are not, so no
/// command keeps them.
#[test]
fn comments_are_dropped_and_descriptions_kept() {
    let schema = scratch_file(
        "comments",
        "schema.graphql",
        "# The schema.\n\"The root.\"\ntype Query {\n  # Who asks.\n  user: User\n}\n\n\"A user.\"\ntype User { id: ID name: String }\n",
    );
    let schema = schema.to_str().unwrap();
    let query = "# The query.\n{\n  user {\n    # Who they are.\n    name\n    id\n  }\n}\n";

    for (args, input, descriptions) in [
        (&["query", "normalize"][..], query, &[][..]),
        (&["query", "focus", "-s", schema, "User"][..], query, &[]),
        (&["query", "strip", "-s", schema, "User.id"][..], query, &[]),
        (
            &["schema", "format", "-s", schema][..],
            "",
            &["\"The root.\"", "\"A user.\""],
        ),
        (
            &["schema", "sort", "-s", schema][..],
            "",
            &["\"The root.\"", "\"A user.\""],
        ),
        (
            &["schema", "focus", "-s", schema, "User"][..],
            "",
            &["\"A user.\""],
        ),
        (
            &["schema", "prune", "-s", schema, "-q", "-"][..],
            query,
            &["\"The root.\"", "\"A user.\""],
        ),
    ] {
        let output = success(run_with_input(args, input));
        assert!(output.contains("name"), "{args:?}:\n{output}");
        assert!(!output.contains('#'), "{args:?}:\n{output}");
        for description in descriptions {
            assert!(output.contains(description), "{args:?}:\n{output}");
        }
    }
}

/// A name defined twice warns, and only the first definition is used, except
/// by the commands that only lay a document out, which keep both and say
/// nothing.
#[test]
fn only_the_commands_that_lay_a_document_out_keep_repeats() {
    let schema = scratch_file(
        "repeats-per-command",
        "schema.graphql",
        "type Query { user: User }\ntype User { id: ID }\ntype User { name: String }\n",
    );
    let schema = schema.to_str().unwrap();
    let query = "{ user { ...F } }\nfragment F on User { id }\nfragment F on User { name }\n";

    for (args, input, repeated, keeps_both) in [
        (&["query", "normalize"][..], query, "fragment F", true),
        (
            &["query", "focus", "-s", schema, "User"][..],
            query,
            "fragment F",
            false,
        ),
        (
            &["query", "strip", "-s", schema, "Query"][..],
            query,
            "fragment F",
            false,
        ),
        (
            &["schema", "format", "-s", schema][..],
            "",
            "type User",
            true,
        ),
        (&["schema", "sort", "-s", schema][..], "", "type User", true),
        (
            &["schema", "focus", "-s", schema, "User"][..],
            "",
            "type User",
            false,
        ),
        (
            &["schema", "prune", "-s", schema, "-q", "-"][..],
            query,
            "type User",
            false,
        ),
    ] {
        let output = run_with_input(args, input);
        let stderr = stderr(&output);
        let printed = success(output);
        let copies = if keeps_both { 2 } else { 1 };
        assert_eq!(
            printed.matches(repeated).count(),
            copies,
            "{args:?}:\n{printed}"
        );
        let warnings: Vec<_> = stderr
            .lines()
            .filter(|line| line.starts_with("warning:"))
            .collect();
        assert_eq!(warnings.is_empty(), keeps_both, "{args:?}:\n{stderr}");
        for warning in warnings {
            assert!(
                warning.ends_with("only the first is used"),
                "{args:?}: {warning}"
            );
        }
    }
}

/// `schema format` lays a schema out in the order it is written.
#[test]
fn format_keeps_the_source_order() {
    let schema = "type User { id: ID }\ntype Query { user: User }\n";
    assert_eq!(
        success(run_with_input(&["schema", "format"], schema)),
        "type User {\n  id: ID\n}\n\ntype Query {\n  user: User\n}\n"
    );
}

/// A target that matches nothing is not an error: exit 0, the document on
/// stdout, and the reason on stderr.
#[test]
fn a_target_that_matches_nothing_gets_a_note() {
    let schema = schema_file("matches-nothing");
    let schema = schema.to_str().unwrap();

    let output = run_with_input(&["query", "focus", "-s", schema, "User"], "{ __typename }");
    assert_eq!(
        stderr(&output),
        "note: no selection reaches `User`; output is empty\n"
    );
    assert_eq!(success(output), "");

    let output = run_with_input(&["query", "strip", "-s", schema, "User.id"], QUERY);
    assert_eq!(
        stderr(&output),
        "note: nothing in the query matches `User.id`; nothing was stripped\n"
    );
    assert_eq!(success(output), "{\n  user {\n    name\n  }\n}\n");
}

#[test]
fn an_empty_strip_pipes_into_normalize() {
    let schema = schema_file("empty-strip");
    let schema = schema.to_str().unwrap();

    let stripped = success(run_with_input(
        &["query", "strip", "-s", schema, "User"],
        QUERY,
    ));
    assert_eq!(stripped, "");
    let output = run_with_input(&["query", "normalize"], &stripped);
    assert_eq!(stderr(&output), "");
    assert_eq!(success(output), "");
}

/// `schema focus` on a type the query root does not reach drops the root, so
/// pruning its output for a query keeps nothing: prune first, then focus.
#[test]
fn focusing_before_pruning_leaves_nothing() {
    let query = query_file("focus-then-prune");
    let query = query.to_str().unwrap();

    let focused = success(run_with_input(&["schema", "focus", "User"], SCHEMA));
    assert!(!focused.contains("Query"), "{focused}");
    let output = run_with_input(&["schema", "prune", "-q", query], &focused);
    assert_eq!(stderr(&output), "");
    assert_eq!(success(output), "");
}

/// Every command but `schema split` only prints: the files it reads are left
/// as they were, and none is added beside them. `schema split` writes only
/// where it is told to (`split_writes_only_inside_its_directory`).
#[test]
fn no_command_changes_a_file() {
    let schema = schema_file("changes-no-file");
    let query = query_file("changes-no-file");
    let supergraph = scratch_file("changes-no-file", "supergraph.graphql", SUPERGRAPH);
    let dir = schema.parent().unwrap();
    let listing = || {
        let mut files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let contents = fs::read(&path).unwrap();
                (path, contents)
            })
            .collect();
        files.sort();
        files
    };
    let before = listing();
    let (schema, query) = (schema.to_str().unwrap(), query.to_str().unwrap());
    let supergraph = supergraph.to_str().unwrap();

    for args in [
        &["query", "normalize", "-q", query][..],
        &["query", "focus", "-s", schema, "-q", query, "User"][..],
        &["query", "strip", "-s", schema, "-q", query, "User.id"][..],
        &["schema", "format", "-s", schema][..],
        &["schema", "sort", "-s", schema][..],
        &["schema", "focus", "-s", schema, "User"][..],
        &["schema", "prune", "-s", schema, "-q", query][..],
        &["schema", "subgraph", "-s", supergraph, "a"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_graphql-document-utils"))
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(!output.stdout.is_empty(), "{args:?}");
    }
    assert_eq!(listing(), before);
}

/// `schema split` writes a file per subgraph into the directory `-o` names,
/// relative to where it runs, and nothing else: its input is left as it was,
/// nothing goes beside it, and stdout stays empty.
#[test]
fn split_writes_only_inside_its_directory() {
    let supergraph = scratch_file("split-writes", "supergraph.graphql", SUPERGRAPH);
    let dir = supergraph.parent().unwrap();
    let _ = fs::remove_dir_all(dir.join("out"));

    let output = Command::new(env!("CARGO_BIN_EXE_graphql-document-utils"))
        .args(["schema", "split", "-s", "supergraph.graphql", "-o", "out"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "note: wrote out/a.graphql\nnote: wrote out/b.graphql\n"
    );
    let mut beside: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    beside.sort();
    assert_eq!(beside, ["out", "supergraph.graphql"]);
    assert_eq!(fs::read_to_string(&supergraph).unwrap(), SUPERGRAPH);
    let mut written: Vec<_> = fs::read_dir(dir.join("out"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    written.sort();
    assert_eq!(written, ["a.graphql", "b.graphql"]);
    let a = fs::read_to_string(dir.join("out/a.graphql")).unwrap();
    assert!(
        a.contains("type User @key(fields: \"id\") {\n  id: ID!\n  name: String\n}"),
        "{a}"
    );
}

/// A command reads a pipe until it closes, however long that takes, as a
/// filter does; only a terminal is refused. So a stdin left open and unwritten
/// is waited on.
#[test]
fn a_command_waits_on_an_open_stdin() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphql-document-utils"))
        .args(["schema", "format"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "exited before stdin closed"
    );

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(SCHEMA.as_bytes()).unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(stderr(&output), "");
    assert!(
        stdout(&output).starts_with("type Query {"),
        "{}",
        stdout(&output)
    );
}

/// A noun can be run without a verb, but not with one it does not have, which
/// stays a usage error rather than printing the noun's help.
#[test]
fn an_unknown_verb_is_a_usage_error() {
    for args in [&["query", "prune"][..], &["schema", "normalize"][..]] {
        let output = run_with_open_stdin(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            stderr(&output).contains("unrecognized subcommand"),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

/// A reader that stops early, like `| head -1`, closes the pipe before the
/// document is written. Like any Unix filter, the command then stops quietly
/// and succeeds, rather than panicking on the failed write.
#[test]
fn a_reader_closing_stdout_early_is_not_an_error() {
    // Far larger than a pipe's buffer once normalized, so the command is still
    // writing when the pipe closes, however the two processes are scheduled.
    let fields: Vec<String> = (0..20_000)
        .map(|i| format!("f{i}: user {{ name }}"))
        .collect();
    let query = scratch_file(
        "closed-stdout",
        "query.graphql",
        &format!("{{ {} }}", fields.join(" ")),
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_graphql-document-utils"))
        .args(["query", "normalize", "-q", query.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut start = [0; 16];
    stdout.read_exact(&mut start).unwrap();
    assert_eq!(&start[..1], b"{");
    drop(stdout);

    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > TIMEOUT {
            child.kill().unwrap();
            panic!("`query normalize` hung writing to a closed pipe");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// Any other failure to write stdout, such as a full disk, is an error: one
/// line on stderr and exit 1, not a panic. Only Linux has `/dev/full` to make
/// every write fail that way.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_write_is_an_error() {
    let full = fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphql-document-utils"))
        .args(["query", "normalize"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(QUERY.as_bytes())
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert_eq!(
        stderr(&output),
        "error: cannot write output: No space left on device\n"
    );
    assert_eq!(output.status.code(), Some(1));
}
