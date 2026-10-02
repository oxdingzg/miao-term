# Third-party syntax definitions

mtty highlights code with two engines (ADR 0034, phase E3):

- **tree-sitter grammars** — crates.io dependencies of `miao-term-editor`; each
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
