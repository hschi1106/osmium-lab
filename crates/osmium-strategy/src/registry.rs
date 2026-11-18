use crate::{ParamError, Params, Strategy};

/// How a strategy is found by the name in a run's configuration (`strategy.name`) and
/// built from its parameters (`[strategy.params]`).
#[derive(Debug, Clone, Copy)]
pub struct StrategyEntry {
    pub name: &'static str,
    /// Recorded in the results. Change it whenever the strategy's decisions change.
    pub version: &'static str,
    /// One line for listings.
    pub about: &'static str,
    /// Builds the strategy. It reads every parameter it knows from `Params`; what is
    /// left afterwards is reported to the user as unknown.
    pub build: fn(&mut Params) -> Result<Box<dyn Strategy>, ParamError>,
}

/// The strategies a program can run.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    /// Sorted by name.
    entries: Vec<StrategyEntry>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a strategy.
    ///
    /// # Panics
    ///
    /// Panics when the name is empty or taken: two strategies under one name would
    /// make results ambiguous.
    pub fn add(&mut self, entry: StrategyEntry) {
        assert!(!entry.name.is_empty(), "a strategy needs a name");
        match self
            .entries
            .binary_search_by_key(&entry.name, |other| other.name)
        {
            Ok(_) => panic!("strategy `{}` is registered twice", entry.name),
            Err(index) => self.entries.insert(index, entry),
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&StrategyEntry> {
        self.entries
            .binary_search_by_key(&name, |entry| entry.name)
            .ok()
            .map(|index| &self.entries[index])
    }

    /// Every strategy, by name.
    #[must_use]
    pub fn entries(&self) -> &[StrategyEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Idle;

    impl Strategy for Idle {}

    fn entry(name: &'static str) -> StrategyEntry {
        StrategyEntry {
            name,
            version: "1",
            about: "does nothing",
            build: |_| Ok(Box::new(Idle)),
        }
    }

    #[test]
    fn strategies_are_found_by_name() {
        let mut registry = Registry::new();
        registry.add(entry("b"));
        registry.add(entry("a"));
        let names: Vec<&str> = registry.entries().iter().map(|entry| entry.name).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(registry.get("b").unwrap().name, "b");
        assert!(registry.get("c").is_none());
    }

    #[test]
    #[should_panic(expected = "registered twice")]
    fn a_name_is_registered_once() {
        let mut registry = Registry::new();
        registry.add(entry("a"));
        registry.add(entry("a"));
    }
}
