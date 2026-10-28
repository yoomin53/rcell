// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 두 통합 문서의 의미 비교.
//!
//! 저장 형식이 달라도(XML / 레거시 / 바이너리) 같은 문서인지 확인한다. 스타일은
//! 번호가 아니라 셀에 실제로 적용되는 값(글꼴, 채우기, 테두리, 정렬, 표시 형식)으로
//! 비교하고, 한셀이 형식마다 다르게 쓰는 기본값은 정규화한다.
//!
//! 리버스 엔지니어링 검증에 쓴다: 한셀이 레거시 파일을 읽어 XML 로 다시 저장한 결과와
//! rcell 이 같은 레거시 파일을 읽은 결과가 같아야 한다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::model::{
    AutoFilter, Border, BorderEdge, CellRange, CellValue, Color, Font, PartLink, Sheet, Styles,
    Table, Workbook,
};
use crate::xml::Element;

/// 비교 옵션.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// 셀 서식 비교 생략
    pub skip_styles: bool,
    /// 한컴 글꼴 확장의 상대 크기·장평·자간·위치 비교 생략.
    /// 한셀은 레거시 형식을 다시 열 때 이 네 값을 복원하지 않는다 (docs/format/cell-legacy.md).
    pub ignore_font_scale: bool,
    /// 한컴 확장(글꼴 hs:*, 문단 모양) 전체 비교 생략.
    /// 바이너리 OOXML(xlCellb)은 한컴 확장을 저장하지 않는다 (docs/format/cell-binary.md).
    pub ignore_hancom_ext: bool,
    /// 도형의 글·사용자 정의 모양 비교 생략. 한셀은 레거시 도형(OfficeArt + TXO)에서 이 둘을
    /// 읽지 못한다 (docs/format/cell-legacy.md). 레거시에서 한셀이 버리는 묶기 잠금(noGrp) 도형과
    /// 메모 상자의 선 색도 뺀다.
    pub ignore_shape_detail: bool,
    /// 차트 내용(종류·축·계열·제목·범례·3차원·영역) 비교 생략. 차트 도형의 앵커는 그래도 비교한다.
    pub ignore_chart: bool,
    /// 조건부 서식 비교 생략. 한셀은 레거시의 조건부 서식을 잘못 읽는다 (글 규칙 수식이 #N/A,
    /// CONDFMT12 범위가 깨짐, 차등 서식 표시 형식이 깨짐 — docs/format/cell-legacy.md).
    pub ignore_cf: bool,
    /// 자동 필터 비교 생략. 한셀은 레거시 자동 필터의 값 목록 빈 칸과 상위/하위 백분율을 읽지 못한다.
    pub ignore_filter: bool,
    /// 사용자 정의 표 스타일 비교 생략 (기본 스타일 이름만 비교). 한셀은 레거시의 표 스타일 레코드를
    /// 읽지 않는다 (docs/format/cell-legacy.md).
    pub ignore_table_styles: bool,
}

fn color_key(c: Option<&Color>) -> String {
    match c {
        None => "-".into(),
        Some(c) if c.auto || c.indexed == Some(64) => "auto".into(),
        Some(c) => match (&c.rgb, c.theme, c.indexed) {
            (Some(rgb), _, _) => {
                let rgb = rgb.to_ascii_lowercase();
                rgb.strip_prefix("ff").unwrap_or(&rgb).to_string()
            }
            (None, Some(t), _) => format!(
                "theme{t}{}",
                c.tint.map(|v| format!("{v:+.3}")).unwrap_or_default()
            ),
            (None, None, Some(i)) => format!("idx{i}"),
            _ => "-".into(),
        },
    }
}

fn font_key(f: Option<&Font>, opts: Options) -> String {
    let Some(f) = f else { return "-".into() };
    let mut parts = vec![
        format!("name={}", f.name.as_deref().unwrap_or("-")),
        format!(
            "size={}",
            f.size.map(|s| format!("{s}")).unwrap_or_default()
        ),
    ];
    if f.bold {
        parts.push("bold".into());
    }
    if f.italic {
        parts.push("italic".into());
    }
    if f.strike {
        parts.push("strike".into());
    }
    for (on, k) in [
        (f.outline, "outline"),
        (f.shadow, "shadow"),
        (f.condense, "condense"),
        (f.extend, "extend"),
    ] {
        if on {
            parts.push(k.into());
        }
    }
    if let Some(u) = &f.underline {
        parts.push(format!("u={u}"));
    }
    if let Some(v) = &f.vert_align {
        parts.push(format!("va={v}"));
    }
    let color = color_key(f.color.as_ref());
    if color != "-" && color != "auto" && color != "000000" {
        parts.push(format!("color={color}"));
    }
    if let Some(h) = f.hancom.as_ref().filter(|_| !opts.ignore_hancom_ext) {
        let mut push = |k: &str, v: String| parts.push(format!("hs.{k}={v}"));
        let scale = !opts.ignore_font_scale;
        if let Some(v) = h.size.filter(|&v| scale && v != 100 && v != 0) {
            push("size", v.to_string());
        }
        if let Some(v) = h.ratio.filter(|&v| scale && v != 100 && v != 0) {
            push("ratio", v.to_string());
        }
        if let Some(v) = h.spacing.filter(|&v| scale && v != 0) {
            push("spacing", v.to_string());
        }
        if let Some(v) = h.offset.filter(|&v| scale && v != 0) {
            push("offset", v.to_string());
        }
        for (k, v) in [
            ("emboss", h.emboss),
            ("engrave", h.engrave),
            ("useKerning", h.use_kerning),
        ] {
            if v == Some(true) {
                push(k, "1".into());
            }
        }
        if let Some(v) = &h.underline_shape {
            push("ulShape", v.clone());
        }
        if let Some(v) = &h.strikeout_shape {
            push("soShape", v.clone());
        }
        if let Some(v) = &h.outline_type {
            push("outline", v.clone());
        }
        if let Some(v) = &h.shadow_type {
            push(
                "shadow",
                format!(
                    "{v}/{}/{}/{}",
                    color_key(h.shadow_color.as_ref()),
                    h.shadow_offset_x.unwrap_or(0),
                    h.shadow_offset_y.unwrap_or(0)
                ),
            );
        }
        if let Some(v) = h.diacsym_mark.as_deref().filter(|v| *v != "none") {
            push("diacsym", v.into());
        }
        if let Some(c) = h.shade_color.as_ref() {
            let k = color_key(Some(c));
            if k != "-" && k != "ffffff" {
                push("shade", k);
            }
        }
    }
    parts.join(" ")
}

fn edge_key(e: Option<&BorderEdge>) -> String {
    match e {
        None => "-".into(),
        Some(e) => match e.style.as_deref() {
            None | Some("none") => "-".into(),
            Some(s) => format!("{s}/{}", color_key(e.color.as_ref())),
        },
    }
}

/// 차등 서식의 선: 없는 선('그대로 둠')은 "-", 스타일 없는 선('선 없음')은 "none" (선 없음의 색은 보지 않는다)
fn dxf_edge_key(e: Option<&BorderEdge>) -> String {
    match e {
        None => "-".into(),
        Some(e) => match e.style.as_deref() {
            None | Some("none") => "none".into(),
            Some(s) => format!("{s}/{}", color_key(e.color.as_ref())),
        },
    }
}

/// 차등 서식의 테두리: 선 일곱 개를 모두 쓴다 (셀 테두리와 달리 없는 선과 선 없음이 다르다)
fn dxf_border_key(b: Option<&Border>) -> String {
    let Some(b) = b else { return "-".into() };
    format!(
        "L={} R={} T={} B={} D{}{}={} V={} H={}",
        dxf_edge_key(b.left.as_ref()),
        dxf_edge_key(b.right.as_ref()),
        dxf_edge_key(b.top.as_ref()),
        dxf_edge_key(b.bottom.as_ref()),
        if b.diagonal_up { "U" } else { "" },
        if b.diagonal_down { "D" } else { "" },
        dxf_edge_key(b.diagonal.as_ref()),
        dxf_edge_key(b.vertical.as_ref()),
        dxf_edge_key(b.horizontal.as_ref()),
    )
}

fn border_key(b: Option<&Border>) -> String {
    let Some(b) = b else { return "-".into() };
    let diag = if b.diagonal_up || b.diagonal_down {
        format!(
            " diag{}{}={}",
            if b.diagonal_up { "U" } else { "" },
            if b.diagonal_down { "D" } else { "" },
            edge_key(b.diagonal.as_ref())
        )
    } else {
        String::new()
    };
    let mut inner = String::new();
    for (k, e) in [("V", &b.vertical), ("H", &b.horizontal)] {
        let v = edge_key(e.as_ref());
        if v != "-" {
            let _ = write!(inner, " {k}={v}");
        }
    }
    format!(
        "L={} R={} T={} B={}{diag}{inner}",
        edge_key(b.left.as_ref()),
        edge_key(b.right.as_ref()),
        edge_key(b.top.as_ref()),
        edge_key(b.bottom.as_ref())
    )
}

/// 표시 형식 번호의 서식 문자열. 내장 번호는 한셀(ko-KR) 표를 따른다 ([`crate::numfmt::builtin`]).
pub fn num_fmt_code(styles: &Styles, id: u32) -> String {
    styles
        .num_fmts
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.code.clone())
        .or_else(|| crate::numfmt::builtin(id).map(str::to_string))
        .unwrap_or_else(|| format!("#{id}"))
}

/// 셀 서식 번호가 실제로 뜻하는 서식을 한 줄 문자열로.
pub fn style_key(styles: &Styles, xf_index: u32, opts: Options) -> String {
    let Some(xf) = styles.xf(xf_index) else {
        return format!("xf#{xf_index}?");
    };
    let font = font_key(styles.fonts.get(xf.font_id as usize), opts);
    let fill = styles
        .fills
        .get(xf.fill_id as usize)
        .map(|f| match (&f.gradient, f.pattern_type.as_deref()) {
            (Some(g), _) => gradient_key(g),
            (None, None | Some("none")) => "-".to_string(),
            (None, Some(p)) => format!("{p}/{}", color_key(f.fg_color.as_ref())),
        })
        .unwrap_or_else(|| "-".into());
    let border = border_key(styles.borders.get(xf.border_id as usize));
    let align = xf
        .alignment
        .as_ref()
        .map(|a| {
            let mut s = format!(
                "h={} v={}",
                a.horizontal.as_deref().unwrap_or("general"),
                a.vertical.as_deref().unwrap_or("bottom")
            );
            if a.wrap_text {
                s.push_str(" wrap");
            }
            if a.shrink_to_fit {
                s.push_str(" shrink");
            }
            if let Some(i) = a.indent.filter(|&i| i != 0) {
                s.push_str(&format!(" indent={i}"));
            }
            if let Some(r) = a.text_rotation.filter(|&r| r != 0) {
                s.push_str(&format!(" rot={r}"));
            }
            if let Some(o) = a.reading_order.filter(|&o| o != 0) {
                s.push_str(&format!(" ro={o}"));
            }
            s
        })
        .unwrap_or_else(|| "h=general v=bottom".into());
    let prot = xf
        .protection
        .as_ref()
        .map(|p| {
            format!(
                "{}{}",
                if p.locked == Some(false) {
                    " unlocked"
                } else {
                    ""
                },
                if p.hidden == Some(true) {
                    " fhidden"
                } else {
                    ""
                }
            )
        })
        .unwrap_or_default();
    let para = xf
        .hancom
        .as_ref()
        .filter(|_| !opts.ignore_hancom_ext)
        .and_then(|h| h.parashape.as_ref())
        .map(|p| {
            let mut v = vec![
                format!(
                    "ls={}:{}",
                    p.line_spacing_type.as_deref().unwrap_or("-"),
                    p.line_spacing.unwrap_or(0)
                ),
                format!(
                    "brk={}/{}",
                    p.break_latin_word.as_deref().unwrap_or("-"),
                    p.break_non_latin_word.as_deref().unwrap_or("-")
                ),
                format!("ta={}", p.text_alignment.as_deref().unwrap_or("-")),
            ];
            for (k, x) in [
                ("lm", p.left_margin),
                ("rm", p.right_margin),
                ("ind", p.indentation),
                ("prev", p.prev_spacing),
                ("next", p.next_spacing),
                ("cond", p.condense),
            ] {
                if let Some(x) = x.filter(|&x| x != 0) {
                    v.push(format!("{k}={x}"));
                }
            }
            if p.font_line_height == Some(true) {
                v.push("flh".into());
            }
            format!(" para[{}]", v.join(" "))
        })
        .unwrap_or_default();
    format!(
        "font[{font}] fill[{fill}] border[{border}] align[{align}] fmt[{}]{prot}{para}{}",
        num_fmt_code(styles, xf.num_fmt_id),
        if xf.quote_prefix { " '" } else { "" }
    )
}

/// 셀 수식: 원문, 배열 수식은 범위, 데이터 표는 범위와 입력 (2변수 표의 행 입력 표시는 뜻이 없어 뺀다 — 한셀은
/// 쓰지 않는다). 공유 수식은 저장 방식일 뿐이라 원문만 본다
fn formula_key(f: &crate::model::Formula) -> String {
    use crate::model::FormulaKind;
    match (f.kind, &f.data_table) {
        (FormulaKind::Array, _) => format!(
            "{{{}}} 배열 {}",
            unquote_external(&f.text),
            norm_ranges(f.range.as_deref())
        ),
        (FormulaKind::DataTable, Some(d)) => format!(
            "데이터 표 {} {}{}{}{}{}",
            norm_ranges(f.range.as_deref()),
            if d.two_d {
                "2변수"
            } else if d.row_input {
                "행 입력"
            } else {
                "열 입력"
            },
            d.r1.as_deref()
                .map(|r| format!(" r1={r}"))
                .unwrap_or_default(),
            d.r2.as_deref()
                .map(|r| format!(" r2={r}"))
                .unwrap_or_default(),
            if d.del1 { " del1" } else { "" },
            if d.del2 { " del2" } else { "" }
        ),
        _ => unquote_external(&f.text),
    }
}

fn value_key(v: &CellValue) -> String {
    match v {
        CellValue::Empty => "∅".into(),
        CellValue::Number(n) => format!("n:{}", crate::model::format_general(*n)),
        CellValue::Text(s) => format!("s:{s:?}"),
        CellValue::Bool(b) => format!("b:{b}"),
        CellValue::Error(e) => format!("e:{e}"),
        CellValue::Date(d) => format!("d:{d}"),
    }
}

/// 차이 목록. 비어 있으면 같은 문서다.
/// 그리기 개체 하나를 한 줄로 (자식은 괄호 안에)
/// 그리기 개체 하나를 한 줄로. 그림은 파일 바이트가 아니라 형식·픽셀 크기로 본다
/// (한셀은 저장할 때마다 PNG 를 다시 압축한다).
fn shape_key(
    s: &crate::model::Shape,
    parts: &[crate::model::RawPart],
    no_text: bool,
    no_chart: bool,
) -> String {
    let a = &s.anchor;
    // 오프셋은 픽셀로 (한셀은 내부 실수 계산 때문에 1 EMU 씩 어긋나기도 한다)
    let px = |emu: i64| (emu as f64 / crate::units::EMU_PER_PX as f64).round() as i64;
    let m = |m: &crate::model::Marker| {
        format!(
            "{}+{}px/{}+{}px",
            m.col,
            px(m.col_off),
            m.row,
            px(m.row_off)
        )
    };
    let paint = |p: &Option<crate::model::Paint>| match p {
        None => "-".to_string(),
        Some(crate::model::Paint::None) => "none".into(),
        Some(crate::model::Paint::Solid(c)) => {
            let mods: Vec<String> = c.mods.iter().map(|(k, v)| format!("{k}={v}")).collect();
            if mods.is_empty() {
                c.value.clone()
            } else {
                format!("{}[{}]", c.value, mods.join(","))
            }
        }
        Some(crate::model::Paint::Other(n)) => n.clone(),
    };
    // 화살표가 없는 끝은 없음과 같다 (한셀은 레거시에서 읽은 선에 늘 `w="med" len="med"` 를 쓴다)
    let end = |e: &Option<crate::model::LineEnd>| match e {
        Some(e) if e.kind.as_deref().is_some_and(|k| k != "none") => format!(
            "{}:{}:{}",
            e.kind.as_deref().unwrap_or("-"),
            e.w.as_deref().unwrap_or("med"),
            e.len.as_deref().unwrap_or("med")
        ),
        _ => "-".into(),
    };
    let line = s.line.as_ref().map_or("-".to_string(), |l| {
        format!(
            "{}/{}/{}/{}/{}/{} {}>{}",
            l.width.map_or("-".into(), |w| w.to_string()),
            paint(&l.fill),
            l.dash.as_deref().unwrap_or("-"),
            l.cap.as_deref().unwrap_or("-"),
            l.compound.as_deref().unwrap_or("-"),
            l.join.as_deref().unwrap_or("-"),
            end(&l.head),
            end(&l.tail)
        )
    });
    // 그림자: 거리는 100 EMU 단위로 (한셀은 레거시의 X·Y 거리에서 다시 계산해 1 EMU 씩 다르다), 기준점이 없으면 b
    let shadow = s.shadow.as_ref().map_or("-".to_string(), |d| {
        format!(
            "{}/{}/{}/{}",
            (d.dist as f64 / 100.0).round() as i64,
            d.dir,
            d.align.as_deref().unwrap_or("b"),
            paint(&Some(crate::model::Paint::Solid(d.color.clone())))
        )
    });
    let client = format!(
        "{}{}",
        if s.locks_with_sheet.unwrap_or(true) {
            "L"
        } else {
            "-"
        },
        if s.prints_with_sheet.unwrap_or(true) {
            "P"
        } else {
            "-"
        }
    );
    let overflow = s.text.as_ref().map_or(String::new(), |t| {
        format!(
            " overflow={}/{}",
            t.vert_overflow.as_deref().unwrap_or("overflow"),
            t.horz_overflow.as_deref().unwrap_or("overflow")
        )
    });
    let children = if s.children.is_empty() {
        String::new()
    } else {
        format!(
            " ({})",
            s.children
                .iter()
                .map(|c| shape_key(c, parts, no_text, no_chart))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    format!(
        "{:?} {:?} [{}] {}→{} {} fill={} line={} shadow={} locks={:?} client={} text={:?}{}{}",
        s.kind,
        s.name,
        a.edit_as.as_deref().unwrap_or("twoCell"),
        m(&a.from),
        a.to.as_ref().map_or("-".into(), m),
        s.geometry
            .as_deref()
            .filter(|g| !(no_text && *g == "custom"))
            .unwrap_or("-"),
        paint(&s.fill),
        line,
        shadow,
        s.locks,
        client,
        if no_text {
            String::new()
        } else {
            s.plain_text()
        },
        if no_text { "" } else { &overflow },
        children
    ) + &match (&s.chart, no_chart) {
        (Some(c), false) => format!(" chart={}", chart_key(c)),
        _ => String::new(),
    } + &match s.kind {
        crate::model::ShapeKind::Picture => {
            let img = s
                .target
                .as_ref()
                .and_then(|t| parts.iter().find(|p| &p.name == t))
                .map_or("-".to_string(), |p| match crate::image::info(&p.data) {
                    Some(i) => format!("{} {}x{}", i.format, i.width, i.height),
                    None => format!("? {}B", p.data.len()),
                });
            format!(" img={img} descr={:?}", s.descr.as_deref().unwrap_or(""))
        }
        _ => String::new(),
    }
}

/// 차트를 한 줄로 (그리기 목록용 요약): 종류·방향·묶음·제목·범례·계열 수. 자세한 비교는 [`chart_key::aspects`]
fn chart_key(c: &crate::model::Chart) -> String {
    format!(
        "{}{} {}/{} title={:?} legend={} series={}",
        c.kind,
        if c.three_d { "3D" } else { "" },
        c.bar_dir.as_deref().unwrap_or("-"),
        c.grouping.as_deref().unwrap_or("-"),
        c.title.as_deref().unwrap_or(""),
        c.legend.as_deref().unwrap_or("-"),
        c.series.len() + c.more_groups.iter().map(|g| g.series.len()).sum::<usize>()
    )
}

/// 시트의 차트 도형 (그룹 안까지, 그리기 순서)
fn sheet_charts(sh: &Sheet) -> Vec<Option<&crate::model::Chart>> {
    fn walk<'a>(list: &'a [crate::model::Shape], out: &mut Vec<Option<&'a crate::model::Chart>>) {
        for s in list {
            if s.kind == crate::model::ShapeKind::Chart {
                out.push(s.chart.as_ref());
            }
            walk(&s.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&sh.shapes, &mut out);
    out
}

/// 차트의 자세한 비교: 항목(종류·제목·범례·축·계열·3차원·영역·기타)마다 한 줄.
///
/// 한셀이 그리는 결과가 달라지는 차이는 보이고, 표현만 다른 것은 같게 본다: 없는 요소 = 스키마·한셀 기본값
/// (`overlay`·`invertIfNegative`·`smooth` 없음 = 0, `orientation` 없음 = `minMax` …), 16진 색의 대소문자,
/// 캐시 숫자의 표기 (`20` = `20.0`), 축 번호 (XML 원본은 임의 번호 — 교차 축은 순서로 본다), 위치 없는
/// `manualLayout` = 자동 배치, 보이지 않는 데이터 레이블의 위치·형식 (한셀은 XML 을 읽을 때 꺾은선 계열의 숨은
/// 레이블에 `dLblPos r` 를 넣는다 — docs/format/cell-2024.md 11.2절).
mod chart_key {
    use crate::model::{
        Chart, ChartAxis, ChartColor, ChartData, ChartDataLabels, ChartFill, ChartFont,
        ChartGroupProps, ChartLayout, ChartLine, ChartLines, ChartMarker, ChartNumFmt, ChartRich,
        ChartSeries, ChartShapeProps, ChartSurface, ChartTitle, ChartTx,
    };

    fn b(v: Option<bool>, default: bool) -> &'static str {
        if v.unwrap_or(default) { "1" } else { "0" }
    }

    fn o<T: std::fmt::Display>(v: &Option<T>) -> String {
        v.as_ref().map_or_else(|| "-".into(), |x| x.to_string())
    }

    /// 숫자 글은 값으로 (`20` = `20.0` = `2E1`)
    fn num(s: &str) -> String {
        match s.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => format!("{v}"),
            _ => s.to_string(),
        }
    }

    fn color(c: &ChartColor) -> String {
        let mut s = match c.value.split_once(':') {
            Some(("srgb", h)) => format!("srgb:{}", h.to_ascii_uppercase()),
            _ => c.value.clone(),
        };
        for (k, m) in &c.mods {
            s.push_str(&format!("[{k}={m}]"));
        }
        s
    }

    fn fill(f: &Option<ChartFill>) -> String {
        match f {
            None => "-".into(),
            Some(ChartFill::None) => "none".into(),
            Some(ChartFill::Solid(c)) => color(c),
            Some(ChartFill::Other(x)) => format!("other:{x}"),
        }
    }

    /// 선: 두께·채우기·점선 (없는 점선 = 실선). 끝 모양·꺾임은 차트 선에서 보이지 않는다
    fn line(l: &Option<ChartLine>) -> String {
        l.as_ref().map_or_else(
            || "-".into(),
            |l| {
                format!(
                    "{}/{}/{}",
                    o(&l.width),
                    fill(&l.fill),
                    l.dash.as_deref().unwrap_or("solid")
                )
            },
        )
    }

    fn sp(p: &Option<ChartShapeProps>) -> String {
        p.as_ref().map_or_else(
            || "-".into(),
            |p| format!("fill={} ln={}", fill(&p.fill), line(&p.line)),
        )
    }

    fn font(f: Option<&ChartFont>) -> String {
        f.map_or_else(
            || "-".into(),
            |f| {
                format!(
                    "sz={} b={} i={} u={} strike={} base={} color={} face={}",
                    o(&f.size),
                    b(f.bold, false),
                    b(f.italic, false),
                    f.underline.as_deref().unwrap_or("none"),
                    f.strike.as_deref().unwrap_or("noStrike"),
                    f.baseline.unwrap_or(0),
                    fill(&f.fill),
                    f.latin.as_deref().unwrap_or("-")
                )
            },
        )
    }

    /// 글 상자: 방향, 문단마다 조각(글 + 글꼴) — 조각이 없으면 문단 기본 글꼴
    fn rich(r: &ChartRich) -> String {
        let paras: Vec<String> = r
            .paras
            .iter()
            .map(|p| {
                if p.runs.is_empty() {
                    format!("[{}]", font(p.def_font.as_ref()))
                } else {
                    p.runs
                        .iter()
                        .map(|x| {
                            format!(
                                "{:?}[{}]",
                                x.text,
                                font(x.font.as_ref().or(p.def_font.as_ref()))
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("")
                }
            })
            .collect();
        format!(
            "vert={} rot={} {}",
            r.vert.as_deref().unwrap_or("horz"),
            r.rot.unwrap_or(0),
            paras.join(" / ")
        )
    }

    fn opt_rich(r: &Option<ChartRich>) -> String {
        r.as_ref().map_or_else(|| "-".into(), rich)
    }

    fn data(d: Option<&ChartData>) -> String {
        let Some(d) = d else { return "-".into() };
        let cache = d.cache.as_ref().map_or_else(
            || "-".into(),
            |c| {
                let pts: Vec<String> = c
                    .points
                    .iter()
                    .map(|p| match &p.format_code {
                        Some(f) => format!("{}={}@{f}", p.idx, num(&p.value)),
                        None => format!("{}={}", p.idx, num(&p.value)),
                    })
                    .collect();
                format!(
                    "n={} fmt={} [{}]",
                    o(&c.pt_count),
                    c.format_code.as_deref().unwrap_or("-"),
                    pts.join(" ")
                )
            },
        );
        format!(
            "{} {} {}",
            d.kind,
            d.formula.as_deref().unwrap_or("-"),
            cache
        )
    }

    fn tx(t: &Option<ChartTx>) -> String {
        match t {
            None => "-".into(),
            Some(ChartTx::Rich(r)) => format!("rich {}", rich(r)),
            Some(ChartTx::Ref(d)) => format!("ref {}", data(Some(d))),
            Some(ChartTx::Value(v)) => format!("v {v:?}"),
        }
    }

    /// 배치: 위치·크기가 없으면 자동 배치와 같다
    fn layout(l: &Option<ChartLayout>) -> String {
        match l.as_ref().and_then(|l| l.manual.as_ref()) {
            Some(m) if m.x.is_some() || m.y.is_some() || m.w.is_some() || m.h.is_some() => {
                let r =
                    |v: Option<f64>| v.map_or_else(|| "-".into(), |v| format!("{:.6}", v as f32));
                format!(
                    "manual {}/{}/{}/{} {}/{}/{}/{} {}",
                    o(&m.x_mode),
                    o(&m.y_mode),
                    o(&m.w_mode),
                    o(&m.h_mode),
                    r(m.x),
                    r(m.y),
                    r(m.w),
                    r(m.h),
                    o(&m.layout_target)
                )
            }
            _ => "auto".into(),
        }
    }

    fn title(t: &Option<ChartTitle>) -> String {
        t.as_ref().map_or_else(
            || "-".into(),
            |t| {
                format!(
                    "{} overlay={} layout={} sp={} txPr={}",
                    tx(&t.tx),
                    b(t.overlay, false),
                    layout(&t.layout),
                    sp(&t.sp_pr),
                    opt_rich(&t.tx_pr)
                )
            },
        )
    }

    fn num_fmt(n: &Option<ChartNumFmt>) -> String {
        n.as_ref().map_or_else(
            || "-".into(),
            |n| format!("{:?}/linked={}", n.format_code, b(n.source_linked, false)),
        )
    }

    fn lines(l: &Option<ChartLines>) -> String {
        l.as_ref()
            .map_or_else(|| "-".into(), |l| format!("on sp={}", sp(&l.sp_pr)))
    }

    /// 데이터 레이블. 보이는 것이 없으면(없음, 삭제, 표시가 모두 0) 같은 "hidden"
    fn labels(d: &Option<ChartDataLabels>) -> String {
        let Some(d) = d else { return "hidden".into() };
        let shown = [
            d.show_legend_key,
            d.show_val,
            d.show_cat_name,
            d.show_ser_name,
            d.show_percent,
            d.show_bubble_size,
        ]
        .contains(&Some(true));
        if d.delete == Some(true) || !shown {
            return "hidden".into();
        }
        format!(
            "pos={} key={} val={} cat={} ser={} pct={} bubble={} sep={:?} leader={} numFmt={} sp={} txPr={}",
            o(&d.d_lbl_pos),
            b(d.show_legend_key, false),
            b(d.show_val, false),
            b(d.show_cat_name, false),
            b(d.show_ser_name, false),
            b(d.show_percent, false),
            b(d.show_bubble_size, false),
            d.separator,
            b(d.show_leader_lines, false),
            num_fmt(&d.num_fmt),
            sp(&d.sp_pr),
            opt_rich(&d.tx_pr)
        )
    }

    fn marker(m: &Option<ChartMarker>) -> String {
        m.as_ref().map_or_else(
            || "-".into(),
            |m| format!("{}/{} sp={}", o(&m.symbol), o(&m.size), sp(&m.sp_pr)),
        )
    }

    fn series(s: &ChartSeries) -> String {
        let pts: Vec<String> = s
            .d_pts
            .iter()
            .map(|p| {
                format!(
                    "{}:inv={} marker={} bubble3D={} explosion={} sp={}",
                    p.idx,
                    b(p.invert_if_negative, false),
                    marker(&p.marker),
                    b(p.bubble3d, false),
                    p.explosion.unwrap_or(0),
                    sp(&p.sp_pr)
                )
            })
            .collect();
        format!(
            "idx={} order={} tx={} sp={} invertIfNegative={} marker={} explosion={} dPt=[{}] dLbls={} cat={} val={} bubbleSize={} shape={} bubble3D={} smooth={}",
            o(&s.idx),
            o(&s.order),
            tx(&s.tx),
            sp(&s.sp_pr),
            b(s.invert_if_negative, false),
            marker(&s.marker),
            s.explosion.unwrap_or(0),
            pts.join("; "),
            labels(&s.d_lbls),
            data(s.cat.as_ref()),
            data(s.val.as_ref()),
            data(s.bubble_size.as_ref()),
            o(&s.shape),
            b(s.bubble3d, false),
            b(s.smooth, false)
        )
    }

    /// 묶음 (`c:barChart` …): 종류와 묶음 수준 값, 쓰는 축은 축 목록의 순서로
    fn group(
        name: String,
        bar_dir: &Option<String>,
        grouping: &Option<String>,
        vary_colors: bool,
        g: &ChartGroupProps,
        axis_index: &dyn Fn(u32) -> String,
    ) -> String {
        let axes: Vec<String> = g.ax_ids.iter().map(|&i| axis_index(i)).collect();
        format!(
            "{name} barDir={} grouping={} varyColors={} gapWidth={} overlap={} gapDepth={} shape={} marker={} smooth={} radarStyle={} scatterStyle={} ofPie={} wireframe={} firstSliceAng={} holeSize={} split={}/{}/{} bubble={}/{}/{}/{} dLbls={} serLines={} dropLines={} hiLowLines={} axes=[{}]",
            o(bar_dir),
            o(grouping),
            u8::from(vary_colors),
            o(&g.gap_width),
            o(&g.overlap),
            o(&g.gap_depth),
            o(&g.shape),
            o(&g.marker.map(u8::from)),
            b(g.smooth, false),
            o(&g.radar_style),
            o(&g.scatter_style),
            o(&g.of_pie_type),
            b(g.wireframe, false),
            o(&g.first_slice_ang),
            o(&g.hole_size),
            o(&g.split_type),
            o(&g.split_pos),
            o(&g.second_pie_size),
            b(g.bubble3d, false),
            o(&g.bubble_scale),
            o(&g.show_neg_bubbles.map(u8::from)),
            o(&g.size_represents),
            labels(&g.d_lbls),
            lines(&g.ser_lines),
            lines(&g.drop_lines),
            lines(&g.hi_low_lines),
            axes.join(",")
        )
    }

    fn axis(a: &ChartAxis, axis_index: &dyn Fn(u32) -> String) -> String {
        let s = &a.scaling;
        format!(
            "{} delete={} pos={} scaling={}/{}/{}/{} crossAx={} crosses={} crossesAt={} crossBetween={} major={} minor={} title={} numFmt={} tick={}/{} tickLblPos={} sp={} txPr={} unit={}/{} auto={} lblAlgn={} lblOffset={} skip={}/{} noMultiLvlLbl={} time={}/{}/{}",
            a.kind,
            b(a.delete, false),
            o(&a.ax_pos),
            s.orientation.as_deref().unwrap_or("minMax"),
            o(&s.min),
            o(&s.max),
            o(&s.log_base),
            a.cross_ax.map_or_else(|| "-".into(), axis_index),
            if a.crosses_at.is_some() {
                "-"
            } else {
                a.crosses.as_deref().unwrap_or("autoZero")
            },
            o(&a.crosses_at),
            o(&a.cross_between),
            lines(&a.major_gridlines),
            lines(&a.minor_gridlines),
            title(&a.title),
            num_fmt(&a.num_fmt),
            a.major_tick_mark.as_deref().unwrap_or("cross"),
            a.minor_tick_mark.as_deref().unwrap_or("cross"),
            a.tick_lbl_pos.as_deref().unwrap_or("nextTo"),
            sp(&a.sp_pr),
            opt_rich(&a.tx_pr),
            o(&a.major_unit),
            o(&a.minor_unit),
            o(&a.auto.map(u8::from)),
            o(&a.lbl_algn),
            o(&a.lbl_offset),
            o(&a.tick_lbl_skip),
            o(&a.tick_mark_skip),
            o(&a.no_multi_lvl_lbl.map(u8::from)),
            o(&a.base_time_unit),
            o(&a.major_time_unit),
            o(&a.minor_time_unit)
        )
    }

    fn surface(x: &Option<ChartSurface>) -> String {
        x.as_ref().map_or_else(
            || "-".into(),
            |x| format!("{}/{}", o(&x.thickness), sp(&x.sp_pr)),
        )
    }

    /// (항목, 값) — 두 차트에서 같은 이름의 항목끼리 비교한다
    pub(super) fn aspects(c: &Chart) -> Vec<(String, String)> {
        // 축 번호 → 축 목록의 순서 (XML 원본의 번호는 임의다)
        let ids: Vec<u32> = c.axes.iter().map(|a| a.id).collect();
        let axis_index = move |id: u32| {
            ids.iter()
                .position(|&x| x == id)
                .map_or_else(|| format!("?{id}"), |p| (p + 1).to_string())
        };
        let mut out = Vec::new();
        let kind = |k: &str, three_d: bool| format!("{k}{}", if three_d { "3D" } else { "" });
        if !c.kind.is_empty() {
            out.push((
                "묶음 1".to_string(),
                group(
                    kind(&c.kind, c.three_d),
                    &c.bar_dir,
                    &c.grouping,
                    c.vary_colors,
                    &c.group,
                    &axis_index,
                ),
            ));
            for (k, s) in c.series.iter().enumerate() {
                out.push((format!("계열 1.{}", k + 1), series(s)));
            }
        }
        for (g, x) in c.more_groups.iter().enumerate() {
            out.push((
                format!("묶음 {}", g + 2),
                group(
                    kind(&x.kind, x.three_d),
                    &x.bar_dir,
                    &x.grouping,
                    x.vary_colors,
                    &x.props,
                    &axis_index,
                ),
            ));
            for (k, s) in x.series.iter().enumerate() {
                out.push((format!("계열 {}.{}", g + 2, k + 1), series(s)));
            }
        }
        for (i, a) in c.axes.iter().enumerate() {
            out.push((format!("축 {}", i + 1), axis(a, &axis_index)));
        }
        out.push((
            "제목".into(),
            format!(
                "{} autoTitleDeleted={}",
                title(&c.title_full),
                b(c.auto_title_deleted, false)
            ),
        ));
        out.push((
            "범례".into(),
            match &c.legend {
                None => "-".into(),
                Some(pos) => {
                    let l = &c.legend_props;
                    format!(
                        "{pos} overlay={} layout={} sp={} txPr={}",
                        b(l.overlay, false),
                        layout(&l.layout),
                        sp(&l.sp_pr),
                        opt_rich(&l.tx_pr)
                    )
                }
            },
        ));
        let v = c.view3d.as_ref();
        out.push((
            "3차원".into(),
            format!(
                "{} floor={} side={} back={}",
                v.map_or_else(
                    || "-".into(),
                    |v| format!(
                        "rAngAx={} rotX={} rotY={} perspective={} depth={} height={}",
                        b(v.r_ang_ax, true),
                        v.rot_x.unwrap_or(0),
                        v.rot_y.unwrap_or(0),
                        v.perspective.unwrap_or(30),
                        v.depth_percent.unwrap_or(100),
                        v.h_percent.unwrap_or(100)
                    )
                ),
                surface(&c.floor),
                surface(&c.side_wall),
                surface(&c.back_wall)
            ),
        ));
        out.push((
            "영역".into(),
            format!(
                "chart={} plot={} plotLayout={} roundedCorners={}",
                sp(&c.sp_pr),
                sp(&c.plot_sp_pr),
                layout(&c.plot_layout),
                o(&c.rounded_corners.map(u8::from))
            ),
        ));
        out.push((
            "기타".into(),
            format!(
                "date1904={} plotVisOnly={} dispBlanksAs={} style={}/{} hnc={} txPr={}",
                b(c.date1904, false),
                b(c.plot_vis_only, true),
                c.disp_blanks_as.as_deref().unwrap_or("zero"),
                o(&c.style),
                o(&c.c14_style),
                c.hnc_chart_style.as_ref().map_or_else(
                    || "-".into(),
                    |h| format!(
                        "{}/{}/{}",
                        o(&h.layout_index),
                        o(&h.color_index),
                        o(&h.style_index)
                    )
                ),
                opt_rich(&c.tx_pr)
            ),
        ));
        out
    }
}

/// 차등 서식(dxf)을 한 줄로
fn dxf_key(styles: &Styles, id: Option<u32>) -> String {
    let Some(d) = id.and_then(|i| styles.dxfs.get(i as usize)) else {
        return "-".into();
    };
    let font = d.font.as_ref().map_or("-".into(), |f| {
        format!(
            "{}{}{}{}{}{}/{}",
            if f.bold { "b" } else { "" },
            if f.italic { "i" } else { "" },
            // 한셀의 XML 저장은 차등 서식의 이중 회계 밑줄을 단일 회계로 쓴다 (바이너리·레거시는 0x22 를 지킨다,
            // 차등 서식 대조 실험) — 회계 밑줄 둘은 같게 본다
            f.underline.as_deref().map_or(String::new(), |u| match u {
                "singleAccounting" | "doubleAccounting" => "u:accounting".into(),
                u => format!("u:{u}"),
            }),
            if f.strike { "s" } else { "" },
            f.vert_align
                .as_deref()
                .map_or(String::new(), |v| format!(" {v}")),
            f.size.map_or(String::new(), |v| format!(" {v}pt")),
            color_key(f.color.as_ref())
        )
    });
    // 맞춤: 들여쓰기는 한셀이 차등 서식에 남기지 않는다
    let align = d.alignment.as_ref().map_or("-".into(), |a| {
        format!(
            "{}/{}{}{}{}",
            a.horizontal.as_deref().unwrap_or("-"),
            a.vertical.as_deref().unwrap_or("-"),
            if a.wrap_text { " wrap" } else { "" },
            a.text_rotation
                .filter(|&r| r != 0)
                .map_or(String::new(), |r| format!(" rot{r}")),
            if a.shrink_to_fit { " shrink" } else { "" }
        )
    });
    let fill = d.fill.as_ref().map_or("-".into(), |f| match &f.gradient {
        Some(g) => gradient_key(g),
        None => format!(
            "{}/{}/{}",
            f.pattern_type.as_deref().unwrap_or("-"),
            color_key(f.fg_color.as_ref()),
            color_key(f.bg_color.as_ref())
        ),
    });
    format!(
        "dxf[font={font} fill={fill} border={} fmt={} align={align}{}]",
        dxf_border_key(d.border.as_ref()),
        d.num_fmt.as_ref().map_or("-", |n| n.code.as_str()),
        if d.protection.is_some() { " prot" } else { "" }
    )
}

/// 그라데이션: 종류(각도 또는 경로 사각형)와 정지점(위치:색)
fn gradient_key(g: &crate::model::GradientFill) -> String {
    let stops: Vec<String> = g
        .stops
        .iter()
        .map(|s| format!("{:.3}:{}", s.position, color_key(Some(&s.color))))
        .collect();
    if g.path {
        format!(
            "path({:.3},{:.3},{:.3},{:.3})[{}]",
            g.left,
            g.right,
            g.top,
            g.bottom,
            stops.join(" ")
        )
    } else {
        format!("linear{}[{}]", g.degree, stops.join(" "))
    }
}

/// 셀 스타일 목록: 기본 제공은 번호(수준은 행·열 윤곽 스타일 1·2 만), 사용자 정의는 이름.
/// 한셀은 셀 서식이 쓰는 스타일과 표준 스타일만 저장한다 — 레거시에 쓰는 BIFF 상용구(쉼표·통화·백분율)와
/// 쓰이지 않는 사용자 정의 스타일은 다시 저장할 때 사라진다. 그래서 쓰이는 것만 비교한다
fn cell_styles_key(st: &Styles) -> String {
    let used: BTreeSet<u32> = st.cell_xfs.iter().filter_map(|x| x.xf_id).collect();
    let set: BTreeSet<String> = st
        .cell_styles
        .iter()
        .filter(|c| c.builtin_id == Some(0) || used.contains(&c.xf_id))
        .map(|c| match c.builtin_id {
            Some(b) => format!(
                "builtin{b}{}",
                c.i_level
                    .filter(|_| matches!(b, 1 | 2))
                    .map_or(String::new(), |l| format!("/{l}"))
            ),
            None => format!("{:?}", c.name),
        })
        .collect();
    // 셀 스타일이 없으면 쓸 때 표준 스타일 하나를 넣는다 (writer)
    if st.cell_styles.is_empty() {
        return "builtin0".into();
    }
    set.into_iter().collect::<Vec<_>>().join(" ")
}

/// 표 스타일: 기본 이름, 그리고 스타일마다 쓰임새와 요소(종류·폭·차등 서식 내용)
fn table_styles_key(st: &Styles, defaults_only: bool) -> String {
    let Some(t) = &st.table_styles else {
        return "-".into();
    };
    let mut s = format!(
        "default={:?}/{:?}",
        t.default_table_style, t.default_pivot_style
    );
    if defaults_only {
        return s;
    }
    let mut styles: Vec<String> = t
        .styles
        .iter()
        .map(|ts| {
            let mut els: Vec<String> = ts
                .elements
                .iter()
                .map(|e| format!("{}:{}={}", e.kind, e.size, dxf_key(st, e.dxf_id)))
                .collect();
            els.sort();
            format!(
                "{:?}{}{} [{}]",
                ts.name,
                if ts.pivot { " pivot" } else { "" },
                if ts.table { " table" } else { "" },
                els.join(", ")
            )
        })
        .collect();
    styles.sort();
    for x in styles {
        let _ = write!(
            s,
            "
      {x}"
        );
    }
    s
}

/// 조건부 서식 전체를 (범위, 우선순위) 순으로 한 줄씩
fn cf_key(styles: &Styles, sh: &Sheet) -> String {
    let cfvo = |c: &crate::model::Cfvo| {
        format!(
            "{}{}{}",
            c.kind,
            c.val
                .as_deref()
                .map(|v| format!("={v}"))
                .unwrap_or_default(),
            if c.gte { "" } else { "(>)" }
        )
    };
    let mut rules: Vec<(String, u32, String)> = Vec::new();
    for cf in &sh.conditional_formats {
        let sq = norm_ranges(Some(&cf.sqref));
        for r in &cf.rules {
            // 글 규칙은 한셀이 containsText + operator 로 바꿔 쓰기도 한다
            let kind = match r.kind.as_str() {
                "containsText" | "notContainsText" | "beginsWith" | "endsWith" => {
                    format!("text:{}", r.operator.as_deref().unwrap_or(&r.kind))
                }
                k => k.to_string(),
            };
            let mut k = format!("{kind} op={}", r.operator.as_deref().unwrap_or("-"));
            if !r.formulas.is_empty() {
                let _ = write!(k, " f={:?}", r.formulas);
            }
            if let Some(t) = &r.text {
                let _ = write!(k, " text={t:?}");
            }
            if r.kind == "top10" {
                let _ = write!(
                    k,
                    " rank={}{}{}",
                    r.rank.unwrap_or(10),
                    if r.percent { "%" } else { "" },
                    if r.bottom { " bottom" } else { "" }
                );
            }
            if r.kind == "aboveAverage" {
                let _ = write!(
                    k,
                    " above={} eq={} sd={:?}",
                    r.above_average.unwrap_or(true),
                    r.equal_average,
                    r.std_dev
                );
            }
            if let Some(c) = &r.color_scale {
                let v: Vec<String> = c.cfvos.iter().map(cfvo).collect();
                let col: Vec<String> = c.colors.iter().map(|x| color_key(Some(x))).collect();
                let _ = write!(k, " scale={v:?}{col:?}");
            }
            if let Some(d) = &r.data_bar {
                let v: Vec<String> = d.cfvos.iter().map(cfvo).collect();
                let _ = write!(
                    k,
                    " bar={v:?}{} show={} len={}-{}",
                    color_key(d.color.as_ref()),
                    d.show_value,
                    d.min_length.unwrap_or(10),
                    d.max_length.unwrap_or(90)
                );
            }
            if let Some(i) = &r.icon_set {
                let v: Vec<String> = i.cfvos.iter().map(cfvo).collect();
                let _ = write!(
                    k,
                    " icons={}{v:?} show={} rev={}",
                    i.set, i.show_value, i.reverse
                );
            }
            if r.stop_if_true {
                k.push_str(" stop");
            }
            if let Some(t) = &r.time_period {
                let _ = write!(k, " period={t}");
            }
            let _ = write!(k, " {}", dxf_key(styles, r.dxf_id));
            rules.push((sq.clone(), r.priority, k));
        }
    }
    rules.sort();
    rules
        .iter()
        .map(|(sq, p, k)| format!("{sq}#{p} {k}"))
        .collect::<Vec<_>>()
        .join("\n      ")
}

/// 조건부 서식의 x14 확장 (레거시에 남지 않는 정보): 데이터 막대의 확장 — 없으면 한셀이 저장할 때 만드는 값
/// ([`crate::model::DataBarExt::implied`]) — 과 아이콘 집합의 사용자 지정 아이콘.
/// 색은 자동과 검정을 같게 본다: 한셀은 바이너리에 둘을 같은 바이트로 쓴다 (x14 조건부 서식 실험)
fn x14_key(sh: &Sheet) -> String {
    let color = |c: &Option<Color>| match color_key(c.as_ref()).as_str() {
        "auto" => "000000".to_string(),
        k => k.to_string(),
    };
    let mut rules: Vec<(String, u32, String)> = Vec::new();
    for cf in &sh.conditional_formats {
        let sq = norm_ranges(Some(&cf.sqref));
        for r in &cf.rules {
            if let Some(d) = &r.data_bar {
                let e = d
                    .ext
                    .clone()
                    .unwrap_or_else(|| crate::model::DataBarExt::implied(d));
                let v: Vec<String> = e
                    .cfvos
                    .iter()
                    .map(|c| {
                        format!(
                            "{}{}",
                            c.kind,
                            c.val
                                .as_deref()
                                .map(|v| format!("={v}"))
                                .unwrap_or_default()
                        )
                    })
                    .collect();
                let colors: Vec<String> = [
                    &e.border_color,
                    &e.negative_fill_color,
                    &e.negative_border_color,
                    &e.axis_color,
                ]
                .into_iter()
                .map(color)
                .collect();
                rules.push((
                    sq.clone(),
                    r.priority,
                    format!(
                        "bar {v:?} border={} gradient={} dir={} negSame={} negBorderSame={} axis={} colors={colors:?}",
                        e.border,
                        e.gradient,
                        e.direction.as_deref().unwrap_or("context"),
                        e.negative_same_as_positive,
                        e.negative_border_same_as_positive,
                        e.axis_position.as_deref().unwrap_or("automatic")
                    ),
                ));
            }
            if let Some(i) = r.icon_set.as_ref().filter(|i| !i.custom_icons.is_empty()) {
                rules.push((
                    sq.clone(),
                    r.priority,
                    format!("icons={:?}", i.custom_icons),
                ));
            }
        }
    }
    rules.sort();
    rules
        .iter()
        .map(|(sq, p, k)| format!("{sq}#{p} {k}"))
        .collect::<Vec<_>>()
        .join("\n      ")
}

/// 데이터 유효성 전체를 범위 순으로
fn dv_key(sh: &Sheet) -> String {
    let mut v: Vec<String> = sh
        .data_validations
        .iter()
        .map(|d| {
            format!(
                "{} {} {} f1={:?} f2={:?} blank={} nodrop={} in={} err={} {} {:?}/{:?} {:?}/{:?}",
                norm_ranges(Some(&d.sqref)),
                d.kind,
                d.operator.as_deref().unwrap_or("between"),
                d.formula1.as_deref().unwrap_or(""),
                d.formula2.as_deref().unwrap_or(""),
                d.allow_blank,
                d.hide_drop_down,
                d.show_input,
                d.show_error,
                d.error_style.as_deref().unwrap_or("stop"),
                d.error_title.as_deref().unwrap_or(""),
                d.error.as_deref().unwrap_or(""),
                d.prompt_title.as_deref().unwrap_or(""),
                d.prompt.as_deref().unwrap_or("")
            )
        })
        .collect();
    v.sort();
    v.join("\n      ")
}

/// 자동 필터
///
/// 와일드카드 없는 `equal` 조건만 `or` 로 이은 사용자 지정 필터는 같은 값 목록 필터로 본다
/// (한셀은 값 두 개 이하의 목록을 레거시에 그렇게 저장하고, 다시 읽을 때 사용자 지정으로 보여 준다).
fn filter_key(sh: &Sheet) -> String {
    let af = sh.auto_filter.as_ref().map_or("-".into(), af_key);
    format!("filterMode={} {af}", sh.filter_mode)
}

/// 자동 필터 범위와 열 조건 (시트와 표가 함께 쓴다)
fn af_key(f: &AutoFilter) -> String {
    {
        let cols: Vec<String> = f
            .columns
            .iter()
            .map(|c| {
                let as_list = c.values.is_empty()
                    && !c.custom.is_empty()
                    && (!c.custom_and || c.custom.len() == 1)
                    && c.custom
                        .iter()
                        .all(|(op, v)| op == "equal" && !v.contains(['*', '?', '~']));
                let (values, custom): (Vec<&String>, &[(String, String)]) = if as_list {
                    (c.custom.iter().map(|(_, v)| v).collect(), &[])
                } else {
                    (c.values.iter().collect(), &c.custom)
                };
                // 동적 필터의 val·maxVal 은 없으면 0 (한셀은 종류에 따라 val="0" 을 쓰기도 안 쓰기도 한다)
                let dynamic = c.dynamic.as_ref().map(|t| {
                    format!(
                        "{t}({},{})",
                        c.dynamic_val.unwrap_or(0.0),
                        c.dynamic_max_val.unwrap_or(0.0)
                    )
                });
                format!(
                    "#{} values={:?}{} custom={:?}{} top10={:?} dyn={:?}",
                    c.col_id,
                    values,
                    if c.blank { "+blank" } else { "" },
                    custom,
                    if c.custom_and && !custom.is_empty() {
                        "(and)"
                    } else {
                        ""
                    },
                    c.top10,
                    dynamic
                )
            })
            .collect();
        let sort = f
            .sort_state
            .as_ref()
            .map(sort_state_key)
            .unwrap_or_default();
        format!(
            "{} [{}]{sort}",
            norm_ranges(Some(&f.range)),
            cols.join("; ")
        )
    }
}

/// 정렬 상태 (범위, 왼쪽에서 오른쪽, 대소문자, 조건들)
fn sort_state_key(st: &crate::model::SortState) -> String {
    let conds: Vec<String> = st
        .conditions
        .iter()
        .map(|c| {
            format!(
                "{}{}{}",
                norm_ranges(Some(&c.range)),
                if c.descending { " desc" } else { "" },
                c.sort_by
                    .as_deref()
                    .filter(|b| *b != "value")
                    .map(|b| format!(" by={b}"))
                    .unwrap_or_default()
            )
        })
        .collect();
    format!(
        " sort[{}{}{}: {}]",
        norm_ranges(Some(&st.range)),
        if st.column_sort { " cols" } else { "" },
        if st.case_sensitive { " case" } else { "" },
        conds.join(", ")
    )
}

/// 표: 이름·번호·범위·머리글/요약 행·종류·열(요약 함수·글·수식, 계산 열)·자동 필터·스타일. 차등 서식은 번호가
/// 아니라 내용으로 본다 (한셀은 레거시를 읽을 때 같은 내용이 된 차등 서식을 하나로 합쳐 번호가 바뀐다).
/// `no_format` = 차등 서식·셀 스타일 생략
fn tables_key(styles: &Styles, sh: &Sheet, no_format: bool) -> String {
    let mut v: Vec<String> = sh
        .tables
        .iter()
        .map(|t| table_key(styles, t, no_format))
        .collect();
    v.sort();
    v.join("\n      ")
}

fn table_key(styles: &Styles, t: &Table, no_format: bool) -> String {
    let dxfs = |names: &[&str], ids: &[Option<u32>]| {
        let mut s = String::new();
        if !no_format {
            for (n, id) in names.iter().zip(ids) {
                if id.is_some() {
                    let _ = write!(s, " {n}={}", dxf_key(styles, *id));
                }
            }
        }
        s
    };
    let cols: Vec<String> = t
        .columns
        .iter()
        .map(|c| {
            let mut s = format!("{}:{}", c.id, c.name);
            for (k, v) in [
                ("고유", &c.unique_name),
                ("요약", &c.totals_function),
                ("요약 글", &c.totals_label),
                ("요약 수식", &c.totals_formula),
                ("계산", &c.calculated_formula),
            ] {
                if let Some(v) = v {
                    let _ = write!(s, " {k}={v:?}");
                }
            }
            if let Some(q) = c.query_table_field_id {
                let _ = write!(s, " 쿼리 필드={q}");
            }
            s + &dxfs(&["머리글", "데이터", "요약"], &c.dxfs)
        })
        .collect();
    let mut s = format!(
        "{}({}) #{} {} 머리글 {} 요약 {}{}",
        t.name,
        t.display_name,
        t.id,
        t.range,
        t.header_rows,
        t.totals_rows,
        if t.totals_shown { "" } else { " 요약 숨김" }
    );
    if let Some(v) = &t.table_type {
        let _ = write!(s, " 종류={v}");
    }
    if let Some(v) = &t.comment {
        let _ = write!(s, " 설명={v:?}");
    }
    if let Some(v) = t.connection_id {
        let _ = write!(s, " 연결={v}");
    }
    let _ = write!(
        s,
        " 필터={} 열=[{}]",
        t.auto_filter.as_ref().map_or("-".into(), af_key),
        cols.join("; ")
    );
    if let Some(st) = &t.style {
        let _ = write!(
            s,
            " 스타일={}{}{}{}{}",
            st.name.as_deref().unwrap_or("-"),
            if st.first_column { " 첫 열" } else { "" },
            if st.last_column { " 끝 열" } else { "" },
            if st.row_stripes { " 행 줄무늬" } else { "" },
            if st.column_stripes {
                " 열 줄무늬"
            } else {
                ""
            }
        );
    }
    s.push_str(&dxfs(
        &[
            "머리글",
            "데이터",
            "요약",
            "테두리",
            "머리글 테두리",
            "요약 테두리",
        ],
        &t.dxfs,
    ));
    if !no_format {
        for (n, v) in ["머리글", "데이터", "요약"].iter().zip(&t.cell_styles) {
            if let Some(v) = v {
                let _ = write!(s, " {n} 셀 스타일={v:?}");
            }
        }
    }
    s
}

/// 원본 그대로 옮겨 쓴 파트의 XML
fn raw_xml(w: &Workbook, part: &str) -> Option<Element> {
    let p = w.raw_parts.iter().find(|p| p.name == part)?;
    let s = String::from_utf8_lossy(&p.data);
    crate::xml::parse(s.trim_start_matches('\u{feff}'), part).ok()
}

/// 피벗 표 (모델의 XML 파트): 이름, 위치, 캐시의 원본·필드(공유 항목 수, 계산 필드 수식), 필드 축·숨긴 항목·부분합,
/// 행·열·페이지(고른 항목)·데이터 필드, 스타일, 총합계, 값 머리글. 데이터 필드의 함수는 없으면 sum (한셀은 늘 쓴다)
fn pivots_key(w: &Workbook, sh: &Sheet) -> String {
    let mut v: Vec<String> = sh
        .part_links
        .iter()
        .filter(|l| l.rel_type.ends_with("/pivotTable"))
        .map(|l| pivot_key(w, &l.part))
        .collect();
    v.sort();
    v.join("\n      ")
}

fn pivot_key(w: &Workbook, part: &str) -> String {
    let Some(root) = raw_xml(w, part) else {
        return format!("{part}?");
    };
    let a = |e: &Element, n: &str| e.attr(n).unwrap_or("").to_string();
    let cache = root
        .attr_u32("cacheId")
        .and_then(|id| w.part_links.iter().find(|l| l.cache_id == Some(id)))
        .and_then(|l| raw_xml(w, &l.part))
        .map_or("-".into(), |c| {
            let src = c.sml_child("cacheSource").map_or(String::new(), |s| {
                let ws = s.sml_child("worksheetSource").map_or(String::new(), |x| {
                    format!(" {}!{}{}", a(x, "sheet"), a(x, "ref"), a(x, "name"))
                });
                format!("{}{ws}", a(s, "type"))
            });
            // 필드 이름, 공유 항목 수, 계산 필드 수식
            let fields: Vec<String> = c
                .sml_child("cacheFields")
                .into_iter()
                .flat_map(|f| f.sml_children("cacheField"))
                .map(|f| {
                    let n = f
                        .sml_child("sharedItems")
                        .map_or(0, |s| s.elements().count());
                    let fm = f.attr("formula").map_or(String::new(), |x| format!("={x}"));
                    format!("{}#{n}{fm}", a(f, "name"))
                })
                .collect();
            format!("{src} [{}]", fields.join(","))
        });
    let loc = root.sml_child("location").map_or(String::new(), |l| {
        format!(
            "{} {}/{}/{}",
            a(l, "ref"),
            a(l, "firstHeaderRow"),
            a(l, "firstDataRow"),
            a(l, "firstDataCol")
        )
    });
    let fields: Vec<String> = root
        .sml_child("pivotFields")
        .into_iter()
        .flat_map(|f| f.sml_children("pivotField"))
        .map(|f| {
            // 축, 데이터 필드, 숨긴 항목, 부분합 (기본 부분합이 없거나 다른 함수)
            let hidden: Vec<String> = f
                .sml_child("items")
                .into_iter()
                .flat_map(|i| i.sml_children("item"))
                .enumerate()
                .filter(|(_, i)| i.attr_bool("h") == Some(true))
                .map(|(k, _)| k.to_string())
                .collect();
            let subs: Vec<&str> = f
                .sml_child("items")
                .into_iter()
                .flat_map(|i| i.sml_children("item"))
                .filter_map(|i| i.attr("t"))
                .collect();
            format!(
                "{}{}{}{}",
                f.attr("axis").unwrap_or("-"),
                if f.attr_bool("dataField") == Some(true) {
                    "+data"
                } else {
                    ""
                },
                if hidden.is_empty() {
                    String::new()
                } else {
                    format!(" 숨김{}", hidden.join("/"))
                },
                if subs.is_empty() || subs == ["default"] {
                    String::new()
                } else {
                    format!(" 부분합{}", subs.join("/"))
                }
            )
        })
        .collect();
    let list = |n: &str, c: &str, at: &str| {
        root.sml_child(n)
            .into_iter()
            .flat_map(|f| f.sml_children(c))
            .map(|f| a(f, at))
            .collect::<Vec<_>>()
            .join(",")
    };
    let data: Vec<String> = root
        .sml_child("dataFields")
        .into_iter()
        .flat_map(|f| f.sml_children("dataField"))
        .map(|d| {
            format!(
                "{}={}:{}",
                a(d, "name"),
                a(d, "fld"),
                d.attr("subtotal").unwrap_or("sum")
            )
        })
        .collect();
    // 페이지 필드: 필드와 고른 항목
    let pages: Vec<String> = root
        .sml_child("pageFields")
        .into_iter()
        .flat_map(|f| f.sml_children("pageField"))
        .map(|p| match p.attr("item") {
            Some(i) => format!("{}:{i}", a(p, "fld")),
            None => a(p, "fld"),
        })
        .collect();
    let grand = |n: &str| {
        if root.attr_bool(n) == Some(false) {
            "0"
        } else {
            "1"
        }
    };
    format!(
        "{} @{loc} 캐시={cache} 필드=[{}] 행=[{}] 열=[{}] 페이지=[{}] 데이터=[{}] 스타일={} 총합계={}{} 값={}",
        a(&root, "name"),
        fields.join(","),
        list("rowFields", "field", "x"),
        list("colFields", "field", "x"),
        pages.join(","),
        data.join(";"),
        root.sml_child("pivotTableStyleInfo")
            .map_or(String::new(), |s| a(s, "name")),
        grand("rowGrandTotals"),
        grand("colGrandTotals"),
        a(&root, "dataCaption")
    )
}

/// 원본 그대로 옮긴 파트의 관계 종류 (피벗 표·피벗 캐시·한컴 매크로는 따로 비교한다)
fn links_key(links: &[PartLink]) -> String {
    let mut v: Vec<&str> = links
        .iter()
        .filter(|l| {
            !l.rel_type.ends_with("/pivotTable")
                && !l.rel_type.ends_with("/pivotCacheDefinition")
                && l.rel_type != crate::ns::REL_HANCOM_VBS_PROJECT
        })
        .map(|l| l.rel_type.rsplit('/').next().unwrap_or(""))
        .collect();
    v.sort();
    v.join(" ")
}

/// 한컴 매크로 프로젝트 (`_VBS_PROJECT_W` 스트림): 길이, 바이트 지문(FNV-1a), 글 (머리 0x104 바이트 뒤
/// 글자 수 u32 + UTF-16 — 패키지 파트 실험)
fn macro_key(w: &Workbook) -> String {
    let Some(b) = &w.hancom.vbs_project else {
        return "-".into();
    };
    let n = b
        .get(0x104..0x108)
        .map_or(0, |x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize);
    let units: Vec<u16> = b
        .get(0x108..)
        .unwrap_or(&[])
        .as_chunks::<2>()
        .0
        .iter()
        .take(n)
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b.iter() {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let text: String = String::from_utf16_lossy(&units).chars().take(80).collect();
    format!("{} 바이트 {h:016x} {text:?}", b.len())
}

/// 두 시트에 모두 있는 상위/하위 기준값(filterVal)을 열 번호 순으로
fn filter_vals(a: &Sheet, b: &Sheet) -> (String, String) {
    let cols = |s: &Sheet| -> Vec<(u32, f64)> {
        s.auto_filter
            .iter()
            .flat_map(|f| &f.columns)
            .filter_map(|c| c.filter_val.map(|v| (c.col_id, v)))
            .collect()
    };
    let (ca, cb) = (cols(a), cols(b));
    let pick = |x: &[(u32, f64)], y: &[(u32, f64)]| {
        x.iter()
            .filter(|(c, _)| y.iter().any(|(d, _)| d == c))
            .map(|(c, v)| format!("#{c}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    (pick(&ca, &cb), pick(&cb, &ca))
}

/// 개요 요약 행·열의 위치
fn summary_key(sh: &Sheet) -> String {
    format!(
        "{}/{}",
        if sh.summary_above { "above" } else { "below" },
        if sh.summary_left { "left" } else { "right" },
    )
}

/// 시트 보호: OOXML 기본값을 채운 16개 항목 + 암호 해시.
/// 잠기지 않은 셀 선택을 막으면 잠긴 셀 선택도 막힌다 (한셀·Excel 이 저장할 때 그렇게 맞춘다).
fn protection_key(p: Option<&crate::model::SheetProtection>) -> String {
    let Some(p) = p else { return "-".into() };
    let v: Vec<String> = crate::model::SHEET_PROTECTION_FLAGS
        .iter()
        .map(|(k, _)| {
            let on = p.flag(k) || (*k == "selectLockedCells" && p.flag("selectUnlockedCells"));
            format!("{k}={}", on as u8)
        })
        .collect();
    format!(
        "pw={} {}",
        p.password
            .as_deref()
            .map(str::to_ascii_uppercase)
            .unwrap_or_default(),
        v.join(" ")
    )
}

/// 시트 참조 `'시트'!`·`'[n]시트'!` 의 따옴표가 시트 이름 때문에 필요한 것이 아니면 뗀다 (문자열 안은 그대로).
/// 한셀은 외부 연결의 상태(경로 없음, 갱신 오류)에 따라 외부 참조의 따옴표를 붙이기도 떼기도 하고, 바이너리를 거친
/// 수식은 필요한 따옴표만 남는다 — 같은 참조다.
fn unquote_external(f: &str) -> String {
    let c: Vec<char> = f.chars().collect();
    let mut out = String::with_capacity(f.len());
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '"' => {
                // 문자열: 닫는 따옴표까지 그대로 ("" 는 따옴표 하나)
                out.push('"');
                i += 1;
                while i < c.len() {
                    out.push(c[i]);
                    i += 1;
                    if c[i - 1] == '"' {
                        if c.get(i) == Some(&'"') {
                            out.push('"');
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
            }
            '\'' => {
                // 따옴표 시트 이름: 닫는 ' 뒤에 ! 가 와야 참조다
                let mut j = i + 1;
                let mut inner = String::new();
                while j < c.len() {
                    if c[j] == '\'' {
                        if c.get(j + 1) == Some(&'\'') {
                            inner.push('\'');
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    inner.push(c[j]);
                    j += 1;
                }
                if j < c.len() && c.get(j + 1) == Some(&'!') {
                    let sheet = match inner.strip_prefix('[').and_then(|r| r.split_once(']')) {
                        Some((n, sh)) if n.chars().all(|d| d.is_ascii_digit()) => sh,
                        _ => inner.as_str(),
                    };
                    let plain = !sheet.is_empty()
                        && !sheet.contains(':')
                        && crate::legacy::formula::quote_sheet(sheet) == sheet;
                    if plain {
                        out.push_str(&inner);
                    } else {
                        out.push('\'');
                        out.push_str(&inner.replace('\'', "''"));
                        out.push('\'');
                    }
                    i = j + 1;
                } else {
                    out.push(c[i]);
                    i += 1;
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// 범위 목록을 같은 꼴로 (`A1:A1` → `A1`)
fn norm_ranges(s: Option<&str>) -> String {
    s.unwrap_or("A1")
        .split_whitespace()
        .map(|r| CellRange::parse(r).map_or_else(|| r.to_string(), |c| c.to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 시트 보기를 기본값을 채운 한 줄로.
///
/// `zoomScaleNormal`·`zoomScaleSheetLayoutView` 는 비교하지 않는다: 한셀은 같은 XML 을 가진 시트에도
/// 형식마다 다른 값을 쓴다 (문서에 드러나지 않는 상태가 있다, docs/format/cell-binary.md).
/// 여백은 1/1000 인치까지 본다 (한셀이 바이너리·레거시에 쓸 때 HWPUNIT 로 자른다).
fn view_key(sh: &Sheet) -> String {
    let v = &sh.view;
    let pane = v.pane.as_ref().map(|p| {
        format!(
            "pane[{}x{} {} {} {}]",
            p.x_split,
            p.y_split,
            norm_ranges(p.top_left_cell.as_deref()),
            p.active_pane.as_deref().unwrap_or("topLeft"),
            p.state.as_deref().unwrap_or("split")
        )
    });
    format!(
        "grid={} hdr={} zeros={} fmla={} sel={} view={} tl={} zoom={} act={} sq={} {}",
        sh.show_grid_lines,
        v.show_row_col_headers,
        v.show_zeros,
        v.show_formulas,
        v.tab_selected,
        v.view.as_deref().unwrap_or("normal"),
        norm_ranges(v.top_left_cell.as_deref()),
        v.zoom_scale.unwrap_or(100),
        norm_ranges(v.active_cell.as_deref()),
        norm_ranges(v.sqref.as_deref()),
        pane.unwrap_or_default()
    )
}

/// 머리글·바닥글 여백 (pageMargins 가 없으면 OOXML 기본값 0.3)
fn hf_margins(sh: &Sheet) -> (f64, f64) {
    sh.page
        .margins
        .as_ref()
        .map_or((0.3, 0.3), |m| (m.header, m.footer))
}

/// 인쇄 설정을 기본값을 채운 한 줄로. `hf` 를 주면 머리글·바닥글 여백 대신 그 값을 쓴다
fn page_key(sh: &Sheet, hf: Option<(f64, f64)>) -> String {
    let p = &sh.page;
    // printOptions 가 없으면 모두 꺼짐과 같다 (한셀은 켠 것이 있을 때만 쓴다)
    let o = p.print_options.clone().unwrap_or_default();
    let po = format!(
        "center={}/{} headings={} grid={}",
        o.horizontal_centered, o.vertical_centered, o.headings, o.grid_lines
    );
    // pageMargins 가 없으면 OOXML 기본값
    let m = p.margins.clone().unwrap_or(crate::model::PageMargins {
        left: 0.7,
        right: 0.7,
        top: 0.75,
        bottom: 0.75,
        header: 0.3,
        footer: 0.3,
    });
    let (header, footer) = hf.unwrap_or((m.header, m.footer));
    let m = format!(
        "margin={:.3}/{:.3}/{:.3}/{:.3} hf={header:.3}/{footer:.3}",
        m.left, m.right, m.top, m.bottom
    );
    let su = p.setup.as_ref().map_or("-".into(), |s| {
        format!(
            "paper={} scale={} first={} fit={}x{} {} {} prn={} bw={} draft={} usefirst={} dpi={}x{} copies={}",
            s.paper_size.unwrap_or(1),
            s.scale.unwrap_or(100),
            s.first_page_number.unwrap_or(1),
            s.fit_to_width.unwrap_or(1),
            s.fit_to_height.unwrap_or(1),
            s.orientation.as_deref().unwrap_or("default"),
            s.page_order.as_deref().unwrap_or("downThenOver"),
            s.use_printer_defaults,
            s.black_and_white,
            s.draft,
            s.use_first_page_number,
            s.horizontal_dpi.unwrap_or(600),
            s.vertical_dpi.unwrap_or(600),
            s.copies.unwrap_or(1)
        )
    });
    // headerFooter 가 없으면 기본값 (scaleWithDoc·alignWithMargins 참)
    let h = p
        .header_footer
        .clone()
        .unwrap_or(crate::model::HeaderFooter {
            scale_with_doc: true,
            align_with_margins: true,
            ..Default::default()
        });
    let hf = Some(&h).map_or(String::new(), |h| {
        let t = |x: &Option<String>| x.as_deref().unwrap_or("").to_string();
        format!(
            "oddeven={} first={} scale={} align={} H={:?} F={:?} eH={:?} eF={:?} fH={:?} fF={:?}",
            h.different_odd_even,
            h.different_first,
            h.scale_with_doc,
            h.align_with_margins,
            t(&h.odd_header),
            t(&h.odd_footer),
            t(&h.even_header),
            t(&h.even_footer),
            t(&h.first_header),
            t(&h.first_footer)
        )
    });
    format!(
        "fit={} autobreak={} {po} {m} {su} {hf} rowbrk={:?} colbrk={:?}",
        p.fit_to_page.unwrap_or(false),
        p.auto_page_breaks.unwrap_or(true),
        p.row_breaks,
        p.col_breaks
    )
}

pub fn compare(a: &Workbook, b: &Workbook, opts: Options) -> Vec<String> {
    let mut out = Vec::new();
    let mut diff = |what: String, x: String, y: String| {
        if x != y {
            out.push(format!("{what}\n    A: {x}\n    B: {y}"));
        }
    };

    let names = |w: &Workbook| {
        w.sheets
            .iter()
            .map(|s| format!("{}({:?})", s.name, s.state))
            .collect::<Vec<_>>()
            .join(", ")
    };
    diff("시트".into(), names(a), names(b));

    let dn = |w: &Workbook| {
        let set: BTreeSet<String> = w
            .defined_names
            .iter()
            .filter(|d| !d.name.starts_with("_xlnm._FilterDatabase"))
            .map(|d| {
                format!(
                    "{}@{:?}{}={}{}",
                    d.name,
                    d.local_sheet_id,
                    if d.hidden { "(숨김)" } else { "" },
                    unquote_external(&d.formula),
                    d.comment
                        .as_deref()
                        .map(|c| format!(" //{c:?}"))
                        .unwrap_or_default()
                )
            })
            .collect();
        set.into_iter().collect::<Vec<_>>().join("; ")
    };
    diff("정의된 이름".into(), dn(a), dn(b));
    let wp = |w: &Workbook| {
        w.protection.as_ref().map_or("-".into(), |p| {
            format!(
                "structure={} windows={} pw={}",
                p.lock_structure,
                p.lock_windows,
                p.password
                    .as_deref()
                    .map(str::to_ascii_uppercase)
                    .unwrap_or_default()
            )
        })
    };
    diff("통합 문서 보호".into(), wp(a), wp(b));
    // 문서 속성: 저장할 때마다 바뀌는 수정 날짜·수정한 사람·수정 번호는 빼고, 날짜는 초까지
    let props = |w: &Workbook| {
        let p = &w.properties;
        let t = |d: &Option<String>| d.as_deref().map(|s| s.chars().take(19).collect::<String>());
        format!(
            "title={:?} subject={:?} creator={:?} keywords={:?} description={:?} category={:?} status={:?} created={:?} printed={:?}",
            p.title,
            p.subject,
            p.creator,
            p.keywords,
            p.description,
            p.category,
            p.content_status,
            t(&p.created),
            t(&p.last_printed)
        )
    };
    diff("문서 속성".into(), props(a), props(b));
    // 통합 문서 창: 위치·크기(1/20 pt), 탭 비율, 첫 탭, 스크롤 막대·시트 탭 표시. 활성 탭은 시트 보기의 선택으로 본다
    let wv = |w: &Workbook| {
        let v = w.view.clone().unwrap_or_default();
        format!(
            "{},{} {}x{} tabRatio={} first={} hscroll={} vscroll={} tabs={}",
            v.x_window,
            v.y_window,
            v.window_width,
            v.window_height,
            v.tab_ratio,
            v.first_sheet,
            v.show_horizontal_scroll,
            v.show_vertical_scroll,
            v.show_sheet_tabs
        )
    };
    diff("통합 문서 보기".into(), wv(a), wv(b));
    // 편집 시간 (app.xml TotalTime, 분). 한셀은 원본 값을 그대로 쓴다 (없으면 0)
    diff(
        "편집 시간".into(),
        a.total_time.unwrap_or(0).to_string(),
        b.total_time.unwrap_or(0).to_string(),
    );
    // 코드 이름은 둘 다 있을 때만 (한셀은 없는 코드 이름을 ThisWorkbook 으로 채운다)
    if let (Some(x), Some(y)) = (&a.code_name, &b.code_name) {
        diff("통합 문서 코드 이름".into(), x.clone(), y.clone());
    }
    // 테마: 색 12개와 글꼴. 레거시에는 테마가 없다 (한셀은 다시 열 때 자기 기본 테마를 붙인다) → 둘 다 있을 때만
    if let (Some(ta), Some(tb)) = (&a.theme, &b.theme) {
        let tk = |t: &crate::theme::Theme| {
            format!(
                "{} major={:?} minor={:?}",
                t.colors.join(" ").to_ascii_uppercase(),
                t.major_font,
                t.minor_font
            )
        };
        diff("테마".into(), tk(ta), tk(tb));
    }
    if !opts.skip_styles {
        diff(
            "셀 스타일".into(),
            cell_styles_key(&a.styles),
            cell_styles_key(&b.styles),
        );
        diff(
            "표 스타일".into(),
            table_styles_key(&a.styles, opts.ignore_table_styles),
            table_styles_key(&b.styles, opts.ignore_table_styles),
        );
    }

    // 외부 연결은 시트 이름, 그리고 상태(경로 없음 여부, 값 캐시)를 비교한다 (경로의 드라이브는 한셀이
    // 문서 위치로 채운다). 캐시가 없는 시트 = 오류 없는 빈 캐시 (한셀이 레거시를 열 때 그렇게 채운다).
    // 상태는 한셀이 바이너리에 쓰지 않는다 (docs/format/cell-binary.md)
    let links = |w: &Workbook| {
        w.external_links
            .iter()
            .map(|l| format!("[{}]", l.sheet_names.join("|")))
            .collect::<Vec<_>>()
            .join(" ")
    };
    diff("외부 연결".into(), links(a), links(b));
    let caches = |w: &Workbook| {
        w.external_links
            .iter()
            .map(|l| {
                let cache: Vec<String> = l
                    .sheet_data
                    .iter()
                    .filter(|d| d.refresh_error || !d.cells.is_empty())
                    .map(|d| {
                        let cells: Vec<String> =
                            d.cells.iter().map(|(r, v)| format!("{r}={v:?}")).collect();
                        format!(
                            "{}{}:{}",
                            d.sheet_id,
                            if d.refresh_error { "!" } else { "" },
                            cells.join(",")
                        )
                    })
                    .collect();
                format!(
                    "[{}{}]",
                    if l.path_missing { "경로 없음 " } else { "" },
                    cache.join(" ")
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    diff("외부 연결 상태".into(), caches(a), caches(b));
    diff(
        "패키지 파트".into(),
        links_key(&a.part_links),
        links_key(&b.part_links),
    );
    diff("매크로 프로젝트".into(), macro_key(a), macro_key(b));

    for (sa, sb) in a.sheets.iter().zip(&b.sheets) {
        let s = &sa.name;
        let cells = |w: &Workbook,
                     sh: &crate::model::Sheet|
         -> BTreeMap<(u32, u32), (String, String, String)> {
            sh.cells
                .iter()
                .map(|c| {
                    let style = if opts.skip_styles {
                        String::new()
                    } else {
                        style_key(&w.styles, c.style, opts)
                    };
                    let f = c.formula.as_ref().map(formula_key).unwrap_or_default();
                    ((c.r.row, c.r.col), (value_key(&c.value), f, style))
                })
                .collect()
        };
        let ca = cells(a, sa);
        let cb = cells(b, sb);
        // 셀이 없는 쪽은 행 서식 → 열 서식 → 기본 서식을 쓴다 (한셀 화면과 같다)
        let empty = |w: &Workbook, sh: &crate::model::Sheet, k: (u32, u32)| {
            let style = if opts.skip_styles {
                String::new()
            } else {
                style_key(&w.styles, sh.effective_style(k.0, k.1), opts)
            };
            ("∅".to_string(), String::new(), style)
        };
        let keys: BTreeSet<_> = ca.keys().chain(cb.keys()).copied().collect();
        for k in keys {
            let r = crate::model::CellRef { row: k.0, col: k.1 };
            let (empty_a, empty_b);
            let x = match ca.get(&k) {
                Some(x) => x,
                None => {
                    empty_a = empty(a, sa, k);
                    &empty_a
                }
            };
            let y = match cb.get(&k) {
                Some(y) => y,
                None => {
                    empty_b = empty(b, sb, k);
                    &empty_b
                }
            };
            diff(format!("{s}!{r} 값"), x.0.clone(), y.0.clone());
            diff(format!("{s}!{r} 수식"), x.1.clone(), y.1.clone());
            diff(format!("{s}!{r} 서식"), x.2.clone(), y.2.clone());
        }

        let merges = |sh: &crate::model::Sheet| {
            let set: BTreeSet<String> = sh.merges.iter().map(|m| m.to_string()).collect();
            set.into_iter().collect::<Vec<_>>().join(" ")
        };
        diff(format!("{s} 병합"), merges(sa), merges(sb));
        let notes = |sh: &Sheet| {
            let mut v: Vec<String> = sh
                .comments
                .iter()
                .map(|c| {
                    format!(
                        "{}[{}{}]{:?}",
                        c.cell,
                        c.author,
                        if c.visible { " 표시" } else { "" },
                        c.text.text
                    )
                })
                .collect();
            v.sort();
            v.join(" ")
        };
        diff(format!("{s} 메모"), notes(sa), notes(sb));
        // 메모 상자: 자리(px)·채우기·선·그림자·셀 고정·글 맞춤. 선 색은 한셀이 레거시에서 읽지 못한다
        // (메모 대조 실험) — 도형 세부와 함께 뺀다. 1 pt 선은 0.75 pt 로 본다: 한셀은 1 pt(VML 기본값)를 XML 에 쓰지
        // 않고, 두께가 없으면 0.75 pt 로 읽는다
        // 이름은 한셀이 세 형식에서 지키는 도형 이름 (메모 이름 실험), 없으면 rcell 이 쓰는 `메모 k`
        let note_boxes = |sh: &Sheet| {
            let mut v: Vec<String> = sh
                .comments
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    let b = c
                        .shape
                        .clone()
                        .unwrap_or_else(|| crate::model::CommentShape::implied(c.cell));
                    format!(
                        "{}@{:?} {:?} fill={} line={}/{}pt{}{}{} align={}/{}",
                        c.cell,
                        b.anchor,
                        b.name_or(k),
                        b.fill_color,
                        if opts.ignore_shape_detail {
                            "-"
                        } else {
                            &b.stroke_color
                        },
                        if b.stroke_weight == 1.0 {
                            0.75
                        } else {
                            b.stroke_weight
                        },
                        if b.shadow { " shadow" } else { "" },
                        if b.move_with_cells { " move" } else { "" },
                        if b.size_with_cells { " size" } else { "" },
                        b.text_halign.as_deref().unwrap_or("Left"),
                        b.text_valign.as_deref().unwrap_or("Top")
                    )
                })
                .collect();
            v.sort();
            v.join("\n      ")
        };
        diff(format!("{s} 메모 상자"), note_boxes(sa), note_boxes(sb));
        // 도형 세부를 뺄 때는 묶기 잠금(noGrp) 도형도 뺀다: 한셀은 레거시를 읽을 때 이 도형을 버린다 (도형 대조 실험)
        let shapes = |w: &Workbook, sh: &Sheet| {
            sh.shapes
                .iter()
                .filter(|s| !(opts.ignore_shape_detail && s.locks.iter().any(|l| l == "noGrp")))
                .map(|s| shape_key(s, &w.raw_parts, opts.ignore_shape_detail, opts.ignore_chart))
                .collect::<Vec<_>>()
                .join("\n      ")
        };
        diff(format!("{s} 그리기"), shapes(a, sa), shapes(b, sb));
        if !opts.ignore_chart {
            // 차트는 그리기 순서로 짝짓고 항목마다 비교한다
            let (ca, cb) = (sheet_charts(sa), sheet_charts(sb));
            for i in 0..ca.len().max(cb.len()) {
                let get = |v: &[Option<&crate::model::Chart>]| {
                    v.get(i)
                        .copied()
                        .flatten()
                        .map(chart_key::aspects)
                        .unwrap_or_default()
                };
                let (xa, xb) = (get(&ca), get(&cb));
                let mut names: Vec<&String> = xa.iter().map(|(k, _)| k).collect();
                for (k, _) in &xb {
                    if !names.contains(&k) {
                        names.push(k);
                    }
                }
                let find = |x: &[(String, String)], k: &str| {
                    x.iter()
                        .find(|(n, _)| n == k)
                        .map_or_else(|| "-".to_string(), |(_, v)| v.clone())
                };
                for k in names {
                    diff(
                        format!("{s} 차트 {} {k}", i + 1),
                        find(&xa, k),
                        find(&xb, k),
                    );
                }
            }
        }
        if !opts.ignore_cf {
            diff(
                format!("{s} 조건부 서식"),
                cf_key(&a.styles, sa),
                cf_key(&b.styles, sb),
            );
            diff(format!("{s} x14 확장"), x14_key(sa), x14_key(sb));
        }
        diff(format!("{s} 유효성"), dv_key(sa), dv_key(sb));
        diff(format!("{s} 보기"), view_key(sa), view_key(sb));
        // 코드 이름이 한쪽에만 있으면 비교하지 않는다: 한셀은 바이너리·레거시로 저장할 때
        // 없는 코드 이름을 채워 넣는다
        let (code_a, code_b) = match (&sa.code_name, &sb.code_name) {
            (Some(x), Some(y)) => (Some(x), Some(y)),
            _ => (None, None),
        };
        // 한셀은 저장할 때마다 머리글·바닥글 여백을 HWPUNIT(1/7200 인치) 하나씩 잃는다 (잘라 읽기, 관측:
        // 0.511805 → 0.511666 → 0.511527 …). 판정 비교의 양쪽은 한셀을 거친 횟수가 다를 수 있어 한 단위
        // 안의 차이는 같은 값으로 본다
        let (ha, hb) = (hf_margins(sa), hf_margins(sb));
        let near = |x: f64, y: f64| ((x - y) * 7200.0).abs() <= 1.5;
        let hf_b = (near(ha.0, hb.0) && near(ha.1, hb.1)).then_some(ha);
        diff(format!("{s} 인쇄"), page_key(sa, None), page_key(sb, hf_b));
        diff(
            format!("{s} 시트 속성"),
            format!("{:?} tab={}", code_a, color_key(sa.tab_color.as_ref())),
            format!("{:?} tab={}", code_b, color_key(sb.tab_color.as_ref())),
        );

        // 이웃한 같은 열 정보는 하나로 본다: 한셀은 열 서식 번호가 다르면 같은 너비의 열도 따로 쓴다
        // (열을 넣으면 왼쪽 열 서식을 새 번호로 복사한다 — 편집 대조 실험)
        let cols = |sh: &crate::model::Sheet| {
            let mut runs: Vec<(u32, u32, String)> = Vec::new();
            for c in sh
                .cols
                .iter()
                .filter(|c| c.custom_width || c.hidden || c.outline_level > 0 || c.collapsed)
            {
                let key = format!(
                    "{:.4}{}{}{}",
                    c.width.unwrap_or(0.0),
                    if c.hidden { "h" } else { "" },
                    if c.outline_level > 0 {
                        format!("L{}", c.outline_level)
                    } else {
                        String::new()
                    },
                    if c.collapsed { "c" } else { "" }
                );
                match runs.last_mut() {
                    Some(last) if last.1 + 1 == c.min && last.2 == key => last.1 = c.max,
                    _ => runs.push((c.min, c.max, key)),
                }
            }
            runs.iter()
                .map(|(a, b, k)| format!("{a}-{b}:{k}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        diff(format!("{s} 열"), cols(sa), cols(sb));

        let rows = |sh: &crate::model::Sheet| {
            sh.rows
                .iter()
                .filter(|r| r.custom_height || r.hidden || r.outline_level > 0 || r.collapsed)
                .map(|r| {
                    format!(
                        "{}:{}{}{}{}",
                        r.index,
                        if r.custom_height {
                            r.height.map(|h| format!("{h}")).unwrap_or_default()
                        } else {
                            String::new()
                        },
                        if r.hidden { "h" } else { "" },
                        if r.outline_level > 0 {
                            format!("L{}", r.outline_level)
                        } else {
                            String::new()
                        },
                        if r.collapsed { "c" } else { "" }
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        diff(format!("{s} 행"), rows(sa), rows(sb));
        diff(format!("{s} 개요"), summary_key(sa), summary_key(sb));
        if !opts.ignore_filter {
            diff(format!("{s} 자동 필터"), filter_key(sa), filter_key(sb));
            // 상위/하위의 기준값(filterVal)은 한셀이 필터를 적용할 때 계산해 넣는 값이라 저장본에 있기도 없기도
            // 하다 → 양쪽에 다 있는 열만 비교한다
            let (fa, fb) = filter_vals(sa, sb);
            diff(format!("{s} 자동 필터 기준값"), fa, fb);
        }
        diff(
            format!("{s} 시트 보호"),
            protection_key(sa.protection.as_ref()),
            protection_key(sb.protection.as_ref()),
        );

        let links = |sh: &crate::model::Sheet| {
            sh.hyperlinks
                .iter()
                .map(|h| {
                    let range = crate::model::CellRange::parse(&h.range)
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| h.range.clone());
                    // 화면 설명: 주소를 되풀이한 것은 없는 것으로 본다 — 한셀은 레거시의 설명을 저장하지 않고,
                    // 레거시를 읽을 때 주소(대상, 없으면 위치)를 설명으로 채운다 (하이퍼링크 대조 실험). 줄 바꿈은
                    // 빈칸으로: 한셀은 XML 속성에 줄 바꿈을 그대로 써서 다시 읽으면 빈칸이 된다
                    let tip = h
                        .tooltip
                        .as_deref()
                        .filter(|t| Some(*t) != h.target.as_deref().or(h.location.as_deref()))
                        .map(|t| t.replace(['\n', '\r', '\t'], " "));
                    format!(
                        "{range}→{}{}{}",
                        h.target.as_deref().unwrap_or(""),
                        h.location
                            .as_deref()
                            .map(|l| format!("#{l}"))
                            .unwrap_or_default(),
                        tip.map(|t| format!(" ({t:?})")).unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        diff(format!("{s} 하이퍼링크"), links(sa), links(sb));
        diff(
            format!("{s} 눈금선"),
            sa.show_grid_lines.to_string(),
            sb.show_grid_lines.to_string(),
        );
        diff(
            format!("{s} 표"),
            tables_key(&a.styles, sa, opts.ignore_table_styles),
            tables_key(&b.styles, sb, opts.ignore_table_styles),
        );
        diff(format!("{s} 피벗 표"), pivots_key(a, sa), pivots_key(b, sb));
        diff(
            format!("{s} 패키지 파트"),
            links_key(&sa.part_links),
            links_key(&sb.part_links),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::unquote_external;

    #[test]
    fn external_reference_quotes_only_when_needed() {
        assert_eq!(
            unquote_external("'[2]Intro'!$B$3:$K$21"),
            "[2]Intro!$B$3:$K$21"
        );
        assert_eq!(
            unquote_external("'[1]Average Selling Price'!$B$4"),
            "'[1]Average Selling Price'!$B$4"
        );
        assert_eq!(
            unquote_external("SUM('[3]소개'!A1,'시트 1'!B2)"),
            "SUM([3]소개!A1,'시트 1'!B2)"
        );
    }
}
