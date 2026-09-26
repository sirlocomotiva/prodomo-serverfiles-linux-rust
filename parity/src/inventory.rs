//! The Parity inventory: the markdown tables in `.scratch/parity/`.
//!
//! Every table has the columns `id | source | what | ... | status | scenario | note`. This module
//! reads the `id`, `status`, and `scenario` columns of every table and checks the rules every
//! row keeps; `prodomo/tests/parity.rs` checks that each named scenario exists.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

/// A row's porting status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Nothing in the Rewrite yet.
    Missing,
    /// The wire codec exists and is golden-byte tested; no behaviour behind it.
    Codec,
    /// Some of the behaviour exists; no scenario proves it.
    Partial,
    /// A scenario that drives the real binary passes.
    Ported,
    /// Legacy never reaches it (dead code or an unread file); the note says why.
    Unused,
}

impl Status {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "missing" => Self::Missing,
            "codec" => Self::Codec,
            "partial" => Self::Partial,
            "ported" => Self::Ported,
            "unused" => Self::Unused,
            _ => return None,
        })
    }
}

/// One inventory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The table file the row is in.
    pub file: String,
    /// The row's ID, without backticks.
    pub id: String,
    /// The row's status.
    pub status: Status,
    /// The scenario that proves the row, empty when there is none.
    pub scenario: String,
}

/// A rule an inventory row breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem(pub String);

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Read every `*.md` table in `dir` except `spec.md` and `README.md`, which hold prose.
///
/// # Errors
///
/// Returns every problem found: an unreadable file, a table without the status and scenario
/// columns, a row with the wrong number of cells, an ID without backticks, or an unknown status.
pub fn load(dir: &Path) -> Result<Vec<Row>, Vec<Problem>> {
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    let mut names: Vec<String> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| is_table(name))
            .collect(),
        Err(error) => return Err(vec![Problem(format!("{}: {error}", dir.display()))]),
    };
    names.sort();
    for name in names {
        match fs::read_to_string(dir.join(&name)) {
            Ok(text) => parse_table(&name, &text, &mut rows, &mut problems),
            Err(error) => problems.push(Problem(format!("{name}: {error}"))),
        }
    }
    if problems.is_empty() {
        Ok(rows)
    } else {
        Err(problems)
    }
}

/// Whether `name` is an inventory table: a markdown file other than the two prose files.
fn is_table(name: &str) -> bool {
    let markdown = Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    markdown && !matches!(name, "spec.md" | "README.md")
}

/// The cells of a markdown table line, trimmed; `None` for a line that is not a table row.
fn cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(str::trim).collect())
}

fn parse_table(file: &str, text: &str, rows: &mut Vec<Row>, problems: &mut Vec<Problem>) {
    let mut columns: Option<(usize, usize, usize)> = None;
    for (index, line) in text.lines().enumerate() {
        let at = format!("{file}:{}", index + 1);
        let Some(cells) = cells(line) else {
            continue;
        };
        if cells.iter().all(|cell| cell.chars().all(|c| c == '-')) {
            continue;
        }
        if cells.first() == Some(&"id") {
            let find = |name: &str| cells.iter().position(|cell| *cell == name);
            match (find("status"), find("scenario")) {
                (Some(status), Some(scenario)) => columns = Some((cells.len(), status, scenario)),
                _ => problems.push(Problem(format!("{at}: no status or scenario column"))),
            }
            continue;
        }
        let Some((width, status_at, scenario_at)) = columns else {
            problems.push(Problem(format!("{at}: a row before the header")));
            continue;
        };
        if cells.len() != width {
            problems.push(Problem(format!(
                "{at}: {} cells, the header has {width} (a `|` inside a cell?)",
                cells.len()
            )));
            continue;
        }
        let Some(id) = cells[0]
            .strip_prefix('`')
            .and_then(|id| id.strip_suffix('`'))
        else {
            problems.push(Problem(format!(
                "{at}: the ID {:?} is not in backticks",
                cells[0]
            )));
            continue;
        };
        let Some(status) = Status::parse(cells[status_at]) else {
            problems.push(Problem(format!(
                "{at}: unknown status {:?}",
                cells[status_at]
            )));
            continue;
        };
        rows.push(Row {
            file: file.to_owned(),
            id: id.to_owned(),
            status,
            scenario: cells[scenario_at].trim_matches('`').to_owned(),
        });
    }
}

/// Check the rules that hold across rows: IDs are unique, a `ported` row names a scenario, and
/// only a `ported` row does.
#[must_use]
pub fn check(rows: &[Row]) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for row in rows {
        if let Some(first) = seen.insert(&row.id, &row.file) {
            problems.push(Problem(format!(
                "{}: `{}` is also in {first}",
                row.file, row.id
            )));
        }
        match (row.status, row.scenario.is_empty()) {
            (Status::Ported, true) => problems.push(Problem(format!(
                "{}: `{}` is ported but names no scenario",
                row.file, row.id
            ))),
            (Status::Ported, false) | (_, true) => {}
            (_, false) => problems.push(Problem(format!(
                "{}: `{}` names a scenario but is not ported",
                row.file, row.id
            ))),
        }
    }
    problems
}

/// How many rows have each status.
#[must_use]
pub fn counts(rows: &[Row]) -> BTreeMap<Status, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.status).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
# Title

Intro with a | pipe outside a table.

| id | source | what | status | scenario | note |
|---|---|---|---|---|---|
| `a.one` | `x.cpp:1` | first | ported | `scenario_one` | |
| `a.two` | `x.cpp:2` | second | missing | | a note |
";

    fn parse(text: &str) -> (Vec<Row>, Vec<Problem>) {
        let mut rows = Vec::new();
        let mut problems = Vec::new();
        parse_table("t.md", text, &mut rows, &mut problems);
        (rows, problems)
    }

    #[test]
    fn reads_the_id_status_and_scenario_columns() {
        let (rows, problems) = parse(TABLE);
        assert_eq!(problems, []);
        assert_eq!(
            rows,
            [
                Row {
                    file: "t.md".into(),
                    id: "a.one".into(),
                    status: Status::Ported,
                    scenario: "scenario_one".into(),
                },
                Row {
                    file: "t.md".into(),
                    id: "a.two".into(),
                    status: Status::Missing,
                    scenario: String::new(),
                },
            ]
        );
        assert_eq!(check(&rows), []);
    }

    #[test]
    fn skips_the_prose_files() {
        assert!(is_table("systems.md"));
        assert!(is_table("upper.MD"));
        assert!(!is_table("spec.md"));
        assert!(!is_table("README.md"));
        assert!(!is_table("gen.py"));
    }

    #[test]
    fn finds_the_status_column_wherever_it_is() {
        let text = "| id | path | status | scenario |\n|---|---|---|---|\n| `b` | p | unused | |\n";
        let (rows, problems) = parse(text);
        assert_eq!(problems, []);
        assert_eq!(rows[0].status, Status::Unused);
    }

    #[test]
    fn reports_a_malformed_row() {
        let text = TABLE.replace("| second |", "| sec | ond |");
        let (_, problems) = parse(&text);
        assert_eq!(
            problems,
            [Problem(
                "t.md:8: 7 cells, the header has 6 (a `|` inside a cell?)".into()
            )]
        );
    }

    #[test]
    fn reports_an_unknown_status_and_a_bare_id() {
        let text = TABLE
            .replace("| missing |", "| done |")
            .replace("`a.one`", "a.one");
        let (_, problems) = parse(&text);
        assert_eq!(
            problems,
            [
                Problem("t.md:7: the ID \"a.one\" is not in backticks".into()),
                Problem("t.md:8: unknown status \"done\"".into()),
            ]
        );
    }

    #[test]
    fn reports_duplicates_and_scenario_mismatches() {
        let (mut rows, _) = parse(TABLE);
        rows[1].scenario = "stray".into();
        rows.push(Row {
            file: "u.md".into(),
            id: "a.one".into(),
            status: Status::Ported,
            scenario: String::new(),
        });
        assert_eq!(
            check(&rows),
            [
                Problem("t.md: `a.two` names a scenario but is not ported".into()),
                Problem("u.md: `a.one` is also in t.md".into()),
                Problem("u.md: `a.one` is ported but names no scenario".into()),
            ]
        );
    }
}
