use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::market::{MarketType, Symbol};
use crate::orders::Side;

/// Direction of a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
    Flat,
}

/// A tracked position in a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub side: PositionSide,
    pub quantity: Decimal,
    pub entry_price: Decimal,
    pub current_price: Decimal,
    pub unrealized_pnl: Decimal,
    pub realized_pnl: Decimal,
    pub total_commission: Decimal,
    pub leverage: Decimal,
    pub liquidation_price: Option<Decimal>,
    pub opened_at: DateTime<Utc>,
    pub last_update: DateTime<Utc>,
    pub strategy_name: String,
}

impl Position {
    pub fn notional_value(&self) -> Decimal {
        self.quantity * self.current_price
    }

    pub fn is_flat(&self) -> bool {
        self.quantity == dec!(0) || self.side == PositionSide::Flat
    }

    /// Update mark price and recalculate unrealized PnL.
    pub fn update_mark_price(&mut self, price: Decimal, timestamp: DateTime<Utc>) {
        self.current_price = price;
        self.unrealized_pnl = match self.side {
            PositionSide::Long => (price - self.entry_price) * self.quantity,
            PositionSide::Short => (self.entry_price - price) * self.quantity,
            PositionSide::Flat => dec!(0),
        };
        self.last_update = timestamp;
    }

    /// Compute the return on the position.
    pub fn return_pct(&self) -> Option<Decimal> {
        if self.entry_price > dec!(0) {
            let raw = match self.side {
                PositionSide::Long => {
                    (self.current_price - self.entry_price) / self.entry_price
                }
                PositionSide::Short => {
                    (self.entry_price - self.current_price) / self.entry_price
                }
                PositionSide::Flat => return Some(dec!(0)),
            };
            Some(raw * dec!(100))
        } else {
            None
        }
    }
}

impl PositionSide {
    pub fn from_side(side: Side) -> Self {
        match side {
            Side::Buy => Self::Long,
            Side::Sell => Self::Short,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_position(side: PositionSide, qty: Decimal, entry: Decimal, current: Decimal) -> Position {
        Position {
            symbol: Symbol::new("BTCUSDT"),
            market_type: MarketType::Spot,
            side,
            quantity: qty,
            entry_price: entry,
            current_price: current,
            unrealized_pnl: dec!(0),
            realized_pnl: dec!(0),
            total_commission: dec!(0),
            leverage: dec!(1),
            liquidation_price: None,
            opened_at: Utc::now(),
            last_update: Utc::now(),
            strategy_name: "test".into(),
        }
    }

    #[test]
    fn long_pnl_calculation() {
        let mut pos = make_position(PositionSide::Long, dec!(1), dec!(50000), dec!(50000));
        pos.update_mark_price(dec!(51000), Utc::now());
        assert_eq!(pos.unrealized_pnl, dec!(1000));
    }

    #[test]
    fn short_pnl_calculation() {
        let mut pos = make_position(PositionSide::Short, dec!(1), dec!(50000), dec!(50000));
        pos.update_mark_price(dec!(49000), Utc::now());
        assert_eq!(pos.unrealized_pnl, dec!(1000));
    }

    #[test]
    fn return_pct() {
        let mut pos = make_position(PositionSide::Long, dec!(1), dec!(100), dec!(100));
        pos.update_mark_price(dec!(110), Utc::now());
        assert_eq!(pos.return_pct(), Some(dec!(10)));
    }
}
