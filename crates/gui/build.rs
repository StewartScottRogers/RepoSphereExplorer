//! Compiles the Slint UI markup into generated Rust types.

fn main() {
    // Debug info is what lets `tests/context_menu.rs` locate elements through
    // Slint's testing API. It is only emitted for debug builds - which is
    // every build `cargo test` makes - so the release binary keeps none of it.
    let debug_info = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug_info);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
