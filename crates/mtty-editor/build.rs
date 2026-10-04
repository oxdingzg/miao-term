//! Compile the Sublime syntaxes mtty highlights with when no tree-sitter
//! grammar claims a file: syntect's default set plus the permissively
//! licensed syntaxes vendored under `vendor/syntaxes` (see
//! `docs/third-party/SYNTAXES.md`). The result is a dump the crate embeds.

use std::path::{Path, PathBuf};

use syntect::parsing::syntax_definition::SyntaxDefinition;
use syntect::parsing::SyntaxSet;

fn sublime_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("vendor/syntaxes is readable")
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sublime_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "sublime-syntax") {
            out.push(path);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendored = manifest.join("../../vendor/syntaxes");
    println!("cargo:rerun-if-changed={}", vendored.display());
    let mut files = Vec::new();
    sublime_files(&vendored, &mut files);
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    let mut broken = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let name = path.file_stem().and_then(|s| s.to_str());
        match SyntaxDefinition::load_from_str(&text, true, name) {
            Ok(def) => builder.add(def),
            Err(e) => broken.push(format!("{}: {e}", path.display())),
        }
    }
    // Every vendored file must load: drop one from vendor/syntaxes rather
    // than skipping it silently here.
    assert!(
        broken.is_empty(),
        "vendored syntaxes fail to load:\n{}",
        broken.join("\n")
    );
    let set = builder.build();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("syntaxes.packdump");
    syntect::dumps::dump_to_uncompressed_file(&set, out).expect("write the syntax dump");
}
