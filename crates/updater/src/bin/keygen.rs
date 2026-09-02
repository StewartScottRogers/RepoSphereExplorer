//! Generates a new Ed25519 keypair for signing releases.
//!
//! Run with `cargo run -p updater --bin keygen`. Paste the public key into
//! `updater::PUBLIC_KEY` and store the private key only as the
//! `UPDATER_SIGNING_KEY` GitHub Actions secret - never commit it.

use ed25519_dalek::SigningKey;

fn main() {
    let mut rng = rand::rng();
    let signing_key = SigningKey::generate(&mut rng);
    let public_bytes = signing_key.verifying_key().to_bytes();
    let private_bytes = signing_key.to_bytes();

    println!("Public key (paste into updater::PUBLIC_KEY):");
    println!("{}", format_as_rust_array(&public_bytes));
    println!();
    println!("Private key, hex-encoded (store ONLY as the UPDATER_SIGNING_KEY secret):");
    println!("{}", hex_encode(&private_bytes));
}

fn format_as_rust_array(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("[\n    ");
    for (i, byte) in bytes.iter().enumerate() {
        let _ = write!(out, "0x{byte:02x}, ");
        if (i + 1) % 12 == 0 {
            out.push_str("\n    ");
        }
    }
    out.push_str("\n]");
    out
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}
