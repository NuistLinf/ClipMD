//! 日期时间：Unix 秒数和 ISO 8601 互转、本地日期、列表里的"3 分钟前"。
//!
//! 不引入日期库：只需要公历换算，时区偏移由调用方（Windows 的 GetTimeZoneInformation）传进来。

use std::time::{SystemTime, UNIX_EPOCH};

/// 本地日期，文件名前缀用
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl LocalDate {
    /// `2026-10-24`
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// 现在的 Unix 秒数
pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn unix_of(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// 公历日期 → 从 1970-01-01 起的天数（Howard Hinnant 的算法）
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 天数 → 公历日期
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + if m <= 2 { 1 } else { 0 }) as i32, m, d)
}

/// 本地时间的各个部分
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub date: LocalDate,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 从 1970-01-01 起的本地天数，比较"是不是同一天"用
    pub day_number: i64,
}

/// `utc_offset` 是本地时间比 UTC 快多少秒（北京时间是 28800）
pub fn local_time(unix: i64, utc_offset: i64) -> LocalTime {
    let local = unix + utc_offset;
    let day_number = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(day_number);
    LocalTime {
        date: LocalDate { year, month, day },
        hour: (secs / 3600) as u32,
        minute: (secs % 3600 / 60) as u32,
        second: (secs % 60) as u32,
        day_number,
    }
}

/// `2026-10-24T08:00:00Z`，和 Swift 的 `JSONEncoder.dateEncodingStrategy = .iso8601` 一样
pub fn format_iso8601(unix: i64) -> String {
    let t = local_time(unix, 0);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.date.year, t.date.month, t.date.day, t.hour, t.minute, t.second
    )
}

/// 解析 `2026-10-24T08:00:00Z`、`2026-10-24T16:00:00.123+08:00` 这类时间
pub fn parse_iso8601(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<i64> { text.get(range)?.parse().ok() };
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(stripped) = rest.strip_prefix('.') {
        let digits = stripped.find(|c: char| !c.is_ascii_digit()).unwrap_or(stripped.len());
        rest = &stripped[digits..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() != 4 {
                return None;
            }
            let hours: i64 = digits[..2].parse().ok()?;
            let minutes: i64 = digits[2..].parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let days = days_from_civil(year as i32, month as u32, day as u32);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// 列表里显示的时间："刚刚""3 分钟前""14:02""昨天""10月24日"。已保存和剪贴板两个列表共用
pub fn relative_label(date: i64, now: i64, utc_offset: i64) -> String {
    let interval = now - date;
    if interval < 60 {
        return "刚刚".into();
    }
    if interval < 3600 {
        return format!("{} 分钟前", interval / 60);
    }
    let then = local_time(date, utc_offset);
    let today = local_time(now, utc_offset);
    if then.day_number == today.day_number {
        return format!("{:02}:{:02}", then.hour, then.minute);
    }
    if then.day_number == today.day_number - 1 {
        return "昨天".into();
    }
    if then.date.year == today.date.year {
        format!("{}月{}日", then.date.month, then.date.day)
    } else {
        format!("{}年{}月{}日", then.date.year, then.date.month, then.date.day)
    }
}

/// ClipItem 的日期在 JSON 里存成 ISO 8601 字符串，和 macOS 版的记录文件格式一样
pub mod iso8601_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::format_iso8601(*value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        let text = String::deserialize(deserializer)?;
        super::parse_iso8601(&text).ok_or_else(|| serde::de::Error::custom("日期格式不对"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-1_000_000, -1, 0, 1, 20_000, 20_750, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn iso8601() {
        let t = parse_iso8601("2026-10-24T08:00:00Z").unwrap();
        assert_eq!(format_iso8601(t), "2026-10-24T08:00:00Z");
        assert_eq!(parse_iso8601("2026-10-24T16:00:00.250+08:00"), Some(t));
        assert_eq!(parse_iso8601("2026-10-24T03:00:00-05:00"), Some(t));
        assert_eq!(parse_iso8601("不是时间"), None);
        assert_eq!(parse_iso8601("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn labels() {
        let offset = 8 * 3600;
        // 北京时间 2026-10-24 15:30
        let now = parse_iso8601("2026-10-24T15:30:00+08:00").unwrap();
        assert_eq!(relative_label(now - 10, now, offset), "刚刚");
        assert_eq!(relative_label(now - 180, now, offset), "3 分钟前");
        assert_eq!(relative_label(now - 2 * 3600, now, offset), "13:30");
        assert_eq!(relative_label(now - 20 * 3600, now, offset), "昨天");
        assert_eq!(relative_label(now - 5 * 86_400, now, offset), "10月19日");
        assert_eq!(relative_label(now - 400 * 86_400, now, offset), "2025年9月19日");
        assert_eq!(local_time(now, offset).date.iso(), "2026-10-24");
    }
}
