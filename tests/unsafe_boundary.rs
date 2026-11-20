//! The crate keeps `unsafe` inside `src/win/`; this test enforces that policy.

use std::path::{Path, PathBuf};

const FORBIDDEN: [&str; 5] = [
    "unsafe {",
    "unsafe fn",
    "unsafe impl",
    "unsafe extern",
    "unsafe trait",
];

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn is_win_module(root: &Path, file: &Path) -> bool {
    let relative = file.strip_prefix(root).unwrap_or(file);
    relative
        .components()
        .any(|component| component.as_os_str() == "win")
}

#[test]
fn unsafe_is_confined_to_the_win_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(!files.is_empty(), "no sources found");

    let mut violations = Vec::new();
    for file in &files {
        if is_win_module(&root, file) {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("read source");
        for pattern in FORBIDDEN {
            if text.contains(pattern) {
                violations.push(format!("{} contains `{pattern}`", file.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "unsafe outside src/win: {violations:#?}"
    );
}

#[test]
fn the_win_module_actually_contains_unsafe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let win_files: Vec<&PathBuf> = files
        .iter()
        .filter(|file| is_win_module(&root, file))
        .collect();
    assert!(!win_files.is_empty(), "the win module is missing");
    let has_unsafe = win_files.iter().any(|file| {
        let text = std::fs::read_to_string(file).expect("read source");
        FORBIDDEN.iter().any(|pattern| text.contains(pattern))
    });
    assert!(has_unsafe, "the FFI layer should contain the unsafe blocks");
}
