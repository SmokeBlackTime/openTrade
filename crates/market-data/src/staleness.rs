use chrono::{DateTime, Utc};
use std::collections::HashMap;
use ot_types::market::Symbol;

/// Tracks last-received timestamps per symbol to detect stale data.
pub struct StalenessTracker {
    last_seen: HashMap<Symbol, DateTime<Utc>>,
    max_age_secs: i64,
}

impl StalenessTracker {
    pub fn new(max_age_secs: i64) -> Self {
        Self {
            last_seen: HashMap::new(),
            max_age_secs,
        }
    }

    pub fn update(&mut self, symbol: &Symbol, timestamp: DateTime<Utc>) {
        self.last_seen.insert(symbol.clone(), timestamp);
    }

    pub fn is_stale(&self, symbol: &Symbol) -> bool {
        match self.last_seen.get(symbol) {
            Some(ts) => ot_common::time_utils::is_stale(ts, self.max_age_secs),
            None => true, // Never seen = stale
        }
    }

    pub fn stale_symbols(&self) -> Vec<&Symbol> {
        self.last_seen
            .iter()
            .filter(|(_, ts)| ot_common::time_utils::is_stale(ts, self.max_age_secs))
            .map(|(sym, _)| sym)
            .collect()
    }

    pub fn all_fresh(&self) -> bool {
        self.last_seen
            .values()
            .all(|ts| !ot_common::time_utils::is_stale(ts, self.max_age_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unseen_symbol_is_stale() {
        let tracker = StalenessTracker::new(30);
        assert!(tracker.is_stale(&Symbol::new("BTCUSDT")));
    }

    #[test]
    fn recent_update_is_fresh() {
        let mut tracker = StalenessTracker::new(30);
        tracker.update(&Symbol::new("BTCUSDT"), Utc::now());
        assert!(!tracker.is_stale(&Symbol::new("BTCUSDT")));
    }

    #[test]
    fn old_update_is_stale() {
        let mut tracker = StalenessTracker::new(30);
        let old = Utc::now() - chrono::Duration::seconds(60);
        tracker.update(&Symbol::new("BTCUSDT"), old);
        assert!(tracker.is_stale(&Symbol::new("BTCUSDT")));
    }
}
