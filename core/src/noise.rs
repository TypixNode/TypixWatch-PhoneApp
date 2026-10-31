//! Noise_IK initiator on top of `snow`.  The first frame of a session, the hello, is
//! the prologue; the watch's static key is the one pinned at pairing.

use crate::error::CoreError;
use crate::proto::{IK1_LEN, IK2_LEN, TAG_LEN};

pub const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_SHA512";

fn crypto(e: snow::Error) -> CoreError {
    CoreError::Crypto {
        detail: e.to_string(),
    }
}

pub struct Initiator {
    hs: snow::HandshakeState,
}

impl Initiator {
    pub fn new(identity: &[u8; 32], watch: &[u8; 32], prologue: &[u8]) -> Result<Self, CoreError> {
        Self::build(identity, watch, prologue, None)
    }

    /// A fixed ephemeral makes the handshake reproducible, for known-answer tests.
    /// Reusing one in a real session breaks forward secrecy.
    #[doc(hidden)]
    pub fn with_ephemeral_for_test(
        identity: &[u8; 32],
        watch: &[u8; 32],
        prologue: &[u8],
        ephemeral: &[u8; 32],
    ) -> Result<Self, CoreError> {
        Self::build(identity, watch, prologue, Some(ephemeral))
    }

    fn build(
        identity: &[u8; 32],
        watch: &[u8; 32],
        prologue: &[u8],
        ephemeral: Option<&[u8; 32]>,
    ) -> Result<Self, CoreError> {
        let params = PATTERN.parse().map_err(crypto)?;
        let mut b = snow::Builder::new(params)
            .local_private_key(identity)
            .map_err(crypto)?
            .remote_public_key(watch)
            .map_err(crypto)?
            .prologue(prologue)
            .map_err(crypto)?;
        if let Some(e) = ephemeral {
            b = b.fixed_ephemeral_key_for_testing_only(e);
        }
        Ok(Self {
            hs: b.build_initiator().map_err(crypto)?,
        })
    }

    /// Message 1 carries no payload: the watch reads no command before the handshake
    /// ends, so a replayed message 1 gets nothing.
    pub fn write1(&mut self) -> Result<Vec<u8>, CoreError> {
        let mut out = vec![0u8; IK1_LEN];
        let n = self.hs.write_message(&[], &mut out).map_err(crypto)?;
        out.truncate(n);
        if n != IK1_LEN {
            return Err(CoreError::protocol("message 1 has the wrong length"));
        }
        Ok(out)
    }

    /// `Err` means this is not the watch that was pinned.
    pub fn read2(mut self, m2: &[u8]) -> Result<Transport, CoreError> {
        if m2.len() != IK2_LEN {
            return Err(CoreError::WatchKeyMismatch);
        }
        let mut payload = [0u8; 16];
        self.hs
            .read_message(m2, &mut payload)
            .map_err(|_| CoreError::WatchKeyMismatch)?;
        Ok(Transport {
            ts: self.hs.into_transport_mode().map_err(crypto)?,
        })
    }
}

/// The records of a session.  `snow` keeps the counters, so they cannot repeat.
pub struct Transport {
    ts: snow::TransportState,
}

impl Transport {
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let mut out = vec![0u8; plaintext.len() + TAG_LEN];
        let n = self.ts.write_message(plaintext, &mut out).map_err(crypto)?;
        out.truncate(n);
        Ok(out)
    }

    pub fn open(&mut self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        let mut out = vec![0u8; ciphertext.len()];
        let n = self.ts.read_message(ciphertext, &mut out).ok()?;
        out.truncate(n);
        Some(out)
    }
}
