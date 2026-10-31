//! Data the platform layers see.

use std::collections::HashMap;

use crate::error::CoreError;
use crate::proto::PackageType;

/// How much a link can carry at once.  The numbers are the Mac tool's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct LinkProfile {
    /// File bytes per PUT_DATA.
    pub chunk: u32,
    /// PUT_DATA records in flight.
    pub window: u32,
    /// File bytes per GET_READ.
    pub pull: u32,
}

#[uniffi::export]
pub fn tcp_profile() -> LinkProfile {
    LinkProfile {
        chunk: 32 * 1024,
        window: 4,
        pull: 32 * 1024,
    }
}

#[uniffi::export]
pub fn ble_profile() -> LinkProfile {
    LinkProfile {
        chunk: 400,
        window: 2,
        pull: 2048,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionState {
    Idle,
    Pairing,
    Handshaking,
    Ready,
}

/// `OP_STATUS`, decoded.  `raw` keeps every key, including ones this version does not
/// know about.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WatchStatus {
    pub protocol_version: u32,
    /// Seconds until the watch closes this window.
    pub seconds_left: i64,
    pub shell_enabled: bool,
    pub wifi_ip: Option<String>,
    /// `None` when the watch could not read its gauge.
    pub battery_percent: Option<u32>,
    pub wall_clock: i64,
    pub data_free: u64,
    pub ota_used: u64,
    pub ota_quota: u64,
    pub overlay_launcher: u64,
    pub overlay_assets: u64,
    pub staged_image: u64,
    pub trial: bool,
    pub rollback: Option<String>,
    pub raw: HashMap<String, String>,
}

pub(crate) fn parse_lines(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

pub(crate) fn parse_seq(raw: &HashMap<String, String>, package: PackageType) -> u32 {
    raw.get(package.seq_key())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub(crate) fn parse_status(body: &[u8]) -> Result<WatchStatus, CoreError> {
    let text =
        std::str::from_utf8(body).map_err(|_| CoreError::protocol("status is not UTF-8"))?;
    let raw = parse_lines(text);
    let int = |k: &str| raw.get(k).and_then(|v| v.parse::<i64>().ok());
    let num = |k: &str| raw.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    Ok(WatchStatus {
        protocol_version: num("version") as u32,
        seconds_left: int("left").unwrap_or(0),
        shell_enabled: int("shell") == Some(1),
        wifi_ip: raw.get("wifi_ip").filter(|s| !s.is_empty()).cloned(),
        battery_percent: int("battery").and_then(|v| u32::try_from(v).ok()),
        wall_clock: int("wall").unwrap_or(0),
        data_free: num("data_free"),
        ota_used: num("ota_used"),
        ota_quota: num("ota_quota"),
        overlay_launcher: num("overlay_launcher"),
        overlay_assets: num("overlay_assets"),
        staged_image: num("staged_image"),
        trial: int("trial") == Some(1),
        rollback: raw.get("rollback").cloned(),
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_as_the_watch_prints_it() {
        let s = parse_status(
            b"version=1\nport=7423\nleft=1794\nshell=0\nwifi_ip=\nbattery=-1\nwall=100\n\
              seq_launcher=3\nseq_assets=0\nseq_script=9\nseq_image=0\n\
              data_free=100\nota_used=2\nota_quota=15728640\ntrial=1\nrollback=crash loop\n",
        )
        .unwrap();
        assert_eq!(s.seconds_left, 1794);
        assert_eq!(s.wifi_ip, None);
        assert_eq!(s.battery_percent, None);
        assert_eq!(parse_seq(&s.raw, PackageType::Script), 9);
        assert_eq!(parse_seq(&s.raw, PackageType::Image), 0);
        assert!(s.trial);
        assert_eq!(s.rollback.as_deref(), Some("crash loop"));
    }

    #[test]
    fn negative_zero_left_is_zero() {
        assert_eq!(parse_status(b"left=-0\n").unwrap().seconds_left, 0);
    }
}
