//! This crate against the vectors the watch's C code produced.  Needs nothing outside
//! this repository.  `interop_c.rs` regenerates them.

#[path = "support/vectors.rs"]
mod vectors;

use serde_json::Value;

#[test]
fn matches_the_vectors_from_the_c_code() {
    let want: Value =
        serde_json::from_str(include_str!("vectors/w1a-sec.json")).expect("vector file");
    let got = vectors::build(&mut vectors::Rust);
    for key in want.as_object().unwrap().keys() {
        assert_eq!(got[key], want[key], "{key}");
    }
    assert_eq!(got, want);
}

#[test]
fn fips_180_answers_are_the_ones_sectest_checks() {
    let want: Value = serde_json::from_str(include_str!("vectors/w1a-sec.json")).unwrap();
    let s = &want["sha256"];
    assert_eq!(
        s["abc"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        s["empty"],
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        s["two_block"],
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        s["million_a"],
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn no_private_input_is_stored() {
    // Inputs are derived from labels, so the file cannot contain one verbatim.
    let text = include_str!("vectors/w1a-sec.json");
    for label in ["ya", "yb", "ik-i", "ik-r", "ik-ei", "ik-er", "seed", "map"] {
        for i in 0..4 {
            let secret = typix_core::sec::hex(&vectors::derive(label, i));
            assert!(!text.contains(&secret), "{label}/{i}");
        }
    }
}
