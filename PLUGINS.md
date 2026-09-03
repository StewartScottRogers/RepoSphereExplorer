# Plugin registry

Tracks every file-type plugin this project has built or rejected, per
[GUIDANCE.md §3.1](GUIDANCE.md#31-plugin-architecture) and
[§7 D5](GUIDANCE.md#7-open-decisions). Consulted before proposing or
building any plugin work order, so the factory never duplicates a plugin
that already exists or retries a format already ruled out.

## Built

| Format | Crate | Issue | Notes |
| --- | --- | --- | --- |
| Plain text | `crates/plugins/text` | — | First plugin, built with the architecture itself |
| Image (raster codecs) | `crates/plugins/image` | — | |
| Archive (zip) | `crates/plugins/archive` | — | Also provides `extract` for the D4 operations |
| PDF | `crates/plugins/pdf` | — | |
| Directory-as-file | `crates/plugins/directory` | — | |
| Python source | `crates/plugins/python` | #7 | Sniffs by shebang or top-level `def`/`class`, ahead of `text` in `CORE_PLUGINS` |
| JavaScript source | `crates/plugins/javascript` | #8 | Sniffs by `node` shebang, top-level `function`/`class`, or CommonJS/ES-module/arrow-function markers, ahead of `text` in `CORE_PLUGINS` |
| TypeScript source | `crates/plugins/typescript` | #9 | Sniffs by TypeScript-only markers (`interface`/`enum`/type-alias declarations, type annotations, visibility modifiers, `implements`, `import type`), ahead of `javascript` in `CORE_PLUGINS` so it claims TS-specific syntax first |
| Rust source | `crates/plugins/rust` | #10 | Sniffs by `fn`/`struct`/`enum`/`trait`/`impl` declarations and markers (`let mut`, `println!(`, `#[derive(`, `use std::`) not used by this project's other source-language plugins; placed just ahead of `text` in `CORE_PLUGINS` |
| Go source | `crates/plugins/go` | #11 | Sniffs by `package`/`func`/`import (` declarations and markers (`:=`, `fmt.Println(`, `fmt.Printf(`) not used by this project's other source-language plugins; placed just ahead of `text` in `CORE_PLUGINS` |
| C source | `crates/plugins/c` | #12 | Sniffs by `#include <...>`/`#include "..."` directives and markers (`int main(`, `void main(`, `printf(`, `malloc(`, `NULL`) not used by this project's other source-language plugins; placed just ahead of `text` in `CORE_PLUGINS`. No path/extension-based dispatch exists in this architecture (sniffing is content-only), so disambiguating `.c`/`.h` from a future C++ plugin will need that plugin's sniff to avoid these same markers, or to be ordered after `c` |
| C++ source | `crates/plugins/cpp` | #13 | Sniffs by C++-only markers (`#include <iostream>`/`<vector>`/`<string>`, `class `, `namespace `, `std::`, `cout <<`, `cin >>`, `nullptr`, `public:`/`private:`/`protected:`, `template<`) that avoid the C plugin's markers per its note, so a C++ file that also contains C-style constructs (`int main(`, `printf(`) is still claimed correctly; placed just ahead of `c` in `CORE_PLUGINS` |
| C# source | `crates/plugins/csharp` | #14 | Sniffs by C#-only markers (`using System`, `Console.WriteLine(`/`Console.Write(`, `public class `/`internal class `, `public static void Main(`/`static void Main(`, `{ get; set; }`) that avoid the C++ plugin's overlapping `class `/`namespace ` markers, so a C# file whose `namespace` block also matches C++'s bare check is still claimed correctly; placed just ahead of `cpp` in `CORE_PLUGINS` |
| Java source | `crates/plugins/java` | #15 | Sniffs by Java-only markers (`import java.`, `System.out.println(`/`System.out.print(`/`System.err.println(`, `public static void main(String`, `@Override`) that avoid the C#'s bare `public class ` marker and the Go plugin's bare `package ` marker, so a Java file is still claimed correctly rather than by either; placed just ahead of `csharp` in `CORE_PLUGINS` |
| Kotlin source | `crates/plugins/kotlin` | #16 | Sniffs by Kotlin-only markers (`import kotlin.`, a bare `println(` at the start of a line, `fun main(`, `data class `, `companion object`) that avoid other plugins' overlapping substrings (Java's `System.out.println(` and Go's `fmt.Println(` both contain `println(` but never at the start of a line); placed just ahead of `csharp` in `CORE_PLUGINS` |
| Ruby source | `crates/plugins/ruby` | #18 | Sniffs by Ruby-only markers (a bare `end` line, `require '`/`require "`, `attr_accessor`/`attr_reader`/`attr_writer`, a bare `puts` call, `module `, a `do \|...\|` block, or a `ruby` shebang) that avoid the Python plugin's bare `def `/`class ` check, which Ruby's own `def`/`class` lines would also match; placed just ahead of `python` in `CORE_PLUGINS` so it claims Ruby files first |
| PHP source | `crates/plugins/php` | #19 | Sniffs by the `<?php`/`<?=` opening tag, the one marker unique to PHP source; ordinary PHP code also produces bare top-level `function `/`class ` lines and a bare `require '...'` line that would otherwise be claimed first by the JavaScript/Python and Ruby plugins' overlapping markers, so it is placed first in `CORE_PLUGINS`, ahead of all other source-language plugins |
| Perl source | `crates/plugins/perl` | #20 | Sniffs by a `perl` shebang, `use strict`/`use warnings` lines, a `package Name;` declaration (semicolon-terminated, distinct from the Go plugin's bare `package main`), a top-level `sub name {` declaration, a lexical `my $` variable, or the `=~` regex-binding operator — markers not used by any sibling plugin; placed ahead of `php` in `CORE_PLUGINS`. Perl and Prolog both commonly use the `.pl` extension, but this project sniffs by content only, so a future Prolog plugin must avoid these same markers or be ordered after `perl` |
| Swift source | `crates/plugins/swift` | #17 | Sniffs by Swift-only markers (`import Foundation`/`import UIKit`/`import SwiftUI`, `protocol `/`extension ` declarations, `guard let `/`guard var `, `@IBOutlet`, `@escaping`, and `func` declarations with an arrow return type `) -> `) that don't overlap this project's other source-language plugins, in particular avoiding the Go plugin's bare `func ` marker by requiring the arrow; placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint against a specific sibling since it has no overlapping markers |

## Rejected / infeasible

_(none yet)_

## Maintaining this file

- **Built**: when a plugin's PR merges, add a row here with its format,
  crate path, and issue number, in the same PR or a prompt follow-up.
- **Rejected**: when a work order concludes a format can't reasonably
  become a plugin, add a row here with the format, its issue number, and
  a one-line reason, then close the issue without a PR. Do not reopen or
  retry a rejected format's work order unless explicitly asked to.
- Before proposing or starting any plugin work order, check both
  sections here, plus `gh issue list --label work-order --state all`
  for one already filed on the same format.
