//! The connection state machine.  No I/O and no threads: bytes go in, `Output`s come
//! out, time arrives as a number.  `client.rs` connects it to the platform.

use std::sync::Arc;

use zeroize::Zeroizing;

use crate::cpace;
use crate::error::CoreError;
use crate::model::{parse_status, ConnectionState, LinkProfile, WatchStatus};
use crate::noise::{Initiator, Transport};
use crate::proto::{be32, encode_frame, hello, op, FrameDecoder};
use crate::proto::{MODE_PAIR, MODE_SESSION, SID_LEN};
use crate::sec::{fingerprint, random32, sign_public, x25519_public, Cipher};
use crate::transfer::{ByteSink, PullJob, PullResult, PushJob, PushResult};
use crate::transfer::{Request, Step, REQUEST_MS};

/// The Mac tool's wait for each handshake step.
const HANDSHAKE_MS: u64 = 10_000;
/// `pair_confirm` copies at most this much of the name.
const PEER_NAME_MAX: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WatchEvent {
    State {
        state: ConnectionState,
    },
    Paired {
        watch_fingerprint: String,
        watch_name: String,
    },
    Status {
        status: WatchStatus,
    },
    /// A push started; non-zero when the watch already had the first bytes.
    PushStarted {
        resumed_from: u64,
    },
    Progress {
        done: u64,
        total: u64,
    },
    PushFinished {
        result: PushResult,
    },
    PullFinished {
        result: PullResult,
    },
    PushAborted,
    Failed {
        error: CoreError,
    },
}

pub(crate) enum Output {
    Send(Vec<u8>),
    Close,
    Event(WatchEvent),
    /// Pin the key first; the event only goes out if that worked.
    SavePin { key: [u8; 32], event: WatchEvent },
    SaveHost(String),
}

pub(crate) struct Keys {
    pub identity: Zeroizing<[u8; 32]>,
    pub seed: Zeroizing<[u8; 32]>,
}

// One value per client, for the life of a connection: boxing would save nothing.
#[allow(clippy::large_enum_variant)]
enum Phase {
    Idle,
    PairWaitSid {
        code: Zeroizing<String>,
        name: Vec<u8>,
    },
    PairWaitShare {
        y_sk: Zeroizing<[u8; 32]>,
        ya: [u8; 32],
        sid: [u8; SID_LEN],
        name: Vec<u8>,
    },
    PairWaitConfirm {
        rx: Cipher,
    },
    SessionWaitReply {
        initiator: Initiator,
    },
    Ready {
        transport: Transport,
    },
}

enum Job {
    Status,
    Push(PushJob),
    Pull(PullJob),
    Abort,
}

pub(crate) struct Engine {
    keys: Keys,
    watch: Option<[u8; 32]>,
    phase: Phase,
    decoder: FrameDecoder,
    out: Vec<Output>,
    job: Option<Job>,
    /// Replies owed by the watch.  They come in order, so after a failed job the ones
    /// still in flight are recognised by this count and dropped.
    pending: u32,
    deadline: Option<u64>,
    /// The time of the call being handled.
    clock: u64,
    /// The last state the app was told about.  `on_frame` holds the phase out of
    /// `self` while it works, so changes are compared with this, not with `phase`.
    announced: ConnectionState,
    profile: LinkProfile,
}

fn whole_chars(name: &str, max: usize) -> Vec<u8> {
    let mut end = 0;
    for (i, c) in name.char_indices() {
        if i + c.len_utf8() > max {
            break;
        }
        end = i + c.len_utf8();
    }
    name.as_bytes()[..end].to_vec()
}

impl Engine {
    pub fn new(keys: Keys, watch: Option<[u8; 32]>, profile: LinkProfile) -> Self {
        Self {
            keys,
            watch,
            phase: Phase::Idle,
            decoder: FrameDecoder::default(),
            out: vec![],
            job: None,
            pending: 0,
            deadline: None,
            clock: 0,
            announced: ConnectionState::Idle,
            profile,
        }
    }

    pub fn take_outputs(&mut self) -> Vec<Output> {
        std::mem::take(&mut self.out)
    }

    pub fn identity_public(&self) -> [u8; 32] {
        x25519_public(&self.keys.identity)
    }

    pub fn watch(&self) -> Option<[u8; 32]> {
        self.watch
    }

    pub fn set_watch(&mut self, watch: Option<[u8; 32]>) {
        self.watch = watch;
    }

    pub fn state(&self) -> ConnectionState {
        match self.phase {
            Phase::Idle => ConnectionState::Idle,
            Phase::PairWaitSid { .. }
            | Phase::PairWaitShare { .. }
            | Phase::PairWaitConfirm { .. } => ConnectionState::Pairing,
            Phase::SessionWaitReply { .. } => ConnectionState::Handshaking,
            Phase::Ready { .. } => ConnectionState::Ready,
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.announce();
    }

    fn announce(&mut self) {
        let state = self.state();
        if state != self.announced {
            self.announced = state;
            self.out.push(Output::Event(WatchEvent::State { state }));
        }
    }

    fn send_frame(&mut self, payload: &[u8]) -> Result<(), CoreError> {
        self.out.push(Output::Send(encode_frame(payload)?));
        Ok(())
    }

    fn emit(&mut self, event: WatchEvent) {
        self.out.push(Output::Event(event));
    }

    fn require_idle(&self) -> Result<(), CoreError> {
        match self.phase {
            Phase::Idle => Ok(()),
            _ => Err(CoreError::Busy),
        }
    }

    /// The link is open; the watch expects the hello first.
    pub fn begin_pairing(
        &mut self,
        code: &str,
        phone_name: &str,
        profile: LinkProfile,
        now: u64,
    ) -> Result<(), CoreError> {
        self.require_idle()?;
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CoreError::InvalidCode);
        }
        self.reset_link(profile);
        self.send_frame(&hello(MODE_PAIR))?;
        self.set_phase(Phase::PairWaitSid {
            code: Zeroizing::new(code.to_string()),
            name: whole_chars(phone_name, PEER_NAME_MAX),
        });
        self.deadline = Some(now + HANDSHAKE_MS);
        Ok(())
    }

    pub fn begin_session(&mut self, profile: LinkProfile, now: u64) -> Result<(), CoreError> {
        self.require_idle()?;
        let watch = self.watch.ok_or(CoreError::NotPaired)?;
        self.reset_link(profile);
        let h = hello(MODE_SESSION);
        let mut initiator = Initiator::new(&self.keys.identity, &watch, &h)?;
        let m1 = initiator.write1()?;
        self.send_frame(&h)?;
        self.send_frame(&m1)?;
        self.set_phase(Phase::SessionWaitReply { initiator });
        self.deadline = Some(now + HANDSHAKE_MS);
        Ok(())
    }

    fn reset_link(&mut self, profile: LinkProfile) {
        self.profile = profile;
        self.decoder.reset();
        self.pending = 0;
        self.deadline = None;
    }

    pub fn on_bytes(&mut self, bytes: &[u8], now: u64) {
        if matches!(self.phase, Phase::Idle) {
            return;
        }
        self.clock = now;
        self.decoder.push(bytes);
        while let Some(frame) = self.decoder.next_frame() {
            if let Err(e) = self.on_frame(&frame, now) {
                self.fail_link(e);
                return;
            }
            if matches!(self.phase, Phase::Idle) {
                return;
            }
        }
    }

    /// What a close means depends on where it lands: the watch answers most refusals
    /// by closing.
    pub fn on_closed(&mut self) {
        let error = match self.phase {
            Phase::Idle => return,
            Phase::PairWaitSid { .. } => CoreError::PairingNotOpen,
            Phase::PairWaitConfirm { .. } => CoreError::WrongCode,
            Phase::SessionWaitReply { .. } => CoreError::SessionRejected,
            Phase::PairWaitShare { .. } | Phase::Ready { .. } => CoreError::LinkClosed,
        };
        self.end_link(error, false);
    }

    /// Ends the link at the app's request.  An unfinished push stays on the watch and
    /// resumes next time.
    pub fn disconnect(&mut self) {
        if matches!(self.phase, Phase::Idle) {
            return;
        }
        if self.job.is_some() {
            self.end_link(CoreError::LinkClosed, true);
            return;
        }
        self.deadline = None;
        self.pending = 0;
        self.set_phase(Phase::Idle);
        self.out.push(Output::Close);
    }

    pub fn tick(&mut self, now: u64) {
        if self.deadline.is_some_and(|d| now >= d) {
            self.fail_link(CoreError::Timeout);
        }
    }

    fn fail_link(&mut self, error: CoreError) {
        self.end_link(error, true);
    }

    fn end_link(&mut self, error: CoreError, close: bool) {
        self.abandon_job();
        self.deadline = None;
        self.pending = 0;
        self.set_phase(Phase::Idle);
        if close {
            self.out.push(Output::Close);
        }
        self.emit(WatchEvent::Failed { error });
    }

    fn abandon_job(&mut self) {
        if let Some(Job::Pull(p)) = self.job.as_mut() {
            p.abandon();
        }
        self.job = None;
    }

    fn on_frame(&mut self, frame: &[u8], now: u64) -> Result<(), CoreError> {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => Ok(()),
            Phase::PairWaitSid { code, name } => {
                let sid: [u8; SID_LEN] = frame
                    .try_into()
                    .map_err(|_| CoreError::protocol("bad sid"))?;
                let y_sk = random32()?;
                let ya = cpace::share(&y_sk, &code, &sid);
                self.send_frame(&ya)?;
                self.phase = Phase::PairWaitShare {
                    y_sk,
                    ya,
                    sid,
                    name,
                };
                self.deadline = Some(now + HANDSHAKE_MS);
                Ok(())
            }
            Phase::PairWaitShare {
                y_sk,
                ya,
                sid,
                name,
            } => {
                let yb: [u8; 32] = frame
                    .try_into()
                    .map_err(|_| CoreError::protocol("bad share"))?;
                let (mut tx, rx) = cpace::finish(&y_sk, &yb, &ya, &yb, &sid)?;
                let mut body = Vec::with_capacity(64 + name.len());
                body.extend_from_slice(&self.identity_public());
                body.extend_from_slice(&sign_public(&self.keys.seed));
                body.extend_from_slice(&name);
                let sealed = tx.seal(&body)?;
                self.send_frame(&sealed)?;
                self.phase = Phase::PairWaitConfirm { rx };
                self.deadline = Some(now + HANDSHAKE_MS);
                Ok(())
            }
            Phase::PairWaitConfirm { mut rx } => {
                let body = rx.open(frame).ok_or(CoreError::WrongCode)?;
                let key: [u8; 32] = body
                    .get(..32)
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| CoreError::protocol("short pairing reply"))?;
                let watch_name = String::from_utf8_lossy(&body[32..])
                    .trim_end_matches('\0')
                    .to_string();
                self.watch = Some(key);
                self.deadline = None;
                // This link has done its job; sessions use a new one.
                self.announce();
                self.out.push(Output::Close);
                self.out.push(Output::SavePin {
                    key,
                    event: WatchEvent::Paired {
                        watch_fingerprint: fingerprint(&key),
                        watch_name,
                    },
                });
                Ok(())
            }
            Phase::SessionWaitReply { initiator } => {
                let transport = initiator.read2(frame)?;
                self.deadline = None;
                self.set_phase(Phase::Ready { transport });
                Ok(())
            }
            Phase::Ready { mut transport } => {
                let pt = transport
                    .open(frame)
                    .ok_or_else(|| CoreError::protocol("record failed to authenticate"))?;
                self.phase = Phase::Ready { transport };
                self.on_record(&pt, now)
            }
        }
    }

    fn on_record(&mut self, pt: &[u8], now: u64) -> Result<(), CoreError> {
        let (&code, body) = pt
            .split_first()
            .ok_or_else(|| CoreError::protocol("empty record"))?;
        if code != op::REPLY_OK && code != op::REPLY_ERR {
            return Err(CoreError::protocol(format!("unexpected op {code:#x}")));
        }
        self.pending = self
            .pending
            .checked_sub(1)
            .ok_or_else(|| CoreError::protocol("a reply nobody asked for"))?;
        self.deadline = (self.pending > 0).then_some(now + REQUEST_MS);

        match self.job {
            None => Ok(()),
            // Whatever was in flight lands first; the last reply is the abort's.
            Some(Job::Abort) => {
                if self.pending == 0 {
                    self.job = None;
                    match reply(code, body) {
                        Ok(_) => self.emit(WatchEvent::PushAborted),
                        Err(error) => self.emit(WatchEvent::Failed { error }),
                    }
                }
                Ok(())
            }
            Some(_) => {
                let result = reply(code, body).and_then(|b| self.advance(b));
                match result {
                    Ok(()) => {}
                    Err(e @ CoreError::Protocol { .. }) => return Err(e),
                    Err(error) => {
                        // A refusal ends the job, not the session.
                        self.abandon_job();
                        self.emit(WatchEvent::Failed { error });
                    }
                }
                Ok(())
            }
        }
    }

    fn advance(&mut self, body: &[u8]) -> Result<(), CoreError> {
        let step = match self.job.as_mut() {
            Some(Job::Status) => {
                let status = parse_status(body)?;
                self.job = None;
                self.finish_status(status);
                return Ok(());
            }
            Some(Job::Push(p)) => p.on_reply(body)?,
            Some(Job::Pull(p)) => p.on_reply(body)?,
            _ => return Ok(()),
        };
        self.apply(step)
    }

    fn finish_status(&mut self, status: WatchStatus) {
        if let Some(ip) = &status.wifi_ip {
            self.out.push(Output::SaveHost(ip.clone()));
        }
        self.emit(WatchEvent::Status { status });
    }

    fn apply(&mut self, step: Step) -> Result<(), CoreError> {
        if let Some(resumed_from) = step.resumed_from {
            self.emit(WatchEvent::PushStarted { resumed_from });
        }
        if let Some((done, total)) = step.progress {
            self.emit(WatchEvent::Progress { done, total });
        }
        if let Some(result) = step.pushed {
            self.job = None;
            self.emit(WatchEvent::PushFinished { result });
        }
        if let Some(result) = step.pulled {
            self.job = None;
            self.emit(WatchEvent::PullFinished { result });
        }
        self.send_requests(step.requests)
    }

    fn send_requests(&mut self, requests: Vec<Request>) -> Result<(), CoreError> {
        for r in requests {
            let mut pt = Vec::with_capacity(1 + r.body.len());
            pt.push(r.op);
            pt.extend_from_slice(&r.body);
            let Phase::Ready { transport } = &mut self.phase else {
                return Err(CoreError::NotConnected);
            };
            let sealed = transport.seal(&pt)?;
            self.send_frame(&sealed)?;
            self.pending += 1;
            self.deadline = Some(self.clock + r.timeout_ms);
        }
        Ok(())
    }

    fn start_job(&mut self, job: Job, requests: Vec<Request>, now: u64) -> Result<(), CoreError> {
        self.clock = now;
        self.job = Some(job);
        if let Err(e) = self.send_requests(requests) {
            self.abandon_job();
            return Err(e);
        }
        Ok(())
    }

    fn require_free(&self) -> Result<(), CoreError> {
        if !matches!(self.phase, Phase::Ready { .. }) {
            return Err(CoreError::NotConnected);
        }
        if self.job.is_some() || self.pending > 0 {
            return Err(CoreError::Busy);
        }
        Ok(())
    }

    pub fn request_status(&mut self, now: u64) -> Result<(), CoreError> {
        self.require_free()?;
        let r = Request {
            op: op::STATUS,
            body: vec![],
            timeout_ms: REQUEST_MS,
        };
        self.start_job(Job::Status, vec![r], now)
    }

    /// What building a `PushJob` needs.  The job hashes the whole file, so the client
    /// builds it outside the lock and hands it to `start_push`.
    pub(crate) fn push_context(&self) -> Result<(LinkProfile, Zeroizing<[u8; 32]>), CoreError> {
        self.require_free()?;
        Ok((self.profile, Zeroizing::new(*self.keys.seed)))
    }

    pub(crate) fn start_push(&mut self, mut job: PushJob, now: u64) -> Result<(), CoreError> {
        self.require_free()?;
        let requests = job.start();
        self.start_job(Job::Push(job), requests, now)
    }

    /// Drops the part on the watch.  To stop and resume later, disconnect instead.
    pub fn abort_push(&mut self, now: u64) -> Result<(), CoreError> {
        if !matches!(self.phase, Phase::Ready { .. }) {
            return Err(CoreError::NotConnected);
        }
        if !matches!(self.job, Some(Job::Push(_))) {
            return Err(CoreError::input("no push is running"));
        }
        self.clock = now;
        self.job = Some(Job::Abort);
        self.send_requests(vec![Request {
            op: op::PUT_ABORT,
            body: vec![],
            timeout_ms: REQUEST_MS,
        }])
    }

    pub fn pull(
        &mut self,
        path: String,
        sink: Arc<dyn ByteSink>,
        now: u64,
    ) -> Result<(), CoreError> {
        self.require_free()?;
        let job = PullJob::new(path, sink, self.profile.pull)?;
        let requests = job.start();
        self.start_job(Job::Pull(job), requests, now)
    }
}

fn reply(code: u8, body: &[u8]) -> Result<&[u8], CoreError> {
    if code == op::REPLY_OK {
        return Ok(body);
    }
    let errno = be32(body).ok_or_else(|| CoreError::protocol("short error reply"))?;
    Err(CoreError::Remote {
        code: errno as i32,
        text: String::from_utf8_lossy(&body[4..]).into_owned(),
    })
}
