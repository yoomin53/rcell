// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 표(ListObject)의 스타일을 칸마다 덧입힌다 (한셀 화면 — 표 스타일 실험, docs/format/screen.md 5절).
//!
//! - 사용자 정의 스타일(`x:tableStyles/x:tableStyle`)은 요소마다 차등 서식(dxf)의 채우기·글꼴 색·굵게·테두리.
//!   요소는 전체 → 열 줄무늬 → 행 줄무늬 → 마지막 열 → 첫 열 → 머리글 행 → 요약 행 → 머리글·요약의 첫·끝 칸
//!   순서로 덮는다. 테두리의 왼·오른·위·아래는 그 요소 자리의 바깥, 가로·세로(`horizontal`·`vertical`)는 안쪽 선
//! - 기본 스타일 60 개 (밝게 1–21, 보통 1–28, 어둡게 1–11)는 계열마다 dk1·테마 강조색으로 지어 쓴다 ([`builtin`],
//!   표 스타일 실험). 그 밖의 이름은 그리지 않는다
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
    /// 표 크기 조절 손잡이 (표의 마지막 칸 오른쪽 아래)
    pub handle: bool,
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

/// HSL 밝기 L → L·k + off (한셀 화면의 표 색: 0.8 밝게 = L·0.2 + 0.8 — 289B6E → CDF2E4, 6182D6 → DFE6F7.
/// RGB 직선 섞기는 289B6E 에서 D4EBE2 로 어긋난다)
fn hsl_l(hex: &str, k: f64, off: f64) -> String {
    let h = hex.trim_start_matches('#');
    let v = u32::from_str_radix(h.get(..6).unwrap_or("000000"), 16).unwrap_or(0);
    let rgb = (
        f64::from((v >> 16) & 0xff),
        f64::from((v >> 8) & 0xff),
        f64::from(v & 0xff),
    );
    let (hh, s, l) = super::draw::rgb_to_hsl(rgb);
    let (r, g, b) = super::draw::hsl_to_rgb(hh, s, (l * k + off).clamp(0.0, 1.0));
    let c = |x: f64| x.round().clamp(0.0, 255.0) as u32;
    format!("#{:02X}{:02X}{:02X}", c(r), c(g), c(b))
}

/// 흰색 쪽으로 t 만큼 (HSL 밝기)
fn lighten(hex: &str, t: f64) -> String {
    hsl_l(hex, 1.0 - t, t)
}

/// HSL 밝기를 k 배로 (한셀 화면: 밝게 2 의 글자 6182D6 → 3057B9 는 0.75, 어둡게 2 의 요약 행 203A7B 는 0.5)
fn lum_mod(hex: &str, k: f64) -> String {
    hsl_l(hex, k, 0.0)
}

fn line(style: &str, color: &str) -> Option<Edge> {
    Some(Edge {
        style: style.into(),
        color: color.into(),
    })
}

/// 기본 스타일 이름 → 요소들 (지은 것). 모르는 이름은 None.
///
/// 표 스타일 실험 (밝게 1–21·보통 1–28·어둡게 1–11 × 머리글·요약·첫·끝 열·행·열 줄무늬): 계열마다 0 번째는 글자색(dk1)
/// 중립, 1–6 번째는 테마 강조 1–6. 중립의 옅은 색은 검정의 0.85·0.65 밝게 (D9D9D9·A6A6A6), 강조는 0.8·0.6·0.4 밝게
/// (RGB 직선 섞기), 진한 색은 HSL 밝기 × 0.75·0.5. 어둡게 8–11 은 강조 두 개 (줄무늬 1·3·5, 머리글 0·2·4·6)
fn builtin(name: &str, theme: &Theme) -> Option<Vec<(String, u32, Look)>> {
    let (family, n) = if let Some(n) = name.strip_prefix("TableStyleMedium") {
        ("medium", n)
    } else if let Some(n) = name.strip_prefix("TableStyleLight") {
        ("light", n)
    } else {
        ("dark", name.strip_prefix("TableStyleDark")?)
    };
    let n = n.parse::<u32>().ok()?;
    let hex = |i: usize| {
        theme
            .colors
            .get(i)
            .filter(|c| c.len() >= 6)
            .map(|c| format!("#{}", &c[c.len() - 6..]))
    };
    // 테마 색 차례: lt1, dk1, lt2, dk2, 강조 1–6
    let dk = hex(1).unwrap_or_else(|| "#000000".into());
    let accent = |k: u32| hex(4 + k as usize).unwrap_or_else(|| "#4472C4".into());
    let (fam, k) = match (family, n) {
        ("light", 1..=7) => ("L1", n - 1),
        ("light", 8..=14) => ("L8", n - 8),
        ("light", 15..=21) => ("L15", n - 15),
        ("medium", 1..=7) => ("M1", n - 1),
        ("medium", 8..=14) => ("M8", n - 8),
        ("medium", 15..=21) => ("M15", n - 15),
        ("medium", 22..=28) => ("M22", n - 22),
        ("dark", 1..=7) => ("D1", n - 1),
        ("dark", 8..=11) => ("D8", n - 8),
        _ => return None,
    };
    let neutral = k == 0;
    let a = if neutral { dk.clone() } else { accent(k - 1) };
    let t80 = |c: &str, neutral: bool| {
        if neutral {
            lighten(&dk, 0.85)
        } else {
            lighten(c, 0.8)
        }
    };
    let t60 = |c: &str, neutral: bool| {
        if neutral {
            lighten(&dk, 0.65)
        } else {
            lighten(c, 0.6)
        }
    };
    let white = "#FFFFFF";
    let el = |k: &str, l: Look| (k.to_string(), 1u32, l);
    let fill = |c: String| Look {
        fill: Some(c),
        ..Default::default()
    };
    let bold = || Look {
        bold: true,
        ..Default::default()
    };
    let header = |c: &str| Look {
        fill: Some(c.into()),
        font_color: Some(white.into()),
        bold: true,
        ..Default::default()
    };
    let v = match fam {
        "L1" => vec![
            el(
                "wholeTable",
                Look {
                    font_color: Some(lum_mod(&a, 0.75)),
                    top: line("thin", &a),
                    bottom: line("thin", &a),
                    ..Default::default()
                },
            ),
            el("firstColumnStripe", fill(t80(&a, neutral))),
            el("firstRowStripe", fill(t80(&a, neutral))),
            el("lastColumn", bold()),
            el("firstColumn", bold()),
            el(
                "headerRow",
                Look {
                    bottom: line("thin", &a),
                    ..bold()
                },
            ),
            el(
                "totalRow",
                Look {
                    top: line("thin", &a),
                    ..bold()
                },
            ),
        ],
        "L8" => vec![
            el(
                "wholeTable",
                Look {
                    left: line("thin", &a),
                    right: line("thin", &a),
                    top: line("thin", &a),
                    bottom: line("thin", &a),
                    ..Default::default()
                },
            ),
            el(
                "firstColumnStripe",
                Look {
                    left: line("thin", &a),
                    right: line("thin", &a),
                    ..Default::default()
                },
            ),
            el(
                "firstRowStripe",
                Look {
                    top: line("thin", &a),
                    bottom: line("thin", &a),
                    ..Default::default()
                },
            ),
            el("lastColumn", bold()),
            el("firstColumn", bold()),
            el("headerRow", header(&a)),
            el(
                "totalRow",
                Look {
                    top: line("double", &a),
                    ..bold()
                },
            ),
        ],
        "L15" => vec![
            el(
                "wholeTable",
                Look {
                    left: line("thin", &a),
                    right: line("thin", &a),
                    top: line("thin", &a),
                    bottom: line("thin", &a),
                    horizontal: line("thin", &a),
                    vertical: line("thin", &a),
                    ..Default::default()
                },
            ),
            el("firstColumnStripe", fill(t80(&a, neutral))),
            el("firstRowStripe", fill(t80(&a, neutral))),
            el("lastColumn", bold()),
            el("firstColumn", bold()),
            el(
                "headerRow",
                Look {
                    bottom: line("medium", &a),
                    ..bold()
                },
            ),
            el(
                "totalRow",
                Look {
                    top: line("double", &a),
                    ..bold()
                },
            ),
        ],
        "M1" => {
            let l = if neutral {
                dk.clone()
            } else {
                lighten(&a, 0.4)
            };
            vec![
                el(
                    "wholeTable",
                    Look {
                        left: line("thin", &l),
                        right: line("thin", &l),
                        top: line("thin", &l),
                        bottom: line("thin", &l),
                        horizontal: line("thin", &l),
                        ..Default::default()
                    },
                ),
                el("firstColumnStripe", fill(t80(&a, neutral))),
                el("firstRowStripe", fill(t80(&a, neutral))),
                el("lastColumn", bold()),
                el("firstColumn", bold()),
                el("headerRow", header(&a)),
                el(
                    "totalRow",
                    Look {
                        top: line("double", &a),
                        ..bold()
                    },
                ),
            ]
        }
        "M8" => vec![
            el(
                "wholeTable",
                Look {
                    fill: Some(t80(&a, neutral)),
                    horizontal: line("thin", white),
                    vertical: line("thin", white),
                    ..Default::default()
                },
            ),
            el("firstColumnStripe", fill(t60(&a, neutral))),
            el("firstRowStripe", fill(t60(&a, neutral))),
            el("lastColumn", header(&a)),
            el("firstColumn", header(&a)),
            el(
                "headerRow",
                Look {
                    bottom: line("thick", white),
                    ..header(&a)
                },
            ),
            el(
                "totalRow",
                Look {
                    top: line("thick", white),
                    ..header(&a)
                },
            ),
        ],
        "M15" => {
            let inner = |s: &str| if neutral { line(s, &dk) } else { None };
            vec![
                el(
                    "wholeTable",
                    Look {
                        top: line("medium", &dk),
                        bottom: line("medium", &dk),
                        left: inner("thin"),
                        right: inner("thin"),
                        horizontal: inner("thin"),
                        vertical: inner("thin"),
                        ..Default::default()
                    },
                ),
                el("firstColumnStripe", fill(lighten(&dk, 0.85))),
                el("firstRowStripe", fill(lighten(&dk, 0.85))),
                // 첫 열 오른쪽·마지막 열 왼쪽의 격자선 자리도 강조색 (줄무늬 칸 옆)
                el(
                    "lastColumn",
                    Look {
                        left: line("thin", &a),
                        ..header(&a)
                    },
                ),
                el(
                    "firstColumn",
                    Look {
                        right: line("thin", &a),
                        ..header(&a)
                    },
                ),
                el(
                    "headerRow",
                    Look {
                        bottom: line("medium", &dk),
                        ..header(&a)
                    },
                ),
                el(
                    "totalRow",
                    Look {
                        top: line("double", &dk),
                        ..Default::default()
                    },
                ),
            ]
        }
        "M22" => {
            let l = if neutral {
                dk.clone()
            } else {
                lighten(&a, 0.4)
            };
            vec![
                el(
                    "wholeTable",
                    Look {
                        fill: Some(t80(&a, neutral)),
                        left: line("thin", &l),
                        right: line("thin", &l),
                        top: line("thin", &l),
                        bottom: line("thin", &l),
                        horizontal: line("thin", &l),
                        vertical: line("thin", &l),
                        ..Default::default()
                    },
                ),
                el("firstColumnStripe", fill(t60(&a, neutral))),
                el("firstRowStripe", fill(t60(&a, neutral))),
                el("lastColumn", bold()),
                el("firstColumn", bold()),
                el("headerRow", bold()),
                el(
                    "totalRow",
                    Look {
                        top: line("medium", &a),
                        ..bold()
                    },
                ),
            ]
        }
        "D1" => {
            let (whole, stripe, total) = if neutral {
                (lighten(&dk, 0.45), lighten(&dk, 0.25), lighten(&dk, 0.15))
            } else {
                (a.clone(), lum_mod(&a, 0.75), lum_mod(&a, 0.5))
            };
            vec![
                el(
                    "wholeTable",
                    Look {
                        fill: Some(whole),
                        font_color: Some(white.into()),
                        ..Default::default()
                    },
                ),
                el("firstColumnStripe", fill(stripe.clone())),
                el("firstRowStripe", fill(stripe.clone())),
                el(
                    "lastColumn",
                    Look {
                        left: line("medium", white),
                        ..header(&stripe)
                    },
                ),
                el(
                    "firstColumn",
                    Look {
                        right: line("medium", white),
                        ..header(&stripe)
                    },
                ),
                el(
                    "headerRow",
                    Look {
                        bottom: line("medium", white),
                        ..header(&dk)
                    },
                ),
                el(
                    "totalRow",
                    Look {
                        top: line("medium", white),
                        ..header(&total)
                    },
                ),
            ]
        }
        _ => {
            // 어둡게 8–11: 줄무늬는 강조 2k−1 (8 은 중립), 머리글은 강조 2k (8 은 dk1)
            let base = if neutral {
                dk.clone()
            } else {
                accent(2 * k - 2)
            };
            let head = if neutral {
                dk.clone()
            } else {
                accent(2 * k - 1)
            };
            vec![
                el("wholeTable", fill(t80(&base, neutral))),
                el("firstColumnStripe", fill(t60(&base, neutral))),
                el("firstRowStripe", fill(t60(&base, neutral))),
                el("lastColumn", bold()),
                el("firstColumn", bold()),
                el("headerRow", header(&head)),
                el(
                    "totalRow",
                    Look {
                        top: line("double", &dk),
                        ..bold()
                    },
                ),
            ]
        }
    };
    Some(v)
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
            // 첫 열·마지막 열은 머리글·요약 행까지 (한셀 화면: 보통 16 의 요약 행 첫 칸이 강조색, 어둡게 2 의 머리글에도
            // 첫 열 오른쪽 흰 선 — 머리글·요약 행 요소가 뒤에서 덮는다)
            "firstColumn" if c == c0 => Some((r0, r1, c0, c0)),
            "lastColumn" if c == c1 => Some((r0, r1, c1, c1)),
            "firstRowStripe" | "secondRowStripe" if (data0..=data1).contains(&r) => {
                let i = r - data0;
                stripe(i, k == "firstRowStripe").then(|| {
                    let b0 = data0 + i / size * size;
                    (b0, (b0 + size - 1).min(data1), c0, c1)
                })
            }
            // 열 줄무늬는 머리글 행에도 (한셀 화면 — 채우기가 없는 머리글에 줄무늬 색)
            "firstColumnStripe" | "secondColumnStripe" if (r0..=data1).contains(&r) => {
                let j = c - c0;
                stripe(j, k == "firstColumnStripe").then(|| {
                    let b0 = c0 + j / size * size;
                    (r0, data1, b0, (b0 + size - 1).min(c1))
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
                handle: r == r1 && c == c1,
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

/// 표 크기 조절 손잡이: 마지막 칸 안쪽 오른쪽 아래 4×4 의 ㄴ자 (위 두 줄은 오른쪽 2px) — 4F81DD (표 스타일 실험)
pub(super) fn handle_svg(w: u32, h: u32) -> String {
    // 칸 상자 w×h 의 오른쪽·아래 끝 픽셀이 격자선 자리
    format!(
        "<svg class=\"rc-bar\" width=\"{w}\" height=\"{h}\" shape-rendering=\"crispEdges\"><path d=\"M{} {}h2v4h-4v-2h2z\" fill=\"#4F81DD\"/></svg>",
        w as i64 - 3,
        h as i64 - 5
    )
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
