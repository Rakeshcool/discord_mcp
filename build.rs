//! Build-time link flags.
//!
//! On MSVC, `link.exe` stamps each executable with the wall-clock time and a
//! random PDB GUID. Everything else in this build is already deterministic
//! (`strip = true`, no debug info, fixed dependency versions), so those stamps
//! were the only difference between two builds of the same source — which made
//! the published checksum unverifiable: a rebuild could never match it.
//!
//! `/Brepro` derives both fields from the output's own contents instead of the
//! clock, so two builds of the same commit are byte-identical. Verified by
//! building twice and comparing hashes.
//!
//! It is emitted as a link argument rather than through `RUSTFLAGS` or a
//! `target.*.rustflags` entry: an environment that sets `RUSTFLAGS` (CI sets it
//! to `-D warnings`) replaces target rustflags, whereas link arguments from a
//! build script apply regardless.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // Only the MSVC linker understands this flag; other targets need nothing.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/Brepro");
    }
}
