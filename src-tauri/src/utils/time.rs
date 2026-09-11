//! 时间与日期工具 —— 全仓唯一来源（不引入 chrono 依赖）。
//!
//! 历史背景：now_iso / 日期换算曾在 6 处独立实现（knowledge.rs、config.rs、
//! memory/store.rs、provider/mod.rs、provider_config.rs、mcp/mod.rs），
//! 且 knowledge.rs 的副本注释自认是"config.rs 的同款实现"——违反
//! AGENTS.md "never duplicate logic"。现统一收敛于此。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前时间的 ISO 8601 字符串（UTC，精度到秒，**不含** Z 后缀）。
pub fn now_iso() -> String {
    secs_to_iso(now_unix_secs())
}

/// 当前时间的 ISO 8601 字符串（UTC，**带** Z 后缀）。
/// 供 profile-model-overrides 等要求 Z 后缀的配置文件使用。
pub fn now_iso_z() -> String {
    format!("{}Z", now_iso())
}

/// 当前 UNIX 时间戳（秒）。
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 当前 UNIX 时间戳（秒）的字符串形式。
/// 供历史配置文件的 updated_at 字段使用（provider_config / mcp 的存量格式）。
pub fn now_unix_secs_string() -> String {
    format!("{}", now_unix_secs())
}

/// 将 UNIX 时间戳（秒）转为 YYYY-MM-DDTHH:MM:SS（UTC，不含 Z）。
pub fn secs_to_iso(secs: u64) -> String {
    let (y, mo, d, h, m, s) = secs_to_date_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

/// YYYY-MM-DDTHH:MM:SS（可带 Z）→ UNIX 秒；解析失败或早于 1970 返回 None。
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
    // days_from_civil 用 i64 计算以支持 1970 之前的日期；UNIX 秒语义下为负值即无效
    if total < 0 {
        return None;
    }
    Some(total as u64)
}

/// 公历换算：UNIX 秒 → (年, 月, 日, 时, 分, 秒)。
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

/// 闰年判定（Gregorian）。
pub fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Howard Hinnant days_from_civil：公历日期 → 自 1970-01-01 的天数。
/// 用 i64 计算，1970 之前的日期返回负值（调用方决定语义，如 iso_to_secs 拒绝负值）。
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Howard Hinnant civil_from_days：自 1970-01-01 的天数 → YYYY-MM-DD。
pub fn days_since_epoch_to_ymd(days: i64) -> Option<String> {
    // 算法参考：http://howardhinnant.github.io/date_algorithms.html
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

/// UNIX 秒时间戳 → YYYY-MM-DD（供 OpenAI created 字段等场景）。
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
        // 含闰日的往返：secs_to_iso ↔ iso_to_secs
        for iso in [
            "1970-01-01T00:00:00",
            "2000-02-29T23:59:59",
            "2024-02-29T12:34:56",
        ] {
            let secs = iso_to_secs(iso).unwrap();
            assert_eq!(secs_to_iso(secs), iso, "往返不一致: {iso}");
        }
        // 带 Z 后缀也应解析
        assert_eq!(
            iso_to_secs("2024-01-15T08:00:00Z"),
            iso_to_secs("2024-01-15T08:00:00")
        );
        assert_eq!(iso_to_secs("bad"), None);
    }

    #[test]
    fn leap_year_rules() {
        assert!(is_leap(2000)); // 400 整除 → 闰
        assert!(!is_leap(1900)); // 100 整除不闰
        assert!(is_leap(2024)); // 4 整除 → 闰
        assert!(!is_leap(2100)); // 100 整除不闰
                                 // 与 Hinnant 算法互证（含 1970 之前的日期：i64 负数路径）
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
        // 早于 1970 的 ISO 时间不应产生包裹溢出的假时间戳
        assert_eq!(iso_to_secs("1969-12-31T23:59:59"), None);
    }
}
