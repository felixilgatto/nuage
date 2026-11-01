//! Sizes and durations as typed by users (`10G`, `30d`).

/// Parses a size: a byte count, or a number followed by `K`, `M`, `G` or
/// `T` (powers of 1024, optionally followed by `i`, `B` or `iB`).
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let digits_end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (number, suffix) = s.split_at(digits_end);
    let number: f64 = number.parse().ok()?;
    let suffix = suffix.trim().to_ascii_lowercase();
    let suffix = suffix
        .strip_suffix("ib")
        .or_else(|| suffix.strip_suffix('b'))
        .or_else(|| suffix.strip_suffix('i'))
        .unwrap_or(&suffix);
    let factor: u64 = match suffix {
        "" => 1,
        "k" => 1 << 10,
        "m" => 1 << 20,
        "g" => 1 << 30,
        "t" => 1 << 40,
        _ => return None,
    };
    let bytes = number * factor as f64;
    (bytes.is_finite() && bytes >= 0.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
}

/// Formats a size so that [`parse_size`] reads it back exactly.
pub fn format_size(bytes: u64) -> String {
    for (suffix, factor) in [
        ("T", 1u64 << 40),
        ("G", 1 << 30),
        ("M", 1 << 20),
        ("K", 1 << 10),
    ] {
        if bytes >= factor && bytes % factor == 0 {
            return format!("{}{suffix}", bytes / factor);
        }
    }
    bytes.to_string()
}

/// Parses a duration: a number of seconds, or a number followed by `s`,
/// `m`, `h`, `d` or `w`.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    let digits_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (number, suffix) = s.split_at(digits_end);
    let number: u64 = number.parse().ok()?;
    let factor = match suffix.trim() {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 7 * 86400,
        _ => return None,
    };
    number.checked_mul(factor)
}

/// Formats a duration so that [`parse_duration`] reads it back exactly.
pub fn format_duration(secs: u64) -> String {
    for (suffix, factor) in [("w", 7 * 86400), ("d", 86400), ("h", 3600), ("m", 60)] {
        if secs >= factor && secs % factor == 0 {
            return format!("{}{suffix}", secs / factor);
        }
    }
    format!("{secs}s")
}

/// A duration in rclone's syntax (`"720h"`).
pub fn rclone_duration(secs: u64) -> String {
    format!("{secs}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("10G"), Some(10 << 30));
        assert_eq!(parse_size("10GiB"), Some(10 << 30));
        assert_eq!(parse_size("1.5g"), Some(3 << 29));
        assert_eq!(parse_size("500 M"), Some(500 << 20));
        assert_eq!(parse_size("1234"), Some(1234));
        assert_eq!(parse_size("ten"), None);
        assert_eq!(parse_size("5X"), None);
        assert_eq!(format_size(20 << 30), "20G");
        assert_eq!(format_size(1536 << 20), "1536M");
        assert_eq!(format_size(1000), "1000");
        for n in [0, 1, 1023, 1 << 20, 3 << 29, 7 << 40] {
            assert_eq!(parse_size(&format_size(n)), Some(n));
        }
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30d"), Some(30 * 86400));
        assert_eq!(parse_duration("720h"), Some(720 * 3600));
        assert_eq!(parse_duration("90"), Some(90));
        assert_eq!(parse_duration("1y"), None);
        assert_eq!(format_duration(30 * 86400), "30d");
        assert_eq!(format_duration(14 * 86400), "2w");
        assert_eq!(format_duration(90), "90s");
        assert_eq!(parse_duration(&format_duration(3600)), Some(3600));
    }
}
