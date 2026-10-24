//! Child processes of the editor: the running game (`spark <game> --devtools ...`), `spark check`, `spark build`.
//!
//! Output is read on background threads and arrives as [`LogLine`]s; the UI drains them every frame.
//! The editor never runs the engine in-process: a crashing game cannot take the editor down, and the game
//! always runs exactly like it will for players.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

/// What a job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// The game in a window, linked to the editor.
    Run,
    /// `spark check --json`: script errors without a window.
    Check,
    /// `spark build`: the player executable.
    Build,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
    /// Written by the editor itself ("game started", "exit code 1" ...).
    Editor,
}

/// One line of output.
#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: Level,
    pub stream: Stream,
    pub job: JobKind,
    pub text: String,
    /// Seconds since the editor started.
    pub time: f64,
    /// `file.luau:LINE` found in the text (click to open).
    pub location: Option<Location>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// As written in the message, usually relative to the game folder.
    pub file: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Running,
    Exited(Option<i32>),
    Killed,
}

/// A running (or finished) child process.
pub struct Job {
    pub kind: JobKind,
    pub state: JobState,
    child: Child,
    output: Receiver<Captured>,
    /// Whole stdout, for jobs that print a machine-readable result (`spark check --json`).
    stdout: String,
    keep_stdout: bool,
}

enum Captured {
    Line(Stream, String),
}

impl Job {
    /// Starts `exe args...` in `cwd`.
    pub fn spawn(kind: JobKind, exe: &Path, args: &[String], cwd: &Path) -> Result<Self, String> {
        let mut cmd = Command::new(exe);
        cmd.args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Colors / timestamps off: the console parses the plain env_logger format.
        cmd.env("RUST_LOG_STYLE", "never");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
        let (tx, rx) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            pipe(out, Stream::Stdout, tx.clone());
        }
        if let Some(err) = child.stderr.take() {
            pipe(err, Stream::Stderr, tx);
        }
        Ok(Self {
            kind,
            state: JobState::Running,
            child,
            output: rx,
            stdout: String::new(),
            keep_stdout: kind == JobKind::Check,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn is_running(&self) -> bool {
        self.state == JobState::Running
    }

    /// New output lines + whether the process has just exited. Call every frame.
    pub fn poll(&mut self, clock: &Instant) -> (Vec<LogLine>, Option<JobState>) {
        let mut lines = Vec::new();
        for Captured::Line(stream, text) in self.output.try_iter() {
            if self.keep_stdout && stream == Stream::Stdout {
                self.stdout.push_str(&text);
                self.stdout.push('\n');
                // Machine output (JSON) is not shown line by line.
                if text.trim_start().starts_with('{') {
                    continue;
                }
            }
            lines.push(parse_line(self.kind, stream, &text, clock.elapsed().as_secs_f64()));
        }
        let mut finished = None;
        if self.state == JobState::Running {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.state = JobState::Exited(exit_code(status));
                finished = Some(self.state.clone());
            }
        }
        (lines, finished)
    }

    /// Everything the job printed to stdout so far (only kept for [`JobKind::Check`]).
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Kills the process (the game gets [`spark_devtools::EditorCommand::Quit`] first, see the session).
    pub fn kill(&mut self) {
        if self.state == JobState::Running {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.state = JobState::Killed;
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.kill();
    }
}

fn exit_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

fn pipe<R: Read + Send + 'static>(reader: R, stream: Stream, tx: Sender<Captured>) {
    let _ = std::thread::Builder::new().name("spark-editor-pipe".into()).spawn(move || {
        let mut r = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
                    if tx.send(Captured::Line(stream, text)).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// A line the editor writes itself.
pub fn editor_line(job: JobKind, level: Level, text: impl Into<String>, clock: &Instant) -> LogLine {
    let text = text.into();
    let location = find_location(&text);
    LogLine { level, stream: Stream::Editor, job, text, time: clock.elapsed().as_secs_f64(), location }
}

/// Understands env_logger lines (`[INFO ] text`, `[WARN  spark] text`) and Spark error lines.
pub fn parse_line(job: JobKind, stream: Stream, raw: &str, time: f64) -> LogLine {
    let mut level = if stream == Stream::Stderr { Level::Info } else { Level::Info };
    let mut text = raw;
    if let Some(rest) = raw.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let tag = rest[..end].split_whitespace().next().unwrap_or("");
            let parsed = match tag {
                "TRACE" => Some(Level::Trace),
                "DEBUG" => Some(Level::Debug),
                "INFO" => Some(Level::Info),
                "WARN" => Some(Level::Warn),
                "ERROR" => Some(Level::Error),
                _ => None,
            };
            if let Some(l) = parsed {
                level = l;
                text = rest[end + 1..].trim_start();
            }
        }
    }
    let lower = text.to_ascii_lowercase();
    if level < Level::Warn && (lower.starts_with("error") || lower.contains(": error:") || lower.starts_with("spark: ") && lower.contains("error")) {
        level = Level::Error;
    } else if level < Level::Warn && lower.contains("warning:") {
        level = Level::Warn;
    }
    LogLine { level, stream, job, text: text.to_string(), time, location: find_location(text) }
}

/// First `something.luau:123` (or `.wgsl:`) in a message.
pub fn find_location(text: &str) -> Option<Location> {
    for ext in [".luau:", ".wgsl:", ".lua:"] {
        let mut from = 0;
        while let Some(i) = text[from..].find(ext).map(|i| i + from) {
            let after = &text[i + ext.len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(line) = digits.parse::<u32>() {
                // Walk back to the start of the file name.
                let start = text[..i]
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '[' | '<' | ','))
                    .map(|(j, c)| j + c.len_utf8())
                    .unwrap_or(0);
                let file = text[start..i + ext.len() - 1].trim_start_matches("./").to_string();
                if !file.is_empty() {
                    return Some(Location { file, line });
                }
            }
            from = i + ext.len();
        }
    }
    None
}

/// The `spark` executable that runs games: `$SPARK_EXE` (set by `spark editor`), the configured path,
/// `spark` next to the editor, then `PATH`.
pub fn find_spark(configured: Option<&Path>) -> Option<PathBuf> {
    let name = if cfg!(windows) { "spark.exe" } else { "spark" };
    let candidates = configured
        .map(Path::to_path_buf)
        .into_iter()
        .chain(std::env::var_os("SPARK_EXE").map(PathBuf::from))
        .chain(std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))));
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.is_file()))
}

/// `spark check --json` result.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct CheckReport {
    pub ok: bool,
    #[serde(default)]
    pub errors: Vec<CheckProblem>,
    #[serde(default)]
    pub frames: u64,
    #[serde(default)]
    pub objects: u64,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct CheckProblem {
    #[serde(default)]
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}

impl CheckReport {
    /// Finds the JSON object in the captured stdout.
    pub fn parse(stdout: &str) -> Option<Self> {
        stdout.lines().rev().find_map(|l| serde_json::from_str(l.trim()).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_lines() {
        let l = parse_line(JobKind::Run, Stream::Stderr, "[WARN ] devtools: oops", 0.0);
        assert_eq!((l.level, l.text.as_str()), (Level::Warn, "devtools: oops"));
        let l = parse_line(JobKind::Run, Stream::Stderr, "[ERROR spark] src/player.luau:12: attempt to index nil", 0.0);
        assert_eq!(l.level, Level::Error);
        assert_eq!(l.location, Some(Location { file: "src/player.luau".into(), line: 12 }));
        let l = parse_line(JobKind::Run, Stream::Stdout, "hello", 0.0);
        assert_eq!((l.level, l.location), (Level::Info, None));
    }

    #[test]
    fn finds_locations() {
        assert_eq!(find_location("error in \"main.luau:3: x\"").unwrap().file, "main.luau");
        assert_eq!(find_location("./assets/shaders/water.wgsl:40:7 bad").unwrap(), Location { file: "assets/shaders/water.wgsl".into(), line: 40 });
        assert_eq!(find_location("main.luau: no line"), None);
    }

    #[test]
    fn parses_check_report() {
        let out = "spark check: 1 error\n{\"ok\":false,\"errors\":[{\"file\":\"main.luau\",\"line\":3,\"message\":\"boom\"}],\"frames\":60,\"objects\":2}\n";
        let r = CheckReport::parse(out).unwrap();
        assert!(!r.ok);
        assert_eq!(r.errors[0].line, Some(3));
    }
}
