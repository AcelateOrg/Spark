//! The error screen shown while a script is paused: where (file:line + the code around it), what
//! (the message), how we got there (the game's own stack frames) and what to do next.
//! Drawn by the engine in Rust, so it works even when the game's Lua state is broken.

use std::path::Path;

use spark_core::{Align, Color, Layer, TextParams, Vec2, World};

/// A Luau error split into parts for display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedError {
    /// `src/player.luau`
    pub file: Option<String>,
    pub line: Option<u32>,
    /// The message without location / "runtime error:" noise (may span several lines, e.g. hints).
    pub message: String,
    /// Where the engine called the game: `start()`, `update`, `task`, `event 'hit'`...
    pub context: Vec<String>,
    /// The game's own frames, innermost first: `("src/player.luau:12", "move")`.
    pub stack: Vec<(String, String)>,
}

/// Finds the first `name.luau:LINE:` in `s`: (start, end of `:LINE:`, file, line).
fn find_location(s: &str) -> Option<(usize, usize, String, u32)> {
    let mut search = 0;
    while let Some(pos) = s[search..].find(".luau") {
        let at = search + pos;
        let after_name = at + 5;
        let rest = &s[after_name..];
        let skip = rest.len() - rest.trim_start_matches(['"', '\'', ']']).len();
        let rest = &rest[skip..];
        if let Some(r) = rest.strip_prefix(':') {
            let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                let start = s[..at]
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '[' | '@' | '<'))
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                let mut end = after_name + skip + 1 + digits.len();
                if s[end..].starts_with(':') {
                    end += 1;
                }
                return Some((start, end, s[start..after_name].replace('\\', "/"), digits.parse().ok()?));
            }
        }
        search = after_name;
    }
    None
}

fn clean(line: &str) -> &str {
    let mut l = line.trim();
    for p in ["runtime error:", "syntax error:", "error:"] {
        if let Some(r) = l.strip_prefix(p) {
            l = r.trim_start();
        }
    }
    l
}

impl ParsedError {
    pub fn parse(text: &str) -> Self {
        let mut out = ParsedError::default();
        let (body, trace) = match text.split_once("stack traceback:") {
            Some((b, t)) => (b, Some(t)),
            None => (text, None),
        };
        let mut message: Vec<String> = Vec::new();
        for raw in body.lines() {
            let line = clean(raw);
            if line.is_empty() {
                continue;
            }
            // mlua contexts: "in start()", "in task", "in handler of event 'x'" (outermost first).
            if message.is_empty() && out.file.is_none() && line.starts_with("in ") && line.len() < 80 && find_location(line).is_none() {
                out.context.push(line.trim_start_matches("in ").trim_end_matches(':').to_string());
                continue;
            }
            if out.file.is_none() {
                if let Some((start, end, file, n)) = find_location(line) {
                    out.file = Some(file);
                    out.line = Some(n);
                    let before = clean(line[..start].trim().trim_end_matches(['[', '"', '\'', '(', '@', '<']));
                    let after = line[end..].trim();
                    let joined = if before.is_empty() || before.starts_with("[string") { after.to_string() } else { format!("{before} {after}") };
                    message.push(joined.trim().to_string());
                    continue;
                }
            }
            message.push(line.to_string());
        }
        out.message = message.join("\n").trim().to_string();
        if out.message.is_empty() {
            out.message = text.trim().to_string();
        }
        if let Some(trace) = trace {
            for l in trace.lines().map(str::trim) {
                if l.is_empty() || l.starts_with("[C]") || l.starts_with("[string") || l.starts_with("test:") {
                    continue;
                }
                let Some((_, end, file, n)) = find_location(l) else { continue };
                let func = l[end..].trim().trim_start_matches("in ").trim();
                let func = func.strip_prefix("function ").unwrap_or(func).trim_matches(['\'', '<', '>']);
                let func = if func.starts_with(file.as_str()) || func.is_empty() || func == "?" { "(main chunk)".to_string() } else { func.to_string() };
                let at = format!("{file}:{n}");
                if out.stack.last().is_none_or(|(a, _)| *a != at) {
                    out.stack.push((at, func));
                }
            }
        }
        // The first frame repeats the location shown on top.
        if let (Some(f), Some(n)) = (&out.file, out.line) {
            if out.stack.first().is_some_and(|(a, _)| *a == format!("{f}:{n}")) && out.stack.len() == 1 {
                out.stack.clear();
            }
        }
        out
    }
}

/// Everything the error screen shows (built once when the error happens).
#[derive(Debug, Clone)]
pub struct ErrorView {
    pub heading: String,
    pub parsed: ParsedError,
    /// `(line number, text)` around the error line.
    pub code: Vec<(u32, String)>,
    pub hint: String,
}

impl ErrorView {
    /// `root` = the game folder (to read the source around the error line).
    pub fn new(heading: &str, msg: &str, root: &Path, hint: &str) -> Self {
        let parsed = ParsedError::parse(msg);
        let code = match (&parsed.file, parsed.line) {
            (Some(f), Some(n)) => source_lines(root, f, n),
            _ => Vec::new(),
        };
        Self { heading: heading.into(), parsed, code, hint: hint.into() }
    }
}

fn source_lines(root: &Path, file: &str, line: u32) -> Vec<(u32, String)> {
    let p = Path::new(file);
    let path = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let Ok(src) = spark_core::vfs::read_string(&path) else { return Vec::new() };
    let first = line.saturating_sub(3).max(1);
    src.lines()
        .enumerate()
        .map(|(i, l)| (i as u32 + 1, l))
        .filter(|(n, _)| *n >= first && *n <= line + 2)
        .map(|(n, l)| (n, l.replace('\t', "    ").trim_end().chars().take(160).collect()))
        .collect()
}

const BG: Color = Color::rgba(0.055, 0.05, 0.06, 0.95);
const PANEL: Color = Color::rgba(1.0, 1.0, 1.0, 0.045);
const RED: Color = Color::rgb(1.0, 0.36, 0.33);
const RED_BAND: Color = Color::rgba(1.0, 0.3, 0.28, 0.16);
const TEXT: Color = Color::rgb(0.93, 0.92, 0.9);
const DIM: Color = Color::rgb(0.55, 0.53, 0.52);
const ACCENT: Color = Color::rgb(0.98, 0.78, 0.45);

/// Draws the screen over the (frozen) game.
pub fn draw(world: &mut World, v: &ErrorView) {
    let c = &mut world.canvas;
    let prev = c.layer();
    c.set_layer(Layer::Ui);
    let size = c.size();
    let s = (size.y / 720.0).clamp(0.6, 2.0);
    let pad = 44.0 * s;
    let width = (size.x - pad * 2.0).max(100.0);
    c.rect(Vec2::ZERO, size, BG, Align::TOP_LEFT);
    c.rect(Vec2::ZERO, Vec2::new(6.0 * s, size.y), RED, Align::TOP_LEFT);

    let mut y = pad * 0.8;
    let t = |size: f32, color: Color| TextParams { size: size * s, color, ..Default::default() };
    // Heading + location.
    let h = c.text(&v.heading.to_uppercase(), Vec2::new(pad, y), &t(15.0, RED));
    let mut x = pad + h.x + 14.0 * s;
    if let Some(f) = &v.parsed.file {
        let loc = match v.parsed.line {
            Some(n) => format!("{f}:{n}"),
            None => f.clone(),
        };
        let w = c.text(&loc, Vec2::new(x, y), &t(15.0, ACCENT));
        x += w.x + 14.0 * s;
    }
    if !v.parsed.context.is_empty() {
        c.text(&format!("in {}", v.parsed.context.join(" > ")), Vec2::new(x, y), &t(15.0, DIM));
    }
    y += h.y.max(15.0 * s) + 16.0 * s;

    // Message (first line big, the rest - hints, details - smaller).
    let mut msg_lines = v.parsed.message.lines();
    let first: String = msg_lines.next().unwrap_or("").chars().take(600).collect();
    let big = TextParams { width: Some(width), line_height: 1.2, ..t(25.0, TEXT) };
    y += c.text(&first, Vec2::new(pad, y), &big).y + 8.0 * s;
    let rest: Vec<&str> = msg_lines.take(10).collect();
    if !rest.is_empty() {
        let small = TextParams { width: Some(width), line_height: 1.25, ..t(16.0, Color::rgb(0.78, 0.76, 0.74)) };
        y += c.text(&rest.join("\n"), Vec2::new(pad, y), &small).y + 8.0 * s;
    }
    y += 14.0 * s;

    // Code around the error line.
    if !v.code.is_empty() {
        let lh = 22.0 * s;
        let ph = lh * v.code.len() as f32 + 16.0 * s;
        c.rect(Vec2::new(pad, y), Vec2::new(width, ph), PANEL, Align::TOP_LEFT);
        let mut ly = y + 8.0 * s;
        let num_w = 48.0 * s;
        for (n, text) in &v.code {
            let here = Some(*n) == v.parsed.line;
            if here {
                c.rect(Vec2::new(pad, ly - 2.0 * s), Vec2::new(width, lh), RED_BAND, Align::TOP_LEFT);
                c.rect(Vec2::new(pad, ly - 2.0 * s), Vec2::new(3.0 * s, lh), RED, Align::TOP_LEFT);
            }
            let num = TextParams { align: Align { x: 1.0, y: 0.0 }, ..t(15.0, if here { RED } else { DIM }) };
            c.text(&n.to_string(), Vec2::new(pad + num_w - 10.0 * s, ly), &num);
            c.text(text, Vec2::new(pad + num_w + 8.0 * s, ly), &t(16.0, if here { TEXT } else { DIM }));
            ly += lh;
        }
        y += ph + 22.0 * s;
    }

    // The game's call stack.
    if !v.parsed.stack.is_empty() {
        c.text("CALL STACK", Vec2::new(pad, y), &t(13.0, DIM));
        y += 22.0 * s;
        for (at, func) in v.parsed.stack.iter().take(6) {
            let w = c.text(func, Vec2::new(pad + 12.0 * s, y), &t(16.0, TEXT));
            c.text(at, Vec2::new(pad + 24.0 * s + w.x, y), &t(15.0, DIM));
            y += 22.0 * s;
        }
        if v.parsed.stack.len() > 6 {
            c.text(&format!("... {} more", v.parsed.stack.len() - 6), Vec2::new(pad + 12.0 * s, y), &t(14.0, DIM));
        }
    }

    // What to do.
    let hint = TextParams { align: Align { x: 0.0, y: 1.0 }, width: Some(width), line_height: 1.25, ..t(15.0, DIM) };
    c.text(&v.hint, Vec2::new(pad, size.y - pad * 0.7), &hint);
    c.set_layer(prev);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_runtime_error_with_context_and_stack() {
        let e = ParsedError::parse(
            "in start()\nin task\nruntime error: src/player.luau:12: attempt to index nil with 'x'\nstack traceback:\n\t[C]: in ?\n\tsrc/player.luau:12: in function 'move'\n\tmain.luau:5: in function <main.luau:3>\n",
        );
        assert_eq!(e.file.as_deref(), Some("src/player.luau"));
        assert_eq!(e.line, Some(12));
        assert_eq!(e.message, "attempt to index nil with 'x'");
        assert_eq!(e.context, vec!["start()".to_string(), "task".to_string()]);
        assert_eq!(e.stack, vec![("src/player.luau:12".into(), "move".into()), ("main.luau:5".into(), "(main chunk)".into())]);
    }

    #[test]
    fn parses_string_chunk_and_plain_messages() {
        let e = ParsedError::parse("[string \"main.luau\"]:3: boom\nhint: do this");
        assert_eq!((e.file.as_deref(), e.line), (Some("main.luau"), Some(3)));
        assert_eq!(e.message, "boom\nhint: do this");
        let e = ParsedError::parse("cannot read main.luau");
        assert_eq!(e.file, None);
        assert_eq!(e.message, "cannot read main.luau");
    }

    #[test]
    fn reads_code_around_the_line() {
        let dir = std::env::temp_dir().join(format!("spark-errscreen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.luau"), "a\nb\nc\nd\ne\nf\ng\n").unwrap();
        let v = ErrorView::new("Script error", "main.luau:4: bad", &dir, "");
        assert_eq!(v.code.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(v.code[3].1, "d");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
