use chrono::{DateTime, TimeZone, Utc};

/// Convert millisecond timestamp to DateTime<Utc>.
pub fn ms_to_datetime(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
}

/// Convert DateTime<Utc> to millisecond timestamp.
pub fn datetime_to_ms(dt: &DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}

/// Check if a timestamp is stale (older than max_age_secs).
pub fn is_stale(timestamp: &DateTime<Utc>, max_age_secs: i64) -> bool {
    let age = Utc::now().signed_duration_since(*timestamp);
    age.num_seconds() > max_age_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ms_conversion() {
        let now = Utc::now();
        let ms = datetime_to_ms(&now);
        let back = ms_to_datetime(ms);
        // Within 1ms due to truncation
        assert!((now - back).num_milliseconds().abs() <= 1);
    }

    #[test]
    fn staleness_check() {
        let old = Utc::now() - chrono::Duration::seconds(120);
        assert!(is_stale(&old, 60));
        assert!(!is_stale(&old, 300));
    }
}
