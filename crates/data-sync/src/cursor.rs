use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
};

use serde_json::Value;

use crate::{ArchiveKind, SanitizedQueryIdentity, TeralionCredential, TeralionQuery};

#[derive(Clone, PartialEq, Eq)]
pub struct TeralionCursor(Box<str>);

impl TeralionCursor {
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, CursorError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CursorError::EmptyCursor);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn digest(&self) -> [u8; 32] {
        *blake3::hash(self.0.as_bytes()).as_bytes()
    }
}

impl fmt::Debug for TeralionCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TeralionCursor([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeralionRequest {
    query: TeralionQuery,
    query_identity: SanitizedQueryIdentity,
    cursor: Option<TeralionCursor>,
}

impl TeralionRequest {
    #[must_use]
    pub fn first(query: TeralionQuery) -> Self {
        Self {
            query_identity: query.identity(),
            query,
            cursor: None,
        }
    }

    #[must_use]
    pub const fn query(&self) -> &TeralionQuery {
        &self.query
    }

    #[must_use]
    pub const fn query_identity(&self) -> SanitizedQueryIdentity {
        self.query_identity
    }

    #[must_use]
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_ref().map(TeralionCursor::as_str)
    }
}

pub trait TeralionTransport {
    fn execute(
        &mut self,
        request: &TeralionRequest,
        credential: &TeralionCredential,
    ) -> Result<Vec<u8>, TransportError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    retryable: bool,
    message: Box<str>,
}

impl TransportError {
    #[must_use]
    pub fn new(retryable: bool, message: impl Into<Box<str>>) -> Self {
        Self {
            retryable,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for TransportError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorState {
    Ready,
    RequestInFlight,
    AwaitingDurableCommit,
    Terminal,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPage {
    ordinal: u32,
    query_identity: SanitizedQueryIdentity,
    body: Box<[u8]>,
    body_fingerprint: [u8; 32],
    record_count: u64,
    next_cursor: Option<TeralionCursor>,
}

impl PendingPage {
    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub const fn query_identity(&self) -> SanitizedQueryIdentity {
        self.query_identity
    }

    #[must_use]
    pub const fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub const fn body_fingerprint(&self) -> &[u8; 32] {
        &self.body_fingerprint
    }

    #[must_use]
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_ref().map(TeralionCursor::as_str)
    }

    #[must_use]
    pub const fn commit_receipt(&self) -> PageCommitReceipt {
        PageCommitReceipt {
            ordinal: self.ordinal,
            body_fingerprint: self.body_fingerprint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCommitReceipt {
    ordinal: u32,
    body_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorCheckpoint {
    query_identity: SanitizedQueryIdentity,
    committed_pages: u32,
    next_cursor: Option<TeralionCursor>,
    seen_cursor_digests: BTreeSet<[u8; 32]>,
    seen_page_fingerprints: BTreeSet<[u8; 32]>,
    terminal: bool,
}

impl CursorCheckpoint {
    #[must_use]
    pub const fn query_identity(&self) -> SanitizedQueryIdentity {
        self.query_identity
    }

    #[must_use]
    pub const fn committed_pages(&self) -> u32 {
        self.committed_pages
    }

    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_ref().map(TeralionCursor::as_str)
    }

    #[must_use]
    pub const fn terminal(&self) -> bool {
        self.terminal
    }

    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        let value = serde_json::json!({
            "query_identity": hex(self.query_identity.as_bytes()),
            "committed_pages": self.committed_pages,
            "next_cursor": self.next_cursor.as_ref().map(TeralionCursor::as_str),
            "seen_cursor_digests": self.seen_cursor_digests.iter().map(|value| hex(value)).collect::<Vec<_>>(),
            "seen_page_fingerprints": self.seen_page_fingerprints.iter().map(|value| hex(value)).collect::<Vec<_>>(),
            "terminal": self.terminal,
        });
        let bytes = serde_json::to_vec_pretty(&value).map_err(io::Error::other)?;
        let temporary = path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temporary, path)?;
        File::open(path.parent().expect("checkpoint has parent"))?.sync_all()
    }

    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        let required_text = |field: &str| {
            value
                .get(field)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, field.to_owned()))
        };
        let digest_set = |field: &str| -> Result<BTreeSet<[u8; 32]>, io::Error> {
            value
                .get(field)
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, field.to_owned()))?
                .iter()
                .map(|item| {
                    item.as_str()
                        .and_then(decode_hex_32)
                        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, field.to_owned()))
                })
                .collect()
        };
        Ok(Self {
            query_identity: SanitizedQueryIdentity::from_bytes(
                decode_hex_32(required_text("query_identity")?)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "query identity"))?,
            ),
            committed_pages: value["committed_pages"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "committed_pages"))?,
            next_cursor: value
                .get("next_cursor")
                .and_then(serde_json::Value::as_str)
                .map(TeralionCursor::new)
                .transpose()
                .map_err(io::Error::other)?,
            seen_cursor_digests: digest_set("seen_cursor_digests")?,
            seen_page_fingerprints: digest_set("seen_page_fingerprints")?,
            terminal: value["terminal"]
                .as_bool()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "terminal"))?,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex_32(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut output = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = decode_nibble(pair[0])? << 4 | decode_nibble(pair[1])?;
    }
    Some(output)
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct CursorStateMachine {
    query: TeralionQuery,
    checkpoint: CursorCheckpoint,
    state: CursorState,
    pending: Option<PendingPage>,
}

impl CursorStateMachine {
    pub fn new(query: TeralionQuery) -> Result<Self, CursorError> {
        if !query.is_paged() {
            return Err(CursorError::QueryIsNotPaged);
        }
        Ok(Self {
            checkpoint: CursorCheckpoint {
                query_identity: query.identity(),
                committed_pages: 0,
                next_cursor: None,
                seen_cursor_digests: BTreeSet::new(),
                seen_page_fingerprints: BTreeSet::new(),
                terminal: false,
            },
            query,
            state: CursorState::Ready,
            pending: None,
        })
    }

    pub fn resume(query: TeralionQuery, checkpoint: CursorCheckpoint) -> Result<Self, CursorError> {
        if query.identity() != checkpoint.query_identity {
            return Err(CursorError::QueryDrift);
        }
        if checkpoint.terminal {
            return Ok(Self {
                query,
                checkpoint,
                state: CursorState::Terminal,
                pending: None,
            });
        }
        Ok(Self {
            query,
            checkpoint,
            state: CursorState::Ready,
            pending: None,
        })
    }

    #[must_use]
    pub const fn state(&self) -> CursorState {
        self.state
    }

    #[must_use]
    pub const fn checkpoint(&self) -> &CursorCheckpoint {
        &self.checkpoint
    }

    pub fn request_next(&mut self) -> Result<TeralionRequest, CursorError> {
        if self.state != CursorState::Ready {
            return Err(CursorError::InvalidTransition {
                state: self.state,
                action: "request_next",
            });
        }
        self.state = CursorState::RequestInFlight;
        Ok(TeralionRequest {
            query: self.query.clone(),
            query_identity: self.checkpoint.query_identity,
            cursor: self.checkpoint.next_cursor.clone(),
        })
    }

    pub fn request_failed(&mut self, request: &TeralionRequest) -> Result<(), CursorError> {
        self.validate_request(request)?;
        if self.state != CursorState::RequestInFlight {
            return Err(CursorError::InvalidTransition {
                state: self.state,
                action: "request_failed",
            });
        }
        self.state = CursorState::Ready;
        Ok(())
    }

    pub fn accept_response(
        &mut self,
        request: &TeralionRequest,
        body: Vec<u8>,
    ) -> Result<&PendingPage, CursorError> {
        self.validate_request(request)?;
        if self.state != CursorState::RequestInFlight {
            return Err(CursorError::InvalidTransition {
                state: self.state,
                action: "accept_response",
            });
        }
        let parsed = parse_page(&self.query, &body)?;
        let body_fingerprint = *blake3::hash(&body).as_bytes();
        if self
            .checkpoint
            .seen_page_fingerprints
            .contains(&body_fingerprint)
        {
            self.state = CursorState::Invalid;
            return Err(CursorError::DuplicatePage);
        }
        if let Some(next) = &parsed.next_cursor {
            let digest = next.digest();
            if request.cursor.as_ref() == Some(next)
                || self.checkpoint.seen_cursor_digests.contains(&digest)
            {
                self.state = CursorState::Invalid;
                return Err(CursorError::CursorDidNotAdvance);
            }
        }
        self.pending = Some(PendingPage {
            ordinal: self.checkpoint.committed_pages,
            query_identity: self.checkpoint.query_identity,
            body: body.into_boxed_slice(),
            body_fingerprint,
            record_count: parsed.record_count,
            next_cursor: parsed.next_cursor,
        });
        self.state = CursorState::AwaitingDurableCommit;
        Ok(self.pending.as_ref().expect("pending page was just set"))
    }

    pub fn commit_page(&mut self, receipt: PageCommitReceipt) -> Result<(), CursorError> {
        if self.state != CursorState::AwaitingDurableCommit {
            return Err(CursorError::InvalidTransition {
                state: self.state,
                action: "commit_page",
            });
        }
        let pending = self.pending.take().expect("pending state has a page");
        if receipt.ordinal != pending.ordinal
            || receipt.body_fingerprint != pending.body_fingerprint
        {
            self.pending = Some(pending);
            return Err(CursorError::CommitReceiptMismatch);
        }
        self.checkpoint
            .seen_page_fingerprints
            .insert(pending.body_fingerprint);
        if let Some(cursor) = &pending.next_cursor {
            self.checkpoint.seen_cursor_digests.insert(cursor.digest());
        }
        self.checkpoint.committed_pages += 1;
        self.checkpoint.next_cursor = pending.next_cursor;
        self.checkpoint.terminal = self.checkpoint.next_cursor.is_none();
        self.state = if self.checkpoint.terminal {
            CursorState::Terminal
        } else {
            CursorState::Ready
        };
        Ok(())
    }

    fn validate_request(&mut self, request: &TeralionRequest) -> Result<(), CursorError> {
        if request.query_identity != self.checkpoint.query_identity
            || request.query.identity() != self.checkpoint.query_identity
        {
            self.state = CursorState::Invalid;
            return Err(CursorError::QueryDrift);
        }
        if request.cursor != self.checkpoint.next_cursor {
            self.state = CursorState::Invalid;
            return Err(CursorError::CursorDrift);
        }
        Ok(())
    }
}

struct ParsedPage {
    record_count: u64,
    next_cursor: Option<TeralionCursor>,
}

fn parse_page(query: &TeralionQuery, body: &[u8]) -> Result<ParsedPage, CursorError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| CursorError::InvalidJson(error.to_string()))?;
    let object = value.as_object().ok_or(CursorError::InvalidEnvelope)?;
    let items = object
        .get("items")
        .and_then(Value::as_array)
        .ok_or(CursorError::InvalidEnvelope)?;
    if let TeralionQuery::Ticks {
        instrument, kinds, ..
    } = query
    {
        for item in items {
            validate_tick(
                item,
                instrument,
                kinds,
                query
                    .archive_market()
                    .expect("tick query has a source market"),
            )?;
        }
    }
    let next_cursor = match object.get("next_cursor") {
        Some(Value::Null) => None,
        Some(Value::String(value)) => Some(TeralionCursor::new(value.as_str())?),
        _ => return Err(CursorError::InvalidEnvelope),
    };
    Ok(ParsedPage {
        record_count: items.len() as u64,
        next_cursor,
    })
}

fn validate_tick(
    item: &Value,
    instrument: &market_types::InstrumentId,
    kinds: &[ArchiveKind],
    archive_market: crate::ArchiveMarket,
) -> Result<(), CursorError> {
    let item = item.as_object().ok_or(CursorError::InvalidTickEnvelope)?;
    let kind = required_string(item, "type")?;
    if !kinds.iter().any(|expected| expected.slug() == kind) {
        return Err(CursorError::TickIdentityMismatch("type"));
    }
    let market = required_string(item, "market")?;
    let expected_market = archive_market.wire_market();
    if market != expected_market {
        return Err(CursorError::TickIdentityMismatch("market"));
    }
    if required_string(item, "symbol")? != instrument.symbol().as_str() {
        return Err(CursorError::TickIdentityMismatch("symbol"));
    }
    for field in ["format", "match_time", "received_at"] {
        if required_string(item, field)?.is_empty() {
            return Err(CursorError::InvalidTickEnvelope);
        }
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, CursorError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(CursorError::MissingTickField(field))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorError {
    QueryIsNotPaged,
    EmptyCursor,
    QueryDrift,
    CursorDrift,
    CursorDidNotAdvance,
    DuplicatePage,
    InvalidEnvelope,
    InvalidTickEnvelope,
    MissingTickField(&'static str),
    TickIdentityMismatch(&'static str),
    InvalidJson(String),
    CommitReceiptMismatch,
    InvalidTransition {
        state: CursorState,
        action: &'static str,
    },
}

impl fmt::Display for CursorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for CursorError {}

#[cfg(test)]
mod tests {
    use market_types::{InstrumentId, MarketId, Symbol};

    use super::*;
    use crate::{ArchiveMarket, ArchiveTimestamp};

    fn query() -> TeralionQuery {
        TeralionQuery::ticks(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote],
            5_000,
        )
        .unwrap()
    }

    fn body(next: Option<&str>) -> Vec<u8> {
        let next = next
            .map(|value| format!("\"{value}\""))
            .unwrap_or_else(|| "null".to_owned());
        format!(
            r#"{{"items":[{{"type":"quote","market":"twse","format":"STOCK_SNAPSHOT","symbol":"2330","match_time":"2026-07-27T09:00:00+08:00","received_at":"2026-07-27T09:00:00+08:00"}}],"next_cursor":{next}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn taifex_query_accepts_wire_kinds_and_market() {
        let query = TeralionQuery::ticks(
            InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap()),
            ArchiveTimestamp::parse("2026-07-20T08:40:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-20T13:50:00+08:00").unwrap(),
            [
                ArchiveKind::Book,
                ArchiveKind::Close,
                ArchiveKind::Stats,
                ArchiveKind::Trade,
            ],
            5_000,
        )
        .unwrap();
        let body = br#"{"items":[{"type":"book","market":"taifex_fut","format":"I080","symbol":"TXFH6","match_time":"2026-07-20T08:45:00+08:00","received_at":"2026-07-20T08:45:00+08:00"}],"next_cursor":null}"#;
        let mut machine = CursorStateMachine::new(query).unwrap();
        let request = machine.request_next().unwrap();
        let pending = machine.accept_response(&request, body.to_vec()).unwrap();
        assert_eq!(pending.record_count(), 1);
    }

    #[test]
    fn tpex_query_accepts_quote_wire_kind_and_market() {
        let query = TeralionQuery::ticks(
            InstrumentId::new(MarketId::Tpex, Symbol::new("6488").unwrap()),
            ArchiveTimestamp::parse("2026-07-20T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-20T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote],
            5_000,
        )
        .unwrap();
        let body = br#"{"items":[{"type":"quote","market":"tpex","format":"STOCK_REALTIME","symbol":"6488","match_time":"2026-07-20T09:00:00+08:00","received_at":"2026-07-20T09:00:00+08:00"}],"next_cursor":null}"#;
        let mut machine = CursorStateMachine::new(query).unwrap();
        let request = machine.request_next().unwrap();
        let pending = machine.accept_response(&request, body.to_vec()).unwrap();
        assert_eq!(pending.record_count(), 1);
    }

    #[test]
    fn explicit_option_query_rejects_futures_market_payload() {
        let query = TeralionQuery::ticks_for_market(
            InstrumentId::new(MarketId::Taifex, Symbol::new("TXO24000U6").unwrap()),
            ArchiveTimestamp::parse("2026-07-27T14:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-28T13:50:00+08:00").unwrap(),
            [ArchiveKind::Book, ArchiveKind::Trade],
            5_000,
            ArchiveMarket::TaifexOptions,
        )
        .unwrap();
        let body = br#"{"items":[{"type":"book","market":"taifex_fut","format":"I080","symbol":"TXO24000U6","match_time":"2026-07-28T09:00:00+08:00","received_at":"2026-07-28T09:00:00+08:00"}],"next_cursor":null}"#;
        let mut machine = CursorStateMachine::new(query).unwrap();
        let request = machine.request_next().unwrap();
        assert!(matches!(
            machine.accept_response(&request, body.to_vec()),
            Err(CursorError::TickIdentityMismatch("market"))
        ));
    }

    #[test]
    fn cursor_only_advances_after_durable_commit() {
        let mut machine = CursorStateMachine::new(query()).unwrap();
        let request = machine.request_next().unwrap();
        let pending = machine
            .accept_response(&request, body(Some("opaque-1")))
            .unwrap();
        let receipt = pending.commit_receipt();
        assert_eq!(machine.state(), CursorState::AwaitingDurableCommit);
        assert_eq!(
            machine.request_next().unwrap_err(),
            CursorError::InvalidTransition {
                state: CursorState::AwaitingDurableCommit,
                action: "request_next"
            }
        );
        machine.commit_page(receipt).unwrap();
        assert_eq!(machine.checkpoint().next_cursor(), Some("opaque-1"));
    }

    #[test]
    fn terminal_is_only_reached_after_terminal_page_commit() {
        let mut machine = CursorStateMachine::new(query()).unwrap();
        let request = machine.request_next().unwrap();
        let receipt = machine
            .accept_response(&request, body(None))
            .unwrap()
            .commit_receipt();
        assert_ne!(machine.state(), CursorState::Terminal);
        machine.commit_page(receipt).unwrap();
        assert_eq!(machine.state(), CursorState::Terminal);
    }

    #[test]
    fn retry_reuses_the_exact_request_cursor() {
        let mut machine = CursorStateMachine::new(query()).unwrap();
        let request = machine.request_next().unwrap();
        machine.request_failed(&request).unwrap();
        assert_eq!(machine.request_next().unwrap(), request);
    }

    #[test]
    fn repeated_cursor_or_page_invalidates_query() {
        let mut machine = CursorStateMachine::new(query()).unwrap();
        let first = machine.request_next().unwrap();
        let receipt = machine
            .accept_response(&first, body(Some("opaque-1")))
            .unwrap()
            .commit_receipt();
        machine.commit_page(receipt).unwrap();

        let second = machine.request_next().unwrap();
        let error = machine
            .accept_response(&second, body(Some("opaque-1")))
            .unwrap_err();
        assert_eq!(error, CursorError::DuplicatePage);
        assert_eq!(machine.state(), CursorState::Invalid);
    }

    #[test]
    fn cursor_and_credential_debug_are_redacted() {
        let cursor = TeralionCursor::new("sensitive").unwrap();
        assert_eq!(format!("{cursor:?}"), "TeralionCursor([REDACTED])");
    }
}
