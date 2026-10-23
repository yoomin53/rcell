// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 스파크라인 (`x:extLst/x:ext/x14:sparklineGroups`, Excel 2010 확장 — 한셀도 그대로 쓴다).
//!
//! 문서 모델은 그룹 수만 세므로, 그리기에 필요한 내용은 원본 패키지의 시트 파트에서 다시 읽는다.
//! 칸 안에 꺾은선·막대·승패 그림을 인라인 SVG 로 그린다.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::model::{CellRef, CellValue, Color, Workbook};
use crate::ns;
use crate::theme::{self, Theme};
use crate::xml::Element;

/// 스파크라인 그룹 하나
#[derive(Debug, Clone, Default)]
pub struct SparklineGroup {
    /// `line`(기본) / `column` / `stacked`(승패)
    pub kind: String,
    pub color_series: Option<Color>,
    pub color_negative: Option<Color>,
    pub color_axis: Option<Color>,
    pub color_markers: Option<Color>,
    pub color_first: Option<Color>,
    pub color_last: Option<Color>,
    pub color_high: Option<Color>,
    pub color_low: Option<Color>,
    /// 선 굵기 (pt, 기본 0.75)
    pub line_weight: f64,
    pub markers: bool,
    pub high: bool,
    pub low: bool,
    pub first: bool,
    pub last: bool,
    pub negative: bool,
    pub display_x_axis: bool,
    /// 빈 칸: `gap`(기본) / `zero` / `span`
    pub empty_as: String,
    /// 세로축 최소·최대: `individual`(기본) / `group` / `custom`
    pub min_type: String,
    pub max_type: String,
    pub manual_min: Option<f64>,
    pub manual_max: Option<f64>,
    pub right_to_left: bool,
    /// (데이터 범위 `시트!A1:E1`, 그릴 칸)
    pub items: Vec<(String, CellRef)>,
}

/// 그리기에만 쓰는 추가 정보 (모델에 없는 것을 원본에서 다시 읽는다)
#[derive(Debug, Clone, Default)]
pub struct Extras {
    /// 시트 파트 이름 → 스파크라인 그룹
    pub sparklines: HashMap<String, Vec<SparklineGroup>>,
}

impl Extras {
    /// 파일 바이트(XML `.cell`/`.xlsx`)에서 읽는다. 다른 형식이거나 읽지 못하면 비어 있다
    pub fn from_bytes(bytes: &[u8]) -> Extras {
        let mut out = Extras::default();
        if crate::detect(bytes) != crate::FileKind::Zip {
            return out;
        }
        let Ok(mut pkg) = crate::Package::from_bytes(bytes.to_vec()) else {
            return out;
        };
        let parts: Vec<String> = pkg
            .part_names()
            .iter()
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.starts_with("xl/worksheets/") && l.ends_with(".xml")
            })
            .cloned()
            .collect();
        for p in parts {
            let Ok(Some(root)) = pkg.read_xml(&p) else {
                continue;
            };
            let groups = parse_groups(&root);
            if !groups.is_empty() {
                out.sparklines.insert(p, groups);
            }
        }
        out
    }
}

fn parse_groups(root: &Element) -> Vec<SparklineGroup> {
    let mut out = Vec::new();
    let Some(ext_lst) = root.sml_child("extLst") else {
        return out;
    };
    for ext in ext_lst.sml_children("ext") {
        let Some(groups) = ext.child_ns(ns::X14, "sparklineGroups") else {
            continue;
        };
        for g in groups
            .elements()
            .filter(|g| g.is(ns::X14, "sparklineGroup"))
        {
            let c = |n: &str| {
                g.child_ns(ns::X14, n)
                    .map(crate::parser::styles::parse_color)
            };
            let b = |n: &str| g.attr_bool(n).unwrap_or(false);
            let mut sg = SparklineGroup {
                kind: g.attr("type").unwrap_or("line").to_string(),
                color_series: c("colorSeries"),
                color_negative: c("colorNegative"),
                color_axis: c("colorAxis"),
                color_markers: c("colorMarkers"),
                color_first: c("colorFirst"),
                color_last: c("colorLast"),
                color_high: c("colorHigh"),
                color_low: c("colorLow"),
                line_weight: g.attr_f64("lineWeight").unwrap_or(0.75),
                markers: b("markers"),
                high: b("high"),
                low: b("low"),
                first: b("first"),
                last: b("last"),
                negative: b("negative"),
                display_x_axis: b("displayXAxis"),
                empty_as: g.attr("displayEmptyCellsAs").unwrap_or("gap").to_string(),
                min_type: g.attr("minAxisType").unwrap_or("individual").to_string(),
                max_type: g.attr("maxAxisType").unwrap_or("individual").to_string(),
                manual_min: g.attr_f64("manualMin"),
                manual_max: g.attr_f64("manualMax"),
                right_to_left: b("rightToLeft"),
                items: Vec::new(),
            };
            if let Some(list) = g.child_ns(ns::X14, "sparklines") {
                for s in list.elements().filter(|s| s.is(ns::X14, "sparkline")) {
                    let f = s
                        .child_ns(ns::XM, "f")
                        .map(|e| e.text())
                        .unwrap_or_default();
                    let Some(at) = s
                        .child_ns(ns::XM, "sqref")
                        .and_then(|e| CellRef::parse(&e.text()))
                    else {
                        continue;
                    };
                    sg.items.push((f, at));
                }
            }
            out.push(sg);
        }
    }
    out
}

/// `시트!$A$1:$E$1` → 값들 (빈 칸 None). 한 행 또는 한 열
fn range_values(wb: &Workbook, here: usize, f: &str) -> Vec<Option<f64>> {
    let (sheet_name, range) = match f.rsplit_once('!') {
        Some((s, r)) => (Some(s.trim_matches('\'').replace("''", "'")), r),
        None => (None, f),
    };
    let sheet = sheet_name
        .as_deref()
        .and_then(|n| wb.sheet_by_name(n))
        .or_else(|| wb.sheets.get(here));
    let (Some(sheet), Some(r)) = (sheet, crate::model::CellRange::parse(range)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in r.start.row..=r.end.row {
        for col in r.start.col..=r.end.col {
            let v = sheet
                .cell(CellRef { row, col })
                .and_then(|c| match &c.value {
                    CellValue::Number(n) => Some(*n),
                    CellValue::Bool(b) => Some(*b as u8 as f64),
                    _ => None,
                });
            out.push(v);
        }
    }
    out
}

/// 칸 → (그룹 번호, 값)
pub type Items = HashMap<(u32, u32), (usize, Vec<Option<f64>>)>;

/// 시트 하나의 스파크라인: 칸 → (그룹 번호, 값)
pub fn sheet_items(wb: &Workbook, sheet_index: usize, groups: &[SparklineGroup]) -> Items {
    let mut out = HashMap::new();
    for (gi, g) in groups.iter().enumerate() {
        for (f, at) in &g.items {
            out.insert((at.row, at.col), (gi, range_values(wb, sheet_index, f)));
        }
    }
    out
}

/// 그룹 단위 최소·최대 (minAxisType/maxAxisType = group)
pub fn group_range(items: &Items, gi: usize) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for (g, vals) in items.values() {
        if *g == gi {
            for v in vals.iter().flatten() {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
    }
    (lo, hi)
}

fn color(c: &Option<Color>, theme: &Theme, default: &str) -> String {
    c.as_ref()
        .and_then(|c| theme::resolve(c, theme))
        .unwrap_or_else(|| default.into())
}

/// 칸 하나의 스파크라인 SVG (w×h px). 한셀 화면 (스파크라인 실험 — 꺾은선·막대·승패 8개, 행 40px·열 160px):
/// - 점은 칸 폭 − 3 을 값 수로 나눈 칸의 가운데 (왼쪽 1.5px 부터), 막대 폭은 그 칸의 0.65.
/// - 꺾은선은 위·아래 20% 를 띄운다. 막대는 위·아래 2px — 음수가 섞이면 0 이 바닥, 모두 양수면 최솟값이 바닥이고
///   가장 낮은 막대도 높이의 9% (자동 최솟값일 때).
/// - 빈 칸은 span 일 때만 잇고, gap·zero 는 0 으로 그린다 (한셀은 gap 을 0 으로 그린다). 가로 축(displayXAxis)은
///   그리지 않는다. 승패의 0 은 막대 없음.
pub fn svg(
    g: &SparklineGroup,
    vals: &[Option<f64>],
    group_lohi: (f64, f64),
    w: f64,
    h: f64,
    theme: &Theme,
) -> String {
    let mut vals: Vec<Option<f64>> = vals.to_vec();
    if g.empty_as != "span" {
        for v in &mut vals {
            v.get_or_insert(0.0);
        }
    }
    if g.right_to_left {
        vals.reverse();
    }
    let n = vals.len();
    let present: Vec<f64> = vals.iter().flatten().copied().collect();
    if n == 0 || present.is_empty() {
        return String::new();
    }
    let (mut lo, mut hi) = (
        present.iter().copied().fold(f64::INFINITY, f64::min),
        present.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    let auto_min = !matches!(g.min_type.as_str(), "group" | "custom");
    match g.min_type.as_str() {
        "group" => lo = group_lohi.0,
        "custom" => lo = g.manual_min.unwrap_or(lo),
        _ => {}
    }
    match g.max_type.as_str() {
        "group" => hi = group_lohi.1,
        "custom" => hi = g.manual_max.unwrap_or(hi),
        _ => {}
    }
    let series = color(&g.color_series, theme, "#376092");
    let negative = color(&g.color_negative, theme, "#D00000");
    let slot = ((w - 3.0) / n as f64).max(1.0);
    let cx = |i: usize| 1.5 + slot * (i as f64 + 0.5);
    let mut s = format!(
        "<svg class=\"rc-spark\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.1} {h:.1}\">"
    );
    let (imin, imax) = {
        let (mut a, mut b) = (0usize, 0usize);
        for (i, v) in vals.iter().enumerate() {
            if let Some(v) = v {
                if vals[a].is_none_or(|x| *v < x) {
                    a = i;
                }
                if vals[b].is_none_or(|x| *v > x) {
                    b = i;
                }
            }
        }
        (a, b)
    };
    let first = vals.iter().position(Option::is_some).unwrap_or(0);
    let last = vals.iter().rposition(Option::is_some).unwrap_or(0);
    // 점 하나의 강조 색 (높음·낮음·처음·끝·음수 순서는 Excel 과 같다)
    let point_color = |i: usize, v: f64, base: &str| -> String {
        if g.high && i == imax {
            color(&g.color_high, theme, base)
        } else if g.low && i == imin {
            color(&g.color_low, theme, base)
        } else if g.first && i == first {
            color(&g.color_first, theme, base)
        } else if g.last && i == last {
            color(&g.color_last, theme, base)
        } else if g.negative && v < 0.0 {
            negative.clone()
        } else {
            base.to_string()
        }
    };
    match g.kind.as_str() {
        "column" | "stacked" => {
            let bw = (slot * 0.65).max(1.0);
            let win = g.kind == "stacked";
            // 막대 자리: 음수가 섞이면 위 0.1·h, 높이 0.75·h (한셀 화면 — 높이 40px 칸의 −4~6 막대가 위 4px 부터
            // 30px, 1단위 3px). 모두 양수인 막대와 승패는 위·아래 2px
            let (top, bot) = if win || lo >= 0.0 {
                (2.0, (h - 2.0).max(3.0))
            } else {
                // 칸 높이(격자선 포함, h + 1) 기준
                let hh = h + 1.0;
                (hh * 0.1, (hh * 0.85).max(hh * 0.1 + 1.0))
            };
            let ph = bot - top;
            // 바닥: 음수가 섞이면 0, 모두 양수면(자동 최솟값) 최솟값 — 가장 낮은 막대도 9%
            let floor_min = !win && auto_min && lo > 0.0;
            let (lo2, hi2) = if win {
                (-1.0, 1.0)
            } else if floor_min {
                (lo, hi)
            } else {
                (lo.min(0.0), hi.max(0.0))
            };
            let span = (hi2 - lo2).max(f64::EPSILON);
            let hmin = if floor_min { ph * 0.09 } else { 0.0 };
            let y = |v: f64| top + (hi2 - v) / span * (ph - hmin);
            let base = if floor_min {
                bot
            } else {
                y(0.0f64.clamp(lo2, hi2))
            };
            for (i, v) in vals.iter().enumerate() {
                let Some(v) = *v else { continue };
                // 0 은 막대 없음 (승패도, 빈 칸을 0 으로 채운 것도)
                if v == 0.0 && !floor_min {
                    continue;
                }
                let v = if win { v.signum() } else { v };
                let x = cx(i) - bw / 2.0;
                let (y0, y1) = if win {
                    if v > 0.0 {
                        (top, top + ph / 2.0 - 0.5)
                    } else {
                        (top + ph / 2.0 + 0.5, bot)
                    }
                } else {
                    let yv = y(v);
                    (yv.min(base), yv.max(base).max(yv.min(base) + 1.0))
                };
                let c = point_color(
                    i,
                    v,
                    if v < 0.0 && g.negative {
                        &negative
                    } else {
                        &series
                    },
                );
                let _ = write!(
                    s,
                    "<rect x=\"{x:.1}\" y=\"{y0:.1}\" width=\"{bw:.1}\" height=\"{:.1}\" fill=\"{c}\"/>",
                    (y1 - y0).max(1.0)
                );
            }
        }
        _ => {
            let span = (hi - lo).max(f64::EPSILON);
            let (py0, ph) = (h * 0.2, (h * 0.6).max(1.0));
            let xy = |i: usize, v: f64| {
                let y = if hi == lo {
                    py0 + ph / 2.0
                } else {
                    py0 + (hi - v) / span * ph
                };
                (cx(i), y)
            };
            let lw = (g.line_weight * 96.0 / 72.0).max(0.75);
            // 빈 칸: span 이면 잇는다 (나머지는 위에서 0 으로 채웠다)
            let mut d = String::new();
            let mut pen = false;
            for (i, v) in vals.iter().enumerate() {
                if let Some(v) = v {
                    let (x, y) = xy(i, *v);
                    let _ = write!(d, "{}{x:.1},{y:.1}", if pen { "L" } else { "M" });
                    pen = true;
                }
            }
            let _ = write!(
                s,
                "<path d=\"{d}\" fill=\"none\" stroke=\"{series}\" stroke-width=\"{lw:.2}\" stroke-linejoin=\"round\"/>"
            );
            for (i, v) in vals.iter().enumerate() {
                let Some(v) = *v else { continue };
                let c = point_color(i, v, "");
                let show = g.markers || !c.is_empty();
                if !show {
                    continue;
                }
                let c = if c.is_empty() {
                    color(&g.color_markers, theme, &series)
                } else {
                    c
                };
                let (x, y) = xy(i, v);
                let _ = write!(
                    s,
                    "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"3\" height=\"3\" fill=\"{c}\"/>",
                    x - 1.5,
                    y - 1.5
                );
            }
        }
    }
    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_draws_groups() {
        let xml = r#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetData/><x:extLst><x:ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklineGroup type="column" negative="1" displayEmptyCellsAs="gap"><x14:colorSeries rgb="FF376092"/><x14:colorNegative rgb="FFD00000"/><x14:sparklines><x14:sparkline><xm:f>Sheet1!A1:D1</xm:f><xm:sqref>E1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></x:ext></x:extLst></x:worksheet>"#;
        let root = crate::xml::parse(xml, "sheet1.xml").unwrap();
        let g = parse_groups(&root);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].kind, "column");
        assert_eq!(g[0].items[0].1, CellRef::parse("E1").unwrap());
        let s = svg(
            &g[0],
            &[Some(1.0), Some(-2.0), None, Some(3.0)],
            (0.0, 0.0),
            72.0,
            20.0,
            &Theme::default(),
        );
        assert_eq!(s.matches("<rect").count(), 3);
        assert!(s.contains("#D00000"));
    }
}
