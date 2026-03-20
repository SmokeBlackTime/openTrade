//! Persistent memory system for neural trading.
//!
//! Stores and retrieves:
//! - Trade outcomes with context (what the AI "saw" before each trade)
//! - Market regime patterns it has identified
//! - Strategy performance observations
//! - Lessons learned from losing trades
//! - Parameter adjustments and their outcomes
//!
//! This is the "remember" capability — the AI learns from every trade.

use chrono::{DateTime, Utc};
use ot_common::OtError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

/// A memory entry stored by the trading AI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub category: MemoryCategory,
    pub symbol: String,
    pub content: String,
    /// Structured data (features, signals, etc.)
    pub context: serde_json::Value,
    /// How important this memory is (0-1). Higher = recalled more often.
    pub importance: f64,
    /// How many times this memory has been recalled.
    pub recall_count: u32,
    /// The outcome if this was a trade memory (profit/loss ratio).
    pub outcome: Option<f64>,
    /// Tags for filtering.
    pub tags: Vec<String>,
}

/// Categories of memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// Trade entry/exit decision and outcome.
    TradeOutcome,
    /// Observed market regime pattern.
    RegimePattern,
    /// Strategy performance observation.
    StrategyPerformance,
    /// Lesson learned from a losing trade.
    LessonLearned,
    /// Parameter adjustment and its effect.
    ParameterTuning,
    /// General market insight.
    MarketInsight,
    /// Risk event that was handled.
    RiskEvent,
}

impl MemoryCategory {
    fn as_str(&self) -> &'static str {
        match self {
            Self::TradeOutcome => "trade_outcome",
            Self::RegimePattern => "regime_pattern",
            Self::StrategyPerformance => "strategy_performance",
            Self::LessonLearned => "lesson_learned",
            Self::ParameterTuning => "parameter_tuning",
            Self::MarketInsight => "market_insight",
            Self::RiskEvent => "risk_event",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "trade_outcome" => Self::TradeOutcome,
            "regime_pattern" => Self::RegimePattern,
            "strategy_performance" => Self::StrategyPerformance,
            "lesson_learned" => Self::LessonLearned,
            "parameter_tuning" => Self::ParameterTuning,
            "market_insight" => Self::MarketInsight,
            "risk_event" => Self::RiskEvent,
            _ => Self::MarketInsight,
        }
    }
}

/// The persistent memory store.
///
/// Thread-safe via internal Mutex on the SQLite connection.
pub struct NeuralMemory {
    conn: std::sync::Mutex<Connection>,
    max_entries: usize,
}

// Safety: We wrap Connection in a Mutex, ensuring exclusive access.
unsafe impl Send for NeuralMemory {}
unsafe impl Sync for NeuralMemory {}

impl NeuralMemory {
    /// Create a new memory store at the given path.
    pub fn new(path: &Path, max_entries: usize) -> Result<Self, OtError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| OtError::Storage(e.to_string()))?;
        }

        let conn = Connection::open(path)
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let memory = Self {
            conn: std::sync::Mutex::new(conn),
            max_entries,
        };
        memory.init_schema()?;
        Ok(memory)
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory(max_entries: usize) -> Result<Self, OtError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| OtError::Storage(e.to_string()))?;
        let memory = Self {
            conn: std::sync::Mutex::new(conn),
            max_entries,
        };
        memory.init_schema()?;
        Ok(memory)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("NeuralMemory mutex poisoned")
    }

    fn init_schema(&self) -> Result<(), OtError> {
        self.conn()
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                category TEXT NOT NULL,
                symbol TEXT NOT NULL,
                content TEXT NOT NULL,
                context TEXT NOT NULL,
                importance REAL NOT NULL DEFAULT 0.5,
                recall_count INTEGER NOT NULL DEFAULT 0,
                outcome REAL,
                tags TEXT NOT NULL DEFAULT '[]'
            );

            CREATE INDEX IF NOT EXISTS idx_memories_category
                ON memories(category, timestamp);
            CREATE INDEX IF NOT EXISTS idx_memories_symbol
                ON memories(symbol, timestamp);
            CREATE INDEX IF NOT EXISTS idx_memories_importance
                ON memories(importance DESC);

            CREATE TABLE IF NOT EXISTS learning_stats (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            ",
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Store a new memory.
    pub fn remember(&self, entry: &MemoryEntry) -> Result<(), OtError> {
        let tags_json = serde_json::to_string(&entry.tags).unwrap_or_else(|_| "[]".into());
        let context_json = serde_json::to_string(&entry.context).unwrap_or_else(|_| "{}".into());

        self.conn()
            .execute(
                "INSERT OR REPLACE INTO memories
                 (id, timestamp, category, symbol, content, context, importance, recall_count, outcome, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    entry.id,
                    entry.timestamp.to_rfc3339(),
                    entry.category.as_str(),
                    entry.symbol,
                    entry.content,
                    context_json,
                    entry.importance,
                    entry.recall_count,
                    entry.outcome,
                    tags_json,
                ],
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;

        // Enforce max entries
        self.prune()?;
        Ok(())
    }

    /// Recall memories by category, optionally filtered by symbol.
    pub fn recall(
        &self,
        category: MemoryCategory,
        symbol: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, OtError> {
        let conn = self.conn();

        let (query, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match symbol {
            Some(sym) => (
                "SELECT id, timestamp, category, symbol, content, context, importance, recall_count, outcome, tags
                 FROM memories
                 WHERE category = ?1 AND symbol = ?2
                 ORDER BY importance DESC, timestamp DESC
                 LIMIT ?3".into(),
                vec![
                    Box::new(category.as_str().to_string()),
                    Box::new(sym.to_string()),
                    Box::new(limit as i64),
                ],
            ),
            None => (
                "SELECT id, timestamp, category, symbol, content, context, importance, recall_count, outcome, tags
                 FROM memories
                 WHERE category = ?1
                 ORDER BY importance DESC, timestamp DESC
                 LIMIT ?2".into(),
                vec![
                    Box::new(category.as_str().to_string()),
                    Box::new(limit as i64),
                ],
            ),
        };

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let entries = stmt
            .query_map(params_refs.as_slice(), Self::row_to_entry)
            .map_err(|e| OtError::Storage(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| OtError::Storage(e.to_string()))?;

        // Increment recall count for returned memories
        for entry in &entries {
            let _ = conn.execute(
                "UPDATE memories SET recall_count = recall_count + 1 WHERE id = ?1",
                params![entry.id],
            );
        }

        Ok(entries)
    }

    /// Map a SQLite row to a MemoryEntry.
    fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
        let timestamp_str: String = row.get(1)?;
        let category_str: String = row.get(2)?;
        let context_str: String = row.get(5)?;
        let tags_str: String = row.get(9)?;

        Ok(MemoryEntry {
            id: row.get(0)?,
            timestamp: DateTime::parse_from_rfc3339(&timestamp_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            category: MemoryCategory::from_str(&category_str),
            symbol: row.get(3)?,
            content: row.get(4)?,
            context: serde_json::from_str(&context_str).unwrap_or(serde_json::json!({})),
            importance: row.get(6)?,
            recall_count: row.get::<_, u32>(7)?,
            outcome: row.get(8)?,
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        })
    }

    /// Recall the most recent memories regardless of category.
    pub fn recall_recent(&self, limit: usize) -> Result<Vec<MemoryEntry>, OtError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, category, symbol, content, context, importance, recall_count, outcome, tags
                 FROM memories
                 ORDER BY timestamp DESC
                 LIMIT ?1",
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let entries = stmt
            .query_map(params![limit as i64], Self::row_to_entry)
            .map_err(|e| OtError::Storage(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| OtError::Storage(e.to_string()))?;

        Ok(entries)
    }

    /// Search memories by keyword in content.
    pub fn search(&self, keyword: &str, limit: usize) -> Result<Vec<MemoryEntry>, OtError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, category, symbol, content, context, importance, recall_count, outcome, tags
                 FROM memories
                 WHERE content LIKE '%' || ?1 || '%'
                 ORDER BY importance DESC, timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let entries = stmt
            .query_map(params![keyword, limit as i64], Self::row_to_entry)
            .map_err(|e| OtError::Storage(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| OtError::Storage(e.to_string()))?;

        Ok(entries)
    }

    /// Get/set learning statistics.
    pub fn get_stat(&self, key: &str) -> Result<Option<String>, OtError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT value FROM learning_stats WHERE key = ?1")
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![key], |row| row.get(0))
            .ok();
        Ok(result)
    }

    pub fn set_stat(&self, key: &str, value: &str) -> Result<(), OtError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO learning_stats (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, Utc::now().to_rfc3339()],
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Count memories by category.
    pub fn count_by_category(&self, category: MemoryCategory) -> Result<usize, OtError> {
        let conn = self.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE category = ?1",
                params![category.as_str()],
                |row| row.get(0),
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;

        Ok(count as usize)
    }

    /// Total memory count.
    pub fn total_count(&self) -> Result<usize, OtError> {
        let conn = self.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .map_err(|e| OtError::Storage(e.to_string()))?;

        Ok(count as usize)
    }

    /// Remove least important/oldest memories to stay under max_entries.
    fn prune(&self) -> Result<(), OtError> {
        let count = self.total_count()?;
        if count <= self.max_entries {
            return Ok(());
        }

        let excess = count - self.max_entries;
        info!(
            excess = excess,
            max = self.max_entries,
            "Pruning oldest/least important memories"
        );

        self.conn()
            .execute(
                "DELETE FROM memories WHERE id IN (
                    SELECT id FROM memories
                    ORDER BY importance ASC, recall_count ASC, timestamp ASC
                    LIMIT ?1
                )",
                params![excess as i64],
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_entry(category: MemoryCategory, symbol: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: uuid::Uuid::new_v4().simple().to_string(),
            timestamp: Utc::now(),
            category,
            symbol: symbol.into(),
            content: content.into(),
            context: json!({"test": true}),
            importance: 0.7,
            recall_count: 0,
            outcome: Some(0.02),
            tags: vec!["test".into()],
        }
    }

    #[test]
    fn store_and_recall() {
        let mem = NeuralMemory::in_memory(1000).unwrap();
        let entry = make_entry(MemoryCategory::TradeOutcome, "BTCUSDT", "Long trade won 2%");
        mem.remember(&entry).unwrap();

        let recalled = mem
            .recall(MemoryCategory::TradeOutcome, Some("BTCUSDT"), 10)
            .unwrap();
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].content, "Long trade won 2%");
    }

    #[test]
    fn search_memories() {
        let mem = NeuralMemory::in_memory(1000).unwrap();
        mem.remember(&make_entry(
            MemoryCategory::LessonLearned,
            "BTCUSDT",
            "Avoid trading during low volume weekends",
        ))
        .unwrap();
        mem.remember(&make_entry(
            MemoryCategory::LessonLearned,
            "ETHUSDT",
            "Mean reversion works well in ranging regime",
        ))
        .unwrap();

        let results = mem.search("volume", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("volume"));
    }

    #[test]
    fn prune_when_over_limit() {
        let mem = NeuralMemory::in_memory(5).unwrap();
        for i in 0..10 {
            mem.remember(&make_entry(
                MemoryCategory::MarketInsight,
                "BTCUSDT",
                &format!("Insight {}", i),
            ))
            .unwrap();
        }

        let count = mem.total_count().unwrap();
        assert!(count <= 5);
    }

    #[test]
    fn learning_stats() {
        let mem = NeuralMemory::in_memory(100).unwrap();
        mem.set_stat("total_trades_analyzed", "42").unwrap();
        let val = mem.get_stat("total_trades_analyzed").unwrap();
        assert_eq!(val, Some("42".to_string()));
    }
}
