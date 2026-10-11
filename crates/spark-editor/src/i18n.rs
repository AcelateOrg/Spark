//! Translations. Every UI string goes through [`t`] / [`tf`] with a key from `locales/<lang>.toml`.
//!
//! - Adding a language: add a variant to [`Lang`], a `locales/<code>.toml` with the same keys, and an entry in
//!   [`SOURCES`]. The tests fail when a key is missing in any locale or used in code but not defined.
//! - Missing keys fall back to English, then to the key itself (never a panic).
//! - Placeholders: `"Opened {name}"` + `tf("key", &[("name", &value)])`.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    #[serde(rename = "en")]
    En,
    #[serde(rename = "ru")]
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    /// The language's own name (shown in the language picker).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }

    /// The OS UI language when we have it, else English.
    pub fn system() -> Lang {
        let locale = sys_locale::get_locale().unwrap_or_default().to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| locale.starts_with(l.code())).unwrap_or_default()
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// `(language, embedded locale file)`, in [`Lang`] order.
const SOURCES: [(Lang, &str); 2] = [(Lang::En, include_str!("../locales/en.toml")), (Lang::Ru, include_str!("../locales/ru.toml"))];

static CURRENT: AtomicU8 = AtomicU8::new(0);
static TABLES: OnceLock<Vec<HashMap<String, &'static str>>> = OnceLock::new();

fn tables() -> &'static [HashMap<String, &'static str>] {
    TABLES.get_or_init(|| {
        SOURCES
            .iter()
            .map(|(lang, src)| match parse(src) {
                // Strings live for the whole run: leak them once so `t()` can hand out `&'static str`.
                Ok(map) => map.into_iter().map(|(k, v)| (k, &*Box::leak(v.into_boxed_str()))).collect(),
                Err(e) => {
                    log::error!("locales/{}.toml: {e}", lang.code());
                    HashMap::new()
                }
            })
            .collect()
    })
}

/// Flattens `[section] key = "text"` into `section.key -> text`.
fn parse(src: &str) -> Result<HashMap<String, String>, String> {
    let value: toml::Table = toml::from_str(src).map_err(|e| e.to_string())?;
    let mut out = HashMap::new();
    fn walk(prefix: &str, table: &toml::Table, out: &mut HashMap<String, String>) {
        for (k, v) in table {
            let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            match v {
                toml::Value::String(s) => {
                    out.insert(key, s.clone());
                }
                toml::Value::Table(t) => walk(&key, t, out),
                other => {
                    out.insert(key, other.to_string());
                }
            }
        }
    }
    walk("", &value, &mut out);
    Ok(out)
}

pub fn set_lang(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Ru,
        _ => Lang::En,
    }
}

/// The translation of `key` in the current language.
pub fn t(key: &'static str) -> &'static str {
    let tables = tables();
    tables
        .get(lang().index())
        .and_then(|m| m.get(key))
        .or_else(|| tables.first().and_then(|m| m.get(key)))
        .copied()
        .unwrap_or(key)
}

/// [`t`] with `{name}` placeholders replaced.
pub fn tf(key: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = t(key).to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys(lang: Lang) -> BTreeSet<String> {
        parse(SOURCES[lang.index()].1).unwrap().into_keys().collect()
    }

    #[test]
    fn every_locale_has_every_key() {
        let en = keys(Lang::En);
        assert!(!en.is_empty());
        for lang in Lang::ALL {
            let other = keys(lang);
            let missing: Vec<_> = en.difference(&other).collect();
            let extra: Vec<_> = other.difference(&en).collect();
            assert!(missing.is_empty() && extra.is_empty(), "locales/{}.toml: missing {missing:?}, unknown {extra:?}", lang.code());
        }
    }

    /// Every `t("key")` / `tf("key"` in the source exists in the English locale.
    #[test]
    fn every_used_key_is_defined() {
        let en = keys(Lang::En);
        let mut missing = Vec::new();
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src_dir];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                // This file's own tests use made-up keys on purpose.
                if p.extension().is_none_or(|e| e != "rs") || p.file_name().is_some_and(|n| n == "i18n.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap();
                for pat in ["t(\"", "tf(\""] {
                    let mut rest = text.as_str();
                    while let Some(i) = rest.find(pat) {
                        let before = rest[..i].chars().last();
                        rest = &rest[i + pat.len()..];
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue; // e.g. `split("` or `fmt("`
                        }
                        let key = &rest[..rest.find('"').unwrap_or(0)];
                        if key.contains('.') && !key.contains(' ') && !en.contains(key) {
                            missing.push(format!("{}: {key}", p.file_name().unwrap().to_string_lossy()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "keys used but not in locales/en.toml: {missing:#?}");
    }

    #[test]
    fn lookup_and_format() {
        set_lang(Lang::Ru);
        assert_ne!(t("app.name"), "app.name");
        assert_eq!(t("no.such.key"), "no.such.key");
        set_lang(Lang::En);
        assert!(tf("welcome.opened", &[("name", &"X")]).contains('X'));
    }
}
