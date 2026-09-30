//! The Rust `qc` against the Game data: the `quest_list` scripts compile to the FreeBSD `qc`
//! output in `object/` byte for byte, and every script compiles.

use std::fs;
use std::path::{Path, PathBuf};

use quest::qc::{Compiler, Objects};
use quest::sources;

fn quest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/quest")
}

fn read_tree(root: &Path, relative: &Path, files: &mut Objects) {
    for entry in fs::read_dir(root.join(relative)).unwrap() {
        let entry = entry.unwrap();
        let path = relative.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            read_tree(root, &path, files);
        } else {
            let key = path.to_str().unwrap().as_bytes().to_vec();
            files.insert(key, fs::read(root.join(&path)).unwrap());
        }
    }
}

fn compile(compiler: &Compiler, path: &Path) -> quest::qc::Compiled {
    let source = fs::read(quest_dir().join(path)).unwrap();
    compiler
        .compile(&source)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn quest_list_compiles_to_the_reference_objects() {
    let qc = Compiler::new();
    let mut compiled = Objects::new();
    let listed = sources::quest_list(&quest_dir()).unwrap();
    assert_eq!(listed.len(), 16);
    for path in &listed {
        compiled.extend(compile(&qc, path).files);
    }
    let mut reference = Objects::new();
    read_tree(&quest_dir().join("object"), Path::new(""), &mut reference);
    assert_eq!(reference.len(), 95);
    let names = |files: &Objects| -> Vec<String> {
        files
            .keys()
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect()
    };
    assert_eq!(names(&compiled), names(&reference));
    for (path, bytes) in &reference {
        assert!(
            compiled[path] == *bytes,
            "{} differs:\n{}\n---\n{}",
            String::from_utf8_lossy(path),
            String::from_utf8_lossy(&compiled[path]),
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn every_script_but_change_empire_compiles_and_no_library_does() {
    let qc = Compiler::new();
    let scripts = sources::scripts(&quest_dir()).unwrap();
    assert_eq!(scripts.len(), 51);
    let broken = Path::new("_basic/change_empire.lua");
    for path in scripts.iter().filter(|path| *path != broken) {
        compile(&qc, path);
    }
    // Line 132 closes the `if ret == 1` chain before its `elseif ret == 4`; legacy's `qc`
    // aborts on it as well, so it never reached `object/`.
    let source = fs::read(quest_dir().join(broken)).unwrap();
    let error = qc.compile(&source).unwrap_err();
    assert_eq!(
        error.to_string(),
        "line 139: expecting 'when' or 'function'"
    );
    let mut libraries: Vec<PathBuf> = [
        "questlib.lua",
        "questlib_extra.lua",
        "questing.lua",
        "GFquestlib.lua",
        "locale.lua",
        "multiLocale.lua",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    for entry in fs::read_dir(quest_dir().join("luaLibrary")).unwrap() {
        libraries.push(Path::new("luaLibrary").join(entry.unwrap().file_name()));
    }
    assert_eq!(libraries.len(), 10);
    for path in &libraries {
        let source = fs::read(quest_dir().join(path)).unwrap();
        let error = qc.compile(&source).unwrap_err();
        assert_eq!(
            error.message,
            "must start with 'quest'",
            "{}",
            path.display()
        );
    }
}

#[test]
fn the_load_order_starts_with_quest_list() {
    let order = sources::load_order(&quest_dir()).unwrap();
    let listed = sources::quest_list(&quest_dir()).unwrap();
    assert_eq!(order.len(), 51);
    assert_eq!(order[..listed.len()], listed[..]);
    let mut rest = order[listed.len()..].to_vec();
    rest.sort();
    assert_eq!(rest, order[listed.len()..]);
}
