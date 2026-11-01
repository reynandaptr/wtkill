//! Human-readable sizes and ages.

const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// Binary units, one decimal above bytes: `1536` → `"1.5 KB"`.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Compact age: `<1m`, `5m`, `3h`, `12d`, `3mo` (from 60 days), `2y` (from 365 days).
pub fn format_age(secs: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    match secs {
        s if s < MIN => "<1m".to_string(),
        s if s < HOUR => format!("{}m", s / MIN),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < 60 * DAY => format!("{}d", s / DAY),
        s if s < 365 * DAY => format!("{}mo", s / (30 * DAY)),
        s => format!("{}y", s / (365 * DAY)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(38 * 1024 * 1024 * 1024), "38.0 GB");
        assert_eq!(format_size(3 * 1024u64.pow(4)), "3.0 TB");
    }

    #[test]
    fn ages() {
        assert_eq!(format_age(30), "<1m");
        assert_eq!(format_age(5 * 60), "5m");
        assert_eq!(format_age(3 * 3600), "3h");
        assert_eq!(format_age(12 * 86400), "12d");
        assert_eq!(format_age(90 * 86400), "3mo");
        assert_eq!(format_age(800 * 86400), "2y");
        assert_eq!(format_age(-5), "<1m");
    }
}
