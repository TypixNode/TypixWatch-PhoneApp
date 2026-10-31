//! The watch's CPace variant.  Everything here must match `w1a_cpace_share` and
//! `w1a_cpace_finish` in `w1a_sec.c` bit for bit: the construction is deliberately not
//! the draft's, so the draft's vectors do not apply.  `tests/interop_c.rs` checks it
//! against the C code.

use sha2::{Digest, Sha512};
use subtle::ConstantTimeEq;
use x25519_dalek::x25519;
use zeroize::Zeroizing;

use crate::error::CoreError;
use crate::proto::SID_LEN;
use crate::sec::{split_keys, Cipher};

const CI: &[u8] = b"w1a-ota-v1";
const DSI: &[u8] = b"CPace255";
const DSI_ISK: &[u8] = b"CPace255_ISK";

extern "C" {
    fn crypto_elligator_map(curve: *mut u8, hidden: *const u8);
}

/// Monocypher's map, compiled from the same source as the watch.  It reads the first
/// 32 bytes and ignores the top two bits.
pub fn elligator_map(hidden: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    // SAFETY: the C function writes 32 bytes to `curve`, reads 32 from `hidden`, and
    // keeps no reference to either.
    unsafe { crypto_elligator_map(out.as_mut_ptr(), hidden.as_ptr()) };
    out
}

fn lv(h: &mut Sha512, s: &[u8]) {
    debug_assert!(s.len() < 256);
    h.update([s.len() as u8]);
    h.update(s);
}

fn generator(code: &str, sid: &[u8; SID_LEN]) -> Zeroizing<[u8; 32]> {
    // SHA-512's block: the secret ends a block away from the public sid.
    let zeros = [0u8; 128];
    let pad = 128 - 1 - (code.len() + 1) - (DSI.len() + 1);
    let mut h = Sha512::new();
    lv(&mut h, DSI);
    lv(&mut h, code.as_bytes());
    lv(&mut h, &zeros[..pad]);
    lv(&mut h, CI);
    lv(&mut h, sid);
    let digest = Zeroizing::new(<[u8; 64]>::from(h.finalize()));
    let mut hidden = Zeroizing::new([0u8; 32]);
    hidden.copy_from_slice(&digest[..32]);
    Zeroizing::new(elligator_map(&hidden))
}

pub fn share(y_sk: &[u8; 32], code: &str, sid: &[u8; SID_LEN]) -> [u8; 32] {
    x25519(*y_sk, *generator(code, sid))
}

/// `ya` is always the phone's share, `yb` the watch's.  Returns (phone to watch, watch
/// to phone).  Fails on a degenerate shared point.
pub fn finish(
    y_sk: &[u8; 32],
    peer: &[u8; 32],
    ya: &[u8; 32],
    yb: &[u8; 32],
    sid: &[u8; SID_LEN],
) -> Result<(Cipher, Cipher), CoreError> {
    let k = Zeroizing::new(x25519(*y_sk, *peer));
    if bool::from(k.ct_eq(&[0u8; 32])) {
        return Err(CoreError::protocol("degenerate share"));
    }
    let mut h = Sha512::new();
    lv(&mut h, DSI_ISK);
    lv(&mut h, sid);
    h.update(&k[..]);
    lv(&mut h, ya);
    lv(&mut h, yb);
    let isk = Zeroizing::new(<[u8; 64]>::from(h.finalize()));
    Ok(split_keys(&isk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sec::random32;

    #[test]
    fn same_code_agrees_and_wrong_code_does_not() {
        let sid = [7u8; 16];
        let (a, b) = (random32().unwrap(), random32().unwrap());
        let ya = share(&a, "123456", &sid);
        let yb = share(&b, "123456", &sid);
        let (a1, _) = finish(&a, &yb, &ya, &yb, &sid).unwrap();
        let (b1, _) = finish(&b, &ya, &ya, &yb, &sid).unwrap();
        assert_eq!(a1.key(), b1.key());

        let yb = share(&b, "123457", &sid);
        let (a1, _) = finish(&a, &yb, &ya, &yb, &sid).unwrap();
        let (b1, _) = finish(&b, &ya, &ya, &yb, &sid).unwrap();
        assert_ne!(a1.key(), b1.key());
    }

    #[test]
    fn low_order_share_is_refused() {
        let a = random32().unwrap();
        assert!(finish(&a, &[0u8; 32], &[1u8; 32], &[0u8; 32], &[0u8; 16]).is_err());
    }
}
