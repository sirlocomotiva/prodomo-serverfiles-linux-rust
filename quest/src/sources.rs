//! The quest sources of a locale's `quest` directory.
//!
//! Every file in a subdirectory of `quest/` is a quest script (ADR-0004 makes all of them live),
//! except the compiled `object/` and the libraries of `luaLibrary/`; the files at the top level
//! are libraries and lists. `quest_list` names the scripts legacy compiled, one path per line
//! (CRLF in the Game data). Legacy numbers quests in the order of `object/state/`, which its file
//! system chooses; the Rewrite compiles `quest_list` in order and then the other scripts by path
//! (ADR-0006).

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// The directories below `quest/` that hold no quest script.
const NOT_SCRIPTS: [&str; 2] = ["object", "luaLibrary"];

/// The scripts `quest_list` names, in its order, as paths relative to `quest_dir`.
///
/// # Errors
///
/// The list cannot be read, or a line is not UTF-8 or leaves `quest_dir`.
pub fn quest_list(quest_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let list = fs::read(quest_dir.join("quest_list"))?;
    let mut paths = Vec::new();
    for line in list.split(|byte| *byte == b'\n') {
        let line = std::str::from_utf8(line)
            .map_err(|_| invalid("a quest_list line is not UTF-8".to_owned()))?
            .trim();
        if line.is_empty() {
            continue;
        }
        let path = PathBuf::from(line);
        if !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(invalid(format!(
                "quest_list names {line}, outside the quest directory"
            )));
        }
        paths.push(path);
    }
    Ok(paths)
}

/// Every quest script, as paths relative to `quest_dir`, sorted.
///
/// # Errors
///
/// A directory cannot be read.
pub fn scripts(quest_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(quest_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if entry.file_type()?.is_dir() && !NOT_SCRIPTS.iter().any(|skip| name == *skip) {
            walk(quest_dir, &PathBuf::from(name), &mut paths)?;
        }
    }
    paths.sort();
    Ok(paths)
}

fn walk(root: &Path, relative: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            walk(root, &path, paths)?;
        } else {
            paths.push(path);
        }
    }
    Ok(())
}

/// The order the Rewrite compiles and numbers scripts in: `quest_list`, then every other script
/// by path.
///
/// # Errors
///
/// As [`quest_list`] and [`scripts`], or `quest_list` names a file that is not a script.
pub fn load_order(quest_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let listed = quest_list(quest_dir)?;
    let all = scripts(quest_dir)?;
    if let Some(missing) = listed.iter().find(|path| !all.contains(path)) {
        return Err(invalid(format!(
            "quest_list names {}, which is not a quest script",
            missing.display()
        )));
    }
    let mut order = Vec::with_capacity(all.len());
    for path in listed.into_iter().chain(all) {
        if !order.contains(&path) {
            order.push(path);
        }
    }
    Ok(order)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes `files` below a quest directory of the test's own, runs `run` on it and removes the
    /// directory.
    fn in_quest_dir<T>(name: &str, files: &[(&str, &str)], run: impl FnOnce(&Path) -> T) -> T {
        let root =
            std::env::temp_dir().join(format!("quest-sources-{}-{name}", std::process::id()));
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let result = run(&root);
        fs::remove_dir_all(&root).unwrap();
        result
    }

    fn refusal(result: io::Result<Vec<PathBuf>>) -> String {
        result.unwrap_err().to_string()
    }

    /// A `quest_list` line is trimmed, and a path that climbs out of the quest directory or starts
    /// at the root is refused.
    #[test]
    fn a_quest_list_path_stays_inside_the_quest_directory() {
        let files = [("quest_list", "a/one.lua\r\n\r\n  b/two.lua  ")];
        let listed = in_quest_dir("inside", &files, quest_list).unwrap();
        assert_eq!(listed, ["a/one.lua", "b/two.lua"].map(PathBuf::from));
        let outside = ["../one.lua", "a/../../one.lua", "/etc/one.lua"];
        for (index, line) in outside.into_iter().enumerate() {
            let files = [("quest_list", line)];
            let refused = in_quest_dir(&format!("outside-{index}"), &files, quest_list);
            let message = format!("quest_list names {line}, outside the quest directory");
            assert_eq!(refusal(refused), message);
        }
    }

    /// The load order is `quest_list`, then every other script by path, each once; `quest_list`
    /// may name only a script, not a missing file, a compiled file or a library.
    #[test]
    fn the_load_order_is_the_list_then_the_other_scripts_each_once() {
        let files = [
            ("quest_list", "b/two.lua\na/one.lua\nb/two.lua\n"),
            ("a/one.lua", ""),
            ("a/three.lua", ""),
            ("b/two.lua", ""),
            ("object/state/one", ""),
            ("luaLibrary/lib.lua", ""),
            ("questlib.lua", ""),
        ];
        let order = in_quest_dir("order", &files, load_order).unwrap();
        assert_eq!(
            order,
            ["b/two.lua", "a/one.lua", "a/three.lua"].map(PathBuf::from)
        );
        let not_scripts = [
            "a/four.lua",
            "object/state/one",
            "luaLibrary/lib.lua",
            "questlib.lua",
        ];
        for (index, listed) in not_scripts.into_iter().enumerate() {
            let mut files = files;
            files[0] = ("quest_list", listed);
            let refused = in_quest_dir(&format!("not-a-script-{index}"), &files, load_order);
            let message = format!("quest_list names {listed}, which is not a quest script");
            assert_eq!(refusal(refused), message);
        }
    }
}
