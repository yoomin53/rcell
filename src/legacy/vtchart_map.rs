// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 차트 레코드(0x7EF0) → 차트 모델 ([`Chart`]).
//!
//! [`super::vtchart`] 가 읽은 Vt 아카이브와 꼬리 수식을 옮긴다. 목표는 한셀이 같은 레거시 파일을 읽어 XML 로 쓴
//! 차트 파트(`*.legacy.as.cell`)다: [`crate::chart_xml::chart_xml`] 로 쓰면 그 파트와 바이트까지 같다 (표본 77개).
//! 그래서 한셀이 레거시에서 읽지 않는 값(3차원 회전, 3차원 막대의 누적, 분산형의 부드러운 선, 원형의 데이터 레이블,
//! 글자로 쓴 계열 이름 …)은 옮기지 않고, 한셀이 늘 채우는 값(docs/format/cell-2024.md 11.2절)은 채운다.
//! 캐시의 표시 형식과 빈 칸·오류 칸은 데이터 표가 아니라 수식이 가리키는 셀에서 온다 — 데이터 표는 빈 칸과 오류를
//! 0 으로 담는다. 필드 대응: docs/format/cell-legacy-chart.md 9절.

use super::vtchart::{ChartFormula, ChartRef, Fields, Obj, VtRecord};
use crate::model::{
    Cell, CellValue, Chart, ChartAxis, ChartCache, ChartColor, ChartData, ChartDataLabels,
    ChartFill, ChartFont, ChartGroupProps, ChartLayout, ChartLine, ChartLines, ChartManualLayout,
    ChartMarker, ChartNumFmt, ChartPara, ChartPoint, ChartRich, ChartRun, ChartScaling,
    ChartSeries, ChartShapeProps, ChartTitle, ChartTx, ChartView3D, HncChartStyle, Sheet, Styles,
};

/// 한셀이 빈 글꼴 이름 대신 쓰는 글꼴 (제목·축 제목, 관측)
const DEFAULT_FACE: &str = "함초롬돋움";
/// 한셀 차트의 일반 표시 형식 (셀 서식 번호 0)
const GENERAL_KO: &str = "G/표준";
/// 선이 없을 때 한셀이 쓰는 두께 (EMU)
const THIN: i64 = 9525;

/// 차트 수식이 가리키는 셀을 읽는 통합 문서. 시트 번호는 수식 참조의 푼 시트 (`ChartRef::sheet`, BOUNDSHEET 순서)
pub(crate) struct Book<'a> {
    pub sheets: &'a [Sheet],
    pub styles: &'a Styles,
}

impl Book<'_> {
    /// 셀 (시트의 셀은 행·열 순서로 정렬되어 있다)
    fn cell(&self, sheet: u16, row: u32, col: u32) -> Option<&Cell> {
        let cells = &self.sheets.get(sheet as usize)?.cells;
        cells
            .binary_search_by(|c| (c.r.row, c.r.col).cmp(&(row, col)))
            .ok()
            .map(|i| &cells[i])
    }

    /// 셀의 표시 형식을 한셀이 차트에 쓰는 꼴로
    fn format(&self, cell: &Cell) -> String {
        let id = self
            .styles
            .cell_xfs
            .get(cell.style as usize)
            .map_or(0, |x| x.num_fmt_id);
        chart_format_code(&crate::compare::num_fmt_code(self.styles, id))
    }

    /// 수식의 참조 칸을 차례로 (참조마다 행 우선) — k 번째 칸
    fn nth_cell(&self, f: &ChartFormula, mut k: u64) -> Option<(u16, u32, u32)> {
        for r in &f.refs {
            let (rows, cols) = ref_size(r);
            let n = rows * cols;
            if k < n {
                let row = u64::from(r.rw_first) + k / cols;
                let col = u64::from(r.col_first) + k % cols;
                return r.sheet.map(|s| (s, row as u32, col as u32));
            }
            k -= n;
        }
        None
    }

    /// 수식의 참조 범위 안에 있는 셀 (빈 칸 아닌 것만)
    fn cells_in(&self, f: &ChartFormula) -> Vec<&Cell> {
        let mut out = Vec::new();
        for r in &f.refs {
            let Some(sheet) = r.sheet.and_then(|s| self.sheets.get(s as usize)) else {
                continue;
            };
            let (c1, c2) = (u32::from(r.col_first), u32::from(r.col_last));
            let start = sheet
                .cells
                .partition_point(|c| (c.r.row, c.r.col) < (r.rw_first, c1));
            for c in &sheet.cells[start..] {
                if c.r.row > r.rw_last {
                    break;
                }
                if (c1..=c2).contains(&c.r.col) && !c.value.is_empty() {
                    out.push(c);
                }
            }
        }
        out
    }
}

fn ref_size(r: &ChartRef) -> (u64, u64) {
    let rows = u64::from(r.rw_last.saturating_sub(r.rw_first)) + 1;
    let cols = u64::from(r.col_last.saturating_sub(r.col_first)) + 1;
    (rows, cols)
}

/// 셀 서식 코드 → 한셀이 차트 캐시·연결된 축 형식에 쓰는 꼴: `General` → `G/표준`, 글자 따옴표와 `\` 를
/// 벗기고 `[Red]` → `[빨강]` (관측: 서식 번호 0, 6 `"₩"#,##0;[Red]\-"₩"#,##0` → `₩#,##0;[빨강]-₩#,##0`,
/// 10 `0.00%`). 다른 색 이름은 표본에 없어 그대로 둔다
fn chart_format_code(code: &str) -> String {
    if code.eq_ignore_ascii_case("General") {
        return GENERAL_KO.into();
    }
    let mut out = String::with_capacity(code.len());
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {}
            '\\' => out.extend(chars.next()),
            _ => out.push(c),
        }
    }
    out.replace("[Red]", "[빨강]")
}

/// 숫자 캐시 값: 유효 숫자 15자리로 맞춘 최단 표기 (표본은 모두 정수)
fn num_text(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if !v.is_finite() || v.fract() == 0.0 {
        return format!("{v}");
    }
    format!("{v:.14e}")
        .parse::<f64>()
        .map_or_else(|_| format!("{v}"), |r| format!("{r}"))
}

// ---------------------------------------------------------------------------
// 차트 종류
// ---------------------------------------------------------------------------

/// 계열 서식의 갈래
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Bar,
    Line,
    Area,
    Radar,
    Pie,
}

/// VtChart.chartType 의 뜻
struct Kind {
    /// 모델의 종류 이름 (`bar` → `c:barChart`)
    name: &'static str,
    three_d: bool,
    bar_dir: Option<&'static str>,
    family: Family,
}

/// 관측: 0 3차원 막대, 1 막대 (주식형도), 2 3차원 꺾은선, 3 꺾은선 (분산형·거품형도), 4 3차원 영역, 5 영역,
/// 11 가로 막대, 13 3차원 원형, 14 원형, 15 도넛형, 18 방사형. 번호 매김은 MSChart 의 VtChChartType 과 같다.
/// 표본에 없는 번호는 그 이름에 가장 가까운 관측 종류로 읽는다 (추정): 단계형 → 꺾은선, 조합 → 막대,
/// 3차원 가로 막대·묶은 3차원 막대 → 3차원 막대, 분산형·거품형 → 꺾은선 (한셀 저장과 같다), 극좌표 → 방사형,
/// 주식형 → 막대 (한셀 저장과 같다), 간트 → 가로 막대, 표면·등고선 → 영역, 3차원 분산형 → 3차원 꺾은선
fn kind_of(chart_type: u16) -> Kind {
    let k = |name, three_d, bar_dir, family| Kind {
        name,
        three_d,
        bar_dir,
        family,
    };
    use Family::*;
    match chart_type {
        0 | 8 | 12 => k("bar", true, Some("col"), Bar),
        10 | 22 => k("bar", true, Some("bar"), Bar),
        11 | 21 => k("bar", false, Some("bar"), Bar),
        2 | 6 | 25 | 26 => k("line", true, None, Line),
        3 | 7 | 16 | 19 => k("line", false, None, Line),
        4 | 23 => k("area", true, None, Area),
        5 | 24 => k("area", false, None, Area),
        13 => k("pie", true, None, Pie),
        14 => k("pie", false, None, Pie),
        15 => k("doughnut", false, None, Pie),
        17 | 18 => k("radar", false, None, Radar),
        _ => k("bar", false, Some("col"), Bar),
    }
}

// ---------------------------------------------------------------------------
// 공통 조각
// ---------------------------------------------------------------------------

fn srgb(c: super::vtchart::VtColor) -> ChartFill {
    ChartFill::Solid(ChartColor::srgb(&c.hex()))
}

/// VtFont → 글꼴: 크기 pt × 100, 스타일 비트 0 굵게·비트 1 기울임 (표본은 0 과 3 만), 밑줄은 저장되지 않는다
/// (`c2_e2_title_u` 가 기준 파일과 같은 바이트), 빈 이름은 함초롬돋움
fn font_of(f: Option<Obj<'_>>) -> ChartFont {
    let size = f
        .and_then(|f| f.f32("size"))
        .map_or(1000, |s| (f64::from(s) * 100.0).round() as u32);
    let style = f.and_then(|f| f.u32("style")).unwrap_or(0);
    let color = f
        .and_then(|f| f.color("color"))
        .map_or_else(|| "000000".into(), |c| c.hex());
    let face = f
        .and_then(|f| f.string("name"))
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_FACE);
    ChartFont::hcell(size, style & 1 != 0, style & 2 != 0, &color, face)
}

/// VtTextBlock.u16_e (글 방향) → `a:bodyPr` 의 (`rot`, `vert`): 0 가로 (`rot="0"`), 2 세로 270 (값 축 제목, 관측).
/// 1·3 은 MSChart 순서로 본 추정
fn orientation(tb: Obj<'_>) -> (Option<i64>, &'static str) {
    match tb.u16("u16_e").unwrap_or(0) {
        1 => (None, "wordArtVert"),
        2 => (None, "vert270"),
        3 => (None, "vert"),
        _ => (Some(0), "horz"),
    }
}

/// 제목 글 상자: 줄마다 문단 하나, 문단마다 같은 글꼴의 `defRPr` 과 조각 하나 (한셀 꼴)
fn title_rich(tb: Obj<'_>, text: &str) -> ChartRich {
    let font = font_of(tb.ptr("font"));
    let (rot, vert) = orientation(tb);
    let paras = text
        .split("\r\n")
        .flat_map(|l| l.split(['\r', '\n']))
        .map(|line| ChartPara {
            align: Some("l".into()),
            def_font: Some(font.clone()),
            runs: vec![ChartRun {
                font: Some(font.clone()),
                text: line.to_string(),
            }],
        })
        .collect();
    ChartRich {
        rot,
        vert: Some(vert.into()),
        paras,
    }
}

/// VtBackdrop 의 테두리: 그림(u16_c = 1)이면 테두리 색·두께(twip × 635 EMU), 아니면 선 없음 9525
fn frame_line(bd: Option<Obj<'_>>) -> ChartLine {
    let frame = bd
        .filter(|b| b.u16("u16_c") == Some(1))
        .and_then(|b| b.group("frame"));
    match frame {
        Some(fr) => {
            let w = fr
                .f32("width")
                .map_or(0, |w| (f64::from(w) * 635.0).round() as i64);
            let fill = fr.color("color").map_or(ChartFill::None, srgb);
            ChartLine::hcell(if w == 0 { THIN } else { w }, fill)
        }
        None => ChartLine::hcell(THIN, ChartFill::None),
    }
}

/// VtBackdrop 의 채우기: VtFill.style 1 + 칠 style 1 = 단색, 그 밖은 채우기 없음 (추정)
fn backdrop_fill(bd: Option<Obj<'_>>) -> ChartFill {
    let fill = bd.and_then(|b| b.ptr("fill"));
    let brush = fill
        .filter(|f| f.u16("style") == Some(1))
        .and_then(|f| f.group("brush"))
        .filter(|b| b.u16("style") == Some(1));
    match brush.and_then(|b| b.color("fill")) {
        Some(c) => srgb(c),
        None => ChartFill::None,
    }
}

/// 선 두께: twip → pt 를 가장 가까운 기본 두께로 (같으면 굵은 쪽), EMU. 관측 20 → 12700, 40 → 28575
fn line_width(twips: f32) -> i64 {
    const PRESET: [f64; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];
    let pt = f64::from(twips) / 20.0;
    let mut best = PRESET[0];
    for p in PRESET {
        if (p - pt).abs() <= (best - pt).abs() {
            best = p;
        }
    }
    (best * 12700.0).round() as i64
}

/// Pen.style → `a:prstDash`: 1 실선 (관측). 나머지는 MSChart VtPenStyle 순서로 본 추정
fn pen_dash(style: u16) -> &'static str {
    match style {
        2 => "dash",
        3 => "sysDot",
        4 => "dashDot",
        5 => "sysDashDotDot",
        7 => "lgDashDot",
        8 => "lgDashDotDot",
        _ => "solid",
    }
}

/// VtSeries.markerStyle → `c:symbol`: 5 square, 9 circle (관측), 나머지는 MSChart VtMarkerStyle 순서 (추정)
fn marker_symbol(style: u16) -> &'static str {
    match style {
        0 => "dash",
        1 => "plus",
        2 => "x",
        3 => "star",
        4 | 9 | 14 => "circle",
        5 | 10 => "square",
        6 | 11 => "diamond",
        7 | 8 | 12 | 13 => "triangle",
        _ => "none",
    }
}

// ---------------------------------------------------------------------------
// 표시 형식 (VtTextFormat)
// ---------------------------------------------------------------------------

/// VtFormatter.code 의 내부 부호 → 서식 코드. 글자마다 부호 하나 (`g` 와 `B` 는 뒤 글자 하나를 더 쓴다), `n` 은 끝:
/// `A` General, `B<d>` 0, `E` 점, `J` %, `O` m, `W` yy, `Z` g, `l` /, `g<c>` 글자 c (표본의 모든 부호).
/// G/표준 은 연호(`Z`) + `/` + 글자 `표준` 으로 저장되어 `g/표준` 이 된다 (한셀도 그렇게 읽는다)
fn decode_format_code(code: &str) -> String {
    let mut out = String::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            'n' => break,
            'g' => out.extend(chars.next()),
            'B' => {
                chars.next();
                out.push('0');
            }
            'A' => out.push_str("General"),
            'E' => out.push('.'),
            'J' => out.push('%'),
            'O' => out.push('m'),
            'W' => out.push_str("yy"),
            'Z' => out.push('g'),
            'l' => out.push('/'),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

/// 축 이름표의 표시 형식 (VtTextFormat → VtFormatGroup). group.u16_a 0 = 원본 셀에 연결:
/// 첫 계열 값 범위의 첫 칸 형식 (`sourceLinked="1"`), 계열이 없으면 `General` (관측). 1 = 자기 형식: 쓰는 구역
/// (VtFormatter.u16_a ≠ 10 이고 부호가 있는 것)을 `;` 로 잇는다
fn axis_num_fmt(tf: Obj<'_>, rec: &VtRecord, book: &Book<'_>) -> Option<ChartNumFmt> {
    let group = tf.ptr("group")?;
    if group.u16("u16_a") == Some(0) {
        let first = rec
            .series
            .first()
            .map(|s| &s.values)
            .filter(|v| !v.refs.is_empty());
        let Some(values) = first else {
            return Some(ChartNumFmt {
                format_code: "General".into(),
                source_linked: Some(false),
            });
        };
        let code = values
            .refs
            .first()
            .and_then(|r| book.cell(r.sheet?, r.rw_first, u32::from(r.col_first)))
            .map_or_else(|| GENERAL_KO.into(), |c| book.format(c));
        return Some(ChartNumFmt {
            format_code: code,
            source_linked: Some(true),
        });
    }
    let used: Vec<String> = ["section1", "section2", "section3", "section4"]
        .iter()
        .filter_map(|s| group.ptr(s))
        .filter(|f| f.u16("u16_a") != Some(10))
        .filter_map(|f| f.string("code").filter(|c| !c.is_empty()))
        .map(decode_format_code)
        .collect();
    (!used.is_empty()).then(|| ChartNumFmt {
        format_code: used.join(";"),
        source_linked: Some(false),
    })
}

// ---------------------------------------------------------------------------
// 축
// ---------------------------------------------------------------------------

/// 축 하나 (VtAxis). 한셀은 눈금 모양(tickStyle)을 읽지 않고 늘 `out`·`none` 을 쓴다 (`c2_e7_tick_none`),
/// 최소·최대·단위·거꾸로·로그 눈금은 레거시에 저장되지 않는다
fn axis(
    ax: Obj<'_>,
    kind: &str,
    id: u32,
    cross: u32,
    pos: &str,
    num_fmt: Option<ChartNumFmt>,
) -> ChartAxis {
    let hidden = ax.u16("hidden").unwrap_or(0) != 0;
    let pen_on = |name: &str| {
        ax.group(name)
            .and_then(|p| p.u16("style"))
            .is_some_and(|s| s != 0)
    };
    let label_block = ax
        .ptr("labels")
        .and_then(|l| l.ptrs("items"))
        .and_then(|items| items.into_iter().next().flatten());
    let title = ax.ptr("title").and_then(|tb| {
        let text = tb.string("text").filter(|t| !t.is_empty())?;
        (tb.u16("u16_b") == Some(1)).then(|| ChartTitle {
            tx: Some(ChartTx::Rich(title_rich(tb, text))),
            layout: Some(ChartLayout::default()),
            overlay: Some(false),
            ..Default::default()
        })
    });
    let mut a = ChartAxis {
        kind: kind.into(),
        id,
        scaling: ChartScaling {
            orientation: Some("minMax".into()),
            ..Default::default()
        },
        delete: Some(hidden),
        ax_pos: Some(pos.into()),
        major_gridlines: pen_on("majorGridPen").then(ChartLines::default),
        minor_gridlines: pen_on("minorGridPen").then(ChartLines::default),
        title,
        num_fmt,
        major_tick_mark: Some("out".into()),
        minor_tick_mark: Some("none".into()),
        // u16_p 0 = 이름표 아래쪽 (서식 파일의 X 축)
        tick_lbl_pos: Some(
            if ax.u16("u16_p") == Some(0) {
                "low"
            } else {
                "nextTo"
            }
            .into(),
        ),
        // 숨긴 축에만 선 없음 (보이는 축의 axisPen 808080 은 쓰지 않는다)
        sp_pr: hidden.then(|| ChartShapeProps {
            fill: None,
            line: Some(ChartLine::hcell(THIN, ChartFill::None)),
        }),
        tx_pr: label_block.map(|b| ChartRich::hcell(font_of(b.ptr("font")), None, false)),
        cross_ax: Some(cross),
        crosses: Some("autoZero".into()),
        ..Default::default()
    };
    match kind {
        "catAx" => {
            a.auto = Some(true);
            a.lbl_algn = Some("ctr".into());
            a.lbl_offset = Some(100);
            a.tick_lbl_skip = Some(1);
            a.tick_mark_skip = Some(1);
            a.no_multi_lvl_lbl = Some(false);
        }
        "valAx" => a.cross_between = Some("between".into()),
        _ => a.tick_mark_skip = Some(1),
    }
    a
}

/// 축 이름표 단계의 표시 형식 객체 (VtAxis.labels[0] 의 VtValueBlock.textFormat)
fn label_format<'a>(ax: Obj<'a>) -> Option<Obj<'a>> {
    ax.ptr("labels")?
        .ptrs("items")?
        .into_iter()
        .next()
        .flatten()?
        .ptr("textFormat")
}

// ---------------------------------------------------------------------------
// 데이터 (VtDataGrid + 꼬리 수식)
// ---------------------------------------------------------------------------

/// 데이터 표: 열 우선 행렬, 0열 = 항목 이름표, 0행 = 계열 이름 (계열은 늘 열이다 — 행 방향 차트도 바꿔 담는다)
struct Grid<'a> {
    label_cols: u16,
    label_rows: u16,
    rows: u16,
    width: u16,
    height: u16,
    cells: Vec<Option<Obj<'a>>>,
}

impl<'a> Grid<'a> {
    fn read(g: Option<Obj<'a>>) -> Self {
        let f = |n: &str| g.and_then(|g| g.u16(n)).unwrap_or(0);
        Grid {
            label_cols: f("labelCols"),
            label_rows: f("labelRows"),
            rows: f("rows"),
            width: f("width"),
            height: f("height"),
            cells: g.and_then(|g| g.ptrs("cells")).unwrap_or_default(),
        }
    }

    /// 칸의 글: VtString 은 그대로, VtDouble 은 숫자 표기
    fn text(&self, col: u32, row: u32) -> Option<String> {
        if col >= u32::from(self.width) || row >= u32::from(self.height) {
            return None;
        }
        let o = (*self
            .cells
            .get((col * u32::from(self.height) + row) as usize)?)?;
        match o.class() {
            "VtString" => o.string("text").map(str::to_string),
            _ => o.f64("value").map(num_text),
        }
    }
}

/// 항목 (모든 계열이 같이 쓴다). 숫자 칸만이면 `numRef` + 형식 (칸마다 형식은 쓰지 않는다), 아니면 `strRef`.
/// 캐시 값은 데이터 표의 이름표 글 (칸의 표시 글). `numRef` 의 값은 그 칸의 숫자 — 표본에서는 이름표 글과 같다,
/// 표시 형식이 숫자 표기를 바꾸는 칸(날짜 등)은 표본에 없다 (추정)
fn categories(rec: &VtRecord, grid: &Grid<'_>, book: &Book<'_>) -> Option<ChartData> {
    let f = &rec.categories;
    if f.refs.is_empty() {
        return None;
    }
    let cells = book.cells_in(f);
    let numeric = !cells.is_empty()
        && cells
            .iter()
            .all(|c| matches!(c.value, CellValue::Number(_) | CellValue::Bool(_)));
    let points: Vec<ChartPoint> = (0..grid.rows)
        .filter(|_| grid.label_cols > 0)
        .filter_map(|k| {
            let v = grid.text(0, u32::from(grid.label_rows) + u32::from(k))?;
            let number = numeric
                .then(|| book.nth_cell(f, u64::from(k)))
                .flatten()
                .and_then(|(ixti, row, col)| book.cell(ixti, row, col))
                .and_then(|c| match c.value {
                    CellValue::Number(n) => Some(num_text(n)),
                    _ => None,
                });
            Some(ChartPoint {
                idx: u32::from(k),
                format_code: None,
                value: number.unwrap_or(v),
            })
        })
        .collect();
    let mut formats: Vec<String> = cells.iter().map(|c| book.format(c)).collect();
    formats.sort();
    formats.dedup();
    let format_code = match formats.as_slice() {
        [] => GENERAL_KO.to_string(),
        [one] => one.clone(),
        _ => "General".into(),
    };
    Some(ChartData {
        kind: if numeric { "numRef" } else { "strRef" }.into(),
        formula: Some(f.text.clone()),
        cache: Some(ChartCache {
            format_code: numeric.then_some(format_code),
            pt_count: Some(u32::from(grid.rows)),
            points,
        }),
    })
}

/// 값: 데이터 표의 계열 열 + 칸마다 형식. 빈 칸은 빈 값 + `General`, 오류 칸은 빈 값 + 그 칸의 형식
/// (`c2_e10_blank_na`), 묶음 형식은 칸 형식이 모두 같을 때 그것, 아니면 `General`
fn values(f: &ChartFormula, col: u32, grid: &Grid<'_>, book: &Book<'_>) -> ChartData {
    let mut points: Vec<ChartPoint> = (0..grid.rows)
        .filter_map(|k| {
            let v = grid.text(col, u32::from(grid.label_rows) + u32::from(k))?;
            Some(ChartPoint {
                idx: u32::from(k),
                format_code: Some(GENERAL_KO.into()),
                value: v,
            })
        })
        .collect();
    for p in &mut points {
        let Some((ixti, row, c)) = book.nth_cell(f, u64::from(p.idx)) else {
            continue;
        };
        match book.cell(ixti, row, c) {
            None => {
                p.value.clear();
                p.format_code = Some("General".into());
            }
            Some(cell) => match cell.value {
                CellValue::Empty => {
                    p.value.clear();
                    p.format_code = Some("General".into());
                }
                CellValue::Error(_) => {
                    p.value.clear();
                    p.format_code = Some(book.format(cell));
                }
                _ => p.format_code = Some(book.format(cell)),
            },
        }
    }
    let mut formats: Vec<&str> = points
        .iter()
        .filter_map(|p| p.format_code.as_deref())
        .collect();
    formats.sort_unstable();
    formats.dedup();
    let format_code = match formats.as_slice() {
        [one] => one.to_string(),
        _ => "General".into(),
    };
    ChartData {
        kind: "numRef".into(),
        formula: Some(f.text.clone()),
        cache: Some(ChartCache {
            format_code: Some(format_code),
            pt_count: Some(u32::from(grid.rows)),
            points,
        }),
    }
}

/// 계열 이름: 참조면 `strRef` + 데이터 표 0행의 글 (이름표 행이 없으면 계열 목록의 이름). 글자로 쓴 이름
/// (ptgStr)은 한셀이 읽지 않는다 (`chart_litname`: 이름 없음)
fn series_name(
    f: &ChartFormula,
    col: u32,
    entry: Option<Obj<'_>>,
    grid: &Grid<'_>,
) -> Option<ChartTx> {
    if f.refs.is_empty() {
        return None;
    }
    let name = (grid.label_rows > 0)
        .then(|| grid.text(col, 0))
        .flatten()
        .or_else(|| entry.and_then(|e| e.string("name")).map(str::to_string));
    Some(ChartTx::Ref(ChartData {
        kind: "strRef".into(),
        formula: Some(f.text.clone()),
        cache: Some(ChartCache {
            format_code: None,
            pt_count: Some(1),
            points: name
                .map(|v| ChartPoint {
                    idx: 0,
                    format_code: None,
                    value: v,
                })
                .into_iter()
                .collect(),
        }),
    }))
}

// ---------------------------------------------------------------------------
// 계열 서식 (VtSeries)
// ---------------------------------------------------------------------------

/// 계열의 채우기·선. 막대·영역은 테두리 선(edgePen: style 0 = 선 없음), 꺾은선·방사형은 계열 선(linePen —
/// style·showLine 은 한셀이 보지 않는다: 거품형 style 0, 3차원 꺾은선 showLine 0 도 실선)
fn series_sp_pr(e: Obj<'_>, family: Family) -> ChartShapeProps {
    let brush = e.group("brush");
    let fill = match brush
        .filter(|b| b.u16("style") != Some(0))
        .and_then(|b| b.color("fill"))
    {
        Some(c) => srgb(c),
        None => ChartFill::None,
    };
    let pen = e.group(if matches!(family, Family::Line | Family::Radar) {
        "linePen"
    } else {
        "edgePen"
    });
    let style = pen.and_then(|p| p.u16("style")).unwrap_or(0);
    let line = if matches!(family, Family::Bar | Family::Area) && style == 0 {
        ChartLine::hcell(THIN, ChartFill::None)
    } else {
        let w = line_width(pen.and_then(|p| p.f32("width")).unwrap_or(0.0));
        let fill = pen
            .and_then(|p| p.color("color"))
            .map_or(ChartFill::None, srgb);
        ChartLine {
            dash: Some(pen_dash(style).into()),
            ..ChartLine::hcell(w, fill)
        }
    };
    ChartShapeProps {
        fill: Some(fill),
        line: Some(line),
    }
}

/// 표식: VtSeries.u16_d 1 = 자동 (한셀은 `none` 크기 7 로 읽는다), 0 = 지정 (모양, 크기 twip/20 pt, 채우기 = pen5 의 색 —
/// 검정이면 채우기 없음. 거품형 표식 실험: c2_e6_bubble 의 pen5 색을 ff0000 으로 바꾸면 표식이 ff0000, 000000 이면 채우기
/// 없음; 꺾은선 c2_e5_line_marker 의 pen5 를 123456 으로 바꾸면 123456. 색 플래그 0x20·pen5 의 선 모양·계열 칠·표식 색
/// markerColor 는 보지 않는다)
fn series_marker(e: Obj<'_>) -> ChartMarker {
    if e.u16("u16_d").unwrap_or(1) != 0 {
        return ChartMarker {
            symbol: Some("none".into()),
            size: Some(7),
            sp_pr: None,
        };
    }
    let size = e
        .f32("markerSize")
        .map_or(7, |s| (f64::from(s) / 20.0).round().clamp(2.0, 72.0) as u32);
    let fill = e
        .group("pen5")
        .and_then(|p| p.color("color"))
        .filter(|c| c.rgb != [0, 0, 0]);
    ChartMarker {
        symbol: Some(marker_symbol(e.u16("markerStyle").unwrap_or(15)).into()),
        size: Some(size),
        sp_pr: fill.map(|c| ChartShapeProps {
            fill: Some(srgb(c)),
            line: None,
        }),
    }
}

/// 데이터 레이블: VtSeries.u16s3[8] 위치 (0 = 없음), labelComponents 비트 1 값, 2 백분율, 4 계열 이름 (추정),
/// 8 항목. 막대는 켰을 때만, 꺾은선·영역·방사형은 늘 쓴다 (끄면 표시가 모두 0). 원형은 한셀이 읽지 않는다
fn series_labels(e: Obj<'_>, family: Family) -> Option<ChartDataLabels> {
    let on = e
        .u16s("u16s3")
        .and_then(|v| v.get(8).copied())
        .is_some_and(|l| l != 0);
    match family {
        Family::Pie => return None,
        Family::Bar if !on => return None,
        _ => {}
    }
    let comp = e.u16("labelComponents").unwrap_or(0);
    let show = |bit: u16| Some(on && comp & bit != 0);
    Some(ChartDataLabels {
        delete: Some(false),
        show_legend_key: Some(false),
        show_val: show(1),
        show_cat_name: show(8),
        show_ser_name: show(4),
        show_percent: show(2),
        show_bubble_size: Some(false),
        show_leader_lines: Some(false),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// 차트
// ---------------------------------------------------------------------------

/// 0x7EF0 레코드 하나 → 차트. 한셀이 레거시를 읽어 XML 로 쓰는 차트와 같은 모델이다
pub(crate) fn map(rec: &VtRecord, book: &Book<'_>) -> Chart {
    let root = rec.root();
    let chart_type = root.u16("chartType").unwrap_or(1);
    let k = kind_of(chart_type);
    let plot = root.ptr("plot");
    let pool: Vec<Option<Obj<'_>>> = root
        .ptr("seriesList")
        .and_then(|l| l.ptrs("items"))
        .unwrap_or_default();
    let grid = Grid::read(root.ptr("dataGrid"));
    let pie = k.family == Family::Pie;

    // 누적: 쓰는 계열의 u16s2[6] (묶음) ≠ 순서 또는 [7] (묶음 안 순서) ≠ 0. 100%: 값 축 u16_r = 2
    let used = rec.series.len().clamp(1, pool.len().max(1));
    let stacked = pool.iter().take(used).enumerate().any(|(i, e)| {
        let v = e.and_then(|e| e.u16s("u16s2")).unwrap_or_default();
        v.get(6).is_some_and(|&x| usize::from(x) != i) || v.get(7).is_some_and(|&x| x != 0)
    });
    let axis_x = plot.and_then(|p| p.ptr("axisX"));
    let axis_y = plot
        .and_then(|p| p.ptr("axesY"))
        .and_then(|a| a.ptrs("items"))
        .and_then(|items| items.into_iter().flatten().next());
    let axis_z = plot.and_then(|p| p.ptr("axisZ"));
    let percent = axis_y.and_then(|a| a.u16("u16_r")) == Some(2);
    let f32_of = |n: &str| plot.and_then(|p| p.f32(n)).map_or(0.0, f64::from);
    let pct = |v: f64| (v * 100.0).round();

    let grouping = match (k.name, k.three_d) {
        ("bar", false) => Some(if percent {
            "percentStacked"
        } else if stacked {
            "stacked"
        } else {
            "clustered"
        }),
        // 3차원 막대·꺾은선·영역은 누적이 저장되어 있어도 standard (서식 파일 8개)
        ("bar" | "line" | "area", true) => Some(if chart_type == 12 {
            "clustered"
        } else {
            "standard"
        }),
        ("line" | "area", false) => Some(if percent {
            "percentStacked"
        } else if stacked {
            "stacked"
        } else {
            "standard"
        }),
        _ => None,
    };

    let mut group = ChartGroupProps {
        radar_style: (k.name == "radar").then(|| "marker".into()),
        ..Default::default()
    };
    if k.name == "bar" {
        group.gap_width = Some(pct(f32_of("gap")) as u32);
        if !k.three_d {
            group.overlap = Some(pct(-f32_of("overlap")) as i32);
        }
    }
    if k.three_d && !pie {
        group.gap_depth = Some(pct(f32_of("gapDepth")) as u32);
    }
    if k.name == "bar" && k.three_d {
        group.shape = Some("box".into());
    }
    if k.name == "line" && !k.three_d {
        group.marker = Some(true);
        group.smooth = Some(false);
    }
    if pie && !k.three_d {
        group.first_slice_ang = Some(0);
    }
    if k.name == "doughnut" {
        group.hole_size = Some(pct(f32_of("f32_k")) as u32);
    }

    // 계열: 꼬리의 계열 중 값 수식이 있고 계열 목록의 u16s2[3] 이 0 인 것 (거품형의 둘째 열은 둘 다 아니다)
    let cat = categories(rec, &grid, book);
    let mut series = Vec::new();
    for (t, sf) in rec.series.iter().enumerate() {
        let entry = pool.get(t).copied().flatten();
        let excluded = entry
            .and_then(|e| e.u16s("u16s2"))
            .and_then(|v| v.get(3).copied())
            .is_some_and(|x| x != 0);
        if sf.values.refs.is_empty() || excluded {
            continue;
        }
        let i = series.len() as u32;
        let col = u32::from(grid.label_cols) + t as u32;
        let mut s = ChartSeries {
            idx: Some(i),
            order: Some(i),
            tx: series_name(&sf.name, col, entry, &grid),
            cat: cat.clone(),
            val: Some(values(&sf.values, col, &grid, book)),
            ..Default::default()
        };
        if let Some(e) = entry {
            if !pie {
                s.sp_pr = Some(series_sp_pr(e, k.family));
            }
            if matches!(k.family, Family::Line | Family::Radar) {
                s.marker = Some(series_marker(e));
            }
            s.d_lbls = series_labels(e, k.family);
        }
        match k.family {
            Family::Bar => s.invert_if_negative = Some(false),
            Family::Pie => s.explosion = Some(0),
            // 한셀은 부드러운 선(u16s3[5] = 2)을 읽지 않는다 (`c2_e6_scatter_smooth`)
            Family::Line => s.smooth = Some(false),
            _ => {}
        }
        series.push(s);
    }

    // 축: X → catAx (분산형·거품형의 X 값 축도), Y → valAx, 3차원이면 Z → serAx. 원형은 없다.
    // 이름표 형식의 한셀 특이점: 100% 차트의 값 축과 항목 축, 누적 막대(겹치기 −1)의 항목 축은 `0%`
    let mut axes = Vec::new();
    if !pie {
        let zero_pct = || ChartNumFmt {
            format_code: "0%".into(),
            source_linked: Some(false),
        };
        let stacked_bar = matches!(chart_type, 0 | 1 | 11) && f32_of("overlap") == -1.0;
        let fmt = |ax: Option<Obj<'_>>| {
            ax.and_then(label_format)
                .and_then(|tf| axis_num_fmt(tf, rec, book))
        };
        if let Some(x) = axis_x {
            let nf = if stacked_bar || percent {
                Some(zero_pct())
            } else {
                fmt(Some(x))
            };
            axes.push(axis(x, "catAx", 0, 1, "b", nf));
        }
        if let Some(y) = axis_y {
            let nf = if percent {
                Some(zero_pct())
            } else {
                fmt(Some(y))
            };
            axes.push(axis(y, "valAx", 1, 0, "l", nf));
        }
        if k.three_d
            && let Some(z) = axis_z
        {
            axes.push(axis(z, "serAx", 2, 1, "b", None));
        }
    }
    // 묶음이 쓰는 축: 0, 1 (3차원이면 2 도)
    group.ax_ids = axes.iter().map(|a| a.id).collect();

    // 제목 (VtChartTitle: 구역이 보일 때). 한셀은 위치를 저장하지 않고 xMode·yMode 만 쓴다
    let title = root.ptr("title");
    let title_full = title
        .filter(|t| t.u16("visible") == Some(1))
        .and_then(|t| t.ptr("textBlock"))
        .map(|tb| ChartTitle {
            tx: Some(ChartTx::Rich(title_rich(
                tb,
                tb.string("text").unwrap_or(""),
            ))),
            layout: Some(ChartLayout {
                manual: Some(ChartManualLayout {
                    x_mode: Some("edge".into()),
                    y_mode: Some("edge".into()),
                    ..Default::default()
                }),
            }),
            overlay: Some(false),
            sp_pr: Some(ChartShapeProps {
                fill: Some(ChartFill::None),
                line: None,
            }),
            tx_pr: None,
        });

    // 범례 (VtChartLegend: 구역이 보일 때). 위치 1 t, 3 l, 4 r (오른쪽 위도), 6 b (관측); 2·0 tr, 5·7 b 는 추정.
    // 겹침·배치·채우기·테두리는 저장되지 않는다 — 한셀은 검은 9525 테두리를 쓴다
    let legend_obj = root.ptr("legend").filter(|l| l.u16("visible") == Some(1));
    let legend = legend_obj.map(|l| {
        match l.u16("location").unwrap_or(4) {
            1 => "t",
            3 => "l",
            5..=7 => "b",
            0 | 2 => "tr",
            _ => "r",
        }
        .to_string()
    });
    let legend_props = legend_obj
        .map(|l| crate::model::ChartLegendProps {
            layout: Some(ChartLayout::default()),
            overlay: Some(false),
            sp_pr: Some(ChartShapeProps {
                fill: None,
                line: Some(ChartLine::hcell(
                    THIN,
                    ChartFill::Solid(ChartColor {
                        value: "prst:black".into(),
                        ..Default::default()
                    }),
                )),
            }),
            tx_pr: Some(ChartRich::hcell(font_of(l.ptr("font")), None, false)),
        })
        .unwrap_or_default();

    // 3차원 보기: 한셀은 저장된 회전·높이(VtChartPlot.rotation/elevation)를 쓰지 않는다
    let view3d = k.three_d.then(|| {
        let (r_ang_ax, rot_x, rot_y) = if pie { (false, 30, 0) } else { (true, 15, 20) };
        ChartView3D {
            r_ang_ax: Some(r_ang_ax),
            rot_x: Some(rot_x),
            rot_y: Some(rot_y),
            perspective: Some(30),
            depth_percent: Some(100),
            h_percent: None,
        }
    });

    // 그림 영역: 채우기는 쓰지 않고 (저장된 흰 채우기도) 테두리만, 차트 영역: 채우기 + 테두리
    let plot_backdrop = plot.and_then(|p| p.ptr("backdrop"));
    let chart_backdrop = root.ptr("backdrop");

    let mut ch = Chart {
        kind: k.name.into(),
        three_d: k.three_d,
        bar_dir: k.bar_dir.map(str::to_string),
        grouping: grouping.map(str::to_string),
        legend,
        vary_colors: pie,
        series,
        date1904: Some(false),
        rounded_corners: Some(false),
        style: Some(2),
        c14_style: Some(102),
        title_full,
        auto_title_deleted: Some(false),
        view3d,
        plot_layout: Some(ChartLayout::default()),
        group,
        axes,
        plot_sp_pr: Some(ChartShapeProps {
            fill: Some(ChartFill::None),
            line: Some(frame_line(plot_backdrop)),
        }),
        legend_props,
        plot_vis_only: Some(true),
        disp_blanks_as: Some("gap".into()),
        sp_pr: Some(ChartShapeProps {
            fill: Some(backdrop_fill(chart_backdrop)),
            line: Some(frame_line(chart_backdrop)),
        }),
        hnc_chart_style: Some(HncChartStyle {
            layout_index: Some(-1),
            color_index: Some(0),
            style_index: Some(0),
        }),
        ..Default::default()
    };
    ch.refresh_summary();
    ch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_codes_take_hancell_chart_form() {
        assert_eq!(chart_format_code("General"), "G/표준");
        assert_eq!(
            chart_format_code("\"₩\"#,##0;[Red]\\-\"₩\"#,##0"),
            "₩#,##0;[빨강]-₩#,##0"
        );
        assert_eq!(chart_format_code("0.00%"), "0.00%");
        assert_eq!(chart_format_code("m\\/yy"), "m/yy");
    }

    #[test]
    fn decodes_internal_format_codes() {
        assert_eq!(decode_format_code("Zlg표g준n"), "g/표준");
        assert_eq!(decode_format_code("B0Jn"), "0%");
        assert_eq!(decode_format_code("B0EB1B1Jn"), "0.00%");
        assert_eq!(decode_format_code("OlWn"), "m/yy");
        assert_eq!(decode_format_code("An"), "General");
        assert_eq!(decode_format_code("Q"), "\u{FFFD}");
    }

    #[test]
    fn line_widths_snap_to_presets() {
        assert_eq!(line_width(20.0), 12700);
        assert_eq!(line_width(40.0), 28575);
        assert_eq!(line_width(0.0), 3175);
        assert_eq!(line_width(120.0), 76200);
    }

    #[test]
    fn cache_numbers_like_hancell() {
        assert_eq!(num_text(20.0), "20");
        assert_eq!(num_text(-5.0), "-5");
        assert_eq!(num_text(-0.0), "0");
        assert_eq!(num_text(0.1 + 0.2), "0.3");
        assert_eq!(num_text(2.5), "2.5");
    }
}
