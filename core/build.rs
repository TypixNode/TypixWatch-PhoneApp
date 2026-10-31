//! Compiles Monocypher for one function, `crypto_elligator_map`.
//!
//! The pairing generator has to be the exact point the watch computes, and that
//! function masks two high bits where the RFC 9380 map masks one. No maintained crate
//! exposes this map, so the app uses the same C file as the watch and every other
//! primitive comes from a Rust crate. See README.md.
fn main() {
    println!("cargo:rerun-if-changed=vendor/monocypher/monocypher.c");
    println!("cargo:rerun-if-changed=vendor/monocypher/monocypher.h");
    cc::Build::new()
        .file("vendor/monocypher/monocypher.c")
        .include("vendor/monocypher")
        .warnings(false)
        .compile("monocypher");
}
