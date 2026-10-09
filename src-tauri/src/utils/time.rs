//! Time and date utilities — the single source repo-wide (no chrono dependency).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current time as an ISO 8601 string (UTC, second precision, **without** a Z suffix).
pub fn now_iso() -> String {
    secs_to_iso(now_unix_secs())
}

/// Current time as an ISO 8601 string (UTC, **with** a Z suffix).
/// For configs that require the Z suffix, e.g. profile-model-overrides.
pub fn now_iso_z() -> String {
    format!("{}Z", now_iso())
}

/// Current UNIX timestamp (seconds).
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Current UNIX timestamp (seconds) as a string.
/// For the updated_at field of legacy config files (the provider_config / mcp on-disk format).
pub fn now_unix_secs_string() -> String {
    format!("{}", now_unix_secs())
}

/// Converts a UNIX timestamp (seconds) to YYYY-MM-DDTHH:MM:SS (UTC, no Z).
pub fn secs_to_iso(secs: u64) -> String {
    let (y, mo, d, h, m, s) = secs_to_date_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

/// YYYY-MM-DDTHH:MM:SS (Z optional) → UNIX seconds; returns None on parse failure or dates before 1970.
pub fn iso_to_secs(iso: &str) -> Option<u64> {
    let s = iso.trim();
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let mo: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    let h: i64 = s[11..13].parse().ok()?;
    let mi: i64 = s[14..16].parse().ok()?;
    let se: i64 = s[17..19].parse().ok()?;
    let total = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se;
    // days_from_civil computes in i64 to support dates before 1970; a negative total is invalid in UNIX-seconds semantics
    if total < 0 {
        return None;
    }
    Some(total as u64)
}

/// Gregorian conversion: UNIX seconds → (year, month, day, hour, minute, second).
pub fn secs_to_date_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    const SECS_PER_DAY: u64 = 86400;
    let days = secs / SECS_PER_DAY;
    let time_secs = secs % SECS_PER_DAY;
    let hours = time_secs / 3600;
    let mins = (time_secs % 3600) / 60;
    let s = time_secs % 60;

    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days: &[u64] = if is_leap(y) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 1u64;
    for &md in month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        mo += 1;
    }
    (y, mo, remaining + 1, hours, mins, s)
}

/// Leap year check (Gregorian).
pub fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Howard Hinnant days_from_civil: Gregorian date → days since 1970-01-01.
/// Computed in i64; dates before 1970 yield negative values (semantics are up to the caller, e.g. iso_to_secs rejects them).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Howard Hinnant civil_from_days: days since 1970-01-01 → YYYY-MM-DD.
pub fn days_since_epoch_to_ymd(days: i64) -> Option<String> {
    // Algorithm reference: http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    Some(format!("{:04}-{:02}-{:02}", y, m, d))
}

/// UNIX seconds timestamp → YYYY-MM-DD (e.g. for the OpenAI created field).
pub fn unix_to_ymd(secs: i64) -> Option<String> {
    days_since_epoch_to_ymd(secs.div_euclid(86_400))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970_01_01() {
        assert_eq!(secs_to_iso(0), "1970-01-01T00:00:00");
        assert_eq!(secs_to_iso(86400), "1970-01-02T00:00:00");
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_since_epoch_to_ymd(0).as_deref(), Some("1970-01-01"));
        assert_eq!(unix_to_ymd(0).as_deref(), Some("1970-01-01"));
    }

    #[test]
    fn iso_roundtrip() {
        // Round-trip including a leap day: secs_to_iso ↔ iso_to_secs
        for iso in [
            "1970-01-01T00:00:00",
            "2000-02-29T23:59:59",
            "2024-02-29T12:34:56",
        ] {
            let secs = iso_to_secs(iso).unwrap();
            assert_eq!(secs_to_iso(secs), iso, "往返不一致: {iso}");
        }
        // The Z suffix should also parse
        assert_eq!(
            iso_to_secs("2024-01-15T08:00:00Z"),
            iso_to_secs("2024-01-15T08:00:00")
        );
        assert_eq!(iso_to_secs("bad"), None);
    }

    #[test]
    fn leap_year_rules() {
        assert!(is_leap(2000)); // divisible by 400 → leap
        assert!(!is_leap(1900)); // divisible by 100 → not leap
        assert!(is_leap(2024)); // divisible by 4 → leap
        assert!(!is_leap(2100)); // divisible by 100 → not leap
                                 // Cross-checked against the Hinnant algorithm (incl. dates before 1970: the i64 negative path)
        assert_eq!(
            days_from_civil(2000, 2, 29) + 1,
            days_from_civil(2000, 3, 1)
        );
        assert_eq!(
            days_from_civil(1900, 2, 28) + 1,
            days_from_civil(1900, 3, 1),
            "1900 非闰年"
        );
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        // ISO times before 1970 must not produce wrapped-around fake timestamps
        assert_eq!(iso_to_secs("1969-12-31T23:59:59"), None);
    }
}
