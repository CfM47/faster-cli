//! Running the phases of a speed test in order.

use std::time::Instant;

use crate::endpoint::{Endpoint, http_client};
use crate::error::Result;
use crate::probe::latency;
use crate::probe::transfer::{self, Plan, Progress};
use crate::report::Report;

/// How much of a test to run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Measure latency, download and upload.
    #[default]
    Full,
    /// Skip the upload phase.
    DownloadOnly,
}

/// Something worth showing while a run is in progress.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// Contacting the endpoint and timing round trips.
    Probing,
    /// A transfer phase produced a sample.
    Transferring(Progress),
}

/// Runs a speed test and collects the result.
///
/// `observe` is called as each phase makes progress so a caller can draw a
/// live display; pass `&mut |_| {}` when nothing is watching.
///
/// Cancellation is the caller's to handle: dropping this future aborts every
/// request in flight, so racing it against a signal needs no plumbing here.
pub async fn run(scope: Scope, observe: &mut dyn FnMut(Event)) -> Result<Report> {
    let endpoint = Endpoint::cloudflare();
    let client = http_client()?;
    let started = Instant::now();

    observe(Event::Probing);
    // Connection metadata only decorates the verbose output, so a server that
    // declines to describe itself must not cost the user their measurement.
    let client_info = endpoint.client_info(&client).await.unwrap_or_default();
    let latency = latency::measure(&endpoint).await?;

    let plan = Plan::default();
    let download = transfer::download(&client, &endpoint, plan, &mut |progress| {
        observe(Event::Transferring(progress));
    })
    .await?;

    let upload = match scope {
        Scope::DownloadOnly => None,
        Scope::Full => Some(
            transfer::upload(&client, &endpoint, plan, &mut |progress| {
                observe(Event::Transferring(progress));
            })
            .await?,
        ),
    };

    Ok(Report::new(
        endpoint,
        client_info,
        latency,
        download,
        upload,
        started.elapsed(),
    ))
}
