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

    /// The endpoint is refusing further transfers from this address.
    ///
    /// Distinct from a generic bad status because it is temporary and the user
    /// can do something about it: a plain "HTTP 429" reads like a defect.
    #[error("the endpoint is rate limiting this address, wait a few minutes and try again")]
    RateLimited,

    /// The server responded in a shape the client cannot read.
    #[error("server returned an unexpected response: {detail}")]
    MalformedResponse { detail: String },

    /// A transfer phase ran to completion without moving any bytes.
    #[error("no data was transferred during the {direction} phase")]
    NoData { direction: &'static str },

    /// The report could not be rendered as JSON.
    #[error("could not encode the result: {0}")]
    Encoding(#[from] serde_json::Error),

    /// The user interrupted the run.
    #[error("test cancelled")]
    Cancelled,
}

/// A result carrying this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
