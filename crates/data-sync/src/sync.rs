use std::{error::Error, fmt, io};

use crate::{
    CursorError, CursorState, CursorStateMachine, StagingError, StagingRevision,
    TeralionCredential, TeralionQuery, TeralionRequest, TeralionTransport, TransportError,
};

#[derive(Debug)]
pub struct TeralionSync<T> {
    transport: T,
}

impl<T: TeralionTransport> TeralionSync<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn fetch_single(
        &mut self,
        query: TeralionQuery,
        credential: &TeralionCredential,
    ) -> Result<Vec<u8>, SyncError> {
        if query.is_paged() {
            return Err(SyncError::ExpectedSingleResponse);
        }
        self.transport
            .execute(&TeralionRequest::first(query), credential)
            .map_err(SyncError::Transport)
    }

    pub fn sync_pages(
        &mut self,
        query: TeralionQuery,
        credential: &TeralionCredential,
        staging: &mut StagingRevision,
    ) -> Result<PagedSyncReport, SyncError> {
        let checkpoint_path = staging.path().join("checkpoint.json");
        let mut machine = if checkpoint_path.exists() {
            CursorStateMachine::resume(query, crate::CursorCheckpoint::load(&checkpoint_path)?)?
        } else {
            CursorStateMachine::new(query)?
        };
        while machine.state() != CursorState::Terminal {
            let request = machine.request_next()?;
            let body = match self.transport.execute(&request, credential) {
                Ok(body) => body,
                Err(error) => {
                    machine.request_failed(&request)?;
                    return Err(SyncError::Transport(error));
                }
            };
            let staged = staging.stage_page(machine.accept_response(&request, body)?)?;
            machine.commit_page(staged.commit_receipt())?;
            machine.checkpoint().save(&checkpoint_path)?;
        }
        Ok(PagedSyncReport {
            page_count: machine.checkpoint().committed_pages(),
            terminal: machine.checkpoint().terminal(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PagedSyncReport {
    pub page_count: u32,
    pub terminal: bool,
}

#[derive(Debug)]
pub enum SyncError {
    Transport(TransportError),
    Cursor(CursorError),
    Staging(StagingError),
    Io(io::Error),
    ExpectedSingleResponse,
}

impl fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for SyncError {}

impl From<CursorError> for SyncError {
    fn from(error: CursorError) -> Self {
        Self::Cursor(error)
    }
}

impl From<StagingError> for SyncError {
    fn from(error: StagingError) -> Self {
        Self::Staging(error)
    }
}

impl From<io::Error> for SyncError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use market_types::{InstrumentId, MarketId, Symbol};

    use super::*;
    use crate::{ArchiveKind, ArchiveTimestamp};

    #[derive(Default)]
    struct FakeTransport;

    impl TeralionTransport for FakeTransport {
        fn execute(
            &mut self,
            request: &TeralionRequest,
            _: &TeralionCredential,
        ) -> Result<Vec<u8>, TransportError> {
            Ok(match request.cursor() {
                None => br#"{"items":[],"next_cursor":"next"}"#.to_vec(),
                Some("next") => br#"{"items":[],"next_cursor":null}"#.to_vec(),
                Some(_) => unreachable!(),
            })
        }
    }

    #[test]
    fn sync_durably_checkpoints_every_committed_page() {
        let root = tempfile::tempdir().unwrap();
        let query = TeralionQuery::ticks(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote],
            5_000,
        )
        .unwrap();
        let mut staging = StagingRevision::create(root.path(), "sync").unwrap();
        let report = TeralionSync::new(FakeTransport)
            .sync_pages(
                query,
                &TeralionCredential::new("test-only").unwrap(),
                &mut staging,
            )
            .unwrap();
        assert_eq!(report.page_count, 2);
        assert!(report.terminal);
        assert!(staging.path().join("checkpoint.json").is_file());
        let resumed = StagingRevision::resume(root.path(), "sync").unwrap();
        assert_eq!(resumed.path(), staging.path());
    }
}
