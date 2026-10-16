//! Local wall-clock time for timestamps, CTCP TIME, and logs.
//!
//! One place owns the libc calls so every platform builds: Unix uses
//! `localtime_r`, Windows uses `localtime_s`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

pub fn now() -> LocalTime {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    local(now)
}

/// Local time for a moment given in seconds since 1970.
pub fn from_unix(seconds: i64) -> LocalTime {
    local(seconds as libc::time_t)
}

fn local(now: libc::time_t) -> LocalTime {
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    #[cfg(unix)]
    unsafe {
        libc::localtime_r(&now, &mut tm);
    }
    #[cfg(windows)]
    unsafe {
        libc::localtime_s(&mut tm, &now);
    }
    LocalTime {
        year: tm.tm_year + 1900,
        month: tm.tm_mon + 1,
        day: tm.tm_mday,
        hour: tm.tm_hour,
        minute: tm.tm_min,
        second: tm.tm_sec,
    }
}

/// `HH:MM`, used for transcript timestamps and `$time`.
pub fn hm() -> String {
    let now = now();
    format!("{:02}:{:02}", now.hour, now.minute)
}

/// `YYYY-MM-DD HH:MM:SS`, used for CTCP TIME and log lines.
pub fn stamp() -> String {
    let now = now();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.year, now.month, now.day, now.hour, now.minute, now.second
    )
}

/// Seconds since 1970 for an IRCv3 `server-time` value such as
/// `2026-10-13T12:34:56.789Z`. Only UTC times (ending in `Z`) are read.
pub fn parse_server_time(value: &str) -> Option<i64> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;
    // Fixed-width digits only: `YYYY-MM-DDTHH:MM:SS`, never signs or huge
    // years, so the arithmetic below cannot overflow.
    let digits = |part: &str, width: usize| -> Option<i64> {
        (part.len() == width && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    let mut date = date.splitn(3, '-');
    let (year, month, day) = (
        digits(date.next()?, 4)?,
        digits(date.next()?, 2)?,
        digits(date.next()?, 2)?,
    );
    // A fraction of a second, if any, is digits only.
    let (time, fraction) = time.split_once('.').unwrap_or((time, "0"));
    if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut time = time.splitn(3, ':');
    let (hour, minute, second) = (
        digits(time.next()?, 2)?,
        digits(time.next()?, 2)?,
        digits(time.next()?, 2)?,
    );
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=12).contains(&month)
        || !(1..=days_in_month).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Days from 1970-01-01 to the date, for the proleptic Gregorian calendar.
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * m + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_server_times_are_refused() {
        assert_eq!(parse_server_time("1970-01-01T00:00:01.000Z"), Some(1));
        assert_eq!(
            parse_server_time("2026-10-13T12:34:56Z"),
            Some(1_791_894_896)
        );
        for bad in [
            "9223372036854775807-03-01T00:00:00Z",
            "2026-10-13T-1:00:00Z",
            "2026-10-13T+1:00:00Z",
            "2026-1-13T01:00:00Z",
            "2026-10-13T01:00:00",
            "-001-10-13T01:00:00Z",
            "2026-10-13T01:00:00.abcZ",
            "2026-10-13T01:00:00.Z",
            "2026-02-31T01:00:00Z",
            "2026-04-31T01:00:00Z",
            "2025-02-29T01:00:00Z",
            "1900-02-29T01:00:00Z",
        ] {
            assert_eq!(parse_server_time(bad), None, "{bad}");
        }
        // Leap days that exist, and a fraction of any length.
        assert!(parse_server_time("2024-02-29T01:00:00Z").is_some());
        assert!(parse_server_time("2000-02-29T01:00:00Z").is_some());
        assert_eq!(parse_server_time("1970-01-01T00:00:01.5Z"), Some(1));
    }

    #[test]
    fn formats_have_fixed_width() {
        assert_eq!(hm().len(), 5);
        let stamp = stamp();
        assert_eq!(stamp.len(), 19);
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[13..14], ":");
    }

    #[test]
    fn server_time_is_read_as_utc() {
        assert_eq!(parse_server_time("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_server_time("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(
            parse_server_time("2026-10-13T12:34:56.789Z"),
            Some(1_791_894_896)
        );
        assert_eq!(parse_server_time("2026-10-13T12:34:56"), None);
        assert_eq!(parse_server_time("nonsense"), None);
    }
}
