use std::time::Duration;

use reqwest::{
    StatusCode,
    blocking::{Client, RequestBuilder},
};

use crate::{
    TeralionCredential, TeralionQuery, TeralionRequest, TeralionTransport, TransportError,
};

pub const TERALION_BASE_URL: &str = "https://app.teraliontech.com";

#[derive(Debug, Clone)]
pub struct FeedArchiveTransport {
    client: Client,
    base_url: Box<str>,
}

impl FeedArchiveTransport {
    pub fn new() -> Result<Self, TransportError> {
        Self::with_base_url(TERALION_BASE_URL)
    }

    pub fn with_base_url(base_url: &str) -> Result<Self, TransportError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| TransportError::new(false, error.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').into(),
        })
    }

    fn request(&self, request: &TeralionRequest) -> RequestBuilder {
        let builder = match request.query() {
            TeralionQuery::Coverage { start, end } => self
                .client
                .get(format!("{}/api/feed/coverage", self.base_url))
                .query(&[("start", start.to_string()), ("end", end.to_string())]),
            TeralionQuery::SymbolRange { instrument } => self.client.get(format!(
                "{}/api/feed/range/{}",
                self.base_url,
                instrument.symbol()
            )),
            TeralionQuery::Ticks {
                instrument,
                start,
                end,
                kinds,
                limit,
            } => self
                .client
                .get(format!(
                    "{}/api/feed/ticks/{}",
                    self.base_url,
                    instrument.symbol()
                ))
                .query(&[
                    ("start", start.as_str().to_owned()),
                    ("end", end.as_str().to_owned()),
                    (
                        "kinds",
                        kinds
                            .iter()
                            .map(|kind| kind.slug())
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                    ("limit", limit.to_string()),
                ]),
            TeralionQuery::DailyInstrument {
                instrument,
                trading_date,
            } => self
                .client
                .get(format!(
                    "{}/api/feed/instruments/{}",
                    self.base_url,
                    instrument.symbol()
                ))
                .query(&[("date", trading_date.to_string())]),
        };
        if let Some(cursor) = request.cursor() {
            builder.query(&[("cursor", cursor)])
        } else {
            builder
        }
    }
}

impl TeralionTransport for FeedArchiveTransport {
    fn execute(
        &mut self,
        request: &TeralionRequest,
        credential: &TeralionCredential,
    ) -> Result<Vec<u8>, TransportError> {
        let response = self
            .request(request)
            .header("X-API-Key", credential.expose_secret())
            .send()
            .map_err(|error| {
                TransportError::new(error.is_timeout() || error.is_connect(), error.to_string())
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(TransportError::new(
                retryable_status(status),
                format!("Teralion request failed with HTTP {}", status.as_u16()),
            ));
        }
        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|error| TransportError::new(true, error.to_string()))
    }
}

fn retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}
