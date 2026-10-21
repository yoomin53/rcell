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
    Border, BorderEdge, CellRange, CellValue, Color, Font, Sheet, Styles, Workbook,
};

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
    /// 읽지 못한다 (docs/format/cell-legacy.md).
    pub ignore_shape_detail: bool,
    /// 차트 내용 비교 생략. rcell 은 레거시 차트(한컴 VtChart)의 내용을 아직 해석하지 못한다.
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
        (FormulaKind::Array, _) => {
            format!("{{{}}} 배열 {}", f.text, norm_ranges(f.range.as_deref()))
        }
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
        _ => f.text.clone(),
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
    let line = s.line.as_ref().map_or("-".to_string(), |l| {
        format!(
            "{}/{}/{}",
            l.width.map_or("-".into(), |w| w.to_string()),
            paint(&l.fill),
            l.dash.as_deref().unwrap_or("-")
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
        "{:?} {:?} [{}] {}→{} {} fill={} line={} text={:?}{}",
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
        if no_text {
            String::new()
        } else {
            s.plain_text()
        },
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

/// 차트를 한 줄로: 종류·방향·묶음·제목·범례·계열(이름, 참조, 값)
fn chart_key(c: &crate::model::Chart) -> String {
    let series: Vec<String> = c
        .series
        .iter()
        .map(|s| {
            let vals: Vec<String> = s
                .values
                .iter()
                .map(|v| v.map_or("-".into(), |x| format!("{x}")))
                .collect();
            format!(
                "{:?}@{}|{}|{}=[{}]",
                s.name.as_deref().unwrap_or(""),
                s.name_ref.as_deref().unwrap_or("-"),
                s.cat_ref.as_deref().unwrap_or("-"),
                s.val_ref.as_deref().unwrap_or("-"),
                vals.join(",")
            )
        })
        .collect();
    format!(
        "{}{} {}/{} title={:?} legend={} series={}",
        c.kind,
        if c.three_d { "3D" } else { "" },
        c.bar_dir.as_deref().unwrap_or("-"),
        c.grouping.as_deref().unwrap_or("-"),
        c.title.as_deref().unwrap_or(""),
        c.legend.as_deref().unwrap_or("-"),
        series.join(";")
    )
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
    let af = sh.auto_filter.as_ref().map_or("-".into(), |f| {
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
        format!("{} [{}]", norm_ranges(Some(&f.range)), cols.join("; "))
    });
    format!("filterMode={} {af}", sh.filter_mode)
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

/// 외부 참조 `'[n]시트'!` 의 따옴표가 시트 이름 때문에 필요한 것이 아니면 뗀다.
/// 한셀은 외부 연결의 상태(경로 없음, 갱신 오류)에 따라 이 따옴표를 붙이기도 떼기도 한다 — 같은 참조다.
fn unquote_external(f: &str) -> String {
    let mut out = String::with_capacity(f.len());
    let mut rest = f;
    while let Some(i) = rest.find("'[") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let end = tail.find("'!");
        let inner = end.map(|e| &tail[..e]);
        match inner.and_then(|s| s.split_once(']').map(|(n, sh)| (s, n, sh))) {
            Some((s, n, sheet))
                if n[1..].chars().all(|c| c.is_ascii_digit())
                    && !sheet.is_empty()
                    && sheet
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.') =>
            {
                out.push_str(s);
                out.push('!');
                rest = &tail[s.len() + 2..];
            }
            _ => {
                out.push('\'');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
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
        let note_boxes = |sh: &Sheet| {
            let mut v: Vec<String> = sh
                .comments
                .iter()
                .map(|c| {
                    let b = c
                        .shape
                        .clone()
                        .unwrap_or_else(|| crate::model::CommentShape::implied(c.cell));
                    format!(
                        "{}@{:?} fill={} line={}/{}pt{}{}{} align={}/{}",
                        c.cell,
                        b.anchor,
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
        let shapes = |w: &Workbook, sh: &Sheet| {
            sh.shapes
                .iter()
                .map(|s| shape_key(s, &w.raw_parts, opts.ignore_shape_detail, opts.ignore_chart))
                .collect::<Vec<_>>()
                .join("\n      ")
        };
        diff(format!("{s} 그리기"), shapes(a, sa), shapes(b, sb));
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

        let cols = |sh: &crate::model::Sheet| {
            sh.cols
                .iter()
                .filter(|c| c.custom_width || c.hidden || c.outline_level > 0 || c.collapsed)
                .map(|c| {
                    format!(
                        "{}-{}:{:.4}{}{}{}",
                        c.min,
                        c.max,
                        c.width.unwrap_or(0.0),
                        if c.hidden { "h" } else { "" },
                        if c.outline_level > 0 {
                            format!("L{}", c.outline_level)
                        } else {
                            String::new()
                        },
                        if c.collapsed { "c" } else { "" }
                    )
                })
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
                    format!(
                        "{range}→{}{}",
                        h.target.as_deref().unwrap_or(""),
                        h.location
                            .as_deref()
                            .map(|l| format!("#{l}"))
                            .unwrap_or_default()
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
