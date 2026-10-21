// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 날짜 일련 번호.
//!
//! 1900 날짜 체계: 1 = 1900-01-01, 60 = 1900-02-29 (실제로 없는 날 — Lotus 호환 오류를 그대로 둔다),
//! 61 = 1900-03-01, 0 = "1900-01-00". 1904 체계(`workbookPr/@date1904`): 0 = 1904-01-01.
//! 시각은 하루의 비율(소수 부분)이다.

/// 9999-12-31 (1900 체계)
pub const MAX_SERIAL_1900: f64 = 2_958_465.0;
/// 9999-12-31 (1904 체계)
pub const MAX_SERIAL_1904: f64 = 2_957_003.0;

pub fn max_serial(date1904: bool) -> f64 {
    if date1904 {
        MAX_SERIAL_1904
    } else {
        MAX_SERIAL_1900
    }
}

/// 그레고리력 날짜 → 1970-01-01 기준 일수
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 1970-01-01 기준 일수 → 그레고리력 날짜
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 1899-12-30 (1900 체계의 61 이후 기준점)
fn epoch_1900() -> i64 {
    days_from_civil(1899, 12, 30)
}

fn epoch_1904() -> i64 {
    days_from_civil(1904, 1, 1)
}

pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

/// 연·월(넘침 허용)·일(넘침 허용) → 일련 번호 (DATE 함수 규칙). 범위를 벗어나면 None.
pub fn serial_from_ymd(year: i64, month: i64, day: i64, date1904: bool) -> Option<f64> {
    let total = year * 12 + (month - 1);
    let y = total.div_euclid(12);
    let m = (total.rem_euclid(12) + 1) as u32;
    let serial = if date1904 {
        days_from_civil(y, m, 1) - epoch_1904() + day - 1
    } else {
        // 그 달의 "0일" 일련 번호. 1900년 1·2월은 2월 29일이 있는 달력으로 센다.
        let day0 = match (y, m) {
            (1900, 1) => 0,
            (1900, 2) => 31,
            _ => days_from_civil(y, m, 1) - epoch_1900() - 1,
        };
        if y < 1900 {
            return None;
        }
        day0 + day
    };
    let s = serial as f64;
    (s >= 0.0 && s <= max_serial(date1904)).then_some(s)
}

/// 일련 번호(정수 부분) → (연, 월, 일). 0 은 1900-01-00.
pub fn ymd_from_serial(serial: f64, date1904: bool) -> Option<(i64, u32, u32)> {
    if !serial.is_finite() || serial < 0.0 || serial >= max_serial(date1904) + 1.0 {
        return None;
    }
    let n = serial.floor() as i64;
    if date1904 {
        return Some(civil_from_days(n + epoch_1904()));
    }
    Some(match n {
        0 => (1900, 1, 0),
        1..=31 => (1900, 1, n as u32),
        32..=60 => (1900, 2, (n - 31) as u32),
        _ => civil_from_days(n + epoch_1900()),
    })
}

/// 요일 (0 = 일요일). 1900 체계의 1–59 는 오류를 품은 달력 그대로 (1900-01-01 은 일요일).
pub fn weekday(serial: f64, date1904: bool) -> u32 {
    let n = serial.floor() as i64;
    // 1900 체계: 1 = 일요일 (1900-01-01 을 일요일로 센다), 61 이후는 실제 요일과 같다
    let n = if date1904 { n + 1462 } else { n };
    ((n + 6).rem_euclid(7)) as u32
}

/// 시각 부분 → (시, 분, 초). 가장 가까운 초로 반올림한다.
pub fn hms_from_serial(serial: f64) -> (u32, u32, u32) {
    let frac = serial - serial.floor();
    let mut secs = (frac * 86_400.0).round() as i64;
    if secs >= 86_400 {
        secs -= 86_400;
    }
    ((secs / 3600) as u32, ((secs / 60) % 60) as u32, (secs % 60) as u32)
}

/// `t="d"` 셀 값 (ISO 8601) → 일련 번호
pub fn parse_iso(s: &str, date1904: bool) -> Option<f64> {
    let s = s.trim();
    let (d, t) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let dd: i64 = it.next()?.parse().ok()?;
    let mut v = serial_from_ymd(y, m, dd, date1904)?;
    if let Some(t) = t {
        let t = t.trim_end_matches('Z');
        let mut p = t.split(':');
        let h: f64 = p.next()?.parse().ok()?;
        let mi: f64 = p.next().unwrap_or("0").parse().ok()?;
        let se: f64 = p.next().unwrap_or("0").parse().ok()?;
        v += (h * 3600.0 + mi * 60.0 + se) / 86_400.0;
    }
    Some(v)
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn month_name(s: &str) -> Option<u32> {
    let l = s.to_ascii_lowercase();
    if l.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| l.starts_with(m))
        .map(|i| i as u32 + 1)
        .filter(|_| {
            const FULL: [&str; 12] = [
                "january",
                "february",
                "march",
                "april",
                "may",
                "june",
                "july",
                "august",
                "september",
                "october",
                "november",
                "december",
            ];
            l.len() == 3 || FULL.iter().any(|f| f.starts_with(&l))
        })
}

/// 글 → 시각 (하루의 비율). `10:30`, `10:30:15`, `10:30 PM`, `오후 3:00`, `1:30.5`
pub fn parse_time(s: &str) -> Option<f64> {
    let mut t = s.trim().to_string();
    let mut pm: Option<bool> = None;
    for (tag, is_pm) in [
        ("오전", false),
        ("오후", true),
        ("AM", false),
        ("PM", true),
        ("am", false),
        ("pm", true),
        ("A", false),
        ("P", true),
        ("a", false),
        ("p", true),
    ] {
        if let Some(r) = t.strip_suffix(tag) {
            if r.ends_with(|c: char| c.is_ascii_digit() || c == ' ') {
                pm = Some(is_pm);
                t = r.trim_end().to_string();
                break;
            }
        }
        if let Some(r) = t.strip_prefix(tag)
            && (tag == "오전" || tag == "오후")
        {
            pm = Some(is_pm);
            t = r.trim_start().to_string();
            break;
        }
    }
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        // `10 PM` 같이 시만 있는 꼴
        if pm.is_some() && parts.len() == 1 {
            let h: f64 = parts[0].trim().parse().ok()?;
            return hour12(h, 0.0, 0.0, pm);
        }
        return None;
    }
    if parts.iter().any(|p| p.trim().is_empty()) {
        return None;
    }
    let h: f64 = parts[0].trim().parse().ok()?;
    let m: f64 = parts[1].trim().parse().ok()?;
    let sec: f64 = match parts.get(2) {
        Some(x) => x.trim().parse().ok()?,
        None => 0.0,
    };
    if h < 0.0 || m < 0.0 || sec < 0.0 || h.fract() != 0.0 || m.fract() != 0.0 {
        return None;
    }
    if parts.len() == 2 && m >= 60.0 {
        return None;
    }
    if parts.len() == 3 && (m >= 60.0 || sec >= 60.0) {
        return None;
    }
    hour12(h, m, sec, pm)
}

fn hour12(h: f64, m: f64, s: f64, pm: Option<bool>) -> Option<f64> {
    let h = match pm {
        Some(is_pm) => {
            if h > 12.0 || h < 0.0 {
                return None;
            }
            let h = if h == 12.0 { 0.0 } else { h };
            if is_pm { h + 12.0 } else { h }
        }
        None => h,
    };
    Some((h * 3600.0 + m * 60.0 + s) / 86_400.0)
}

/// 오늘의 연도 (날짜 글에 연도가 없을 때 쓴다)
fn current_year() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days((now + 9 * 3600) / 86_400).0
}

/// 글 → 날짜 (+시각) 일련 번호. 한국어 환경의 날짜 꼴:
/// `2026-10-19`, `2026/10/19`, `2026.10.19`, `2026년 10월 19일`, `10-19`, `10/19`(올해), `19-Oct-2026`, `Oct 19, 2026`
pub fn parse_datetime(s: &str, date1904: bool) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(t) = parse_time(s) {
        return Some(t);
    }
    // 날짜 + 시각
    let (dpart, tpart) = split_date_time(s);
    let d = parse_date(dpart, date1904)?;
    match tpart {
        Some(t) => Some(d + parse_time(t)?),
        None => Some(d),
    }
}

fn split_date_time(s: &str) -> (&str, Option<&str>) {
    // 마지막 공백 뒤가 시각이면 나눈다
    if let Some(i) = s.find(':') {
        let head = &s[..i];
        if let Some(sp) = head.rfind(' ') {
            // `오후 3:00` 처럼 앞에 오전/오후가 있으면 그 앞에서 나눈다
            let mut cut = sp;
            let before = s[..sp].trim_end();
            if before.ends_with("오전") || before.ends_with("오후") {
                cut = before.len() - "오전".len();
            }
            return (s[..cut].trim(), Some(s[cut..].trim()));
        }
    }
    (s, None)
}

fn parse_date(s: &str, date1904: bool) -> Option<f64> {
    let s = s.trim();
    // 한국어: 2026년 10월 19일
    if s.contains('년') || s.contains('월') {
        let mut y: Option<i64> = None;
        let mut m: Option<i64> = None;
        let mut d: Option<i64> = None;
        let mut num = String::new();
        for ch in s.chars() {
            if ch.is_ascii_digit() {
                num.push(ch);
                continue;
            }
            let v = num.parse::<i64>().ok();
            match ch {
                '년' => y = v,
                '월' => m = v,
                '일' => d = v,
                ' ' => {
                    if !num.is_empty() {
                        return None;
                    }
                }
                _ => return None,
            }
            num.clear();
        }
        if !num.is_empty() {
            return None;
        }
        let y = expand_year(y.unwrap_or_else(current_year))?;
        return ymd_checked(y, m?, d.unwrap_or(1), date1904);
    }
    let seps: &[char] = &['-', '/', '.'];
    let parts: Vec<&str> = s.split(seps).map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        // `2026.10.19.` 꼴의 끝 점은 허용
        if s.ends_with('.') && parts.len() >= 3 && parts[..parts.len() - 1].iter().all(|p| !p.is_empty()) {
            return parse_date(&s[..s.len() - 1], date1904);
        }
        return None;
    }
    let is_num = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit());
    match parts.len() {
        3 => {
            let (a, b, c) = (parts[0], parts[1], parts[2]);
            if is_num(a) && is_num(b) && is_num(c) {
                let (av, bv, cv): (i64, i64, i64) =
                    (a.parse().ok()?, b.parse().ok()?, c.parse().ok()?);
                if a.len() >= 3 || av > 31 {
                    // 연-월-일
                    return ymd_checked(expand_year(av)?, bv, cv, date1904);
                }
                // 월/일/연
                return ymd_checked(expand_year(cv)?, av, bv, date1904);
            }
            // 19-Oct-2026
            if is_num(a) && is_num(c) {
                let m = month_name(b)? as i64;
                return ymd_checked(expand_year(c.parse().ok()?)?, m, a.parse().ok()?, date1904);
            }
            None
        }
        2 => {
            let (a, b) = (parts[0], parts[1]);
            if is_num(a) && is_num(b) {
                let (av, bv): (i64, i64) = (a.parse().ok()?, b.parse().ok()?);
                if a.len() >= 3 || av > 12 {
                    // 연-월
                    return ymd_checked(expand_year(av)?, bv, 1, date1904);
                }
                // 월-일 (올해)
                if bv > days_in_month(current_year(), av.clamp(1, 12) as u32) as i64 {
                    // 월-연
                    return ymd_checked(expand_year(bv)?, av, 1, date1904);
                }
                return ymd_checked(current_year(), av, bv, date1904);
            }
            // 19-Oct / Oct-2026
            if is_num(a) {
                let m = month_name(b)? as i64;
                return ymd_checked(current_year(), m, a.parse().ok()?, date1904);
            }
            if is_num(b) {
                let m = month_name(a)? as i64;
                let bv: i64 = b.parse().ok()?;
                if bv > 31 || b.len() >= 3 {
                    return ymd_checked(expand_year(bv)?, m, 1, date1904);
                }
                return ymd_checked(current_year(), m, bv, date1904);
            }
            None
        }
        1 => {
            // Oct 19, 2026 / October 19 2026
            let words: Vec<&str> = s
                .split(|c: char| c == ' ' || c == ',')
                .filter(|w| !w.is_empty())
                .collect();
            if words.len() >= 2 {
                let m = month_name(words[0])? as i64;
                let d: i64 = words[1].parse().ok()?;
                let y = match words.get(2) {
                    Some(y) => expand_year(y.parse().ok()?)?,
                    None => current_year(),
                };
                if words.len() > 3 {
                    return None;
                }
                return ymd_checked(y, m, d, date1904);
            }
            None
        }
        _ => None,
    }
}

/// 두 자리 연도: 0–29 → 2000년대, 30–99 → 1900년대
fn expand_year(y: i64) -> Option<i64> {
    match y {
        0..=29 => Some(2000 + y),
        30..=99 => Some(1900 + y),
        100..=9999 => Some(y),
        _ => None,
    }
}

fn ymd_checked(y: i64, m: i64, d: i64, date1904: bool) -> Option<f64> {
    if !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let dim = if y == 1900 && m == 2 && !date1904 {
        29
    } else {
        days_in_month(y, m as u32) as i64
    };
    if d > dim {
        return None;
    }
    serial_from_ymd(y, m, d, date1904)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials_1900() {
        assert_eq!(serial_from_ymd(1900, 1, 1, false), Some(1.0));
        assert_eq!(serial_from_ymd(1900, 2, 28, false), Some(59.0));
        assert_eq!(serial_from_ymd(1900, 2, 29, false), Some(60.0));
        assert_eq!(serial_from_ymd(1900, 3, 1, false), Some(61.0));
        assert_eq!(serial_from_ymd(2026, 10, 19, false), Some(46314.0));
        assert_eq!(serial_from_ymd(1900, 1, 0, false), Some(0.0));
        assert_eq!(serial_from_ymd(9999, 12, 31, false), Some(MAX_SERIAL_1900));
        assert_eq!(serial_from_ymd(2026, 13, 1, false), serial_from_ymd(2027, 1, 1, false));
        assert_eq!(ymd_from_serial(60.0, false), Some((1900, 2, 29)));
        assert_eq!(ymd_from_serial(61.0, false), Some((1900, 3, 1)));
        assert_eq!(ymd_from_serial(46314.75, false), Some((2026, 10, 19)));
        assert_eq!(ymd_from_serial(0.0, false), Some((1900, 1, 0)));
    }

    #[test]
    fn serials_1904() {
        assert_eq!(serial_from_ymd(1904, 1, 1, true), Some(0.0));
        assert_eq!(serial_from_ymd(2026, 10, 19, true), Some(46314.0 - 1462.0));
        assert_eq!(ymd_from_serial(0.0, true), Some((1904, 1, 1)));
    }

    #[test]
    fn text_dates() {
        let p = |s| parse_datetime(s, false);
        assert_eq!(p("2026-10-19"), Some(46314.0));
        assert_eq!(p("2026/10/19"), Some(46314.0));
        assert_eq!(p("2026년 10월 19일"), Some(46314.0));
        assert_eq!(p("10/19/2026"), Some(46314.0));
        assert_eq!(p("19-Oct-2026"), Some(46314.0));
        assert_eq!(p("12:00"), Some(0.5));
        assert_eq!(p("2026-10-19 18:00"), Some(46314.75));
        assert_eq!(p("6:00 PM"), Some(0.75));
        assert_eq!(p("2026-02-30"), None);
    }

    #[test]
    fn weekdays() {
        // 2026-10-19 은 월요일
        assert_eq!(weekday(46314.0, false), 1);
        assert_eq!(weekday(1.0, false), 0);
    }
}
