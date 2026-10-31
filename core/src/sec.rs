//! Rust counterparts of `w1a_sec.c`.  This file only composes crates: X25519 and
//! Ed25519 from dalek, ChaCha20-Poly1305, SHA-2 and HKDF from RustCrypto.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ed25519_dalek::{Signer, SigningKey};
use hkdf::Hkdf;
use sha2::{Digest, Sha256, Sha512};
use x25519_dalek::{x25519, X25519_BASEPOINT_BYTES};
use zeroize::Zeroizing;

use crate::error::CoreError;
use crate::proto::TAG_LEN;

/// The longest name `op_put_begin` on the watch accepts.
pub const NAME_MAX: usize = 48;

pub fn random32() -> Result<Zeroizing<[u8; 32]>, CoreError> {
    let mut b = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(b.as_mut()).map_err(|e| CoreError::Crypto {
        detail: e.to_string(),
    })?;
    Ok(b)
}

pub fn x25519_public(sk: &[u8; 32]) -> [u8; 32] {
    x25519(*sk, X25519_BASEPOINT_BYTES)
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// First 8 bytes of SHA-256 of a public key, as 16 hex digits.
pub fn fingerprint(public_key: &[u8; 32]) -> String {
    hex(&sha256(public_key)[..8])
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `"w1a-ota-package-v1\0" ‖ type ‖ seq ‖ size ‖ sha256 ‖ len(name) ‖ name`.
pub fn manifest(package: u8, seq: u32, size: u32, sha: &[u8; 32], name: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(128);
    m.extend_from_slice(b"w1a-ota-package-v1\0");
    m.push(package);
    m.extend_from_slice(&seq.to_be_bytes());
    m.extend_from_slice(&size.to_be_bytes());
    m.extend_from_slice(sha);
    m.push(name.len() as u8);
    m.extend_from_slice(name.as_bytes());
    m
}

pub fn sign_public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

pub fn sign(seed: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    SigningKey::from_bytes(seed).sign(msg).to_bytes()
}

/// `hkdf2` of `w1a_sec.c` with an empty input: Noise's HKDF is HKDF-SHA512 with the
/// chaining key as salt and an empty info.
pub(crate) fn split_keys(ck: &[u8; 64]) -> (Cipher, Cipher) {
    let mut okm = Zeroizing::new([0u8; 128]);
    Hkdf::<Sha512>::new(Some(ck), &[])
        .expand(&[], okm.as_mut())
        .expect("128 bytes is within HKDF-SHA512's limit");
    let mut k1 = [0u8; 32];
    let mut k2 = [0u8; 32];
    k1.copy_from_slice(&okm[..32]);
    k2.copy_from_slice(&okm[64..96]);
    (Cipher::new(k1), Cipher::new(k2))
}

/// One direction of the pairing channel: Noise's nonce layout, 32 zero bits and a
/// little-endian counter that only moves on success.
pub struct Cipher {
    key: Zeroizing<[u8; 32]>,
    counter: u64,
}

impl Cipher {
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(key),
            counter: 0,
        }
    }

    /// Starts mid-stream, so tests can reach counters that take too long to count to.
    #[doc(hidden)]
    pub fn with_counter(key: [u8; 32], counter: u64) -> Self {
        let mut c = Self::new(key);
        c.counter = counter;
        c
    }

    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.counter.to_le_bytes());
        n
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let aead = ChaCha20Poly1305::new(Key::from_slice(&self.key[..]));
        let ct = aead
            .encrypt(
                Nonce::from_slice(&self.nonce()),
                Payload {
                    msg: plaintext,
                    aad: &[],
                },
            )
            .map_err(|_| CoreError::Crypto {
                detail: "seal".into(),
            })?;
        self.counter += 1;
        Ok(ct)
    }

    /// `None` when the tag is wrong.
    pub fn open(&mut self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        if ciphertext.len() < TAG_LEN {
            return None;
        }
        let aead = ChaCha20Poly1305::new(Key::from_slice(&self.key[..]));
        let pt = aead
            .decrypt(
                Nonce::from_slice(&self.nonce()),
                Payload {
                    msg: ciphertext,
                    aad: &[],
                },
            )
            .ok()?;
        self.counter += 1;
        Some(pt)
    }
}
