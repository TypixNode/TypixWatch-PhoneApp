//! Differential tests against the watch's C sources, compiled here.
//!
//! The sources live in the firmware repository, one level up, or at `W1A_OTAD_DIR`.
//! Without them these tests print that they were skipped and pass, unless
//! `TYPIX_REQUIRE_C=1`.  `TYPIX_WRITE_VECTORS=1` rewrites `vectors/w1a-sec.json`.

#[path = "support/vectors.rs"]
mod vectors;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use typix_core::noise::Initiator;
use typix_core::proto::hello;
use typix_core::sec::{hex, random32, x25519_public, Cipher};
use typix_core::{cpace, sec};
use vectors::{Backend, Rust};

fn otad_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("W1A_OTAD_DIR").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../linux-build/tools/w1a-otad")
    });
    if dir.join("w1a_sec.c").exists() {
        return Some(dir);
    }
    assert!(
        std::env::var_os("TYPIX_REQUIRE_C").is_none(),
        "w1a_sec.c not found at {}",
        dir.display()
    );
    eprintln!("SKIPPED: no w1a_sec.c at {}", dir.display());
    None
}

/// Tests run in parallel and share the binaries.
fn compile(dir: &Path, main: &Path, output: &str) -> PathBuf {
    static BUILT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let mut built = BUILT.lock().unwrap_or_else(|p| p.into_inner());
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(output);
    if built.contains(&out) {
        return out;
    }
    let mc = dir.join("vendor/monocypher");
    let status = Command::new("cc")
        .args(["-O2", "-w", "-I"])
        .arg(&mc)
        .arg("-I")
        .arg(dir)
        .arg(main)
        .arg(dir.join("w1a_sec.c"))
        .arg(mc.join("monocypher.c"))
        .arg(mc.join("monocypher-ed25519.c"))
        .arg("-o")
        .arg(&out)
        .status()
        .expect("cc");
    assert!(status.success());
    built.push(out.clone());
    out
}

struct C {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl C {
    fn start(dir: &Path) -> Self {
        let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/interop/harness.c");
        let bin = compile(dir, &main, "w1a-harness");
        let mut child = Command::new(bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            stdin: child.stdin.take().unwrap(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            _child: child,
        }
    }

    fn ask(&mut self, line: &str) -> Vec<String> {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut got = String::new();
        self.stdout.read_line(&mut got).unwrap();
        got.split_whitespace().map(str::to_string).collect()
    }
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return vec![];
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn fixed<const N: usize>(s: &str) -> [u8; N] {
    unhex(s).try_into().unwrap()
}

fn h(b: &[u8]) -> String {
    if b.is_empty() {
        "-".into()
    } else {
        hex(b)
    }
}

impl Backend for C {
    fn map(&mut self, hidden: &[u8; 32]) -> [u8; 32] {
        fixed(&self.ask(&format!("map {}", hex(hidden)))[0])
    }

    fn share(&mut self, sk: &[u8; 32], code: &str, sid: &[u8; 16]) -> [u8; 32] {
        fixed(&self.ask(&format!("share {} {code} {}", hex(sk), hex(sid)))[0])
    }

    fn finish(
        &mut self,
        sk: &[u8; 32],
        peer: &[u8; 32],
        ya: &[u8; 32],
        yb: &[u8; 32],
        sid: &[u8; 16],
    ) -> ([u8; 32], [u8; 32]) {
        let r = self.ask(&format!(
            "finish {} {} {} {} {}",
            hex(sk),
            hex(peer),
            hex(ya),
            hex(yb),
            hex(sid)
        ));
        (fixed(&r[0]), fixed(&r[1]))
    }

    fn ik(
        &mut self,
        i: &[u8; 32],
        r: &[u8; 32],
        ei: &[u8; 32],
        er: &[u8; 32],
        prologue: &[u8],
    ) -> [Vec<u8>; 4] {
        let got = self.ask(&format!(
            "ik {} {} {} {} {}",
            hex(i),
            hex(r),
            hex(ei),
            hex(er),
            hex(prologue)
        ));
        assert_eq!(got.len(), 4, "{got:?}");
        [unhex(&got[0]), unhex(&got[1]), unhex(&got[2]), unhex(&got[3])]
    }

    fn manifest(&mut self, t: u8, seq: u32, size: u32, sha: &[u8; 32], name: &str) -> Vec<u8> {
        unhex(&self.ask(&format!(
            "manifest {t} {seq} {size} {} {}",
            hex(sha),
            hex(name.as_bytes())
        ))[0])
    }

    fn sign(&mut self, seed: &[u8; 32], msg: &[u8]) -> ([u8; 32], [u8; 64]) {
        let r = self.ask(&format!("sign {} {}", hex(seed), h(msg)));
        assert_eq!(r.len(), 2, "{r:?}");
        (fixed(&r[0]), fixed(&r[1]))
    }

    fn fingerprint(&mut self, public: &[u8; 32]) -> String {
        self.ask(&format!("fingerprint {}", hex(public)))[0].clone()
    }

    fn x25519_public(&mut self, sk: &[u8; 32]) -> [u8; 32] {
        fixed(&self.ask(&format!("public {}", hex(sk)))[0])
    }

    fn seal(&mut self, key: &[u8; 32], counter: u64, pt: &[u8]) -> Vec<u8> {
        unhex(&self.ask(&format!("seal {} {counter} {}", hex(key), h(pt)))[0])
    }

    fn sha256(&mut self, data: &[u8]) -> [u8; 32] {
        fixed(&self.ask(&format!("sha256 {}", h(data)))[0])
    }
}

#[test]
fn vectors_agree_with_the_c_code() {
    let Some(dir) = otad_dir() else { return };
    let from_c = vectors::build(&mut C::start(&dir));
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/w1a-sec.json");
    if std::env::var_os("TYPIX_WRITE_VECTORS").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&from_c).unwrap() + "\n").unwrap();
    }
    let from_rust = vectors::build(&mut Rust);
    for key in from_c.as_object().unwrap().keys() {
        assert_eq!(from_rust[key], from_c[key], "{key}");
    }
}

#[test]
fn the_vendored_monocypher_is_the_watchs() {
    let Some(dir) = otad_dir() else { return };
    for f in ["monocypher.c", "monocypher.h"] {
        let ours = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/monocypher").join(f));
        let theirs = std::fs::read(dir.join("vendor/monocypher").join(f));
        assert_eq!(ours.unwrap(), theirs.unwrap(), "{f}");
    }
}

/// Random hiddens hit both branches of the map and every value of the top two bits.
#[test]
fn elligator_on_random_inputs() {
    let Some(dir) = otad_dir() else { return };
    let mut c = C::start(&dir);
    for _ in 0..3000 {
        let r = random32().unwrap();
        assert_eq!(cpace::elligator_map(&r), c.map(&r), "{}", hex(&r[..]));
    }
}

/// One side Rust, the other C, both ways round, with fresh randomness every run.
#[test]
fn cpace_agrees_across_implementations() {
    let Some(dir) = otad_dir() else { return };
    let mut c = C::start(&dir);
    for i in 0..300u32 {
        let (phone, watch) = (random32().unwrap(), random32().unwrap());
        let sid: [u8; 16] = random32().unwrap()[..16].try_into().unwrap();
        let code = format!("{:06}", i * 3331 % 1_000_000);

        let ya = cpace::share(&phone, &code, &sid);
        assert_eq!(ya, c.share(&phone, &code, &sid));
        let yb = c.share(&watch, &code, &sid);
        let (tx, rx) = cpace::finish(&phone, &yb, &ya, &yb, &sid).unwrap();
        let (c1, c2) = c.finish(&watch, &ya, &ya, &yb, &sid);
        assert_eq!(tx.key(), &c1);
        assert_eq!(rx.key(), &c2);

        let wrong = format!("{:06}", (i * 3331 + 1) % 1_000_000);
        let yb = c.share(&watch, &wrong, &sid);
        let (tx, _) = cpace::finish(&phone, &yb, &ya, &yb, &sid).unwrap();
        let (c1, _) = c.finish(&watch, &ya, &ya, &yb, &sid);
        assert_ne!(tx.key(), &c1, "a wrong code must not agree");
    }
}

#[test]
fn pairing_confirmation_opens_on_the_other_side() {
    let Some(dir) = otad_dir() else { return };
    let mut c = C::start(&dir);
    let key = random32().unwrap();
    let mut mine = Cipher::new(*key);
    for n in 0..3u64 {
        let ct = mine.seal(b"identity, signing key, name").unwrap();
        let got = c.ask(&format!("open {} {n} {}", hex(&key[..]), hex(&ct)));
        assert_eq!(unhex(&got[0]), b"identity, signing key, name");
    }
    let mut peer = Cipher::new(*key);
    let ct = c.seal(&key, 0, b"watch key");
    assert_eq!(peer.open(&ct).unwrap(), b"watch key");
    let mut bad = ct.clone();
    bad[0] ^= 1;
    assert!(Cipher::new(*key).open(&bad).is_none());
}

#[test]
fn signatures_verify_under_the_watchs_check() {
    let Some(dir) = otad_dir() else { return };
    let mut c = C::start(&dir);
    for i in 0..50u32 {
        let seed = random32().unwrap();
        let sha = *random32().unwrap();
        let m = sec::manifest(3, i + 1, i * 1000, &sha, "fix.sh");
        assert_eq!(m, c.manifest(3, i + 1, i * 1000, &sha, "fix.sh"));
        let sig = sec::sign(&seed, &m);
        let public = sec::sign_public(&seed);
        let line = format!("verify {} {} {}", hex(&sig), hex(&public), hex(&m));
        assert_eq!(c.ask(&line)[0], "1");
        let mut tampered = m.clone();
        tampered[20] ^= 1;
        let line = format!("verify {} {} {}", hex(&sig), hex(&public), hex(&tampered));
        assert_eq!(c.ask(&line)[0], "0");
    }
}

/// The same conversation `tests/interop.py` has with `sectest noise`: that file's
/// responder, with this crate in the place of the `noiseprotocol` package.
#[test]
fn noise_against_sectest_responder() {
    let Some(dir) = otad_dir() else { return };
    let bin = compile(&dir, &dir.join("tests/sectest.c"), "w1a-sectest");

    let own = Command::new(&bin).output().unwrap();
    assert!(own.status.success(), "{}", String::from_utf8_lossy(&own.stdout));

    for _ in 0..20 {
        let mut child = Command::new(&bin)
            .arg("noise")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut tx = |b: &[u8]| {
            writeln!(stdin, "{}", hex(b)).unwrap();
            stdin.flush().unwrap();
        };
        let mut rx = |tag: &str| {
            let mut l = String::new();
            stdout.read_line(&mut l).unwrap();
            let mut p = l.split_whitespace();
            assert_eq!(p.next(), Some(tag), "{l}");
            unhex(p.next().unwrap())
        };

        let (watch_sk, phone_sk, e) = (random32().unwrap(), random32().unwrap(), random32().unwrap());
        tx(&watch_sk[..]);
        tx(&e[..]);
        let prologue = hello(2);
        let mut init =
            Initiator::new(&phone_sk, &x25519_public(&watch_sk), &prologue).unwrap();
        tx(&init.write1().unwrap());
        assert_eq!(rx("peer"), x25519_public(&phone_sk));
        let mut transport = init.read2(&rx("m2")).unwrap();
        tx(&transport.seal(b"interop record").unwrap());
        assert_eq!(transport.open(&rx("echo")).unwrap(), b"interop record");
        child.wait().unwrap();
    }
}
