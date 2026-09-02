//! Compiles the Slint UI markup into generated Rust types.

fn main() {
    slint_build::compile("ui/app.slint").unwrap();
}
