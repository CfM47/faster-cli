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
use reqwest::{Body, Client, StatusCode};
use tokio::task::JoinSet;

use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::units::{Bitrate, Bytes, Direction, Download, Throughput, Upload};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const MAX_CONSECUTIVE_FAILURES: usize = 3;
const STABILITY_WINDOW: usize = 8;
const STABILITY_TOLERANCE: f64 = 0.05;
const SETTLING_SPAN: Duration = Duration::from_secs(1);

/// Sent repeatedly to fill an upload body.
///
/// A static buffer rather than a freshly allocated one per piece: the bytes
/// are never read, so allocating them would only measure the allocator.
static UPLOAD_PIECE: [u8; 64 * 1024] = [0; 64 * 1024];

/// How a transfer phase should be run.
///
/// Fields are private and the only way to build one is [`Plan::default`], so
/// a plan whose warmup outlasts its own time bounds cannot be constructed.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// How long to transfer before the measurement window opens.
    warmup: Duration,
    /// How long to transfer before an early stop is allowed.
    min_duration: Duration,
    /// How long to transfer before stopping regardless of stability.
    max_duration: Duration,
    /// How many connections to end up saturating the link with.
    connections: NonZeroUsize,
    /// How many bytes to move per request.
    chunk: Bytes,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            warmup: Duration::from_secs(3),
            min_duration: Duration::from_secs(8),
            max_duration: Duration::from_secs(20),
            connections: NonZeroUsize::new(8).expect("8 is not zero"),
            chunk: Bytes::new(25_000_000),
        }
    }
}

/// Whether a sample is worth showing as a reading yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Connections are still opening and slow start has not finished, so the
    /// rate on offer is not yet one anybody should be shown.
    Warmup,
    /// The rate is averaged over enough of the measured window to stand.
    Measuring,
}

/// A snapshot of a transfer still in flight.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// Which phase produced this snapshot.
    pub direction: &'static str,
    /// Whether [`Progress::rate`] is settled enough to display.
    pub stage: Stage,
    /// Bytes moved since the phase began, including the warmup.
    pub transferred: Bytes,
    /// Time since the phase began, including the warmup.
    pub elapsed: Duration,
    /// The rate so far, measured the same way the final result is.
    ///
    /// Carried rather than derived from the two fields above so that the live
    /// display converges on the number that is ultimately reported instead of
    /// drifting away from it.
    pub rate: Bitrate,
}

/// Cumulative byte counts sampled while a phase runs.
#[derive(Debug, Default)]
struct Timeline {
    samples: Vec<Sample>,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    at: Duration,
    transferred: Bytes,
}

impl Timeline {
    fn record(&mut self, at: Duration, transferred: Bytes) {
        self.samples.push(Sample { at, transferred });
    }

    /// Returns the bytes moved after `warmup` and how long that took.
    ///
    /// Measuring a span rather than the whole run is what makes the reading
    /// honest: TCP slow start and the initial fill of the socket buffers are
    /// one-off costs at the start, so excluding them removes a bias that would
    /// otherwise shrink as the run got longer.
    fn measured(&self, warmup: Duration) -> Option<(Bytes, Duration)> {
        self.span_since(warmup).or_else(|| self.total())
    }

    fn span_since(&self, warmup: Duration) -> Option<(Bytes, Duration)> {
        let last = self.samples.last()?;
        let first = self.samples.iter().find(|sample| sample.at >= warmup)?;
        let elapsed = last.at.checked_sub(first.at)?;
        if elapsed.is_zero() {
            return None;
        }
        let moved = last
            .transferred
            .get()
            .checked_sub(first.transferred.get())?;
        Some((Bytes::new(moved), elapsed))
    }

    fn total(&self) -> Option<(Bytes, Duration)> {
        let last = self.samples.last()?;
        (!last.at.is_zero()).then_some((last.transferred, last.at))
    }
}

/// Returns whether the recent readings have settled.
///
/// Compares the spread of the trailing window against its own floor, so the
/// threshold means the same thing on a 5 Mbps link as on a 5 Gbps one.
fn has_converged(rates: &[f64], tolerance: f64) -> bool {
    if rates.len() < STABILITY_WINDOW {
        return false;
    }
    let window = &rates[rates.len() - STABILITY_WINDOW..];
    let highest = window.iter().copied().fold(f64::MIN, f64::max);
    let lowest = window.iter().copied().fold(f64::MAX, f64::min);
    if lowest <= 0.0 {
        return false;
    }
    (highest - lowest) / lowest <= tolerance
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
    let deadline = started + plan.max_duration;

    let mut workers = JoinSet::new();
    let mut open_connection = |workers: &mut JoinSet<Result<()>>| {
        let client = client.clone();
        let url = url.clone();
        let counter = Arc::clone(&counter);
        workers.spawn(async move {
            transfer_until(P::KIND, &client, &url, plan.chunk, &counter, deadline).await
        });
    };
    open_connection(&mut workers);

    let (timeline, failure) = supervise(
        &mut workers,
        &mut open_connection,
        &counter,
        started,
        plan,
        P::LABEL,
        report,
    )
    .await;

    let (transferred, elapsed) = timeline.measured(plan.warmup).unwrap_or((
        Bytes::new(counter.load(Ordering::Relaxed)),
        started.elapsed(),
    ));
    if transferred.is_zero() {
        return Err(failure.unwrap_or(Error::NoData {
            direction: P::LABEL,
        }));
    }
    Ok(Throughput::new(transferred, elapsed))
}

/// Samples the shared counter until the phase should end, then stops the workers.
///
/// Connections are opened one per sample rather than all at once, so their
/// slow starts are staggered instead of bursting into the same queue, and the
/// ramp is confined to the warmup: once the measurement window opens the
/// connection count is fixed, which is what makes the bytes counted inside it
/// comparable from one sample to the next.
///
/// Stops early once the reading has settled, so a steady link is not held at
/// full tilt for the maximum duration, but never before the minimum, so the
/// warmup is always excluded from a real measurement window.
///
/// The returned failure is the first one a worker reported, which only matters
/// if the phase ends up having transferred nothing at all.
async fn supervise(
    workers: &mut JoinSet<Result<()>>,
    open_connection: &mut dyn FnMut(&mut JoinSet<Result<()>>),
    counter: &AtomicU64,
    started: Instant,
    plan: Plan,
    direction: &'static str,
    report: &mut dyn FnMut(Progress),
) -> (Timeline, Option<Error>) {
    let mut timeline = Timeline::default();
    let mut rates = Vec::new();
    let mut failure = None;
    let mut opened = 1;

    loop {
        tokio::time::sleep(SAMPLE_INTERVAL).await;
        let elapsed = started.elapsed();
        let transferred = Bytes::new(counter.load(Ordering::Relaxed));
        timeline.record(elapsed, transferred);

        if opened < plan.connections.get() && elapsed < plan.warmup {
            open_connection(workers);
            opened += 1;
        }

        let window = timeline.span_since(plan.warmup);
        let (moved, over) = window
            .or_else(|| timeline.total())
            .unwrap_or((transferred, elapsed));
        let rate = Bitrate::from_transfer(moved, over);
        rates.push(rate.bits_per_second());
        report(Progress {
            direction,
            // A rate averaged over a sliver of the window swings wildly, so a
            // sample only counts as a reading once it covers enough of one.
            stage: match window {
                Some((_, covered)) if covered >= SETTLING_SPAN => Stage::Measuring,
                _ => Stage::Warmup,
            },
            transferred,
            elapsed,
            rate,
        });

        while let Some(joined) = workers.try_join_next() {
            if let Ok(Err(worker_failure)) = joined {
                failure.get_or_insert(worker_failure);
            }
        }

        let settled = elapsed >= plan.min_duration && has_converged(&rates, STABILITY_TOLERANCE);
        if workers.is_empty() || elapsed >= plan.max_duration || settled {
            break;
        }
    }

    workers.shutdown().await;
    (timeline, failure)
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
            // Retrying a refusal to serve is what the refusal is asking us not
            // to do, so it ends the phase rather than spending its attempts.
            Err(Error::RateLimited) => return Err(Error::RateLimited),
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
    match status {
        _ if status.is_success() => Ok(response),
        StatusCode::TOO_MANY_REQUESTS => Err(Error::RateLimited),
        _ => Err(Error::UnexpectedStatus {
            status: status.as_u16(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a timeline from `(second, cumulative megabyte)` pairs.
    fn timeline(points: &[(u64, u64)]) -> Timeline {
        let mut timeline = Timeline::default();
        for (second, megabytes) in points {
            timeline.record(
                Duration::from_secs(*second),
                Bytes::new(megabytes * 1_000_000),
            );
        }
        timeline
    }

    #[test]
    fn the_default_plan_warms_up_inside_its_own_bounds() {
        let plan = Plan::default();
        assert!(plan.warmup < plan.min_duration);
        assert!(plan.min_duration <= plan.max_duration);
    }

    #[test]
    fn the_connection_ramp_finishes_before_the_measurement_window_opens() {
        let plan = Plan::default();
        let ramp = SAMPLE_INTERVAL * (plan.connections.get() as u32 - 1);
        assert!(
            ramp < plan.warmup,
            "opening {} connections takes {ramp:?}, which spills past the {:?} warmup and would \
             leave the connection count changing inside the measured window",
            plan.connections,
            plan.warmup
        );
    }

    #[test]
    fn each_direction_tag_selects_its_own_request() {
        assert_eq!(Download::KIND, Kind::Download);
        assert_eq!(Upload::KIND, Kind::Upload);
    }

    #[test]
    fn the_measured_span_excludes_the_warmup() {
        // A link that crawls for 2s while slow start opens up, then settles
        // at 10 MB/s. Counting from zero would report 8.3 MB/s.
        let recorded = timeline(&[(0, 0), (1, 1), (2, 2), (3, 12), (4, 22), (5, 32)]);
        let (moved, over) = recorded
            .measured(Duration::from_secs(2))
            .expect("the span is well defined");
        assert_eq!(moved, Bytes::new(30_000_000));
        assert_eq!(over, Duration::from_secs(3));
        assert_eq!(
            Bitrate::from_transfer(moved, over).megabits_per_second(),
            80.0
        );
    }

    #[test]
    fn a_span_shorter_than_the_warmup_falls_back_to_the_whole_run() {
        let recorded = timeline(&[(0, 0), (1, 5)]);
        let (moved, over) = recorded
            .measured(Duration::from_secs(30))
            .expect("the total is still known");
        assert_eq!(moved, Bytes::new(5_000_000));
        assert_eq!(over, Duration::from_secs(1));
    }

    #[test]
    fn an_empty_timeline_measures_nothing() {
        assert!(Timeline::default().measured(Duration::ZERO).is_none());
    }

    #[test]
    fn convergence_needs_a_full_settled_window() {
        let settled = vec![100.0; STABILITY_WINDOW];
        assert!(has_converged(&settled, STABILITY_TOLERANCE));

        assert!(!has_converged(&settled[1..], STABILITY_TOLERANCE));

        let mut climbing = settled.clone();
        climbing.push(140.0);
        assert!(!has_converged(&climbing, STABILITY_TOLERANCE));
    }

    #[test]
    fn convergence_is_judged_relative_to_the_link_speed() {
        // The same 2 Mbps spread settles a slow link but not a fast one.
        let fast: Vec<f64> = (0..STABILITY_WINDOW)
            .map(|index| 1_000.0 + index as f64 * 0.25)
            .collect();
        assert!(has_converged(&fast, STABILITY_TOLERANCE));

        let slow: Vec<f64> = (0..STABILITY_WINDOW)
            .map(|index| 10.0 + index as f64 * 0.25)
            .collect();
        assert!(!has_converged(&slow, STABILITY_TOLERANCE));
    }
}
