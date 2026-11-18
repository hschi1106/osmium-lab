//! What a run reads for one trading day.

use std::collections::{BTreeMap, HashMap};

use osmium_book::{BookId, BookState, Method, MethodEvidence};
use osmium_ledger::PreviousDay;
use osmium_matching::BookSpec;
use osmium_md::{
    CreditQuotas, DailyQuotes, DayInstruments, DayManifest, FeedDay, ManifestEntry, MdError,
    MdRoot, ReferenceFile,
};
use osmium_types::{Board, Date, Feed, Instrument, RejectReason, Symbol, Venue};

use crate::{MissingDays, RunConfig, RunError};

/// One book of the day: how it matches, and whether orders can be simulated on it.
pub(crate) struct BookSetup {
    pub(crate) id: BookId,
    pub(crate) method: Method,
    pub(crate) spec: Result<BookSpec, RejectReason>,
}

/// The market data of one trading day.
pub(crate) struct DayData {
    /// The instruments of both markets.
    pub(crate) instruments: Vec<Instrument>,
    pub(crate) feeds: Vec<FeedDay>,
    pub(crate) books: Vec<BookSetup>,
    /// The manifest rows of every file read, in reading order.
    pub(crate) files: Vec<ManifestEntry>,
    /// The configured feeds the day does not have.
    pub(crate) skipped: Vec<Feed>,
    /// Regular-board instruments whose lot size was assumed.
    pub(crate) assumed_lots: Vec<Symbol>,
}

fn missing(error: &MdError) -> bool {
    matches!(
        error,
        MdError::MissingManifest { .. } | MdError::MissingManifestEntry { .. }
    )
}

pub(crate) fn load_day(root: &MdRoot, config: &RunConfig, date: Date) -> Result<DayData, RunError> {
    let skip = config.missing_days == MissingDays::Skip;
    let mut data = DayData {
        instruments: Vec::new(),
        feeds: Vec::new(),
        books: Vec::new(),
        files: Vec::new(),
        skipped: Vec::new(),
        assumed_lots: Vec::new(),
    };
    let manifest = match DayManifest::load(root, date) {
        Ok(manifest) => manifest,
        Err(error) if skip && missing(&error) => {
            data.skipped.clone_from(&config.feeds);
            return Ok(data);
        }
        Err(error) => return Err(error.into()),
    };

    let mut by_venue: Vec<(Venue, DayInstruments)> = Vec::new();
    for venue in Venue::ALL {
        match DayInstruments::load(root, &manifest, venue) {
            Ok(instruments) => {
                data.files.push(instruments.manifest_entry().clone());
                data.instruments.extend_from_slice(instruments.records());
                by_venue.push((venue, instruments));
            }
            // A market without instruments has no feed either; a configured feed of
            // that market is reported below.
            Err(error) if missing(&error) => {}
            Err(error) => return Err(error.into()),
        }
    }

    for &feed in &config.feeds {
        let instruments = by_venue
            .iter()
            .find(|(venue, _)| *venue == feed.venue())
            .map(|(_, instruments)| instruments);
        let loaded = match instruments {
            Some(instruments) => FeedDay::load(
                root,
                &manifest,
                feed,
                instruments,
                config.symbols.as_deref(),
                config.missing_lot_size,
            ),
            None => Err(MdError::MissingManifestEntry {
                date,
                table: "instruments",
                part: feed.venue().as_str().to_owned(),
            }),
        };
        let day = match loaded {
            Ok(day) => day,
            Err(error) if skip && missing(&error) => {
                data.skipped.push(feed);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let instruments = instruments.expect("the feed was loaded with them");
        data.files.push(day.identity().books.clone());
        data.files.push(day.identity().trades.clone());
        data.assumed_lots.extend(day.assumed_lot_sizes());
        books_of(&day, instruments, config, &mut data.books);
        data.feeds.push(day);
    }
    Ok(data)
}

/// Adds the books of one feed day.
fn books_of(
    day: &FeedDay,
    instruments: &DayInstruments,
    config: &RunConfig,
    books: &mut Vec<BookSetup>,
) {
    let board = day.feed().board();
    let known = |symbol: Symbol| {
        instruments
            .get(symbol)
            .and_then(|instrument| Method::known(board, instrument.match_interval_s))
    };
    // Books whose method marketdata does not state show it in the day's messages
    // (`docs/reference/matching.md` section 4.1).
    let mut evidence: HashMap<Symbol, MethodEvidence> = day
        .symbols()
        .filter(|&symbol| known(symbol).is_none())
        .map(|symbol| (symbol, MethodEvidence::new()))
        .collect();
    if !evidence.is_empty() {
        let close_collection = BookState::close_collection(day.date());
        for message in day.messages() {
            if let Some(evidence) = evidence.get_mut(&message.symbol) {
                evidence.observe(&message, close_collection);
            }
        }
    }
    let assumed = match config.missing_lot_size {
        osmium_md::MissingLotSize::Assume(lot) => Some(lot),
        osmium_md::MissingLotSize::Reject => None,
    };
    for symbol in day.symbols() {
        let method = known(symbol).unwrap_or_else(|| evidence[&symbol].method());
        let spec = match instruments.get(symbol) {
            None => Err(RejectReason::UnknownInstrument),
            Some(instrument) => instrument
                .shares_per_lot
                .or(assumed)
                .and_then(|lot| BookSpec::new(instrument, method, lot))
                .ok_or(RejectReason::UnsupportedInstrument),
        };
        books.push(BookSetup {
            id: BookId { symbol, board },
            method,
            spec,
        });
    }
    debug_assert!(board == Board::Regular || board == Board::OddLot);
}

/// The official prices of one day, and which reference files they came from.
pub(crate) struct Reference {
    files: BTreeMap<String, ReferenceFile>,
}

impl Reference {
    pub(crate) const fn new() -> Self {
        Self {
            files: BTreeMap::new(),
        }
    }

    pub(crate) fn note(&mut self, file: &ReferenceFile) {
        self.files.insert(file.path.clone(), file.clone());
    }

    pub(crate) fn into_files(self) -> Vec<ReferenceFile> {
        self.files.into_values().collect()
    }

    /// The closing quotes of `date`; a day without any is an error, because the
    /// positions could not be valued.
    pub(crate) fn quotes(&mut self, root: &MdRoot, date: Date) -> Result<DailyQuotes, RunError> {
        let quotes = DailyQuotes::load(root, date)?;
        self.note(quotes.file());
        if quotes.rows().is_empty() {
            return Err(RunError::Unsupported {
                date,
                reason: format!("marketdata {} has no row for this day", quotes.file().path),
            });
        }
        Ok(quotes)
    }

    /// What the ledger needs from the trading day before, and whether that day has
    /// credit quotas at all.
    pub(crate) fn previous(
        &mut self,
        root: &MdRoot,
        quotes: &DailyQuotes,
    ) -> Result<(HashMap<Symbol, PreviousDay>, bool), RunError> {
        let mut previous: HashMap<Symbol, PreviousDay> = quotes
            .rows()
            .iter()
            .map(|quote| (quote.symbol, PreviousDay::of(quote)))
            .collect();
        let quotas = CreditQuotas::load(root, quotes.date())?;
        self.note(quotas.file());
        for quota in quotas.rows() {
            previous.entry(quota.symbol).or_default().set_quota(quota);
        }
        Ok((previous, !quotas.rows().is_empty()))
    }
}
