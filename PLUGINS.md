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
| Shell script | `crates/plugins/shell` | #22 | Sniffs by a shebang naming `sh`/`bash`/`zsh`/`dash`/`ksh` (directly or via `env`), or POSIX shell keywords not used by any sibling plugin (a bare `fi`/`done`/`esac`/`then` line, `elif `, a `case ... in` opener, a line ending `; then`) or `$(` command substitution; deliberately does not sniff `${...}` parameter expansion since JavaScript/TypeScript template literals use the same syntax; placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint against a specific sibling since it has no overlapping markers |
| PowerShell source | `crates/plugins/powershell` | #23 | Sniffs by a `pwsh`/`powershell` shebang (directly or via `env`), a `<#` block/help comment opener, `[CmdletBinding()]`, a `param(`/`param (` block, or the `$PSScriptRoot`/`$PSVersionTable`/`$ErrorActionPreference` automatic/preference variables — markers not used by any sibling plugin; placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint against a specific sibling since it has no overlapping markers |
| R source | `crates/plugins/r` | #25 | Sniffs by an `Rscript` shebang, the `<-` assignment operator, the `%>%` pipe, or a top-level `library(` call — markers not used by any sibling plugin; placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint against a specific sibling since it has no overlapping markers. A future Haskell, OCaml, or F# plugin sniffing `<-` for a monadic bind or list-comprehension generator must avoid this same marker, or be ordered after `r` |
| Haskell source | `crates/plugins/haskell` | #26 | Sniffs by a `runghc`/`runhaskell` shebang, a `{-# LANGUAGE` pragma, an `import qualified ` statement, or a spaced ` :: ` type signature — markers not used by any sibling plugin; deliberately does not sniff the `<-` operator per the R plugin's note above, since Haskell also uses `<-` for monadic bind; placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint against a specific sibling since it has no overlapping markers |
| Scala source | `crates/plugins/scala` | #27 | Sniffs by a `scala` shebang, an `import scala.` statement, a `case class ` declaration, `extends App`, a `def main(args: Array[String]` signature, or a compound `sealed trait ` marker — markers not used by any sibling plugin; deliberately checks `sealed trait ` rather than a bare `trait ` line start, since the Rust plugin already claims that; placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint against a specific sibling since it has no overlapping markers |
| SQL source | `crates/plugins/sql` | #24 | Sniffs case-insensitively by a statement-introducing keyword at the start of a line (`SELECT`, `INSERT INTO`, `UPDATE`, `DELETE FROM`, `CREATE TABLE`/`INDEX`/`VIEW`, `ALTER TABLE`, `DROP TABLE`) or a `PRIMARY KEY`/`FOREIGN KEY` constraint anywhere — markers not used by any sibling plugin; placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint against a specific sibling since it has no overlapping markers |
| Elixir source | `crates/plugins/elixir` | #28 | Sniffs by an `elixir` shebang, a `defmodule ` declaration, `IO.puts`/`IO.inspect`, the `@moduledoc` attribute, or the `\|>` pipe operator — markers not used by any sibling plugin; deliberately does not sniff bare `end` lines, since the Ruby plugin already claims those, so this plugin is placed just ahead of `ruby` in `CORE_PLUGINS` to claim genuine Elixir files (which also close blocks with bare `end`) via one of its own stronger markers first |

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
