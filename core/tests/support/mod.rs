//! An in-memory watch and the plumbing around a `WatchClient`.
//!
//! `MockWatch` follows `otad.c` where the client can tell the difference: it refuses by
//! closing, resumes only an identical package, and answers in order.  It exists to reach
//! failures that are awkward to stage against the real daemon, which `e2e_otad.rs` drives.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use typix_core::proto::{op, FrameDecoder, MODE_PAIR, MODE_SESSION};
use typix_core::sec::{manifest, random32, sha256, x25519_public, Cipher};
use typix_core::{
    cpace, ByteSink, ByteSource, ConnectionState, CoreError, SecureStore, WatchClient,
    WatchEvent, WatchEventListener, WatchLink,
};

const EALREADY: i32 = 114;
const ESPIPE: i32 = 29;
const EBADMSG: i32 = 74;
const EPERM: i32 = 1;
const ENOENT: i32 = 2;

enum St {
    Hello,
    PairShare,
    PairConfirm,
    Ik1,
    Ready,
}

#[derive(Clone, PartialEq)]
struct Meta {
    package: u8,
    seq: u32,
    size: u32,
    sha: [u8; 32],
    name: String,
}

pub struct MockWatch {
    identity: [u8; 32],
    pub code: Option<String>,
    /// x25519 key to Ed25519 key.
    pub peers: HashMap<[u8; 32], [u8; 32]>,
    pub peer_names: Vec<String>,
    st: St,
    decoder: FrameDecoder,
    sid: [u8; 16],
    ya: [u8; 32],
    yb: [u8; 32],
    tx: Option<Cipher>,
    rx: Option<Cipher>,
    hs: Option<snow::HandshakeState>,
    transport: Option<snow::TransportState>,
    peer: Option<[u8; 32]>,
    pub out: Vec<u8>,
    pub closed: bool,
    pub seq: [u32; 5],
    meta: Option<Meta>,
    pub part: Vec<u8>,
    active: bool,
    pub installed: Vec<(String, Vec<u8>)>,
    pub files: HashMap<String, Vec<u8>>,
    /// Handing back different bytes from what GET_OPEN hashed.
    pub corrupt_reads: bool,
    /// Close the link after this many more PUT_DATA records.
    pub drop_after_data: Option<u32>,
    pub silent: bool,
    pub seen_put_data: u32,
    pub max_inflight_chunk: usize,
}

impl MockWatch {
    pub fn new() -> Self {
        Self {
            identity: *random32().unwrap(),
            code: None,
            peers: HashMap::new(),
            peer_names: vec![],
            st: St::Hello,
            decoder: FrameDecoder::default(),
            sid: [0; 16],
            ya: [0; 32],
            yb: [0; 32],
            tx: None,
            rx: None,
            hs: None,
            transport: None,
            peer: None,
            out: vec![],
            closed: false,
            seq: [0; 5],
            meta: None,
            part: vec![],
            active: false,
            installed: vec![],
            files: HashMap::new(),
            corrupt_reads: false,
            drop_after_data: None,
            silent: false,
            seen_put_data: 0,
            max_inflight_chunk: 0,
        }
    }

    pub fn public(&self) -> [u8; 32] {
        x25519_public(&self.identity)
    }

    /// What a new TCP accept resets.  The part, peers and sequence numbers stay.
    pub fn new_connection(&mut self) {
        self.st = St::Hello;
        self.decoder.reset();
        self.closed = false;
        self.out.clear();
        self.transport = None;
        self.hs = None;
        self.active = false;
    }

    fn frame(&mut self, p: &[u8]) {
        self.out
            .extend_from_slice(&(p.len() as u16).to_be_bytes());
        self.out.extend_from_slice(p);
    }

    fn send_op(&mut self, code: u8, body: &[u8]) {
        let mut pt = vec![code];
        pt.extend_from_slice(body);
        let mut ct = vec![0u8; pt.len() + 16];
        let n = self
            .transport
            .as_mut()
            .unwrap()
            .write_message(&pt, &mut ct)
            .unwrap();
        ct.truncate(n);
        self.frame(&ct);
    }

    fn ok(&mut self, body: &[u8]) {
        self.send_op(op::REPLY_OK, body);
    }

    fn err(&mut self, code: i32, text: &str) {
        let mut b = code.to_be_bytes().to_vec();
        b.extend_from_slice(text.as_bytes());
        self.send_op(op::REPLY_ERR, &b);
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if self.closed || self.silent {
            return;
        }
        self.decoder.push(bytes);
        while !self.closed {
            let Some(f) = self.decoder.next_frame() else {
                break;
            };
            self.on_frame(&f);
        }
    }

    fn on_frame(&mut self, f: &[u8]) {
        match self.st {
            St::Hello => {
                if f.len() != 6 || &f[..4] != b"W1AO" || f[4] != 1 {
                    self.closed = true;
                } else if f[5] == MODE_PAIR {
                    if self.code.is_none() {
                        self.closed = true;
                        return;
                    }
                    self.sid = *<&[u8; 16]>::try_from(&random32().unwrap()[..16]).unwrap();
                    let sid = self.sid;
                    self.frame(&sid);
                    self.st = St::PairShare;
                } else if f[5] == MODE_SESSION {
                    let e = random32().unwrap();
                    self.hs = Some(
                        snow::Builder::new(typix_core::noise::PATTERN.parse().unwrap())
                            .local_private_key(&self.identity)
                            .unwrap()
                            .fixed_ephemeral_key_for_testing_only(&e[..])
                            .prologue(f)
                            .unwrap()
                            .build_responder()
                            .unwrap(),
                    );
                    self.st = St::Ik1;
                } else {
                    self.closed = true;
                }
            }
            St::PairShare => {
                self.ya.copy_from_slice(f);
                let y = random32().unwrap();
                let code = self.code.clone().unwrap();
                self.yb = cpace::share(&y, &code, &self.sid);
                let (rx, tx) = cpace::finish(&y, &self.ya, &self.ya, &self.yb, &self.sid).unwrap();
                self.rx = Some(rx);
                self.tx = Some(tx);
                let yb = self.yb;
                self.frame(&yb);
                self.st = St::PairConfirm;
            }
            St::PairConfirm => {
                let Some(pt) = self.rx.as_mut().unwrap().open(f) else {
                    self.code = None; // one guess per button press
                    self.closed = true;
                    return;
                };
                let x: [u8; 32] = pt[..32].try_into().unwrap();
                let ed: [u8; 32] = pt[32..64].try_into().unwrap();
                self.peers.insert(x, ed);
                self.peer_names
                    .push(String::from_utf8(pt[64..].to_vec()).unwrap());
                let mut body = [0u8; 48];
                body[..32].copy_from_slice(&self.public());
                body[32..40].copy_from_slice(b"CMCC W1A");
                let reply = self.tx.as_mut().unwrap().seal(&body).unwrap();
                self.frame(&reply);
                self.code = None;
            }
            St::Ik1 => {
                let mut hs = self.hs.take().unwrap();
                let mut p = [0u8; 64];
                if f.len() != 96 || hs.read_message(f, &mut p).is_err() {
                    self.closed = true;
                    return;
                }
                let rs: [u8; 32] = hs.get_remote_static().unwrap().try_into().unwrap();
                if !self.peers.contains_key(&rs) {
                    self.closed = true;
                    return;
                }
                let mut m2 = [0u8; 48];
                let n = hs.write_message(&[], &mut m2).unwrap();
                self.frame(&m2[..n]);
                self.transport = Some(hs.into_transport_mode().unwrap());
                self.peer = Some(rs);
                self.st = St::Ready;
            }
            St::Ready => {
                let mut pt = vec![0u8; f.len()];
                let Ok(n) = self.transport.as_mut().unwrap().read_message(f, &mut pt) else {
                    self.closed = true;
                    return;
                };
                pt.truncate(n);
                self.record(pt[0], &pt[1..]);
            }
        }
    }

    fn record(&mut self, code: u8, b: &[u8]) {
        match code {
            op::PING => self.ok(&[]),
            op::STATUS => {
                let text = format!(
                    "version=1\nport=7423\nleft=1700\nshell=0\nwifi_ip=192.0.2.7\nbattery=64\n\
                     wall=1000\nseq_launcher={}\nseq_assets={}\nseq_script={}\nseq_image={}\n\
                     data_free=9000\nota_used=1\nota_quota=15728640\ntrial=0\n",
                    self.seq[1], self.seq[2], self.seq[3], self.seq[4]
                );
                self.ok(text.as_bytes());
            }
            op::PUT_BEGIN => self.put_begin(b),
            op::PUT_DATA => self.put_data(b),
            op::PUT_COMMIT => self.put_commit(b),
            op::PUT_ABORT => {
                self.part.clear();
                self.meta = None;
                self.active = false;
                self.ok(&[]);
            }
            op::GET_OPEN => {
                let path = String::from_utf8(b.to_vec()).unwrap();
                match self.files.get(&path).cloned() {
                    None => self.err(ENOENT, "no such file"),
                    Some(data) => {
                        self.files.insert("@open".into(), data.clone());
                        let mut r = (data.len() as u32).to_be_bytes().to_vec();
                        r.extend_from_slice(&sha256(&data));
                        self.ok(&r);
                    }
                }
            }
            op::GET_READ => {
                let off = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
                let want = u32::from_be_bytes(b[4..8].try_into().unwrap()) as usize;
                let data = self.files["@open"].clone();
                let end = (off + want.min(32 * 1024)).min(data.len());
                let mut piece = data[off.min(end)..end].to_vec();
                if self.corrupt_reads && !piece.is_empty() {
                    piece[0] ^= 1;
                }
                self.ok(&piece);
            }
            _ => self.err(38, "unknown op"),
        }
    }

    fn put_begin(&mut self, b: &[u8]) {
        let m = Meta {
            package: b[0],
            seq: u32::from_be_bytes(b[1..5].try_into().unwrap()),
            size: u32::from_be_bytes(b[5..9].try_into().unwrap()),
            sha: b[9..41].try_into().unwrap(),
            name: String::from_utf8(b[41..].to_vec()).unwrap(),
        };
        if m.seq <= self.seq[m.package as usize] {
            self.err(EALREADY, "sequence number already used");
            return;
        }
        if self.meta.as_ref() != Some(&m) {
            self.part.clear();
        }
        self.meta = Some(m);
        self.active = true;
        let have = self.part.len() as u32;
        self.ok(&have.to_be_bytes());
    }

    fn put_data(&mut self, b: &[u8]) {
        self.seen_put_data += 1;
        self.max_inflight_chunk = self.max_inflight_chunk.max(b.len() - 4);
        let off = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
        if !self.active || off != self.part.len() {
            self.err(ESPIPE, "wrong offset");
            return;
        }
        self.part.extend_from_slice(&b[4..]);
        let n = self.part.len() as u32;
        self.ok(&n.to_be_bytes());
        if let Some(left) = self.drop_after_data {
            if left <= 1 {
                self.drop_after_data = None;
                self.closed = true;
            } else {
                self.drop_after_data = Some(left - 1);
            }
        }
    }

    fn put_commit(&mut self, sig: &[u8]) {
        let m = self.meta.clone().unwrap();
        if sha256(&self.part) != m.sha {
            self.err(EBADMSG, "sha256 mismatch");
            return;
        }
        let bytes = manifest(m.package, m.seq, m.size, &m.sha, &m.name);
        let ed = self.peers[&self.peer.unwrap()];
        let key = VerifyingKey::from_bytes(&ed).unwrap();
        let sig = Signature::from_bytes(sig.try_into().unwrap());
        if key.verify(&bytes, &sig).is_err() {
            self.err(EPERM, "bad signature");
            return;
        }
        self.seq[m.package as usize] = m.seq;
        self.installed
            .push((m.name.clone(), std::mem::take(&mut self.part)));
        self.meta = None;
        self.active = false;
        self.ok(b"stored");
    }
}

#[derive(Default)]
pub struct MemLink {
    pub outbox: Mutex<VecDeque<Vec<u8>>>,
    pub closes: Mutex<u32>,
}

impl WatchLink for MemLink {
    fn send(&self, bytes: Vec<u8>) {
        self.outbox.lock().unwrap().push_back(bytes);
    }
    fn close_link(&self) {
        *self.closes.lock().unwrap() += 1;
    }
}

#[derive(Default)]
pub struct MemStore(pub Mutex<HashMap<String, Vec<u8>>>);

impl SecureStore for MemStore {
    fn load(&self, name: String) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self.0.lock().unwrap().get(&name).cloned())
    }
    fn save(&self, name: String, value: Vec<u8>) -> Result<(), CoreError> {
        self.0.lock().unwrap().insert(name, value);
        Ok(())
    }
    fn remove(&self, name: String) -> Result<(), CoreError> {
        self.0.lock().unwrap().remove(&name);
        Ok(())
    }
}

#[derive(Default)]
pub struct Events(pub Mutex<Vec<WatchEvent>>);

impl WatchEventListener for Events {
    fn on_event(&self, event: WatchEvent) {
        self.0.lock().unwrap().push(event);
    }
}

pub struct MemFile(pub Vec<u8>);

impl ByteSource for MemFile {
    fn size(&self) -> Result<u64, CoreError> {
        Ok(self.0.len() as u64)
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, CoreError> {
        Ok(self.0[offset as usize..offset as usize + length as usize].to_vec())
    }
}

#[derive(Default)]
pub struct MemSink {
    pub data: Mutex<Vec<u8>>,
    pub verdict: Mutex<Option<bool>>,
}

impl ByteSink for MemSink {
    fn write(&self, bytes: Vec<u8>) -> Result<(), CoreError> {
        self.data.lock().unwrap().extend(bytes);
        Ok(())
    }
    fn finish(&self, verified: bool) {
        *self.verdict.lock().unwrap() = Some(verified);
    }
}

/// A client and a watch, joined by a link that cuts the stream at `mtu` bytes.
pub struct Rig {
    pub watch: MockWatch,
    pub client: Arc<WatchClient>,
    pub store: Arc<MemStore>,
    pub events: Arc<Events>,
    pub link: Arc<MemLink>,
    pub mtu: usize,
}

impl Rig {
    pub fn new() -> Self {
        let store = Arc::new(MemStore::default());
        let events = Arc::new(Events::default());
        let client = WatchClient::new(store.clone(), events.clone()).unwrap();
        Self {
            watch: MockWatch::new(),
            client,
            store,
            events,
            link: Arc::new(MemLink::default()),
            mtu: 185,
        }
    }

    /// Moves bytes both ways until neither side has anything to say.
    pub fn pump(&mut self) {
        loop {
            let mut moved = false;
            let queued: Vec<Vec<u8>> = self.link.outbox.lock().unwrap().drain(..).collect();
            for b in queued {
                moved = true;
                for piece in b.chunks(self.mtu) {
                    self.watch.feed(piece);
                }
            }
            let out = std::mem::take(&mut self.watch.out);
            for piece in out.chunks(self.mtu) {
                moved = true;
                self.client.on_bytes(piece.to_vec());
            }
            if self.watch.closed {
                self.client.on_link_closed();
                return;
            }
            if !moved {
                return;
            }
        }
    }

    pub fn taken(&self) -> Vec<WatchEvent> {
        std::mem::take(&mut *self.events.0.lock().unwrap())
    }

    pub fn failure(&self) -> Option<CoreError> {
        self.events.0.lock().unwrap().iter().find_map(|e| match e {
            WatchEvent::Failed { error } => Some(error.clone()),
            _ => None,
        })
    }

    pub fn pair(&mut self) {
        self.watch.code = Some("482913".into());
        self.client
            .begin_pairing(
                self.link.clone(),
                typix_core::tcp_profile(),
                "482913".into(),
                "Test Phone".into(),
            )
            .unwrap();
        self.pump();
        self.watch.new_connection();
        assert!(self.client.is_paired(), "{:?}", self.taken());
    }

    pub fn connect(&mut self, profile: typix_core::LinkProfile) {
        // A new connection: what was queued on the old one is gone with it.
        self.link.outbox.lock().unwrap().clear();
        self.watch.new_connection();
        self.client.begin_session(self.link.clone(), profile).unwrap();
        self.pump();
        assert_eq!(self.client.state(), ConnectionState::Ready);
    }
}
