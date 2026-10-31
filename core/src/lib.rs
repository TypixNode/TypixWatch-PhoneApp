//! Everything the Typix Watch companion does that is not Bluetooth or UI: the w1a-otad
//! protocol, Noise sessions, CPace pairing, OTA pushes and resume, and log pulls.
//!
//! Layers, bottom up: `proto` and `sec` are data; `cpace` and `noise` are the handshakes;
//! `transfer` and `engine` are pure state machines; `client` is the only part with locks,
//! and the only surface the apps see, through UniFFI.

uniffi::setup_scaffolding!();

pub mod client;
pub mod cpace;
pub mod engine;
pub mod error;
pub mod model;
pub mod noise;
pub mod proto;
pub mod sec;
pub mod transfer;

pub use client::{SecureStore, WatchClient, WatchEventListener, WatchLink};
pub use engine::WatchEvent;
pub use error::CoreError;
pub use model::{ble_profile, tcp_profile, ConnectionState, LinkProfile, WatchStatus};
pub use proto::{LogKind, PackageType};
pub use transfer::{ByteSink, ByteSource, PullResult, PushResult};

#[uniffi::export]
pub fn ble_service_uuid() -> String {
    proto::BLE_SERVICE.to_string()
}

#[uniffi::export]
pub fn ble_rx_uuid() -> String {
    proto::BLE_RX.to_string()
}

#[uniffi::export]
pub fn ble_tx_uuid() -> String {
    proto::BLE_TX.to_string()
}

#[uniffi::export]
pub fn tcp_port() -> u16 {
    proto::PORT
}

#[uniffi::export]
pub fn is_valid_pairing_code(code: String) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}
