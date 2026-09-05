//! Presenting a finished report.

use std::time::Duration;

use crate::report::Report;

const LABEL_WIDTH: usize = 10;

/// How much of a report to show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Detail {
    /// Just the measurements.
    #[default]
    Plain,
    /// The measurements plus where they were taken.
    Verbose,
}

/// Formats a finished report as lines for a person to read.
pub fn summary(report: &Report, detail: Detail) -> Vec<String> {
    let latency = report.latency();
    let mut lines = vec![
        row(
            "Latency",
            &format!(
                "{} (±{} jitter)",
                milliseconds(latency.median()),
                milliseconds(latency.jitter())
            ),
        ),
        row("Download", &report.download().bitrate().to_string()),
    ];

    // A skipped upload leaves no row rather than a zero, so the output never
    // implies a measurement that was not taken.
    if let Some(upload) = report.upload() {
        lines.push(row("Upload", &upload.bitrate().to_string()));
    }

    if detail == Detail::Verbose {
        let client = report.client();
        lines.push(row(
            "Server",
            &annotate(report.endpoint().host(), client.colo.as_deref()),
        ));
        lines.push(row(
            "Client",
            &annotate(
                client.ip.as_deref().unwrap_or("unknown"),
                client.country.as_deref(),
            ),
        ));
        lines.push(row(
            "Duration",
            &format!("{:.1}s", report.duration().as_secs_f64()),
        ));
    }

    lines
}

fn row(label: &str, value: &str) -> String {
    format!("{label:<LABEL_WIDTH$} {value}")
}

fn annotate(value: &str, note: Option<&str>) -> String {
    match note {
        Some(note) => format!("{value} ({note})"),
        None => value.to_owned(),
    }
}

fn milliseconds(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::{ClientInfo, Endpoint};
    use crate::probe::latency::Latency;
    use crate::units::{Bytes, Throughput};

    fn report(upload: bool, client: ClientInfo) -> Report {
        Report::new(
            Endpoint::cloudflare(),
            client,
            Latency::from_samples(vec![
                Duration::from_millis(12),
                Duration::from_millis(14),
                Duration::from_millis(13),
            ])
            .expect("samples were provided"),
            Throughput::new(Bytes::new(125_000_000), Duration::from_secs(5)),
            upload.then(|| Throughput::new(Bytes::new(25_000_000), Duration::from_secs(5))),
            Duration::from_millis(24_500),
        )
    }

    fn located() -> ClientInfo {
        ClientInfo {
            ip: Some("203.0.113.7".to_owned()),
            colo: Some("YYZ".to_owned()),
            country: Some("CA".to_owned()),
        }
    }

    #[test]
    fn a_plain_summary_is_only_the_measurements() {
        assert_eq!(
            summary(&report(true, located()), Detail::Plain),
            [
                "Latency    13.0 ms (±1.5 ms jitter)",
                "Download   200.0 Mbps",
                "Upload     40.0 Mbps",
            ]
        );
    }

    #[test]
    fn a_verbose_summary_says_where_it_was_measured() {
        assert_eq!(
            summary(&report(true, located()), Detail::Verbose)[3..],
            [
                "Server     speed.cloudflare.com (YYZ)",
                "Client     203.0.113.7 (CA)",
                "Duration   24.5s",
            ]
        );
    }

    #[test]
    fn a_skipped_upload_leaves_no_row() {
        let lines = summary(&report(false, located()), Detail::Plain);
        assert!(!lines.iter().any(|line| line.starts_with("Upload")));
    }

    #[test]
    fn missing_metadata_degrades_instead_of_printing_empty_brackets() {
        assert_eq!(
            summary(&report(false, ClientInfo::default()), Detail::Verbose)[2..],
            [
                "Server     speed.cloudflare.com",
                "Client     unknown",
                "Duration   24.5s",
            ]
        );
    }
}
