//! The outcome of a completed speed test.

use std::time::Duration;

use serde::{Serialize, Serializer};

use crate::endpoint::{ClientInfo, Endpoint};
use crate::probe::latency::Latency;
use crate::units::{Download, Throughput, Upload};

/// Everything a finished run has to say.
///
/// Holds the measurements in their own types rather than as loose numbers, so
/// the upload being absent is the type saying it was never run, not a zero
/// that has to be interpreted.
#[derive(Clone, Debug)]
pub struct Report {
    endpoint: Endpoint,
    client: ClientInfo,
    latency: Latency,
    download: Throughput<Download>,
    upload: Option<Throughput<Upload>>,
    duration: Duration,
}

impl Report {
    /// Assembles a report from the measurements a run produced.
    pub fn new(
        endpoint: Endpoint,
        client: ClientInfo,
        latency: Latency,
        download: Throughput<Download>,
        upload: Option<Throughput<Upload>>,
        duration: Duration,
    ) -> Self {
        Self {
            endpoint,
            client,
            latency,
            download,
            upload,
            duration,
        }
    }

    /// Returns the host that served the test.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Returns what the server reported about this connection.
    pub fn client(&self) -> &ClientInfo {
        &self.client
    }

    /// Returns the round trip time and its variation.
    pub fn latency(&self) -> Latency {
        self.latency
    }

    /// Returns the download measurement.
    pub fn download(&self) -> Throughput<Download> {
        self.download
    }

    /// Returns the upload measurement, absent when the phase was skipped.
    pub fn upload(&self) -> Option<Throughput<Upload>> {
        self.upload
    }

    /// Returns how long the whole run took, probing included.
    pub fn duration(&self) -> Duration {
        self.duration
    }
}

/// The JSON representation, kept separate from the measurement types.
///
/// Defining the wire format in one struct rather than deriving `Serialize`
/// across the domain means the output a script depends on cannot change as a
/// side effect of refactoring a field somewhere else.
#[derive(Serialize)]
struct Wire<'a> {
    server: ServerWire<'a>,
    client: ClientWire<'a>,
    latency_ms: f64,
    jitter_ms: f64,
    download_mbps: f64,
    downloaded_bytes: u64,
    upload_mbps: Option<f64>,
    uploaded_bytes: Option<u64>,
    duration_s: f64,
}

#[derive(Serialize)]
struct ServerWire<'a> {
    host: &'a str,
    /// Airport code of the point of presence that served the test.
    colo: Option<&'a str>,
}

#[derive(Serialize)]
struct ClientWire<'a> {
    ip: Option<&'a str>,
    country: Option<&'a str>,
}

impl<'a> From<&'a Report> for Wire<'a> {
    fn from(report: &'a Report) -> Self {
        Self {
            server: ServerWire {
                host: report.endpoint.host(),
                colo: report.client.colo.as_deref(),
            },
            client: ClientWire {
                ip: report.client.ip.as_deref(),
                country: report.client.country.as_deref(),
            },
            latency_ms: milliseconds(report.latency.median()),
            jitter_ms: milliseconds(report.latency.jitter()),
            download_mbps: round(report.download.bitrate().megabits_per_second()),
            downloaded_bytes: report.download.bytes().get(),
            upload_mbps: report
                .upload
                .map(|upload| round(upload.bitrate().megabits_per_second())),
            uploaded_bytes: report.upload.map(|upload| upload.bytes().get()),
            duration_s: round(report.duration.as_secs_f64()),
        }
    }
}

impl Serialize for Report {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Wire::from(self).serialize(serializer)
    }
}

fn milliseconds(duration: Duration) -> f64 {
    round(duration.as_secs_f64() * 1_000.0)
}

/// Rounds to two decimals.
///
/// Full float precision would leak the sampling noise into output a script
/// compares against, while the raw byte counts stay exact for anyone who needs
/// to recompute the rate themselves.
fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Bytes;

    fn report(upload: Option<Throughput<Upload>>) -> Report {
        Report::new(
            Endpoint::cloudflare(),
            ClientInfo {
                ip: Some("203.0.113.7".to_owned()),
                colo: Some("YYZ".to_owned()),
                country: Some("CA".to_owned()),
            },
            Latency::from_samples(vec![
                Duration::from_millis(12),
                Duration::from_millis(14),
                Duration::from_millis(13),
            ])
            .expect("samples were provided"),
            Throughput::new(Bytes::new(125_000_000), Duration::from_secs(5)),
            upload,
            Duration::from_secs(24),
        )
    }

    #[test]
    fn json_carries_every_measurement() {
        let measured = report(Some(Throughput::new(
            Bytes::new(25_000_000),
            Duration::from_secs(5),
        )));
        let json = serde_json::to_value(&measured).expect("a report serializes");

        assert_eq!(json["server"]["host"], "speed.cloudflare.com");
        assert_eq!(json["server"]["colo"], "YYZ");
        assert_eq!(json["client"]["ip"], "203.0.113.7");
        assert_eq!(json["client"]["country"], "CA");
        assert_eq!(json["latency_ms"], 13.0);
        assert_eq!(json["jitter_ms"], 1.5);
        assert_eq!(json["download_mbps"], 200.0);
        assert_eq!(json["downloaded_bytes"], 125_000_000u64);
        assert_eq!(json["upload_mbps"], 40.0);
        assert_eq!(json["uploaded_bytes"], 25_000_000u64);
        assert_eq!(json["duration_s"], 24.0);
    }

    #[test]
    fn a_skipped_upload_is_null_rather_than_missing() {
        let json = serde_json::to_value(report(None)).expect("a report serializes");

        // Present-but-null keeps `jq .upload_mbps` working for callers instead
        // of making them distinguish an absent key from a measured zero.
        assert!(json.get("upload_mbps").is_some());
        assert!(json["upload_mbps"].is_null());
        assert!(json["uploaded_bytes"].is_null());
    }

    #[test]
    fn readings_are_rounded_but_byte_counts_stay_exact() {
        let measured = Report::new(
            Endpoint::cloudflare(),
            ClientInfo::default(),
            Latency::from_samples(vec![Duration::from_micros(12_345)]).expect("one sample"),
            Throughput::new(Bytes::new(123_456_789), Duration::from_secs(7)),
            None,
            Duration::from_millis(21_499),
        );
        let json = serde_json::to_value(&measured).expect("a report serializes");

        assert_eq!(json["latency_ms"], 12.35);
        assert_eq!(json["download_mbps"], 141.09);
        assert_eq!(json["downloaded_bytes"], 123_456_789u64);
        assert_eq!(json["duration_s"], 21.5);
    }
}
