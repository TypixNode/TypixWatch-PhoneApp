//! Wire protocol of w1a-otad, from `linux-build/tools/w1a-otad/w1a_proto.h`.
//!
//! TCP and BLE carry the same stream: frames of a big-endian u16 length and a payload.

use crate::error::CoreError;

pub const PORT: u16 = 7423;
pub const VERSION: u8 = 1;
pub const MODE_PAIR: u8 = 1;
pub const MODE_SESSION: u8 = 2;
pub const SID_LEN: usize = 16;
pub const IK1_LEN: usize = 96;
pub const IK2_LEN: usize = 48;
pub const TAG_LEN: usize = 16;

pub const BLE_SERVICE: &str = "5f1d0a00-7c3e-4b52-9a61-3e0c8f6b1a70";
/// The phone writes here, without response.
pub const BLE_RX: &str = "5f1d0a01-7c3e-4b52-9a61-3e0c8f6b1a70";
/// The watch notifies here.
pub const BLE_TX: &str = "5f1d0a02-7c3e-4b52-9a61-3e0c8f6b1a70";

pub mod op {
    pub const PING: u8 = 0x00;
    pub const STATUS: u8 = 0x01;
    pub const PUT_BEGIN: u8 = 0x02;
    pub const PUT_DATA: u8 = 0x03;
    pub const PUT_COMMIT: u8 = 0x04;
    pub const PUT_ABORT: u8 = 0x05;
    pub const GET_OPEN: u8 = 0x06;
    pub const GET_READ: u8 = 0x07;
    pub const REPLY_OK: u8 = 0x80;
    pub const REPLY_ERR: u8 = 0x81;
}

/// What a package installs as on the watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PackageType {
    Launcher,
    Assets,
    Script,
    Image,
}

impl PackageType {
    pub fn wire(self) -> u8 {
        match self {
            PackageType::Launcher => 1,
            PackageType::Assets => 2,
            PackageType::Script => 3,
            PackageType::Image => 4,
        }
    }

    pub(crate) fn seq_key(self) -> &'static str {
        match self {
            PackageType::Launcher => "seq_launcher",
            PackageType::Assets => "seq_assets",
            PackageType::Script => "seq_script",
            PackageType::Image => "seq_image",
        }
    }
}

/// The logs the watch serves without the shell switch, except `Dmesg`'s pseudo path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LogKind {
    Shell,
    Connectd,
    Otad,
    Dmesg,
    Audit,
}

impl LogKind {
    pub fn path(self) -> &'static str {
        match self {
            LogKind::Shell => "/tmp/w1a-shell.log",
            LogKind::Connectd => "/tmp/w1a-connectd.log",
            LogKind::Otad => "/tmp/w1a-otad.log",
            LogKind::Dmesg => "@dmesg",
            LogKind::Audit => "/data/ota/audit.log",
        }
    }
}

pub fn hello(mode: u8) -> [u8; 6] {
    [b'W', b'1', b'A', b'O', VERSION, mode]
}

pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, CoreError> {
    let n = u16::try_from(payload.len())
        .map_err(|_| CoreError::protocol("frame longer than 65535 bytes"))?;
    let mut out = Vec::with_capacity(payload.len() + 2);
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Reassembles frames from however the link cut the stream: BLE notifications can
/// split or join them freely.
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    pos: usize,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) {
        if self.pos > 0 && self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        let rest = &self.buf[self.pos..];
        if rest.len() < 2 {
            return None;
        }
        let n = u16::from_be_bytes([rest[0], rest[1]]) as usize;
        if rest.len() < 2 + n {
            return None;
        }
        let frame = rest[2..2 + n].to_vec();
        self.pos += 2 + n;
        Some(frame)
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.pos = 0;
    }
}

pub(crate) fn be32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(..4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_any_cut() {
        let mut stream = encode_frame(b"abc").unwrap();
        stream.extend(encode_frame(b"").unwrap());
        stream.extend(encode_frame(&[7; 300]).unwrap());
        for cut in 1..8 {
            let mut d = FrameDecoder::default();
            let mut got = vec![];
            for piece in stream.chunks(cut) {
                d.push(piece);
                while let Some(f) = d.next_frame() {
                    got.push(f);
                }
            }
            assert_eq!(got, vec![b"abc".to_vec(), vec![], vec![7; 300]]);
        }
    }

    #[test]
    fn oversized_frame_is_refused() {
        assert!(encode_frame(&vec![0; 65536]).is_err());
        assert!(encode_frame(&vec![0; 65535]).is_ok());
    }
}
