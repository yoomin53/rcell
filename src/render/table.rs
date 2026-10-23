// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 표(ListObject)의 스타일을 칸마다 덧입힌다 (한셀 화면 — 표 스타일 실험, docs/format/screen.md 5절).
//!
//! - 사용자 정의 스타일(`x:tableStyles/x:tableStyle`)은 요소마다 차등 서식(dxf)의 채우기·글꼴 색·굵게·테두리.
//!   요소는 전체 → 열 줄무늬 → 행 줄무늬 → 마지막 열 → 첫 열 → 머리글 행 → 요약 행 → 머리글·요약의 첫·끝 칸
//!   순서로 덮는다. 테두리의 왼·오른·위·아래는 그 요소 자리의 바깥, 가로·세로(`horizontal`·`vertical`)는 안쪽 선
//! - 기본 스타일 이름 중 보통 2~7, 밝게 9~14, 보통 9~14 는 강조색(테마 강조 1~6)으로 지어 쓴다 (한셀 화면 대조:
//!   보통 2 는 머리글 강조색·흰 굵은 글, 첫 줄무늬 강조색 0.8 밝게, 가로선 0.4 밝게 / 밝게 9 는 머리글 강조색, 가로선
//!   강조색 / 보통 9 는 전체 0.8 밝게, 줄무늬 0.6 밝게, 안쪽 흰 선). 그 밖의 이름은 그리지 않는다
//! - 칸 자체의 채우기·테두리·글꼴(서식 0 이 아닌 것)이 이긴다
//! - 머리글 칸의 자동 필터 단추: 오른쪽 2px·아래 2px 에 16×16 (테두리 486C97·안쪽 B3D0F2·흰 바탕·검은 세모)

use std::collections::HashMap;

use super::paint::{CellBorders, Edge};
use crate::model::{CellRange, Dxf, Sheet, Styles, Table, TableStyleElement};
use crate::theme::{self, Theme};

/// 칸 하나에 덧입힐 표 서식
#[derive(Debug, Clone, Default)]
pub(super) struct TableLook {
    pub fill: Option<String>,
    pub font_color: Option<String>,
    pub bold: bool,
    pub borders: CellBorders,
    /// 자동 필터 단추를 그린다 (머리글 칸)
    pub button: bool,
}

/// 요소 하나의 서식 (사용자 정의는 dxf 에서, 기본 스타일은 지은 것)
#[derive(Debug, Clone, Default)]
struct Look {
    fill: Option<String>,
    font_color: Option<String>,
    bold: bool,
    left: Option<Edge>,
    right: Option<Edge>,
    top: Option<Edge>,
    bottom: Option<Edge>,
    horizontal: Option<Edge>,
    vertical: Option<Edge>,
}

fn thin(color: &str) -> Option<Edge> {
    Some(Edge {
        style: "thin".into(),
        color: color.into(),
    })
}

/// dxf → 요소 서식. 단색 채우기는 bgColor (차등 서식의 관례), 없으면 fgColor
fn look_of(d: &Dxf, theme: &Theme) -> Look {
    let col = |c: Option<&crate::model::Color>| c.and_then(|c| theme::resolve(c, theme));
    let fill = d
        .fill
        .as_ref()
        .filter(|f| f.pattern_type.as_deref().is_none_or(|p| p != "none"))
        .and_then(|f| col(f.bg_color.as_ref()).or_else(|| col(f.fg_color.as_ref())));
    let font_color = d.font.as_ref().and_then(|f| col(f.color.as_ref()));
    let bold = d.font.as_ref().is_some_and(|f| f.bold);
    let e = |x: Option<&crate::model::BorderEdge>| super::paint::edge(x, theme);
    let b = d.border.as_ref();
    Look {
        fill,
        font_color,
        bold,
        left: e(b.and_then(|b| b.left.as_ref())),
        right: e(b.and_then(|b| b.right.as_ref())),
        top: e(b.and_then(|b| b.top.as_ref())),
        bottom: e(b.and_then(|b| b.bottom.as_ref())),
        horizontal: e(b.and_then(|b| b.horizontal.as_ref())),
        vertical: e(b.and_then(|b| b.vertical.as_ref())),
    }
}

/// 강조색을 흰색 쪽으로 t 만큼 (한셀 화면의 표 색은 RGB 직선 섞기: 6182D6 의 0.8 → DFE6F7)
fn lighten(hex: &str, t: f64) -> String {
    let h = hex.trim_start_matches('#');
    let v = u32::from_str_radix(h.get(..6).unwrap_or("000000"), 16).unwrap_or(0);
    let f = |s: u32| {
        let c = f64::from((v >> s) & 0xff);
        (c + (255.0 - c) * t).round().clamp(0.0, 255.0) as u32
    };
    format!("#{:02X}{:02X}{:02X}", f(16), f(8), f(0))
}

/// 기본 스타일 이름 → 요소들 (지은 것). 모르는 이름은 None
fn builtin(name: &str, theme: &Theme) -> Option<Vec<(String, u32, Look)>> {
    let (family, n) = match name.strip_prefix("TableStyleMedium") {
        Some(n) => ("medium", n),
        None => ("light", name.strip_prefix("TableStyleLight")?),
    };
    let n = n.parse::<u32>().ok()?;
    let accent = |k: u32| {
        theme
            .colors
            .get(4 + k as usize)
            .map(|c| format!("#{}", &c[c.len().saturating_sub(6)..]))
            .unwrap_or_else(|| "#4472C4".into())
    };
    let header = |a: &str| Look {
        fill: Some(a.into()),
        font_color: Some("#FFFFFF".into()),
        bold: true,
        ..Default::default()
    };
    let el = |k: &str, l: Look| (k.to_string(), 1u32, l);
    match (family, n) {
        ("medium", 2..=7) => {
            let a = accent(n - 2);
            let line = lighten(&a, 0.4);
            Some(vec![
                el(
                    "wholeTable",
                    Look {
                        top: thin(&line),
                        bottom: thin(&line),
                        horizontal: thin(&line),
                        ..Default::default()
                    },
                ),
                el(
                    "firstRowStripe",
                    Look {
                        fill: Some(lighten(&a, 0.8)),
                        ..Default::default()
                    },
                ),
                el("headerRow", header(&a)),
            ])
        }
        ("light", 9..=14) => {
            let a = accent(n - 9);
            Some(vec![
                el(
                    "wholeTable",
                    Look {
                        left: thin(&a),
                        right: thin(&a),
                        top: thin(&a),
                        bottom: thin(&a),
                        horizontal: thin(&a),
                        ..Default::default()
                    },
                ),
                el("headerRow", header(&a)),
            ])
        }
        ("medium", 9..=14) => {
            let a = accent(n - 9);
            let stripe = Look {
                fill: Some(lighten(&a, 0.6)),
                ..Default::default()
            };
            Some(vec![
                el(
                    "wholeTable",
                    Look {
                        fill: Some(lighten(&a, 0.8)),
                        horizontal: thin("#FFFFFF"),
                        vertical: thin("#FFFFFF"),
                        ..Default::default()
                    },
                ),
                el("firstColumnStripe", stripe.clone()),
                el("firstRowStripe", stripe),
                el("headerRow", header(&a)),
            ])
        }
        _ => None,
    }
}

/// 요소의 차례 (뒤가 앞을 덮는다)
const ORDER: [&str; 13] = [
    "wholeTable",
    "firstColumnStripe",
    "secondColumnStripe",
    "firstRowStripe",
    "secondRowStripe",
    "lastColumn",
    "firstColumn",
    "headerRow",
    "totalRow",
    "firstHeaderCell",
    "lastHeaderCell",
    "firstTotalCell",
    "lastTotalCell",
];

/// 시트의 표마다 칸 → 덧입힐 서식
pub(super) fn sheet_looks(
    sheet: &Sheet,
    styles: &Styles,
    theme: &Theme,
) -> HashMap<(u32, u32), TableLook> {
    let mut out = HashMap::new();
    for t in &sheet.tables {
        table_looks(t, styles, theme, &mut out);
    }
    out
}

fn table_looks(
    t: &Table,
    styles: &Styles,
    theme: &Theme,
    out: &mut HashMap<(u32, u32), TableLook>,
) {
    let Some(rg) = CellRange::parse(&t.range) else {
        return;
    };
    let info = t.style.as_ref();
    let name = info.and_then(|s| s.name.as_deref()).unwrap_or("");
    // 요소: 사용자 정의 → 기본 스타일
    let custom = styles
        .table_styles
        .as_ref()
        .and_then(|ts| ts.styles.iter().find(|s| s.name == name));
    let elements: Vec<(String, u32, Look)> = match custom {
        Some(s) => s
            .elements
            .iter()
            .filter_map(|e: &TableStyleElement| {
                let d = styles.dxfs.get(e.dxf_id? as usize)?;
                Some((e.kind.clone(), e.size.max(1), look_of(d, theme)))
            })
            .collect(),
        None => builtin(name, theme).unwrap_or_default(),
    };
    let (r0, r1, c0, c1) = (rg.start.row, rg.end.row, rg.start.col, rg.end.col);
    let hdr = t.header_rows.min(r1 - r0 + 1);
    let tot = t.totals_rows;
    let data0 = r0 + hdr;
    let data1 = r1.saturating_sub(tot);
    let on = |k: &str| -> bool {
        match k {
            "firstRowStripe" | "secondRowStripe" => info.is_some_and(|s| s.row_stripes),
            "firstColumnStripe" | "secondColumnStripe" => info.is_some_and(|s| s.column_stripes),
            "firstColumn" => info.is_some_and(|s| s.first_column),
            "lastColumn" => info.is_some_and(|s| s.last_column),
            _ => true,
        }
    };
    // 요소의 자리 (행 범위, 열 범위) — 칸 (r, c) 가 들어 있으면
    let region = |k: &str, size: u32, r: u32, c: u32| -> Option<(u32, u32, u32, u32)> {
        let stripe = |i: u32, first: bool| {
            let band = i / size;
            band.is_multiple_of(2) == first
        };
        match k {
            "wholeTable" => Some((r0, r1, c0, c1)),
            "headerRow" if hdr > 0 && r < data0 => Some((r0, data0 - 1, c0, c1)),
            "totalRow" if tot > 0 && r > data1 => Some((data1 + 1, r1, c0, c1)),
            "firstColumn" if (data0..=data1).contains(&r) && c == c0 => {
                Some((data0, data1, c0, c0))
            }
            "lastColumn" if (data0..=data1).contains(&r) && c == c1 => Some((data0, data1, c1, c1)),
            "firstRowStripe" | "secondRowStripe" if (data0..=data1).contains(&r) => {
                let i = r - data0;
                stripe(i, k == "firstRowStripe").then(|| {
                    let b0 = data0 + i / size * size;
                    (b0, (b0 + size - 1).min(data1), c0, c1)
                })
            }
            "firstColumnStripe" | "secondColumnStripe" if (data0..=data1).contains(&r) => {
                let j = c - c0;
                stripe(j, k == "firstColumnStripe").then(|| {
                    let b0 = c0 + j / size * size;
                    (data0, data1, b0, (b0 + size - 1).min(c1))
                })
            }
            "firstHeaderCell" if hdr > 0 && r < data0 && c == c0 => Some((r0, data0 - 1, c0, c0)),
            "lastHeaderCell" if hdr > 0 && r < data0 && c == c1 => Some((r0, data0 - 1, c1, c1)),
            "firstTotalCell" if tot > 0 && r > data1 && c == c0 => Some((data1 + 1, r1, c0, c0)),
            "lastTotalCell" if tot > 0 && r > data1 && c == c1 => Some((data1 + 1, r1, c1, c1)),
            _ => None,
        }
    };
    for r in r0..=r1 {
        for c in c0..=c1 {
            let mut tl = TableLook {
                button: hdr > 0 && r == r0 && t.auto_filter.is_some(),
                ..Default::default()
            };
            for k in ORDER {
                for (_, size, l) in elements.iter().filter(|e| e.0 == k) {
                    if !on(k) {
                        continue;
                    }
                    let Some((ra, rb, ca, cb)) = region(k, *size, r, c) else {
                        continue;
                    };
                    if let Some(f) = &l.fill {
                        tl.fill = Some(f.clone());
                    }
                    if let Some(f) = &l.font_color {
                        tl.font_color = Some(f.clone());
                    }
                    if l.bold {
                        tl.bold = true;
                    }
                    let pick = |outer: &Option<Edge>, inner: &Option<Edge>, at_edge: bool| {
                        if at_edge {
                            outer.clone()
                        } else {
                            inner.clone()
                        }
                    };
                    let set = |slot: &mut Option<Edge>, v: Option<Edge>| {
                        if v.is_some() {
                            *slot = v;
                        }
                    };
                    set(&mut tl.borders.top, pick(&l.top, &l.horizontal, r == ra));
                    set(
                        &mut tl.borders.bottom,
                        pick(&l.bottom, &l.horizontal, r == rb),
                    );
                    set(&mut tl.borders.left, pick(&l.left, &l.vertical, c == ca));
                    set(&mut tl.borders.right, pick(&l.right, &l.vertical, c == cb));
                }
            }
            out.insert((r, c), tl);
        }
    }
}

/// 자동 필터 단추 (칸 안 SVG, 칸 폭·높이 px)
pub(super) fn button_svg(w: u32, h: u32) -> String {
    let (x, y) = (w as f64 - 18.0, (h as f64 - 18.0).max(0.0));
    format!(
        "<svg class=\"rc-bar\" width=\"{w}\" height=\"{h}\"><rect x=\"{:.1}\" y=\"{:.1}\" width=\"15\" height=\"15\" rx=\"1.5\" fill=\"#FFFFFF\" stroke=\"#486C97\" stroke-width=\"1\"/><rect x=\"{:.1}\" y=\"{:.1}\" width=\"13\" height=\"13\" fill=\"none\" stroke=\"#B3D0F2\" stroke-width=\"1\"/><path d=\"M{:.1},{:.1}h7l-3.5,4z\" fill=\"#000000\"/></svg>",
        x + 0.5,
        y + 0.5,
        x + 1.5,
        y + 1.5,
        x + 5.0,
        y + 5.0
    )
}
