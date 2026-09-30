//! End-to-end checks of what the binary itself does with its command line:
//! exit codes from clap, which document each command reads from stdin, failing
//! before stdin is read, refusing to read stdin from a terminal, blank
//! documents passing from one command to the next in a pipe, warnings printed
//! beside a result or ahead of an error, the exact bytes of stdout: one
//! trailing newline, or nothing for an empty result, what happens when stdout
//! stops taking them, and the output conventions every command's help ends
//! with.
//! Everything else is tested against each module's `process()`.

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

    for args in [
        &["schema", "format"][..],
        &["schema", "sort"][..],
        &["schema", "focus", "User"][..],
        &["schema", "prune", "-q", query][..],
    ] {
        for output in [
            run_with_open_stdin(&[args, &["-s", blank]].concat()),
            run_with_input(args, ""),
        ] {
            assert_eq!(stderr(&output), "", "{args:?}");
            assert_eq!(success(output), "", "{args:?}");
        }
    }
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
        }
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
