//! Every command the help and the README show, run as the shell would run it
//! against the files in `tests/fixtures`, so what they show is what the tool
//! does. Each leaf's examples, the common tasks, and the README's other
//! commands are snapshotted with `insta`: what every stage of a pipeline exits
//! with and prints to stderr, and what the last one prints to stdout. A change
//! in behavior then shows up as a change to a snapshot, next to the help it
//! makes wrong.
//!
//! The README's sample documents are the fixtures, byte for byte, and each
//! output it shows is what the command above it prints, so neither can go
//! stale.
//!
//! The commands come from the binary's help, as a user reads them, and from
//! the README, through the same `extract` the tests in `docs` parse them with.

#[path = "../src/docs/extract.rs"]
mod extract;

use std::{
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
};

use extract::{Fence, Stage};

/// Where the commands run, so the files they name are the fixtures.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

const README: &str = include_str!("../README.md");

/// Runs the binary in the fixtures directory with `input`, if any, piped to
/// its stdin, which is otherwise empty. Not a terminal, so a command that
/// reads stdin when an example meant it not to prints nothing rather than
/// failing, which the non-empty check in `transcript` catches.
fn spawn(args: &[&str], input: Option<Vec<u8>>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphql-document-utils"))
        .args(args)
        .current_dir(FIXTURES)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Written from another thread, so a command that prints before it has
    // read all of its input cannot fill stdout's pipe and wait on this one.
    // One that exits without reading breaks the pipe, which is ignored so the
    // test reports what it printed.
    let writer = input.map(|input| {
        let mut stdin = child.stdin.take().unwrap();
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let output = child.wait_with_output().unwrap();
    if let Some(writer) = writer {
        writer.join().unwrap();
    }
    output
}

/// The files `cat` reads for `pattern`, relative to the fixtures. A `*` in the
/// file name matches as the shell's would, in sorted order, and has to match
/// something.
fn glob(pattern: &str) -> Vec<PathBuf> {
    let path = Path::new(FIXTURES).join(pattern);
    let name = path.file_name().unwrap().to_str().unwrap();
    let Some((prefix, suffix)) = name.split_once('*') else {
        return vec![path];
    };
    let mut matches: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            name.len() >= prefix.len() + suffix.len()
                && name.starts_with(prefix)
                && name.ends_with(suffix)
        })
        .collect();
    matches.sort();
    assert!(!matches.is_empty(), "`{pattern}` matches a fixture");
    matches
}

/// What a command printed: the status of each invocation of the binary in it,
/// their stderr in order, and the last stage's stdout.
struct Run {
    statuses: Vec<i32>,
    stderr: String,
    stdout: String,
}

/// Runs `command` as the shell would, one stage at a time, each stage's stdout
/// piped to the next one's stdin. A `>` is left to the transcript, which
/// captures what would have been written, so nothing is.
fn run(command: &str) -> Run {
    let mut piped: Option<Vec<u8>> = None;
    let mut statuses = Vec::new();
    let mut stderr = String::new();
    for stage in extract::stages(command) {
        piped = Some(match stage {
            Stage::Cat(patterns) => {
                assert!(piped.is_none(), "`cat` starts the pipeline: {command}");
                let files = patterns.iter().flat_map(|pattern| glob(pattern));
                files.flat_map(|file| fs::read(file).unwrap()).collect()
            }
            Stage::Bin(args) => {
                let output = spawn(&args, piped.take());
                statuses.push(output.status.code().expect("the command exits"));
                stderr.push_str(&String::from_utf8(output.stderr).unwrap());
                output.stdout
            }
        });
    }
    // The version the skill is stamped with is made generic, so a release,
    // which bumps it before running the tests, does not change a snapshot.
    let version = format!("{} {}", extract::BIN, env!("CARGO_PKG_VERSION"));
    let stdout = String::from_utf8(piped.unwrap()).unwrap();
    Run {
        statuses,
        stderr,
        stdout: stdout.replace(&version, &format!("{} [version]", extract::BIN)),
    }
}

/// Runs each of `commands` and writes down what it did, for a snapshot. Every
/// one has to succeed and print something, since an example shows what the
/// tool is for. A command printing what one before it printed says so rather
/// than repeating it, which also shows the two agree.
fn transcript(commands: &[String]) -> String {
    let mut transcript = String::new();
    let mut printed: Vec<(&str, String)> = Vec::new();
    for command in commands {
        let Run {
            statuses,
            stderr,
            stdout,
        } = run(command);
        assert!(
            statuses.iter().all(|status| *status == 0),
            "{command}\nexits {statuses:?}:\n{stderr}"
        );
        assert!(!stdout.is_empty(), "{command}\nprints something:\n{stderr}");

        let statuses: Vec<_> = statuses.iter().map(ToString::to_string).collect();
        writeln!(transcript, "$ {command}\nexit: {}", statuses.join(" | ")).unwrap();
        if !stderr.is_empty() {
            write!(transcript, "stderr:\n{stderr}").unwrap();
        }
        if stdout.trim_end() == skill().trim_end() {
            writeln!(transcript, "stdout: the skill, as in {SKILL_SNAPSHOT}").unwrap();
        } else if let Some((earlier, _)) = printed.iter().find(|(_, earlier)| *earlier == stdout) {
            writeln!(transcript, "stdout: as for `{earlier}`").unwrap();
        } else {
            write!(transcript, "stdout:\n{stdout}").unwrap();
        }
        transcript.push('\n');
        printed.push((command, stdout));
    }
    transcript
}

/// The snapshot of the skill the tests of `skill` take, which is what `skill`
/// prints, so a transcript points to it rather than holding a second copy.
const SKILL_SNAPSHOT: &str = "src/snapshots/graphql_document_utils__skill__tests__snapshot.snap";

/// The skill as its snapshot holds it, after the snapshot's header.
fn skill() -> String {
    let snapshot = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(SKILL_SNAPSHOT));
    let snapshot = snapshot.unwrap();
    let (_, skill) = snapshot
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .unwrap_or_else(|| panic!("{SKILL_SNAPSHOT} has a header"));
    skill.to_string()
}

/// What `-h` prints for the command `path` names.
fn help(path: &[String]) -> String {
    let mut args: Vec<&str> = path.iter().map(String::as_str).collect();
    args.push("-h");
    let output = spawn(&args, None);
    assert_eq!(output.status.code(), Some(0), "{path:?} -h");
    String::from_utf8(output.stdout).unwrap()
}

/// The part of `help` from the line `heading` up to the next blank line.
fn section(help: &str, heading: &str) -> Option<String> {
    let (_, rest) = help.split_once(&format!("\n{heading}\n"))?;
    let lines = rest.lines().take_while(|line| !line.is_empty());
    Some(lines.fold(format!("{heading}\n"), |section, line| {
        section + line + "\n"
    }))
}

/// Every leaf command, as the words that name it, with its examples, found by
/// walking the `Commands:` lists of the help from the top.
fn leaf_examples() -> Vec<(Vec<String>, Vec<String>)> {
    fn walk(path: Vec<String>, leaves: &mut Vec<(Vec<String>, Vec<String>)>) {
        let help = help(&path);
        let Some(commands) = section(&help, "Commands:") else {
            let examples = section(&help, "Examples:");
            let examples = examples.unwrap_or_else(|| panic!("{path:?} has examples:\n{help}"));
            leaves.push((path, extract::examples(examples.trim_end())));
            return;
        };
        for line in commands.lines().skip(1) {
            let name = line.split_whitespace().next().unwrap();
            if name != "help" {
                walk([path.clone(), vec![name.to_string()]].concat(), leaves);
            }
        }
    }
    let mut leaves = Vec::new();
    walk(Vec::new(), &mut leaves);
    assert!(leaves.len() > 1, "{leaves:?}");
    leaves
}

/// The commands of the top-level help's common tasks.
fn common_tasks() -> Vec<String> {
    let help = help(&[]);
    let (_, table) = help
        .split_once("\nCommon tasks:\n")
        .unwrap_or_else(|| panic!("the help has common tasks:\n{help}"));
    let (tasks, _) = extract::tasks(&format!("Common tasks:\n{table}"));
    assert!(!tasks.is_empty());
    tasks.into_iter().map(|(_, command)| command).collect()
}

/// `skill` prints the skill its tests snapshot, so the transcripts of the
/// commands that print it can say so rather than repeat it. After a change to
/// the skill, accept that snapshot first.
#[test]
fn skill_prints_the_snapshotted_skill() {
    let run = run(&format!("{} skill", extract::BIN));
    pretty_assertions::assert_eq!(run.stdout.trim_end(), skill().trim_end());
}

#[test]
fn every_example_runs() {
    for (path, examples) in leaf_examples() {
        insta::assert_snapshot!(path.join("_"), transcript(&examples));
    }
}

#[test]
fn every_common_task_runs() {
    insta::assert_snapshot!("common_tasks", transcript(&common_tasks()));
}

/// The README's commands the help does not show too; those it does are in the
/// help's snapshots.
#[test]
fn every_readme_command_runs() {
    let shown: Vec<String> = leaf_examples()
        .into_iter()
        .flat_map(|(_, examples)| examples)
        .chain(common_tasks())
        .collect();
    let commands: Vec<String> = extract::readme_commands(README)
        .into_iter()
        .filter(|command| !shown.contains(command))
        .collect();
    assert!(!commands.is_empty());
    insta::assert_snapshot!("readme", transcript(&commands));
}

/// A document the README introduces by a fixture's name, as in "this query,
/// `query.graphql`:", is that fixture, so the outputs it shows are the ones
/// the fixtures give.
#[test]
fn the_readme_samples_are_the_fixtures() {
    let mut samples = 0;
    for fence in extract::fences(README) {
        if fence.info != "graphql" {
            continue;
        }
        let intro = fence.before.trim_end();
        let Some(name) = intro
            .strip_suffix("`:")
            .and_then(|intro| intro.rsplit_once('`'))
            .map(|(_, name)| name)
        else {
            continue;
        };
        let fixture = fs::read_to_string(Path::new(FIXTURES).join(name))
            .unwrap_or_else(|error| panic!("the README shows `{name}`, a fixture: {error}"));
        pretty_assertions::assert_eq!(fence.body, fixture, "the README's `{name}`");
        samples += 1;
    }
    assert!(samples >= 2, "the README shows the schema and the query");
}

/// A `graphql` block right after a `bash` block of one command is what that
/// command prints, exactly.
#[test]
fn the_readme_shows_what_its_commands_print() {
    let fences = extract::fences(README);
    let shown: Vec<(&Fence, &Fence)> = fences
        .iter()
        .zip(&fences[1..])
        .filter(|(command, output)| {
            command.info == "bash" && output.info == "graphql" && output.before.trim().is_empty()
        })
        .collect();
    assert!(!shown.is_empty(), "the README shows output");
    for (command, output) in shown {
        let commands = extract::shell_commands(&command.body);
        let [command] = commands.as_slice() else {
            panic!("a block whose output is shown has one command:\n{commands:?}");
        };
        let run = run(command);
        assert_eq!(run.statuses.last(), Some(&0), "{command}\n{}", run.stderr);
        pretty_assertions::assert_eq!(run.stdout, output.body, "the output of `{command}`");
    }
}
