//! Round trip time measured by opening TCP connections.
//!
//! ICMP echo, what `ping` uses, needs a raw socket and therefore root or
//! `CAP_NET_RAW`. Timing the TCP handshake to the same host traverses the same
//! path without any privilege, at the cost of also including the time the
//! server takes to accept the connection.

use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::{TcpStream, lookup_host};
use tokio::time::timeout;

use crate::endpoint::Endpoint;
use crate::error::{Error, Result};

const ATTEMPTS: usize = 5;
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// A summary of the round trip times observed for one host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Latency {
    median: Duration,
    jitter: Duration,
    samples: usize,
}

impl Latency {
    /// Returns the typical round trip time.
    ///
    /// The median rather than the mean, so one connection that stalls behind a
    /// retransmit does not drag the whole reading up.
    pub const fn median(self) -> Duration {
        self.median
    }

    /// Returns how much consecutive round trips varied.
    ///
    /// Computed as the mean absolute difference between successive samples,
    /// which is what makes a link feel unstable for calls and games even when
    /// its median latency looks fine.
    pub const fn jitter(self) -> Duration {
        self.jitter
    }

    /// Returns how many probes succeeded.
    pub const fn samples(self) -> usize {
        self.samples
    }

    /// Summarises raw round trip times, or returns `None` if there are none.
    fn from_samples(mut samples: Vec<Duration>) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let jitter = mean_consecutive_deviation(&samples);
        samples.sort_unstable();
        Some(Self {
            median: median(&samples),
            jitter,
            samples: samples.len(),
        })
    }
}

/// Measures the round trip time to `endpoint`.
///
/// The host is resolved once up front so that DNS lookup time is excluded from
/// every sample rather than only from the ones after the first.
pub async fn measure(endpoint: &Endpoint) -> Result<Latency> {
    let address = resolve(endpoint).await?;

    let mut samples = Vec::with_capacity(ATTEMPTS);
    let mut last_failure = None;
    for _ in 0..ATTEMPTS {
        match probe_once(address).await {
            Ok(round_trip) => samples.push(round_trip),
            Err(failure) => last_failure = Some(failure),
        }
    }

    Latency::from_samples(samples).ok_or_else(|| Error::Connect {
        host: endpoint.host().to_owned(),
        source: last_failure
            .unwrap_or_else(|| io::Error::other("connection failed for an unknown reason")),
    })
}

async fn resolve(endpoint: &Endpoint) -> Result<SocketAddr> {
    let mut addresses = lookup_host(endpoint.socket_address())
        .await
        .map_err(|source| Error::Connect {
            host: endpoint.host().to_owned(),
            source,
        })?;
    addresses.next().ok_or_else(|| Error::Resolve {
        host: endpoint.host().to_owned(),
    })
}

async fn probe_once(address: SocketAddr) -> io::Result<Duration> {
    let started = Instant::now();
    match timeout(PROBE_TIMEOUT, TcpStream::connect(address)).await {
        Ok(Ok(_stream)) => Ok(started.elapsed()),
        Ok(Err(failure)) => Err(failure),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("no answer within {PROBE_TIMEOUT:?}"),
        )),
    }
}

fn median(sorted: &[Duration]) -> Duration {
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2
    } else {
        sorted[middle]
    }
}

fn mean_consecutive_deviation(samples: &[Duration]) -> Duration {
    let Some(gaps) = samples.len().checked_sub(1).filter(|gaps| *gaps > 0) else {
        return Duration::ZERO;
    };
    let total: Duration = samples
        .windows(2)
        .map(|pair| pair[0].abs_diff(pair[1]))
        .sum();
    total / gaps as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_millis).collect()
    }

    #[test]
    fn median_ignores_a_single_stalled_probe() {
        let latency =
            Latency::from_samples(millis(&[20, 21, 19, 22, 400])).expect("samples were provided");
        assert_eq!(latency.median(), Duration::from_millis(21));
        assert_eq!(latency.samples(), 5);
    }

    #[test]
    fn median_of_an_even_count_averages_the_middle_pair() {
        let latency = Latency::from_samples(millis(&[10, 20, 30, 40])).expect("samples exist");
        assert_eq!(latency.median(), Duration::from_millis(25));
    }

    #[test]
    fn jitter_measures_variation_between_consecutive_probes() {
        let steady = Latency::from_samples(millis(&[20, 20, 20])).expect("samples exist");
        assert_eq!(steady.jitter(), Duration::ZERO);

        let erratic = Latency::from_samples(millis(&[20, 30, 20])).expect("samples exist");
        assert_eq!(erratic.jitter(), Duration::from_millis(10));
    }

    #[test]
    fn a_lone_probe_has_no_jitter_to_report() {
        let latency = Latency::from_samples(millis(&[42])).expect("one sample is enough");
        assert_eq!(latency.median(), Duration::from_millis(42));
        assert_eq!(latency.jitter(), Duration::ZERO);
    }

    #[test]
    fn no_successful_probe_yields_no_summary() {
        assert!(Latency::from_samples(Vec::new()).is_none());
    }
}
