//! Executable/object file type plugin: core and presentation halves.
//!
//! Covers `.exe`/`.dll` (PE/COFF), `.so` (ELF), and `.dylib` (Mach-O) as one
//! plugin, matching how `font` uses one crate pairing to cover multiple
//! container formats: `sniff` recognises each container's own magic bytes
//! directly, while `view` hands the whole file to the `object` crate's
//! unified reader rather than writing a parser per format. The view is
//! headers, sections, and symbols, not a raw hex dump, per the issue.

use object::{Object, ObjectSection, ObjectSymbol};
use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of sections/symbols listed in the view; binaries with
/// more are truncated, matching the `archive` plugin's own `MAX_ENTRIES`.
const MAX_ENTRIES: usize = 200;

/// Detects one of this plugin's recognised container formats from a file's
/// leading bytes: ELF, PE/COFF (`MZ` DOS stub), or Mach-O (thin 32/64-bit or
/// universal/fat, in either byte order).
fn recognised(prefix: &[u8]) -> bool {
    prefix.starts_with(b"\x7fELF")
        || prefix.starts_with(b"MZ")
        || prefix.starts_with(&[0xFE, 0xED, 0xFA, 0xCE])
        || prefix.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
        || prefix.starts_with(&[0xFE, 0xED, 0xFA, 0xCF])
        || prefix.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
        || prefix.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE])
        || prefix.starts_with(&[0xBE, 0xBA, 0xFE, 0xCA])
}

/// One section in an object file, from [`ExecutableView::sections`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionInfo {
    /// The section's name, e.g. `.text`, `.data`.
    pub name: String,
    /// The section's size in bytes.
    pub size: u64,
}

/// View data produced by [`ExecutableCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutableView {
    /// The container format, e.g. `"Elf"`, `"Pe"`, `"MachO"`.
    pub format: String,
    /// The target instruction set architecture, e.g. `"X86_64"`, `"Aarch64"`.
    pub architecture: String,
    /// Whether this is a 64-bit (rather than 32-bit) object.
    pub is_64_bit: bool,
    /// The entry point's virtual address.
    pub entry: u64,
    /// Total number of sections.
    pub section_count: usize,
    /// The first [`MAX_ENTRIES`] sections.
    pub sections: Vec<SectionInfo>,
    /// Total number of named symbols.
    pub symbol_count: usize,
    /// The first [`MAX_ENTRIES`] symbol names.
    pub symbols: Vec<String>,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// The executable/object plugin's core half.
#[derive(Debug, Default)]
pub struct ExecutableCore;

impl PluginCore for ExecutableCore {
    fn name(&self) -> &'static str {
        "executable"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        recognised(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let file_size = std::fs::metadata(path)?.len();
        let data = std::fs::read(path)?;
        let obj = object::File::parse(&*data)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

        let mut sections = Vec::new();
        let mut section_count = 0usize;
        for section in obj.sections() {
            section_count += 1;
            if sections.len() < MAX_ENTRIES {
                sections.push(SectionInfo {
                    name: section.name().unwrap_or("<invalid>").to_owned(),
                    size: section.size(),
                });
            }
        }

        let mut symbols = Vec::new();
        let mut symbol_count = 0usize;
        for symbol in obj.symbols() {
            let Ok(name) = symbol.name() else { continue };
            if name.is_empty() {
                continue;
            }
            symbol_count += 1;
            if symbols.len() < MAX_ENTRIES {
                symbols.push(name.to_owned());
            }
        }

        let view = ExecutableView {
            format: format!("{:?}", obj.format()),
            architecture: format!("{:?}", obj.architecture()),
            is_64_bit: obj.is_64(),
            entry: obj.entry(),
            section_count,
            sections,
            symbol_count,
            symbols,
            file_size,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The executable/object plugin's presentation half.
#[derive(Debug, Default)]
pub struct ExecutablePresentation;

impl PluginPresentation for ExecutablePresentation {
    fn name(&self) -> &'static str {
        "executable"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ExecutableView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "{} ({}, {}-bit)",
            view.format,
            view.architecture,
            if view.is_64_bit { 64 } else { 32 }
        )];
        lines.push(format!("Entry point: 0x{:x}", view.entry));
        lines.push(format!("{} sections", view.section_count));
        lines.extend(
            view.sections
                .iter()
                .map(|section| format!("  {} ({} bytes)", section.name, section.size)),
        );
        if view.section_count > view.sections.len() {
            lines.push(format!(
                "  ... {} more sections not shown",
                view.section_count - view.sections.len()
            ));
        }
        lines.push(format!("{} symbols", view.symbol_count));
        lines.extend(view.symbols.iter().map(|symbol| format!("  {symbol}")));
        if view.symbol_count > view.symbols.len() {
            lines.push(format!(
                "  ... {} more symbols not shown",
                view.symbol_count - view.symbols.len()
            ));
        }
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutableCore, ExecutablePresentation, ExecutableView, SectionInfo};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-executable-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn sniffs_elf_pe_and_macho_magic() {
        assert!(ExecutableCore.sniff(b"\x7fELF\x02\x01\x01\x00"));
        assert!(ExecutableCore.sniff(b"MZ\x90\x00"));
        assert!(ExecutableCore.sniff(&[0xFE, 0xED, 0xFA, 0xCF]));
        assert!(ExecutableCore.sniff(&[0xCA, 0xFE, 0xBA, 0xBE]));
        assert!(!ExecutableCore.sniff(b"not an object file"));
    }

    #[test]
    fn views_a_real_elf_shared_object() {
        // A real, freshly compiled ELF shared object rather than a hand
        // built one: the `object` crate's ELF reader exercises section
        // header string tables and the dynamic symbol table, both of which
        // are easy to get subtly wrong by hand.
        let source = unique_temp_file("lib.c");
        let output = unique_temp_file("lib.so");
        std::fs::write(&source, "int answer(void) { return 42; }\n").unwrap();
        let status = std::process::Command::new("cc")
            .args(["-shared", "-fPIC", "-o"])
            .arg(&output)
            .arg(&source)
            .status()
            .expect("a C compiler is required to build this test's fixture");
        assert!(status.success());

        let data = ExecutableCore.view(&output).unwrap();
        let view: ExecutableView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "Elf");
        assert!(view.is_64_bit);
        assert!(view.section_count > 0);
        assert!(view.symbols.iter().any(|symbol| symbol.contains("answer")));

        std::fs::remove_file(&source).unwrap();
        std::fs::remove_file(&output).unwrap();
    }

    #[test]
    fn presents_header_sections_and_symbols() {
        let data = serde_json::to_value(ExecutableView {
            format: "Elf".to_owned(),
            architecture: "X86_64".to_owned(),
            is_64_bit: true,
            entry: 0x1000,
            section_count: 1,
            sections: vec![SectionInfo {
                name: ".text".to_owned(),
                size: 256,
            }],
            symbol_count: 1,
            symbols: vec!["answer".to_owned()],
            file_size: 4096,
        })
        .unwrap();

        let lines = ExecutablePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Elf (X86_64, 64-bit)",
                "Entry point: 0x1000",
                "1 sections",
                "  .text (256 bytes)",
                "1 symbols",
                "  answer",
                "4096 bytes on disk",
            ]
        );
    }
}
