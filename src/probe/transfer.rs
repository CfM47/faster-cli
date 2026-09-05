//! The engine that saturates a link and counts the bytes that move.
//!
//! Throughput is measured over a fixed window rather than by timing a
//! fixed-size file: on a fast link a small file finishes inside TCP slow start
//! and reads low, while on a slow link a large one takes minutes. A time box
//! bounds the run regardless of the link it is pointed at.

use std::io;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Body, Client};
use tokio::task::JoinSet;

use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::units::{Bitrate, Bytes, Direction, Download, Throughput, Upload};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const MAX_CONSECUTIVE_FAILURES: usize = 3;

/// Sent repeatedly to fill an upload body.
///
/// A static buffer rather than a freshly allocated one per piece: the bytes
/// are never read, so allocating them would only measure the allocator.
static UPLOAD_PIECE: [u8; 64 * 1024] = [0; 64 * 1024];

/// How a transfer phase should be run.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// How long to keep transferring for.
    pub duration: Duration,
    /// How many connections to saturate the link with.
    pub connections: NonZeroUsize,
    /// How many bytes to move per request.
    pub chunk: Bytes,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            duration: Duration::from_secs(10),
            connections: NonZeroUsize::new(4).expect("4 is not zero"),
            chunk: Bytes::new(25_000_000),
        }
    }
}

/// A snapshot of a transfer still in flight.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// Which phase produced this snapshot.
    pub direction: &'static str,
    /// Bytes moved so far.
    pub transferred: Bytes,
    /// Time since the phase began.
    pub elapsed: Duration,
}

impl Progress {
    /// Returns the rate averaged over the phase so far.
    pub fn rate(self) -> Bitrate {
        Bitrate::from_transfer(self.transferred, self.elapsed)
    }
}

/// Which way the bytes flow, resolved at runtime inside a worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Download,
    Upload,
}

/// Ties a direction tag to the request it performs.
///
/// Pairing the two in one impl is what stops [`run`] from being asked for an
/// upload while returning a download-tagged measurement.
trait Phase: Direction {
    const KIND: Kind;
}

impl Phase for Download {
    const KIND: Kind = Kind::Download;
}

impl Phase for Upload {
    const KIND: Kind = Kind::Upload;
}

/// Measures how fast the link pulls data from `endpoint`.
///
/// `report` is called roughly five times a second so a caller can draw a live
/// display; pass `&mut |_| {}` when nothing is watching.
pub async fn download(
    client: &Client,
    endpoint: &Endpoint,
    plan: Plan,
    report: &mut dyn FnMut(Progress),
) -> Result<Throughput<Download>> {
    run::<Download>(client, endpoint, plan, report).await
}

/// Measures how fast the link pushes data to `endpoint`.
///
/// Bytes are counted as they are handed to the connection rather than when the
/// server acknowledges them. The body stream is only polled as the socket
/// drains, so the count tracks the send rate but leads it by about one socket
/// buffer, which a window of several seconds absorbs.
pub async fn upload(
    client: &Client,
    endpoint: &Endpoint,
    plan: Plan,
    report: &mut dyn FnMut(Progress),
) -> Result<Throughput<Upload>> {
    run::<Upload>(client, endpoint, plan, report).await
}

async fn run<P: Phase>(
    client: &Client,
    endpoint: &Endpoint,
    plan: Plan,
    report: &mut dyn FnMut(Progress),
) -> Result<Throughput<P>> {
    let url = match P::KIND {
        Kind::Download => endpoint.download_url(plan.chunk.get()),
        Kind::Upload => endpoint.upload_url(),
    };

    let counter = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let deadline = started + plan.duration;

    let mut workers = JoinSet::new();
    for _ in 0..plan.connections.get() {
        let client = client.clone();
        let url = url.clone();
        let counter = Arc::clone(&counter);
        workers.spawn(async move {
            transfer_until(P::KIND, &client, &url, plan.chunk, &counter, deadline).await
        });
    }

    let failure = supervise(&mut workers, &counter, started, deadline, P::LABEL, report).await;

    let transferred = Bytes::new(counter.load(Ordering::Relaxed));
    let elapsed = started.elapsed();
    if transferred.is_zero() {
        return Err(failure.unwrap_or(Error::NoData {
            direction: P::LABEL,
        }));
    }
    Ok(Throughput::new(transferred, elapsed))
}

/// Samples the shared counter until the deadline, then stops the workers.
///
/// Returns the first worker failure seen, which only matters if the phase ends
/// up having transferred nothing at all.
async fn supervise(
    workers: &mut JoinSet<Result<()>>,
    counter: &AtomicU64,
    started: Instant,
    deadline: Instant,
    direction: &'static str,
    report: &mut dyn FnMut(Progress),
) -> Option<Error> {
    let mut failure = None;

    loop {
        tokio::time::sleep(SAMPLE_INTERVAL).await;
        report(Progress {
            direction,
            transferred: Bytes::new(counter.load(Ordering::Relaxed)),
            elapsed: started.elapsed(),
        });

        while let Some(joined) = workers.try_join_next() {
            if let Ok(Err(worker_failure)) = joined {
                failure.get_or_insert(worker_failure);
            }
        }

        if Instant::now() >= deadline || workers.is_empty() {
            break;
        }
    }

    workers.shutdown().await;
    failure
}

/// Issues requests until the deadline, counting bytes as they move.
///
/// A request that fails is retried rather than ending the phase, since a
/// single reset connection says nothing about the link; only an unbroken run
/// of failures does.
async fn transfer_until(
    kind: Kind,
    client: &Client,
    url: &str,
    chunk: Bytes,
    counter: &Arc<AtomicU64>,
    deadline: Instant,
) -> Result<()> {
    let mut consecutive_failures = 0;

    while Instant::now() < deadline {
        let outcome = match kind {
            Kind::Download => drain(client, url, counter, deadline).await,
            Kind::Upload => fill(client, url, chunk, counter, deadline).await,
        };
        match outcome {
            Ok(()) => consecutive_failures = 0,
            Err(failure) => {
                consecutive_failures += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    return Err(failure);
                }
            }
        }
    }

    Ok(())
}

async fn drain(client: &Client, url: &str, counter: &AtomicU64, deadline: Instant) -> Result<()> {
    let response = check(client.get(url).send().await?)?;

    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        counter.fetch_add(chunk?.len() as u64, Ordering::Relaxed);
        if Instant::now() >= deadline {
            break;
        }
    }
    Ok(())
}

async fn fill(
    client: &Client,
    url: &str,
    chunk: Bytes,
    counter: &Arc<AtomicU64>,
    deadline: Instant,
) -> Result<()> {
    let request = client
        .post(url)
        .header(CONTENT_TYPE, "application/octet-stream")
        .body(upload_body(chunk, Arc::clone(counter), deadline));
    check(request.send().await?)?;
    Ok(())
}

/// Streams `total` bytes, stopping early once the deadline passes.
///
/// Sending a chunked stream rather than one buffer lets a request be cut
/// mid-flight on a slow uplink, where a single 25 MB body could outlast the
/// whole measurement window.
fn upload_body(total: Bytes, counter: Arc<AtomicU64>, deadline: Instant) -> Body {
    Body::wrap_stream(stream::unfold(total.get(), move |remaining| {
        let counter = Arc::clone(&counter);
        async move {
            if remaining == 0 || Instant::now() >= deadline {
                return None;
            }
            let piece = remaining.min(UPLOAD_PIECE.len() as u64);
            counter.fetch_add(piece, Ordering::Relaxed);
            let sent: io::Result<&'static [u8]> = Ok(&UPLOAD_PIECE[..piece as usize]);
            Some((sent, remaining - piece))
        }
    }))
}

fn check(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else {
        Err(Error::UnexpectedStatus {
            status: status.as_u16(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reports_the_average_rate_so_far() {
        let progress = Progress {
            direction: Download::LABEL,
            transferred: Bytes::new(2_500_000),
            elapsed: Duration::from_secs(2),
        };
        assert_eq!(progress.rate().megabits_per_second(), 10.0);
    }

    #[test]
    fn the_default_plan_is_time_boxed() {
        let plan = Plan::default();
        assert_eq!(plan.duration, Duration::from_secs(10));
        assert_eq!(plan.connections.get(), 4);
    }

    #[test]
    fn each_direction_tag_selects_its_own_request() {
        assert_eq!(Download::KIND, Kind::Download);
        assert_eq!(Upload::KIND, Kind::Upload);
    }
}
