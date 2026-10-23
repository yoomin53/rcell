// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 차트 모델 → 한컴 차트 레코드 0x7EF0 (한셀이 XML 차트를 레거시로 저장하는 꼴) — [`super::vtchart_map`] 의 반대.
//!
//! 한셀은 차트마다 같은 꼴의 Vt 객체 나무(뿌리 VtChart, 객체 400개 남짓)를 쓰고 모델에 따라 값만 바꾼다
//! (차트 표본 77개의 나무가 클래스·포인터 배치가 같다 — docs/format/cell-legacy-chart.md 10절). 여기 기본값은 그
//! 표본에서 늘 같은 값이고, 모델에 따라 바뀌는 값은 표본끼리의 대조로 정했다. 같이 쓰는 객체는 둘 뿐이다: 기본
//! 글꼴 이름 문자열(첫 VtFont 가 만들고 나머지 기본 글꼴이 가리킨다)과 계열 이름(계열 이름표·글 상자가 가리킨다).

use super::vtwrite::{W, WGroup, WObj, write_archive};
use crate::model::{Chart, ChartFill, ChartSeries};

/// 회색 (무늬·테두리·그림자)
const GRAY: u32 = 0x8C8C8C;
const WHITE: u32 = 0xFFFFFF;
/// 기본 계열 색 (docs/format/cell-legacy-chart.md 6절)
const PALETTE: [u32; 8] = [
    0x73B1D1, 0xCD6565, 0xEBC452, 0x90C968, 0x7581C8, 0xE585B0, 0x8FC1DA, 0xD78484,
];
/// 기본 글꼴 이름 (각주 글꼴이 만들고 다른 기본 글꼴이 같이 쓴다)
const DEFAULT_FACE: &str = "함초롬돋움";
/// 축 이름표 글꼴 (축마다 새 문자열)
const LABEL_FACE: &str = "돋움";
/// 계열 종류 표 (27개, 늘 같다)
const TYPE_TABLE: [u16; 27] = [
    1, 5, 6, 7, 8, 9, 10, 0, 1, 2, 3, 4, 23, 24, 25, 11, 12, 13, 15, 16, 21, 22, 32, 33, 34, 34, 5,
];

// ---------------------------------------------------------------------------
// 조각
// ---------------------------------------------------------------------------

/// 선: u16_a 1, 한계 3.0
fn pen(style: u16, u16_c: u16, color: u32, flags: u8, width: f32) -> W {
    WGroup::new()
        .u16("u16_a", 1)
        .u16("style", style)
        .u16("u16_c", u16_c)
        .set("color", W::Color(color, flags))
        .f32("width", width)
        .f32("limit", 3.0)
        .into_w()
}

/// 칠: 무늬 색은 늘 회색
fn brush(style: u16, fill: u32) -> WGroup {
    WGroup::new()
        .u16("style", style)
        .color("fill", fill)
        .color("pattern", GRAY)
}

fn picture() -> WObj {
    WObj::new("VtPicture").u16("u16_a", 1).u16("u16_b", 1)
}

fn fill() -> WObj {
    WObj::new("VtFill")
        .u16("style", 1)
        .set("brush", brush(1, WHITE).into_w())
        .set(
            "gradient",
            WGroup::new()
                .u16("style", 1)
                .color("from", WHITE)
                .color("to", 0x80C0FF)
                .into_w(),
        )
        .ptr("picture", picture())
}

/// 배경: 그림자·테두리·채우기. `u16_c` 는 차트 영역만 1
fn backdrop(u16_c: u16) -> WObj {
    WObj::new("VtBackdrop")
        .u16("u16_a", 1)
        .set(
            "shadow",
            WGroup::new()
                .u16("style", 1)
                .group("brush", brush(1, GRAY))
                .f32("offsetX", 100.0)
                .f32("offsetY", 100.0)
                .into_w(),
        )
        .set(
            "frame",
            WGroup::new()
                .u16("style", 1)
                .color("color", GRAY)
                .color("spaceColor", WHITE)
                .f32("width", 20.0)
                .into_w(),
        )
        .u16("u16_c", u16_c)
        .ptr("fill", fill())
}

fn string(s: &str) -> WObj {
    WObj::new("VtString").set("text", W::Str(s.to_string()))
}

/// 기본 글꼴 이름 (같이 쓰는 문자열)
fn default_face() -> W {
    W::Shared("$face".into(), Box::new(string(DEFAULT_FACE)))
}

fn font(name: W, size: f32, style: u32) -> WObj {
    WObj::new("VtFont")
        .set("name", name)
        .f32("size", size)
        .u32("style", style)
}

/// 글 상자의 공통 값
struct Tb {
    /// 사용자 글 (보임)
    u16_b: u16,
    /// 0, 계열 글 상자만 다르다 (아래)
    u16_c: u16,
    /// 글 방향: 0 가로, 2 세로 270 (값 축 제목)
    u16_e: u16,
    backdrop: bool,
    font: WObj,
    text: W,
}

fn text_block(t: Tb) -> WObj {
    let mut o = WObj::new("VtTextBlock")
        .u16("u16_a", 1)
        .u16("u16_b", t.u16_b)
        .u16("u16_c", t.u16_c)
        .u16("u16_d", 2)
        .u16("u16_e", t.u16_e)
        .u16("u16_f", 1);
    if t.backdrop {
        o = o.ptr("backdrop", backdrop(0));
    }
    o.ptr("font", t.font).u16("u16_h", 100).set("text", t.text)
}

/// 위치가 있는 요소: u16_a 160, u16_b 150, u16_c 1
fn section(visible: bool, location: u16, with_backdrop: bool) -> WObj {
    let mut o = WObj::new("VtChartSection")
        .u16("u16_a", 160)
        .u16("u16_b", 150)
        .u16("u16_c", 1)
        .u16("visible", visible as u16)
        .u16("location", location);
    if with_backdrop {
        o = o.ptr("backdrop", backdrop(0));
    }
    o
}

/// 빈 포인터 배열
fn empty_array() -> WObj {
    WObj::new("VtArray").u16("u16_a", 1).u32("u32_a", 3)
}

/// 포인터 배열
fn array(items: Vec<W>) -> WObj {
    let non_null = items.iter().filter(|w| !matches!(w, W::Null)).count();
    WObj::new("VtArray")
        .u16("size", items.len() as u16)
        .u16("u16_a", 1)
        .u32("u32_a", 3)
        .u16("nonNull", non_null as u16)
        .set("items", W::Arr(items))
}

fn new(o: WObj) -> W {
    W::New(Box::new(o))
}

// ---------------------------------------------------------------------------
// 표시 형식 (VtTextFormat)
// ---------------------------------------------------------------------------

fn digit_set(u16_a: u16) -> WObj {
    WObj::new("VtDigitSet").u16("u16_a", u16_a)
}

/// 표시 형식 구역 하나: 쓰지 않는 구역은 (10, 110, 7, 9) + 빈 부호
fn formatter(u16_a: u16, u16_c: u16, code: &str) -> WObj {
    WObj::new("VtFormatter")
        .u16("u16_a", u16_a)
        .u16("u16_b", 110)
        .u16("u16_c", u16_c)
        .u16("u16_d", 9)
        .ptr("code", string(code))
        .ptr("digits1", digit_set(0))
        .ptr("digits2", digit_set(0))
        .ptr("digits3", digit_set(0))
}

fn unused_formatter() -> WObj {
    formatter(10, 7, "")
}

fn format_group(u16_a: u16, section1: WObj) -> WObj {
    WObj::new("VtFormatGroup")
        .u16("u16_a", u16_a)
        .ptr("section1", section1)
        .ptr("section2", unused_formatter())
        .ptr("section3", unused_formatter())
        .ptr("section4", unused_formatter())
}

fn text_format(group: WObj) -> WObj {
    WObj::new("VtTextFormat").ptr("group", group)
}

/// 계열 값 레이블 형식: 네 구역 모두 쓰지 않음
fn value_format() -> WObj {
    text_format(format_group(1, unused_formatter()))
}

/// 계열 백분율 레이블 형식: `0%` (부호 `B0Jn`)
fn percent_format() -> WObj {
    let mut s1 = formatter(6, 6, "B0Jn").set(
        "u16s",
        W::Arr(vec![
            W::U16(0),
            W::U16(0),
            W::U16(65534),
            W::U16(0),
            W::U16(0),
            W::U16(0),
            W::U16(0),
            W::U16(0),
            W::U16(0),
        ]),
    );
    s1.put("digits1", new(digit_set(1)));
    text_format(format_group(1, s1))
}

/// 축 이름표의 표시 형식 → VtFormatGroup. 한셀이 부호로 바꾸는 형식 (관측: `General` → `An` 종류 0, `G/표준` →
/// `Zlg표g준n` 종류 3, `m/yy` → `OlWn` 종류 3, `0%` → `B0Jn`·`0.00%` → `B0EB1B1Jn` 종류 6) 은 첫 구역에, 그 밖
/// (`₩#,##0;[빨강]-₩#,##0` …) 은 VtFormatGroup.u16_a 0 + 빈 구역 넷 (서식 파일). 레거시 읽기가 `Zlg표g준n` 을
/// `g/표준` 으로 읽으므로 그것도 같은 부호로 쓴다
fn axis_format(code: &str) -> WObj {
    let pct = |digits: u16| {
        let code = if digits == 0 { "B0Jn" } else { "B0EB1B1Jn" };
        let mut s = formatter(6, 6, code).set(
            "u16s",
            W::Arr(
                [0u16, 0, 65534, 0, 0, 0, 0, 0, 0]
                    .into_iter()
                    .map(W::U16)
                    .collect(),
            ),
        );
        s.put("digits1", new(digit_set(1)));
        if digits > 0 {
            s.put(
                "digits2",
                new(WObj::new("VtDigitSet").u16("u16_a", digits).u16("u16_b", 1)),
            );
        }
        s
    };
    let section = match code {
        "General" => Some(formatter(0, 6, "An")),
        "G/표준" | "g/표준" => Some(formatter(3, 6, "Zlg표g준n")),
        "m/yy" => Some(formatter(3, 6, "OlWn")),
        "0%" => Some(pct(0)),
        "0.00%" => Some(pct(2)),
        _ => None,
    };
    match section {
        Some(s) => text_format(format_group(1, s)),
        None => text_format(format_group(0, unused_formatter())),
    }
}

// ---------------------------------------------------------------------------
// 축
// ---------------------------------------------------------------------------

/// 축 이름표 단계 (VtAxisScaleBlock): 빈 자리 5개. `format` 이 있으면 이름표 형식이 붙고, `value_axis` 면 u16_b 1
fn scale_block(value_axis: bool, format: Option<&str>) -> WObj {
    let slots = array(vec![W::Null; 5]);
    let mut vb = WObj::new("VtValueBlock");
    if let Some(f) = format {
        vb = vb.ptr("textFormat", axis_format(f));
    }
    vb = vb.ptr("formatCode", string("")).base(text_block(Tb {
        u16_b: 1,
        u16_c: 2,
        u16_e: 0,
        backdrop: true,
        font: font(new(string(LABEL_FACE)), 10.0, 0),
        text: W::Null,
    }));
    WObj::new("VtAxisScaleBlock")
        .ptr("slots", slots)
        .u16("u16_b", value_axis as u16)
        .base(vb)
}

/// 축의 종류
#[derive(Clone, Copy, PartialEq, Eq)]
enum AxisKind {
    X,
    Y,
    Y2,
    Z,
}

/// 축 하나의 값. 모델에 그 축이 있으면(`present`) 축 선 808080·눈금 모양은 모델의 주 눈금, 없으면 한셀 기본값
/// (축 선 8C8C8C, X 눈금 0, 그 밖 1 — 원형의 X·Y 축이 그렇다)
struct AxisSpec {
    kind: AxisKind,
    present: bool,
    hidden: bool,
    /// 사용자 제목과 글 방향 (VtTextBlock.u16_e)
    title: Option<(String, u16)>,
    /// 이름표 단계가 있다
    labels: bool,
    /// 주 눈금선 (style, 색)
    major_grid: (u16, u32),
    tick_style: u16,
    /// 100% 누적 차트의 값 축 2
    u16_r: u16,
    /// 이름표 표시 형식 (값 축은 늘, 항목 축은 모델에 형식이 있을 때)
    format: Option<String>,
    /// 분산형·거품형의 X 축 (VtAxis.xy 1 + XYScale)
    xy: bool,
    /// 영역형의 X 축 1
    u16_q: u16,
    /// 이름표 위치 `none` 인 X 축 0, 그 밖 1 (서식 파일 CM_and_BreakEven_Analysis — 값 축은 `none` 이어도 1).
    /// 레거시 읽기가 0 을 `low` 로 읽으므로 `low` 도 0 (추정: 읽기의 반대)
    u16_p: u16,
}

/// `c:majorTickMark` → VtAxis.tickStyle (0 none, 1 cross, 2 in, 3 out — c2_e7_tick_none 의 cross → 1, 서식 파일)
fn tick_style(mark: Option<&str>) -> u16 {
    match mark {
        Some("none") => 0,
        Some("cross") => 1,
        Some("in") => 2,
        _ => 3,
    }
}

fn axis(a: &AxisSpec) -> WObj {
    let default_title = match a.kind {
        AxisKind::X => "X 축 제목",
        AxisKind::Y => "Y 축 제목",
        AxisKind::Y2 => "2차 Y 축 제목",
        AxisKind::Z => "Z 축 제목",
    };
    // 숨긴 축은 기본 색 (c2_e7_cat_del, c2_e7_val_del)
    let axis_color = if a.present && !a.hidden {
        0x808080
    } else {
        GRAY
    };
    // 사용자 제목: 글 상자 u16_b 1, 글꼴은 빈 이름의 새 문자열 10pt 굵게·기울임 (c2_e7_ax_titles)
    let title = text_block(Tb {
        u16_b: a.title.is_some() as u16,
        u16_c: 2,
        u16_e: a.title.as_ref().map_or(0, |t| t.1),
        backdrop: true,
        font: match a.title {
            Some(_) => font(new(string("")), 10.0, 3),
            None => font(default_face(), 9.0, 0),
        },
        text: new(string(a.title.as_ref().map_or(default_title, |t| &t.0))),
    });
    let labels = if a.labels {
        array(vec![new(scale_block(
            a.kind == AxisKind::Y,
            a.format.as_deref(),
        ))])
    } else {
        empty_array()
    };
    let mut o = WObj::new("VtAxis")
        .u16("hidden", a.hidden as u16)
        .u16("u16_a", a.title.is_none() as u16)
        .ptr(
            "bytes",
            WObj::new("VtBytes").set("data", W::Bytes(Vec::new())),
        )
        .u16("u16_b", 10)
        .u16("xy", a.xy as u16)
        .set("axisPen", pen(1, 0, axis_color, 0, 20.0))
        .set(
            "majorGridPen",
            pen(a.major_grid.0, 0, a.major_grid.1, 0, 20.0),
        )
        .set("minorGridPen", pen(0, 0, 0xCCCCCC, 0, 20.0))
        .f32("f32_a", 60.0)
        .ptr("title", title)
        .ptr("labels", labels);
    for f in [
        "u16_c", "u16_d", "u16_e", "u16_f", "u16_g", "u16_h", "u16_i",
    ] {
        o = o.u16(f, 1);
    }
    o = o.u16("u16_p", a.u16_p);
    if a.kind == AxisKind::X && a.present {
        o = o.u16("u16_k", 1);
    }
    if a.xy {
        // 분산형 X 축 눈금 (chart_scatter: 0, 0, 1, 1, 1, 1, 1)
        o = o.set(
            "xyScale",
            WGroup::new()
                .u16("u16_a", 1)
                .u16("u16_b", 1)
                .u16("u16_c", 1)
                .set("u8_a", W::U8(1))
                .set("u8_b", W::U8(1))
                .into_w(),
        );
    }
    o.u16("u16_q", a.u16_q)
        .f32("f32_d", 1.0)
        .u16("u16_r", a.u16_r)
        .u16("u16_s", 4)
        .u16("tickStyle", a.tick_style)
}

/// 표면 축 (늘 같다)
fn surface_axis() -> WObj {
    let title = text_block(Tb {
        u16_b: 0,
        u16_c: 2,
        u16_e: 0,
        backdrop: false,
        font: font(default_face(), 9.0, 0),
        text: W::Null,
    });
    WObj::new("VtAxis")
        .ptr(
            "bytes",
            WObj::new("VtBytes").set("data", W::Bytes(Vec::new())),
        )
        .u16("u16_b", 10)
        .set("axisPen", pen(0, 0, GRAY, 0, 20.0))
        .set("majorGridPen", pen(0, 0, GRAY, 0, 20.0))
        .set("minorGridPen", pen(0, 0, GRAY, 0, 20.0))
        .ptr("title", title)
        .ptr("labels", empty_array())
        .f32("f32_d", 1.0)
        .u16("u16_s", 2)
}

fn line_item(color: u32) -> WObj {
    WObj::new("VtCLineItem")
        .u16("u16_a", 1)
        .set("brush", brush(1, color).into_w())
        .set("pen", pen(9, 0, color, 0, 20.0))
}

fn surface() -> WObj {
    WObj::new("VtSurfaceDesc")
        .u16("u16_a", 1)
        .set("brush", brush(1, WHITE).into_w())
        .set("pen", pen(1, 0, GRAY, 0, 20.0))
        .ptr("list", empty_array())
        .ptr("axis", surface_axis())
        .u16("u16_b", 1)
        .ptr("item1", line_item(0xFF0000))
        .ptr("item2", line_item(0xFFFF00))
        .set(
            "u16s",
            W::Arr([0u16, 0, 0, 1, 1, 1, 0].into_iter().map(W::U16).collect()),
        )
}

/// 조명: 2차원 광원 둘, 3차원 하나
fn light(three_d: bool) -> WObj {
    let src = |x: f32, y: f32, z: f32| {
        new(WObj::new("VtInfLight3")
            .f32("x", x)
            .f32("y", y)
            .f32("z", z)
            .f32("intensity", 1.0))
    };
    if three_d {
        WObj::new("VtLight3")
            .ptr(
                "sources",
                array(vec![src(0.801_783_7, 0.267_261_24, 0.534_522_5)]),
            )
            .f32("ambient", 0.6)
            .f32("edge", 0.5)
            .u16("edgeVisible", 1)
    } else {
        WObj::new("VtLight3")
            .ptr(
                "sources",
                array(vec![src(0.0, 0.0, 1.0), src(0.832_050_3, 0.0, 0.554_700_2)]),
            )
            .f32("ambient", 0.15)
            .f32("edge", 1.0)
            .u16("edgeVisible", 1)
    }
}

// ---------------------------------------------------------------------------
// 차트 종류
// ---------------------------------------------------------------------------

/// 모델의 종류 → VtChart.chartType (docs/format/cell-legacy-chart.md 6절)
fn chart_type(c: &Chart) -> u16 {
    let horizontal = c.bar_dir.as_deref() == Some("bar");
    match (c.kind.as_str(), c.three_d) {
        ("bar", false) if horizontal => 11,
        ("bar", true) if horizontal => 10,
        ("bar", false) => 1,
        ("bar", true) => 0,
        ("line", false) | ("scatter", _) | ("bubble", _) => 3,
        ("line", true) => 2,
        ("area", false) => 5,
        ("area", true) => 4,
        ("pie", false) => 14,
        ("pie", true) => 13,
        ("doughnut", _) => 15,
        ("radar", _) => 18,
        ("stock", _) => 1,
        _ => 1,
    }
}

// ---------------------------------------------------------------------------
// 계열
// ---------------------------------------------------------------------------

/// `srgb:RRGGBB` → 0xRRGGBB (변형이 없는 것만)
fn srgb_value(c: &crate::model::ChartColor) -> Option<u32> {
    let hex = c.value.strip_prefix("srgb:")?;
    if !c.mods.is_empty() {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// 계열 모델의 단색 선 (색, 두께 twip — EMU/635)
fn series_line(s: Option<&ChartSeries>) -> Option<(u32, f32)> {
    let line = s?.sp_pr.as_ref()?.line.as_ref()?;
    let color = match line.fill.as_ref()? {
        ChartFill::Solid(c) => srgb_value(c)?,
        _ => return None,
    };
    let width = line.width.map_or(20.0, |w| (w as f32 / 635.0).round());
    Some((color, width))
}

/// 계열 모델의 단색 채우기 (없으면 기본 팔레트)
fn series_color(s: Option<&ChartSeries>, i: usize) -> u32 {
    s.and_then(|s| s.sp_pr.as_ref())
        .and_then(|p| p.fill.as_ref())
        .and_then(|f| match f {
            ChartFill::Solid(c) => srgb_value(c),
            _ => None,
        })
        .unwrap_or(PALETTE[i % PALETTE.len()])
}

/// 계열 목록 항목 하나의 값
struct SeriesSpec<'a> {
    /// 계열 목록의 자리
    i: usize,
    /// 계열 이름 (기본 자리 계열은 `행N`, 원형은 항목 이름표)
    name: &'a str,
    model: Option<&'a ChartSeries>,
    /// 누적 묶음: (묶음 번호, 묶음 안 차례) — 누적·100% 차트의 실제 계열 (c2_e3_*: 계열 둘이면 (1, 0), (1, 1))
    stack: Option<(u16, u16)>,
    /// 꺾은선·분산형·방사형 1
    show_line: bool,
    /// 꺾은선의 표식 지정 (VtSeries.markerStyle, 크기 twip, pen5 에 쓸 표식 채우기 색)
    marker: Option<(u16, f32, Option<u32>)>,
    /// 데이터 레이블 (구성 비트, 위치 u16s3[8])
    labels: Option<(u16, u16)>,
    /// 거품형: Some(false) 값 계열, Some(true) 거품 크기 계열
    bubble: Option<bool>,
    /// 꺾은선 갈래 (꺾은선·분산형·거품형·방사형): 계열 선 색이 linePen 이 된다
    line_family: bool,
    /// 분산형의 부드러운 선 (u16s3[5] 2)
    smooth: bool,
}

/// `c:marker/c:symbol` → VtSeries.markerStyle (표식 모양 실험: c2_e5_line_marker 의 모양을 바꾼 XML 을 한셀이 레거시로
/// 저장한 것 — plus 1, x 2, star 3, circle·auto 4, square 5, diamond 6, triangle 7, 모양을 담을 자리가 없는 dash·dot·picture
/// 는 6 (한셀의 레거시 읽기는 diamond). 거품형은 따로 9 — bubble_series)
fn marker_style(symbol: Option<&str>) -> u16 {
    match symbol {
        Some("plus") => 1,
        Some("x") => 2,
        Some("star") => 3,
        Some("square") => 5,
        Some("diamond") | Some("dash") | Some("dot") | Some("picture") => 6,
        Some("triangle") => 7,
        _ => 4,
    }
}

/// 계열의 데이터 레이블 (보이는 것이 있을 때): 구성 비트 (1 값, 2 백분율, 4 계열 이름, 8 항목), 위치 (`outEnd` 2,
/// 그 밖 1 — c2_e8_*)
fn series_labels(d: Option<&crate::model::ChartDataLabels>) -> Option<(u16, u16)> {
    let d = d?;
    if d.delete == Some(true) {
        return None;
    }
    let mut bits = 0u16;
    for (on, b) in [
        (d.show_val, 1u16),
        (d.show_percent, 2),
        (d.show_ser_name, 4),
        (d.show_cat_name, 8),
    ] {
        if on == Some(true) {
            bits |= b;
        }
    }
    (bits != 0).then(|| {
        let pos = match d.d_lbl_pos.as_deref() {
            Some("outEnd") => 2,
            _ => 1,
        };
        (bits, pos)
    })
}

/// 계열 하나 (VtSeries)
fn series(sp: &SeriesSpec) -> WObj {
    let (i, name, s) = (sp.i, sp.name, sp.model);
    // 칠은 계열 채우기, 선·표식·색 사본은 기본 팔레트 (chart_color: 채우기 FF0000 이어도 선은 73B1D1). 꺾은선 갈래는
    // 계열 선 색이 계열 선, 막대·영역은 테두리 (c2_e5_ser_pen: 실선 20 twip)
    let color = series_color(s, i);
    let palette = PALETTE[i % PALETTE.len()];
    let line = series_line(s);
    let (line_color, line_width) = match line {
        Some((c, w)) if sp.line_family => (c, w),
        _ => (palette, 40.0),
    };
    let edge = match line {
        Some((c, _)) if !sp.line_family => pen(1, 0, c, 0, 20.0),
        _ => WGroup::new()
            .u16("u16_a", 1)
            .color("color", 0x878787)
            .f32("limit", 3.0)
            .into_w(),
    };
    let name_key = format!("$series{i}");
    let shared_name = || W::Shared(name_key.clone(), Box::new(string(name)));
    let order = i as u16;
    let mut u16s2 = vec![W::U16(0); 18];
    u16s2[0] = W::U16(12);
    u16s2[4] = W::U16(order);
    u16s2[5] = W::U16(order);
    u16s2[6] = W::U16(order);
    if let Some((group, k)) = sp.stack {
        u16s2[6] = W::U16(group);
        u16s2[7] = W::U16(k);
    }
    let label = |size: f32, text: W| {
        WObj::new("VtSeriesLabel").base(text_block(Tb {
            u16_b: 0,
            u16_c: 2,
            u16_e: 0,
            backdrop: false,
            font: font(default_face(), size, 0),
            text,
        }))
    };
    let mut u16s3 = vec![W::U16(0); 10];
    for (k, v) in [(0, 2u16), (1, 2), (2, 2), (3, 6), (4, 8)] {
        u16s3[k] = W::U16(v);
    }
    if sp.smooth {
        u16s3[5] = W::U16(2);
    }
    if let Some((_, pos)) = sp.labels {
        u16s3[8] = W::U16(pos);
    }
    // 데이터 레이블을 보이면 글 상자 u16_b 1 + 배경, 글꼴은 새 `돋움` 10pt (c2_e8_*)
    let data_label = match sp.labels {
        Some(_) => WObj::new("VtSeriesLabel").base(text_block(Tb {
            u16_b: 1,
            u16_c: 2,
            u16_e: 0,
            backdrop: true,
            font: font(new(string(LABEL_FACE)), 10.0, 0),
            text: W::Null,
        })),
        None => label(8.0, W::Null),
    };
    // 표식 지정: u16_d 0, 표식·pen5 색 플래그 0x20 (chart_line, c2_e5_line_marker). 한셀의 레거시 읽기는 pen5 의 색을
    // 표식 채우기로 읽고 검정이면 채우기 없음으로 읽는다 (거품형 표식 실험: 색 플래그와 상관없다). 한셀의 XML→레거시
    // 저장은 꺾은선 표식의 채우기를 버리고 검정을 쓴다 (표식 채우기 실험 m1–m6) — 채우기 색은 레거시에서 읽은 문서만 준다
    let (u16_d, auto_flag, marker_size, style, pen5_color) = match sp.marker {
        Some((style, size, fill)) => (0, 0x20, size, style, fill),
        None => (1, 0, 200.0, 2, None),
    };
    let o = WObj::new("VtSeries")
        .set("linePen", pen(1, 2, line_color, 0, line_width))
        .set("edgePen", edge)
        .u16("u16_a", 1)
        .set("brush", brush(1, color).into_w())
        .ptr("points", empty_array())
        .set(
            "u16s1",
            W::Arr(vec![W::U16(1), W::U16(0), W::U16(0), W::U16(0)]),
        )
        .set("pen3", pen(1, 0, 0x939393, 0, 20.0))
        .set("u16s2", W::Arr(u16s2))
        .set("markerPen", pen(0, 0, palette, 0x80, 20.0))
        .set("name", shared_name())
        .ptr("seriesLabel", label(10.0, shared_name()))
        .ptr(
            "textBlock",
            text_block(Tb {
                u16_b: 0,
                u16_c: 0,
                u16_e: 0,
                backdrop: false,
                font: font(default_face(), 10.0, 0),
                text: shared_name(),
            }),
        )
        .u16("labelComponents", sp.labels.map_or(1, |l| l.0))
        .ptr("valueFormat", value_format())
        .ptr("percentFormat", percent_format())
        .ptr("dataLabel", data_label)
        .u16("u16_d", u16_d)
        .u16("u16_e", 2)
        .set(
            "pen5",
            match pen5_color {
                Some(c) => pen(1, 0, c, 0, 20.0),
                None => pen(1, 0, 0, auto_flag, 20.0),
            },
        )
        .f32("markerSize", marker_size)
        .u16("showLine", sp.show_line as u16)
        .set("markerColor", W::Color(0, auto_flag))
        .ptr("picture", picture())
        .u16("markerStyle", style)
        .u16("u16_g", 4)
        .f32("f32_a", 1.0)
        .set("color2", W::Color(palette, 0x80))
        .set("color3", W::Color(palette, 0x80))
        .u16("index", i as u16)
        .u32("typeCount", TYPE_TABLE.len() as u32)
        .set(
            "typeTable",
            W::Arr(TYPE_TABLE.iter().map(|&v| W::U16(v)).collect()),
        )
        .set("u16s3", W::Arr(u16s3));
    match sp.bubble {
        Some(size_series) => bubble_series(o, size_series),
        None => o,
    }
}

/// 거품형 계열 (c2_e6_bubble): 값 계열은 회색 선·칠 없음, 거품 크기 계열은 u16s2[3] 1. 둘 다 표식 9 (9999FF)
fn bubble_series(mut o: WObj, size_series: bool) -> WObj {
    let mut table: Vec<W> = TYPE_TABLE.iter().map(|&v| W::U16(v)).collect();
    for (k, v) in [(7usize, 5u16), (8, 6), (19, 6)] {
        table[k] = W::U16(v);
    }
    o.put("typeTable", W::Arr(table));
    o.put(
        "u16s1",
        W::Arr(vec![W::U16(0), W::U16(0), W::U16(0), W::U16(0)]),
    );
    o.put("u16_d", W::U16(0));
    o.put("pen5", pen(0, 0, GRAY, 0, 20.0));
    o.put("markerColor", W::Color(0x9999FF, 0));
    o.put("markerStyle", W::U16(9));
    if size_series {
        o.put("markerSize", W::F32(0.0));
        if let Some(W::Arr(a)) = o.get_mut("u16s2") {
            a[3] = W::U16(1);
        }
    } else {
        o.put("markerSize", W::F32(600.0));
        o.put("linePen", pen(0, 0, GRAY, 0, 20.0));
        o.put("brush", brush(1, GRAY).into_w());
        o.put("markerPen", pen(0, 0, GRAY, 0x80, 20.0));
        o.put("color2", W::Color(GRAY, 0x80));
        o.put("color3", W::Color(GRAY, 0x80));
    }
    o
}

// ---------------------------------------------------------------------------
// 데이터 표 (VtDataGrid)
// ---------------------------------------------------------------------------

/// 차트 데이터: 항목 이름표, 계열 이름, 값 (행 = 항목, 열 = 계열)
pub(crate) struct GridData {
    pub categories: Vec<String>,
    /// 열마다 계열 이름 (이름이 없는 계열은 `계열N`)
    pub names: Vec<String>,
    /// 이름이 있는 계열이 하나라도 있다 — 없으면 이름 행이 없다 (서식 파일)
    pub has_names: bool,
    /// 거품형: 홀수 열(거품 크기)의 이름 칸은 비어 있다 (c2_e6_bubble)
    pub bubble: bool,
    /// 꺾은선 표식의 단색 채우기를 pen5 에 쓴다 (레거시에서 읽은 문서 — 한셀의 XML→레거시 저장은 버린다)
    pub marker_fill: bool,
    pub values: Vec<Vec<Option<f64>>>,
}

fn data_grid(d: &GridData, pie: bool) -> WObj {
    let label_cols = u16::from(!d.categories.is_empty());
    let label_rows = u16::from(d.has_names);
    let rows = d
        .categories
        .len()
        .max(d.values.iter().map(Vec::len).max().unwrap_or(0)) as u16;
    let cols = d.names.len() as u16;
    let width = label_cols + cols;
    let height = label_rows + rows;
    let mut cells: Vec<W> = Vec::with_capacity(width as usize * height as usize);
    for c in 0..width {
        for r in 0..height {
            // 이름 행 (있으면 0행), 그 아래 데이터 행
            let name_row = label_rows == 1 && r == 0;
            let row = (r - label_rows) as usize;
            let cell = if c < label_cols {
                // 항목 이름표 열
                if name_row {
                    W::Null
                } else {
                    d.categories.get(row).map_or(W::Null, |t| new(string(t)))
                }
            } else {
                let k = (c - label_cols) as usize;
                if name_row && d.bubble && k % 2 == 1 {
                    W::Null
                } else if name_row {
                    d.names.get(k).map_or(W::Null, |t| new(string(t)))
                } else {
                    let v = d
                        .values
                        .get(k)
                        .and_then(|v| v.get(row).copied().flatten())
                        .unwrap_or(0.0);
                    new(WObj::new("VtDouble")
                        .set("value", W::F64(v))
                        .u16("flags", 0xFFFF))
                }
            };
            cells.push(cell);
        }
    }
    let count = cells.iter().filter(|c| !matches!(c, W::Null)).count() as u16;
    let matrix = WObj::new("VtMatrix")
        .base(WObj::new("VtCollection").u16("count", count))
        .u16("width", width)
        .u16("height", height)
        .set("cells", W::Arr(cells));
    WObj::new("VtDataGrid")
        .base(matrix)
        .u16("labelCols", label_cols)
        .u16("labelRows", label_rows)
        .u16("rows", rows)
        .u16("cols", cols)
        .u16("u16_a", u16::from(!pie))
}

// ---------------------------------------------------------------------------
// 차트
// ---------------------------------------------------------------------------

fn legend_location(pos: &str) -> u16 {
    match pos {
        "t" => 1,
        "l" => 3,
        "b" => 6,
        _ => 4,
    }
}

/// 차트 하나의 Vt 아카이브 바이트
pub(crate) fn archive(c: &Chart, data: &GridData) -> Vec<u8> {
    write_archive(&chart(c, data))
}

// ---------------------------------------------------------------------------
// 레코드 (머리·수식·꼬리)
// ---------------------------------------------------------------------------

/// 차트 수식의 참조 하나 (시트 번호, 행·열 범위) 또는 글자
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChartTok {
    Area {
        sheet: usize,
        r1: u32,
        c1: u32,
        r2: u32,
        c2: u32,
    },
    Str(String),
}

/// 차트 수식 글(`시트!$A$2:$A$6`, `(…,…)`, `"글자"`)을 참조들로. 시트 이름은 `sheet_of` 로 번호를 찾는다
pub(crate) fn parse_chart_formula(
    text: &str,
    sheet_of: &dyn Fn(&str) -> Option<usize>,
) -> Option<Vec<ChartTok>> {
    let t = text.trim();
    let t = t
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(t);
    if t.is_empty() {
        return Some(Vec::new());
    }
    if let Some(s) = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return Some(vec![ChartTok::Str(s.replace("\"\"", "\""))]);
    }
    let mut out = Vec::new();
    // 따옴표 안의 쉼표는 시트 이름이다
    let mut parts = Vec::new();
    let (mut start, mut quoted) = (0usize, false);
    for (i, ch) in t.char_indices() {
        match ch {
            '\'' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(&t[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&t[start..]);
    for p in parts {
        let (sheet, cells) = p.trim().rsplit_once('!')?;
        let sheet = sheet.trim_matches('\'').replace("''", "'");
        let si = sheet_of(&sheet)?;
        let (a, b) = cells.split_once(':').unwrap_or((cells, cells));
        let a = crate::model::CellRef::parse(&a.replace('$', ""))?;
        let b = crate::model::CellRef::parse(&b.replace('$', ""))?;
        out.push(ChartTok::Area {
            sheet: si,
            r1: a.row,
            c1: a.col,
            r2: b.row,
            c2: b.col,
        });
    }
    Some(out)
}

/// 참조들 → `cce u16` + 토큰 (Ref3d 0x3A / Area3d 0x3B, 둘 이상이면 합집합 0x10, 글자 0x17). `xti` 는 시트 번호 → ixti
pub(crate) fn chart_formula(toks: &[ChartTok], xti: &mut dyn FnMut(usize) -> u16) -> Vec<u8> {
    let mut t = Vec::new();
    for (k, tok) in toks.iter().enumerate() {
        match tok {
            ChartTok::Area {
                sheet,
                r1,
                c1,
                r2,
                c2,
            } => {
                let ixti = xti(*sheet);
                if r1 == r2 && c1 == c2 {
                    t.push(0x3A);
                    t.extend_from_slice(&ixti.to_le_bytes());
                    t.extend_from_slice(&r1.to_le_bytes());
                    t.extend_from_slice(&(*c1 as u16).to_le_bytes());
                } else {
                    t.push(0x3B);
                    t.extend_from_slice(&ixti.to_le_bytes());
                    t.extend_from_slice(&r1.to_le_bytes());
                    t.extend_from_slice(&r2.to_le_bytes());
                    t.extend_from_slice(&(*c1 as u16).to_le_bytes());
                    t.extend_from_slice(&(*c2 as u16).to_le_bytes());
                }
            }
            ChartTok::Str(s) => {
                t.push(0x17);
                t.extend_from_slice(&(s.encode_utf16().count() as u16).to_le_bytes());
                t.push(1);
                for u in s.encode_utf16() {
                    t.extend_from_slice(&u.to_le_bytes());
                }
            }
        }
        if k > 0 {
            t.push(0x10);
        }
    }
    let mut b = (t.len() as u16).to_le_bytes().to_vec();
    b.extend_from_slice(&t);
    b
}

/// 원본 범위 수식: ixti 는 시트 번호. 참조가 하나뿐이면 값 클래스 — 영역 0x5B (c2_e10_str_cat: 한셀 레코드 77개의
/// 원본 범위 중 참조 하나짜리는 이것뿐이고 0x5B, 합집합의 피연산자는 모두 0x3B·0x3A), 칸 0x5A (추정)
pub(crate) fn source_formula(toks: &[ChartTok]) -> Vec<u8> {
    let mut b = chart_formula(toks, &mut |si| si as u16);
    if toks.len() == 1
        && matches!(toks[0], ChartTok::Area { .. })
        && let Some(ptg) = b.get_mut(2)
    {
        // 참조 클래스 0x20 → 값 클래스 0x40
        *ptg = (*ptg & 0x1F) | 0x40;
    }
    b
}

/// 0x7EF0 레코드: 판 1, 종류 0x67, 차트 번호, 앵커 22바이트 (flags 0x0606), 예약 11, 0x0011, 원본 범위 수식, 아카이브
/// 길이 + 아카이브, 항목 수식, 계열 수, 계열마다 값 수식 + 이름 수식 (docs/format/cell-legacy-chart.md 1절)
pub(crate) fn record(
    chart_index: u16,
    anchor: [u32; 8],
    source: &[u8],
    archive: &[u8],
    categories: &[u8],
    series: &[(Vec<u8>, Vec<u8>)],
) -> Vec<u8> {
    let mut b = Vec::with_capacity(archive.len() + 256);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&0x67u16.to_le_bytes());
    b.extend_from_slice(&chart_index.to_le_bytes());
    b.extend_from_slice(&0x0606u16.to_le_bytes());
    b.extend_from_slice(&(anchor[0] as u16).to_le_bytes());
    b.extend_from_slice(&(anchor[1] as u16).to_le_bytes());
    b.extend_from_slice(&anchor[2].to_le_bytes());
    b.extend_from_slice(&(anchor[3] as u16).to_le_bytes());
    b.extend_from_slice(&(anchor[4] as u16).to_le_bytes());
    b.extend_from_slice(&(anchor[5] as u16).to_le_bytes());
    b.extend_from_slice(&anchor[6].to_le_bytes());
    b.extend_from_slice(&(anchor[7] as u16).to_le_bytes());
    b.extend_from_slice(&[0; 11]);
    b.extend_from_slice(&0x0011u16.to_le_bytes());
    b.extend_from_slice(source);
    b.extend_from_slice(&(archive.len() as u32).to_le_bytes());
    b.extend_from_slice(archive);
    b.extend_from_slice(categories);
    b.extend_from_slice(&(series.len() as u16).to_le_bytes());
    for (values, name) in series {
        b.extend_from_slice(values);
        b.extend_from_slice(name);
    }
    b
}

/// 사각형 (시트, 첫 행, 첫 열, 끝 행, 끝 열)
type Rect = (usize, u32, u32, u32, u32);

fn rect_of(t: &ChartTok) -> Option<Rect> {
    match *t {
        ChartTok::Area {
            sheet,
            r1,
            c1,
            r2,
            c2,
        } => Some((sheet, r1, c1, r2, c2)),
        ChartTok::Str(_) => None,
    }
}

/// 두 사각형을 합쳐 사각형이 되면 그것 (같은 행 범위에서 열이 이어지거나, 같은 열 범위에서 행이 이어진다)
fn join(a: Rect, b: Rect) -> Option<Rect> {
    if a.0 != b.0 {
        return None;
    }
    let (s, (ar1, ac1, ar2, ac2), (br1, bc1, br2, bc2)) =
        (a.0, (a.1, a.2, a.3, a.4), (b.1, b.2, b.3, b.4));
    if ar1 == br1 && ar2 == br2 && (ac2 + 1 == bc1 || bc2 + 1 == ac1) {
        return Some((s, ar1, ac1.min(bc1), ar2, ac2.max(bc2)));
    }
    if ac1 == bc1 && ac2 == bc2 && (ar2 + 1 == br1 || br2 + 1 == ar1) {
        return Some((s, ar1.min(br1), ac1, ar2.max(br2), ac2));
    }
    None
}

/// 원본 범위 (한셀이 XML 차트를 열 때 다시 만드는 범위): 목록 (계열 이름들, 항목, 계열 값들, 머리 칸)에서 합쳐
/// 사각형이 되는 첫 쌍(뒤 원소 차례로 찾는다)을 빼고 합친 것을 끝에 붙이기를 더 없을 때까지 한다. 머리 칸은 이름이
/// 있을 때만: 항목이 세로면 (첫 이름 행, 항목 열), 가로면 (항목 행, 첫 이름 열). 표본 관측: chart_base `(C1:C6,A1:A6)`,
/// 계열 둘 `(A1:A6,C1:D6)`, 행 방향 계열 `(A1:A3,C1:E3)`, 주식형 `(F1:F6,A1:A6,C1:D6)`, 서식 파일
/// `(B33,E6:P6,E33:P33,B6)`·`(F6:Q6,F66:Q66)`, 이웃한 항목 `B1:C6`
pub(crate) fn source_range(
    cat: Option<&ChartTok>,
    series: &[(Option<ChartTok>, Option<ChartTok>)],
) -> Vec<ChartTok> {
    let names: Vec<Rect> = series
        .iter()
        .filter_map(|(_, n)| n.as_ref().and_then(rect_of))
        .collect();
    let values: Vec<Rect> = series
        .iter()
        .filter_map(|(v, _)| v.as_ref().and_then(rect_of))
        .collect();
    let cat = cat.and_then(rect_of);
    let mut rects: Vec<Rect> = names.clone();
    rects.extend(cat);
    rects.extend(values);
    if let (Some(c), Some(n)) = (cat, names.first()) {
        let vertical = c.2 == c.4 && c.1 != c.3;
        let k = if vertical {
            (c.0, n.1, c.2, n.1, c.2)
        } else {
            (c.0, c.1, n.2, c.1, n.2)
        };
        if !rects.contains(&k) {
            rects.push(k);
        }
    }
    loop {
        let pair = (1..rects.len())
            .flat_map(|j| (0..j).map(move |i| (i, j)))
            .find_map(|(i, j)| join(rects[i], rects[j]).map(|r| (i, j, r)));
        let Some((i, j, r)) = pair else { break };
        rects.remove(j);
        rects.remove(i);
        rects.push(r);
    }
    rects
        .into_iter()
        .map(|r| ChartTok::Area {
            sheet: r.0,
            r1: r.1,
            c1: r.2,
            r2: r.3,
            c2: r.4,
        })
        .collect()
}

/// 모델의 축 고르기: X 는 항목·날짜 축 (분산형·거품형은 아래쪽 값 축), Y 는 값 축, Z 는 계열 축
fn model_axes(c: &Chart) -> [Option<&crate::model::ChartAxis>; 3] {
    let xy = matches!(c.kind.as_str(), "scatter" | "bubble");
    let x = if xy {
        c.axes
            .iter()
            .find(|a| a.kind == "valAx" && a.ax_pos.as_deref() == Some("b"))
            .or_else(|| c.axes.iter().find(|a| a.kind == "valAx"))
    } else {
        c.axes
            .iter()
            .find(|a| matches!(a.kind.as_str(), "catAx" | "dateAx"))
    };
    let y = c
        .axes
        .iter()
        .find(|a| a.kind == "valAx" && !x.is_some_and(|x| std::ptr::eq(x, *a)));
    let z = c.axes.iter().find(|a| a.kind == "serAx");
    [x, y, z]
}

/// 축 제목 글과 글 방향 (`vert270` → 2, `wordArtVert` → 1, `vert` → 3, 그 밖 0)
fn axis_title(a: Option<&crate::model::ChartAxis>, text: Option<&str>) -> Option<(String, u16)> {
    let text = text.filter(|t| !t.is_empty())?;
    let vert = a.and_then(|a| a.title.as_ref()).and_then(|t| match &t.tx {
        Some(crate::model::ChartTx::Rich(r)) => r.vert.clone(),
        _ => None,
    });
    let dir = match vert.as_deref() {
        Some("vert270") => 2,
        Some("wordArtVert") => 1,
        Some("vert") => 3,
        _ => 0,
    };
    Some((text.to_string(), dir))
}

fn chart(c: &Chart, data: &GridData) -> WObj {
    let ty = chart_type(c);
    let pie = matches!(ty, 13..=15);
    let doughnut = ty == 15;
    let three_d = matches!(ty, 0 | 2 | 4 | 10 | 13);
    // 3차원 축이 있는 차트 (3차원 원형 빼고)
    let three_d_axes = matches!(ty, 0 | 2 | 4 | 10);
    let bar = matches!(ty, 0 | 1 | 10 | 11);
    let horizontal = matches!(ty, 10 | 11);
    let grouping = c.grouping.as_deref();
    let stacked = matches!(grouping, Some("stacked" | "percentStacked"));
    let percent = grouping == Some("percentStacked");
    let footnote = WObj::new("VtChartFootnote").base(
        WObj::new("VtChartText")
            .ptr(
                "textBlock",
                text_block(Tb {
                    u16_b: 0,
                    u16_c: 2,
                    u16_e: 0,
                    backdrop: false,
                    font: font(default_face(), 9.0, 0),
                    text: new(string("각주")),
                }),
            )
            .base(section(false, 5, true)),
    );
    let legend = WObj::new("VtChartLegend")
        .ptr("font", font(default_face(), 9.0, 0))
        .u16("u16_a", 1)
        .u16("u16_c", 2)
        .u16("u16_d", 2)
        .base(section(
            c.legend.is_some(),
            legend_location(c.legend.as_deref().unwrap_or("r")),
            true,
        ));
    let [mx, my, mz] = model_axes(c);
    let hidden = |a: Option<&crate::model::ChartAxis>| a.and_then(|a| a.delete) == Some(true);
    let tick = |a: Option<&crate::model::ChartAxis>| {
        tick_style(a.and_then(|a| a.major_tick_mark.as_deref()))
    };
    let y_grid = match my {
        Some(a) if a.major_gridlines.is_some() => (1, 0x808080),
        Some(_) => (0, GRAY),
        // 원형: 값 축이 없으면 한셀의 기본 (1, 8C8C8C)
        None => (1, GRAY),
    };
    let xy = matches!(c.kind.as_str(), "scatter" | "bubble");
    let x_percent_quirk = percent
        || (stacked && matches!(ty, 0 | 1 | 11) && c.group.overlap.is_some_and(|o| o == 100));
    let fmt_of = |a: Option<&crate::model::ChartAxis>| {
        a.and_then(|a| a.num_fmt.as_ref())
            .map(|n| n.format_code.clone())
    };
    let x = axis(&AxisSpec {
        kind: AxisKind::X,
        present: mx.is_some(),
        hidden: hidden(mx),
        title: axis_title(mx, c.cat_axis_title.as_deref()),
        labels: !pie,
        major_grid: match mx {
            Some(a) if a.major_gridlines.is_some() => (1, 0x808080),
            _ => (0, GRAY),
        },
        tick_style: if mx.is_some() { tick(mx) } else { 0 },
        u16_r: 0,
        // 레거시 읽기의 특이점(100% 차트, 겹치기 −1 인 누적 막대의 항목 축은 늘 `0%`)이 만든 형식은 쓰지 않는다
        // (추정: 읽기의 반대 — chart_stacked·c2_e3_*_pct 의 한셀 레거시에는 항목 축 형식이 없다)
        format: fmt_of(mx)
            .filter(|f| !(f == "0%" && x_percent_quirk))
            .or_else(|| xy.then(|| "General".to_string())),
        xy,
        u16_q: u16::from(matches!(ty, 4 | 5)),
        u16_p: u16::from(!matches!(
            mx.and_then(|a| a.tick_lbl_pos.as_deref()),
            Some("none" | "low")
        )),
    });
    let y = axis(&AxisSpec {
        kind: AxisKind::Y,
        present: my.is_some(),
        hidden: hidden(my),
        title: axis_title(my, c.val_axis_title.as_deref()),
        labels: !pie,
        major_grid: y_grid,
        tick_style: if my.is_some() { tick(my) } else { 1 },
        u16_r: if percent { 2 } else { 0 },
        format: Some(fmt_of(my).unwrap_or_else(|| "G/표준".to_string())),
        xy: false,
        u16_q: 0,
        u16_p: 1,
    });
    // 보조 축: 늘 숨김. 3차원 차트는 주 눈금선이 값 축과 같다 (c2_e4_bar3d_a)
    let y2 = axis(&AxisSpec {
        kind: AxisKind::Y2,
        present: false,
        hidden: true,
        title: None,
        labels: false,
        major_grid: if three_d_axes { y_grid } else { (0, GRAY) },
        tick_style: 1,
        u16_r: 0,
        format: None,
        xy: false,
        u16_q: 0,
        u16_p: 1,
    });
    // 계열 축: 3차원 차트에만 이름표가 있고 숨긴다 (모델에 계열 축이 있으면 그 눈금·숨김)
    let z = axis(&AxisSpec {
        kind: AxisKind::Z,
        present: false,
        hidden: three_d_axes && (mz.is_none() || hidden(mz)),
        title: None,
        labels: three_d_axes,
        major_grid: (0, GRAY),
        tick_style: match mz {
            Some(_) => tick(mz),
            None if three_d_axes => 3,
            None => 1,
        },
        u16_r: 0,
        format: None,
        xy: false,
        u16_q: 0,
        u16_p: 1,
    });
    // 방사형: X 축 f32_c 90, 값 축 f32_b 90, X·값·계열 축 u16_s 0 (c2_e6_radar)
    let (mut x, mut y, mut z) = (x, y, z);
    if ty == 18 {
        x.put("f32_c", W::F32(90.0));
        y.put("f32_b", W::F32(90.0));
        for a in [&mut x, &mut y, &mut z] {
            a.put("u16_s", W::U16(0));
        }
    }
    // 3차원: 벽 선 없음(검정), 바닥 칠
    let wall = WGroup::new()
        .u16("u16_a", 1)
        .group("brush", brush(1, WHITE))
        .set(
            "pen",
            if three_d_axes {
                pen(0, 0, 0, 0, 20.0)
            } else {
                pen(1, 0, GRAY, 0, 20.0)
            },
        );
    let floor = WGroup::new()
        .u16("u16_a", 1)
        .group("brush", brush(three_d_axes as u16, WHITE))
        .set(
            "pen",
            pen(0, 0, if three_d_axes { 0 } else { WHITE }, 0, 20.0),
        )
        .u16("u16_b", 1);
    let mut plot_u16s = vec![W::U16(0); 8];
    for (k, v) in [(1, 65535u16), (2, 1), (3, 1), (5, 1), (6, 1)] {
        plot_u16s[k] = W::U16(v);
    }
    if three_d {
        plot_u16s[2] = W::U16(0);
    }
    if doughnut {
        plot_u16s[3] = W::U16(0);
    }
    // 간격: 막대는 gapWidth/100, 도넛 0, 그 밖 1. 겹치기: 막대 −overlap/100
    let g = &c.group;
    let gap = if c.kind == "stock" {
        // 주식형 (c2_e6_stock: 모델에 간격 없음 → 0.5)
        g.gap_width.map_or(0.5, |w| w as f32 / 100.0)
    } else if bar {
        g.gap_width.unwrap_or(150) as f32 / 100.0
    } else if doughnut {
        0.0
    } else {
        1.0
    };
    let overlap = match g.overlap {
        Some(o) if bar && o != 0 => -(o as f32) / 100.0,
        _ => 0.0,
    };
    let gap_depth = if three_d_axes {
        g.gap_depth.unwrap_or(150) as f32 / 100.0
    } else if pie && (three_d || doughnut) {
        0.0
    } else {
        1.0
    };
    // 3차원 보기: rotY 0 이면 회전 0 + VtChart.f32_a 90, 아니면 90 − rotY. 높이 rotX (c2_e4_*, chart_col3d)
    let view = c.view3d.as_ref();
    let (rotation, elevation, root_f32a) = if doughnut {
        (0.0, 90.0, 90.0)
    } else if three_d {
        let rot_y = view.and_then(|v| v.rot_y).unwrap_or(0) as f32;
        let rot_x = view.and_then(|v| v.rot_x).unwrap_or(0) as f32;
        if rot_y == 0.0 {
            (0.0, rot_x, 90.0)
        } else {
            (90.0 - rot_y, rot_x, 0.0)
        }
    } else {
        (30.0, 25.0, 0.0)
    };
    let plot = WObj::new("VtChartPlot")
        .ptr("ptr_a", empty_array())
        .f32("f32_a", 1.0)
        .f32("f32_b", 1.0)
        .f32("overlap", overlap)
        .f32("gap", gap)
        .f32("gapDepth", gap_depth)
        .set("wall", wall.into_w())
        .set("floor", floor.into_w())
        .f32("rotation", rotation)
        .f32("elevation", elevation)
        .f32("f32_d", 0.5)
        .f32("f32_e", 0.5)
        .f32("f32_f", 2.0)
        .ptr("light", light(three_d || doughnut))
        .ptr("axisX", x)
        .ptr("axesY", array(vec![new(y)]))
        .ptr("axesY2", array(vec![new(y2)]))
        .ptr("axisZ", z)
        .u16("u16_a", 1)
        .f32("f32_g", if ty == 13 { 70.0 } else { 90.0 })
        .f32("f32_h", 90.0)
        .f32("f32_i", if doughnut { 0.01 } else { 0.33 })
        .f32("f32_j", 1.0)
        .f32("f32_k", 0.67)
        .u16("u16_b", 4)
        .f32("f32_l", 0.1)
        .u16("u16_c", 65535)
        .ptr("surface", surface())
        .set("u16s", W::Arr(plot_u16s))
        .base(section(true, 1, true));
    // 계열 목록: 실제 계열 뒤에 기본 자리 계열 (`행N`) — 계열이 1–5개면 5개. 원형은 항목마다 하나 (이름 = 항목)
    let names: Vec<String> = if pie {
        data.categories.clone()
    } else {
        data.names.clone()
    };
    let real = names.len();
    // 데이터가 없으면 (항목·값 모두 없음) 기본 자리 계열은 `열N` 이고 목록은 4개 (서식 파일 Marketing_Budget)
    let no_rows = data.categories.is_empty() && data.values.iter().all(Vec::is_empty);
    let (min_items, prefix) = if no_rows { (4, "열") } else { (5, "행") };
    let n = real.max(min_items);
    let show_line = matches!(ty, 3 | 18);
    let items: Vec<W> = (0..n)
        .map(|i| {
            let name = names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("{prefix}{}", i + 1));
            let stack = (stacked && !pie && i < data.names.len()).then_some((1, i as u16));
            // 거품형: 표의 열은 (값, 거품 크기) 쌍 — 모델 계열은 값 열에만
            let bubble = (c.kind == "bubble" && i < data.names.len()).then_some(i % 2 == 1);
            let model = match bubble {
                _ if pie => None,
                Some(true) => None,
                Some(false) => c.series.get(i / 2),
                None => c.series.get(i),
            };
            // 원형은 첫 계열의 레이블이 항목마다 붙는다
            let labels = if pie {
                (i < real)
                    .then(|| series_labels(c.series.first().and_then(|s| s.d_lbls.as_ref())))
                    .flatten()
            } else {
                series_labels(model.and_then(|s| s.d_lbls.as_ref()))
            };
            // 표식 `none` 은 크기와 상관없이 자동 표식(u16_d 1)으로 쓴다 — 레거시 읽기는 `none` 크기 7 로 읽는다
            // (표식 모양 실험: none 크기 9 → u16_d 1, 크기 200, 모양 2)
            let marker = (ty == 3 && c.kind == "line")
                .then(|| model.and_then(|s| s.marker.as_ref()))
                .flatten()
                .filter(|m| m.symbol.as_deref() != Some("none"))
                .map(|m| {
                    let fill = m
                        .sp_pr
                        .as_ref()
                        .and_then(|p| p.fill.as_ref())
                        .and_then(|f| match f {
                            ChartFill::Solid(c) => srgb_value(c),
                            _ => None,
                        })
                        .filter(|_| data.marker_fill);
                    (
                        marker_style(m.symbol.as_deref()),
                        m.size.unwrap_or(7) as f32 * 20.0,
                        fill,
                    )
                });
            new(series(&SeriesSpec {
                i,
                name: &name,
                model,
                stack,
                show_line: show_line && i < data.names.len(),
                marker,
                labels,
                bubble,
                line_family: matches!(c.kind.as_str(), "line" | "scatter" | "bubble" | "radar"),
                smooth: c.kind == "scatter" && model.and_then(|s| s.smooth) == Some(true),
            }))
        })
        .collect();
    // 제목: 글이 있으면 글 상자 u16_b 1 + 빈 이름의 새 글꼴 12pt 굵게·기울임 (chart_title, c2_e2_*)
    let title_text = c.title.as_deref().filter(|t| !t.is_empty());
    let title = WObj::new("VtChartTitle").base(
        WObj::new("VtChartText")
            .ptr(
                "textBlock",
                text_block(Tb {
                    u16_b: title_text.is_some() as u16,
                    u16_c: 2,
                    u16_e: 0,
                    backdrop: false,
                    font: match title_text {
                        Some(_) => font(new(string("")), 12.0, 3),
                        None => font(default_face(), 12.0, 0),
                    },
                    text: new(string(title_text.unwrap_or("차트 제목"))),
                }),
            )
            .base(section(title_text.is_some(), 1, true)),
    );
    // 표시: 가로 막대 0xFFCF, 2차원 영역·누적·100% 0xFF3F, 그 밖 0xFFFF
    let mask = if horizontal {
        0xFFCF
    } else if ty == 5 || stacked {
        0xFF3F
    } else {
        0xFFFF
    };
    WObj::new("VtChart")
        .ptr("dataGrid", data_grid(data, pie))
        .ptr("backdrop", backdrop(1))
        .ptr("footnote", footnote)
        .ptr("legend", legend)
        .ptr("plot", plot)
        .ptr("seriesList", array(items))
        .ptr("title", title)
        .ptr("list", WObj::new("VtList").u32("u32_a", 3))
        .u16("u16_b", 1)
        .f32("f32_a", root_f32a)
        .ptr(
            "ssLink",
            WObj::new("VtChSsLink").ptr("dataSource", WObj::new("VtDataSourceSsLink")),
        )
        .u16("mask", mask)
        .u16("u16_c", 1)
        .u16("u16_d", 1)
        .u16("u16_e", 1)
        .u16("u16_f", 1)
        .u16("chartType", ty)
}
