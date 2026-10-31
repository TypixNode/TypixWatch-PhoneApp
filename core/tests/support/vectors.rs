//! One vector set, two implementations.  `build` runs against a `Backend`, once with
//! this crate and once with the watch's own `w1a_sec.c`; the JSON must come out equal.
//!
//! The committed file holds public outputs only.  Inputs are not stored: they are
//! hashes of a label, or the same trivial constants as `tests/sectest.c` on the watch
//! side.  Session keys appear as their SHA-256.
#![allow(dead_code)]

use serde_json::{json, Value};
use typix_core::sec::{hex, sha256};

pub trait Backend {
    fn map(&mut self, hidden: &[u8; 32]) -> [u8; 32];
    fn share(&mut self, sk: &[u8; 32], code: &str, sid: &[u8; 16]) -> [u8; 32];
    /// The two directions' keys.
    fn finish(
        &mut self,
        sk: &[u8; 32],
        peer: &[u8; 32],
        ya: &[u8; 32],
        yb: &[u8; 32],
        sid: &[u8; 16],
    ) -> ([u8; 32], [u8; 32]);
    /// Message 1, message 2, a record each way: `"ping"` and `"pong"`.
    fn ik(
        &mut self,
        i: &[u8; 32],
        r: &[u8; 32],
        ei: &[u8; 32],
        er: &[u8; 32],
        prologue: &[u8],
    ) -> [Vec<u8>; 4];
    fn manifest(&mut self, t: u8, seq: u32, size: u32, sha: &[u8; 32], name: &str) -> Vec<u8>;
    fn sign(&mut self, seed: &[u8; 32], msg: &[u8]) -> ([u8; 32], [u8; 64]);
    fn fingerprint(&mut self, public: &[u8; 32]) -> String;
    fn x25519_public(&mut self, sk: &[u8; 32]) -> [u8; 32];
    fn seal(&mut self, key: &[u8; 32], counter: u64, pt: &[u8]) -> Vec<u8>;
    fn sha256(&mut self, data: &[u8]) -> [u8; 32];
}

pub fn derive(label: &str, i: u32) -> [u8; 32] {
    sha256(format!("typix-vector/{label}/{i}").as_bytes())
}

fn sid(i: u32) -> [u8; 16] {
    derive("sid", i)[..16].try_into().unwrap()
}

fn code(i: u32) -> String {
    let d = derive("code", i);
    format!("{:06}", u32::from_le_bytes(d[..4].try_into().unwrap()) % 1_000_000)
}

fn first(v: u8) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0] = v;
    b
}

pub fn build(b: &mut dyn Backend) -> Value {
    // Every combination of the two bits the two Elligator conventions disagree about.
    let mut map = vec![];
    for i in 0..64u32 {
        let mut h = derive("map", i);
        h[31] = (h[31] & 0x3f) | ((i % 4) as u8) << 6;
        map.push(json!({"hi": i % 4, "g": hex(&b.map(&h))}));
    }

    let mut pace = vec![];
    for i in 0..16u32 {
        let (y1, y2) = (derive("ya", i), derive("yb", i));
        let (c, s) = (code(i), sid(i));
        let ya = b.share(&y1, &c, &s);
        let yb = b.share(&y2, &c, &s);
        let (k1, k2) = b.finish(&y1, &yb, &ya, &yb, &s);
        let mut both = k1.to_vec();
        both.extend_from_slice(&k2);
        pace.push(json!({"ya": hex(&ya), "yb": hex(&yb), "keys_sha256": hex(&sha256(&both))}));
    }

    let mut ik = vec![];
    for i in 0..4u32 {
        let [m1, m2, ping, pong] = b.ik(
            &derive("ik-i", i),
            &derive("ik-r", i),
            &derive("ik-ei", i),
            &derive("ik-er", i),
            b"W1AO\x01\x02",
        );
        ik.push(json!({"m1": hex(&m1), "m2": hex(&m2), "ping": hex(&ping), "pong": hex(&pong)}));
    }

    let mut sigs = vec![];
    for (t, seq, size, name) in [
        (1u8, 1u32, 1u32, "w1a-shell.out"),
        (2, 7, 8_600_000, "assets.pak"),
        (3, 4_000_000_000, 0, "fix.sh"),
        (4, 2, 15_728_640, "錶.img"),
    ] {
        let sha = derive("file", seq);
        let m = b.manifest(t, seq, size, &sha, name);
        let (public, sig) = b.sign(&derive("seed", seq), &m);
        sigs.push(json!({"manifest": hex(&m), "public": hex(&public), "sig": hex(&sig),
                         "fingerprint": b.fingerprint(&public)}));
    }

    let aead: Vec<_> = [0u64, 1, 255, 1 << 32]
        .iter()
        .map(|n| hex(&b.seal(&derive("aead", 0), *n, b"sealed under the pairing key")))
        .collect();

    // The inputs of tests/sectest.c, which only checks round trips: here the outputs
    // are pinned too.  sk2 = 0x20, y1 = 0x50, y2 = 0x60, sid = 7, seed = 8, sha = 9.
    let mut s = [0u8; 16];
    s[0] = 7;
    let ya = b.share(&first(0x50), "123456", &s);
    let yb = b.share(&first(0x60), "123456", &s);
    let wrong = b.share(&first(0x60), "123457", &s);
    let m = b.manifest(1, 2, 3, &first(9), "x");
    let (public, sig) = b.sign(&first(8), &m);
    let sectest = json!({
        "pub2": hex(&b.x25519_public(&first(0x20))),
        "ya": hex(&ya), "yb": hex(&yb), "yb_wrong_code": hex(&wrong),
        "manifest": hex(&m), "public": hex(&public), "sig": hex(&sig),
    });

    json!({
        "sha256": {
            "abc": hex(&b.sha256(b"abc")),
            "empty": hex(&b.sha256(b"")),
            "two_block": hex(&b.sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "million_a": hex(&b.sha256(&vec![b'a'; 1_000_000])),
        },
        "elligator": map,
        "cpace": pace,
        "noise_ik": ik,
        "packages": sigs,
        "aead": aead,
        "sectest": sectest,
    })
}

/// This crate as a `Backend`.
pub struct Rust;

impl Backend for Rust {
    fn map(&mut self, hidden: &[u8; 32]) -> [u8; 32] {
        typix_core::cpace::elligator_map(hidden)
    }

    fn share(&mut self, sk: &[u8; 32], code: &str, sid: &[u8; 16]) -> [u8; 32] {
        typix_core::cpace::share(sk, code, sid)
    }

    fn finish(
        &mut self,
        sk: &[u8; 32],
        peer: &[u8; 32],
        ya: &[u8; 32],
        yb: &[u8; 32],
        sid: &[u8; 16],
    ) -> ([u8; 32], [u8; 32]) {
        let (a, b) = typix_core::cpace::finish(sk, peer, ya, yb, sid).unwrap();
        (*a.key(), *b.key())
    }

    fn ik(
        &mut self,
        i: &[u8; 32],
        r: &[u8; 32],
        ei: &[u8; 32],
        er: &[u8; 32],
        prologue: &[u8],
    ) -> [Vec<u8>; 4] {
        use typix_core::noise::{Initiator, PATTERN};
        use typix_core::sec::x25519_public;

        let mut init =
            Initiator::with_ephemeral_for_test(i, &x25519_public(r), prologue, ei).unwrap();
        let m1 = init.write1().unwrap();
        let mut resp = snow::Builder::new(PATTERN.parse().unwrap())
            .local_private_key(r)
            .unwrap()
            .fixed_ephemeral_key_for_testing_only(er)
            .prologue(prologue)
            .unwrap()
            .build_responder()
            .unwrap();
        let mut p = [0u8; 64];
        resp.read_message(&m1, &mut p).unwrap();
        let mut m2 = vec![0u8; 48];
        let n = resp.write_message(&[], &mut m2).unwrap();
        m2.truncate(n);
        let mut tx = init.read2(&m2).unwrap();
        let ping = tx.seal(b"ping").unwrap();
        let mut resp = resp.into_transport_mode().unwrap();
        let mut pong = vec![0u8; 20];
        let n = resp.write_message(b"pong", &mut pong).unwrap();
        pong.truncate(n);
        [m1, m2, ping, pong]
    }

    fn manifest(&mut self, t: u8, seq: u32, size: u32, sha: &[u8; 32], name: &str) -> Vec<u8> {
        typix_core::sec::manifest(t, seq, size, sha, name)
    }

    fn sign(&mut self, seed: &[u8; 32], msg: &[u8]) -> ([u8; 32], [u8; 64]) {
        (
            typix_core::sec::sign_public(seed),
            typix_core::sec::sign(seed, msg),
        )
    }

    fn fingerprint(&mut self, public: &[u8; 32]) -> String {
        typix_core::sec::fingerprint(public)
    }

    fn x25519_public(&mut self, sk: &[u8; 32]) -> [u8; 32] {
        typix_core::sec::x25519_public(sk)
    }

    fn seal(&mut self, key: &[u8; 32], counter: u64, pt: &[u8]) -> Vec<u8> {
        typix_core::sec::Cipher::with_counter(*key, counter)
            .seal(pt)
            .unwrap()
    }

    fn sha256(&mut self, data: &[u8]) -> [u8; 32] {
        sha256(data)
    }
}
