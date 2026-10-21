// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 날짜·시간 함수. 한셀과 다른 점은 함수 대조 실험(date·date1904·conv 묶음)으로 맞췄다
//! (`docs/format/calc.md`).

use super::{Arg, FnSpec, V, f, flatten, given, num_arg, num_or, opt, volatile};
use crate::calc::date::{
    self as dt, Parsed, days_in_month, hms_from_serial, is_leap, serial_from_ymd, ymd_for_months,
    ymd_from_serial,
};
use crate::calc::eval::Ctx;
use crate::calc::value::{ErrorKind, Value, num};

const VVR: &[Arg] = &[Arg::V, Arg::V, Arg::R];
const VVVR: &[Arg] = &[Arg::V, Arg::V, Arg::V, Arg::R];

pub const FNS: &[FnSpec] = &[
    f("DATE", 3, 3, V, date),
    f("TIME", 3, 3, V, time),
    f("DATEVALUE", 1, 1, V, |c, a| match &a[0] {
        Value::Text(t) => match dt::classify_text(t, c.date1904()) {
            Some(Parsed::Date(v)) | Some(Parsed::DateTime(v, _)) => num(v.floor()),
            Some(Parsed::Time(_)) => Value::Number(0.0),
            _ => Value::Error(ErrorKind::Value),
        },
        Value::Error(e) => Value::Error(*e),
        _ => Value::Error(ErrorKind::Value),
    }),
    f("TIMEVALUE", 1, 1, V, |c, a| match &a[0] {
        // 시각만 쓴 글은 하루를 넘어도 그대로 (TIMEVALUE("25:00") = 1.0417, 한셀)
        Value::Text(t) => match dt::classify_text(t, c.date1904()) {
            Some(Parsed::Time(v)) => num(v),
            Some(Parsed::DateTime(_, t)) => num(t),
            Some(Parsed::Date(_)) => Value::Number(0.0),
            _ => Value::Error(ErrorKind::Value),
        },
        Value::Error(e) => Value::Error(*e),
        _ => Value::Error(ErrorKind::Value),
    }),
    f("YEAR", 1, 1, V, |c, a| ymd_part(c, a, 0)),
    f("MONTH", 1, 1, V, |c, a| ymd_part(c, a, 1)),
    f("DAY", 1, 1, V, |c, a| ymd_part(c, a, 2)),
    f("HOUR", 1, 1, V, |c, a| hms_part(c, a, 0)),
    f("MINUTE", 1, 1, V, |c, a| hms_part(c, a, 1)),
    f("SECOND", 1, 1, V, |c, a| hms_part(c, a, 2)),
    f("WEEKDAY", 1, 2, V, weekday),
    f("WEEKNUM", 1, 2, V, weeknum),
    f("ISOWEEKNUM", 1, 1, V, |c, a| {
        let s = try_v!(serial_arg(c, a, 0));
        match iso_week(s, c.date1904()) {
            Some(w) => num(w as f64),
            None => Value::Error(ErrorKind::Num),
        }
    }),
    volatile(f("NOW", 0, 0, V, |c, _| num(c.now()))),
    volatile(f("TODAY", 0, 0, V, |c, _| num(c.now().floor()))),
    f("EDATE", 2, 2, V, |c, a| add_months(c, a, false)),
    f("EOMONTH", 2, 2, V, |c, a| add_months(c, a, true)),
    // DAYS 는 음수 일련 번호도 받는다 (DAYS(-1,0) = −1, 한셀)
    f("DAYS", 2, 2, V, |c, a| {
        let e = try_v!(num_arg(c, a, 0)).floor();
        let s = try_v!(num_arg(c, a, 1)).floor();
        num(e - s)
    }),
    f("DAYS360", 2, 3, V, days360),
    f("DATEDIF", 3, 3, V, datedif),
    f("YEARFRAC", 2, 3, V, |c, a| {
        let s = try_v!(serial_arg(c, a, 0)).floor();
        let e = try_v!(serial_arg(c, a, 1)).floor();
        let b = try_v!(num_or(c, a, 2, 0.0)).trunc();
        if !(0.0..=4.0).contains(&b) {
            return Value::Error(ErrorKind::Num);
        }
        value_r(year_frac(s, e, b as u8, c.date1904()))
    }),
    f("NETWORKDAYS", 2, 3, VVR, |c, a| networkdays(c, a, false)),
    f("NETWORKDAYS.INTL", 2, 4, VVVR, |c, a| networkdays(c, a, true)),
    f("WORKDAY", 2, 3, VVR, |c, a| workday(c, a, false)),
    f("WORKDAY.INTL", 2, 4, VVVR, |c, a| workday(c, a, true)),
];

fn value_r(r: Result<f64, ErrorKind>) -> Value {
    crate::calc::value::num_r(r)
}

/// 날짜 인수 (음수·마지막 날 뒤는 #NUM!)
pub fn serial_arg(ctx: &mut Ctx, a: &[Value], i: usize) -> Result<f64, ErrorKind> {
    let v = num_arg(ctx, a, i)?;
    if v < 0.0 || v >= dt::max_serial(ctx.date1904()) + 1.0 {
        return Err(ErrorKind::Num);
    }
    Ok(v)
}

fn date(ctx: &mut Ctx, a: &[Value]) -> Value {
    let mut y = try_v!(num_arg(ctx, a, 0)).trunc();
    let m = try_v!(num_arg(ctx, a, 1)).trunc();
    let d = try_v!(num_arg(ctx, a, 2)).trunc();
    if !(0.0..10000.0).contains(&y) {
        return Value::Error(ErrorKind::Num);
    }
    // 1900 보다 작은 해는 1900 을 더한다 (1904 체계도 1900 기준 — 한셀: DATE(0,1,1) = −1461)
    if y < 1900.0 {
        y += 1900.0;
    }
    // 한셀은 일 인수가 크면(DATE(2024,1,1000000)) #NUM! — 32767 을 넘는 일은 받지 않는다고 본다
    if m.abs() > 1e9 || d.abs() > 32_767.0 {
        return Value::Error(ErrorKind::Num);
    }
    match serial_from_ymd(y as i64, m as i64, d as i64, ctx.date1904()) {
        Some(s) => num(s),
        None => Value::Error(ErrorKind::Num),
    }
}

fn time(ctx: &mut Ctx, a: &[Value]) -> Value {
    let h = try_v!(num_arg(ctx, a, 0)).trunc();
    let m = try_v!(num_arg(ctx, a, 1)).trunc();
    let s = try_v!(num_arg(ctx, a, 2)).trunc();
    if h > 32767.0 || m > 32767.0 || s > 32767.0 {
        return Value::Error(ErrorKind::Num);
    }
    let total = h * 3600.0 + m * 60.0 + s;
    if total < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num((total % 86_400.0) / 86_400.0)
}

fn ymd_part(ctx: &mut Ctx, a: &[Value], which: u8) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0));
    match ymd_from_serial(s, ctx.date1904()) {
        Some((y, m, d)) => num(match which {
            0 => y as f64,
            1 => m as f64,
            _ => d as f64,
        }),
        None => Value::Error(ErrorKind::Num),
    }
}

fn hms_part(ctx: &mut Ctx, a: &[Value], which: u8) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0));
    let (h, m, sec) = hms_from_serial(s);
    num(match which {
        0 => h,
        1 => m,
        _ => sec,
    } as f64)
}

fn weekday(ctx: &mut Ctx, a: &[Value]) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0));
    let t = try_v!(num_or(ctx, a, 1, 1.0)).trunc() as i64;
    let w = dt::weekday(s, ctx.date1904()) as i64; // 0 = 일요일
    let r = match t {
        1 => w + 1,
        2 => (w + 6) % 7 + 1,
        3 => (w + 6) % 7,
        11..=17 => {
            // 11 월요일 시작 … 17 일요일 시작
            let start = (t - 10) % 7; // 월=1 … 일=0
            (w - start).rem_euclid(7) + 1
        }
        _ => return Value::Error(ErrorKind::Num),
    };
    num(r as f64)
}

/// 한 해의 몇째 날 (1월 1일 = 1, 한셀 달력)
fn day_of_year(serial: f64, date1904: bool) -> Option<f64> {
    let (y, _, _) = ymd_from_serial(serial, date1904)?;
    let jan1 = serial_from_ymd(y, 1, 1, date1904)?;
    Some(serial.floor() - jan1 + 1.0)
}

/// ISO 주 번호 (한셀): ⌊(연중 날 − ISO 요일 + 10) / 7⌋. 0 이면 지난해 12월 31일의 번호. 연말에 다음 해
/// 1주로 넘기지 않는다 (2024-12-31 → 53, 표준은 1). 1900-01-00 과 그 이전은 #NUM!
/// (한셀은 ISOWEEKNUM(1) 에서 죽는다).
fn iso_week(serial: f64, date1904: bool) -> Option<u32> {
    let n = serial.floor();
    let (y, m, d) = ymd_from_serial(n, date1904)?;
    if d == 0 {
        return None;
    }
    let w = dt::weekday(n, date1904) as i64; // 0 일
    let iso_dow = if w == 0 { 7 } else { w };
    let doy = day_of_year(n, date1904)? as i64;
    let week = (doy - iso_dow + 10) / 7;
    if week >= 1 {
        return Some(week as u32);
    }
    let _ = m;
    // 기준일 앞으로는 가지 않는다 (1904 체계 ISOWEEKNUM(0) = #NUM!)
    let dec31 = serial_from_ymd(y - 1, 12, 31, date1904).filter(|&v| v >= 0.0)?;
    iso_week(dec31, date1904)
}

fn weeknum(ctx: &mut Ctx, a: &[Value]) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0)).floor();
    let t = try_v!(num_or(ctx, a, 1, 1.0)).trunc() as i64;
    let start = match t {
        1 | 17 => 0, // 일요일
        2 | 11 => 1, // 월요일
        12 => 2,
        13 => 3,
        14 => 4,
        15 => 5,
        16 => 6,
        21 => {
            return match iso_week(s, ctx.date1904()) {
                Some(w) => num(w as f64),
                None => Value::Error(ErrorKind::Num),
            };
        }
        _ => return Value::Error(ErrorKind::Num),
    };
    let d1904 = ctx.date1904();
    // 0 (1900-01-00) 은 1900년 1주
    let s = s.max(dt::min_serial(d1904) + 1.0);
    let Some((y, _, _)) = ymd_from_serial(s, d1904) else {
        return Value::Error(ErrorKind::Num);
    };
    let jan1 = serial_from_ymd(y, 1, 1, d1904).unwrap_or(1.0);
    let wd = dt::weekday(jan1, d1904) as i64;
    let offset = (wd - start).rem_euclid(7);
    num(((s - jan1 + offset as f64) / 7.0).floor() + 1.0)
}

fn add_months(ctx: &mut Ctx, a: &[Value], eom: bool) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0)).floor();
    let m = try_v!(num_arg(ctx, a, 1)).trunc();
    let d1904 = ctx.date1904();
    let Some((y, mo, d)) = ymd_for_months(s, d1904) else {
        return Value::Error(ErrorKind::Num);
    };
    let total = y * 12 + (mo as i64 - 1) + m as i64;
    let ny = total.div_euclid(12);
    let nm = (total.rem_euclid(12) + 1) as u32;
    let dim = days_in_month(ny, nm);
    let nd = if eom { dim } else { d.min(dim).max(1) };
    match serial_from_ymd(ny, nm as i64, nd as i64, d1904) {
        Some(v) if v >= 0.0 => num(v),
        _ => Value::Error(ErrorKind::Num),
    }
}

fn is_last_of_feb(y: i64, m: u32, d: u32) -> bool {
    m == 2 && d == days_in_month(y, 2)
}

/// 30/360 일수. `us` 는 NASD 방식, 아니면 유럽 방식
pub fn days_30_360(s: f64, e: f64, us: bool, date1904: bool) -> Option<f64> {
    let (y1, m1, mut d1) = ymd_from_serial(s, date1904)?;
    let (y2, m2, mut d2) = ymd_from_serial(e, date1904)?;
    if us {
        if is_last_of_feb(y1, m1, d1) {
            if is_last_of_feb(y2, m2, d2) {
                d2 = 30;
            }
            d1 = 30;
        }
        if d2 == 31 && d1 >= 30 {
            d2 = 30;
        }
        if d1 == 31 {
            d1 = 30;
        }
    } else {
        if d1 == 31 {
            d1 = 30;
        }
        if d2 == 31 {
            d2 = 30;
        }
    }
    Some(((y2 - y1) * 360 + (m2 as i64 - m1 as i64) * 30 + (d2 as i64 - d1 as i64)) as f64)
}

fn days360(ctx: &mut Ctx, a: &[Value]) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0)).floor();
    let e = try_v!(serial_arg(ctx, a, 1)).floor();
    let euro = match opt(a, 2) {
        Some(v) => try_v!(ctx.boolean(v)),
        None => false,
    };
    let d1904 = ctx.date1904();
    let (Some((y1, m1, mut d1)), Some((y2, m2, mut d2))) =
        (ymd_from_serial(s, d1904), ymd_from_serial(e, d1904))
    else {
        return Value::Error(ErrorKind::Num);
    };
    if euro {
        d1 = d1.min(30);
        d2 = d2.min(30);
    } else {
        // 시작일이 그 달 마지막 날이면 30일. 끝 날이 31일이면 시작일이 30일 전일 때 다음 달 1일, 아니면 30일.
        // 한셀은 두 날이 모두 2월 말일이어도 끝 날을 30일로 바꾸지 않는다 (DAYS360(2023-02-28,2024-02-29) = 359).
        if d1 == days_in_month(y1, m1) {
            d1 = 30;
        }
        if d2 == 31 {
            if d1 < 30 {
                return num(
                    ((y2 - y1) * 360 + (m2 as i64 + 1 - m1 as i64) * 30 + (1 - d1 as i64)) as f64,
                );
            }
            d2 = 30;
        }
    }
    num(((y2 - y1) * 360 + (m2 as i64 - m1 as i64) * 30 + (d2 as i64 - d1 as i64)) as f64)
}

fn datedif(ctx: &mut Ctx, a: &[Value]) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0)).floor();
    let e = try_v!(serial_arg(ctx, a, 1)).floor();
    let unit = try_v!(ctx.text(&a[2])).to_ascii_uppercase();
    if s > e {
        return Value::Error(ErrorKind::Num);
    }
    let d1904 = ctx.date1904();
    let (Some((y1, m1, d1)), Some((y2, m2, d2))) = (ymd_from_serial(s, d1904), ymd_from_serial(e, d1904))
    else {
        return Value::Error(ErrorKind::Num);
    };
    let months = (y2 - y1) * 12 + m2 as i64 - m1 as i64 - if d2 < d1 { 1 } else { 0 };
    match unit.as_str() {
        "Y" => num((months / 12) as f64),
        "M" => num(months as f64),
        "D" => num(e - s),
        "YM" => num((months % 12) as f64),
        "MD" => {
            if d2 >= d1 {
                num((d2 - d1) as f64)
            } else {
                // 앞 달의 같은 날부터
                let (py, pm) = if m2 == 1 { (y2 - 1, 12) } else { (y2, m2 - 1) };
                let dim = days_in_month(py, pm) as i64;
                num((dim - d1 as i64).max(0) as f64 + d2 as f64)
            }
        }
        "YD" => {
            // 끝 날의 월·일을 시작 해에 놓는다 (넘치는 날은 DATE 처럼 다음 달로). 시작보다 앞이면 다음 해.
            // 한셀: DATEDIF(2020-01-01, 2023-03-15, "YD") = 74 (2020 년은 윤년)
            let here = serial_from_ymd(y1, m2 as i64, d2 as i64, d1904).unwrap_or(s);
            if here >= s {
                num(here - s)
            } else {
                let next = serial_from_ymd(y1 + 1, m2 as i64, d2 as i64, d1904).unwrap_or(e);
                num(next - s)
            }
        }
        _ => Value::Error(ErrorKind::Num),
    }
}

/// 연 비율 (YEARFRAC, 금융 함수의 기준 0–4)
pub fn year_frac(s: f64, e: f64, basis: u8, date1904: bool) -> Result<f64, ErrorKind> {
    let (s, e) = if s > e { (e, s) } else { (s, e) };
    if s == e {
        return Ok(0.0);
    }
    match basis {
        0 => Ok(days_30_360(s, e, true, date1904).ok_or(ErrorKind::Num)? / 360.0),
        2 => Ok((e - s) / 360.0),
        3 => Ok((e - s) / 365.0),
        4 => Ok(days_30_360(s, e, false, date1904).ok_or(ErrorKind::Num)? / 360.0),
        _ => {
            let (y1, m1, d1) = ymd_from_serial(s, date1904).ok_or(ErrorKind::Num)?;
            let (y2, m2, d2) = ymd_from_serial(e, date1904).ok_or(ErrorKind::Num)?;
            let days = e - s;
            let within_year = y1 == y2 || (y2 == y1 + 1 && (m1 > m2 || (m1 == m2 && d1 >= d2)));
            if within_year {
                let denom = if y1 == y2 {
                    if is_leap(y1) { 366.0 } else { 365.0 }
                } else {
                    let feb29_y1 = is_leap(y1) && (m1 < 3);
                    let feb29_y2 = is_leap(y2) && (m2 > 2 || (m2 == 2 && d2 == 29));
                    if feb29_y1 || feb29_y2 { 366.0 } else { 365.0 }
                };
                Ok(days / denom)
            } else {
                let start = serial_from_ymd(y1, 1, 1, date1904).ok_or(ErrorKind::Num)?;
                let end = serial_from_ymd(y2 + 1, 1, 1, date1904).ok_or(ErrorKind::Num)?;
                let avg = (end - start) / (y2 - y1 + 1) as f64;
                Ok(days / avg)
            }
        }
    }
}

/// 주말 지정 (번호 1–7, 11–17 또는 "0000011" 글) → 요일(0 일요일)별 쉬는 날. 한셀은 모든 날이 주말인
/// "1111111" 도 받는다 (WORKDAY.INTL 은 그때 끝나지 않는다). 없는 번호는 `bad` 오류.
fn weekend_mask(ctx: &mut Ctx, v: Option<&Value>, bad: ErrorKind) -> Result<[bool; 7], ErrorKind> {
    let mut m = [false; 7];
    match v {
        None => {
            m[0] = true;
            m[6] = true;
        }
        Some(Value::Text(t)) => {
            let b: Vec<char> = t.chars().collect();
            if b.len() != 7 || b.iter().any(|c| *c != '0' && *c != '1') {
                return Err(ErrorKind::Value);
            }
            // 글은 월요일부터
            for (i, c) in b.iter().enumerate() {
                m[(i + 1) % 7] = *c == '1';
            }
        }
        Some(v) => {
            let n = ctx.num(v)?.trunc() as i64;
            match n {
                1..=7 => {
                    // 1 토·일, 2 일·월, … 7 금·토
                    let a = (n + 5) % 7; // 1 → 6(토)
                    m[a as usize] = true;
                    m[((a + 1) % 7) as usize] = true;
                }
                // 11 일요일만, 12 월요일만 … 17 토요일만
                11..=17 => m[(n - 11) as usize] = true,
                _ => return Err(bad),
            }
        }
    }
    Ok(m)
}

fn holidays(ctx: &mut Ctx, v: Option<&Value>) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    if let Some(v) = v {
        for x in flatten(ctx, v) {
            match x {
                Value::Number(n) => out.push(n.floor()),
                Value::Error(e) => return Err(e),
                Value::Empty => {}
                other => out.push(ctx.num(&other)?.floor()),
            }
        }
    }
    Ok(out)
}

fn networkdays(ctx: &mut Ctx, a: &[Value], intl: bool) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0)).floor();
    let e = try_v!(serial_arg(ctx, a, 1)).floor();
    let (wk, hol_i) = if intl { (given(a, 2), 3) } else { (None, 2) };
    // 한셀의 NETWORKDAYS.INTL·WORKDAY.INTL 은 휴일 인수가 있으면 #NUM! (함수 대조 실험)
    if intl && given(a, hol_i).is_some() {
        return Value::Error(ErrorKind::Num);
    }
    let mask = try_v!(weekend_mask(ctx, wk, ErrorKind::Value));
    let hol = try_v!(holidays(ctx, given(a, hol_i)));
    let (lo, hi, sign) = if s <= e { (s, e, 1.0) } else { (e, s, -1.0) };
    let d1904 = ctx.date1904();
    let mut n = 0.0;
    let mut d = lo;
    while d <= hi {
        let w = dt::weekday(d, d1904) as usize;
        if !mask[w] && !hol.contains(&d) {
            n += 1.0;
        }
        d += 1.0;
    }
    num(n * sign)
}

fn workday(ctx: &mut Ctx, a: &[Value], intl: bool) -> Value {
    let s = try_v!(serial_arg(ctx, a, 0)).floor();
    let days = try_v!(num_arg(ctx, a, 1)).trunc();
    let (wk, hol_i) = if intl { (given(a, 2), 3) } else { (None, 2) };
    if intl && given(a, hol_i).is_some() {
        return Value::Error(ErrorKind::Num);
    }
    let mask = try_v!(weekend_mask(ctx, wk, ErrorKind::Num));
    let hol = try_v!(holidays(ctx, given(a, hol_i)));
    let d1904 = ctx.date1904();
    if days != 0.0 && mask.iter().all(|&w| w) {
        // 한셀은 여기서 끝나지 않는다 — 대신 #NUM!
        return Value::Error(ErrorKind::Num);
    }
    let step = if days >= 0.0 { 1.0 } else { -1.0 };
    let mut left = days.abs();
    let mut d = s;
    while left > 0.0 {
        d += step;
        if d < 0.0 || d > dt::max_serial(d1904) {
            return Value::Error(ErrorKind::Num);
        }
        let w = dt::weekday(d, d1904) as usize;
        if !mask[w] && !hol.contains(&d) {
            left -= 1.0;
        }
    }
    num(d)
}
