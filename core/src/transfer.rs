//! Push and pull, as state machines fed one reply at a time.
//!
//! The watch answers every request in order, so a job only sees the OK bodies; the
//! engine handles errors and timeouts.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::CoreError;
use crate::model::{parse_lines, parse_seq, LinkProfile};
use crate::proto::{be32, op, PackageType};
use crate::sec::{hex, manifest, sign, NAME_MAX};

/// A request to send, and how long the watch may take over it.
pub(crate) struct Request {
    pub op: u8,
    pub body: Vec<u8>,
    pub timeout_ms: u64,
}

pub(crate) const REQUEST_MS: u64 = 60_000;
/// The watch hashes the whole file before it answers these.
pub(crate) const HASHING_MS: u64 = 120_000;

/// Where a push reads from.  Called on whichever thread drives the client, and for
/// chunks with the client's lock held: read and return, never call back into the client.
#[uniffi::export(with_foreign)]
pub trait ByteSource: Send + Sync {
    fn size(&self) -> Result<u64, CoreError>;
    /// Exactly `length` bytes at `offset`.
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, CoreError>;
}

/// Where a pull writes to.  Bytes arrive in order; nothing is trustworthy until
/// `finish(true)`, and `finish(false)` must discard what was written.  Called with the
/// client's lock held: never call back into the client from here.
#[uniffi::export(with_foreign)]
pub trait ByteSink: Send + Sync {
    fn write(&self, bytes: Vec<u8>) -> Result<(), CoreError>;
    fn finish(&self, verified: bool);
}

/// What a finished push left on the watch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PushResult {
    pub sha256: String,
    pub seq: u32,
    pub resumed_from: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PullResult {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Default)]
pub(crate) struct Step {
    pub requests: Vec<Request>,
    pub progress: Option<(u64, u64)>,
    pub resumed_from: Option<u64>,
    pub pushed: Option<PushResult>,
    pub pulled: Option<PullResult>,
}

pub(crate) fn check_name(package: PackageType, name: &str) -> Result<(), CoreError> {
    if name.is_empty() || name.len() > NAME_MAX || name.as_bytes().contains(&0) {
        return Err(CoreError::input("name must be 1 to 48 bytes"));
    }
    // safe_name() in otad.c: this one becomes a path under /data/ota/scripts.
    let script_ok = !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if package == PackageType::Script && !script_ok {
        return Err(CoreError::input("script names use letters, digits, . _ -"));
    }
    Ok(())
}

enum PushStage {
    WaitStatus,
    WaitBegin,
    Sending,
    WaitCommit,
}

pub(crate) struct PushJob {
    package: PackageType,
    name: String,
    source: Arc<dyn ByteSource>,
    profile: LinkProfile,
    seed: Zeroizing<[u8; 32]>,
    size: u32,
    sha: [u8; 32],
    seq: Option<u32>,
    stage: PushStage,
    resumed_from: u32,
    acked: u32,
    sent: u32,
    inflight: u32,
}

impl PushJob {
    /// Reads the whole source to hash it, so the signature covers what is on disk now.
    pub fn new(
        package: PackageType,
        name: String,
        source: Arc<dyn ByteSource>,
        profile: LinkProfile,
        seed: Zeroizing<[u8; 32]>,
        seq: Option<u32>,
    ) -> Result<Self, CoreError> {
        check_name(package, &name)?;
        if profile.chunk == 0 || profile.window == 0 {
            return Err(CoreError::input("link profile cannot be zero"));
        }
        let total = source.size()?;
        let size = u32::try_from(total).map_err(|_| CoreError::input("file over 4 GB"))?;
        let mut h = Sha256::new();
        let mut off = 0u64;
        while off < total {
            let want = (total - off).min(256 * 1024) as u32;
            let piece = source.read_at(off, want)?;
            if piece.len() != want as usize {
                return Err(CoreError::File {
                    detail: "short read".into(),
                });
            }
            h.update(&piece);
            off += piece.len() as u64;
        }
        Ok(Self {
            package,
            name,
            source,
            profile,
            seed,
            size,
            sha: h.finalize().into(),
            seq,
            stage: PushStage::WaitStatus,
            resumed_from: 0,
            acked: 0,
            sent: 0,
            inflight: 0,
        })
    }

    /// The sequence number comes from the watch, so a resumed push repeats the number
    /// of the part it left there and the watch recognises it.
    pub fn start(&mut self) -> Vec<Request> {
        match self.seq {
            Some(_) => self.begin(),
            None => vec![Request {
                op: op::STATUS,
                body: vec![],
                timeout_ms: REQUEST_MS,
            }],
        }
    }

    fn begin(&mut self) -> Vec<Request> {
        self.stage = PushStage::WaitBegin;
        let mut body = vec![self.package.wire()];
        body.extend_from_slice(&self.seq.unwrap_or(0).to_be_bytes());
        body.extend_from_slice(&self.size.to_be_bytes());
        body.extend_from_slice(&self.sha);
        body.extend_from_slice(self.name.as_bytes());
        vec![Request {
            op: op::PUT_BEGIN,
            body,
            timeout_ms: REQUEST_MS,
        }]
    }

    fn fill_window(&mut self) -> Result<Vec<Request>, CoreError> {
        let mut out = vec![];
        while self.inflight < self.profile.window && self.sent < self.size {
            let n = self.profile.chunk.min(self.size - self.sent);
            let bytes = self.source.read_at(self.sent as u64, n)?;
            if bytes.len() != n as usize {
                return Err(CoreError::File {
                    detail: "short read".into(),
                });
            }
            let mut body = self.sent.to_be_bytes().to_vec();
            body.extend_from_slice(&bytes);
            out.push(Request {
                op: op::PUT_DATA,
                body,
                timeout_ms: REQUEST_MS,
            });
            self.sent += n;
            self.inflight += 1;
        }
        Ok(out)
    }

    fn commit(&mut self) -> Vec<Request> {
        self.stage = PushStage::WaitCommit;
        let m = manifest(
            self.package.wire(),
            self.seq.unwrap_or(0),
            self.size,
            &self.sha,
            &self.name,
        );
        vec![Request {
            op: op::PUT_COMMIT,
            body: sign(&self.seed, &m).to_vec(),
            timeout_ms: HASHING_MS,
        }]
    }

    pub fn on_reply(&mut self, body: &[u8]) -> Result<Step, CoreError> {
        let mut step = Step::default();
        match self.stage {
            PushStage::WaitStatus => {
                let text = std::str::from_utf8(body)
                    .map_err(|_| CoreError::protocol("status is not UTF-8"))?;
                let last = parse_seq(&parse_lines(text), self.package);
                let seq = last
                    .checked_add(1)
                    .ok_or_else(|| CoreError::protocol("sequence number exhausted"))?;
                self.seq = Some(seq);
                step.requests = self.begin();
            }
            PushStage::WaitBegin => {
                let off = be32(body).ok_or_else(|| CoreError::protocol("short PUT_BEGIN"))?;
                if off > self.size {
                    return Err(CoreError::protocol("resume offset past the end"));
                }
                self.resumed_from = off;
                self.acked = off;
                self.sent = off;
                self.stage = PushStage::Sending;
                step.resumed_from = Some(off as u64);
                step.progress = Some((off as u64, self.size as u64));
                step.requests = self.next_requests()?;
            }
            PushStage::Sending => {
                let next = be32(body).ok_or_else(|| CoreError::protocol("short PUT_DATA"))?;
                if next <= self.acked || next > self.sent {
                    return Err(CoreError::protocol("the watch acked an unsent offset"));
                }
                self.acked = next;
                self.inflight -= 1;
                step.progress = Some((next as u64, self.size as u64));
                step.requests = self.next_requests()?;
            }
            PushStage::WaitCommit => {
                step.pushed = Some(PushResult {
                    sha256: hex(&self.sha),
                    seq: self.seq.unwrap_or(0),
                    resumed_from: self.resumed_from as u64,
                    message: String::from_utf8_lossy(body).into_owned(),
                });
            }
        }
        Ok(step)
    }

    fn next_requests(&mut self) -> Result<Vec<Request>, CoreError> {
        if self.acked == self.size {
            return Ok(self.commit());
        }
        self.fill_window()
    }
}

enum PullStage {
    WaitOpen,
    Reading,
}

pub(crate) struct PullJob {
    path: String,
    sink: Arc<dyn ByteSink>,
    pull: u32,
    stage: PullStage,
    size: u32,
    want: [u8; 32],
    got: u32,
    hash: Sha256,
    finished: bool,
}

impl PullJob {
    pub fn new(path: String, sink: Arc<dyn ByteSink>, pull: u32) -> Result<Self, CoreError> {
        if path.is_empty() || path.len() >= 256 || path.as_bytes().contains(&0) {
            return Err(CoreError::input("bad path"));
        }
        if pull == 0 {
            return Err(CoreError::input("link profile cannot be zero"));
        }
        Ok(Self {
            path,
            sink,
            pull,
            stage: PullStage::WaitOpen,
            size: 0,
            want: [0; 32],
            got: 0,
            hash: Sha256::new(),
            finished: false,
        })
    }

    pub fn start(&self) -> Vec<Request> {
        vec![Request {
            op: op::GET_OPEN,
            body: self.path.as_bytes().to_vec(),
            timeout_ms: HASHING_MS,
        }]
    }

    fn read(&self) -> Vec<Request> {
        let n = self.pull.min(self.size - self.got);
        let mut body = self.got.to_be_bytes().to_vec();
        body.extend_from_slice(&n.to_be_bytes());
        vec![Request {
            op: op::GET_READ,
            body,
            timeout_ms: REQUEST_MS,
        }]
    }

    fn finish(&mut self) -> Result<PullResult, CoreError> {
        let digest: [u8; 32] = std::mem::take(&mut self.hash).finalize().into();
        if digest != self.want {
            return Err(CoreError::HashMismatch);
        }
        self.finished = true;
        self.sink.finish(true);
        Ok(PullResult {
            path: self.path.clone(),
            size: self.size as u64,
            sha256: hex(&digest),
        })
    }

    pub fn on_reply(&mut self, body: &[u8]) -> Result<Step, CoreError> {
        let mut step = Step::default();
        match self.stage {
            PullStage::WaitOpen => {
                if body.len() != 36 {
                    return Err(CoreError::protocol("bad GET_OPEN reply"));
                }
                self.size = be32(body).ok_or_else(|| CoreError::protocol("short GET_OPEN"))?;
                self.want.copy_from_slice(&body[4..]);
                self.stage = PullStage::Reading;
                step.progress = Some((0, self.size as u64));
            }
            PullStage::Reading => {
                if body.is_empty() {
                    // The file shrank under us; the hash cannot match.
                    return Err(CoreError::HashMismatch);
                }
                if body.len() as u64 > (self.size - self.got) as u64 {
                    return Err(CoreError::protocol("more bytes than the file has"));
                }
                self.hash.update(body);
                self.sink.write(body.to_vec())?;
                self.got += body.len() as u32;
                step.progress = Some((self.got as u64, self.size as u64));
            }
        }
        if self.got == self.size {
            step.pulled = Some(self.finish()?);
        } else {
            step.requests = self.read();
        }
        Ok(step)
    }

    /// The job ended without a verified file.
    pub fn abandon(&mut self) {
        if !self.finished {
            self.finished = true;
            self.sink.finish(false);
        }
    }
}
