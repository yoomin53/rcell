// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 칸 너비에 맞춘 숫자 표시 (한셀 화면 대조 실험).
//!
//! - 일반 서식: 부호를 뺀 11글자 안에서 보인다. 정수부가 12자리 이상이거나, 1E-4 보다 작은데 고정
//!   표기가 9글자를 넘으면 지수 표기. 지수는 0 을 채우지 않는다 (`1E+20`, `1.234E-6`) — `Range.Text` 대조.
//! - 좁은 칸: 고정 표기면 소수 자리를 줄이고(반올림), 정수부도 안 들어가면 지수 표기의 가수 자리를
//!   줄인다. 지수 표기가 기본인 수는 지수 → 고정(0 으로 반올림) 순. 그래도 안 들어가면 `#` 로 채운다.
//! - 표시 형식이 있는 숫자가 칸보다 넓으면 `#` 로 채운다.

use super::metrics::FontMetrics;

/// 15 유효 숫자로 정리한 뒤 소수 `d` 자리로 반올림 (0.5 는 0 에서 먼 쪽)
fn round_dec(a: f64, d: i32) -> f64 {
    let a: f64 = format!("{a:.14e}").parse().unwrap_or(a);
    let p = 10f64.powi(d);
    let r = (a * p).round() / p;
    if r.is_finite() { r } else { a }
}

/// 고정 표기 (소수 `d` 자리, 뒤 0 과 점은 지운다)
fn fixed(a: f64, d: usize) -> String {
    let r = round_dec(a, d as i32);
    let mut s = format!("{r:.d$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// 지수 표기 (가수 소수 `m` 자리, 뒤 0 은 지우고 지수는 0 을 채우지 않는다)
fn sci(a: f64, m: usize) -> String {
    let s = format!("{a:.m$E}");
    let (mant, exp) = s.split_once('E').unwrap_or((&s, "0"));
    let mut mant = mant.to_string();
    if mant.contains('.') {
        while mant.ends_with('0') {
            mant.pop();
        }
        if mant.ends_with('.') {
            mant.pop();
        }
    }
    let e: i32 = exp.parse().unwrap_or(0);
    format!("{mant}E{}{}", if e < 0 { '-' } else { '+' }, e.abs())
}

/// 정수부 자릿수 (반올림 뒤)
fn int_digits(a: f64) -> usize {
    if a < 1.0 { 1 } else { format!("{:.0}", a.trunc()).len() }
}

/// 고정 표기의 최대 소수 자리 (11글자 안)
fn max_fixed_decimals(a: f64) -> usize {
    if a < 1.0 { 9 } else { 10usize.saturating_sub(int_digits(a)) }
}

/// 지수 표기의 최대 가수 소수 자리 (11글자 안)
fn max_sci_decimals(a: f64) -> usize {
    let e = a.log10().floor() as i32;
    let ed = e.unsigned_abs().to_string().len();
    // d.mmmmE+x: 1 + 1 + m + 2 + ed <= 11
    7usize.saturating_sub(ed)
}

/// 일반 서식이 지수 표기를 고르는가 (넓은 칸)
fn prefers_sci(a: f64) -> bool {
    if a == 0.0 {
        return false;
    }
    let f = fixed(a, max_fixed_decimals(a));
    if f.len() > 11 || f == "0" {
        return true;
    }
    a < 1e-4 && f.len() > 9
}

/// 일반 서식 표시 (넓은 칸) — 한셀 `Range.Text` 와 같다
pub fn general(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v == 0.0 {
        return "0".into();
    }
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let body = if prefers_sci(a) {
        sci(a, max_sci_decimals(a))
    } else {
        let f = fixed(a, max_fixed_decimals(a));
        // 반올림으로 12자리가 되면 (99999999999.5) 지수
        if f.len() > 11 && !f.contains('.') {
            sci(a, max_sci_decimals(a))
        } else {
            f
        }
    };
    format!("{sign}{body}")
}

/// 일반 서식 숫자를 `avail` px 에 맞춘다. 안 들어가면 `#` 채움
pub fn general_fit(v: f64, avail: f64, m: &FontMetrics) -> String {
    let wide = general(v);
    if m.text_width(&wide) <= avail || !v.is_finite() {
        return wide;
    }
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let fits = |s: &String| m.text_width(s) <= avail;
    let fixed_try = || -> Option<String> {
        (0..=max_fixed_decimals(a))
            .rev()
            .map(|d| format!("{sign}{}", fixed(a, d)))
            .find(|s| s.len() - sign.len() <= 11 && fits(s))
    };
    let sci_try = || -> Option<String> {
        (0..=max_sci_decimals(a))
            .rev()
            .map(|d| format!("{sign}{}", sci(a, d)))
            .find(fits)
    };
    let pick = if prefers_sci(a) {
        sci_try().or_else(|| if a < 1.0 { fixed_try() } else { None })
    } else if int_digits(round_dec(a, 0)) <= 11
        && m.text_width(&format!("{sign}{}", fixed(a, 0))) <= avail
    {
        fixed_try()
    } else {
        sci_try()
    };
    pick.unwrap_or_else(|| hashes(avail, m))
}

/// 칸 너비만큼 `#`
pub fn hashes(avail: f64, m: &FontMetrics) -> String {
    let w = m.char_width('#').max(1.0);
    "#".repeat((avail / w).floor().max(0.0) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_matches_hancell_range_text() {
        // 한셀 Range.Text (넓은 칸) 대조값
        for (v, want) in [
            (1234567890123.0, "1.23457E+12"),
            (123456789.0, "123456789"),
            (12345.6789, "12345.6789"),
            (0.123456789012, "0.123456789"),
            (-0.00012345, "-0.00012345"),
            (1e20, "1E+20"),
            (1.5e-10, "1.5E-10"),
            (3.14159265358979, "3.141592654"),
            (100000.0, "100000"),
            (0.000001234, "1.234E-6"),
            (1e-7, "0.0000001"),
            (1.2e-7, "1.2E-7"),
            (1.23e-5, "0.0000123"),
            (1.234e-5, "1.234E-5"),
            (0.000123456, "0.000123456"),
            (99999999999.0, "99999999999"),
            (1e11, "1E+11"),
            (99999999999.5, "1E+11"),
            (0.999999999999, "1"),
            (123.456789012345, "123.456789"),
            (1234567.89012345, "1234567.89"),
            (-123456789012.0, "-1.23457E+11"),
            (1.23456e-6, "1.23456E-6"),
            (1e100, "1E+100"),
            (0.5, "0.5"),
        ] {
            assert_eq!(general(v), want, "{v}");
        }
    }

    #[test]
    fn narrow_columns_shorten_like_hancell() {
        // 맑은 고딕 11 (K=8): 화면 대조 실험의 칸 너비별 표시
        let m = FontMetrics::new(Some("맑은 고딕"), Some(11.0), false);
        assert_eq!(general_fit(12345.6789, 36.0, &m), "1E+4");
        assert_eq!(general_fit(12345.6789, 40.0, &m), "12346");
        assert_eq!(general_fit(12345.6789, 52.0, &m), "12345.7");
        assert_eq!(general_fit(0.123456789012, 16.0, &m), "0");
        assert_eq!(general_fit(0.123456789012, 20.0, &m), "0.1");
        assert_eq!(general_fit(1234567890123.0, 44.0, &m), "1E+12");
        assert_eq!(general_fit(1234567890123.0, 56.0, &m), "1.2E+12");
        assert_eq!(general_fit(0.000001234, 28.0, &m), "0");
        assert_eq!(general_fit(0.000001234, 32.0, &m), "1E-6");
        assert_eq!(general_fit(-0.00012345, 16.0, &m), "-0");
        assert_eq!(general_fit(0.5, 8.0, &m), "1");
        assert_eq!(general_fit(1e100, 40.0, &m), "####");
        assert_eq!(general_fit(-7.0, 12.0, &m), "#");
    }
}
