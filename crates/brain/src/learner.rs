//! Trade learner: analyzes completed trades and extracts lessons.
//!
//! After each trade closes, the learner:
//! 1. Compares the outcome to the AI's pre-trade analysis
//! 2. Identifies what the AI got right and wrong
//! 3. Stores lessons in memory for future reference
//! 4. Tracks strategy-level performance patterns
//! 5. Suggests parameter adjustments

use ot_neural::memory::{MemoryCategory, MemoryEntry, NeuralMemory};
use ot_neural::proxy::NeuralProxy;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// A completed trade for review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedTrade {
    pub symbol: String,
    pub direction: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl_pct: f64,
    pub strategy: String,
    pub regime_at_entry: String,
    pub regime_at_exit: String,
    pub hold_bars: u32,
    pub confidence_at_entry: f64,
    pub features_at_entry: serde_json::Value,
    pub features_at_exit: serde_json::Value,
}

/// Lesson extracted from a trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeLesson {
    pub summary: String,
    pub was_profitable: bool,
    pub regime_changed: bool,
    pub confidence_justified: bool,
    pub suggested_adjustment: Option<String>,
    pub importance: f64,
}

/// The trade learning engine.
pub struct TradeLearner;

impl TradeLearner {
    /// Analyze a completed trade and extract lessons.
    pub async fn review_trade(
        proxy: &NeuralProxy,
        trade: &CompletedTrade,
        memory: &NeuralMemory,
    ) -> Option<TradeLesson> {
        // First, do rule-based analysis (no LLM needed for basic patterns)
        let mut lesson = Self::rule_based_review(trade);

        // If the trade was significant (large P&L or unexpected), use LLM for deeper analysis
        if trade.pnl_pct.abs() > 0.02 || !lesson.confidence_justified {
            if let Some(llm_lesson) = Self::llm_review(proxy, trade).await {
                // Merge LLM insights with rule-based analysis
                lesson.summary = format!("{} | AI: {}", lesson.summary, llm_lesson.summary);
                if let Some(adj) = llm_lesson.suggested_adjustment {
                    lesson.suggested_adjustment = Some(adj);
                }
                lesson.importance = lesson.importance.max(llm_lesson.importance);
            }
        }

        // Store the lesson in memory
        let category = if trade.pnl_pct < -0.01 {
            MemoryCategory::LessonLearned
        } else {
            MemoryCategory::TradeOutcome
        };

        let memory_entry = MemoryEntry {
            id: uuid::Uuid::new_v4().simple().to_string(),
            timestamp: Utc::now(),
            category,
            symbol: trade.symbol.clone(),
            content: lesson.summary.clone(),
            context: serde_json::to_value(trade).unwrap_or_default(),
            importance: lesson.importance,
            recall_count: 0,
            outcome: Some(trade.pnl_pct),
            tags: vec![
                trade.direction.clone(),
                trade.regime_at_entry.clone(),
                trade.strategy.clone(),
                if trade.pnl_pct > 0.0 {
                    "winner".to_string()
                } else {
                    "loser".to_string()
                },
            ],
        };

        if let Err(e) = memory.remember(&memory_entry) {
            warn!(error = %e, "Failed to store trade lesson");
        }

        info!(
            symbol = %trade.symbol,
            pnl = format!("{:.2}%", trade.pnl_pct * 100.0),
            lesson = %lesson.summary,
            "Trade reviewed"
        );

        Some(lesson)
    }

    /// Rule-based trade review (fast, no LLM needed).
    fn rule_based_review(trade: &CompletedTrade) -> TradeLesson {
        let regime_changed = trade.regime_at_entry != trade.regime_at_exit;
        let was_profitable = trade.pnl_pct > 0.0;

        // Was the confidence justified by the outcome?
        let confidence_justified = if was_profitable {
            trade.confidence_at_entry > 0.5 // High confidence + win = justified
        } else {
            trade.confidence_at_entry < 0.6 // Low confidence + loss = at least calibrated
        };

        let mut observations = Vec::new();

        // Regime change analysis
        if regime_changed && !was_profitable {
            observations.push(format!(
                "Lost money when regime changed from {} to {}",
                trade.regime_at_entry, trade.regime_at_exit
            ));
        }

        // Holding time analysis
        if trade.hold_bars > 30 && !was_profitable {
            observations.push("Held losing position too long".into());
        }
        if trade.hold_bars < 3 && was_profitable && trade.pnl_pct < 0.005 {
            observations.push("Exited winner too early".into());
        }

        // Confidence calibration
        if trade.confidence_at_entry > 0.8 && trade.pnl_pct < -0.02 {
            observations.push(format!(
                "High confidence ({:.0}%) trade lost {:.1}% — overconfident",
                trade.confidence_at_entry * 100.0,
                trade.pnl_pct.abs() * 100.0
            ));
        }

        let summary = if observations.is_empty() {
            if was_profitable {
                format!(
                    "{} {} won {:.2}% in {} regime ({})",
                    trade.direction, trade.symbol, trade.pnl_pct * 100.0,
                    trade.regime_at_entry, trade.strategy
                )
            } else {
                format!(
                    "{} {} lost {:.2}% in {} regime ({})",
                    trade.direction, trade.symbol, trade.pnl_pct.abs() * 100.0,
                    trade.regime_at_entry, trade.strategy
                )
            }
        } else {
            observations.join(". ")
        };

        let importance = if trade.pnl_pct.abs() > 0.05 {
            0.9
        } else if !confidence_justified {
            0.8
        } else if regime_changed {
            0.7
        } else {
            0.5
        };

        TradeLesson {
            summary,
            was_profitable,
            regime_changed,
            confidence_justified,
            suggested_adjustment: None,
            importance,
        }
    }

    /// LLM-powered trade review (deeper analysis for significant trades).
    async fn llm_review(
        proxy: &NeuralProxy,
        trade: &CompletedTrade,
    ) -> Option<TradeLesson> {
        let prompt = format!(
            r#"Review this completed trade and extract the key lesson:

Trade: {} {} on {}
Entry: {:.2}, Exit: {:.2}, P&L: {:.2}%
Strategy: {}, Hold: {} bars
Regime at entry: {}, Regime at exit: {}
Confidence at entry: {:.0}%

Entry features: {}
Exit features: {}

Respond with JSON:
{{"summary": "one-sentence lesson", "suggested_adjustment": "null or parameter change suggestion", "importance": 0.0-1.0}}"#,
            trade.direction,
            trade.symbol,
            trade.symbol,
            trade.entry_price,
            trade.exit_price,
            trade.pnl_pct * 100.0,
            trade.strategy,
            trade.hold_bars,
            trade.regime_at_entry,
            trade.regime_at_exit,
            trade.confidence_at_entry * 100.0,
            serde_json::to_string(&trade.features_at_entry).unwrap_or_default(),
            serde_json::to_string(&trade.features_at_exit).unwrap_or_default(),
        );

        match proxy
            .ask(
                "You are a trade reviewer. Extract actionable lessons from completed trades. Output JSON only.",
                &prompt,
                true,
            )
            .await
        {
            Ok(response) => {
                let v: serde_json::Value = serde_json::from_str(&response).ok()?;
                Some(TradeLesson {
                    summary: v["summary"].as_str().unwrap_or("").to_string(),
                    was_profitable: trade.pnl_pct > 0.0,
                    regime_changed: trade.regime_at_entry != trade.regime_at_exit,
                    confidence_justified: true, // LLM doesn't judge this
                    suggested_adjustment: v["suggested_adjustment"]
                        .as_str()
                        .map(String::from),
                    importance: v["importance"].as_f64().unwrap_or(0.5),
                })
            }
            Err(e) => {
                warn!(error = %e, "LLM trade review failed");
                None
            }
        }
    }

    /// Analyze strategy performance across multiple trades.
    pub fn analyze_strategy_performance(
        memory: &NeuralMemory,
        strategy: &str,
        _symbol: Option<&str>,
    ) -> StrategyPerformanceReport {
        let trades = memory
            .search(strategy, 100)
            .unwrap_or_default();

        let mut wins = 0u32;
        let mut losses = 0u32;
        let mut total_pnl = 0.0;
        let mut max_win = 0.0f64;
        let mut max_loss = 0.0f64;

        for trade in &trades {
            if let Some(outcome) = trade.outcome {
                if outcome > 0.0 {
                    wins += 1;
                    max_win = max_win.max(outcome);
                } else {
                    losses += 1;
                    max_loss = max_loss.min(outcome);
                }
                total_pnl += outcome;
            }
        }

        let total = wins + losses;
        let win_rate = if total > 0 {
            wins as f64 / total as f64
        } else {
            0.0
        };

        StrategyPerformanceReport {
            strategy: strategy.to_string(),
            total_trades: total,
            win_rate,
            total_pnl,
            max_win,
            max_loss,
            avg_pnl: if total > 0 {
                total_pnl / total as f64
            } else {
                0.0
            },
        }
    }
}

/// Performance report for a strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyPerformanceReport {
    pub strategy: String,
    pub total_trades: u32,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub max_win: f64,
    pub max_loss: f64,
    pub avg_pnl: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_based_review_profitable() {
        let trade = CompletedTrade {
            symbol: "BTCUSDT".into(),
            direction: "long".into(),
            entry_price: 50000.0,
            exit_price: 51000.0,
            pnl_pct: 0.02,
            strategy: "trend_following".into(),
            regime_at_entry: "TrendingUp".into(),
            regime_at_exit: "TrendingUp".into(),
            hold_bars: 10,
            confidence_at_entry: 0.75,
            features_at_entry: serde_json::json!({}),
            features_at_exit: serde_json::json!({}),
        };

        let lesson = TradeLearner::rule_based_review(&trade);
        assert!(lesson.was_profitable);
        assert!(!lesson.regime_changed);
        assert!(lesson.confidence_justified);
    }

    #[test]
    fn rule_based_review_overconfident_loss() {
        let trade = CompletedTrade {
            symbol: "BTCUSDT".into(),
            direction: "long".into(),
            entry_price: 50000.0,
            exit_price: 48000.0,
            pnl_pct: -0.04,
            strategy: "trend_following".into(),
            regime_at_entry: "TrendingUp".into(),
            regime_at_exit: "TrendingDown".into(),
            hold_bars: 25,
            confidence_at_entry: 0.9,
            features_at_entry: serde_json::json!({}),
            features_at_exit: serde_json::json!({}),
        };

        let lesson = TradeLearner::rule_based_review(&trade);
        assert!(!lesson.was_profitable);
        assert!(lesson.regime_changed);
        assert!(!lesson.confidence_justified);
        assert!(lesson.importance > 0.7);
    }
}
