use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Extension trait for safe decimal operations in trading contexts.
pub trait DecimalExt {
    fn safe_div(self, rhs: Decimal) -> Option<Decimal>;
    fn clamp_to(self, min: Decimal, max: Decimal) -> Decimal;
    fn is_positive_nonzero(&self) -> bool;
    fn abs_val(self) -> Decimal;
    fn bps(self) -> Decimal;
}

impl DecimalExt for Decimal {
    fn safe_div(self, rhs: Decimal) -> Option<Decimal> {
        if rhs == dec!(0) {
            None
        } else {
            Some(self / rhs)
        }
    }

    fn clamp_to(self, min: Decimal, max: Decimal) -> Decimal {
        if self < min {
            min
        } else if self > max {
            max
        } else {
            self
        }
    }

    fn is_positive_nonzero(&self) -> bool {
        *self > dec!(0)
    }

    fn abs_val(self) -> Decimal {
        if self < dec!(0) { -self } else { self }
    }

    /// Convert basis points to decimal (e.g., 10 bps = 0.001)
    fn bps(self) -> Decimal {
        self / dec!(10000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_div_by_zero_returns_none() {
        assert_eq!(dec!(100).safe_div(dec!(0)), None);
    }

    #[test]
    fn safe_div_normal() {
        assert_eq!(dec!(100).safe_div(dec!(4)), Some(dec!(25)));
    }

    #[test]
    fn bps_conversion() {
        assert_eq!(dec!(10).bps(), dec!(0.001));
        assert_eq!(dec!(100).bps(), dec!(0.01));
    }

    #[test]
    fn clamp_works() {
        assert_eq!(dec!(5).clamp_to(dec!(0), dec!(10)), dec!(5));
        assert_eq!(dec!(-1).clamp_to(dec!(0), dec!(10)), dec!(0));
        assert_eq!(dec!(15).clamp_to(dec!(0), dec!(10)), dec!(10));
    }
}
