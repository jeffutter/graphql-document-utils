//! The commands the help and the README show, read out of the text they are
//! written in: an examples block, the common tasks table, and the README's
//! fenced code blocks. Shared by the tests in `docs`, which parse every command
//! as the CLI's own command line, and by `tests/examples.rs`, which runs every
//! one against the files in `tests/fixtures`. It reads only text, so the
//! integration test, which cannot reach this crate's items, includes it by
//! path.

// Each of the two test crates uses only some of this.
#![allow(dead_code)]

/// The name the commands invoke the binary by. An integration test is not
/// compiled with `CARGO_BIN_NAME`, so it is spelled out here, and the tests in
/// `docs` check it against that.
pub const BIN: &str = "graphql-document-utils";

/// `line` without its indent of exactly `width` spaces.
fn indented(line: &str, width: usize) -> &str {
    line.strip_prefix(&" ".repeat(width))
        .filter(|rest| !rest.starts_with(' '))
        .unwrap_or_else(|| panic!("indented {width} spaces: {line:?}"))
}

/// The commands in an examples block, with continued lines joined: the
/// `Examples:` line, then commands indented two spaces, each continuing onto
/// lines indented four while it ends in `\`.
pub fn examples(block: &str) -> Vec<String> {
    let mut lines = block
        .strip_prefix("Examples:\n")
        .unwrap_or_else(|| panic!("examples start with `Examples:`:\n{block}"))
        .lines();
    let mut commands = Vec::new();
    while let Some(line) = lines.next() {
        let mut command = indented(line, 2).to_string();
        while let Some(head) = command.strip_suffix(" \\") {
            let next = lines.next().expect("a `\\` is followed by a line");
            command = format!("{head} {}", indented(next, 4));
        }
        commands.push(command);
    }
    commands
}

/// The common tasks in `block`, each intent with its command, and the text
/// after the table. The table is a `Common tasks:` line, then each intent
/// indented two spaces with its command on the next line indented four, then a
/// blank line.
pub fn tasks(block: &str) -> (Vec<(String, String)>, String) {
    let (table, footer) = block
        .strip_prefix("Common tasks:\n")
        .and_then(|rest| rest.split_once("\n\n"))
        .unwrap_or_else(|| panic!("a `Common tasks:` table and a footer:\n{block}"));
    let mut lines = table.lines();
    let mut tasks = Vec::new();
    while let Some(intent) = lines.next() {
        let command = lines.next().expect("an intent is followed by its command");
        tasks.push((
            indented(intent, 2).to_string(),
            indented(command, 4).to_string(),
        ));
    }
    (tasks, footer.to_string())
}

/// One stage of a pipeline the help or the README shows.
#[derive(Debug, PartialEq)]
pub enum Stage<'a> {
    /// `cat` and the files it reads, which may hold a `*`.
    Cat(Vec<&'a str>),
    /// The binary, with the arguments after its name. A `>` ends them, as the
    /// shell redirects what follows.
    Bin(Vec<&'a str>),
}

/// The stages of `command`, a pipeline of `cat` and the binary, split on `|`.
/// Nothing is quoted, so the words are split on whitespace. Panics on any other
/// program, as nothing but these two can be checked or run.
pub fn stages(command: &str) -> Vec<Stage<'_>> {
    command
        .split('|')
        .map(|stage| {
            let mut words = stage.split_whitespace();
            match words.next() {
                Some("cat") => Stage::Cat(words.collect()),
                Some(BIN) => Stage::Bin(words.take_while(|word| !word.starts_with('>')).collect()),
                _ => panic!("a stage runs `cat` or `{BIN}`: {command}"),
            }
        })
        .collect()
}

/// The arguments after the binary name in each invocation of it in `command`.
pub fn invocations(command: &str) -> Vec<Vec<&str>> {
    stages(command)
        .into_iter()
        .filter_map(|stage| match stage {
            Stage::Bin(args) => Some(args),
            Stage::Cat(_) => None,
        })
        .collect()
}

/// A fenced code block in Markdown.
#[derive(Debug)]
pub struct Fence {
    /// What follows the opening ```` ``` ````, such as `bash`.
    pub info: String,
    /// The lines between the fences, each ending in a newline.
    pub body: String,
    /// The text between the previous fence, or the start, and this one.
    pub before: String,
}

/// Every fenced code block in `markdown`, in order.
pub fn fences(markdown: &str) -> Vec<Fence> {
    let mut lines = markdown.lines();
    let mut fences = Vec::new();
    let mut before = String::new();
    while let Some(line) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            before.push_str(line);
            before.push('\n');
            continue;
        };
        let body = lines
            .by_ref()
            .take_while(|line| *line != "```")
            .map(|line| format!("{line}\n"))
            .collect();
        fences.push(Fence {
            info: info.to_string(),
            body,
            before: std::mem::take(&mut before),
        });
    }
    fences
}

/// The commands in a `bash` block that invoke the binary, in any stage of a
/// pipeline, with a line ending in `\` joined to the next as in an examples
/// block. Comments and other tools' commands are skipped.
pub fn shell_commands(block: &str) -> Vec<String> {
    let mut lines = block.lines();
    let mut commands = Vec::new();
    while let Some(line) = lines.next() {
        let mut command = line.trim().to_string();
        while let Some(head) = command.strip_suffix('\\') {
            let next = lines.next().expect("a `\\` is followed by a line");
            command = format!("{} {}", head.trim_end(), next.trim());
        }
        let invokes_bin = command
            .split('|')
            .any(|stage| stage.split_whitespace().next() == Some(BIN));
        if invokes_bin {
            commands.push(command);
        }
    }
    commands
}

/// The commands in every `bash` block of `markdown` that invoke the binary.
pub fn readme_commands(markdown: &str) -> Vec<String> {
    fences(markdown)
        .iter()
        .filter(|fence| fence.info == "bash")
        .flat_map(|fence| shell_commands(&fence.body))
        .collect()
}
