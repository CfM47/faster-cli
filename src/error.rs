//! Failure modes of a speed test.

use std::io;

/// Anything that can go wrong while measuring.
///
/// Variants are the distinct situations a user can act on, not a mirror of
/// the underlying libraries: an unreachable network, a misbehaving server and
/// a deliberate cancellation each warrant a different response.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request never completed: DNS, TLS, connection reset or timeout.
    #[error("network request failed: {0}")]
    Transport(#[from] reqwest::Error),

    /// A TCP connection used for latency probing could not be established.
    #[error("could not connect to {host}: {source}")]
    Connect {
        host: String,
        #[source]
        source: io::Error,
    },

    /// The host name did not resolve to any address.
    #[error("could not resolve {host}")]
    Resolve { host: String },

    /// The server answered, but not with a success status.
    #[error("server responded with HTTP {status}")]
    UnexpectedStatus { status: u16 },

    /// The server responded in a shape the client cannot read.
    #[error("server returned an unexpected response: {detail}")]
    MalformedResponse { detail: String },

    /// Every latency probe failed, so no round trip time could be derived.
    #[error("no latency probe completed")]
    NoLatencySamples,

    /// A transfer phase ran to completion without moving any bytes.
    #[error("no data was transferred during the {direction} phase")]
    NoData { direction: &'static str },

    /// The user interrupted the run.
    #[error("test cancelled")]
    Cancelled,
}

/// A result carrying this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
