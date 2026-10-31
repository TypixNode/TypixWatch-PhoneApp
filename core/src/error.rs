/// Everything that can go wrong, in terms the UI can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("the pairing code is six digits")]
    InvalidCode,
    #[error("the watch is not in pairing mode, or the code expired")]
    PairingNotOpen,
    /// The watch closes the link on a wrong code and voids this pairing.
    #[error("wrong code; the pairing was voided, start again on the watch")]
    WrongCode,
    #[error("not paired yet")]
    NotPaired,
    /// Unknown or revoked on the watch, or the watch is busy with another client.
    #[error("the watch refused this phone")]
    SessionRejected,
    /// The handshake failed against the key pinned at pairing.
    #[error("the watch's key differs from the one pinned at pairing")]
    WatchKeyMismatch,
    #[error("no connection")]
    NotConnected,
    #[error("another operation is running")]
    Busy,
    #[error("timed out waiting for the watch")]
    Timeout,
    #[error("the link closed")]
    LinkClosed,
    #[error("sha256 does not match; nothing was kept")]
    HashMismatch,
    #[error("the watch said: {text}")]
    Remote { code: i32, text: String },
    #[error("protocol error: {detail}")]
    Protocol { detail: String },
    #[error("invalid input: {detail}")]
    InvalidInput { detail: String },
    #[error("storage: {detail}")]
    Storage { detail: String },
    #[error("file: {detail}")]
    File { detail: String },
    #[error("crypto: {detail}")]
    Crypto { detail: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for CoreError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        CoreError::Storage { detail: e.reason }
    }
}

impl CoreError {
    pub(crate) fn protocol(detail: impl Into<String>) -> Self {
        CoreError::Protocol {
            detail: detail.into(),
        }
    }

    pub(crate) fn input(detail: impl Into<String>) -> Self {
        CoreError::InvalidInput {
            detail: detail.into(),
        }
    }
}
