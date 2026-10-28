// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 자동 채우기(채우기 핸들)와 연속 데이터 채우기 (채우기 대조 실험 — docs/editing.md 13절).
//!
//! 자동 채우기는 씨앗 칸들을 채울 방향으로 되풀이해 붙이고(서식·수식·조건부 서식은 복사 규칙), 값은 씨앗을 같은
//! 종류끼리 이어진 묶음으로 나눠 묶음마다 이어 간다:
//! - 숫자: 하나뿐이면 복사(씨앗 전체가 한 칸일 때 — 여러 칸 씨앗 안의 숫자 하나는 1씩), 둘 이상이면 최소 제곱 직선.
//!   날짜는 하나면 하루씩(시각만이면 한 시간씩), 둘 이상이면 같은 날짜의 달·해 간격 또는 같은 날 수 간격 (간격이
//!   고르지 않으면 복사).
//! - 끝에 숫자가 있는 글(`항목1`, `x10y`, `007`, `1st`): 마지막 숫자를 1씩(둘 이상이면 그 간격) 늘린다. 0 앞채움
//!   자릿수와 영어 서수 꼬리(`st` `nd` `rd` `th`)를 맞추고, 0 아래로 가면 절댓값.
//! - 한셀 기본 목록 18가지(요일·월·분기·간지 …)의 항목: 목록을 돈다 (영어는 대소문자 꼴을 따른다).
//! - 그 밖의 글·논리값·오류: 복사.
//! 위·왼쪽으로 채우면 거꾸로 이어 간다 (한셀: 날짜·시각 하나는 멀어질수록 늘고, 둘 이상인 글·목록 묶음은 마지막
//! 항목에서 앞으로 이어 간 값을 쓴다).

use super::clip;
use super::err;
use crate::model::{CellRange, CellRef, CellValue, Workbook};

/// 한셀 기본 목록 (COM `Application.GetCustomListContents` 1–18)
pub const LISTS: [&[&str]; 18] = [
    &["일", "월", "화", "수", "목", "금", "토"],
    &[
        "일요일",
        "월요일",
        "화요일",
        "수요일",
        "목요일",
        "금요일",
        "토요일",
    ],
    &[
        "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
    ],
    &[
        "일월",
        "이월",
        "삼월",
        "사월",
        "오월",
        "유월",
        "칠월",
        "팔월",
        "구월",
        "시월",
        "십일월",
        "십이월",
    ],
    &["1사분기", "2사분기", "3사분기", "4사분기"],
    &["일사분기", "이사분기", "삼사분기", "사사분기"],
    &["1/4 분기", "2/4 분기", "3/4 분기", "4/4 분기"],
    &["1Q", "2Q", "3Q", "4Q"],
    &[
        "First Quarter",
        "Second Quarter",
        "Third Quarter",
        "Fourth Quarter",
    ],
    &["日", "月", "火", "水", "木", "金", "土"],
    &[
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    &["1四分期", "2四分期", "3四分期", "4四分期"],
    &[
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    &[
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    &[
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    &[
        "자", "축", "인", "묘", "진", "사", "오", "미", "신", "유", "술", "해",
    ],
    &["갑", "을", "병", "정", "무", "기", "경", "신", "임", "계"],
];

/// 자동 채우기 종류 (COM `XlAutoFillType`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FillType {
    /// 채우기 핸들 기본
    #[default]
    Default,
    /// 셀 복사
    Copy,
    /// 연속 데이터 (숫자 하나도 1씩)
    Series,
    /// 서식만
    Formats,
    /// 서식 없이 값만
    Values,
    Days,
    Weekdays,
    Months,
    Years,
}

/// 영어 목록의 대소문자 꼴
fn case_like(item: &str, seed: &str) -> String {
    let letters: Vec<char> = seed.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() || !seed.is_ascii() {
        return item.to_string();
    }
    if letters.iter().all(|c| c.is_lowercase()) {
        item.to_lowercase()
    } else if letters.iter().all(|c| c.is_uppercase()) && letters.len() > 1 {
        item.to_uppercase()
    } else {
        // 첫 글자만 대문자 (한셀: First Quarter → Second quarter)
        let low = item.to_lowercase();
        let mut cs = low.chars();
        match cs.next() {
            Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
            None => low,
        }
    }
}

/// 목록 항목: (목록 번호, 항목 번호)
fn list_item(s: &str) -> Option<(usize, usize)> {
    LISTS.iter().enumerate().find_map(|(li, l)| {
        l.iter()
            .position(|x| x.eq_ignore_ascii_case(s))
            .map(|i| (li, i))
    })
}

/// 끝에 숫자가 있는 글: (앞, 숫자, 자릿수(0 앞채움이면), 뒤)
#[derive(Debug, Clone, PartialEq)]
struct TextNum {
    head: String,
    n: i64,
    width: usize,
    tail: String,
}

fn text_num(s: &str) -> Option<TextNum> {
    let ch: Vec<char> = s.chars().collect();
    let end = ch.iter().rposition(|c| c.is_ascii_digit())? + 1;
    let mut start = end;
    while start > 0 && ch[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let digits: String = ch[start..end].iter().collect();
    if digits.len() > 15 {
        return None;
    }
    let n: i64 = digits.parse().ok()?;
    let width = if digits.len() > 1 && digits.starts_with('0') {
        digits.len()
    } else {
        0
    };
    Some(TextNum {
        head: ch[..start].iter().collect(),
        n,
        width,
        tail: ch[end..].iter().collect(),
    })
}

fn ordinal(n: i64) -> &'static str {
    let n = n.abs();
    if (11..=13).contains(&(n % 100)) {
        return "th";
    }
    match n % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

fn text_num_at(t: &TextNum, n: i64) -> String {
    let n = n.abs();
    let num = if t.width > 0 {
        format!("{n:0w$}", w = t.width)
    } else {
        n.to_string()
    };
    let tail = match t.tail.to_ascii_lowercase().as_str() {
        "st" | "nd" | "rd" | "th" => {
            let o = ordinal(n);
            if t.tail.chars().all(|c| c.is_uppercase()) {
                o.to_uppercase()
            } else {
                o.to_string()
            }
        }
        _ => t.tail.clone(),
    };
    format!("{}{num}{tail}", t.head)
}

/// 씨앗 칸 하나의 종류
#[derive(Debug, Clone)]
enum Seed {
    /// 숫자 (날짜 서식이면 Some(시각만인가))
    Num(f64, Option<bool>),
    Text(TextNum),
    List(usize, usize, String),
    /// 복사만 (글·논리값·오류·빈 칸·수식)
    Other,
}

fn same_run(a: &Seed, b: &Seed) -> bool {
    match (a, b) {
        (Seed::Num(_, x), Seed::Num(_, y)) => x.is_some() == y.is_some(),
        (Seed::Text(x), Seed::Text(y)) => x.head == y.head && x.tail == y.tail,
        (Seed::List(x, _, _), Seed::List(y, _, _)) => x == y,
        _ => false,
    }
}

/// 날짜에 달 수를 더한다 (끝날은 그 달의 마지막 날로)
fn add_months(serial: f64, months: i64, d1904: bool) -> Option<f64> {
    use crate::calc::date::{days_in_month, serial_from_ymd, ymd_from_serial};
    let (y, m, d) = ymd_from_serial(serial.floor(), d1904)?;
    let total = y * 12 + (m as i64 - 1) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let nd = d.min(days_in_month(ny, nm));
    Some(serial_from_ymd(ny, nm as i64, nd as i64, d1904)? + serial.fract())
}

/// 한 줄(채울 방향의 열 또는 행)의 씨앗 묶음으로 자리 `p`(씨앗 첫 칸에서 센 자리, 음수면 위·왼쪽)의 값.
/// None 이면 복사 (붙인 그대로 둔다)
fn series_value(seeds: &[Seed], p: i64, kind: FillType, d1904: bool) -> Option<CellValue> {
    let k = seeds.len() as i64;
    let j = p.rem_euclid(k) as usize;
    let c = p.div_euclid(k);
    // j 가 속한 묶음
    let mut s = j;
    while s > 0 && same_run(&seeds[s - 1], &seeds[j]) {
        s -= 1;
    }
    let mut e = j;
    while e + 1 < seeds.len() && same_run(&seeds[e + 1], &seeds[j]) {
        e += 1;
    }
    let run = &seeds[s..=e];
    let m = run.len() as i64;
    let x = c * m + (j - s) as i64;
    match &seeds[j] {
        Seed::Other => None,
        Seed::Num(_, date) => {
            let vals: Vec<f64> = run
                .iter()
                .map(|r| if let Seed::Num(v, _) = r { *v } else { 0.0 })
                .collect();
            match date {
                Some(time_only) => {
                    let v0 = vals[0];
                    let unit = match kind {
                        FillType::Days | FillType::Weekdays => Some(1.0),
                        _ => None,
                    };
                    if m == 1 {
                        // 날짜·시각 하나를 위·왼쪽으로 채우면 멀어질수록 는다 (한셀)
                        let x = x.abs();
                        return match kind {
                            FillType::Months => add_months(v0, x, d1904).map(CellValue::Number),
                            FillType::Years => add_months(v0, 12 * x, d1904).map(CellValue::Number),
                            FillType::Weekdays => {
                                Some(CellValue::Number(add_weekdays(v0, x, d1904)))
                            }
                            _ => {
                                let step =
                                    unit.unwrap_or(if *time_only { 1.0 / 24.0 } else { 1.0 });
                                Some(CellValue::Number(round_serial(v0 + step * x as f64)))
                            }
                        };
                    }
                    // 같은 날짜면 달·해 간격, 아니면 같은 날 수 간격 (고르지 않으면 복사)
                    use crate::calc::date::ymd_from_serial;
                    let ymd: Vec<(i64, u32, u32)> = vals
                        .iter()
                        .filter_map(|v| ymd_from_serial(v.floor(), d1904))
                        .collect();
                    if ymd.len() == vals.len()
                        && !*time_only
                        && ymd.iter().all(|t| t.2 == ymd[0].2)
                        && vals.iter().all(|v| v.fract() == vals[0].fract())
                    {
                        let mon: Vec<i64> = ymd.iter().map(|t| t.0 * 12 + t.1 as i64).collect();
                        let step = mon[1] - mon[0];
                        if step != 0 && mon.windows(2).all(|w| w[1] - w[0] == step) {
                            return add_months(vals[0], step * x, d1904).map(CellValue::Number);
                        }
                    }
                    let step = vals[1] - vals[0];
                    if vals.windows(2).all(|w| ((w[1] - w[0]) - step).abs() < 1e-9) {
                        return Some(CellValue::Number(round_serial(vals[0] + step * x as f64)));
                    }
                    None
                }
                None => {
                    if m == 1 {
                        let single = seeds.len() == 1;
                        if single && matches!(kind, FillType::Default | FillType::Values) {
                            return None;
                        }
                        return Some(CellValue::Number(vals[0] + x as f64));
                    }
                    // 최소 제곱 직선
                    let n = m as f64;
                    let sx: f64 = (0..m).map(|i| i as f64).sum();
                    let sy: f64 = vals.iter().sum();
                    let sxx: f64 = (0..m).map(|i| (i * i) as f64).sum();
                    let sxy: f64 = vals.iter().enumerate().map(|(i, v)| i as f64 * v).sum();
                    let b = (n * sxy - sx * sy) / (n * sxx - sx * sx);
                    let a = (sy - b * sx) / n;
                    Some(CellValue::Number(crate::calc::value::round15(
                        a + b * x as f64,
                    )))
                }
            }
        }
        Seed::Text(t) => {
            let nums: Vec<i64> = run
                .iter()
                .map(|r| if let Seed::Text(t) = r { t.n } else { 0 })
                .collect();
            let step = if m == 1 { 1 } else { nums[1] - nums[0] };
            // 위·왼쪽으로 채울 때 둘 이상인 묶음은 마지막 항목에서 앞으로 이어 간 값 (한셀)
            let x = if m > 1 && p < 0 { m - x } else { x };
            Some(CellValue::Text(text_num_at(t, nums[0] + step * x)))
        }
        Seed::List(li, _, raw) => {
            let idx: Vec<i64> = run
                .iter()
                .map(|r| {
                    if let Seed::List(_, i, _) = r {
                        *i as i64
                    } else {
                        0
                    }
                })
                .collect();
            let step = if m == 1 { 1 } else { idx[1] - idx[0] };
            let x = if m > 1 && p < 0 { m - x } else { x };
            let list = LISTS[*li];
            let i = (idx[0] + step * x).rem_euclid(list.len() as i64) as usize;
            Some(CellValue::Text(case_like(list[i], raw)))
        }
    }
}

/// 날짜 계산의 부동소수 찌꺼기를 지운다
fn round_serial(v: f64) -> f64 {
    crate::calc::value::round15(v)
}

/// 평일만 세어 `n` 날 뒤 (토·일은 건너뛴다)
fn add_weekdays(v: f64, n: i64, d1904: bool) -> f64 {
    let mut d = v;
    let step = if n >= 0 { 1.0 } else { -1.0 };
    let mut left = n.abs();
    while left > 0 {
        d += step;
        let w = crate::calc::date::weekday(d.floor(), d1904);
        // weekday: 0 = 일요일 … 6 = 토요일
        if w != 0 && w != 6 {
            left -= 1;
        }
    }
    d
}

/// 씨앗 칸의 종류
fn classify(wb: &Workbook, si: usize, r: CellRef, kind: FillType) -> Seed {
    let sheet = &wb.sheets[si];
    let Some(c) = sheet.cell(r) else {
        return Seed::Other;
    };
    if c.formula.is_some() || kind == FillType::Copy || kind == FillType::Formats {
        return Seed::Other;
    }
    match &c.value {
        CellValue::Number(v) => {
            let code = wb
                .styles
                .xf(c.style)
                .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
                .unwrap_or_else(|| "General".into());
            let date = match super::input::category(&code) {
                super::input::Cat::Date => Some(false),
                super::input::Cat::Time => Some(*v < 1.0),
                _ => None,
            };
            Seed::Num(*v, date)
        }
        CellValue::Text(t) => {
            if let Some((li, i)) = list_item(t) {
                Seed::List(li, i, t.clone())
            } else if let Some(tn) = text_num(t) {
                Seed::Text(tn)
            } else {
                Seed::Other
            }
        }
        _ => Seed::Other,
    }
}

/// 채울 방향
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Down,
    Up,
    Right,
    Left,
}

/// 자동 채우기: `src` 씨앗으로 `dst`(씨앗을 품고 한 방향으로 넓힌 범위)를 채운다
pub fn auto_fill(
    wb: &mut Workbook,
    si: usize,
    src: CellRange,
    dst: CellRange,
    kind: FillType,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let same_cols = src.start.col == dst.start.col && src.end.col == dst.end.col;
    let same_rows = src.start.row == dst.start.row && src.end.row == dst.end.row;
    let dir = if same_cols && dst.start.row == src.start.row && dst.end.row > src.end.row {
        Dir::Down
    } else if same_cols && dst.end.row == src.end.row && dst.start.row < src.start.row {
        Dir::Up
    } else if same_rows && dst.start.col == src.start.col && dst.end.col > src.end.col {
        Dir::Right
    } else if same_rows && dst.end.col == src.end.col && dst.start.col < src.start.col {
        Dir::Left
    } else if src == dst {
        return Ok(());
    } else {
        return Err(err(
            "채울 범위는 씨앗 범위를 한 방향으로 넓힌 것이어야 합니다",
        ));
    };
    super::guard_cells(wb, si, dst)?;
    // 씨앗이나 채울 범위에 일부만 걸친 병합이 있으면 한셀은 아무것도 하지 않는다 (corpus 대조 — 오류도 없다)
    let partial = |r: CellRange| {
        wb.sheets[si].merges.iter().any(|m| {
            let over = !(m.end.row < r.start.row
                || m.start.row > r.end.row
                || m.end.col < r.start.col
                || m.start.col > r.end.col);
            over && !(r.contains(m.start) && r.contains(m.end))
        })
    };
    if partial(src) || partial(dst) {
        return Ok(());
    }
    let vertical = matches!(dir, Dir::Down | Dir::Up);
    let k = if vertical {
        src.end.row - src.start.row + 1
    } else {
        src.end.col - src.start.col + 1
    };
    // 1. 씨앗 값의 종류 (붙이기 전)
    let lines: Vec<u32> = if vertical {
        (src.start.col..=src.end.col).collect()
    } else {
        (src.start.row..=src.end.row).collect()
    };
    let at = |line: u32, i: u32| -> CellRef {
        if vertical {
            CellRef {
                row: src.start.row + i,
                col: line,
            }
        } else {
            CellRef {
                row: line,
                col: src.start.col + i,
            }
        }
    };
    let seeds: Vec<Vec<Seed>> = lines
        .iter()
        .map(|&l| (0..k).map(|i| classify(wb, si, at(l, i), kind)).collect())
        .collect();
    let old_values: Vec<(CellRef, CellValue, Option<crate::model::Formula>, u32)> =
        if kind == FillType::Formats || kind == FillType::Values {
            ext_cells(wb, si, src, dst)
        } else {
            Vec::new()
        };
    // 2. 씨앗을 되풀이해 붙인다 (서식·수식·유효성 — 채우기 규칙. 조건부 서식은 아래에서 따로)
    let ext = extension(src, dst, dir);
    let cfs_before = wb.sheets[si].conditional_formats.clone();
    let dvs_before = wb.sheets[si].data_validations.clone();
    let len = if vertical {
        ext.end.row - ext.start.row + 1
    } else {
        ext.end.col - ext.start.col + 1
    };
    let full = len / k;
    let rem = len % k;
    let seed_part = |from: u32, n: u32| -> CellRange {
        if vertical {
            CellRange {
                start: CellRef {
                    row: src.start.row + from,
                    col: src.start.col,
                },
                end: CellRef {
                    row: src.start.row + from + n - 1,
                    col: src.end.col,
                },
            }
        } else {
            CellRange {
                start: CellRef {
                    row: src.start.row,
                    col: src.start.col + from,
                },
                end: CellRef {
                    row: src.end.row,
                    col: src.start.col + from + n - 1,
                },
            }
        }
    };
    let shift = |r: CellRef, d: u32| -> CellRef {
        if vertical {
            CellRef {
                row: r.row + d,
                col: r.col,
            }
        } else {
            CellRef {
                row: r.row,
                col: r.col + d,
            }
        }
    };
    let clip_all = clip::copy(wb, si, src)?;
    let tile = |wb: &mut Workbook, start: CellRef, n_tiles: u32| -> crate::Result<()> {
        if n_tiles == 0 {
            return Ok(());
        }
        let end = shift(start, n_tiles * k - 1);
        let end = if vertical {
            CellRef {
                row: end.row,
                col: src.end.col,
            }
        } else {
            CellRef {
                row: src.end.row,
                col: end.col,
            }
        };
        clip::paste_with(
            wb,
            si,
            start,
            &clip_all,
            Some(CellRange { start, end }),
            true,
        )?;
        Ok(())
    };
    match dir {
        Dir::Down | Dir::Right => {
            tile(wb, ext.start, full)?;
            if rem > 0 {
                let part = clip::copy(wb, si, seed_part(0, rem))?;
                clip::paste_with(wb, si, shift(ext.start, full * k), &part, None, true)?;
            }
        }
        Dir::Up | Dir::Left => {
            if rem > 0 {
                let part = clip::copy(wb, si, seed_part(k - rem, rem))?;
                clip::paste_with(wb, si, ext.start, &part, None, true)?;
            }
            tile(wb, shift(ext.start, rem), full)?;
        }
    }
    // 조건부 서식: 채운 칸마다 그 씨앗 칸의 규칙 묶음 범위를 넓히고, 채운 칸은 다른 묶음에서 뺀다. 데이터 막대가 든
    // 묶음은 건드리지 않고 그 묶음의 칸은 넓히지도 않는다 (우선순위는 그대로 — 자동 채우기 대조)
    {
        let seed_of = |r: CellRef| -> CellRef {
            if vertical {
                let p = r.row as i64 - src.start.row as i64;
                CellRef {
                    row: src.start.row + p.rem_euclid(k as i64) as u32,
                    col: r.col,
                }
            } else {
                let p = r.col as i64 - src.start.col as i64;
                CellRef {
                    row: r.row,
                    col: src.start.col + p.rem_euclid(k as i64) as u32,
                }
            }
        };
        let sq: Vec<&str> = cfs_before.iter().map(|c| c.sqref.as_str()).collect();
        let bars: Vec<bool> = cfs_before.iter().map(super::cfpaste::has_databar).collect();
        let new = autofill_ranges(&sq, &bars, ext, &seed_of);
        wb.sheets[si].conditional_formats = cfs_before
            .iter()
            .zip(new)
            .filter_map(|(c, n)| {
                n.map(|sqref| crate::model::ConditionalFormat { sqref, ..c.clone() })
            })
            .collect();
        // 유효성도 같은 규칙 (범위 목록의 차례를 뒤집지 않는다)
        let sq: Vec<&str> = dvs_before.iter().map(|d| d.sqref.as_str()).collect();
        let new = autofill_ranges(&sq, &vec![false; sq.len()], ext, &seed_of);
        wb.sheets[si].data_validations = dvs_before
            .iter()
            .zip(new)
            .filter_map(|(d, n)| n.map(|sqref| crate::model::DataValidation { sqref, ..d.clone() }))
            .collect();
        // 채운 칸의 하이퍼링크는 없어진다
        wb.sheets[si].hyperlinks.retain(|h| {
            !CellRange::parse(&h.range).is_some_and(|r| {
                !(r.end.row < ext.start.row
                    || r.start.row > ext.end.row
                    || r.end.col < ext.start.col
                    || r.start.col > ext.end.col)
            })
        });
    }
    // 3. 값 이어 가기
    let d1904 = wb.date1904;
    if kind != FillType::Copy && kind != FillType::Formats {
        for (li, &line) in lines.iter().enumerate() {
            for q in 0..len {
                let pos = if vertical {
                    ext.start.row + q
                } else {
                    ext.start.col + q
                };
                let base = if vertical {
                    src.start.row
                } else {
                    src.start.col
                };
                let p = pos as i64 - base as i64;
                let Some(v) = series_value(&seeds[li], p, kind, d1904) else {
                    continue;
                };
                let r = if vertical {
                    CellRef {
                        row: pos,
                        col: line,
                    }
                } else {
                    CellRef {
                        row: line,
                        col: pos,
                    }
                };
                let sheet = &mut wb.sheets[si];
                let c = super::cell_mut(sheet, r);
                c.value = v;
                c.formula = None;
            }
        }
    }
    // 서식만 / 값만: 다른 쪽은 채우기 전으로
    if kind == FillType::Formats || kind == FillType::Values {
        let sheet = &mut wb.sheets[si];
        for (r, v, f, st) in old_values {
            let c = super::cell_mut(sheet, r);
            if kind == FillType::Formats {
                c.value = v;
                c.formula = f;
            } else {
                c.style = st;
            }
        }
    }
    super::structure::update_dimension(&mut wb.sheets[si]);
    Ok(())
}

/// 자동 채우기의 조건부 서식·유효성 범위 (`ext` = 채운 칸, `seed_of` = 채운 칸의 씨앗 칸): 채운 칸마다 그 씨앗
/// 칸이 든 묶음의 범위를 넓히고, 씨앗이 들지 않은 묶음에서는 뺀다. `skip` 묶음(데이터 막대)은 건드리지 않고 그
/// 칸들도 바꾸지 않는다. 돌려주는 값이 None 이면 그 묶음이 없어진다
fn autofill_ranges(
    sqrefs: &[&str],
    skip: &[bool],
    ext: CellRange,
    seed_of: &dyn Fn(CellRef) -> CellRef,
) -> Vec<Option<String>> {
    use super::refs;
    let areas: Vec<Vec<(u32, u32, u32, u32)>> = sqrefs
        .iter()
        .map(|s| refs::parse_sqref(s).iter().map(|a| a.bounds()).collect())
        .collect();
    let n = sqrefs.len();
    let has = |b: usize, r: CellRef| {
        areas[b]
            .iter()
            .any(|a| r.row >= a.0 && r.row <= a.2 && r.col >= a.1 && r.col <= a.3)
    };
    let mut add: Vec<Vec<(u32, u32, u32, u32)>> = vec![Vec::new(); n];
    let mut cut: Vec<bool> = vec![false; n];
    let mut keep_cells: Vec<Vec<(u32, u32, u32, u32)>> = vec![Vec::new(); n];
    for r in ext.start.row..=ext.end.row {
        for c in ext.start.col..=ext.end.col {
            let at = CellRef { row: r, col: c };
            if (0..n).any(|b| skip[b] && has(b, at)) {
                // 건드리지 않는 칸: 다른 묶음의 그 칸도 그대로 둔다
                for b in 0..n {
                    if has(b, at) {
                        keep_cells[b].push((r, c, r, c));
                    }
                }
                continue;
            }
            let seed = seed_of(at);
            for b in 0..n {
                if skip[b] {
                    continue;
                }
                let (want, now) = (has(b, seed), has(b, at));
                if want && !now {
                    add[b].push((r, c, r, c));
                } else if !want && now {
                    cut[b] = true;
                } else if want && now {
                    keep_cells[b].push((r, c, r, c));
                }
            }
        }
    }
    let hole = (ext.start.row, ext.start.col, ext.end.row, ext.end.col);
    (0..n)
        .map(|b| {
            if skip[b] || (add[b].is_empty() && !cut[b]) {
                return Some(sqrefs[b].to_string());
            }
            let mut cur: Vec<(u32, u32, u32, u32)> = Vec::new();
            let mut pieces = std::mem::take(&mut add[b]);
            if cut[b] {
                for &a in &areas[b] {
                    cur.extend(super::clip::subtract_rect(a, hole));
                }
                pieces.extend(keep_cells[b].iter().copied());
            } else {
                cur = areas[b].clone();
            }
            pieces.sort();
            for p in pieces {
                super::sort::merge_into(&mut cur, p);
            }
            if cur.is_empty() {
                return None;
            }
            let text: Vec<refs::Area> = cur
                .iter()
                .map(|&(a, b2, c, d)| refs::Area::rect(a, b2, c, d))
                .collect();
            Some(refs::sqref_text(&text))
        })
        .collect()
}

/// 씨앗 밖으로 넓힌 몫
fn extension(src: CellRange, dst: CellRange, dir: Dir) -> CellRange {
    match dir {
        Dir::Down => CellRange {
            start: CellRef {
                row: src.end.row + 1,
                col: dst.start.col,
            },
            end: dst.end,
        },
        Dir::Up => CellRange {
            start: dst.start,
            end: CellRef {
                row: src.start.row - 1,
                col: dst.end.col,
            },
        },
        Dir::Right => CellRange {
            start: CellRef {
                row: dst.start.row,
                col: src.end.col + 1,
            },
            end: dst.end,
        },
        Dir::Left => CellRange {
            start: dst.start,
            end: CellRef {
                row: dst.end.row,
                col: src.start.col - 1,
            },
        },
    }
}

/// 넓힌 몫의 지금 셀 (값·수식·서식)
fn ext_cells(
    wb: &Workbook,
    si: usize,
    src: CellRange,
    dst: CellRange,
) -> Vec<(CellRef, CellValue, Option<crate::model::Formula>, u32)> {
    let sheet = &wb.sheets[si];
    let mut v = Vec::new();
    for r in dst.start.row..=dst.end.row {
        for c in dst.start.col..=dst.end.col {
            let at = CellRef { row: r, col: c };
            if src.contains(at) {
                continue;
            }
            let (val, f) = sheet
                .cell(at)
                .map(|x| (x.value.clone(), x.formula.clone()))
                .unwrap_or_default();
            v.push((at, val, f, super::style_at(sheet, at)));
        }
    }
    v
}

/// 연속 데이터 종류 (COM `XlDataSeriesType`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SeriesType {
    #[default]
    Linear,
    Growth,
    Date,
    AutoFill,
}

/// 날짜 단위 (COM `XlDataSeriesDate`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateUnit {
    #[default]
    Day,
    Weekday,
    Month,
    Year,
}

/// 연속 데이터 설정 (연속 데이터 대화상자)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesOptions {
    /// 행 방향으로 (오른쪽으로) 채운다. 아니면 열 방향 (아래로)
    pub rows: bool,
    pub kind: SeriesType,
    pub date: DateUnit,
    pub step: f64,
    pub stop: Option<f64>,
    /// 처음 값들의 추세 (직선·지수)로 채운다
    pub trend: bool,
}

/// 연속 데이터 채우기: 줄마다 첫 칸(추세면 처음 값들)에서 시작해 나머지 칸을 채운다. 끝 값을 넘는 칸은 그대로
pub fn data_series(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    o: &SeriesOptions,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    super::guard_cells(wb, si, range)?;
    let (lines, len): (Vec<u32>, u32) = if o.rows {
        (
            (range.start.row..=range.end.row).collect(),
            range.end.col - range.start.col + 1,
        )
    } else {
        (
            (range.start.col..=range.end.col).collect(),
            range.end.row - range.start.row + 1,
        )
    };
    let at = |line: u32, i: u32| -> CellRef {
        if o.rows {
            CellRef {
                row: line,
                col: range.start.col + i,
            }
        } else {
            CellRef {
                row: range.start.row + i,
                col: line,
            }
        }
    };
    if o.kind == SeriesType::AutoFill {
        // 처음 값들을 씨앗으로 자동 채우기
        for &l in &lines {
            let n = (0..len)
                .take_while(|&i| {
                    wb.sheets[si]
                        .cell(at(l, i))
                        .is_some_and(|c| !c.value.is_empty())
                })
                .count() as u32;
            if n == 0 || n == len {
                continue;
            }
            let src = CellRange {
                start: at(l, 0),
                end: at(l, n - 1),
            };
            let dst = CellRange {
                start: at(l, 0),
                end: at(l, len - 1),
            };
            auto_fill(wb, si, src, dst, FillType::Series)?;
        }
        return Ok(());
    }
    let d1904 = wb.date1904;
    for &l in &lines {
        let sheet = &wb.sheets[si];
        let firsts: Vec<f64> = (0..len)
            .map_while(|i| match sheet.cell(at(l, i)).map(|c| &c.value) {
                Some(CellValue::Number(v)) => Some(*v),
                _ => None,
            })
            .collect();
        let Some(&v0) = firsts.first() else { continue };
        let style = super::style_at(sheet, at(l, 0));
        let values: Vec<Option<f64>> = if o.trend && firsts.len() > 1 {
            let n = firsts.len() as f64;
            let xs: Vec<f64> = (0..firsts.len()).map(|i| i as f64).collect();
            let (sx, sxx): (f64, f64) = (xs.iter().sum(), xs.iter().map(|x| x * x).sum());
            match o.kind {
                SeriesType::Growth => {
                    let ys: Vec<f64> = firsts.iter().map(|v| v.ln()).collect();
                    let sy: f64 = ys.iter().sum();
                    let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| x * y).sum();
                    let b = (n * sxy - sx * sy) / (n * sxx - sx * sx);
                    let a = (sy - b * sx) / n;
                    (0..len)
                        .map(|i| Some(crate::calc::value::round15((a + b * i as f64).exp())))
                        .collect()
                }
                _ => {
                    let sy: f64 = firsts.iter().sum();
                    let sxy: f64 = xs.iter().zip(&firsts).map(|(x, y)| x * y).sum();
                    let b = (n * sxy - sx * sy) / (n * sxx - sx * sx);
                    let a = (sy - b * sx) / n;
                    (0..len)
                        .map(|i| Some(crate::calc::value::round15(a + b * i as f64)))
                        .collect()
                }
            }
        } else {
            let mut out = vec![Some(v0)];
            let mut cur = v0;
            for i in 1..len {
                let next = match o.kind {
                    SeriesType::Growth => cur * o.step,
                    SeriesType::Date => {
                        let n = (o.step * i as f64).round() as i64;
                        match o.date {
                            DateUnit::Day => v0 + o.step * i as f64,
                            DateUnit::Weekday => add_weekdays(v0, n, d1904),
                            DateUnit::Month => add_months(v0, n, d1904).unwrap_or(v0),
                            DateUnit::Year => add_months(v0, 12 * n, d1904).unwrap_or(v0),
                        }
                    }
                    _ => cur + o.step,
                };
                let next = crate::calc::value::round15(next);
                if let Some(stop) = o.stop
                    && ((o.step >= 0.0 && next > stop) || (o.step < 0.0 && next < stop))
                {
                    break;
                }
                out.push(Some(next));
                cur = next;
            }
            out
        };
        let sheet = &mut wb.sheets[si];
        for (i, v) in values.into_iter().enumerate() {
            let Some(v) = v else { continue };
            let c = super::cell_mut(sheet, at(l, i as u32));
            c.value = CellValue::Number(v);
            c.formula = None;
            if i > 0 {
                c.style = style;
            }
        }
    }
    super::structure::update_dimension(&mut wb.sheets[si]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_numbers_like_hcell() {
        let t = text_num("항목 9").unwrap();
        assert_eq!(text_num_at(&t, 10), "항목 10");
        let t = text_num("007").unwrap();
        assert_eq!(text_num_at(&t, 8), "008");
        assert_eq!(text_num_at(&t, 100), "100");
        let t = text_num("1st").unwrap();
        assert_eq!(
            (2..=5).map(|n| text_num_at(&t, n)).collect::<Vec<_>>(),
            ["2nd", "3rd", "4th", "5th"]
        );
        assert_eq!(text_num_at(&t, 12), "12th");
        assert_eq!(text_num_at(&t, 102), "102nd");
        // 0 아래는 절댓값
        assert_eq!(text_num_at(&text_num("항목3").unwrap(), -1), "항목1");
        let t = text_num("가1나2").unwrap();
        assert_eq!(text_num_at(&t, 3), "가1나3");
    }

    #[test]
    fn list_case_like_hcell() {
        assert_eq!(case_like("Tuesday", "mon"), "tuesday");
        assert_eq!(case_like("Tuesday", "MONDAY"), "TUESDAY");
        assert_eq!(
            case_like("Second Quarter", "First Quarter"),
            "Second quarter"
        );
        assert_eq!(list_item("월"), Some((0, 1)));
        assert_eq!(list_item("mon"), Some((13, 1)));
    }

    #[test]
    fn series_values_like_hcell() {
        let num = |v: f64| Seed::Num(v, None);
        let val = |seeds: &[Seed], p: i64, kind| match series_value(seeds, p, kind, false) {
            Some(CellValue::Number(v)) => format!("{v}"),
            Some(CellValue::Text(t)) => t,
            _ => "copy".into(),
        };
        // 숫자 하나는 복사, 연속 데이터면 1씩
        assert_eq!(val(&[num(1.0)], 1, FillType::Default), "copy");
        assert_eq!(val(&[num(1.0)], 2, FillType::Series), "3");
        // 1, 2, 4 → 최소 제곱 직선
        let s = [num(1.0), num(2.0), num(4.0)];
        assert_eq!(val(&s, 3, FillType::Default), "5.33333333333333");
        // 위로: 1, 3 → -1
        assert_eq!(val(&[num(1.0), num(3.0)], -1, FillType::Default), "-1");
        // [1, 항목1]: 각 묶음이 따로
        let s = [num(1.0), Seed::Text(text_num("항목1").unwrap())];
        assert_eq!(val(&s, 2, FillType::Default), "2");
        assert_eq!(val(&s, 3, FillType::Default), "항목2");
        // 위로 채우는 두 항목 글 묶음: 마지막 항목에서 앞으로 (한셀)
        let t = |s: &str| Seed::Text(text_num(s).unwrap());
        let s = [t("항목1"), t("항목3")];
        assert_eq!(val(&s, -1, FillType::Default), "항목7");
        assert_eq!(val(&s, -6, FillType::Default), "항목17");
        // 목록 (월, 수 → 금, 일 …)
        let l = |s: &str| {
            let (a, b) = list_item(s).unwrap();
            Seed::List(a, b, s.into())
        };
        let s = [l("월"), l("수")];
        assert_eq!(val(&s, 2, FillType::Default), "금");
        assert_eq!(val(&s, 3, FillType::Default), "일");
    }

    #[test]
    fn dates_like_hcell() {
        // 2024-01-31 + 1달 = 2024-02-29, 2024-02-29 + 1해 = 2025-02-28
        let jan31 = crate::calc::date::serial_from_ymd(2024, 1, 31, false).unwrap();
        let feb29 = crate::calc::date::serial_from_ymd(2024, 2, 29, false).unwrap();
        assert_eq!(add_months(jan31, 1, false), Some(feb29));
        let feb28 = crate::calc::date::serial_from_ymd(2025, 2, 28, false).unwrap();
        assert_eq!(add_months(feb29, 12, false), Some(feb28));
        // 금요일(2024-01-26) 다음 평일은 월요일(29일)
        let fri = crate::calc::date::serial_from_ymd(2024, 1, 26, false).unwrap();
        assert_eq!(add_weekdays(fri, 1, false), fri + 3.0);
    }
}
