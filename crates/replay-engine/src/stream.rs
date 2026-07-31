use std::error::Error;

use market_types::DomainEvent;

use crate::ReplayStreamBinding;

pub trait EventStream {
    type Error: Error;

    fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error>;
}

pub trait ReplayStreamFactory {
    type Stream: EventStream;
    type Error: Error;

    fn open(&mut self, binding: &ReplayStreamBinding) -> Result<Self::Stream, Self::Error>;
}
