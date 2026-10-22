// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 조건부 서식 평가 (뷰어용).
//!
//! 셀 값 비교(상수), 글 포함·시작·끝, 빈 칸·오류, 상위/하위, 평균 위/아래, 중복/고유,
//! 색 눈금, 데이터 막대, 아이콘을 계산한다. 조건식(`expression`)은 수식 계산기가 없어 건너뛴다.
//! 우선순위 순으로 적용하고, 앞선 규칙이 정한 속성은 뒤 규칙이 덮지 않는다 (Excel·한셀과 같다).

use std::collections::HashMap;

use crate::model::{CellRange, CellRef, CellValue, Cfvo, Color, Dxf, Sheet, Styles};

/// 한 칸에 적용될 조건부 서식 결과
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CfStyle {
    /// 차등 서식 (우선순위가 높은 것부터 채운 속성들)
    pub font_color: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub fill: Option<Color>,
    /// 색 눈금으로 정한 배경 (#RRGGBB)
    pub scale: Option<String>,
    /// 데이터 막대
    pub bar: Option<BarDraw>,
    /// 아이콘: (집합 이름, 집합 안 번호 — 0 이 가장 낮은 단계, 집합의 아이콘 수). 사용자 지정 아이콘이면 그 집합·번호
    pub icon: Option<(String, usize, usize)>,
    pub hide_value: bool,
}

/// 데이터 막대 한 칸. 한셀 화면은 막대를 직각삼각형으로 그린다 — 시작(축) 쪽이 막대 높이 그대로이고 끝에서 0
/// (x14 데이터 막대 실험: 그라데이션·단색, 테두리, 음수, 축 위치, 방향, 확장 없는 막대 모두 같은 모양)
#[derive(Debug, Clone, PartialEq)]
pub struct BarDraw {
    /// 축 자리 (막대 영역의 0–1)와 축 색 (#RRGGBB). 음수가 섞였거나 가운데 축일 때만 — 이때 막대는 축에서 시작
    pub axis: Option<(f64, String)>,
    /// 막대 길이: 시작에서 그쪽 끝까지의 비율 (0–1). 축이 없으면 영역 왼쪽에서
    pub len: f64,
    /// 축에서 왼쪽으로 (음수)
    pub leftward: bool,
    /// 채우기·테두리 색 (#RRGGBB)
    pub fill: String,
    pub border: Option<String>,
    /// 그라데이션 (시작 색 → 끝에서 흰색, 막대 길이에 맞춰)
    pub gradient: bool,
    /// 오른쪽에서 왼쪽 (direction="rightToLeft") — 칸 안에서 좌우를 뒤집는다
    pub rtl: bool,
}

/// 아이콘 집합의 아이콘 수 (이름 앞 숫자)
pub fn icon_count(set: &str) -> usize {
    set.chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .map_or(3, |d| d as usize)
}

/// 데이터 막대의 기준 값: x14 의 autoMin·autoMax 는 0 을 넣는다 (0 또는 음수 최솟값, 0 또는 양수 최댓값)
fn bar_bound(c: &Cfvo, nums: &[f64]) -> Option<f64> {
    if nums.is_empty() {
        return None;
    }
    let (lo, hi) = nums
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    match c.kind.as_str() {
        "autoMin" => Some(lo.min(0.0)),
        "autoMax" => Some(hi.max(0.0)),
        "formula" => c.val.as_deref().and_then(|v| v.trim().parse().ok()),
        _ => cfvo_value(c, nums),
    }
}

fn cells_in(sqref: &str) -> Vec<CellRange> {
    sqref
        .split_whitespace()
        .filter_map(CellRange::parse)
        .collect()
}

fn num(v: &CellValue) -> Option<f64> {
    match v {
        CellValue::Number(n) => Some(*n),
        CellValue::Bool(b) => Some(*b as u8 as f64),
        _ => None,
    }
}

fn text(v: &CellValue) -> String {
    match v {
        CellValue::Text(s) => s.clone(),
        CellValue::Number(n) => crate::numfmt::general(*n),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        CellValue::Error(e) => e.clone(),
        _ => String::new(),
    }
}

/// 상수 비교 값: 숫자 또는 "글자"
fn constant(f: &str) -> Option<CellValue> {
    let t = f.trim().trim_start_matches('=');
    if let Ok(n) = t.parse::<f64>() {
        return Some(CellValue::Number(n));
    }
    let s = t.strip_prefix('"')?.strip_suffix('"')?;
    Some(CellValue::Text(s.replace("\"\"", "\"")))
}

fn cmp(a: &CellValue, b: &CellValue) -> Option<std::cmp::Ordering> {
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y),
        _ => Some(text(a).to_lowercase().cmp(&text(b).to_lowercase())),
    }
}

/// cfvo 의 실제 값 (범위의 숫자들 기준)
fn cfvo_value(c: &Cfvo, nums: &[f64]) -> Option<f64> {
    let (lo, hi) = nums
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    if nums.is_empty() {
        return None;
    }
    let val = || c.val.as_deref().and_then(|v| v.parse::<f64>().ok());
    Some(match c.kind.as_str() {
        "min" | "autoMin" => lo,
        "max" | "autoMax" => hi,
        "num" => val()?,
        "percent" => lo + (hi - lo) * val()? / 100.0,
        "percentile" => percentile(nums, val()? / 100.0),
        _ => return None,
    })
}

fn percentile(nums: &[f64], p: f64) -> f64 {
    let mut v = nums.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let k = (v.len() - 1) as f64 * p.clamp(0.0, 1.0);
    let (i, f) = (k.floor() as usize, k.fract());
    v[i] + (v.get(i + 1).copied().unwrap_or(v[i]) - v[i]) * f
}

fn rgb(c: &Color, theme: &crate::theme::Theme) -> Option<(f64, f64, f64)> {
    let s = crate::theme::resolve(c, theme)?;
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).ok()?;
    Some((
        ((v >> 16) & 0xFF) as f64,
        ((v >> 8) & 0xFF) as f64,
        (v & 0xFF) as f64,
    ))
}

/// 시트의 조건부 서식을 평가해 칸마다 결과를 낸다.
pub fn evaluate(
    sheet: &Sheet,
    styles: &Styles,
    theme: &crate::theme::Theme,
) -> HashMap<(u32, u32), CfStyle> {
    let mut out: HashMap<(u32, u32), CfStyle> = HashMap::new();
    let value = |r: CellRef| sheet.cell(r).map(|c| c.value.clone()).unwrap_or_default();
    let mut rules: Vec<(&crate::model::CfRule, Vec<CellRange>)> = sheet
        .conditional_formats
        .iter()
        .flat_map(|cf| cf.rules.iter().map(move |r| (r, cells_in(&cf.sqref))))
        .collect();
    rules.sort_by_key(|(r, _)| r.priority);
    let mut stopped: std::collections::HashSet<(u32, u32)> = Default::default();
    for (rule, ranges) in rules {
        // 범위의 칸들 (값이 있는 칸만 비교에 쓴다, 아주 큰 범위는 셀 목록으로 줄인다)
        let mut cells: Vec<CellRef> = Vec::new();
        for rg in &ranges {
            let area =
                (rg.end.row - rg.start.row + 1) as u64 * (rg.end.col - rg.start.col + 1) as u64;
            if area <= 200_000 {
                for row in rg.start.row..=rg.end.row {
                    for col in rg.start.col..=rg.end.col {
                        cells.push(CellRef { row, col });
                    }
                }
            } else {
                cells.extend(
                    sheet
                        .cells
                        .iter()
                        .filter(|c| {
                            (rg.start.row..=rg.end.row).contains(&c.r.row)
                                && (rg.start.col..=rg.end.col).contains(&c.r.col)
                        })
                        .map(|c| c.r),
                );
            }
        }
        let nums: Vec<f64> = cells.iter().filter_map(|&r| num(&value(r))).collect();
        let dxf = rule.dxf_id.and_then(|i| styles.dxfs.get(i as usize));
        let mut hit = |r: CellRef, out: &mut HashMap<(u32, u32), CfStyle>| {
            if stopped.contains(&(r.row, r.col)) {
                return;
            }
            if let Some(d) = dxf {
                apply_dxf(out.entry((r.row, r.col)).or_default(), d);
            }
            if rule.stop_if_true {
                stopped.insert((r.row, r.col));
            }
        };
        match rule.kind.as_str() {
            "cellIs" => {
                let a = rule.formulas.first().and_then(|f| constant(f));
                let b = rule.formulas.get(1).and_then(|f| constant(f));
                let Some(a) = a else { continue };
                for &r in &cells {
                    let v = value(r);
                    if matches!(v, CellValue::Empty) {
                        continue;
                    }
                    use std::cmp::Ordering::*;
                    let c = cmp(&v, &a);
                    let ok = match rule.operator.as_deref().unwrap_or("between") {
                        "equal" => c == Some(Equal),
                        "notEqual" => c != Some(Equal),
                        "greaterThan" => c == Some(Greater),
                        "lessThan" => c == Some(Less),
                        "greaterThanOrEqual" => matches!(c, Some(Greater | Equal)),
                        "lessThanOrEqual" => matches!(c, Some(Less | Equal)),
                        op => {
                            let Some(b) = &b else { continue };
                            let (lo, hi) = if cmp(&a, b) == Some(Greater) {
                                (b, &a)
                            } else {
                                (&a, b)
                            };
                            let inside = !matches!(cmp(&v, lo), Some(Less))
                                && !matches!(cmp(&v, hi), Some(Greater));
                            if op == "notBetween" { !inside } else { inside }
                        }
                    };
                    if ok {
                        hit(r, &mut out);
                    }
                }
            }
            "containsText" | "notContainsText" | "beginsWith" | "endsWith" => {
                let needle = rule.text.as_deref().unwrap_or("").to_lowercase();
                for &r in &cells {
                    let t = text(&value(r)).to_lowercase();
                    let ok = match rule.operator.as_deref().unwrap_or(&rule.kind) {
                        "beginsWith" => t.starts_with(&needle),
                        "endsWith" => t.ends_with(&needle),
                        "notContains" | "notContainsText" => !t.contains(&needle),
                        _ => t.contains(&needle),
                    };
                    if ok {
                        hit(r, &mut out);
                    }
                }
            }
            "containsBlanks" | "notContainsBlanks" | "containsErrors" | "notContainsErrors" => {
                for &r in &cells {
                    let v = value(r);
                    let ok = match rule.kind.as_str() {
                        "containsBlanks" => text(&v).trim().is_empty(),
                        "notContainsBlanks" => !text(&v).trim().is_empty(),
                        "containsErrors" => matches!(v, CellValue::Error(_)),
                        _ => !matches!(v, CellValue::Error(_)),
                    };
                    if ok {
                        hit(r, &mut out);
                    }
                }
            }
            "top10" => {
                let mut sorted = nums.clone();
                sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                if rule.bottom {
                    sorted.reverse();
                }
                let rank = rule.rank.unwrap_or(10) as usize;
                let n = if rule.percent {
                    ((sorted.len() * rank) as f64 / 100.0).floor().max(1.0) as usize
                } else {
                    rank
                };
                let Some(&cut) = sorted.get(n.min(sorted.len()).saturating_sub(1)) else {
                    continue;
                };
                for &r in &cells {
                    if let Some(v) = num(&value(r))
                        && (if rule.bottom { v <= cut } else { v >= cut })
                    {
                        hit(r, &mut out);
                    }
                }
            }
            "aboveAverage" => {
                if nums.is_empty() {
                    continue;
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let sd = (nums.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / nums.len() as f64)
                    .sqrt();
                let above = rule.above_average.unwrap_or(true);
                let k = rule.std_dev.unwrap_or(0) as f64;
                let line = if above { mean + k * sd } else { mean - k * sd };
                for &r in &cells {
                    if let Some(v) = num(&value(r)) {
                        let ok = if above {
                            v > line || (rule.equal_average && v == line)
                        } else {
                            v < line || (rule.equal_average && v == line)
                        };
                        if ok {
                            hit(r, &mut out);
                        }
                    }
                }
            }
            "duplicateValues" | "uniqueValues" => {
                let mut count: HashMap<String, usize> = HashMap::new();
                for &r in &cells {
                    let t = text(&value(r)).to_lowercase();
                    if !t.is_empty() {
                        *count.entry(t).or_default() += 1;
                    }
                }
                for &r in &cells {
                    let t = text(&value(r)).to_lowercase();
                    if t.is_empty() {
                        continue;
                    }
                    let dup = count[&t] > 1;
                    if dup == (rule.kind == "duplicateValues") {
                        hit(r, &mut out);
                    }
                }
            }
            "colorScale" => {
                let Some(cs) = &rule.color_scale else {
                    continue;
                };
                let stops: Vec<(f64, (f64, f64, f64))> = cs
                    .cfvos
                    .iter()
                    .zip(&cs.colors)
                    .filter_map(|(c, col)| Some((cfvo_value(c, &nums)?, rgb(col, theme)?)))
                    .collect();
                if stops.len() < 2 {
                    continue;
                }
                for &r in &cells {
                    let Some(v) = num(&value(r)) else { continue };
                    let e = out.entry((r.row, r.col)).or_default();
                    if e.scale.is_some() || stopped.contains(&(r.row, r.col)) {
                        continue;
                    }
                    let c = interpolate(&stops, v);
                    e.scale = Some(format!(
                        "#{:02X}{:02X}{:02X}",
                        c.0 as u8, c.1 as u8, c.2 as u8
                    ));
                }
            }
            "dataBar" => {
                let Some(db) = &rule.data_bar else { continue };
                // 확장이 없으면 한셀이 저장할 때 만드는 확장 (기준 값은 본 규칙 것, 축 자동)
                let implied = db.ext.is_none();
                let ext = db
                    .ext
                    .clone()
                    .unwrap_or_else(|| crate::model::DataBarExt::implied(db));
                let cfvos = if ext.cfvos.len() >= 2 {
                    &ext.cfvos
                } else {
                    &db.cfvos
                };
                let lo = cfvos.first().and_then(|c| bar_bound(c, &nums));
                let hi = cfvos.get(1).and_then(|c| bar_bound(c, &nums));
                let (Some(lo), Some(hi)) = (lo, hi) else {
                    continue;
                };
                let (min_len, max_len) = (
                    db.min_length.unwrap_or(10) as f64 / 100.0,
                    db.max_length.unwrap_or(90) as f64 / 100.0,
                );
                let len = |t: f64| min_len + (max_len - min_len) * t.clamp(0.0, 1.0);
                let res = |c: &Option<Color>| {
                    c.as_ref()
                        .and_then(|c| crate::theme::resolve(c, theme))
                        .map(|s| s.to_ascii_uppercase())
                };
                let fill = res(&db.color).unwrap_or_else(|| "#638EC6".into());
                // 음수 색이 없으면: 확장 없는 막대는 검정 (한셀 화면), 확장이 있으면 x14 기본 빨강
                let neg_default = if implied { "#000000" } else { "#FF0000" };
                let neg_fill = if ext.negative_same_as_positive {
                    fill.clone()
                } else {
                    res(&ext.negative_fill_color).unwrap_or_else(|| neg_default.into())
                };
                let border = ext
                    .border
                    .then(|| res(&ext.border_color).unwrap_or_else(|| fill.clone()));
                let neg_border = border.as_ref().map(|b| {
                    if ext.negative_border_same_as_positive {
                        b.clone()
                    } else {
                        res(&ext.negative_border_color).unwrap_or_else(|| neg_default.into())
                    }
                });
                let axis_color = res(&ext.axis_color).unwrap_or_else(|| "#000000".into());
                let middle = ext.axis_position.as_deref() == Some("middle");
                // 축 자리: 자동은 음수·양수가 섞일 때 0 의 자리, 모두 음수면 오른쪽 끝, 가운데는 0.5
                let axis = match ext.axis_position.as_deref() {
                    Some("none") => None,
                    Some("middle") => Some(0.5),
                    _ if lo < 0.0 && hi > 0.0 => Some(-lo / (hi - lo)),
                    _ if hi <= 0.0 && lo < 0.0 => Some(1.0),
                    _ => None,
                };
                for &r in &cells {
                    let Some(v) = num(&value(r)) else { continue };
                    let neg = v < 0.0;
                    let (len, leftward) = match axis {
                        Some(_) if middle => {
                            let s = lo.abs().max(hi.abs());
                            let t = if s > 0.0 { v.abs() / s } else { 0.0 };
                            (if v == 0.0 { 0.0 } else { len(t) }, neg)
                        }
                        Some(_) => {
                            let t = if neg {
                                v / lo
                            } else if hi > 0.0 {
                                v / hi
                            } else {
                                0.0
                            };
                            (if v == 0.0 { 0.0 } else { len(t) }, neg)
                        }
                        None => {
                            let t = if hi > lo { (v - lo) / (hi - lo) } else { 1.0 };
                            (len(t), false)
                        }
                    };
                    let e = out.entry((r.row, r.col)).or_default();
                    if e.bar.is_none() {
                        e.bar = Some(BarDraw {
                            axis: axis.map(|a| (a, axis_color.clone())),
                            len,
                            leftward,
                            fill: if neg { neg_fill.clone() } else { fill.clone() },
                            border: if neg {
                                neg_border.clone()
                            } else {
                                border.clone()
                            },
                            gradient: ext.gradient,
                            rtl: ext.direction.as_deref() == Some("rightToLeft"),
                        });
                        e.hide_value |= !db.show_value;
                    }
                }
            }
            "iconSet" => {
                let Some(is) = &rule.icon_set else { continue };
                let th: Vec<(f64, bool)> = is
                    .cfvos
                    .iter()
                    .filter_map(|c| Some((cfvo_value(c, &nums)?, c.gte)))
                    .collect();
                let n = icon_count(&is.set);
                for &r in &cells {
                    let Some(v) = num(&value(r)) else { continue };
                    // 마지막으로 넘어선 기준의 단계 (첫 기준은 바닥). 한셀 화면은 거꾸로(reverse)를 따르지 않는다
                    // (아이콘 실험: 본 구역·x14 구역 모두 순서 그대로)
                    let mut level = 0;
                    for (i, &(t, gte)) in th.iter().enumerate().skip(1) {
                        if if gte { v >= t } else { v > t } {
                            level = i;
                        }
                    }
                    let level = level.min(n - 1);
                    // 사용자 지정 아이콘: 단계마다 (집합, 번호), NoIcons 는 아이콘 없음
                    let icon = match is.custom_icons.get(level) {
                        Some((set, _)) if set == "NoIcons" => None,
                        Some((set, id)) => Some((set.clone(), *id as usize, icon_count(set))),
                        None => Some((is.set.clone(), level, n)),
                    };
                    let e = out.entry((r.row, r.col)).or_default();
                    if e.icon.is_none() {
                        e.icon = icon;
                        e.hide_value |= !is.show_value;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn interpolate(stops: &[(f64, (f64, f64, f64))], v: f64) -> (f64, f64, f64) {
    if v <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if v <= b {
            let t = if b > a { (v - a) / (b - a) } else { 1.0 };
            return (
                ca.0 + (cb.0 - ca.0) * t,
                ca.1 + (cb.1 - ca.1) * t,
                ca.2 + (cb.2 - ca.2) * t,
            );
        }
    }
    stops[stops.len() - 1].1
}

fn apply_dxf(e: &mut CfStyle, d: &Dxf) {
    if let Some(f) = &d.font {
        if e.font_color.is_none() {
            e.font_color = f.color.clone();
        }
        if f.bold && e.bold.is_none() {
            e.bold = Some(true);
        }
        if f.italic && e.italic.is_none() {
            e.italic = Some(true);
        }
        if f.underline.is_some() && e.underline.is_none() {
            e.underline = Some(true);
        }
        if f.strike && e.strike.is_none() {
            e.strike = Some(true);
        }
    }
    if let Some(fl) = &d.fill
        && e.fill.is_none()
    {
        // 차등 서식의 단색 채우기는 bgColor (patternType 이 solid 면 fgColor)
        e.fill = if fl.pattern_type.as_deref() == Some("solid") {
            fl.fg_color.clone().or_else(|| fl.bg_color.clone())
        } else {
            fl.bg_color.clone().or_else(|| fl.fg_color.clone())
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cell, CfRule, ConditionalFormat, Fill, Font};

    fn sheet_with(values: &[f64], rules: Vec<CfRule>) -> Sheet {
        Sheet {
            cells: values
                .iter()
                .enumerate()
                .map(|(i, v)| Cell {
                    r: CellRef {
                        row: i as u32,
                        col: 0,
                    },
                    style: 0,
                    value: CellValue::Number(*v),
                    formula: None,
                })
                .collect(),
            conditional_formats: vec![ConditionalFormat {
                sqref: format!("A1:A{}", values.len()),
                rules,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn cell_is_top10_and_stop_if_true() {
        let styles = Styles {
            dxfs: vec![
                Dxf {
                    font: Some(Font {
                        bold: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Dxf {
                    fill: Some(Fill {
                        bg_color: Some(Color {
                            rgb: Some("FFFF0000".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let s = sheet_with(
            &[1.0, 5.0, 9.0, 3.0],
            vec![
                CfRule {
                    kind: "cellIs".into(),
                    operator: Some("greaterThan".into()),
                    formulas: vec!["4".into()],
                    dxf_id: Some(0),
                    priority: 1,
                    stop_if_true: true,
                    ..Default::default()
                },
                CfRule {
                    kind: "top10".into(),
                    rank: Some(2),
                    dxf_id: Some(1),
                    priority: 2,
                    ..Default::default()
                },
            ],
        );
        let r = evaluate(&s, &styles, &Default::default());
        assert_eq!(r[&(1, 0)].bold, Some(true));
        // 9 는 굵게(1순위, 멈춤) — 상위 2 의 채우기는 받지 않는다
        assert_eq!(r[&(2, 0)].fill, None);
        assert!(r.get(&(0, 0)).is_none_or(|c| c.bold.is_none()));
    }

    #[test]
    fn color_scale_and_data_bar() {
        let s = sheet_with(
            &[0.0, 50.0, 100.0],
            vec![CfRule {
                kind: "colorScale".into(),
                priority: 1,
                color_scale: Some(crate::model::ColorScale {
                    cfvos: vec![
                        Cfvo {
                            kind: "min".into(),
                            ..Default::default()
                        },
                        Cfvo {
                            kind: "max".into(),
                            ..Default::default()
                        },
                    ],
                    colors: vec![
                        Color {
                            rgb: Some("FFFFFFFF".into()),
                            ..Default::default()
                        },
                        Color {
                            rgb: Some("FF000000".into()),
                            ..Default::default()
                        },
                    ],
                }),
                ..Default::default()
            }],
        );
        let r = evaluate(&s, &Styles::default(), &Default::default());
        assert_eq!(r[&(0, 0)].scale.as_deref(), Some("#FFFFFF"));
        assert_eq!(r[&(1, 0)].scale.as_deref(), Some("#7F7F7F"));
        assert_eq!(r[&(2, 0)].scale.as_deref(), Some("#000000"));
    }

    fn cfvo(kind: &str) -> Cfvo {
        Cfvo {
            kind: kind.into(),
            ..Default::default()
        }
    }

    #[test]
    fn data_bar_axis_and_negative_colors() {
        // x14 막대 (x14 데이터 막대 실험의 B 열): −40·0·100 → 축은 40/140 자리, 음수는 축에서 왼쪽으로 빨강
        let bar = crate::model::DataBar {
            cfvos: vec![cfvo("min"), cfvo("max")],
            color: Some(Color {
                rgb: Some("FF638EC6".into()),
                ..Default::default()
            }),
            show_value: true,
            min_length: Some(0),
            max_length: Some(100),
            ext: Some(crate::model::DataBarExt {
                cfvos: vec![cfvo("autoMin"), cfvo("autoMax")],
                negative_fill_color: Some(Color {
                    rgb: Some("FFFF0000".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        let rule = |b: crate::model::DataBar| CfRule {
            kind: "dataBar".into(),
            priority: 1,
            data_bar: Some(b),
            ..Default::default()
        };
        let r = evaluate(
            &sheet_with(&[-40.0, 0.0, 100.0], vec![rule(bar.clone())]),
            &Styles::default(),
            &Default::default(),
        );
        let b0 = r[&(0, 0)].bar.as_ref().unwrap();
        let (a, c) = b0.axis.clone().unwrap();
        assert!((a - 40.0 / 140.0).abs() < 1e-9 && c == "#000000");
        assert!(b0.leftward && b0.fill == "#FF0000" && (b0.len - 1.0).abs() < 1e-9);
        assert_eq!(r[&(1, 0)].bar.as_ref().unwrap().len, 0.0);
        let b2 = r[&(2, 0)].bar.as_ref().unwrap();
        assert!(!b2.leftward && b2.fill == "#638EC6" && (b2.len - 1.0).abs() < 1e-9);
        // 확장 없는 막대: 한셀이 만드는 확장(축 자동) — 음수는 검정, 길이 10–90%
        let mut old = bar;
        old.ext = None;
        old.min_length = None;
        old.max_length = None;
        let r = evaluate(
            &sheet_with(&[-40.0, 100.0], vec![rule(old)]),
            &Styles::default(),
            &Default::default(),
        );
        let b0 = r[&(0, 0)].bar.as_ref().unwrap();
        assert!(b0.leftward && b0.fill == "#000000" && (b0.len - 0.9).abs() < 1e-9);
        // 모두 양수면 축이 없고 autoMin 은 0
        let r = evaluate(
            &sheet_with(
                &[10.0, 80.0],
                vec![rule(crate::model::DataBar {
                    cfvos: vec![cfvo("min"), cfvo("max")],
                    show_value: true,
                    min_length: Some(0),
                    max_length: Some(100),
                    ext: Some(crate::model::DataBarExt {
                        cfvos: vec![cfvo("autoMin"), cfvo("autoMax")],
                        ..Default::default()
                    }),
                    ..Default::default()
                })],
            ),
            &Styles::default(),
            &Default::default(),
        );
        let b0 = r[&(0, 0)].bar.as_ref().unwrap();
        assert!(b0.axis.is_none() && (b0.len - 0.125).abs() < 1e-9);
    }

    #[test]
    fn custom_icons_and_reverse() {
        let pct = |v: &str| Cfvo {
            kind: "percent".into(),
            val: Some(v.into()),
            gte: true,
        };
        let set = |reverse: bool, custom: Vec<(String, u32)>| CfRule {
            kind: "iconSet".into(),
            priority: 1,
            icon_set: Some(crate::model::IconSet {
                set: "3TrafficLights1".into(),
                cfvos: vec![pct("0"), pct("33"), pct("67")],
                show_value: true,
                reverse,
                percent: true,
                custom_icons: custom,
            }),
            ..Default::default()
        };
        // 한셀은 거꾸로를 따르지 않는다
        let r = evaluate(
            &sheet_with(&[1.0, 3.0, 5.0], vec![set(true, vec![])]),
            &Styles::default(),
            &Default::default(),
        );
        assert_eq!(r[&(0, 0)].icon, Some(("3TrafficLights1".into(), 0, 3)));
        assert_eq!(r[&(2, 0)].icon, Some(("3TrafficLights1".into(), 2, 3)));
        // 사용자 지정: 단계마다 (집합, 번호), NoIcons 는 없음
        let custom = vec![
            ("3Flags".into(), 0),
            ("NoIcons".into(), 0),
            ("3Symbols".into(), 2),
        ];
        let r = evaluate(
            &sheet_with(&[1.0, 3.0, 5.0], vec![set(false, custom)]),
            &Styles::default(),
            &Default::default(),
        );
        assert_eq!(r[&(0, 0)].icon, Some(("3Flags".into(), 0, 3)));
        assert_eq!(r[&(1, 0)].icon, None);
        assert_eq!(r[&(2, 0)].icon, Some(("3Symbols".into(), 2, 3)));
    }
}
