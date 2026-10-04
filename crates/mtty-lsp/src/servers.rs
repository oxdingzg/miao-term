//! Which server a file goes to, how it is started, and the workspace root
//! it is started for.

use std::path::{Path, PathBuf};

/// A server for a group of languages: the key that names it in
/// `config.toml` (`[lsp.rust]`), its default command and the files that
/// mark a project's root.
pub struct Server {
    pub key: &'static str,
    pub command: &'static [&'static str],
    pub root_markers: &'static [&'static str],
}

pub const SERVERS: &[Server] = &[
    Server {
        key: "rust",
        command: &["rust-analyzer"],
        root_markers: &["Cargo.toml"],
    },
    Server {
        key: "typescript",
        command: &["typescript-language-server", "--stdio"],
        root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
    },
    Server {
        key: "python",
        command: &["pyright-langserver", "--stdio"],
        root_markers: &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
        ],
    },
    Server {
        key: "go",
        command: &["gopls"],
        root_markers: &["go.mod", "go.work"],
    },
    Server {
        key: "c",
        command: &["clangd"],
        root_markers: &[
            "compile_commands.json",
            "compile_flags.txt",
            ".clangd",
            "CMakeLists.txt",
        ],
    },
];

pub fn server(key: &str) -> Option<&'static Server> {
    SERVERS.iter().find(|s| s.key == key)
}

/// The server key and LSP language id for a file, from the language the
/// editor highlights it as ("Rust", "TSX"…) and its extension.
pub fn language(language: &str, path: &Path) -> Option<(&'static str, &'static str)> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    Some(match language {
        "Rust" => ("rust", "rust"),
        "TypeScript" => ("typescript", "typescript"),
        "TSX" => ("typescript", "typescriptreact"),
        "JavaScript" if ext == "jsx" => ("typescript", "javascriptreact"),
        "JavaScript" => ("typescript", "javascript"),
        "Python" => ("python", "python"),
        "Go" => ("go", "go"),
        "C" => ("c", "c"),
        "C++" => ("c", "cpp"),
        "Objective-C" => ("c", "objective-c"),
        _ => return None,
    })
}

/// The workspace root for `file`: the nearest folder holding one of
/// `markers`, else the nearest with `.git`, else the file's folder.
pub fn find_root(file: &Path, markers: &[String]) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("/"));
    let find = |names: &[&str]| {
        dir.ancestors()
            .find(|d| names.iter().any(|n| d.join(n).exists()))
            .map(Path::to_path_buf)
    };
    let markers: Vec<&str> = markers.iter().map(String::as_str).collect();
    find(&markers)
        .or_else(|| find(&[".git"]))
        .unwrap_or_else(|| dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_prefer_project_markers_then_git() {
        let base = std::env::temp_dir().join(format!("mtty-lsp-root-{}", std::process::id()));
        let crate_dir = base.join("repo/crates/a");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::create_dir_all(base.join("repo/.git")).unwrap();
        std::fs::create_dir_all(base.join("repo/notes")).unwrap();
        std::fs::write(crate_dir.join("Cargo.toml"), "").unwrap();
        let markers = vec!["Cargo.toml".to_string()];
        assert_eq!(
            find_root(&crate_dir.join("src/lib.rs"), &markers),
            crate_dir
        );
        assert_eq!(
            find_root(&base.join("repo/notes/x.rs"), &markers),
            base.join("repo")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn languages_map_to_servers() {
        assert_eq!(
            language("TSX", Path::new("a.tsx")),
            Some(("typescript", "typescriptreact"))
        );
        assert_eq!(
            language("JavaScript", Path::new("a.JSX")),
            Some(("typescript", "javascriptreact"))
        );
        assert_eq!(language("C++", Path::new("a.cc")), Some(("c", "cpp")));
        assert_eq!(language("Markdown", Path::new("a.md")), None);
        assert!(SERVERS.iter().all(|s| !s.command.is_empty()));
    }
}
