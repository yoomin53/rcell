// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 날짜 일련 번호와 날짜·시각 글.
//!
//! 1900 날짜 체계: 1 = 1900-01-01, 60 = 1900-02-29 (실제로 없는 날 — Lotus 호환 오류를 그대로 둔다),
//! 61 = 1900-03-01, 0 = "1900-01-00". 한셀의 달력은 날짜 계산(EDATE, DAYS360, YEARFRAC, DATEDIF …)에서도
//! 1900년을 윤년으로 센다 (함수 대조 실험). 1904 체계(`workbookPr/@date1904`)는 1900 체계에서 1462 를 뺀
//! 것이고, 한셀은 1904 이전 날짜를 음수로 둔다 (DATE(1900,1,1) = −1461). 시각은 하루의 비율이다.

use std::cell::Cell;

/// 9999-12-31 (1900 체계)
pub const MAX_SERIAL_1900: f64 = 2_958_465.0;
/// 9999-12-31 (1904 체계)
pub const MAX_SERIAL_1904: f64 = 2_957_003.0;
/// 1904 체계 = 1900 체계 − 1462
pub const OFFSET_1904: i64 = 1462;

pub fn max_serial(date1904: bool) -> f64 {
    if date1904 {
        MAX_SERIAL_1904
    } else {
        MAX_SERIAL_1900
    }
}

/// 1904 체계에서 가장 이른 일련 번호 (1900-01-00)
pub fn min_serial(date1904: bool) -> f64 {
    if date1904 { -(OFFSET_1904 as f64) } else { 0.0 }
}

thread_local! {
    /// 연도 없는 날짜 글(`1/15`, `Jan 1`)이 쓰는 해. 엔진이 계산 시각에서 정한다.
    static THIS_YEAR: Cell<Option<i64>> = const { Cell::new(None) };
}

/// 연도 없는 날짜 글의 해를 정한다 (None 이면 시스템 시각)
pub fn set_this_year(y: Option<i64>) {
    THIS_YEAR.with(|c| c.set(y));
}

/// 오늘의 연도 (날짜 글에 연도가 없을 때 쓴다)
pub fn current_year() -> i64 {
    if let Some(y) = THIS_YEAR.with(|c| c.get()) {
        return y;
    }
    let now = crate::calc::unix_now() as i64;
    civil_from_days((now + 9 * 3600) / 86_400).0
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

/// 윤년 (한셀 달력: 1900 도 윤년)
pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 || y == 1900
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
    if y < 1900 {
        return None;
    }
    // 그 달의 "0일" 일련 번호 (1900 체계). 1900년 1·2월은 2월 29일이 있는 달력으로 센다.
    let day0 = match (y, m) {
        (1900, 1) => 0,
        (1900, 2) => 31,
        _ => days_from_civil(y, m, 1) - epoch_1900() - 1,
    };
    let mut s = (day0 + day) as f64;
    if date1904 {
        s -= OFFSET_1904 as f64;
    }
    (s >= min_serial(date1904) && s <= max_serial(date1904)).then_some(s)
}

/// 일련 번호(정수 부분) → (연, 월, 일). 0 은 1900-01-00.
pub fn ymd_from_serial(serial: f64, date1904: bool) -> Option<(i64, u32, u32)> {
    if !serial.is_finite() || serial < min_serial(date1904) || serial >= max_serial(date1904) + 1.0
    {
        return None;
    }
    let mut n = serial.floor() as i64;
    if date1904 {
        n += OFFSET_1904;
    }
    Some(match n {
        0 => (1900, 1, 0),
        1..=31 => (1900, 1, n as u32),
        32..=60 => (1900, 2, (n - 31) as u32),
        _ => civil_from_days(n + epoch_1900()),
    })
}

/// 달 계산(EDATE, EOMONTH, 이자 지급일)용 (연, 월, 일): 일련 번호 0 은 1899-12-31 로 본다 (한셀:
/// EDATE(0,1) = 31, EOMONTH(0,1) = 31).
pub fn ymd_for_months(serial: f64, date1904: bool) -> Option<(i64, u32, u32)> {
    match ymd_from_serial(serial, date1904)? {
        (1900, 1, 0) => Some((1899, 12, 31)),
        x => Some(x),
    }
}

/// 요일 (0 = 일요일). 1900 체계의 1–59 는 오류를 품은 달력 그대로 (1900-01-01 은 일요일).
pub fn weekday(serial: f64, date1904: bool) -> u32 {
    let n = serial.floor() as i64;
    // 1900 체계: 1 = 일요일 (1900-01-01 을 일요일로 센다), 61 이후는 실제 요일과 같다
    let n = if date1904 { n + OFFSET_1904 } else { n };
    ((n + 6).rem_euclid(7)) as u32
}

/// 시각 부분 → (시, 분, 초). 1/1000 초로 반올림한 뒤 초로 반올림한다 (한셀: 59.5초 → 1분,
/// 0.5초 → 1초).
pub fn hms_from_serial(serial: f64) -> (u32, u32, u32) {
    let frac = serial - serial.floor();
    let ms = (frac * 86_400_000.0).round() as i64;
    let mut secs = (ms + 500) / 1000;
    if secs >= 86_400 {
        secs -= 86_400;
    }
    (
        (secs / 3600) as u32,
        ((secs / 60) % 60) as u32,
        (secs % 60) as u32,
    )
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

// ---------------------------------------------------------------------------
// 글 → 숫자·날짜·시각 (한셀의 문법, 함수 대조 실험 conv 묶음)
// ---------------------------------------------------------------------------

/// 글을 읽은 결과
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Parsed {
    Number(f64),
    Date(f64),
    /// 시각만 (하루를 넘을 수 있다: `25:00` = 1.0417)
    Time(f64),
    /// (날짜, 시각) — TIMEVALUE 는 시각 부분을 따로 쓴다 (더하고 빼면 끝자리가 바뀐다)
    DateTime(f64, f64),
}

impl Parsed {
    pub fn value(self) -> f64 {
        match self {
            Parsed::Number(v) | Parsed::Date(v) | Parsed::Time(v) => v,
            Parsed::DateTime(d, t) => d + t,
        }
    }
}

/// 숫자 꼴 → 대분수 → 시각 → 날짜(·시각) 순으로 읽는다.
pub fn classify_text(s: &str, date1904: bool) -> Option<Parsed> {
    if let Some(v) = crate::calc::value::parse_plain_number(s) {
        return Some(Parsed::Number(v));
    }
    if let Some(v) = crate::calc::value::parse_fraction(s) {
        return Some(Parsed::Number(v));
    }
    if let Some(t) = parse_time(s, true) {
        return Some(Parsed::Time(t));
    }
    if let Some(d) = parse_date(s, date1904) {
        return Some(Parsed::Date(d));
    }
    // 날짜 + 공백 + 시각
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c != b' ' || i == 0 {
            continue;
        }
        let (d, t) = (&s[..i], s[i..].trim_start_matches(' '));
        if t.is_empty() || d.ends_with(' ') {
            continue;
        }
        if let (Some(dv), Some(tv)) = (parse_date(d, date1904), parse_time(t, false)) {
            return Some(Parsed::DateTime(dv, tv));
        }
    }
    None
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}

/// 시각: `H:M[:S[.소수]]`, 뒤에 `AM`·`PM`·`오전`·`오후`(공백 있어도 없어도). `10 AM` 처럼 시만 쓰면 오전·오후가
/// 있어야 한다. 분·초는 60 을 넘어도 된다 (`1:90` = 2:30). 오전·오후가 있으면 시는 1–12. 날짜 뒤의 시각은
/// 23시까지 (`standalone` 이 아니면).
pub fn parse_time(s: &str, standalone: bool) -> Option<f64> {
    let mut body = s;
    let mut pm: Option<bool> = None;
    for (tag, is_pm) in [("AM", false), ("PM", true), ("오전", false), ("오후", true)] {
        let n = tag.len();
        if body.len() >= n && body.is_char_boundary(body.len() - n) {
            let (head, tail) = body.split_at(body.len() - n);
            if tail.eq_ignore_ascii_case(tag) {
                body = head.trim_end_matches(' ');
                pm = Some(is_pm);
                break;
            }
        }
    }
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() > 3 || (parts.len() == 1 && pm.is_none()) {
        return None;
    }
    let h = parts[0];
    if !all_digits(h) {
        return None;
    }
    let h: f64 = h.parse().ok()?;
    let m: f64 = match parts.get(1) {
        Some(m) if all_digits(m) => m.parse().ok()?,
        Some(_) => return None,
        None => 0.0,
    };
    let sec: f64 = match parts.get(2) {
        Some(x) => {
            let (i, f) = x.split_once('.').unwrap_or((x, ""));
            if !all_digits(i)
                || (!f.is_empty() && !all_digits(f))
                || (x.contains('.') && f.is_empty())
            {
                return None;
            }
            x.parse().ok()?
        }
        None => 0.0,
    };
    let h = match pm {
        Some(is_pm) => {
            if !(1.0..=12.0).contains(&h) {
                return None;
            }
            let h = if h == 12.0 { 0.0 } else { h };
            if is_pm { h + 12.0 } else { h }
        }
        None => {
            if !standalone && h > 23.0 {
                return None;
            }
            h
        }
    };
    Some((h * 3600.0 + m * 60.0 + sec) / 86_400.0)
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const MONTH_NAMES: [&str; 12] = [
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

/// 달 이름 → 번호. `full` 이면 전체 이름도 받는다 (세 글자 약어는 늘 받는다).
fn month_name(s: &str, full: bool) -> Option<i64> {
    let l = s.to_ascii_lowercase();
    if let Some(i) = MONTHS.iter().position(|m| *m == l) {
        return Some(i as i64 + 1);
    }
    if full && let Some(i) = MONTH_NAMES.iter().position(|m| *m == l) {
        return Some(i as i64 + 1);
    }
    None
}

/// 두 자리 이하 연도: 0–29 → 2000년대, 30–99 → 1900년대. 네 자리는 그대로, 세 자리는 안 된다.
fn year_of(s: &str) -> Option<i64> {
    if !all_digits(s) {
        return None;
    }
    let y: i64 = s.parse().ok()?;
    match s.len() {
        1 | 2 => Some(if y < 30 { 2000 + y } else { 1900 + y }),
        4 => Some(y),
        _ => None,
    }
}

fn num(s: &str) -> Option<i64> {
    if all_digits(s) && s.len() <= 5 {
        s.parse().ok()
    } else {
        None
    }
}

fn ymd_checked(y: i64, m: i64, d: i64, date1904: bool) -> Option<f64> {
    if !(1..=12).contains(&m) || d < 1 || !(1900..=9999).contains(&y) {
        return None;
    }
    if d > days_in_month(y, m as u32) as i64 {
        return None;
    }
    // 글 날짜는 기준일 이후만 (1904 체계에서 DATEVALUE("1903-12-31") 은 #VALUE! — DATE 는 음수를 낸다)
    serial_from_ymd(y, m, d, date1904).filter(|&s| s >= 0.0)
}

/// 날짜 글 (한국어 환경의 한셀이 받는 꼴):
/// - 숫자 `y-m-d`·`y/m/d` (`/` 양옆 공백 허용). 안 되면 `-` 일 때만 `m-d-y`. 두 부분은 첫 부분이 네 자리면
///   `y-m`(1일), 아니면 `m-d`(올해).
/// - `Mon D`, `D Mon`, `D-Mon`(올해), `Mon D, Y`, `D-Mon-Y`, `Mon-Y`(Y 가 31 이하면 올해의 그날).
///   달 이름은 세 글자 약어, `Mon D, Y`·`D-Mon-Y` 에서는 전체 이름도.
/// - `Y년 M월 D일` (한 칸씩 띄어서).
///
/// 앞뒤 공백이 있으면 날짜가 아니다.
pub fn parse_date(s: &str, date1904: bool) -> Option<f64> {
    if s.is_empty() || s.starts_with(' ') || s.ends_with(' ') {
        return None;
    }
    // 한국어: 2024년 1월 15일
    if s.contains('년') {
        let parts: Vec<&str> = s.split(' ').collect();
        if parts.len() != 3 {
            return None;
        }
        let y = year_of(parts[0].strip_suffix('년')?)?;
        let m = num(parts[1].strip_suffix('월')?)?;
        let d = num(parts[2].strip_suffix('일')?)?;
        return ymd_checked(y, m, d, date1904);
    }
    let this = current_year();
    // 달 이름이 있는 꼴
    if s.bytes().any(|c| c.is_ascii_alphabetic()) {
        // Mon D, Y
        if let Some((md, y)) = s.split_once(", ") {
            let (mo, d) = md.split_once(' ')?;
            return ymd_checked(year_of(y)?, month_name(mo, true)?, num(d)?, date1904);
        }
        let sp: Vec<&str> = s.split(' ').collect();
        if sp.len() == 2 {
            // Mon D / D Mon (약어만)
            if let (Some(m), Some(d)) = (month_name(sp[0], false), num(sp[1])) {
                return ymd_checked(this, m, d, date1904);
            }
            if let (Some(d), Some(m)) = (num(sp[0]), month_name(sp[1], false)) {
                return ymd_checked(this, m, d, date1904);
            }
            return None;
        }
        let dash: Vec<&str> = s.split('-').collect();
        match dash.len() {
            3 => {
                let d = num(dash[0])?;
                let m = month_name(dash[1], true)?;
                return ymd_checked(year_of(dash[2])?, m, d, date1904);
            }
            2 => {
                if let (Some(d), Some(m)) = (num(dash[0]), month_name(dash[1], false)) {
                    return ymd_checked(this, m, d, date1904);
                }
                let m = month_name(dash[0], false)?;
                let n = dash[1];
                if !all_digits(n) {
                    return None;
                }
                if n.len() <= 2 && n.parse::<i64>().ok()? <= 31 {
                    return ymd_checked(this, m, n.parse().ok()?, date1904);
                }
                return ymd_checked(year_of(n)?, m, 1, date1904);
            }
            _ => return None,
        }
    }
    // 숫자 꼴
    let sep = if s.contains('/') {
        '/'
    } else if s.contains('-') {
        '-'
    } else {
        return None;
    };
    let parts: Vec<&str> = s
        .split(sep)
        .map(|p| if sep == '/' { p.trim_matches(' ') } else { p })
        .collect();
    if parts.iter().any(|p| !all_digits(p)) {
        return None;
    }
    match parts.len() {
        3 => {
            if let Some(y) = year_of(parts[0])
                && let Some(v) = ymd_checked(y, num(parts[1])?, num(parts[2])?, date1904)
            {
                return Some(v);
            }
            if sep == '-' {
                let y = year_of(parts[2])?;
                return ymd_checked(y, num(parts[0])?, num(parts[1])?, date1904);
            }
            None
        }
        2 => {
            if parts[0].len() == 4 {
                ymd_checked(year_of(parts[0])?, num(parts[1])?, 1, date1904)
            } else {
                ymd_checked(this, num(parts[0])?, num(parts[1])?, date1904)
            }
        }
        _ => None,
    }
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
        assert_eq!(
            serial_from_ymd(2026, 13, 1, false),
            serial_from_ymd(2027, 1, 1, false)
        );
        assert_eq!(ymd_from_serial(60.0, false), Some((1900, 2, 29)));
        assert_eq!(ymd_from_serial(61.0, false), Some((1900, 3, 1)));
        assert_eq!(ymd_from_serial(46314.75, false), Some((2026, 10, 19)));
        assert_eq!(ymd_from_serial(0.0, false), Some((1900, 1, 0)));
        assert_eq!(days_in_month(1900, 2), 29);
    }

    #[test]
    fn serials_1904() {
        assert_eq!(serial_from_ymd(1904, 1, 1, true), Some(0.0));
        assert_eq!(serial_from_ymd(2026, 10, 19, true), Some(46314.0 - 1462.0));
        assert_eq!(serial_from_ymd(1900, 1, 1, true), Some(-1461.0));
        assert_eq!(serial_from_ymd(1903, 12, 31, true), Some(-1.0));
        assert_eq!(ymd_from_serial(0.0, true), Some((1904, 1, 1)));
    }

    #[test]
    fn text_dates() {
        set_this_year(Some(2026));
        let p = |s| classify_text(s, false).map(|x| x.value());
        assert_eq!(p("2024-01-15"), Some(45306.0));
        assert_eq!(p("2024/01/15"), Some(45306.0));
        assert_eq!(p("2024년 1월 15일"), Some(45306.0));
        assert_eq!(p("24-1-5"), Some(45296.0));
        assert_eq!(p("1-5-24"), Some(37035.0));
        assert_eq!(p("05-01-2024"), Some(45413.0));
        assert_eq!(p("05/01/2024"), None);
        assert_eq!(p("15-Jan-2024"), Some(45306.0));
        assert_eq!(p("Jan 1, 2024"), Some(45292.0));
        assert_eq!(p("Jan-24"), Some(46046.0));
        assert_eq!(p("Jan-2024"), Some(45292.0));
        assert_eq!(p("1/15"), Some(46037.0));
        assert_eq!(p("12:00"), Some(0.5));
        assert_eq!(p("25:00"), Some(25.0 / 24.0));
        assert_eq!(p("1:90"), Some(2.5 / 24.0));
        assert_eq!(p("10:30 오후"), Some(22.5 / 24.0));
        assert_eq!(p("0:00 AM"), None);
        assert_eq!(p("2024-01-15 18:00"), Some(46314.0 - 1008.0 + 0.75));
        assert_eq!(p("2024-01-15 25:00"), None);
        assert_eq!(p("6:00 PM"), Some(0.75));
        assert_eq!(p("2024-02-30"), None);
        assert_eq!(p("2024.1.15"), None);
        assert_eq!(p("  2024-01-15  "), None);
        assert_eq!(p("January 1"), None);
        set_this_year(None);
    }

    #[test]
    fn weekdays() {
        // 2026-10-19 은 월요일
        assert_eq!(weekday(46314.0, false), 1);
        assert_eq!(weekday(1.0, false), 0);
    }

    #[test]
    fn time_rounding() {
        // 59.5초 → 0:01:00
        assert_eq!(hms_from_serial(1.0 / 24.0 / 60.0 / 60.0 * 59.5), (0, 1, 0));
        assert_eq!(hms_from_serial(0.000_005_787), (0, 0, 1));
    }
}
