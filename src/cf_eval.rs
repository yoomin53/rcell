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
    /// 데이터 막대: (0–1 길이, 색)
    pub bar: Option<(f64, Color)>,
    /// 아이콘: (집합 이름, 단계, 단계 수), 값 숨김 여부
    pub icon: Option<(String, usize, usize)>,
    pub hide_value: bool,
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
                let lo = db.cfvos.first().and_then(|c| cfvo_value(c, &nums));
                let hi = db.cfvos.get(1).and_then(|c| cfvo_value(c, &nums));
                let (Some(lo), Some(hi)) = (lo, hi) else {
                    continue;
                };
                let (min_len, max_len) = (
                    db.min_length.unwrap_or(10) as f64 / 100.0,
                    db.max_length.unwrap_or(90) as f64 / 100.0,
                );
                let color = db.color.clone().unwrap_or(Color {
                    rgb: Some("FF638EC6".into()),
                    ..Default::default()
                });
                for &r in &cells {
                    let Some(v) = num(&value(r)) else { continue };
                    let t = if hi > lo {
                        ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    let e = out.entry((r.row, r.col)).or_default();
                    if e.bar.is_none() {
                        e.bar = Some((min_len + (max_len - min_len) * t, color.clone()));
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
                let n = is.cfvos.len().max(3);
                for &r in &cells {
                    let Some(v) = num(&value(r)) else { continue };
                    // 마지막으로 넘어선 기준의 단계 (첫 기준은 바닥)
                    let mut level = 0;
                    for (i, &(t, gte)) in th.iter().enumerate().skip(1) {
                        if if gte { v >= t } else { v > t } {
                            level = i;
                        }
                    }
                    if is.reverse {
                        level = n - 1 - level;
                    }
                    let e = out.entry((r.row, r.col)).or_default();
                    if e.icon.is_none() {
                        e.icon = Some((is.set.clone(), level, n));
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
}
