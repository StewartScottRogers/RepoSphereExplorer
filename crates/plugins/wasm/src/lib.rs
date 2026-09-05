//! WebAssembly module file type plugin: core and presentation halves.
//!
//! `.wasm` opens with a fixed 8-byte header (the `\0asm` magic followed by a
//! `u32` version), a marker not used by any sibling plugin, so `sniff` checks
//! only that. `view` parses the whole module structurally with the
//! `wasmparser` crate's low-level `Parser`/`Payload` walk (no `validate`
//! feature enabled, since a metadata view needs the module's shape, not a
//! full type-check), reading its imports, exports, and per-kind counts. The
//! view is a list of imports/exports and section counts, not a
//! disassembly, matching `font`/`executable`'s own precedent that the
//! presentation half is lines of text, not toolkit-specific output.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use wasmparser::{ExternalKind, Parser, Payload, TypeRef};

/// Maximum number of imports/exports listed in the view; modules with more
/// are truncated, matching the `executable` plugin's own `MAX_ENTRIES`.
const MAX_ENTRIES: usize = 200;

/// The WebAssembly binary format's fixed magic number (`\0asm`).
const WASM_MAGIC: &[u8; 4] = b"\0asm";

/// The kind of an import or export, named for [`ImportInfo::kind`] and
/// [`ExportInfo::kind`].
fn type_ref_kind(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Func(_) => "func",
        TypeRef::Table(_) => "table",
        TypeRef::Memory(_) => "memory",
        TypeRef::Global(_) => "global",
        TypeRef::Tag(_) => "tag",
    }
}

/// The kind of an export, named for [`ExportInfo::kind`].
fn external_kind_name(kind: ExternalKind) -> &'static str {
    match kind {
        ExternalKind::Func => "func",
        ExternalKind::Table => "table",
        ExternalKind::Memory => "memory",
        ExternalKind::Global => "global",
        ExternalKind::Tag => "tag",
    }
}

/// One entry in [`WasmView::imports`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    /// The module the import is drawn from.
    pub module: String,
    /// The imported item's name within that module.
    pub name: String,
    /// The imported item's kind, e.g. `"func"`, `"memory"`.
    pub kind: String,
}

/// One entry in [`WasmView::exports`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportInfo {
    /// The exported item's name.
    pub name: String,
    /// The exported item's kind, e.g. `"func"`, `"memory"`.
    pub kind: String,
}

/// View data produced by [`WasmCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmView {
    /// The binary format version from the module header.
    pub version: u16,
    /// Total number of imports.
    pub import_count: usize,
    /// The first [`MAX_ENTRIES`] imports.
    pub imports: Vec<ImportInfo>,
    /// Total number of exports.
    pub export_count: usize,
    /// The first [`MAX_ENTRIES`] exports.
    pub exports: Vec<ExportInfo>,
    /// Total number of functions, imported and defined.
    pub function_count: u32,
    /// Total number of memories, imported and defined.
    pub memory_count: u32,
    /// Total number of tables, imported and defined.
    pub table_count: u32,
    /// Total number of globals, imported and defined.
    pub global_count: u32,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// The WebAssembly module plugin's core half.
#[derive(Debug, Default)]
pub struct WasmCore;

impl PluginCore for WasmCore {
    fn name(&self) -> &'static str {
        "wasm"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(WASM_MAGIC)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let file_size = std::fs::metadata(path)?.len();
        let data = std::fs::read(path)?;

        let mut version = 0u16;
        let mut imports = Vec::new();
        let mut import_count = 0usize;
        let mut exports = Vec::new();
        let mut export_count = 0usize;
        let mut function_count = 0u32;
        let mut memory_count = 0u32;
        let mut table_count = 0u32;
        let mut global_count = 0u32;

        for payload in Parser::new(0).parse_all(&data) {
            let payload = payload.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            match payload {
                Payload::Version { num, .. } => version = num,
                Payload::ImportSection(reader) => {
                    for import in reader {
                        let import = import
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                        match import.ty {
                            TypeRef::Func(_) => function_count += 1,
                            TypeRef::Table(_) => table_count += 1,
                            TypeRef::Memory(_) => memory_count += 1,
                            TypeRef::Global(_) => global_count += 1,
                            TypeRef::Tag(_) => {}
                        }
                        import_count += 1;
                        if imports.len() < MAX_ENTRIES {
                            imports.push(ImportInfo {
                                module: import.module.to_owned(),
                                name: import.name.to_owned(),
                                kind: type_ref_kind(&import.ty).to_owned(),
                            });
                        }
                    }
                }
                Payload::FunctionSection(reader) => function_count += reader.count(),
                Payload::TableSection(reader) => table_count += reader.count(),
                Payload::MemorySection(reader) => memory_count += reader.count(),
                Payload::GlobalSection(reader) => global_count += reader.count(),
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                        export_count += 1;
                        if exports.len() < MAX_ENTRIES {
                            exports.push(ExportInfo {
                                name: export.name.to_owned(),
                                kind: external_kind_name(export.kind).to_owned(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        let view = WasmView {
            version,
            import_count,
            imports,
            export_count,
            exports,
            function_count,
            memory_count,
            table_count,
            global_count,
            file_size,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The WebAssembly module plugin's presentation half.
#[derive(Debug, Default)]
pub struct WasmPresentation;

impl PluginPresentation for WasmPresentation {
    fn name(&self) -> &'static str {
        "wasm"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: WasmView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("WebAssembly module (version {})", view.version)];
        lines.push(format!(
            "{} functions, {} memories, {} tables, {} globals",
            view.function_count, view.memory_count, view.table_count, view.global_count
        ));
        lines.push(format!("{} imports", view.import_count));
        lines.extend(
            view.imports
                .iter()
                .map(|import| format!("  {}.{} ({})", import.module, import.name, import.kind)),
        );
        if view.import_count > view.imports.len() {
            lines.push(format!(
                "  ... {} more imports not shown",
                view.import_count - view.imports.len()
            ));
        }
        lines.push(format!("{} exports", view.export_count));
        lines.extend(
            view.exports
                .iter()
                .map(|export| format!("  {} ({})", export.name, export.kind)),
        );
        if view.export_count > view.exports.len() {
            lines.push(format!(
                "  ... {} more exports not shown",
                view.export_count - view.exports.len()
            ));
        }
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ExportInfo, ImportInfo, WasmCore, WasmPresentation, WasmView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-wasm-test-{}-{name}",
            std::process::id()
        ))
    }

    /// A minimal, valid WebAssembly module built by hand from the binary
    /// format spec: the header, a type section (one `() -> i32` signature),
    /// an import section (one function, `env.log`), a function section (one
    /// defined function using that same signature), a memory section (one
    /// memory), an export section (the defined function exported as `run`),
    /// and a code section with that function's trivial body (`i32.const 0`,
    /// `end`) - enough for `wasmparser` to walk every section this plugin
    /// reads.
    fn write_test_wasm(path: &std::path::Path) {
        fn section(id: u8, body: Vec<u8>) -> Vec<u8> {
            let mut out = vec![id];
            leb128(&mut out, u64::try_from(body.len()).unwrap());
            out.extend(body);
            out
        }

        fn leb128(out: &mut Vec<u8>, mut value: u64) {
            loop {
                let byte = (value & 0x7f) as u8;
                value >>= 7;
                if value == 0 {
                    out.push(byte);
                    break;
                }
                out.push(byte | 0x80);
            }
        }

        fn name(out: &mut Vec<u8>, value: &str) {
            leb128(out, u64::try_from(value.len()).unwrap());
            out.extend(value.as_bytes());
        }

        let mut module = Vec::new();
        module.extend(b"\0asm");
        module.extend([0x01, 0x00, 0x00, 0x00]);

        // Type section: one signature, `() -> i32`.
        let mut types = vec![0x01]; // one type
        types.push(0x60); // func type
        types.push(0x00); // zero params
        types.push(0x01); // one result
        types.push(0x7f); // i32
        module.extend(section(1, types));

        // Import section: one function import, `env.log`.
        let mut imports = vec![0x01]; // one import
        name(&mut imports, "env");
        name(&mut imports, "log");
        imports.push(0x00); // func import
        imports.push(0x00); // type index 0
        module.extend(section(2, imports));

        // Function section: one defined function, using type 0.
        let functions = vec![0x01, 0x00];
        module.extend(section(3, functions));

        // Memory section: one memory, min 1 page.
        let memories = vec![0x01, 0x00, 0x01];
        module.extend(section(5, memories));

        // Export section: the defined function (index 1: import takes 0).
        let mut exports = vec![0x01]; // one export
        name(&mut exports, "run");
        exports.push(0x00); // func export
        exports.push(0x01); // function index 1
        module.extend(section(7, exports));

        // Code section: one function body, `i32.const 0` then `end`.
        let body = vec![0x00, 0x41, 0x00, 0x0b]; // no locals; i32.const 0; end
        let mut code = vec![0x01]; // one function body
        leb128(&mut code, u64::try_from(body.len()).unwrap());
        code.extend(body);
        module.extend(section(10, code));

        std::fs::write(path, module).unwrap();
    }

    #[test]
    fn sniffs_the_wasm_magic() {
        assert!(WasmCore.sniff(b"\0asm\x01\x00\x00\x00"));
        assert!(!WasmCore.sniff(b"not a wasm module"));
    }

    #[test]
    fn views_a_real_wasm_module() {
        let path = unique_temp_file("test.wasm");
        write_test_wasm(&path);

        let data = WasmCore.view(&path).unwrap();
        let view: WasmView = serde_json::from_value(data).unwrap();

        assert_eq!(view.version, 1);
        assert_eq!(view.function_count, 2);
        assert_eq!(view.memory_count, 1);
        assert_eq!(view.table_count, 0);
        assert_eq!(view.import_count, 1);
        assert_eq!(view.imports[0].module, "env");
        assert_eq!(view.imports[0].name, "log");
        assert_eq!(view.imports[0].kind, "func");
        assert_eq!(view.export_count, 1);
        assert_eq!(view.exports[0].name, "run");
        assert_eq!(view.exports[0].kind, "func");
        assert!(view.file_size > 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_summary_imports_and_exports() {
        let data = serde_json::to_value(WasmView {
            version: 1,
            import_count: 1,
            imports: vec![ImportInfo {
                module: "env".to_owned(),
                name: "log".to_owned(),
                kind: "func".to_owned(),
            }],
            export_count: 1,
            exports: vec![ExportInfo {
                name: "run".to_owned(),
                kind: "func".to_owned(),
            }],
            function_count: 2,
            memory_count: 1,
            table_count: 0,
            global_count: 0,
            file_size: 64,
        })
        .unwrap();

        let lines = WasmPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "WebAssembly module (version 1)",
                "2 functions, 1 memories, 0 tables, 0 globals",
                "1 imports",
                "  env.log (func)",
                "1 exports",
                "  run (func)",
                "64 bytes on disk",
            ]
        );
    }
}
