//! Small, fast syntax highlighters (no regex, no grammar files): Luau, WGSL, TOML, Markdown.
//! Output is a list of token spans; the code editor turns them into colors.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Luau,
    Wgsl,
    Toml,
    Markdown,
    Plain,
}

impl Lang {
    pub fn from_path(path: &std::path::Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("luau" | "lua") => Self::Luau,
            Some("wgsl") => Self::Wgsl,
            Some("toml") => Self::Toml,
            Some("md") => Self::Markdown,
            _ => Self::Plain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Luau => "Luau",
            Self::Wgsl => "WGSL",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Plain => "Text",
        }
    }

    /// Line comment prefix (for the upcoming Ctrl+/ toggle).
    #[allow(dead_code)]
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Self::Luau => Some("--"),
            Self::Wgsl => Some("//"),
            Self::Toml => Some("#"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Text,
    Keyword,
    /// Spark API globals (`spawn`, `draw`, `input` ...) / WGSL builtins.
    Builtin,
    Function,
    Number,
    String,
    Comment,
    Punct,
    Heading,
}

pub type Span = (Range<usize>, Token);

const LUAU_KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "export", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "type", "typeof", "until", "while", "self",
];

/// Spark globals (docs/spark.d.luau) + the common Luau standard library.
const LUAU_BUILTINS: &[&str] = &[
    "spawn", "vec2", "vec3", "vec4", "input", "time", "camera", "graphics", "physics", "sound", "draw", "screen", "save",
    "window", "Mesh", "Texture", "Material", "Color", "Model", "Shader", "Font", "ui", "debug", "events", "tween", "task",
    "background", "sun", "ambient", "fog", "sky", "light", "length", "normalize", "dot", "cross", "lerp", "clamp", "quit",
    "restart", "dump_scene", "require", "print", "warn", "error", "assert", "pcall", "xpcall", "ipairs", "pairs", "next",
    "select", "setmetatable", "getmetatable", "tostring", "tonumber", "rawget", "rawset", "math", "string", "table",
    "coroutine", "buffer", "bit32", "utf8", "os", "vector",
];

const WGSL_KEYWORDS: &[&str] = &[
    "fn", "let", "var", "const", "override", "struct", "return", "if", "else", "for", "while", "loop", "break", "continue",
    "switch", "case", "default", "discard", "true", "false", "alias", "enable", "requires",
];

const WGSL_BUILTINS: &[&str] = &[
    "f32", "f16", "i32", "u32", "bool", "vec2", "vec3", "vec4", "vec2f", "vec3f", "vec4f", "vec2i", "vec3i", "vec4i",
    "vec2u", "vec3u", "vec4u", "mat2x2", "mat3x3", "mat4x4", "mat4x4f", "mat3x3f", "array", "texture_2d", "sampler",
    "textureSample", "mix", "clamp", "saturate", "smoothstep", "step", "dot", "cross", "normalize", "length", "sin",
    "cos", "tan", "pow", "exp", "log", "sqrt", "abs", "min", "max", "fract", "floor", "ceil", "select", "reflect",
];

/// Highlights one whole document.
pub fn highlight(lang: Lang, text: &str) -> Vec<Span> {
    match lang {
        Lang::Luau => code(text, LUAU_KEYWORDS, LUAU_BUILTINS, Comments::Luau),
        Lang::Wgsl => code(text, WGSL_KEYWORDS, WGSL_BUILTINS, Comments::CLike),
        Lang::Toml => toml(text),
        Lang::Markdown => markdown(text),
        Lang::Plain => vec![(0..text.len(), Token::Text)],
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Comments {
    Luau,
    CLike,
}

fn push(out: &mut Vec<Span>, range: Range<usize>, token: Token) {
    if range.is_empty() {
        return;
    }
    if let Some((last, t)) = out.last_mut() {
        if *t == token && last.end == range.start {
            last.end = range.end;
            return;
        }
    }
    out.push((range, token));
}

/// Luau long brackets: `[[`, `[==[` ... returns the level when `s` starts one.
fn long_bracket(s: &[u8]) -> Option<usize> {
    if s.first() != Some(&b'[') {
        return None;
    }
    let eq = s[1..].iter().take_while(|&&c| c == b'=').count();
    (s.get(1 + eq) == Some(&b'[')).then_some(eq)
}

fn find_long_close(s: &[u8], from: usize, level: usize) -> usize {
    let mut i = from;
    while i < s.len() {
        if s[i] == b']' {
            let eq = s[i + 1..].iter().take_while(|&&c| c == b'=').count();
            if eq == level && s.get(i + 1 + eq) == Some(&b']') {
                return i + 2 + eq;
            }
        }
        i += 1;
    }
    s.len()
}

fn code(text: &str, keywords: &[&str], builtins: &[&str], comments: Comments) -> Vec<Span> {
    let s = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        let start = i;
        // Comments
        let line_comment = match comments {
            Comments::Luau => c == b'-' && s.get(i + 1) == Some(&b'-'),
            Comments::CLike => c == b'/' && s.get(i + 1) == Some(&b'/'),
        };
        if line_comment {
            if comments == Comments::Luau {
                if let Some(level) = long_bracket(&s[i + 2..]) {
                    i = find_long_close(s, i + 4 + level, level);
                    push(&mut out, start..i, Token::Comment);
                    continue;
                }
            }
            while i < s.len() && s[i] != b'\n' {
                i += 1;
            }
            push(&mut out, start..i, Token::Comment);
            continue;
        }
        if comments == Comments::CLike && c == b'/' && s.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < s.len() && !(s[i] == b'*' && s.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(s.len());
            push(&mut out, start..i, Token::Comment);
            continue;
        }
        // Strings
        if c == b'"' || c == b'\'' || (c == b'`' && comments == Comments::Luau) {
            i += 1;
            while i < s.len() && s[i] != c && s[i] != b'\n' {
                if s[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(s.len());
            push(&mut out, start..i, Token::String);
            continue;
        }
        if comments == Comments::Luau {
            if let Some(level) = long_bracket(&s[i..]) {
                i = find_long_close(s, i + 2 + level, level);
                push(&mut out, start..i, Token::String);
                continue;
            }
        }
        // Numbers
        if c.is_ascii_digit() || (c == b'.' && s.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            i += 1;
            while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'.' || s[i] == b'_') {
                i += 1;
            }
            push(&mut out, start..i, Token::Number);
            continue;
        }
        // Words
        if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'_' || s[i] >= 0x80) {
                i += 1;
            }
            // Never split a UTF-8 sequence.
            while !text.is_char_boundary(i) {
                i += 1;
            }
            let word = &text[start..i];
            let after = s[i..].iter().find(|b| !b.is_ascii_whitespace()).copied();
            let prev_dot = start > 0 && (s[start - 1] == b'.' || s[start - 1] == b':');
            let token = if keywords.contains(&word) {
                Token::Keyword
            } else if !prev_dot && builtins.contains(&word) {
                Token::Builtin
            } else if after == Some(b'(') {
                Token::Function
            } else {
                Token::Text
            };
            push(&mut out, start..i, token);
            continue;
        }
        if c.is_ascii_punctuation() {
            i += 1;
            push(&mut out, start..i, Token::Punct);
            continue;
        }
        // Whitespace and anything else.
        i += 1;
        while !text.is_char_boundary(i) {
            i += 1;
        }
        push(&mut out, start..i, Token::Text);
    }
    out
}

fn toml(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        push(&mut out, offset..offset + indent, Token::Text);
        let base = offset + indent;
        if trimmed.starts_with('#') {
            push(&mut out, base..offset + line.len(), Token::Comment);
        } else if trimmed.starts_with('[') {
            push(&mut out, base..offset + line.len(), Token::Heading);
        } else if let Some(eq) = trimmed.find('=') {
            push(&mut out, base..base + eq, Token::Keyword);
            push(&mut out, base + eq..base + eq + 1, Token::Punct);
            for (r, t) in code(&trimmed[eq + 1..], &["true", "false"], &[], Comments::Luau) {
                let t = if t == Token::Comment { Token::Text } else { t };
                push(&mut out, base + eq + 1 + r.start..base + eq + 1 + r.end, t);
            }
            // `#` comments after the value.
            fix_toml_comment(&mut out, text, base + eq + 1, offset + line.len());
        } else {
            push(&mut out, base..offset + line.len(), Token::Text);
        }
        offset += line.len();
    }
    out
}

/// Marks `# ...` outside strings in `from..to` as a comment.
fn fix_toml_comment(out: &mut Vec<Span>, text: &str, from: usize, to: usize) {
    let mut in_str = None;
    for (i, c) in text[from..to].char_indices() {
        match c {
            '"' | '\'' if in_str.is_none() => in_str = Some(c),
            c2 if Some(c2) == in_str => in_str = None,
            '#' if in_str.is_none() => {
                let at = from + i;
                out.retain_mut(|(r, _)| {
                    if r.start >= at {
                        return false;
                    }
                    r.end = r.end.min(at);
                    true
                });
                let end = text[at..to].trim_end_matches(['\n', '\r']).len() + at;
                push(out, at..end, Token::Comment);
                push(out, end..to, Token::Text);
                return;
            }
            _ => {}
        }
    }
}

fn markdown(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut fenced = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        let token = if t.starts_with("```") {
            fenced = !fenced;
            Token::Comment
        } else if fenced {
            Token::String
        } else if t.starts_with('#') {
            Token::Heading
        } else if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("> ") {
            Token::Keyword
        } else {
            Token::Text
        };
        push(&mut out, offset..offset + line.len(), token);
        offset += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: Lang, text: &str) -> Vec<(&str, Token)> {
        highlight(lang, text).into_iter().map(|(r, t)| (&text[r], t)).filter(|(s, _)| !s.trim().is_empty()).collect()
    }

    #[test]
    fn luau() {
        let k = kinds(Lang::Luau, "local x = spawn(Mesh.cube(), \"red\") -- hi\n");
        assert!(k.contains(&("local", Token::Keyword)));
        assert!(k.contains(&("spawn", Token::Builtin)));
        assert!(k.contains(&("cube", Token::Function)));
        assert!(k.contains(&("\"red\"", Token::String)));
        assert!(k.contains(&("-- hi", Token::Comment)));
        let k = kinds(Lang::Luau, "--[[ a\nb ]] x = [==[s]==] 0x1F");
        assert_eq!(k[0], ("--[[ a\nb ]]", Token::Comment));
        assert!(k.contains(&("[==[s]==]", Token::String)));
        assert!(k.contains(&("0x1F", Token::Number)));
    }

    #[test]
    fn spans_cover_everything_and_keep_utf8() {
        let text = "local имя = \"привет\" -- комментарий\né";
        for lang in [Lang::Luau, Lang::Wgsl, Lang::Toml, Lang::Markdown, Lang::Plain] {
            let spans = highlight(lang, text);
            let mut at = 0;
            for (r, _) in &spans {
                assert_eq!(r.start, at, "{lang:?}");
                assert!(text.is_char_boundary(r.end));
                at = r.end;
            }
            assert_eq!(at, text.len(), "{lang:?}");
        }
    }

    #[test]
    fn toml_lines() {
        let k = kinds(Lang::Toml, "[a]\ntitle = \"x # y\" # c\n");
        assert!(k.contains(&("title ", Token::Keyword)));
        assert!(k.contains(&("\"x # y\"", Token::String)));
        assert!(k.contains(&("# c", Token::Comment)));
    }
}
