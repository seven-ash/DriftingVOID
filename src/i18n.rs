// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! UI languages (docs/updates/localisation.md). The Korean text is the key
//! (Ash, 10-29): `tr("새로 만들기")` shows the chosen language's text, or the
//! Korean when a table lacks it. A word that reads differently by context is
//! keyed with its context (`tr_ctx("혼합", "복사")`, key "혼합|복사"). Text
//! with values goes through `trf`, whose template marks each value `{}` in
//! order; a translation may reorder them as `{0}`, `{1}`, ….
//!
//! Tables: `lang/en.json`, `lang/ja.json` (Korean → text), built into the
//! exe. `tests::every_string_has_a_translation` keeps them complete.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// Marks a Korean string kept in a constant (tool names, presets, filter
/// values) as a key: it stays as it is, and is shown through `tr` where it
/// is drawn. The coverage test reads these too.
macro_rules! k {
    ($s:literal) => {
        $s
    };
}
pub(crate) use k;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ko,
    En,
    Ja,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Ko, Lang::En, Lang::Ja];

    /// The language's own name (the 설정 menu shows each in itself).
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ko => "한국어",
            Lang::En => "English",
            Lang::Ja => "日本語",
        }
    }

    /// The code kept in the preferences.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ko => "ko",
            Lang::En => "en",
            Lang::Ja => "ja",
        }
    }

    pub fn from_code(c: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == c)
    }
}

static CUR: AtomicU8 = AtomicU8::new(0);

pub fn lang() -> Lang {
    Lang::ALL[CUR.load(Ordering::Relaxed) as usize % 3]
}

pub fn set_lang(l: Lang) {
    CUR.store(Lang::ALL.iter().position(|x| *x == l).unwrap_or(0) as u8, Ordering::Relaxed);
}

/// The language a first start uses (Ash, 10-29): Windows' display language
/// when it is English or Japanese, else Korean.
pub fn system_lang() -> Lang {
    #[cfg(windows)]
    {
        // the primary language id is the low 10 bits of the LANGID
        let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff;
        match id {
            0x09 => Lang::En,
            0x11 => Lang::Ja,
            _ => Lang::Ko,
        }
    }
    #[cfg(not(windows))]
    {
        Lang::Ko
    }
}

fn table(l: Lang) -> Option<&'static HashMap<String, String>> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static JA: OnceLock<HashMap<String, String>> = OnceLock::new();
    let parse = |s: &str| serde_json::from_str::<HashMap<String, String>>(s).unwrap_or_default();
    match l {
        Lang::Ko => None,
        Lang::En => Some(EN.get_or_init(|| parse(include_str!("../lang/en.json")))),
        Lang::Ja => Some(JA.get_or_init(|| parse(include_str!("../lang/ja.json")))),
    }
}

/// `ko` in the chosen language.
pub fn tr(ko: &'static str) -> &'static str {
    match table(lang()).and_then(|t| t.get(ko)) {
        Some(s) => s.as_str(),
        None => ko,
    }
}

/// A word whose reading depends on where it stands: keyed "context|word";
/// the Korean shows the word alone.
pub fn tr_ctx(ctx: &'static str, ko: &'static str) -> &'static str {
    let key = format!("{ctx}|{ko}");
    match table(lang()).and_then(|t| t.get(&key)) {
        Some(s) => s.as_str(),
        None => ko,
    }
}

/// `ko` with its `{}` slots filled by `args` in order, in the chosen
/// language (whose text may use `{0}`, `{1}`, … to reorder them).
pub fn trf(ko: &'static str, args: &[String]) -> String {
    fill(tr(ko), args)
}

fn fill(t: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(t.len() + 16);
    let mut next = 0;
    let mut it = t.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c == '{' {
            if let Some(end) = t[i..].find('}') {
                let inner = &t[i + 1..i + end];
                let k = if inner.is_empty() {
                    next += 1;
                    Some(next - 1)
                } else {
                    inner.parse::<usize>().ok()
                };
                if let Some(k) = k {
                    out.push_str(args.get(k).map_or("", |s| s.as_str()));
                    while it.peek().is_some_and(|(j, _)| *j <= i + end) {
                        it.next();
                    }
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// The number of value slots in a template (`{}` or `{n}`).
#[cfg(test)]
fn slots(t: &str) -> usize {
    let mut n = 0;
    let mut i = 0;
    while let Some(a) = t[i..].find('{') {
        let s = i + a;
        match t[s..].find('}') {
            Some(b) if t[s + 1..s + b].is_empty() || t[s + 1..s + b].parse::<usize>().is_ok() => {
                n += 1;
                i = s + b + 1;
            }
            _ => i = s + 1,
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_order_and_reordered() {
        assert_eq!(fill("{}개 불러옴", &["3".into()]), "3개 불러옴");
        assert_eq!(fill("{1} of {0}", &["a".into(), "b".into()]), "b of a");
        assert_eq!(fill("no slots", &[]), "no slots");
    }

    /// Every string the code passes to `tr`, `tr_ctx` or `trf` has an entry in
    /// each table, with as many value slots; no table keeps a key the code
    /// no longer has.
    #[test]
    fn every_string_has_a_translation() {
        let keys = keys_in_source();
        assert!(keys.len() > 300, "found {} keys", keys.len());
        for l in [Lang::En, Lang::Ja] {
            let t = table(l).unwrap();
            let mut missing: Vec<&String> = keys.iter().filter(|k| !t.contains_key(*k)).collect();
            missing.sort();
            assert!(missing.is_empty(), "{l:?}: {} missing, e.g. {:?}", missing.len(), &missing[..missing.len().min(12)]);
            for (k, v) in t {
                assert!(keys.contains(k), "{l:?}: stale key {k:?}");
                let ko = k.split_once('|').map_or(k.as_str(), |x| x.1);
                assert_eq!(slots(ko), slots(v), "{l:?}: slots differ for {k:?} → {v:?}");
            }
        }
    }

    /// Keys as the source writes them: `tr("…")`, `trf("…"`, `tr_ctx("c", "…")`, `k!("…")`.
    fn keys_in_source() -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            // this file only shows the calls in its comments and tests
            if p.extension().is_none_or(|x| x != "rs") || p.file_name().is_some_and(|f| f == "i18n.rs") {
                continue;
            }
            // comments are not code
            let src: String = std::fs::read_to_string(&p).unwrap().lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("
");
            for (pat, ctx) in [("tr(\"", false), ("trf(\"", false), ("tr_ctx(\"", true), ("k!(\"", false)] {
                let mut rest = src.as_str();
                while let Some(i) = rest.find(pat) {
                    // only the calls, not words ending in "tr(" (e.g. `ptr(`)
                    let before = rest[..i].chars().last();
                    rest = &rest[i + pat.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let Some(s) = lit(rest) else { continue };
                    if ctx {
                        let after = &rest[s.1..];
                        let Some(j) = after.find('"') else { continue };
                        if let Some(w) = lit(&after[j + 1..]) {
                            out.insert(format!("{}|{}", s.0, w.0));
                        }
                    } else {
                        out.insert(s.0);
                    }
                }
            }
        }
        out
    }

    /// A Rust string literal's text from just after its opening quote, and
    /// where it ends.
    fn lit(s: &str) -> Option<(String, usize)> {
        let mut out = String::new();
        let mut chars = s.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((out, i + 1)),
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                },
                c => out.push(c),
            }
        }
        None
    }
}
