//! Whole flows through `WatchClient`, against the in-memory watch.

mod support;

use std::sync::Arc;

use support::{MemFile, MemSink, MockWatch, Rig};
use typix_core::{ble_profile, tcp_profile, ConnectionState, CoreError, LogKind, PackageType};
use typix_core::{WatchClient, WatchEvent};

fn file(n: usize) -> Arc<MemFile> {
    Arc::new(MemFile((0..n).map(|i| (i * 31 % 251) as u8).collect()))
}

fn pushed(r: &Rig) -> Option<typix_core::PushResult> {
    r.events.0.lock().unwrap().iter().find_map(|e| match e {
        WatchEvent::PushFinished { result } => Some(result.clone()),
        _ => None,
    })
}

/// One round trip of whatever is queued.
fn step(r: &mut Rig) {
    let queued: Vec<_> = r.link.outbox.lock().unwrap().drain(..).collect();
    queued.iter().for_each(|b| r.watch.feed(b));
    let out = std::mem::take(&mut r.watch.out);
    r.client.on_bytes(out);
}

#[test]
fn pairing_pins_the_watch_and_a_session_reports_status() {
    let mut r = Rig::new();
    r.pair();
    assert_eq!(r.watch.peer_names, vec!["Test Phone"]);
    assert_eq!(*r.link.closes.lock().unwrap(), 1, "pairing closes its link");
    let ev = r.taken();
    assert!(ev.iter().any(|e| matches!(e,
        WatchEvent::Paired { watch_name, .. } if watch_name == "CMCC W1A")));

    r.connect(tcp_profile());
    r.client.request_status().unwrap();
    r.pump();
    let status = r.taken().into_iter().find_map(|e| match e {
        WatchEvent::Status { status } => Some(status),
        _ => None,
    });
    let status = status.unwrap();
    assert_eq!(status.battery_percent, Some(64));
    assert_eq!(r.client.saved_host().as_deref(), Some("192.0.2.7"));
}

#[test]
fn keys_and_pin_survive_a_restart() {
    let mut r = Rig::new();
    r.pair();
    let fp = r.client.phone_fingerprint();
    let again = WatchClient::new(r.store.clone(), r.events.clone()).unwrap();
    assert_eq!(again.phone_fingerprint(), fp);
    assert!(again.is_paired());
    r.client = again;
    r.connect(tcp_profile());
}

#[test]
fn wrong_code_voids_the_pairing() {
    let mut r = Rig::new();
    r.watch.code = Some("111111".into());
    r.client
        .begin_pairing(r.link.clone(), tcp_profile(), "222222".into(), "p".into())
        .unwrap();
    r.pump();
    assert_eq!(r.failure(), Some(CoreError::WrongCode));
    assert!(!r.client.is_paired());
    assert_eq!(r.watch.code, None, "one guess per button press");
    assert_eq!(r.client.state(), ConnectionState::Idle);
}

#[test]
fn pairing_refused_when_the_watch_is_not_pairing() {
    let mut r = Rig::new();
    r.client
        .begin_pairing(r.link.clone(), tcp_profile(), "222222".into(), "p".into())
        .unwrap();
    r.pump();
    assert_eq!(r.failure(), Some(CoreError::PairingNotOpen));
}

#[test]
fn a_bad_code_never_reaches_the_link() {
    let r = Rig::new();
    for bad in ["12345", "1234567", "12a456", "１２３４５６"] {
        let got = r
            .client
            .begin_pairing(r.link.clone(), tcp_profile(), bad.into(), "p".into());
        assert_eq!(got, Err(CoreError::InvalidCode), "{bad}");
    }
    assert!(r.link.outbox.lock().unwrap().is_empty());
}

#[test]
fn long_names_are_cut_at_a_character() {
    let mut r = Rig::new();
    r.watch.code = Some("111111".into());
    r.client
        .begin_pairing(r.link.clone(), tcp_profile(), "111111".into(), "錶".repeat(20))
        .unwrap();
    r.pump();
    assert_eq!(r.watch.peer_names, vec!["錶".repeat(10)]);
}

#[test]
fn session_refused_by_the_watch_is_not_a_key_problem() {
    let mut r = Rig::new();
    r.pair();
    r.watch.peers.clear();
    r.watch.new_connection();
    r.client
        .begin_session(r.link.clone(), tcp_profile())
        .unwrap();
    r.pump();
    assert_eq!(r.failure(), Some(CoreError::SessionRejected));
}

#[test]
fn a_different_watch_fails_the_pin() {
    let mut r = Rig::new();
    r.pair();
    // Someone else's watch that happens to accept this phone: IK binds its static key
    // into the first message, so it cannot even read it.
    let peers = r.watch.peers.clone();
    r.watch = MockWatch::new();
    r.watch.peers = peers;
    r.client
        .begin_session(r.link.clone(), tcp_profile())
        .unwrap();
    r.pump();
    assert_eq!(r.failure(), Some(CoreError::SessionRejected));
    assert_ne!(r.client.state(), ConnectionState::Ready);
}

#[test]
fn a_forged_second_message_fails_the_pin() {
    let mut r = Rig::new();
    r.pair();
    r.client
        .begin_session(r.link.clone(), tcp_profile())
        .unwrap();
    r.link.outbox.lock().unwrap().clear();
    let mut m2 = 48u16.to_be_bytes().to_vec();
    m2.extend([0x55; 48]);
    r.client.on_bytes(m2);
    assert_eq!(r.failure(), Some(CoreError::WatchKeyMismatch));
}

#[test]
fn session_without_pairing_is_refused() {
    let r = Rig::new();
    assert_eq!(
        r.client.begin_session(r.link.clone(), tcp_profile()),
        Err(CoreError::NotPaired)
    );
}

#[test]
fn push_over_each_link_profile() {
    for (profile, chunk) in [(tcp_profile(), 32 * 1024), (ble_profile(), 400)] {
        let mut r = Rig::new();
        r.pair();
        r.connect(profile);
        let f = file(100_000);
        r.client
            .push_file(PackageType::Script, "fix.sh".into(), f.clone())
            .unwrap();
        r.pump();
        let got = pushed(&r).unwrap_or_else(|| panic!("{:?}", r.taken()));
        assert_eq!(got.seq, 1);
        assert_eq!(got.resumed_from, 0);
        assert_eq!(got.message, "stored");
        assert_eq!(r.watch.installed, vec![("fix.sh".to_string(), f.0.clone())]);
        assert_eq!(r.watch.max_inflight_chunk, chunk);
    }
}

#[test]
fn a_dropped_push_resumes_where_it_stopped() {
    let mut r = Rig::new();
    r.pair();
    r.connect(ble_profile());
    let f = file(10_000);
    r.watch.drop_after_data = Some(5);
    r.client
        .push_file(PackageType::Assets, "a.pak".into(), f.clone())
        .unwrap();
    r.pump();
    assert_eq!(r.failure(), Some(CoreError::LinkClosed));
    assert_eq!(r.client.state(), ConnectionState::Idle);
    let have = r.watch.part.len() as u64;
    assert_eq!(have, 2000);
    assert!(r.watch.installed.is_empty());
    r.taken();

    r.connect(tcp_profile());
    r.client
        .push_file(PackageType::Assets, "a.pak".into(), f.clone())
        .unwrap();
    r.pump();
    let got = pushed(&r).unwrap();
    assert_eq!(got.resumed_from, have);
    assert_eq!(got.seq, 1, "a resume must repeat the sequence number");
    assert_eq!(r.watch.installed[0].1, f.0);
    assert!(r
        .taken()
        .contains(&WatchEvent::PushStarted { resumed_from: have }));
}

#[test]
fn a_complete_part_goes_straight_to_commit() {
    let mut r = Rig::new();
    r.pair();
    r.connect(ble_profile());
    let f = file(800);
    r.watch.drop_after_data = Some(2);
    r.client
        .push_file(PackageType::Script, "x".into(), f.clone())
        .unwrap();
    r.pump();
    assert_eq!(r.watch.part.len(), 800);
    r.taken();
    let before = r.watch.seen_put_data;
    r.connect(ble_profile());
    r.client
        .push_file(PackageType::Script, "x".into(), f)
        .unwrap();
    r.pump();
    assert_eq!(pushed(&r).unwrap().resumed_from, 800);
    assert_eq!(r.watch.seen_put_data, before);
}

#[test]
fn a_changed_file_does_not_resume() {
    let mut r = Rig::new();
    r.pair();
    r.connect(ble_profile());
    r.watch.drop_after_data = Some(2);
    r.client
        .push_file(PackageType::Script, "x".into(), file(4000))
        .unwrap();
    r.pump();
    r.taken();
    r.connect(ble_profile());
    let other = Arc::new(MemFile(vec![9; 4000]));
    r.client
        .push_file(PackageType::Script, "x".into(), other.clone())
        .unwrap();
    r.pump();
    assert_eq!(pushed(&r).unwrap().resumed_from, 0);
    assert_eq!(r.watch.installed[0].1, other.0);
}

#[test]
fn a_replayed_sequence_number_is_refused_and_the_session_lives() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    r.client
        .push_file(PackageType::Script, "x".into(), file(10))
        .unwrap();
    r.pump();
    r.taken();
    r.client
        .push_file_with_seq(PackageType::Script, "x".into(), file(10), 1)
        .unwrap();
    r.pump();
    assert!(matches!(
        r.failure(),
        Some(CoreError::Remote { code: 114, .. })
    ));
    assert_eq!(r.client.state(), ConnectionState::Ready);
    r.taken();
    r.client.request_status().unwrap();
    r.pump();
    assert_eq!(r.failure(), None);
}

#[test]
fn a_refusal_mid_window_leaves_the_session_usable() {
    let mut r = Rig::new();
    r.pair();
    r.connect(ble_profile());
    r.client
        .push_file(PackageType::Script, "x".into(), file(4000))
        .unwrap();
    step(&mut r); // STATUS
    step(&mut r); // PUT_BEGIN; a window of PUT_DATA is now queued
    r.watch.part = vec![0; 1]; // the watch's offset no longer agrees
    r.pump();
    assert!(matches!(r.failure(), Some(CoreError::Remote { code: 29, .. })));
    r.taken();
    r.client.request_status().unwrap();
    r.pump();
    assert_eq!(r.failure(), None);
    assert_eq!(r.client.state(), ConnectionState::Ready);
}

#[test]
fn abort_drops_the_part() {
    let mut r = Rig::new();
    r.pair();
    r.connect(ble_profile());
    r.client
        .push_file(PackageType::Script, "x".into(), file(4000))
        .unwrap();
    step(&mut r); // STATUS
    step(&mut r); // PUT_BEGIN; a window of PUT_DATA is now queued
    r.client.abort_push().unwrap();
    r.pump();
    assert!(r.taken().contains(&WatchEvent::PushAborted));
    assert!(r.watch.part.is_empty());
    assert!(r.watch.installed.is_empty());
    r.client.request_status().unwrap();
    r.pump();
    assert_eq!(r.failure(), None);
}

#[test]
fn names_the_watch_would_refuse_never_leave_the_phone() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    for (t, name) in [
        (PackageType::Script, "../evil"),
        (PackageType::Script, ".hidden"),
        (PackageType::Script, "a b"),
        (PackageType::Launcher, ""),
    ] {
        assert!(matches!(
            r.client.push_file(t, name.into(), file(1)),
            Err(CoreError::InvalidInput { .. })
        ));
    }
    assert!(matches!(
        r.client
            .push_file(PackageType::Launcher, "n".repeat(49), file(1)),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(r
        .client
        .push_file(PackageType::Launcher, "n".repeat(48), file(1))
        .is_ok());
}

#[test]
fn pull_verifies_the_hash() {
    for profile in [tcp_profile(), ble_profile()] {
        let mut r = Rig::new();
        r.pair();
        r.connect(profile);
        let log = file(70_000).0.clone();
        r.watch.files.insert("/tmp/w1a-otad.log".into(), log.clone());
        let sink = Arc::new(MemSink::default());
        r.client.pull_log(LogKind::Otad, sink.clone()).unwrap();
        r.pump();
        assert_eq!(*sink.verdict.lock().unwrap(), Some(true));
        assert_eq!(*sink.data.lock().unwrap(), log);
    }
}

#[test]
fn an_empty_file_pulls_cleanly() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    r.watch.files.insert("/tmp/empty".into(), vec![]);
    let sink = Arc::new(MemSink::default());
    r.client.pull_file("/tmp/empty".into(), sink.clone()).unwrap();
    r.pump();
    assert_eq!(*sink.verdict.lock().unwrap(), Some(true));
}

#[test]
fn a_tampered_pull_is_discarded() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    r.watch.files.insert("/tmp/x".into(), file(100).0.clone());
    r.watch.corrupt_reads = true;
    let sink = Arc::new(MemSink::default());
    r.client.pull_file("/tmp/x".into(), sink.clone()).unwrap();
    r.pump();
    assert_eq!(r.failure(), Some(CoreError::HashMismatch));
    assert_eq!(*sink.verdict.lock().unwrap(), Some(false));
    assert_eq!(r.client.state(), ConnectionState::Ready);
}

#[test]
fn a_pull_refused_by_the_watch_tells_the_sink() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    let sink = Arc::new(MemSink::default());
    r.client.pull_file("/nope".into(), sink.clone()).unwrap();
    r.pump();
    assert!(matches!(r.failure(), Some(CoreError::Remote { code: 2, .. })));
    assert_eq!(*sink.verdict.lock().unwrap(), Some(false));
}

#[test]
fn a_lost_link_tells_the_sink() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    r.watch.files.insert("/tmp/x".into(), file(100).0.clone());
    let sink = Arc::new(MemSink::default());
    r.client.pull_file("/tmp/x".into(), sink.clone()).unwrap();
    r.client.on_link_closed();
    assert_eq!(*sink.verdict.lock().unwrap(), Some(false));
    assert_eq!(r.failure(), Some(CoreError::LinkClosed));
}

#[test]
fn one_operation_at_a_time() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    r.client.request_status().unwrap();
    assert_eq!(r.client.request_status(), Err(CoreError::Busy));
    r.pump();
    r.client.request_status().unwrap();
}

#[test]
fn silence_times_out_and_closes_the_link() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    let closes = *r.link.closes.lock().unwrap();
    r.watch.silent = true;
    r.client.request_status().unwrap();
    r.pump();
    let now = r.client.elapsed_ms();
    r.client.tick_at(now + 59_000);
    assert_eq!(r.failure(), None);
    r.client.tick_at(now + 61_000);
    assert_eq!(r.failure(), Some(CoreError::Timeout));
    assert_eq!(*r.link.closes.lock().unwrap(), closes + 1);
    assert_eq!(r.client.state(), ConnectionState::Idle);
}

#[test]
fn the_app_disconnecting_is_not_a_failure() {
    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    r.taken();
    r.client.disconnect();
    assert_eq!(r.failure(), None);
    assert_eq!(r.client.state(), ConnectionState::Idle);
}

#[test]
fn state_changes_are_announced_even_when_a_frame_fails() {
    let mut r = Rig::new();
    r.pair();
    r.client.begin_session(r.link.clone(), tcp_profile()).unwrap();
    r.taken();
    let mut m2 = 48u16.to_be_bytes().to_vec();
    m2.extend([0; 48]);
    r.client.on_bytes(m2);
    assert!(r.taken().contains(&WatchEvent::State {
        state: ConnectionState::Idle
    }));
}

/// A link that answers inside `send`, as a synchronous platform call could.
#[test]
fn a_link_may_call_back_from_inside_send() {
    use std::sync::{Mutex, Weak};
    use support::{Events, MemStore};
    use typix_core::WatchLink;

    struct Loopback {
        client: Mutex<Weak<WatchClient>>,
        watch: Mutex<MockWatch>,
    }
    impl WatchLink for Loopback {
        fn send(&self, bytes: Vec<u8>) {
            let out = {
                let mut w = self.watch.lock().unwrap();
                w.feed(&bytes);
                std::mem::take(&mut w.out)
            };
            if let (Some(c), false) = (self.client.lock().unwrap().upgrade(), out.is_empty()) {
                c.on_bytes(out);
            }
        }
        fn close_link(&self) {}
    }

    let events = Arc::new(Events::default());
    let client = WatchClient::new(Arc::new(MemStore::default()), events.clone()).unwrap();
    let mut watch = MockWatch::new();
    watch.code = Some("000000".into());
    let link = Arc::new(Loopback {
        client: Mutex::new(Arc::downgrade(&client)),
        watch: Mutex::new(watch),
    });
    client
        .begin_pairing(link, tcp_profile(), "000000".into(), "p".into())
        .unwrap();
    assert!(client.is_paired());
}

/// `tick` runs on the UI thread, so hashing a file must not hold the lock it needs.
#[test]
fn hashing_does_not_hold_the_client_lock() {
    use std::sync::{Mutex, Weak};
    use typix_core::{ByteSource, CoreError};

    struct Probe {
        client: Mutex<Weak<WatchClient>>,
        data: Vec<u8>,
    }
    impl ByteSource for Probe {
        fn size(&self) -> Result<u64, CoreError> {
            Ok(self.data.len() as u64)
        }
        fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, CoreError> {
            if let Some(c) = self.client.lock().unwrap().upgrade() {
                c.tick(); // deadlocks if the caller holds the lock
            }
            Ok(self.data[offset as usize..offset as usize + length as usize].to_vec())
        }
    }

    let mut r = Rig::new();
    r.pair();
    r.connect(tcp_profile());
    let probe = Arc::new(Probe {
        client: Mutex::new(Arc::downgrade(&r.client)),
        data: vec![1; 1000],
    });
    r.client
        .push_file(PackageType::Script, "x".into(), probe.clone())
        .unwrap();
    *probe.client.lock().unwrap() = Weak::new();
    r.pump();
    assert!(pushed(&r).is_some());
}
