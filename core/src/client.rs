//! The object the apps hold.  Locks the engine and hands its outputs to the platform.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use zeroize::Zeroizing;

use crate::engine::{Engine, Keys, Output, WatchEvent};
use crate::error::CoreError;
use crate::model::{tcp_profile, ConnectionState, LinkProfile};
use crate::proto::{LogKind, PackageType};
use crate::sec::{fingerprint, random32};
use crate::transfer::{ByteSink, ByteSource, PushJob};

const KEY_IDENTITY: &str = "identity.key";
const KEY_SIGNING: &str = "signing.seed";
const KEY_WATCH: &str = "watch.pub";
const KEY_HOST: &str = "host";

/// A byte pipe to the watch.  TCP and BLE are both this.
///
/// `send` queues and returns: a BLE implementation cuts the bytes to the MTU and waits
/// for write credit itself.  Bytes must reach the watch in call order.  Deliver what
/// arrives with `on_bytes`, and a lost link with `on_link_closed`.
#[uniffi::export(with_foreign)]
pub trait WatchLink: Send + Sync {
    fn send(&self, bytes: Vec<u8>);
    /// Not `close`: the Kotlin bindings put a `close()` on every interface already.
    fn close_link(&self);
}

/// Device-bound secret storage: iOS Keychain, Android Keystore.  Holds the two long-term
/// private keys, which equal root on the watch, so implementations must not sync or back
/// them up.
#[uniffi::export(with_foreign)]
pub trait SecureStore: Send + Sync {
    fn load(&self, name: String) -> Result<Option<Vec<u8>>, CoreError>;
    fn save(&self, name: String, value: Vec<u8>) -> Result<(), CoreError>;
    fn remove(&self, name: String) -> Result<(), CoreError>;
}

#[uniffi::export(with_foreign)]
pub trait WatchEventListener: Send + Sync {
    fn on_event(&self, event: WatchEvent);
}

struct Inner {
    engine: Engine,
    link: Option<Arc<dyn WatchLink>>,
    queue: VecDeque<Output>,
    dispatching: bool,
}

#[derive(uniffi::Object)]
pub struct WatchClient {
    inner: Mutex<Inner>,
    store: Arc<dyn SecureStore>,
    events: Arc<dyn WatchEventListener>,
    started: Instant,
}

fn load_or_create(
    store: &Arc<dyn SecureStore>,
    name: &str,
) -> Result<Zeroizing<[u8; 32]>, CoreError> {
    if let Some(v) = store.load(name.to_string())? {
        let v = Zeroizing::new(v);
        let mut key = Zeroizing::new([0u8; 32]);
        if v.len() != key.len() {
            return Err(CoreError::Storage {
                detail: format!("{name} has the wrong length"),
            });
        }
        key.copy_from_slice(&v);
        return Ok(key);
    }
    let key = random32()?;
    store.save(name.to_string(), key.to_vec())?;
    Ok(key)
}

#[uniffi::export]
impl WatchClient {
    /// Loads the keys, creating them on first use.
    #[uniffi::constructor]
    pub fn new(
        store: Arc<dyn SecureStore>,
        events: Arc<dyn WatchEventListener>,
    ) -> Result<Arc<Self>, CoreError> {
        let keys = Keys {
            identity: load_or_create(&store, KEY_IDENTITY)?,
            seed: load_or_create(&store, KEY_SIGNING)?,
        };
        let watch = match store.load(KEY_WATCH.to_string())? {
            Some(v) => Some(<[u8; 32]>::try_from(v.as_slice()).map_err(|_| {
                CoreError::Storage {
                    detail: "watch.pub has the wrong length".into(),
                }
            })?),
            None => None,
        };
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                engine: Engine::new(keys, watch, tcp_profile()),
                link: None,
                queue: VecDeque::new(),
                dispatching: false,
            }),
            store,
            events,
            started: Instant::now(),
        }))
    }

    pub fn is_paired(&self) -> bool {
        self.lock().engine.watch().is_some()
    }

    /// This phone's identity, as the watch lists it and can revoke it.
    pub fn phone_fingerprint(&self) -> String {
        fingerprint(&self.lock().engine.identity_public())
    }

    pub fn watch_fingerprint(&self) -> Option<String> {
        self.lock().engine.watch().as_ref().map(fingerprint)
    }

    /// The Wi-Fi address the watch last reported, worth trying before BLE.
    pub fn saved_host(&self) -> Option<String> {
        let v = self.store.load(KEY_HOST.to_string()).ok().flatten()?;
        String::from_utf8(v).ok().filter(|s| !s.is_empty())
    }

    pub fn state(&self) -> ConnectionState {
        self.lock().engine.state()
    }

    /// Forgets the pinned watch.  Keeps this phone's keys; revoke on the watch separately.
    pub fn forget_watch(&self) -> Result<(), CoreError> {
        self.store.remove(KEY_WATCH.to_string())?;
        self.store.remove(KEY_HOST.to_string())?;
        self.lock().engine.set_watch(None);
        Ok(())
    }

    /// `link` must be open.  `phone_name` is what the watch lists, cut to 32 bytes.
    pub fn begin_pairing(
        &self,
        link: Arc<dyn WatchLink>,
        profile: LinkProfile,
        code: String,
        phone_name: String,
    ) -> Result<(), CoreError> {
        let now = self.elapsed_ms();
        self.drive(Some(link), |e| {
            e.begin_pairing(&code, &phone_name, profile, now)
        })
    }

    /// `link` must be open.
    pub fn begin_session(
        &self,
        link: Arc<dyn WatchLink>,
        profile: LinkProfile,
    ) -> Result<(), CoreError> {
        let now = self.elapsed_ms();
        self.drive(Some(link), |e| e.begin_session(profile, now))
    }

    pub fn on_bytes(&self, bytes: Vec<u8>) {
        let now = self.elapsed_ms();
        self.drive(None, |e| e.on_bytes(&bytes, now));
    }

    pub fn on_link_closed(&self) {
        self.drive(None, |e| e.on_closed());
    }

    /// Call about once a second while a link is open: this is the only clock timeouts have.
    pub fn tick(&self) {
        self.tick_at(self.elapsed_ms());
    }

    /// Closes the link.  A partly sent file stays on the watch for a later resume.
    pub fn disconnect(&self) {
        self.drive(None, |e| e.disconnect());
    }

    pub fn request_status(&self) -> Result<(), CoreError> {
        let now = self.elapsed_ms();
        self.drive(None, |e| e.request_status(now))
    }

    /// Hashes `source`, then sends it and signs it.  If the watch holds a part of this
    /// exact package, sending resumes where that ended.  Blocks while it hashes, so call
    /// it off the UI thread.
    pub fn push_file(
        &self,
        package: PackageType,
        name: String,
        source: Arc<dyn ByteSource>,
    ) -> Result<(), CoreError> {
        self.push(package, name, source, None)
    }

    /// Stops the push and deletes the part on the watch.
    pub fn abort_push(&self) -> Result<(), CoreError> {
        let now = self.elapsed_ms();
        self.drive(None, |e| e.abort_push(now))
    }

    pub fn pull_file(&self, path: String, sink: Arc<dyn ByteSink>) -> Result<(), CoreError> {
        let now = self.elapsed_ms();
        self.drive(None, |e| e.pull(path, sink, now))
    }

    pub fn pull_log(&self, log: LogKind, sink: Arc<dyn ByteSink>) -> Result<(), CoreError> {
        self.pull_file(log.path().to_string(), sink)
    }
}

impl WatchClient {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Runs one engine call, then delivers its outputs with the lock released, because
    /// a link may call `on_bytes` from inside `send`.
    ///
    /// One thread delivers at a time and the others leave their outputs in the queue:
    /// records carry consecutive nonces, so two threads racing here could put them on
    /// the link out of order, and the watch would close the session.
    fn drive<R>(
        &self,
        link: Option<Arc<dyn WatchLink>>,
        f: impl FnOnce(&mut Engine) -> R,
    ) -> R {
        let mut g = self.lock();
        if let Some(l) = link {
            if g.engine.state() == ConnectionState::Idle {
                g.link = Some(l);
            }
        }
        let result = f(&mut g.engine);
        let outputs = g.engine.take_outputs();
        g.queue.extend(outputs);
        if g.dispatching {
            return result;
        }
        g.dispatching = true;
        while let Some(output) = g.queue.pop_front() {
            let link = g.link.clone();
            drop(g);
            self.deliver(output, link);
            g = self.lock();
            let more = g.engine.take_outputs();
            g.queue.extend(more);
        }
        g.dispatching = false;
        result
    }

    fn deliver(&self, output: Output, link: Option<Arc<dyn WatchLink>>) {
        match output {
            Output::Send(bytes) => {
                if let Some(l) = link {
                    l.send(bytes);
                }
            }
            Output::Close => {
                if let Some(l) = link {
                    l.close_link();
                }
            }
            Output::Event(event) => self.events.on_event(event),
            Output::SavePin { key, event } => {
                match self.store.save(KEY_WATCH.to_string(), key.to_vec()) {
                    Ok(()) => self.events.on_event(event),
                    Err(error) => {
                        self.lock().engine.set_watch(None);
                        self.events.on_event(WatchEvent::Failed { error });
                    }
                }
            }
            Output::SaveHost(host) => {
                let _ = self.store.save(KEY_HOST.to_string(), host.into_bytes());
            }
        }
    }

    /// Milliseconds on the clock `tick` uses.  Public for tests of the timeouts.
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn tick_at(&self, now_ms: u64) {
        self.drive(None, |e| e.tick(now_ms));
    }

    /// Picks the sequence number by hand.  The test that a replay is refused needs one.
    #[doc(hidden)]
    pub fn push_file_with_seq(
        &self,
        package: PackageType,
        name: String,
        source: Arc<dyn ByteSource>,
        seq: u32,
    ) -> Result<(), CoreError> {
        self.push(package, name, source, Some(seq))
    }

    /// Hashing a large file takes a while, and `tick` runs on the UI thread: the lock is
    /// not held for it.  If something else started meanwhile, `start_push` says Busy.
    fn push(
        &self,
        package: PackageType,
        name: String,
        source: Arc<dyn ByteSource>,
        seq: Option<u32>,
    ) -> Result<(), CoreError> {
        let (profile, seed) = self.lock().engine.push_context()?;
        let job = PushJob::new(package, name, source, profile, seed, seq)?;
        let now = self.elapsed_ms();
        self.drive(None, |e| e.start_push(job, now))
    }
}
