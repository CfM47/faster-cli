//! The server side of a measurement.
//!
//! Unlike the Netflix API the Node version used, no token is needed and no
//! server list has to be ranked: the host is anycast, so the network already
//! routes each client to a nearby point of presence.

use reqwest::Client;

use crate::error::{Error, Result};

const CLOUDFLARE_HOST: &str = "speed.cloudflare.com";
const HTTPS_PORT: u16 = 443;
const USER_AGENT: &str = concat!("faster-cli/", env!("CARGO_PKG_VERSION"));

/// A host exposing download, upload and trace routes.
#[derive(Clone, Debug)]
pub struct Endpoint {
    host: String,
}

impl Endpoint {
    /// Returns the Cloudflare speed test host.
    pub fn cloudflare() -> Self {
        Self {
            host: CLOUDFLARE_HOST.to_owned(),
        }
    }

    /// Returns the host name, for latency probes that dial it directly.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the `host:port` pair to open a TCP connection to.
    pub fn socket_address(&self) -> String {
        format!("{}:{HTTPS_PORT}", self.host)
    }

    /// Returns the URL serving a body of exactly `bytes` bytes.
    pub fn download_url(&self, bytes: u64) -> String {
        format!("https://{}/__down?bytes={bytes}", self.host)
    }

    /// Returns the URL that discards whatever body is posted to it.
    pub fn upload_url(&self) -> String {
        format!("https://{}/__up", self.host)
    }

    /// Fetches the connection metadata the server sees for this client.
    pub async fn client_info(&self, client: &Client) -> Result<ClientInfo> {
        let response = client
            .get(format!("https://{}/cdn-cgi/trace", self.host))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::UnexpectedStatus {
                status: status.as_u16(),
            });
        }
        ClientInfo::from_trace(&response.text().await?)
    }
}

/// What the edge server reports about the connection.
///
/// Every field is optional because this is diagnostic detail shown under
/// `--verbose`; a trace missing a key must not fail the measurement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientInfo {
    /// Public IP address the request arrived from.
    pub ip: Option<String>,
    /// IATA code of the point of presence that served the request.
    pub colo: Option<String>,
    /// Two letter country code the request was geolocated to.
    pub country: Option<String>,
}

impl ClientInfo {
    /// Parses the `key=value` lines of a `/cdn-cgi/trace` response.
    fn from_trace(body: &str) -> Result<Self> {
        let mut info = Self::default();
        let mut recognised_any_pair = false;

        for (key, value) in body.lines().filter_map(|line| line.split_once('=')) {
            recognised_any_pair = true;
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key {
                "ip" => info.ip = Some(value.to_owned()),
                "colo" => info.colo = Some(value.to_owned()),
                "loc" => info.country = Some(value.to_owned()),
                _ => {}
            }
        }

        if recognised_any_pair {
            Ok(info)
        } else {
            Err(Error::MalformedResponse {
                detail: "trace response contained no key=value pairs".to_owned(),
            })
        }
    }
}

/// Builds the HTTP client every phase shares.
///
/// HTTP/2 is refused on purpose: it would multiplex all parallel transfers
/// onto a single TCP connection, so the concurrency the measurement relies on
/// to saturate the link would collapse into one congestion window.
pub fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .http1_only()
        .tcp_nodelay(true)
        .build()
        .map_err(Error::Transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_target_the_speed_test_routes() {
        let endpoint = Endpoint::cloudflare();
        assert_eq!(
            endpoint.download_url(25_000_000),
            "https://speed.cloudflare.com/__down?bytes=25000000"
        );
        assert_eq!(endpoint.upload_url(), "https://speed.cloudflare.com/__up");
        assert_eq!(endpoint.socket_address(), "speed.cloudflare.com:443");
    }

    #[test]
    fn trace_yields_the_fields_shown_under_verbose() {
        let info = ClientInfo::from_trace("fl=786f196\nip=203.0.113.7\ncolo=YYZ\nloc=CA\nwarp=off")
            .expect("trace is well formed");
        assert_eq!(info.ip.as_deref(), Some("203.0.113.7"));
        assert_eq!(info.colo.as_deref(), Some("YYZ"));
        assert_eq!(info.country.as_deref(), Some("CA"));
    }

    #[test]
    fn missing_trace_keys_are_absent_rather_than_fatal() {
        let info = ClientInfo::from_trace("fl=786f196\nip=\nwarp=off").expect("trace is parseable");
        assert_eq!(info, ClientInfo::default());
    }

    #[test]
    fn a_trace_without_pairs_is_rejected() {
        assert!(matches!(
            ClientInfo::from_trace("<html>an error page</html>"),
            Err(Error::MalformedResponse { .. })
        ));
    }
}
