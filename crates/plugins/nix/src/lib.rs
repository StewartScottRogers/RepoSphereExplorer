//! Nix expression file type plugin: core and presentation halves.
//!
//! Two shapes under one language. A flake pins its inputs by name and
//! address, which is what makes a build reproducible; the older shape
//! is a function taking an attribute set with defaults, and takes
//! whatever the machine's channel happens to hold. Which of the two a
//! file is decides everything a reader wants to ask about it, so it is
//! the first thing reported.
//!
//! A derivation's inputs are read apart from each other because Nix
//! keeps them apart: what is needed to *build* something is not what is
//! needed to run it, and the distinction is the reason a Nix closure is
//! smaller than a container image.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["nix"];

/// How much of an expression is read.
const READ_CAP: usize = 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 48;

/// One input a flake pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// The name the flake refers to it by.
    pub name: String,
    /// Where it comes from.
    pub url: Option<String>,
    /// What it is told to reuse rather than pin twice - a `follows`,
    /// which is how a flake avoids two copies of the same dependency.
    pub follows: Vec<String>,
}

/// One derivation the expression builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derivation {
    /// Its package name.
    pub name: Option<String>,
    /// Its version.
    pub version: Option<String>,
    /// What is needed on the machine doing the building.
    pub native_build_inputs: Vec<String>,
    /// What is needed by the thing being built.
    pub build_inputs: Vec<String>,
    /// What is needed only to run its checks.
    pub check_inputs: Vec<String>,
    /// The phases it overrides.
    pub phases: Vec<String>,
}

/// View data produced by [`NixCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NixView {
    /// Whether this is a flake.
    pub flake: bool,
    /// What the flake says it is, from its `description`.
    pub description: Option<String>,
    /// The inputs a flake pins.
    pub inputs: Vec<Input>,
    /// The outputs a flake produces, by attribute path.
    pub outputs: Vec<String>,
    /// The arguments the expression takes, for the older shape: an
    /// attribute set, usually with defaults.
    pub arguments: Vec<String>,
    /// The derivation it builds, when it builds one.
    pub derivation: Option<Derivation>,
    /// Whether it brings a scope into view with `with`, which makes
    /// every name in that scope available unqualified.
    pub with_scopes: Vec<String>,
    /// Whether the expression was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a Nix expression.
fn looks_like_it(text: &str) -> bool {
    let mut markers = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        for marker in [
            "mkDerivation",
            "import <nixpkgs>",
            "buildInputs",
            "nativeBuildInputs",
            "stdenv.",
            "pkgs.",
            "lib.",
            "nixpkgs.url",
            "inherit ",
        ] {
            if trimmed.contains(marker) {
                markers += 1;
            }
        }
        if trimmed.starts_with("with ") && trimmed.ends_with(';') {
            markers += 1;
        }
        // A flake is unmistakable: `outputs = { ... }:` beside `inputs`.
        if trimmed.starts_with("outputs = {") || trimmed.starts_with("description = ") {
            markers += 1;
        }
        if markers >= 3 {
            return true;
        }
    }
    false
}

/// How deeply indented a line is.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The text inside the first pair of double quotes.
fn quoted(text: &str) -> Option<&str> {
    let start = text.find('"')? + 1;
    let rest = &text[start..];
    Some(&rest[..rest.find('"')?])
}

/// The names in a `[ a b c ]` list, whether on one line or many.
///
/// Nix separates list items with whitespace rather than commas, and an
/// item may be an expression - `pkgs.openssl`, or a whole
/// `lib.optional x y` call. Only the plain names are taken; a call is
/// reported as written.
fn list_items(lines: &[&str], from: usize) -> Vec<String> {
    let mut found = Vec::new();
    let mut depth = 0usize;
    for (at, line) in lines.iter().enumerate().skip(from) {
        // The first line still carries `name = ` in front of the
        // bracket, and those words are not items.
        let trimmed = if at == from {
            match line.find('[') {
                Some(open) => &line[open..],
                None => continue,
            }
        } else {
            line.trim()
        };
        // The list ends at its closing bracket; whatever follows on
        // that line - `++ lib.optional withDocs pandoc;` - is another
        // expression and not an item of it.
        let trimmed = match trimmed.find(']') {
            Some(close) => &trimmed[..=close],
            None => trimmed,
        };
        let trimmed = trimmed.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        depth += trimmed.matches('[').count();
        for piece in trimmed
            .trim_start_matches(|character: char| character == '[' || character.is_whitespace())
            .trim_end_matches(|character: char| {
                character == ']' || character == ';' || character.is_whitespace()
            })
            .split_whitespace()
        {
            let name = piece.trim();
            if !name.is_empty() && name != "=" && !name.starts_with("++") {
                found.push(name.to_owned());
            }
        }
        depth = depth.saturating_sub(trimmed.matches(']').count());
        if depth == 0 {
            break;
        }
    }
    found
}

/// The inputs a flake pins, read from its `inputs` block.
fn inputs_in(lines: &[&str]) -> Vec<Input> {
    let Some(start) = lines
        .iter()
        .position(|line| line.trim_start().starts_with("inputs = {"))
    else {
        return Vec::new();
    };
    let outer = indent_of(lines[start]);
    let mut found: Vec<Input> = Vec::new();
    let mut nested: Option<String> = None;
    for line in lines.iter().skip(start + 1) {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed == "};" && indent_of(line) <= outer {
            break;
        }
        if let Some(name) = nested.clone() {
            if let Some(rest) = trimmed.strip_prefix("inputs.")
                && let Some((what, _)) = rest.split_once(".follows")
                && let Some(entry) = found.iter_mut().find(|entry| entry.name == name)
            {
                entry.follows.push(what.to_owned());
            }
            if let Some(rest) = trimmed.strip_prefix("url = ")
                && let Some(entry) = found.iter_mut().find(|entry| entry.name == name)
            {
                entry.url = quoted(rest).map(ToOwned::to_owned);
            }
            if trimmed.starts_with('}') {
                nested = None;
            }
            continue;
        }
        // `name.url = "...";` on one line, or `name = {` opening a block.
        if let Some((name, rest)) = trimmed.split_once(".url") {
            found.push(Input {
                name: name.trim().to_owned(),
                url: quoted(rest).map(ToOwned::to_owned),
                follows: Vec::new(),
            });
        } else if let Some((name, rest)) = trimmed.split_once('=')
            && rest.trim().starts_with('{')
        {
            let name = name.trim().to_owned();
            found.push(Input {
                name: name.clone(),
                url: None,
                follows: Vec::new(),
            });
            nested = Some(name);
        }
        if found.len() >= SHOWN {
            break;
        }
    }
    found
}

/// The derivation the expression builds, if it builds one.
fn derivation_in(lines: &[&str]) -> Option<Derivation> {
    let start = lines
        .iter()
        .position(|line| line.contains("mkDerivation"))?;
    let mut found = Derivation {
        name: None,
        version: None,
        native_build_inputs: Vec::new(),
        build_inputs: Vec::new(),
        check_inputs: Vec::new(),
        phases: Vec::new(),
    };
    for (at, line) in lines.iter().enumerate().skip(start) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pname = ") {
            found.name = quoted(rest).map(ToOwned::to_owned);
        } else if let Some(rest) = trimmed.strip_prefix("name = ") {
            found.name = found
                .name
                .take()
                .or_else(|| quoted(rest).map(ToOwned::to_owned));
        } else if let Some(rest) = trimmed.strip_prefix("version = ") {
            found.version = quoted(rest).map(ToOwned::to_owned);
        } else if trimmed.starts_with("nativeBuildInputs") {
            found.native_build_inputs = list_items(lines, at);
        } else if trimmed.starts_with("buildInputs") {
            found.build_inputs = list_items(lines, at);
        } else if trimmed.starts_with("checkInputs") || trimmed.starts_with("nativeCheckInputs") {
            found.check_inputs = list_items(lines, at);
        } else if let Some((name, _)) = trimmed.split_once(" = ''")
            && name.ends_with("Phase")
        {
            found.phases.push(name.trim().to_owned());
        }
    }
    Some(found)
}

/// Everything [`NixView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> NixView {
    let lines: Vec<&str> = source.lines().collect();
    let flake = lines
        .iter()
        .any(|line| line.trim_start().starts_with("outputs = {"))
        && lines
            .iter()
            .any(|line| line.trim_start().starts_with("inputs = {"));

    let mut view = NixView {
        flake,
        description: lines
            .iter()
            .find_map(|line| line.trim().strip_prefix("description = "))
            .and_then(quoted)
            .map(ToOwned::to_owned),
        inputs: if flake { inputs_in(&lines) } else { Vec::new() },
        outputs: Vec::new(),
        arguments: Vec::new(),
        derivation: derivation_in(&lines),
        with_scopes: Vec::new(),
        truncated,
    };

    for line in &lines {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("with ")
            && let Some(scope) = rest.split(';').next()
        {
            let scope = scope.trim();
            if !scope.is_empty() && !view.with_scopes.iter().any(|had| had == scope) {
                view.with_scopes.push(scope.to_owned());
            }
        }
        // A flake's outputs are attribute paths assigned inside the
        // `outputs` function: `packages.default = ...`.
        if flake
            && let Some((path, _)) = trimmed.split_once(" = ")
            && path.contains('.')
            && !path.contains(' ')
            && [
                "packages",
                "devShells",
                "checks",
                "apps",
                "formatter",
                "overlays",
                "nixosModules",
            ]
            .iter()
            .any(|kind| path.starts_with(kind))
            && !view.outputs.iter().any(|had| had == path)
            && view.outputs.len() < SHOWN
        {
            view.outputs.push(path.to_owned());
        }
    }
    view.arguments = arguments_in(&lines);
    view
}

/// The arguments the expression takes, for the older shape.
///
/// A Nix file that is a function opens with its parameter set, and each
/// parameter may carry a default after a `?`.
fn arguments_in(lines: &[&str]) -> Vec<String> {
    let Some(open) = lines
        .iter()
        .position(|line| line.trim_start().starts_with('{'))
    else {
        return Vec::new();
    };
    // Only a set that closes with `}:` is a parameter list; a set that
    // closes with a plain `}` is a value.
    let mut found = Vec::new();
    for line in lines.iter().skip(open) {
        let trimmed = line.trim();
        if trimmed.starts_with("}:") {
            return found;
        }
        if trimmed.starts_with('}') {
            return Vec::new();
        }
        for piece in trimmed.trim_start_matches(['{', ',']).split(',') {
            let name = piece.trim().trim_start_matches('{').trim();
            if !name.is_empty() && !name.starts_with('#') && found.len() < SHOWN {
                found.push(name.to_owned());
            }
        }
    }
    Vec::new()
}

/// Everything [`NixView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<NixView> {
    let source = std::fs::read_to_string(path)?;
    let truncated = source.len() > READ_CAP;
    let source = if truncated {
        let mut end = READ_CAP;
        while end > 0 && !source.is_char_boundary(end) {
            end -= 1;
        }
        &source[..end]
    } else {
        source.as_str()
    };
    if !looks_like_it(source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Nix expression",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Nix expression plugin's core half.
#[derive(Debug, Default)]
pub struct NixCore;

impl PluginCore for NixCore {
    fn name(&self) -> &'static str {
        "nix"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Nix expression plugin's presentation half.
#[derive(Debug, Default)]
pub struct NixPresentation;

impl PluginPresentation for NixPresentation {
    fn name(&self) -> &'static str {
        "nix"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NIX",
            tint: 0x0052_77c3,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NixView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![if view.flake {
            "Nix flake: its inputs are pinned, so the build is reproducible.".to_owned()
        } else {
            "Nix expression: not a flake, so what it builds against is".to_owned()
        }];
        if !view.flake {
            lines.push("whatever the machine's channel holds.".to_owned());
        }
        if let Some(description) = &view.description {
            lines.push(description.clone());
        }
        if !view.inputs.is_empty() {
            lines.push("Pins:".to_owned());
            for input in &view.inputs {
                lines.push(format!(
                    "  {} {}",
                    input.name,
                    input.url.as_deref().unwrap_or("(no url)")
                ));
                for follows in &input.follows {
                    lines.push(format!("      reusing this flake's {follows}"));
                }
            }
        }
        if !view.outputs.is_empty() {
            lines.push(format!("Produces {}", view.outputs.join(", ")));
        }
        if !view.arguments.is_empty() {
            lines.push("Takes:".to_owned());
            for argument in &view.arguments {
                lines.push(format!("  {argument}"));
            }
        }
        if !view.with_scopes.is_empty() {
            lines.push(format!(
                "Brings {} into scope unqualified",
                view.with_scopes.join(", ")
            ));
        }
        match &view.derivation {
            Some(derivation) => {
                lines.push(format!(
                    "Builds {} {}",
                    derivation.name.as_deref().unwrap_or("(unnamed)"),
                    derivation.version.as_deref().unwrap_or("")
                ));
                for (heading, inputs) in [
                    ("  needed to build it", &derivation.native_build_inputs),
                    ("  needed by it", &derivation.build_inputs),
                    ("  needed to check it", &derivation.check_inputs),
                ] {
                    if !inputs.is_empty() {
                        lines.push(format!("{heading}: {}", inputs.join(", ")));
                    }
                }
                if !derivation.phases.is_empty() {
                    lines.push(format!("  overrides {}", derivation.phases.join(", ")));
                }
            }
            None => lines.push("Builds nothing itself.".to_owned()),
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{NixCore, NixPresentation, NixView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/nix")
            .join(name)
    }

    fn view_of(name: &str) -> NixView {
        serde_json::from_value(NixCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&NixCore),
            PluginPresentation::extensions(&NixPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn recognises_a_nix_expression() {
        assert!(looks_like_it(
            "{ pkgs ? import <nixpkgs> {} }:\nwith pkgs;\nstdenv.mkDerivation { }"
        ));
        assert!(!looks_like_it("{ \"a\": 1, \"b\": 2 }"), "that is JSON");
        assert!(!looks_like_it(""));
    }

    #[test]
    fn tells_a_flake_from_the_older_shape() {
        assert!(view_of("flake.nix").flake);
        assert!(!view_of("default.nix").flake);
    }

    #[test]
    fn reads_what_a_flake_pins() {
        let view = view_of("flake.nix");

        assert_eq!(
            view.description.as_deref(),
            Some("csvstats: summary statistics for a column of readings")
        );
        let named: Vec<&str> = view.inputs.iter().map(|one| one.name.as_str()).collect();
        assert_eq!(named, vec!["nixpkgs", "flake-utils", "rust-overlay"]);
        assert_eq!(
            view.inputs[0].url.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-24.05")
        );
        assert_eq!(
            view.inputs[2].url.as_deref(),
            Some("github:oxalica/rust-overlay"),
            "an input declared as a block still has a url"
        );
        assert_eq!(
            view.inputs[2].follows,
            vec!["nixpkgs"],
            "and says which of this flake's inputs it reuses"
        );
    }

    #[test]
    fn reads_the_outputs_a_flake_produces() {
        let view = view_of("flake.nix");

        assert!(view.outputs.iter().any(|one| one == "packages.default"));
        assert!(view.outputs.iter().any(|one| one == "devShells.default"));
        assert!(view.outputs.iter().any(|one| one == "checks.format"));
    }

    #[test]
    fn reads_the_arguments_the_older_shape_takes() {
        let view = view_of("default.nix");

        assert_eq!(
            view.arguments,
            vec![
                "pkgs ? import <nixpkgs> { }",
                "lib ? pkgs.lib",
                "stdenv ? pkgs.stdenv",
                "withDocs ? true",
            ]
        );
        assert_eq!(view.with_scopes, vec!["pkgs"]);
    }

    #[test]
    fn keeps_the_three_kinds_of_input_apart() {
        let derivation = view_of("default.nix").derivation.expect("it builds one");

        assert_eq!(derivation.name.as_deref(), Some("csvstats"));
        assert!(
            derivation.native_build_inputs.contains(&"cargo".to_owned()),
            "{:?}",
            derivation.native_build_inputs
        );
        assert!(derivation.build_inputs.contains(&"openssl".to_owned()));
        assert!(
            derivation
                .check_inputs
                .contains(&"cargo-nextest".to_owned())
        );
        assert!(
            !derivation.build_inputs.contains(&"cargo".to_owned()),
            "what builds it is not what it needs to run"
        );
    }

    #[test]
    fn a_list_ends_at_its_bracket_and_not_at_the_end_of_the_line() {
        // Found in the running application: the build inputs read
        // `cargo, rustc, pkg-config, ], lib.optional, withDocs, pandoc`
        // because the closing line carries `++ lib.optional ...` after
        // the bracket.
        let derivation = view_of("default.nix").derivation.expect("it builds one");

        assert_eq!(
            derivation.native_build_inputs,
            vec!["cargo", "rustc", "pkg-config"]
        );
    }

    #[test]
    fn reads_the_phases_it_overrides() {
        let derivation = view_of("default.nix").derivation.expect("it builds one");

        assert_eq!(
            derivation.phases,
            vec!["buildPhase", "checkPhase", "installPhase"]
        );
    }

    #[test]
    fn a_flake_builds_nothing_itself() {
        let view = view_of("flake.nix");

        assert!(
            view.derivation.is_none(),
            "it calls out to default.nix rather than describing a build"
        );
    }

    #[test]
    fn presents_the_two_shapes_differently() {
        let flake = NixPresentation.present(&NixCore.view(&sample("flake.nix")).unwrap());
        let plain = NixPresentation.present(&NixCore.view(&sample("default.nix")).unwrap());

        assert!(flake[0].starts_with("Nix flake"));
        assert!(
            flake
                .iter()
                .any(|line| line.contains("github:NixOS/nixpkgs"))
        );
        assert!(
            flake
                .iter()
                .any(|line| line.contains("reusing this flake's nixpkgs"))
        );

        assert!(plain[0].starts_with("Nix expression: not a flake"));
        assert!(
            plain
                .iter()
                .any(|line| line.contains("needed to build it: cargo"))
        );
        assert!(plain.iter().any(|line| line.contains("Builds csvstats")));
    }

    #[test]
    fn a_file_that_is_not_nix_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.nix");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(NixCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
