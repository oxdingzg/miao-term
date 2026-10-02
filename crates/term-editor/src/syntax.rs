//! Syntax highlighting with tree-sitter (ADR 0034, phase E3).
//!
//! A [`Syntax`] keeps a parse tree for one document. Edits from
//! [`Document::take_edits`](crate::Document::take_edits) update it
//! incrementally, and [`Syntax::highlights`] runs the language's highlight
//! query over a byte range only (the visible lines), so the cost follows the
//! screen, not the file.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use ropey::Rope;
use tree_sitter::{
    InputEdit, Language, Node, Parser, Point, Query, QueryCursor, StreamingIterator, TextProvider,
    Tree,
};

use crate::change::ByteEdit;

/// Documents larger than this are not parsed (they stay plain text). Parsing
/// runs on the UI thread, and a reparse after a keystroke grows with the file
/// (about 4 ms per MB of Rust on an M4: tree-sitter re-walks the top-level
/// items after the edit), so the cap keeps a keystroke within a frame.
/// Reparsing on a background thread would lift it.
pub const MAX_HIGHLIGHT_BYTES: usize = 1 << 20;

/// What a highlighted span is, mapped to a colour by the editor's palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Highlight {
    Keyword,
    String,
    Escape,
    Comment,
    Number,
    Constant,
    Function,
    Type,
    Property,
    Variable,
    Tag,
    Attribute,
    Operator,
    Punctuation,
    Heading,
    Link,
}

impl Highlight {
    /// A highlight-query capture name (`keyword.control`, `string.special`,
    /// `function.method`…) by its most significant part.
    pub fn from_capture(name: &str) -> Option<Highlight> {
        let head = name.split('.').next().unwrap_or(name);
        Some(match head {
            "keyword" | "include" | "conditional" | "repeat" | "exception" | "storageclass" => {
                Highlight::Keyword
            }
            "string" if name.contains("escape") || name.contains("special") => Highlight::Escape,
            "string" | "character" => Highlight::String,
            "escape" => Highlight::Escape,
            "comment" => Highlight::Comment,
            "number" | "float" | "boolean" => Highlight::Number,
            // Literals: numbers, and the built-in constants queries tag the
            // same way (`true`, `None`, Rust's integer literals).
            "constant" if name.contains("numeric") || name.contains("builtin") => Highlight::Number,
            "constant" => Highlight::Constant,
            "function" | "method" | "constructor" => Highlight::Function,
            "type" => Highlight::Type,
            "property" | "field" => Highlight::Property,
            "variable" if name.contains("builtin") || name.contains("parameter") => {
                Highlight::Variable
            }
            "label" | "module" | "namespace" => Highlight::Type,
            "tag" => Highlight::Tag,
            "attribute" => Highlight::Attribute,
            "operator" => Highlight::Operator,
            "punctuation" => Highlight::Punctuation,
            "text" | "markup" if name.contains("title") || name.contains("heading") => {
                Highlight::Heading
            }
            "text" | "markup" if name.contains("uri") || name.contains("link") => Highlight::Link,
            "text" | "markup" if name.contains("literal") || name.contains("raw") => {
                Highlight::String
            }
            _ => return None,
        })
    }
}

/// A built-in language: how files are recognised, its grammar and query.
struct LangDef {
    name: &'static str,
    extensions: &'static [&'static str],
    file_names: &'static [&'static str],
    language: fn() -> Language,
    query: fn() -> String,
}

fn q(parts: &[&str]) -> String {
    parts.join("\n")
}

static LANGS: &[LangDef] = &[
    LangDef {
        name: "Rust",
        extensions: &["rs"],
        file_names: &[],
        language: || tree_sitter_rust::LANGUAGE.into(),
        query: || tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        file_names: &[],
        language: || tree_sitter_javascript::LANGUAGE.into(),
        query: || {
            q(&[
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            ])
        },
    },
    LangDef {
        name: "TypeScript",
        extensions: &["ts", "mts", "cts"],
        file_names: &[],
        language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        query: || {
            q(&[
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                tree_sitter_javascript::HIGHLIGHT_QUERY,
            ])
        },
    },
    LangDef {
        name: "TSX",
        extensions: &["tsx"],
        file_names: &[],
        language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        query: || {
            q(&[
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            ])
        },
    },
    LangDef {
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        file_names: &[],
        language: || tree_sitter_python::LANGUAGE.into(),
        query: || tree_sitter_python::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Go",
        extensions: &["go"],
        file_names: &[],
        language: || tree_sitter_go::LANGUAGE.into(),
        query: || tree_sitter_go::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "C",
        extensions: &["c", "h"],
        file_names: &[],
        language: || tree_sitter_c::LANGUAGE.into(),
        query: || tree_sitter_c::HIGHLIGHT_QUERY.into(),
    },
    LangDef {
        name: "C++",
        extensions: &["cc", "cpp", "cxx", "hpp", "hh", "hxx", "ipp"],
        file_names: &[],
        language: || tree_sitter_cpp::LANGUAGE.into(),
        query: || {
            q(&[
                tree_sitter_cpp::HIGHLIGHT_QUERY,
                tree_sitter_c::HIGHLIGHT_QUERY,
            ])
        },
    },
    LangDef {
        name: "Java",
        extensions: &["java"],
        file_names: &[],
        language: || tree_sitter_java::LANGUAGE.into(),
        query: || tree_sitter_java::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Shell",
        extensions: &["sh", "bash", "ksh", "env"],
        file_names: &[
            ".bashrc",
            ".bash_profile",
            ".profile",
            "PKGBUILD",
            ".env",
            ".env.local",
            ".env.example",
            ".env.development",
            ".env.production",
        ],
        language: || tree_sitter_bash::LANGUAGE.into(),
        query: || tree_sitter_bash::HIGHLIGHT_QUERY.into(),
    },
    LangDef {
        name: "JSON",
        extensions: &["json", "jsonc", "json5", "webmanifest"],
        file_names: &[".prettierrc", ".babelrc"],
        language: || tree_sitter_json::LANGUAGE.into(),
        query: || tree_sitter_json::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "TOML",
        extensions: &["toml"],
        file_names: &["Cargo.lock", "Pipfile"],
        language: || tree_sitter_toml_ng::LANGUAGE.into(),
        query: || tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "YAML",
        extensions: &["yaml", "yml"],
        file_names: &[],
        language: || tree_sitter_yaml::LANGUAGE.into(),
        query: || tree_sitter_yaml::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "HTML",
        extensions: &["html", "htm", "xhtml"],
        file_names: &[],
        language: || tree_sitter_html::LANGUAGE.into(),
        query: || tree_sitter_html::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "CSS",
        extensions: &["css"],
        file_names: &[],
        language: || tree_sitter_css::LANGUAGE.into(),
        query: || tree_sitter_css::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Markdown",
        extensions: &["md", "markdown", "mdx"],
        file_names: &[],
        language: || tree_sitter_md::LANGUAGE.into(),
        query: || tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.into(),
    },
    LangDef {
        name: "Lua",
        extensions: &["lua"],
        file_names: &[],
        language: || tree_sitter_lua::LANGUAGE.into(),
        query: || tree_sitter_lua::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Ruby",
        extensions: &["rb", "rake", "gemspec"],
        file_names: &["Gemfile", "Rakefile"],
        language: || tree_sitter_ruby::LANGUAGE.into(),
        query: || tree_sitter_ruby::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Swift",
        extensions: &["swift"],
        file_names: &[],
        language: || tree_sitter_swift::LANGUAGE.into(),
        query: || tree_sitter_swift::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Makefile",
        extensions: &["mk", "mak"],
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        language: || tree_sitter_make::LANGUAGE.into(),
        query: || tree_sitter_make::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "C#",
        extensions: &["cs", "csx"],
        file_names: &[],
        language: || tree_sitter_c_sharp::LANGUAGE.into(),
        query: || tree_sitter_c_sharp::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "PHP",
        extensions: &["php", "phtml"],
        file_names: &[],
        language: || tree_sitter_php::LANGUAGE_PHP.into(),
        query: || tree_sitter_php::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Kotlin",
        extensions: &["kt", "kts"],
        file_names: &[],
        language: || tree_sitter_kotlin_sg::LANGUAGE.into(),
        query: || tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Scala",
        extensions: &["scala", "sc", "sbt"],
        file_names: &[],
        language: || tree_sitter_scala::LANGUAGE.into(),
        query: || tree_sitter_scala::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Dart",
        extensions: &["dart"],
        file_names: &[],
        language: || tree_sitter_dart::LANGUAGE.into(),
        query: || tree_sitter_dart::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Objective-C",
        extensions: &["m", "mm"],
        file_names: &[],
        language: || tree_sitter_objc::LANGUAGE.into(),
        query: || {
            q(&[
                tree_sitter_objc::HIGHLIGHTS_QUERY,
                tree_sitter_c::HIGHLIGHT_QUERY,
            ])
        },
    },
    LangDef {
        name: "Perl",
        extensions: &["pl", "pm", "t"],
        file_names: &[],
        language: || arborium_perl::language().into(),
        query: || arborium_perl::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "R",
        extensions: &["r", "R"],
        file_names: &[],
        language: || tree_sitter_r::LANGUAGE.into(),
        query: || tree_sitter_r::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "PowerShell",
        extensions: &["ps1", "psm1", "psd1"],
        file_names: &[],
        language: || tree_sitter_powershell::LANGUAGE.into(),
        query: || tree_sitter_powershell::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Haskell",
        extensions: &["hs", "lhs"],
        file_names: &[],
        language: || tree_sitter_haskell::LANGUAGE.into(),
        query: || tree_sitter_haskell::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Elixir",
        extensions: &["ex", "exs"],
        file_names: &["mix.lock"],
        language: || tree_sitter_elixir::LANGUAGE.into(),
        query: || tree_sitter_elixir::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "HEEx",
        extensions: &["heex"],
        file_names: &[],
        language: || tree_sitter_heex::LANGUAGE.into(),
        query: || tree_sitter_heex::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Erlang",
        extensions: &["erl", "hrl"],
        file_names: &["rebar.config"],
        language: || tree_sitter_erlang::LANGUAGE.into(),
        query: || tree_sitter_erlang::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Zig",
        extensions: &["zig", "zon"],
        file_names: &[],
        language: || tree_sitter_zig::LANGUAGE.into(),
        query: || tree_sitter_zig::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Groovy",
        extensions: &["groovy", "gradle", "gvy"],
        file_names: &["Jenkinsfile"],
        language: || dekobon_tree_sitter_groovy::LANGUAGE.into(),
        query: || dekobon_tree_sitter_groovy::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Clojure",
        extensions: &["clj", "cljs", "cljc", "edn"],
        file_names: &[],
        language: || tree_sitter_clojure_orchard::LANGUAGE.into(),
        query: || tree_sitter_clojure_orchard::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Elm",
        extensions: &["elm"],
        file_names: &[],
        language: || tree_sitter_elm::LANGUAGE.into(),
        query: || tree_sitter_elm::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Gleam",
        extensions: &["gleam"],
        file_names: &[],
        language: || tree_sitter_gleam::LANGUAGE.into(),
        query: || tree_sitter_gleam::HIGHLIGHT_QUERY.into(),
    },
    LangDef {
        name: "Solidity",
        extensions: &["sol"],
        file_names: &[],
        language: || tree_sitter_solidity::LANGUAGE.into(),
        query: || tree_sitter_solidity::HIGHLIGHT_QUERY.into(),
    },
    LangDef {
        name: "Visual Basic",
        extensions: &["vb", "vbs", "bas"],
        file_names: &[],
        language: || arborium_vb::language().into(),
        query: || arborium_vb::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "MATLAB",
        extensions: &["matlab"],
        file_names: &[],
        language: || arborium_matlab::language().into(),
        query: || arborium_matlab::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Scheme",
        extensions: &["scm", "ss"],
        file_names: &[],
        language: || tree_sitter_scheme::LANGUAGE.into(),
        query: || tree_sitter_scheme::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Racket",
        extensions: &["rkt"],
        file_names: &[],
        language: || tree_sitter_racket::LANGUAGE.into(),
        query: || tree_sitter_racket::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Common Lisp",
        extensions: &["lisp", "lsp", "cl", "asd"],
        file_names: &[],
        language: || arborium_commonlisp::language().into(),
        query: || arborium_commonlisp::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Assembly",
        extensions: &["asm", "s", "S"],
        file_names: &[],
        language: || tree_sitter_asm::LANGUAGE.into(),
        query: || tree_sitter_asm::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Ada",
        extensions: &["ada", "adb", "ads"],
        file_names: &[],
        language: || arborium_ada::language().into(),
        query: || arborium_ada::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Vue",
        extensions: &["vue"],
        file_names: &[],
        language: || arborium_vue::language().into(),
        query: || q(&[&arborium_vue::HIGHLIGHTS_QUERY]),
    },
    LangDef {
        name: "Svelte",
        extensions: &["svelte"],
        file_names: &[],
        language: || tree_sitter_svelte_ng::LANGUAGE.into(),
        query: || {
            q(&[
                tree_sitter_svelte_ng::HIGHLIGHTS_QUERY,
                tree_sitter_html::HIGHLIGHTS_QUERY,
            ])
        },
    },
    LangDef {
        name: "SCSS",
        extensions: &["scss", "sass"],
        file_names: &[],
        language: || arborium_scss::language().into(),
        query: || q(&[&arborium_scss::HIGHLIGHTS_QUERY]),
    },
    LangDef {
        name: "LESS",
        extensions: &["less"],
        file_names: &[],
        language: || tree_sitter_less::language(),
        query: || tree_sitter_less::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "GraphQL",
        extensions: &["graphql", "gql"],
        file_names: &[],
        language: || arborium_graphql::language().into(),
        query: || arborium_graphql::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "XML",
        extensions: &[
            "xml", "svg", "xsd", "xsl", "xslt", "plist", "csproj", "fsproj", "vcxproj", "xaml",
            "rss", "atom",
        ],
        file_names: &[],
        language: || tree_sitter_xml::LANGUAGE_XML.into(),
        query: || tree_sitter_xml::XML_HIGHLIGHT_QUERY.into(),
    },
    LangDef {
        name: "Embedded Template",
        extensions: &["erb", "ejs"],
        file_names: &[],
        language: || tree_sitter_embedded_template::LANGUAGE.into(),
        query: || tree_sitter_embedded_template::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Jinja",
        extensions: &["j2", "jinja", "jinja2"],
        file_names: &[],
        language: || arborium_jinja2::language().into(),
        query: || arborium_jinja2::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Regex",
        extensions: &["regex"],
        file_names: &[],
        language: || tree_sitter_regex::LANGUAGE.into(),
        query: || tree_sitter_regex::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Dockerfile",
        extensions: &["dockerfile", "containerfile"],
        file_names: &["Dockerfile", "Containerfile"],
        language: || tree_sitter_containerfile::LANGUAGE.into(),
        query: || tree_sitter_containerfile::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "HCL",
        extensions: &["hcl", "tf", "tfvars", "nomad"],
        file_names: &[],
        language: || arborium_hcl::language().into(),
        query: || arborium_hcl::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Nix",
        extensions: &["nix"],
        file_names: &[],
        language: || tree_sitter_nix::LANGUAGE.into(),
        query: || tree_sitter_nix::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "INI",
        extensions: &["ini", "cfg", "conf", "editorconfig", "desktop", "service"],
        file_names: &[".editorconfig", ".gitconfig", ".npmrc"],
        language: || tree_sitter_ini::LANGUAGE.into(),
        query: || tree_sitter_ini::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "CMake",
        extensions: &["cmake"],
        file_names: &["CMakeLists.txt"],
        language: || tree_sitter_cmake::LANGUAGE.into(),
        query: || tree_sitter_cmake::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Meson",
        extensions: &["meson"],
        file_names: &["meson.build", "meson_options.txt"],
        language: || arborium_meson::language().into(),
        query: || arborium_meson::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Protobuf",
        extensions: &["proto"],
        file_names: &[],
        language: || arborium_proto::language().into(),
        query: || arborium_proto::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Thrift",
        extensions: &["thrift"],
        file_names: &[],
        language: || arborium_thrift::language().into(),
        query: || arborium_thrift::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "SQL",
        extensions: &["sql", "psql", "mysql"],
        file_names: &[],
        language: || tree_sitter_sequel::LANGUAGE.into(),
        query: || tree_sitter_sequel::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Diff",
        extensions: &["diff", "patch"],
        file_names: &[],
        language: || tree_sitter_diff::LANGUAGE.into(),
        query: || tree_sitter_diff::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Git Attributes",
        extensions: &[],
        file_names: &[".gitattributes"],
        language: || arborium_gitattributes::language().into(),
        query: || arborium_gitattributes::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Properties",
        extensions: &["properties"],
        file_names: &[],
        language: || tree_sitter_properties::LANGUAGE.into(),
        query: || tree_sitter_properties::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "SSH Config",
        extensions: &[],
        file_names: &["ssh_config", "sshd_config"],
        language: || arborium_ssh_config::language().into(),
        query: || arborium_ssh_config::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Just",
        extensions: &["just"],
        file_names: &["justfile", "Justfile", ".justfile"],
        language: || arborium_just::language().into(),
        query: || arborium_just::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Starlark",
        extensions: &["bzl", "star", "bazel"],
        file_names: &[
            "BUILD",
            "BUILD.bazel",
            "WORKSPACE",
            "MODULE.bazel",
            "Tiltfile",
        ],
        language: || tree_sitter_starlark::LANGUAGE.into(),
        query: || tree_sitter_starlark::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Fish",
        extensions: &["fish"],
        file_names: &[],
        language: || tree_sitter_fish::language(),
        query: || tree_sitter_fish::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Zsh",
        extensions: &["zsh"],
        file_names: &[".zshrc", ".zprofile", ".zshenv", ".zlogin"],
        language: || arborium_zsh::language().into(),
        query: || arborium_zsh::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Batch",
        extensions: &["bat", "cmd"],
        file_names: &[],
        language: || arborium_batch::language().into(),
        query: || arborium_batch::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Vim Script",
        extensions: &["vim"],
        file_names: &[".vimrc", "_vimrc"],
        language: || tree_sitter_vim::language(),
        query: || tree_sitter_vim::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "jq",
        extensions: &["jq"],
        file_names: &[],
        language: || arborium_jq::language().into(),
        query: || arborium_jq::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Bicep",
        extensions: &["bicep"],
        file_names: &[],
        language: || tree_sitter_bicep::LANGUAGE.into(),
        query: || tree_sitter_bicep::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Requirements",
        extensions: &[],
        file_names: &["requirements.txt", "constraints.txt"],
        language: || tree_sitter_requirements::LANGUAGE.into(),
        query: || tree_sitter_requirements::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "GLSL",
        extensions: &["glsl", "vert", "frag", "geom", "comp", "tesc", "tese"],
        file_names: &[],
        language: || tree_sitter_glsl::LANGUAGE_GLSL.into(),
        query: || tree_sitter_glsl::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "AsciiDoc",
        extensions: &["adoc", "asciidoc"],
        file_names: &[],
        language: || arborium_asciidoc::language().into(),
        query: || arborium_asciidoc::HIGHLIGHTS_QUERY.into(),
    },
    LangDef {
        name: "Typst",
        extensions: &["typ"],
        file_names: &[],
        language: || arborium_typst::language().into(),
        query: || arborium_typst::HIGHLIGHTS_QUERY.into(),
    },
];

/// The built-in language for a file, by name, then extension, then a `#!`
/// interpreter on the first line.
fn lang_for(path: &Path, first_line: &str) -> Option<&'static LangDef> {
    let name = path.file_name()?.to_str()?;
    if let Some(def) = LANGS.iter().find(|l| l.file_names.contains(&name)) {
        return Some(def);
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if let Some(def) = LANGS.iter().find(|l| l.extensions.contains(&ext.as_str())) {
            return Some(def);
        }
    }
    let shebang = first_line.strip_prefix("#!")?;
    let interpreter = shebang
        .split_whitespace()
        .flat_map(|w| w.rsplit('/').next())
        .find(|w| *w != "env")?;
    let ext = match interpreter.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.') {
        "sh" | "bash" | "dash" | "ksh" => "sh",
        "zsh" => "zsh",
        "fish" => "fish",
        "perl" => "pl",
        "php" => "php",
        "python" => "py",
        "node" | "deno" | "bun" => "js",
        "ruby" => "rb",
        "lua" => "lua",
        _ => return None,
    };
    LANGS.iter().find(|l| l.extensions.contains(&ext))
}

/// A compiled query and what each of its captures paints. Queries are built
/// once per language and shared by every document in it.
struct Compiled {
    query: Query,
    kinds: Vec<Option<Highlight>>,
}

/// Compiled queries by language name; `None` when a query failed to build.
type QueryCache = HashMap<&'static str, Option<Arc<Compiled>>>;

fn compiled(def: &'static LangDef) -> Option<Arc<Compiled>> {
    static CACHE: OnceLock<Mutex<QueryCache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    cache
        .entry(def.name)
        .or_insert_with(|| {
            let query = Query::new(&(def.language)(), &(def.query)()).ok()?;
            let kinds = query
                .capture_names()
                .iter()
                .map(|n| Highlight::from_capture(n))
                .collect();
            Some(Arc::new(Compiled { query, kinds }))
        })
        .clone()
}

/// Rope text for tree-sitter predicates (`#match?`, `#eq?`).
struct RopeText<'a>(&'a Rope);

impl<'a> TextProvider<&'a [u8]> for RopeText<'a> {
    type I = Box<dyn Iterator<Item = &'a [u8]> + 'a>;

    fn text(&mut self, node: Node) -> Self::I {
        let range = node.byte_range();
        let end = range.end.min(self.0.len_bytes());
        let start = range.start.min(end);
        Box::new(self.0.byte_slice(start..end).chunks().map(str::as_bytes))
    }
}

/// The parse tree of one document in a built-in language.
pub struct Syntax {
    name: &'static str,
    parser: Parser,
    tree: Option<Tree>,
    compiled: Arc<Compiled>,
    /// Edits were applied to the tree since it was last parsed.
    stale: bool,
}

impl Syntax {
    /// Highlighting for `path` (its first line for a `#!`), when the language
    /// is built in and its query compiles.
    pub fn for_file(path: &Path, rope: &Rope) -> Option<Syntax> {
        let first: String = rope.lines().next().map(String::from).unwrap_or_default();
        let def = lang_for(path, &first)?;
        let compiled = compiled(def)?;
        let mut parser = Parser::new();
        parser.set_language(&(def.language)()).ok()?;
        let mut syntax = Syntax {
            name: def.name,
            parser,
            tree: None,
            compiled,
            stale: true,
        };
        syntax.update(rope, &[]);
        Some(syntax)
    }

    /// The language's display name ("Rust", "TypeScript"…).
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Bring the tree up to date with `rope` after `edits` (from
    /// `Document::take_edits`): incremental when there is a tree.
    pub fn update(&mut self, rope: &Rope, edits: &[ByteEdit]) {
        if rope.len_bytes() > MAX_HIGHLIGHT_BYTES {
            self.tree = None;
            return;
        }
        if let Some(tree) = &mut self.tree {
            for e in edits {
                tree.edit(&InputEdit {
                    start_byte: e.start_byte,
                    old_end_byte: e.old_end_byte,
                    new_end_byte: e.new_end_byte,
                    start_position: Point::new(e.start.0, e.start.1),
                    old_end_position: Point::new(e.old_end.0, e.old_end.1),
                    new_end_position: Point::new(e.new_end.0, e.new_end.1),
                });
            }
            if edits.is_empty() && !self.stale {
                return;
            }
        }
        let len = rope.len_bytes();
        let mut read = |byte: usize, _: Point| -> &[u8] {
            if byte >= len {
                return &[];
            }
            let (chunk, start, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - start..]
        };
        self.tree = self
            .parser
            .parse_with_options(&mut read, self.tree.as_ref(), None);
        self.stale = false;
    }

    /// Highlighted byte ranges within `range`, sorted and not overlapping.
    /// Where captures nest, the innermost wins; for the same span, the
    /// query's earlier pattern wins (the order highlight queries assume).
    pub fn highlights(&self, rope: &Rope, range: Range<usize>) -> Vec<(Range<usize>, Highlight)> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let mut found: Vec<(usize, usize, usize, Highlight)> = Vec::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range.clone());
        let mut captures = cursor.captures(&self.compiled.query, tree.root_node(), RopeText(rope));
        while let Some((m, i)) = captures.next() {
            let capture = m.captures[*i];
            let Some(kind) = self.compiled.kinds[capture.index as usize] else {
                continue;
            };
            let r = capture.node.byte_range();
            let (start, end) = (r.start.max(range.start), r.end.min(range.end));
            if start < end {
                found.push((start, end, m.pattern_index, kind));
            }
        }
        // Paint wide spans first, then narrower ones over them; on equal
        // spans the earliest pattern paints last and so wins.
        found.sort_by(|a, b| {
            (a.0, std::cmp::Reverse(a.1), std::cmp::Reverse(a.2)).cmp(&(
                b.0,
                std::cmp::Reverse(b.1),
                std::cmp::Reverse(b.2),
            ))
        });
        let mut painted: Vec<Option<Highlight>> = vec![None; range.end - range.start];
        for (start, end, _, kind) in found {
            for slot in &mut painted[start - range.start..end - range.start] {
                *slot = Some(kind);
            }
        }
        let mut out: Vec<(Range<usize>, Highlight)> = Vec::new();
        for (i, slot) in painted.into_iter().enumerate() {
            let at = range.start + i;
            match (slot, out.last_mut()) {
                (Some(k), Some((r, last))) if *last == k && r.end == at => r.end = at + 1,
                (Some(k), _) => out.push((at..at + 1, k)),
                (None, _) => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, Selection};

    fn kinds_at(syntax: &Syntax, rope: &Rope, needle: &str) -> Vec<Highlight> {
        let text = rope.to_string();
        let at = text.find(needle).expect("needle in text");
        syntax
            .highlights(rope, 0..rope.len_bytes())
            .into_iter()
            .filter(|(r, _)| r.start <= at && at < r.end)
            .map(|(_, k)| k)
            .collect()
    }

    #[test]
    fn every_built_in_query_compiles() {
        for def in LANGS {
            assert!(compiled(def).is_some(), "{} query", def.name);
        }
    }

    #[test]
    fn languages_by_name_extension_and_shebang() {
        let name = |p: &str, first: &str| lang_for(Path::new(p), first).map(|d| d.name);
        assert_eq!(name("src/main.rs", ""), Some("Rust"));
        assert_eq!(name("App.TSX", ""), Some("TSX"));
        assert_eq!(name("Makefile", ""), Some("Makefile"));
        assert_eq!(name(".zshrc", ""), Some("Zsh"));
        assert_eq!(name(".env", ""), Some("Shell"));
        assert_eq!(name("Dockerfile", ""), Some("Dockerfile"));
        assert_eq!(name("CMakeLists.txt", ""), Some("CMake"));
        assert_eq!(name("main.tf", ""), Some("HCL"));
        assert_eq!(name("App.vue", ""), Some("Vue"));
        assert_eq!(name("Program.cs", ""), Some("C#"));
        assert_eq!(name("run", "#!/usr/bin/env zsh"), Some("Zsh"));
        assert_eq!(name("deploy", "#!/usr/bin/env bash"), Some("Shell"));
        assert_eq!(name("tool", "#!/usr/bin/python3.12"), Some("Python"));
        assert_eq!(name("notes.txt", ""), None);
    }

    #[test]
    fn extensions_and_file_names_are_not_claimed_twice() {
        let mut seen = std::collections::HashMap::new();
        for def in LANGS {
            for key in def.extensions.iter().chain(def.file_names) {
                if let Some(other) = seen.insert(key.to_ascii_lowercase(), def.name) {
                    assert_eq!(other, def.name, "{key} is claimed by two languages");
                }
            }
        }
        assert!(LANGS.len() >= 80, "{} built-in languages", LANGS.len());
    }

    #[test]
    fn a_few_of_the_added_languages_highlight() {
        for (path, text, needle, want) in [
            (
                "a.cs",
                "class A { int x = 1; }",
                "class",
                Highlight::Keyword,
            ),
            (
                "a.php",
                "<?php function f() { return 'x'; }",
                "'x'",
                Highlight::String,
            ),
            (
                "a.kt",
                "fun main() { val s = \"x\" }",
                "\"x\"",
                Highlight::String,
            ),
            (
                "q.sql",
                "SELECT id FROM users -- note",
                "-- note",
                Highlight::Comment,
            ),
            (
                "Dockerfile",
                "FROM alpine\nRUN echo hi",
                "FROM",
                Highlight::Keyword,
            ),
            (
                "main.tf",
                "resource \"a\" \"b\" { x = 1 }",
                "\"a\"",
                Highlight::String,
            ),
        ] {
            let rope = Rope::from_str(text);
            let syntax = Syntax::for_file(Path::new(path), &rope)
                .unwrap_or_else(|| panic!("{path}: no syntax"));
            let kinds = kinds_at(&syntax, &rope, needle);
            assert!(
                kinds.contains(&want),
                "{path}: {needle} is {kinds:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn rust_keywords_strings_comments_and_functions() {
        let rope = Rope::from_str("// note\nfn main() { let s = \"hi\"; helper(1); }\n");
        let syntax = Syntax::for_file(Path::new("a.rs"), &rope).unwrap();
        assert_eq!(syntax.name(), "Rust");
        assert_eq!(
            kinds_at(&syntax, &rope, "// note"),
            vec![Highlight::Comment]
        );
        assert_eq!(kinds_at(&syntax, &rope, "fn"), vec![Highlight::Keyword]);
        assert_eq!(kinds_at(&syntax, &rope, "\"hi\""), vec![Highlight::String]);
        assert_eq!(
            kinds_at(&syntax, &rope, "helper"),
            vec![Highlight::Function]
        );
        assert_eq!(kinds_at(&syntax, &rope, "1)"), vec![Highlight::Number]);
    }

    #[test]
    fn typescript_uses_the_javascript_base_query() {
        let rope = Rope::from_str("const x: number = await load('a');\n");
        let syntax = Syntax::for_file(Path::new("a.ts"), &rope).unwrap();
        assert_eq!(kinds_at(&syntax, &rope, "const"), vec![Highlight::Keyword]);
        assert_eq!(kinds_at(&syntax, &rope, "'a'"), vec![Highlight::String]);
        assert_eq!(kinds_at(&syntax, &rope, "number"), vec![Highlight::Type]);
    }

    #[test]
    fn incremental_edits_keep_the_tree_in_step() {
        let mut doc = Document::from_text("fn a() {}\n");
        let mut syntax = Syntax::for_file(Path::new("a.rs"), doc.rope()).unwrap();
        doc.set_selection(Selection::cursor(doc.rope().len_chars()));
        doc.type_text("// later");
        let edits = doc.take_edits();
        syntax.update(doc.rope(), &edits);
        assert_eq!(
            kinds_at(&syntax, doc.rope(), "// later"),
            vec![Highlight::Comment]
        );
        doc.undo();
        let edits = doc.take_edits();
        syntax.update(doc.rope(), &edits);
        let all = syntax.highlights(doc.rope(), 0..doc.rope().len_bytes());
        assert!(all.iter().all(|(r, _)| r.end <= doc.rope().len_bytes()));
        assert!(
            !all.iter().any(|(_, k)| *k == Highlight::Comment),
            "the comment is gone"
        );
    }

    #[test]
    fn ranges_limit_the_work_and_the_result() {
        let text = "fn a() {}\n".repeat(1000);
        let rope = Rope::from_str(&text);
        let syntax = Syntax::for_file(Path::new("a.rs"), &rope).unwrap();
        let hl = syntax.highlights(&rope, 500..520);
        assert!(!hl.is_empty());
        assert!(hl.iter().all(|(r, _)| r.start >= 500 && r.end <= 520));
    }
}
