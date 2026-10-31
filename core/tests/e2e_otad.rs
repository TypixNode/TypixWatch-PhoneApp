//! `WatchClient` against the real otad.c in a container, over real TCP.  Run through
//! `tests/e2e/otad-docker.sh test`; a plain `cargo test` skips these.
//!
//! Pairing, the session handshake, status, push, resume and pull all talk to the
//! daemon that runs on the watch, with its real checks.  The link is TCP, so the BLE
//! path is exercised only as far as its chunk sizes.

mod support;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use support::{MemFile, MemSink, MemStore};
use typix_core::{
    ble_profile, tcp_profile, CoreError, LinkProfile, LogKind, PackageType, WatchClient,
    WatchEvent, WatchEventListener, WatchLink,
};

fn exec(cmd: &str) -> String {
    let container = std::env::var("TYPIX_OTAD_CONTAINER").expect("use otad-docker.sh test");
    let out = Command::new("docker")
        .args(["exec", &container, "sh", "-c", cmd])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn watch_state(key: &str) -> Option<String> {
    exec("cat /run/w1a-otad.state")
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}\t")).map(str::to_string))
}

/// The daemon publishes its state every 250 ms.
fn expect_state(key: &str, want: &str) {
    for _ in 0..40 {
        if watch_state(key).as_deref() == Some(want) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{key} != {want}: {}", exec("cat /run/w1a-otad.state"));
}

/// The watch's "pair" menu item and key press, then the code off its screen.
fn open_pairing() -> String {
    exec("/obj/w1a-otad-test -c 'arm pair'");
    for _ in 0..40 {
        if let Some(code) = watch_state("code").filter(|c| c != "-") {
            return code;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("no code: {}", exec("cat /run/w1a-otad.state"));
}

struct Channel(Mutex<Sender<WatchEvent>>);

impl WatchEventListener for Channel {
    fn on_event(&self, event: WatchEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

struct Tcp(Mutex<TcpStream>);

impl WatchLink for Tcp {
    fn send(&self, bytes: Vec<u8>) {
        let _ = self.0.lock().unwrap().write_all(&bytes);
    }
    fn close_link(&self) {
        let _ = self.0.lock().unwrap().shutdown(Shutdown::Both);
    }
}

struct Phone {
    client: Arc<WatchClient>,
    events: Receiver<WatchEvent>,
    reader: Option<JoinHandle<()>>,
    stream: Option<TcpStream>,
}

impl Phone {
    fn new() -> Self {
        let (tx, events) = channel();
        let client = WatchClient::new(
            Arc::new(MemStore::default()),
            Arc::new(Channel(Mutex::new(tx))),
        )
        .unwrap();
        Self {
            client,
            events,
            reader: None,
            stream: None,
        }
    }

    fn dial(&mut self) -> Arc<Tcp> {
        self.hang_up();
        let s = TcpStream::connect(("127.0.0.1", 7423)).unwrap();
        s.set_nodelay(true).unwrap();
        let mut rx = s.try_clone().unwrap();
        self.stream = Some(s.try_clone().unwrap());
        let client = self.client.clone();
        self.reader = Some(std::thread::spawn(move || {
            let mut buf = vec![0u8; 65536];
            loop {
                match rx.read(&mut buf) {
                    Ok(n) if n > 0 => client.on_bytes(buf[..n].to_vec()),
                    _ => break,
                }
            }
            client.on_link_closed();
        }));
        Arc::new(Tcp(Mutex::new(s)))
    }

    fn hang_up(&mut self) {
        if let Some(s) = self.stream.take() {
            let _ = s.shutdown(Shutdown::Both);
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        // Whatever the closed link reported belongs to the conversation that just ended.
        while self.events.try_recv().is_ok() {}
    }

    /// Events until `done` says stop; a failure stops too.
    fn wait(&self, mut done: impl FnMut(&WatchEvent) -> bool) -> Result<WatchEvent, CoreError> {
        let end = Instant::now() + Duration::from_secs(60);
        while Instant::now() < end {
            let Ok(e) = self.events.recv_timeout(Duration::from_millis(250)) else {
                continue;
            };
            if let WatchEvent::Failed { error } = e {
                return Err(error);
            }
            if done(&e) {
                return Ok(e);
            }
        }
        panic!("nothing happened for 60 s");
    }

    fn pair(&mut self, code: String) -> Result<WatchEvent, CoreError> {
        let link = self.dial();
        self.client
            .begin_pairing(link, tcp_profile(), code, "typix-e2e".into())
            .unwrap();
        self.wait(|e| matches!(e, WatchEvent::Paired { .. }))
    }

    fn session(&mut self, profile: LinkProfile) -> Result<(), CoreError> {
        let link = self.dial();
        self.client.begin_session(link, profile)?;
        self.wait(|e| {
            matches!(e, WatchEvent::State { state } if *state == typix_core::ConnectionState::Ready)
        })
        .map(|_| ())
    }

    fn status(&self) -> typix_core::WatchStatus {
        self.client.request_status().unwrap();
        match self.wait(|e| matches!(e, WatchEvent::Status { .. })).unwrap() {
            WatchEvent::Status { status } => status,
            _ => unreachable!(),
        }
    }

    fn push(&self, t: PackageType, name: &str, f: Arc<MemFile>) -> Result<typix_core::PushResult, CoreError> {
        self.client.push_file(t, name.into(), f)?;
        match self.wait(|e| matches!(e, WatchEvent::PushFinished { .. }))? {
            WatchEvent::PushFinished { result } => Ok(result),
            _ => unreachable!(),
        }
    }

    fn pull(&self, path: &str) -> Result<Vec<u8>, CoreError> {
        let sink = Arc::new(MemSink::default());
        self.client.pull_file(path.into(), sink.clone())?;
        self.wait(|e| matches!(e, WatchEvent::PullFinished { .. }))?;
        assert_eq!(*sink.verdict.lock().unwrap(), Some(true));
        let data = sink.data.lock().unwrap().clone();
        Ok(data)
    }
}

fn file(n: usize, salt: u8) -> Arc<MemFile> {
    Arc::new(MemFile(
        (0..n).map(|i| ((i * 131 + i / 7) as u8) ^ salt).collect(),
    ))
}

fn sha(path: &str) -> String {
    exec(&format!("sha256sum {path}"))
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

#[test]
#[ignore = "needs the container from tests/e2e/otad-docker.sh"]
fn everything_against_the_real_daemon() {
    let mut phone = Phone::new();

    println!("-- pairing refused while closed");
    exec("/obj/w1a-otad-test -c 'arm'");
    assert_eq!(phone.pair("000000".into()), Err(CoreError::PairingNotOpen));

    println!("-- wrong code voids the pairing");
    let code = open_pairing();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    assert_eq!(phone.pair(wrong.into()), Err(CoreError::WrongCode));
    expect_state("last", "pairfail");
    expect_state("code", "-");

    println!("-- right code");
    let code = open_pairing();
    let paired = phone.pair(code).unwrap();
    let WatchEvent::Paired { watch_name, watch_fingerprint } = paired else {
        unreachable!()
    };
    assert_eq!(watch_name, "CMCC W1A");
    expect_state("last", "paired");
    let peer = watch_state("peer").unwrap();
    assert!(peer.starts_with(&phone.client.phone_fingerprint()), "{peer}");
    assert!(peer.ends_with("typix-e2e"), "{peer}");
    println!("   watch {watch_fingerprint}");

    println!("-- session and status");
    phone.session(tcp_profile()).unwrap();
    let s = phone.status();
    assert_eq!(s.protocol_version, 1);
    assert_eq!(s.ota_quota, 15 * 1024 * 1024);
    assert!(s.seconds_left > 1000, "{s:?}");
    assert!(!s.shell_enabled);
    assert!(s.wifi_ip.is_some());
    assert_eq!(s.battery_percent, None, "no gauge in a container");
    assert_eq!(phone.client.saved_host(), s.wifi_ip);

    println!("-- push over the TCP profile");
    let f = file(300_000, 1);
    let r = phone.push(PackageType::Script, "e2e.sh", f.clone()).unwrap();
    assert_eq!((r.seq, r.resumed_from), (1, 0));
    assert_eq!(r.message, "stored");
    assert_eq!(sha("/data/ota/scripts/e2e.sh"), r.sha256);
    assert_eq!(phone.status().raw["seq_script"], "1");

    println!("-- the watch's content check still applies");
    let got = phone.push(PackageType::Launcher, "w1a-shell", file(100, 2));
    assert!(
        matches!(&got, Err(CoreError::Remote { text, .. }) if text.contains("32-bit ARM ELF")),
        "{got:?}"
    );

    println!("-- replayed sequence number");
    phone
        .client
        .push_file_with_seq(PackageType::Script, "e2e.sh".into(), file(10, 3), 1)
        .unwrap();
    let got = phone.wait(|e| matches!(e, WatchEvent::PushFinished { .. }));
    assert!(
        matches!(&got, Err(CoreError::Remote { text, .. }) if text.contains("sequence")),
        "{got:?}"
    );

    println!("-- interrupted push over the BLE profile, resumed");
    phone.session(ble_profile()).unwrap();
    let f = file(200_000, 4);
    phone
        .client
        .push_file(PackageType::Script, "big.sh".into(), f.clone())
        .unwrap();
    phone
        .wait(|e| matches!(e, WatchEvent::Progress { done, .. } if *done > 40_000))
        .unwrap();
    phone.client.disconnect();
    phone.hang_up();
    std::thread::sleep(Duration::from_millis(500));
    let part: u64 = exec("wc -c < /data/ota/staging/current.part").trim().parse().unwrap();
    assert!(part >= 40_000, "{part}");
    assert_eq!(exec("ls /data/ota/scripts"), "e2e.sh\n", "nothing installed yet");

    phone.session(tcp_profile()).unwrap();
    let r = phone.push(PackageType::Script, "big.sh", f.clone()).unwrap();
    println!("   resumed from {} of {}", r.resumed_from, f.0.len());
    assert_eq!(r.resumed_from, part);
    assert_eq!(r.seq, 2);
    assert_eq!(sha("/data/ota/scripts/big.sh"), r.sha256);

    println!("-- abort deletes the part");
    phone
        .client
        .push_file(PackageType::Script, "dropped.sh".into(), file(2_000_000, 5))
        .unwrap();
    phone
        .wait(|e| matches!(e, WatchEvent::Progress { done, .. } if *done > 100_000))
        .unwrap();
    phone.client.abort_push().unwrap();
    phone.wait(|e| matches!(e, WatchEvent::PushAborted)).unwrap();
    assert_eq!(exec("ls /data/ota/staging"), "");
    assert_eq!(phone.status().raw["seq_script"], "2");

    println!("-- pull: an allowed file, verified");
    exec("seq 1 40000 > /tmp/w1a-otad.log");
    let sink = Arc::new(MemSink::default());
    phone.client.pull_log(LogKind::Otad, sink.clone()).unwrap();
    phone.wait(|e| matches!(e, WatchEvent::PullFinished { .. })).unwrap();
    assert_eq!(*sink.verdict.lock().unwrap(), Some(true));
    let want: Vec<u8> = (1..=40000).map(|n| format!("{n}\n")).collect::<String>().into_bytes();
    assert_eq!(*sink.data.lock().unwrap(), want);

    println!("-- pull: the audit log names this phone");
    let audit = phone.pull(LogKind::Audit.path()).unwrap();
    let audit = String::from_utf8(audit).unwrap();
    assert!(audit.contains("typix-e2e"), "{audit}");
    assert!(audit.contains("installed type=3 seq=2"), "{audit}");

    println!("-- pull: a BLE-sized read");
    phone.session(ble_profile()).unwrap();
    assert_eq!(phone.pull("/tmp/w1a-otad.log").unwrap(), want);

    println!("-- pull: the shell switch still guards the rest");
    let got = phone.pull("/etc/passwd");
    assert!(matches!(&got, Err(CoreError::Remote { code: 1, .. })), "{got:?}");

    println!("-- revoked");
    phone.client.disconnect();
    let fp = phone.client.phone_fingerprint();
    exec(&format!("/obj/w1a-otad-test -c 'revoke {fp}'"));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(phone.session(tcp_profile()), Err(CoreError::SessionRejected));
    phone.hang_up();
}
