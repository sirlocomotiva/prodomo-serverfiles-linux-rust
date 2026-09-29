//! A small reader of the frozen legacy source, for the tests that pin a table to it.
//!
//! It understands exactly as much of the preprocessor as the pinned spans use: `#ifdef`,
//! `#ifndef`, a one-switch `#if defined(...)`, and `#endif`. Any other directive fails the test
//! instead of being guessed at, so a span that grows a new form is noticed.

use std::collections::BTreeSet;

/// A legacy file under `server/server/`, with its code-page bytes kept as replacement
/// characters.
pub fn legacy(path: &str) -> String {
    let bytes = std::fs::read(format!("../server/server/{path}"))
        .unwrap_or_else(|e| panic!("read {path}: {e}"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `text` with its `/* */` and `//` comments removed.
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find("*/").expect("a block comment ends");
        rest = &rest[at + end + 2..];
    }
    out.push_str(rest);
    out.lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The switches `prodomodefines.h` defines.
pub fn switches() -> BTreeSet<String> {
    strip_comments(&legacy("common/prodomodefines.h"))
        .lines()
        .filter_map(|line| line.trim().strip_prefix("#define "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// The lines of `text` the preprocessor keeps under `defined`. A directive this reader does
/// not model fails the test rather than being guessed at.
pub fn preprocess(text: &str, defined: &BTreeSet<String>) -> Vec<String> {
    let mut open: Vec<bool> = Vec::new();
    let mut kept = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("#ifdef ") {
            open.push(defined.contains(name.trim()));
        } else if let Some(name) = trimmed.strip_prefix("#ifndef ") {
            open.push(!defined.contains(name.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("#if defined(") {
            let name = rest.strip_suffix(')').expect("one switch per #if");
            open.push(defined.contains(name));
        } else if trimmed.starts_with("#endif") {
            open.pop().expect("an #endif closes an #if");
        } else if trimmed.starts_with('#') {
            panic!("a directive this reader does not model: {trimmed}");
        } else if open.iter().all(|on| *on) {
            kept.push(line.to_owned());
        }
    }
    assert!(open.is_empty(), "every #if is closed");
    kept
}

/// The part of `text` from the line holding `start` to the next line that is exactly `end`.
pub fn span<'a>(text: &'a str, start: &str, end: &str) -> Vec<&'a str> {
    let mut lines = text.lines().skip_while(|line| !line.contains(start));
    let first = lines.next().unwrap_or_else(|| panic!("no {start}"));
    std::iter::once(first)
        .chain(lines.take_while(|line| line.trim_end() != end))
        .collect()
}

/// The upper-case identifiers in `line` that start with `prefix`.
pub fn names<'a>(line: &'a str, prefix: &str) -> Vec<&'a str> {
    line.split(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .filter(|word| word.starts_with(prefix))
        .collect()
}

/// The members of the auto-numbered enum `enum_name` in `header`, in order, as this build
/// compiles it. The enum must give no member an explicit value except `last`, which ends the
/// list and is not returned.
pub fn auto_numbered(header: &str, enum_name: &str, prefix: &str, last: &str) -> Vec<String> {
    let text = strip_comments(&legacy(header));
    let body = span(&text, &format!("enum {enum_name}"), "};").join("\n");
    let mut members = Vec::new();
    for line in preprocess(&body, &switches()) {
        for name in names(&line, prefix) {
            if name == last {
                return members;
            }
            assert!(!line.contains('='), "{name} is auto-numbered");
            members.push(name.to_owned());
        }
    }
    panic!("{enum_name} has no {last}");
}
