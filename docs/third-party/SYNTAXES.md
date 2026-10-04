# Third-party syntax definitions

mtty highlights code with two engines (ADR 0034, phase E3):

- **tree-sitter grammars** — crates.io dependencies of `mtty-editor`; each
  crate carries its own licence (MIT, Apache-2.0 or CC0-1.0, checked against
  ADR 0006).
- **Sublime syntaxes through syntect** — for languages no built-in grammar
  covers. Below are the licences of the syntax definitions compiled into mtty.

## syntect's default syntax set

The `default-syntaxes` dump of [syntect](https://github.com/trishume/syntect)
(MIT) is built from the [Sublime Text Packages](https://github.com/sublimehq/Packages)
repository, which states:

```text
If not otherwise specified (see below), files in this repository fall under the following license:

    Permission to copy, use, modify, sell and distribute this
    software is granted. This software is provided "as is" without
    express or implied warranty, and with no claim as to its
    suitability for any purpose.

An exception is made for files in readable text which contain their own license information, or files where an accompanying file exists (in the same directory) with a "-license" suffix added to the base-name name of the original file, and an extension of txt, html, or similar. For example "tidy" is accompanied by "tidy-license.txt".
```

## Vendored from bat

`vendor/syntaxes` holds Sublime syntaxes taken from
[bat](https://github.com/sharkdp/bat) v0.26.1 (`assets/syntaxes/02_Extra`, the
set two-face packages), keeping only those whose own licence is permissive
under ADR 0006. Each directory carries the upstream licence file and a
`SOURCE.md` naming the upstream repository and commit. Syntaxes bat converted
from TextMate grammars keep the upstream grammar's licence.

`crates/mtty-editor/build.rs` compiles them with syntect's defaults into one
embedded dump; every vendored file must compile under the pure-Rust regex
engine. Left out: bat's VimHelp and hosts (their patterns need Oniguruma), and
syntaxes for languages a built-in tree-sitter grammar already covers.

| Syntax | Licence | Upstream |
| --- | --- | --- |
| Apache | BSD-2-Clause | https://github.com/colinta/ApacheConf.tmLanguage |
| AWK | MIT | https://github.com/JohnNilsson/awk-sublime |
| bat | MIT OR Apache-2.0 | https://github.com/sharkdp/bat |
| CFML | MIT | https://github.com/jcberquist/sublimetext-cfml.git |
| cmd-help | MIT | https://github.com/victor-gp/cmd-help-sublime-syntax.git |
| CoffeeScript | MIT | https://github.com/sustained/CoffeeScript-Sublime-Plugin |
| Crontab | MIT | https://github.com/michaelblyons/SublimeSyntax-Crontab |
| Crystal | Apache-2.0 | https://github.com/crystal-lang-tools/sublime-crystal.git |
| Email | MIT | https://github.com/mariozaizar/email.sublime-syntax.git |
| GDScript | MIT | https://github.com/beefsack/GDScript-sublime |
| gnuplot | MIT | https://github.com/hesstobi/sublime_gnuplot |
| Groff | MIT | https://github.com/carsonoid/sublime_man_page_support |
| HTTP | MIT | https://github.com/keith-hall/http-request-response-syntax.git |
| Idris2 | Apache-2.0 | https://github.com/buzden/sublime-syntax-idris2 |
| Jsonnet | Apache-2.0 | https://github.com/gburiola/sublime-jsonnet-syntax.git |
| Julia | MIT | https://github.com/JuliaEditorSupport/Julia-sublime |
| Lean | Apache-2.0 | https://github.com/leanprover/vscode-lean4.git |
| LLVM | MIT | https://github.com/ioncodes/LLVM.tmBundle |
| MediaWiki | MIT | https://github.com/tosher/Mediawiker.git |
| Nginx | MIT | https://github.com/SublimeText/nginx |
| Ninja | MIT | https://github.com/pope/SublimeNinja.git |
| NSIS | Apache-2.0 | https://github.com/SublimeText/NSIS |
| Odin | MIT | https://github.com/odin-lang/sublime-odin |
| Org | BSD-2-Clause | https://github.com/jezcope/Org.tmbundle.git |
| Puppet | MIT | https://github.com/russCloak/SublimePuppet |
| QML | MIT | https://github.com/skozlovf/Sublime-QML |
| Robot | MIT | https://github.com/andriyko/sublime-robot-framework-assistant.git |
| Slim | MIT | https://github.com/slim-template/ruby-slim.tmbundle.git |
| Stylus | MIT | https://github.com/billymoon/Stylus |
| SystemVerilog | Apache-2.0 | https://github.com/TheClams/SystemVerilog.git |
| Twig | BSD-3-Clause | https://github.com/Anomareh/PHP-Twig.tmbundle.git |
| varlink | MIT | https://github.com/varlink/syntax-highlight-varlink.git |
| VHDL | Apache-2.0 | https://github.com/TheClams/SmartVHDL |
| WGSL | MIT | https://github.com/PolyMeilex/vscode-wgsl.git |
