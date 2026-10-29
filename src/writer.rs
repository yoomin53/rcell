// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! [`Workbook`] → XML `.cell` (한셀 2014 이후 기본 형식) / `.xlsx` 쓰기.
//!
//! 한셀이 저장하는 구조를 따른다: SpreadsheetML 에 `x:` 접두사, 한컴 확장(`hs:`)이 있는
//! 글꼴·셀 서식은 `mc:AlternateContent` 의 Choice(확장) + Fallback(표준) 쌍으로 쓴다.
//! 그래서 한셀은 확장까지, Excel 등 다른 프로그램은 표준 부분을 읽는다.
//!
//! 개인정보 보호를 위해 저장 경로(`x15ac:absPath`), 작성자 등은 쓰지 않는다.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write};

use zip::write::{SimpleFileOptions, ZipWriter};

use crate::error::Result;
use crate::model::{
    AutoFilter, Border, BorderEdge, CellValue, Color, Fill, Font, HancomFontExt, HancomXfExt,
    PageLayout, PageMargins, ParaShape, PartLink, RawPart, RichText, Sheet, SheetKind, SheetState,
    Styles, Table, Workbook, Xf,
};

/// 저장 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Flavor {
    /// 한셀 `.cell` (통합 문서 콘텐츠 타입이 macroEnabled — 한셀 2024 와 같게)
    #[default]
    Cell,
    /// Excel `.xlsx`
    Xlsx,
}

const SML: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const HS: &str = "http://schemas.haansoft.com/office/spreadsheet/8.0";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>";

/// XML 텍스트/속성 이스케이프. XML 1.0 에서 쓸 수 없는 제어 문자는 버린다.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // CR 은 문자 참조로 (그대로 쓰면 XML 줄 끝 정규화로 LF 가 된다 — CSV 의 CRLF 가 든 칸)
            '\r' => o.push_str("&#13;"),
            '\t' | '\n' => o.push(c),
            c if (c as u32) < 0x20 => {}
            c => o.push(c),
        }
    }
    o
}

fn color_attrs(c: &Color) -> String {
    let mut s = String::new();
    if c.auto {
        s.push_str(" auto=\"1\"");
    }
    if let Some(i) = c.indexed {
        let _ = write!(s, " indexed=\"{i}\"");
    }
    if let Some(rgb) = &c.rgb {
        let _ = write!(s, " rgb=\"{}\"", esc(rgb));
    }
    if let Some(t) = c.theme {
        let _ = write!(s, " theme=\"{t}\"");
    }
    if let Some(t) = c.tint {
        let _ = write!(s, " tint=\"{t}\"");
    }
    s
}

fn color_el(name: &str, c: &Color) -> String {
    format!("<{name}{}/>", color_attrs(c))
}

fn val_el(name: &str, v: impl std::fmt::Display) -> String {
    format!("<{name} val=\"{}\"/>", esc(&v.to_string()))
}

// ---------------------------------------------------------------------------
// 스타일
// ---------------------------------------------------------------------------

fn font_std_children(f: &Font) -> String {
    let mut s = String::new();
    if let Some(n) = &f.name {
        s.push_str(&val_el("x:name", n));
    }
    if let Some(c) = f.charset {
        s.push_str(&val_el("x:charset", c));
    }
    if let Some(fam) = f.family {
        s.push_str(&val_el("x:family", fam));
    }
    if f.bold {
        s.push_str("<x:b val=\"1\"/>");
    }
    if f.italic {
        s.push_str("<x:i val=\"1\"/>");
    }
    if f.strike {
        s.push_str("<x:strike val=\"1\"/>");
    }
    for (on, el) in [
        (f.outline, "x:outline"),
        (f.shadow, "x:shadow"),
        (f.condense, "x:condense"),
        (f.extend, "x:extend"),
    ] {
        if on {
            let _ = write!(s, "<{el} val=\"1\"/>");
        }
    }
    if let Some(c) = &f.color {
        s.push_str(&color_el("x:color", c));
    }
    if let Some(sz) = f.size {
        s.push_str(&val_el("x:sz", sz));
    }
    match f.underline.as_deref() {
        None => {}
        Some("single") => s.push_str("<x:u/>"),
        Some(u) => s.push_str(&val_el("x:u", u)),
    }
    if let Some(v) = &f.vert_align {
        s.push_str(&val_el("x:vertAlign", v));
    }
    if let Some(v) = &f.scheme {
        s.push_str(&val_el("x:scheme", v));
    }
    s
}

fn font_hs_children(h: &HancomFontExt) -> String {
    let mut s = String::new();
    let flag = |s: &mut String, name: &str, v: Option<bool>| {
        if let Some(v) = v {
            s.push_str(&val_el(name, if v { 1 } else { 0 }));
        }
    };
    flag(&mut s, "hs:emboss", h.emboss);
    flag(&mut s, "hs:engrave", h.engrave);
    flag(&mut s, "hs:superscript", h.superscript);
    flag(&mut s, "hs:subscript", h.subscript);
    for (name, c) in [
        ("hs:shadeColor", &h.shade_color),
        ("hs:shadowColor", &h.shadow_color),
        ("hs:underlineColor", &h.underline_color),
        ("hs:strikeoutColor", &h.strikeout_color),
    ] {
        if let Some(c) = c {
            s.push_str(&color_el(name, c));
        }
    }
    for (name, v) in [
        ("hs:underlineType", &h.underline_type),
        ("hs:underlineShape", &h.underline_shape),
        ("hs:strikeoutType", &h.strikeout_type),
        ("hs:strikeoutShape", &h.strikeout_shape),
        ("hs:shadowType", &h.shadow_type),
    ] {
        if let Some(v) = v {
            s.push_str(&val_el(name, v));
        }
    }
    if let Some(v) = h.shadow_offset_x {
        s.push_str(&val_el("hs:shadowOffsetX", v));
    }
    if let Some(v) = h.shadow_offset_y {
        s.push_str(&val_el("hs:shadowOffsetY", v));
    }
    if let Some(v) = &h.outline_type {
        s.push_str(&val_el("hs:outlineType", v));
    }
    if let Some(v) = &h.diacsym_mark {
        s.push_str(&val_el("hs:diacsymMark", v));
    }
    flag(&mut s, "hs:useFontSpace", h.use_font_space);
    flag(&mut s, "hs:useKerning", h.use_kerning);
    for (name, v) in [
        ("hs:size", h.size),
        ("hs:ratio", h.ratio),
        ("hs:spacing", h.spacing),
        ("hs:offset", h.offset),
    ] {
        if let Some(v) = v {
            s.push_str(&val_el(name, v));
        }
    }
    for (k, v) in &h.other {
        s.push_str(&val_el(&format!("hs:{k}"), v));
    }
    s
}

fn alternate(choice: &str, fallback: &str) -> String {
    format!(
        "<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice xmlns:hs=\"{HS}\" Requires=\"hs\">{choice}</mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>"
    )
}

fn font_xml(f: &Font) -> String {
    let std = font_std_children(f);
    match &f.hancom {
        None => format!("<x:font>{std}</x:font>"),
        Some(h) => alternate(
            &format!(
                "<x:font hs:extension=\"1\">{std}{}</x:font>",
                font_hs_children(h)
            ),
            &format!("<x:font>{std}</x:font>"),
        ),
    }
}

/// `x:gradientFill` (0 인 값은 기본값이라 쓰지 않는다)
fn gradient_xml(g: &crate::model::GradientFill) -> String {
    let mut s = String::from("<x:gradientFill");
    if g.path {
        s.push_str(" type=\"path\"");
    }
    for (k, v) in [
        ("degree", g.degree),
        ("left", g.left),
        ("right", g.right),
        ("top", g.top),
        ("bottom", g.bottom),
    ] {
        if v != 0.0 {
            let _ = write!(s, " {k}=\"{}\"", num(v));
        }
    }
    s.push('>');
    for st in &g.stops {
        let _ = write!(
            s,
            "<x:stop position=\"{}\">{}</x:stop>",
            num(st.position),
            color_el("x:color", &st.color)
        );
    }
    s.push_str("</x:gradientFill>");
    s
}

fn fill_xml(f: &Fill) -> String {
    if let Some(g) = &f.gradient {
        return format!("<x:fill>{}</x:fill>", gradient_xml(g));
    }
    let pt = f.pattern_type.as_deref().unwrap_or("none");
    let mut inner = String::new();
    if let Some(c) = &f.fg_color {
        inner.push_str(&color_el("x:fgColor", c));
    }
    if let Some(c) = &f.bg_color {
        inner.push_str(&color_el("x:bgColor", c));
    }
    if inner.is_empty() {
        format!(
            "<x:fill><x:patternFill patternType=\"{}\"/></x:fill>",
            esc(pt)
        )
    } else {
        format!(
            "<x:fill><x:patternFill patternType=\"{}\">{inner}</x:patternFill></x:fill>",
            esc(pt)
        )
    }
}

/// 셀 테두리의 선 하나. 선 종류가 없어도 색은 쓴다: 한셀은 표의 데이터 영역 서식(`dataDxfId`)을 셀
/// 테두리의 색에서 다시 만든다 (패키지 파트 실험 — 색을 버리면 표 열의 차등 서식이 사라진다)
fn edge_xml(name: &str, e: Option<&BorderEdge>) -> String {
    let Some(e) = e else {
        return format!("<{name}/>");
    };
    let style = e
        .style
        .as_deref()
        .map(|s| format!(" style=\"{}\"", esc(s)))
        .unwrap_or_default();
    match &e.color {
        Some(c) => format!("<{name}{style}>{}</{name}>", color_el("x:color", c)),
        None => format!("<{name}{style}/>"),
    }
}

fn border_attrs(b: &Border) -> String {
    let mut attrs = String::new();
    if b.diagonal_up {
        attrs.push_str(" diagonalUp=\"1\"");
    }
    if b.diagonal_down {
        attrs.push_str(" diagonalDown=\"1\"");
    }
    attrs
}

fn border_xml(b: &Border) -> String {
    let mut inner = String::new();
    for (name, e) in [("x:vertical", &b.vertical), ("x:horizontal", &b.horizontal)] {
        if let Some(e) = e {
            inner.push_str(&dxf_edge_xml(name, e));
        }
    }
    format!(
        "<x:border{}>{}{}{}{}{}{inner}</x:border>",
        border_attrs(b),
        edge_xml("x:left", b.left.as_ref()),
        edge_xml("x:right", b.right.as_ref()),
        edge_xml("x:top", b.top.as_ref()),
        edge_xml("x:bottom", b.bottom.as_ref()),
        edge_xml("x:diagonal", b.diagonal.as_ref()),
    )
}

/// 차등 서식의 선 하나: 스타일이 없으면 '선 없음' (색만 쓴다)
fn dxf_edge_xml(name: &str, e: &BorderEdge) -> String {
    let mut s = format!("<{name}");
    if let Some(st) = &e.style {
        let _ = write!(s, " style=\"{}\"", esc(st));
    }
    match &e.color {
        Some(c) => {
            let _ = write!(s, ">{}</{name}>", color_el("x:color", c));
        }
        None => s.push_str("/>"),
    }
    s
}

/// 차등 서식의 테두리: 지정한 선만 쓴다 (없는 선은 '그대로 둠')
fn dxf_border_xml(b: &Border) -> String {
    let mut s = format!("<x:border{}>", border_attrs(b));
    for (name, e) in [
        ("x:left", &b.left),
        ("x:right", &b.right),
        ("x:top", &b.top),
        ("x:bottom", &b.bottom),
        ("x:diagonal", &b.diagonal),
        ("x:vertical", &b.vertical),
        ("x:horizontal", &b.horizontal),
    ] {
        if let Some(e) = e {
            s.push_str(&dxf_edge_xml(name, e));
        }
    }
    s.push_str("</x:border>");
    s
}

fn parashape_xml(p: &ParaShape) -> String {
    let mut s = String::from("<hs:parashape");
    let mut attr = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            let _ = write!(s, " hs:{k}=\"{}\"", esc(&v));
        }
    };
    attr("leftMargin", p.left_margin.map(|v| v.to_string()));
    attr("rightMargin", p.right_margin.map(|v| v.to_string()));
    attr("indentation", p.indentation.map(|v| v.to_string()));
    attr("lineSpacingType", p.line_spacing_type.clone());
    attr("lineSpacing", p.line_spacing.map(|v| v.to_string()));
    attr("prevSpacing", p.prev_spacing.map(|v| v.to_string()));
    attr("nextSpacing", p.next_spacing.map(|v| v.to_string()));
    attr("breakNonLatinWord", p.break_non_latin_word.clone());
    attr("breakLatinWord", p.break_latin_word.clone());
    attr("condense", p.condense.map(|v| v.to_string()));
    attr("textAlignment", p.text_alignment.clone());
    attr(
        "fontLineHeight",
        p.font_line_height.map(|v| (v as u8).to_string()),
    );
    attr("abstractNumId", p.abstract_num_id.map(|v| v.to_string()));
    attr(
        "abstractNumType",
        p.abstract_num_type.map(|v| v.to_string()),
    );
    for (k, v) in &p.other {
        attr(k, Some(v.clone()));
    }
    s.push_str("/>");
    s
}

fn xf_xml(xf: &Xf, is_cell: bool) -> String {
    let mut attrs = format!(
        " numFmtId=\"{}\" fontId=\"{}\" fillId=\"{}\" borderId=\"{}\"",
        xf.num_fmt_id, xf.font_id, xf.fill_id, xf.border_id
    );
    if is_cell {
        let _ = write!(attrs, " xfId=\"{}\"", xf.xf_id.unwrap_or(0));
        if xf.quote_prefix {
            attrs.push_str(" quotePrefix=\"1\"");
        }
    }
    for (k, v) in [
        ("applyNumberFormat", xf.apply_number_format),
        ("applyFont", xf.apply_font),
        ("applyFill", xf.apply_fill),
        ("applyBorder", xf.apply_border),
        ("applyAlignment", xf.apply_alignment),
        ("applyProtection", xf.apply_protection),
    ] {
        if v == Some(true) {
            let _ = write!(attrs, " {k}=\"1\"");
        }
    }
    let mut children = String::new();
    if let Some(a) = &xf.alignment {
        let mut aa = String::new();
        if let Some(h) = &a.horizontal {
            let _ = write!(aa, " horizontal=\"{}\"", esc(h));
        }
        if let Some(v) = &a.vertical {
            let _ = write!(aa, " vertical=\"{}\"", esc(v));
        }
        if let Some(r) = a.text_rotation {
            let _ = write!(aa, " textRotation=\"{r}\"");
        }
        if a.wrap_text {
            aa.push_str(" wrapText=\"1\"");
        }
        if let Some(i) = a.indent {
            let _ = write!(aa, " indent=\"{i}\"");
        }
        if a.shrink_to_fit {
            aa.push_str(" shrinkToFit=\"1\"");
        }
        if let Some(o) = a.reading_order {
            let _ = write!(aa, " readingOrder=\"{o}\"");
        }
        if !aa.is_empty() {
            let _ = write!(children, "<x:alignment{aa}/>");
        }
    }
    if let Some(p) = &xf.protection {
        let mut pa = String::new();
        if p.locked == Some(false) {
            pa.push_str(" locked=\"0\"");
        }
        if p.hidden == Some(true) {
            pa.push_str(" hidden=\"1\"");
        }
        if !pa.is_empty() {
            let _ = write!(children, "<x:protection{pa}/>");
        }
    }
    let std = if children.is_empty() {
        format!("<x:xf{attrs}/>")
    } else {
        format!("<x:xf{attrs}>{children}</x:xf>")
    };
    match &xf.hancom {
        Some(HancomXfExt {
            parashape: Some(p), ..
        }) => alternate(
            &format!(
                "<x:xf{attrs} hs:applyExtension=\"1\">{children}{}</x:xf>",
                parashape_xml(p)
            ),
            &std,
        ),
        _ => std,
    }
}

fn styles_xml(st: &Styles, flavor: Flavor) -> String {
    let mut s = format!("{DECL}<x:styleSheet xmlns:x=\"{SML}\">");
    if !st.num_fmts.is_empty() {
        let _ = write!(s, "<x:numFmts count=\"{}\">", st.num_fmts.len());
        for n in &st.num_fmts {
            let _ = write!(
                s,
                "<x:numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                n.id,
                esc(&n.code)
            );
        }
        s.push_str("</x:numFmts>");
    }
    let default_font = Font {
        name: Some("맑은 고딕".into()),
        size: Some(11.0),
        ..Default::default()
    };
    let fonts: Vec<&Font> = if st.fonts.is_empty() {
        vec![&default_font]
    } else {
        st.fonts.iter().collect()
    };
    let _ = write!(s, "<x:fonts count=\"{}\">", fonts.len());
    for f in fonts {
        s.push_str(&font_xml(f));
    }
    s.push_str("</x:fonts>");

    let mut fills: Vec<Fill> = st.fills.clone();
    if fills.is_empty() {
        fills.push(Fill {
            pattern_type: Some("none".into()),
            ..Default::default()
        });
    }
    let _ = write!(s, "<x:fills count=\"{}\">", fills.len());
    for f in &fills {
        s.push_str(&fill_xml(f));
    }
    s.push_str("</x:fills>");

    let mut borders: Vec<Border> = st.borders.clone();
    if borders.is_empty() {
        borders.push(Border::default());
    }
    let _ = write!(s, "<x:borders count=\"{}\">", borders.len());
    for b in &borders {
        s.push_str(&border_xml(b));
    }
    s.push_str("</x:borders>");
    // 한셀은 문단 번호 정의를 테두리 다음, 스타일 XF 앞에 쓴다. CT_Stylesheet 에 없는 요소라 .xlsx(Excel)에는
    // 쓰지 않는다 — 한셀은 읽을 때 문단 모양에서 같은 정의를 다시 만든다
    if flavor == Flavor::Cell && !st.abstract_nums.is_empty() {
        let _ = write!(s, "<x:abstractNums count=\"{}\">", st.abstract_nums.len());
        for a in &st.abstract_nums {
            let _ = write!(
                s,
                "<x:abstractNum abstractNumType=\"{}\" abstractNumId=\"{}\" startNum=\"{}\" lvlText=\"{}\"/>",
                a.num_type,
                a.id,
                a.start_num,
                esc(&a.lvl_text)
            );
        }
        s.push_str("</x:abstractNums>");
    }

    let style_xfs: Vec<Xf> = if st.cell_style_xfs.is_empty() {
        vec![Xf::default()]
    } else {
        st.cell_style_xfs.clone()
    };
    let _ = write!(s, "<x:cellStyleXfs count=\"{}\">", style_xfs.len());
    for x in &style_xfs {
        s.push_str(&xf_xml(x, false));
    }
    s.push_str("</x:cellStyleXfs>");

    let cell_xfs: Vec<Xf> = if st.cell_xfs.is_empty() {
        vec![Xf::default()]
    } else {
        st.cell_xfs.clone()
    };
    let _ = write!(s, "<x:cellXfs count=\"{}\">", cell_xfs.len());
    for x in &cell_xfs {
        s.push_str(&xf_xml(x, true));
    }
    s.push_str("</x:cellXfs>");

    let names: Vec<(String, u32, Option<u32>, Option<u32>)> = if st.cell_styles.is_empty() {
        vec![("Normal".into(), 0, Some(0), None)]
    } else {
        st.cell_styles
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.xf_id.min(style_xfs.len() as u32 - 1),
                    c.builtin_id,
                    c.i_level,
                )
            })
            .collect()
    };
    let _ = write!(s, "<x:cellStyles count=\"{}\">", names.len());
    for (name, xf, builtin, i_level) in names {
        let _ = write!(s, "<x:cellStyle name=\"{}\" xfId=\"{xf}\"", esc(&name));
        if let Some(b) = builtin {
            let _ = write!(s, " builtinId=\"{b}\"");
        }
        if let Some(l) = i_level {
            let _ = write!(s, " iLevel=\"{l}\"");
        }
        s.push_str("/>");
    }
    s.push_str("</x:cellStyles>");
    if !st.dxfs.is_empty() {
        let _ = write!(s, "<x:dxfs count=\"{}\">", st.dxfs.len());
        for d in &st.dxfs {
            s.push_str(&dxf_xml(d));
        }
        s.push_str("</x:dxfs>");
    }
    if let Some(t) = &st.table_styles {
        s.push_str(&table_styles_xml(t));
    }
    s.push_str("</x:styleSheet>");
    s
}

/// `x:tableStyles`. 한셀처럼 요소를 종류 번호 순으로, size 와 pivot·table 을 늘 쓴다
fn table_styles_xml(t: &crate::model::TableStyles) -> String {
    use crate::model::TABLE_STYLE_TYPES;
    let mut s = format!("<x:tableStyles count=\"{}\"", t.styles.len());
    if let Some(d) = &t.default_table_style {
        let _ = write!(s, " defaultTableStyle=\"{}\"", esc(d));
    }
    if let Some(d) = &t.default_pivot_style {
        let _ = write!(s, " defaultPivotStyle=\"{}\"", esc(d));
    }
    if t.styles.is_empty() {
        s.push_str("/>");
        return s;
    }
    s.push('>');
    for ts in &t.styles {
        let _ = write!(
            s,
            "<x:tableStyle name=\"{}\" pivot=\"{}\" table=\"{}\" count=\"{}\">",
            esc(&ts.name),
            u8::from(ts.pivot),
            u8::from(ts.table),
            ts.elements.len()
        );
        let mut els: Vec<&crate::model::TableStyleElement> = ts.elements.iter().collect();
        els.sort_by_key(|e| {
            TABLE_STYLE_TYPES
                .iter()
                .position(|&k| k == e.kind)
                .unwrap_or(usize::MAX)
        });
        for e in els {
            let _ = write!(
                s,
                "<x:tableStyleElement type=\"{}\" size=\"{}\"",
                esc(&e.kind),
                e.size
            );
            if let Some(d) = e.dxf_id {
                let _ = write!(s, " dxfId=\"{d}\"");
            }
            s.push_str("/>");
        }
        s.push_str("</x:tableStyle>");
    }
    s.push_str("</x:tableStyles>");
    s
}

/// 조건부 서식용 차등 서식 (`x:dxf`: font, numFmt, fill, alignment, border, protection 순 — CT_Dxf, 한셀도 같다).
/// 한컴 확장(글꼴의 `hs:` 자식)이 있으면 셀 서식처럼 Choice(확장) + Fallback(표준) 쌍으로 쓴다. 한셀은 표준
/// 자식이 없는 글꼴도 확장이 붙어 있으면 남기고, 확장 없는 빈 글꼴은 다시 읽을 때 버린다
fn dxf_xml(d: &crate::model::Dxf) -> String {
    let has_hs = d.hancom.is_some() || d.font.as_ref().is_some_and(|f| dxf_font_ext(f).is_some());
    let fallback = format!("<x:dxf>{}</x:dxf>", dxf_children(d, false));
    if has_hs {
        alternate(
            &format!(
                "<x:dxf hs:applyExtension=\"1\">{}</x:dxf>",
                dxf_children(d, true)
            ),
            &fallback,
        )
    } else {
        fallback
    }
}

/// 차등 서식 글꼴의 한컴 확장. 확장도 표준 자식도 없는 글꼴(바이너리·레거시에서 이름(0x18)만 있던 글꼴)에는
/// 한셀이 차등 서식 글꼴에 쓰는 기본 확장을 붙인다 — 한셀은 확장 없는 빈 글꼴을 다시 읽을 때 버린다
fn dxf_font_ext(f: &Font) -> Option<HancomFontExt> {
    match &f.hancom {
        Some(h) => Some(h.clone()),
        None if font_std_children(f).is_empty() => Some(HancomFontExt {
            use_font_space: Some(false),
            size: Some(0),
            ratio: Some(0),
            spacing: Some(0),
            offset: Some(0),
            ..Default::default()
        }),
        None => None,
    }
}

fn dxf_children(d: &crate::model::Dxf, hs: bool) -> String {
    let mut s = String::new();
    if let Some(f) = &d.font {
        let std = font_std_children(f);
        match (dxf_font_ext(f), hs) {
            (Some(h), true) => {
                let _ = write!(
                    s,
                    "<x:font hs:extension=\"1\">{std}{}</x:font>",
                    font_hs_children(&h)
                );
            }
            _ => {
                let _ = write!(s, "<x:font>{std}</x:font>");
            }
        }
    }
    if let Some(n) = &d.num_fmt {
        let _ = write!(
            s,
            "<x:numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
            n.id,
            esc(&n.code)
        );
    }
    if let Some(g) = d.fill.as_ref().and_then(|f| f.gradient.as_ref()) {
        let _ = write!(s, "<x:fill>{}</x:fill>", gradient_xml(g));
    } else if let Some(f) = &d.fill {
        // 차등 서식의 단색 채우기는 bgColor 로 쓴다 (patternType 은 지정한 때만)
        s.push_str("<x:fill><x:patternFill");
        if let Some(p) = &f.pattern_type {
            let _ = write!(s, " patternType=\"{}\"", esc(p));
        }
        s.push('>');
        if let Some(c) = &f.fg_color {
            s.push_str(&color_el("x:fgColor", c));
        }
        if let Some(c) = &f.bg_color {
            s.push_str(&color_el("x:bgColor", c));
        }
        s.push_str("</x:patternFill></x:fill>");
    }
    if let Some(a) = &d.alignment {
        s.push_str("<x:alignment");
        if let Some(h) = &a.horizontal {
            let _ = write!(s, " horizontal=\"{}\"", esc(h));
        }
        if let Some(v) = &a.vertical {
            let _ = write!(s, " vertical=\"{}\"", esc(v));
        }
        if let Some(r) = a.text_rotation {
            let _ = write!(s, " textRotation=\"{r}\"");
        }
        if a.wrap_text {
            s.push_str(" wrapText=\"1\"");
        }
        if let Some(i) = a.indent {
            let _ = write!(s, " indent=\"{i}\"");
        }
        if a.shrink_to_fit {
            s.push_str(" shrinkToFit=\"1\"");
        }
        if let Some(o) = a.reading_order {
            let _ = write!(s, " readingOrder=\"{o}\"");
        }
        s.push_str("/>");
    }
    if let Some(b) = &d.border {
        s.push_str(&dxf_border_xml(b));
    }
    if let Some(p) = &d.protection {
        s.push_str("<x:protection");
        if let Some(l) = p.locked {
            let _ = write!(s, " locked=\"{}\"", u8::from(l));
        }
        if let Some(h) = p.hidden {
            let _ = write!(s, " hidden=\"{}\"", u8::from(h));
        }
        s.push_str("/>");
    }
    s
}

const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";

/// x14 구역에 쓰는 규칙: x14 구역에서 읽은 것, Excel 2010 아이콘 집합, 사용자 지정 아이콘
/// (한셀은 본 규칙의 3Stars 도 x14 구역으로 옮겨 쓴다 — x14 조건부 서식 실험)
fn x14_only(r: &crate::model::CfRule) -> bool {
    r.x14
        || r.icon_set.as_ref().is_some_and(|i| {
            !i.custom_icons.is_empty() || crate::model::X14_ONLY_ICON_SETS.contains(&i.set.as_str())
        })
}

/// x14 구역의 기준 값: 값은 `xm:f` 로 쓴다
fn x14_cfvo_xml(c: &crate::model::Cfvo) -> String {
    let mut x = format!("<x14:cfvo type=\"{}\"", esc(&c.kind));
    if !c.gte {
        x.push_str(" gte=\"0\"");
    }
    match &c.val {
        Some(v) => {
            let _ = write!(x, "><xm:f>{}</xm:f></x14:cfvo>", esc(v));
        }
        None => x.push_str("/>"),
    }
    x
}

/// `x14:dataBar`: 한셀이 쓰는 속성 순서, 기본값은 생략. 색은 테두리·음수 채우기·음수 테두리·축 순서
fn x14_data_bar_xml(bar: &crate::model::DataBar, ext: &crate::model::DataBarExt) -> String {
    let mut s = String::from("<x14:dataBar");
    if let Some(v) = bar.min_length {
        let _ = write!(s, " minLength=\"{v}\"");
    }
    if let Some(v) = bar.max_length {
        let _ = write!(s, " maxLength=\"{v}\"");
    }
    if !bar.show_value {
        s.push_str(" showValue=\"0\"");
    }
    if ext.border {
        s.push_str(" border=\"1\"");
    }
    if !ext.gradient {
        s.push_str(" gradient=\"0\"");
    }
    if let Some(d) = &ext.direction {
        let _ = write!(s, " direction=\"{}\"", esc(d));
    }
    if ext.negative_same_as_positive {
        s.push_str(" negativeBarColorSameAsPositive=\"1\"");
    }
    if !ext.negative_border_same_as_positive {
        s.push_str(" negativeBarBorderColorSameAsPositive=\"0\"");
    }
    if let Some(a) = &ext.axis_position {
        let _ = write!(s, " axisPosition=\"{}\"", esc(a));
    }
    s.push('>');
    for c in &ext.cfvos {
        s.push_str(&x14_cfvo_xml(c));
    }
    for (name, c) in [
        ("x14:borderColor", &ext.border_color),
        ("x14:negativeFillColor", &ext.negative_fill_color),
        ("x14:negativeBorderColor", &ext.negative_border_color),
        ("x14:axisColor", &ext.axis_color),
    ] {
        if let Some(c) = c {
            s.push_str(&color_el(name, c));
        }
    }
    s.push_str("</x14:dataBar>");
    s
}

/// `x14:iconSet`. 한셀은 x14 구역의 아이콘 집합에 늘 `custom="1"` 을 쓴다
fn x14_icon_set_xml(i: &crate::model::IconSet) -> String {
    let mut s = format!("<x14:iconSet iconSet=\"{}\"", esc(&i.set));
    if !i.show_value {
        s.push_str(" showValue=\"0\"");
    }
    if !i.percent {
        s.push_str(" percent=\"0\"");
    }
    if i.reverse {
        s.push_str(" reverse=\"1\"");
    }
    s.push_str(" custom=\"1\">");
    for c in &i.cfvos {
        s.push_str(&x14_cfvo_xml(c));
    }
    for (set, id) in &i.custom_icons {
        let _ = write!(s, "<x14:cfIcon iconSet=\"{}\" iconId=\"{id}\"/>", esc(set));
    }
    s.push_str("</x14:iconSet>");
    s
}

/// `conditionalFormatting` 들 (CT_Worksheet 에서 mergeCells 다음)과 시트 `extLst` 에 들어갈
/// `x14:conditionalFormattings` 확장. 데이터 막대의 x14 확장은 본 규칙의 `x14:id` 로 잇는다
/// (id 는 시트 번호와 규칙 순번으로 만든다 — 한셀은 읽은 id 를 그대로 다시 쓴다)
fn conditional_formats_xml(sheet: &Sheet, sheet_no: usize) -> (String, String) {
    let mut s = String::new();
    let mut x14 = String::new();
    let mut next_id = 0usize;
    let mut guid = || {
        next_id += 1;
        format!("{{{sheet_no:08X}-0000-4000-8000-{next_id:012X}}}")
    };
    let cfvo = |c: &crate::model::Cfvo| {
        let mut x = format!("<x:cfvo type=\"{}\"", esc(&c.kind));
        if let Some(v) = &c.val {
            let _ = write!(x, " val=\"{}\"", esc(v));
        }
        if !c.gte {
            x.push_str(" gte=\"0\"");
        }
        x.push_str("/>");
        x
    };
    for cf in &sheet.conditional_formats {
        // x14 구역에만 쓰는 규칙
        for r in cf.rules.iter().filter(|r| x14_only(r)) {
            let body = if let Some(i) = &r.icon_set {
                x14_icon_set_xml(i)
            } else if let Some(bar) = &r.data_bar {
                x14_data_bar_xml(
                    bar,
                    &bar.ext
                        .clone()
                        .unwrap_or_else(|| crate::model::DataBarExt::implied(bar)),
                )
            } else {
                continue;
            };
            let _ = write!(
                x14,
                "<x14:conditionalFormatting xmlns:xm=\"{XM}\"><x14:cfRule type=\"{}\" priority=\"{}\" id=\"{}\">{body}</x14:cfRule><xm:sqref>{}</xm:sqref></x14:conditionalFormatting>",
                esc(&r.kind),
                r.priority,
                guid(),
                esc(&cf.sqref)
            );
        }
        if cf.rules.iter().all(x14_only) {
            continue;
        }
        let _ = write!(s, "<x:conditionalFormatting sqref=\"{}\">", esc(&cf.sqref));
        for r in cf.rules.iter().filter(|r| !x14_only(r)) {
            let _ = write!(s, "<x:cfRule type=\"{}\"", esc(&r.kind));
            if let Some(d) = r.dxf_id {
                let _ = write!(s, " dxfId=\"{d}\"");
            }
            let _ = write!(s, " priority=\"{}\"", r.priority);
            if r.stop_if_true {
                s.push_str(" stopIfTrue=\"1\"");
            }
            if let Some(v) = r.above_average {
                let _ = write!(s, " aboveAverage=\"{}\"", v as u8);
            }
            if r.percent {
                s.push_str(" percent=\"1\"");
            }
            if r.bottom {
                s.push_str(" bottom=\"1\"");
            }
            if let Some(o) = &r.operator {
                let _ = write!(s, " operator=\"{}\"", esc(o));
            }
            if let Some(t) = &r.text {
                let _ = write!(s, " text=\"{}\"", esc(t));
            }
            if let Some(t) = &r.time_period {
                let _ = write!(s, " timePeriod=\"{}\"", esc(t));
            }
            if let Some(v) = r.rank {
                let _ = write!(s, " rank=\"{v}\"");
            }
            if let Some(v) = r.std_dev {
                let _ = write!(s, " stdDev=\"{v}\"");
            }
            if r.equal_average {
                s.push_str(" equalAverage=\"1\"");
            }
            s.push('>');
            for f in &r.formulas {
                let _ = write!(s, "<x:formula>{}</x:formula>", esc(f));
            }
            if let Some(c) = &r.color_scale {
                s.push_str("<x:colorScale>");
                for v in &c.cfvos {
                    s.push_str(&cfvo(v));
                }
                for col in &c.colors {
                    s.push_str(&color_el("x:color", col));
                }
                s.push_str("</x:colorScale>");
            }
            if let Some(d) = &r.data_bar {
                s.push_str("<x:dataBar");
                if let Some(v) = d.min_length {
                    let _ = write!(s, " minLength=\"{v}\"");
                }
                if let Some(v) = d.max_length {
                    let _ = write!(s, " maxLength=\"{v}\"");
                }
                if !d.show_value {
                    s.push_str(" showValue=\"0\"");
                }
                s.push('>');
                for v in &d.cfvos {
                    s.push_str(&cfvo(v));
                }
                if let Some(col) = &d.color {
                    s.push_str(&color_el("x:color", col));
                }
                s.push_str("</x:dataBar>");
                if let Some(ext) = &d.ext {
                    let id = guid();
                    let _ = write!(
                        s,
                        "<x:extLst><x:ext uri=\"{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}\" xmlns:x14=\"{X14}\"><x14:id>{id}</x14:id></x:ext></x:extLst>"
                    );
                    let _ = write!(
                        x14,
                        "<x14:conditionalFormatting xmlns:xm=\"{XM}\"><x14:cfRule type=\"dataBar\" id=\"{id}\">{}</x14:cfRule><xm:sqref>{}</xm:sqref></x14:conditionalFormatting>",
                        x14_data_bar_xml(d, ext),
                        esc(&cf.sqref)
                    );
                }
            }
            if let Some(i) = &r.icon_set {
                let _ = write!(s, "<x:iconSet iconSet=\"{}\"", esc(&i.set));
                if !i.show_value {
                    s.push_str(" showValue=\"0\"");
                }
                if !i.percent {
                    s.push_str(" percent=\"0\"");
                }
                if i.reverse {
                    s.push_str(" reverse=\"1\"");
                }
                s.push('>');
                for v in &i.cfvos {
                    s.push_str(&cfvo(v));
                }
                s.push_str("</x:iconSet>");
            }
            s.push_str("</x:cfRule>");
        }
        s.push_str("</x:conditionalFormatting>");
    }
    if !x14.is_empty() {
        x14 = format!(
            "<x:ext uri=\"{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}\" xmlns:x14=\"{X14}\"><x14:conditionalFormattings>{x14}</x14:conditionalFormattings></x:ext>"
        );
    }
    (s, x14)
}

/// `dataValidations` (CT_Worksheet 에서 conditionalFormatting 다음)
fn data_validations_xml(sheet: &Sheet) -> String {
    if sheet.data_validations.is_empty() {
        return String::new();
    }
    let mut s = format!(
        "<x:dataValidations count=\"{}\">",
        sheet.data_validations.len()
    );
    for d in &sheet.data_validations {
        let _ = write!(s, "<x:dataValidation type=\"{}\"", esc(&d.kind));
        for (name, v) in [("errorStyle", &d.error_style), ("operator", &d.operator)] {
            if let Some(v) = v {
                let _ = write!(s, " {name}=\"{}\"", esc(v));
            }
        }
        for (name, on) in [
            ("allowBlank", d.allow_blank),
            ("showDropDown", d.hide_drop_down),
            ("showInputMessage", d.show_input),
            ("showErrorMessage", d.show_error),
        ] {
            if on {
                let _ = write!(s, " {name}=\"1\"");
            }
        }
        for (name, v) in [
            ("errorTitle", &d.error_title),
            ("error", &d.error),
            ("promptTitle", &d.prompt_title),
            ("prompt", &d.prompt),
        ] {
            if let Some(v) = v {
                let _ = write!(s, " {name}=\"{}\"", esc(v));
            }
        }
        let _ = write!(s, " sqref=\"{}\">", esc(&d.sqref));
        if let Some(f) = &d.formula1 {
            let _ = write!(s, "<x:formula1>{}</x:formula1>", esc(f));
        }
        if let Some(f) = &d.formula2 {
            let _ = write!(s, "<x:formula2>{}</x:formula2>", esc(f));
        }
        s.push_str("</x:dataValidation>");
    }
    s.push_str("</x:dataValidations>");
    s
}

// ---------------------------------------------------------------------------
// 공유 문자열
// ---------------------------------------------------------------------------

struct Sst {
    index: HashMap<String, u32>,
    items: Vec<RichText>,
    count: u32,
}

impl Sst {
    fn new() -> Self {
        Sst {
            index: HashMap::new(),
            items: Vec::new(),
            count: 0,
        }
    }

    fn add(&mut self, text: &str, rich: Option<&RichText>) -> u32 {
        self.count += 1;
        if let Some(&i) = self.index.get(text) {
            return i;
        }
        let i = self.items.len() as u32;
        self.index.insert(text.to_string(), i);
        self.items.push(rich.cloned().unwrap_or_else(|| RichText {
            text: text.to_string(),
            runs: Vec::new(),
        }));
        i
    }

    fn xml(&self) -> String {
        let mut s = format!(
            "{DECL}<x:sst xmlns:x=\"{SML}\" count=\"{}\" uniqueCount=\"{}\">",
            self.count,
            self.items.len()
        );
        for it in &self.items {
            let _ = write!(s, "<x:si>{}</x:si>", rich_xml(it));
        }
        s.push_str("</x:sst>");
        s
    }
}

/// 서식 있는 문자열의 내용 (`x:t` 하나 또는 `x:r` 런들) — 공유 문자열과 메모가 같이 쓴다
fn rich_xml(it: &RichText) -> String {
    if it.runs.is_empty() {
        return t_el(&it.text);
    }
    let mut s = String::new();
    for run in &it.runs {
        s.push_str("<x:r>");
        if let Some(f) = &run.font {
            let rpr = font_std_children(f).replace("<x:name ", "<x:rFont ");
            let _ = write!(s, "<x:rPr>{rpr}</x:rPr>");
        }
        s.push_str(&t_el(&run.text));
        s.push_str("</x:r>");
    }
    s
}

/// 메모 파트 (`xl/commentsN.xml`)
fn comments_xml(sheet: &Sheet) -> String {
    let mut authors: Vec<&str> = Vec::new();
    for c in &sheet.comments {
        if !authors.contains(&c.author.as_str()) {
            authors.push(&c.author);
        }
    }
    let mut s = format!("{DECL}<x:comments xmlns:x=\"{SML}\"><x:authors>");
    for a in &authors {
        let _ = write!(s, "<x:author>{}</x:author>", esc(a));
    }
    s.push_str("</x:authors><x:commentList>");
    for c in &sheet.comments {
        let id = authors.iter().position(|a| *a == c.author).unwrap_or(0);
        let _ = write!(
            s,
            "<x:comment ref=\"{}\" authorId=\"{id}\"><x:text>{}</x:text></x:comment>",
            c.cell,
            rich_xml(&c.text)
        );
    }
    s.push_str("</x:commentList></x:comments>");
    s
}

/// 메모 상자의 `x:ShapeUri` GUID. 한셀은 문서를 읽을 때마다 새로 만든다 (같은 세션의 XML·바이너리 저장본은 같고, 다시
/// 읽으면 달라진다 — 메모 이름 실험) — rcell 은 시트 번호와 메모 순서로 정한다
fn note_guid(n: usize, k: usize) -> String {
    let h = crate::crypto::sha1(format!("rcell-note-{n}-{k}").as_bytes());
    let mut g = [0u8; 16];
    g.copy_from_slice(&h[..16]);
    // RFC 4122 변형 4 (한셀의 GUID 도 이 꼴이다)
    g[6] = (g[6] & 0x0F) | 0x40;
    g[8] = (g[8] & 0x3F) | 0x80;
    let x: String = g.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &x[0..8],
        &x[8..12],
        &x[12..16],
        &x[16..20],
        &x[20..32]
    )
}

/// 메모 도형 VML (`xl/drawings/vmlDrawingN.vml`) — 한셀이 쓰는 꼴 ([`hcell_comments_vml`]). 한셀의 `.xlsx` 저장도
/// `.cell` 과 같은 VML 을 쓴다 (xlsx 대조 실험: 메모 문서 4개)
pub(crate) fn comments_vml(sheet: &Sheet, n: usize) -> String {
    hcell_comments_vml(sheet, n)
}

/// 한셀이 쓰는 메모 VML (메모 대조 실험, 메모 이름 실험 — 한셀의 XML·바이너리 저장본이 같다): 이름은 `v:shape/@id`
/// (모델의 이름, 없으면 `메모 k`), `o:spid` 는 시트 안 순서 `_x0000_s1`…, `z-index` 는 시트의 그리기 개체 수 + 순서 (서식
/// 파일: 그리기 개체 12개인 시트의 메모가 12, 13), `o:idmap data="0"` 은 모든 시트, 선 두께는 소수 둘째 자리 pt —
/// 1 pt(VML 기본값)면 두께를 쓰지 않는다, `x:ShapeUri` 는 GUID ([`note_guid`])
fn hcell_comments_vml(sheet: &Sheet, n: usize) -> String {
    let mut s = format!(
        "{DECL}<xml xmlns:r=\"{REL}\" xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\" xmlns:p=\"urn:schemas-microsoft-com:office:powerpoint\"><v:shapetype coordsize=\"21600,21600\" id=\"_x0000_t202\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>"
    );
    let base = sheet.shapes.len();
    for (k, c) in sheet.comments.iter().enumerate() {
        let sh = c
            .shape
            .clone()
            .unwrap_or_else(|| crate::model::CommentShape::implied(c.cell));
        let (weight, stroke_weight) = if sh.stroke_weight == 1.0 {
            (String::new(), String::new())
        } else {
            let w = format!("{:.2}pt", sh.stroke_weight);
            (format!(" strokeweight=\"{w}\""), format!(" weight=\"{w}\""))
        };
        let _ = write!(
            s,
            "<v:shape id=\"{}\" style=\"position:absolute;visibility:{};z-index:{}\" o:spid=\"_x0000_s{}\" o:hralign=\"left\" o:insetmode=\"auto\" stroked=\"t\" strokecolor=\"{}\"{weight} filled=\"t\" fillcolor=\"{}\" o:connectortype=\"straight\" type=\"#_x0000_t202\"><v:stroke{stroke_weight} color=\"{}\" joinstyle=\"miter\" filltype=\"solid\"/>{}<v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"/><x:ClientData ObjectType=\"Note\">{}{}<x:Anchor>{}</x:Anchor><x:AutoFill>False</x:AutoFill>{}{}<x:Row>{}</x:Row><x:Column>{}</x:Column>{}<x:ShapeUri>{}</x:ShapeUri></x:ClientData></v:shape>",
            esc(&sh.name_or(k)),
            if c.visible { "visible" } else { "hidden" },
            base + k,
            k + 1,
            esc(&sh.stroke_color),
            esc(&sh.fill_color),
            esc(&sh.stroke_color),
            if sh.shadow {
                "<v:shadow on=\"t\" obscured=\"t\" color=\"#000000\"/>"
            } else {
                ""
            },
            if sh.move_with_cells {
                ""
            } else {
                "<x:MoveWithCells></x:MoveWithCells>"
            },
            if sh.size_with_cells {
                ""
            } else {
                "<x:SizeWithCells></x:SizeWithCells>"
            },
            sh.anchor.map(|v| v.to_string()).join(", "),
            sh.text_halign
                .as_ref()
                .map(|v| format!("<x:TextHAlign>{}</x:TextHAlign>", esc(v)))
                .unwrap_or_default(),
            sh.text_valign
                .as_ref()
                .map(|v| format!("<x:TextVAlign>{}</x:TextVAlign>", esc(v)))
                .unwrap_or_default(),
            c.cell.row,
            c.cell.col,
            if c.visible {
                "<x:Visible></x:Visible>"
            } else {
                ""
            },
            note_guid(n, k)
        );
    }
    s.push_str(
        "<o:shapelayout v:ext=\"edit\"><o:idmap data=\"0\" v:ext=\"edit\"/></o:shapelayout></xml>",
    );
    s
}

fn t_el(text: &str) -> String {
    let preserve = text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.contains('\n');
    if preserve {
        format!(
            "<x:t xml:space=\"preserve\">{}</x:t>",
            esc(&crate::text::xstring_encode(text))
        )
    } else {
        format!("<x:t>{}</x:t>", esc(&crate::text::xstring_encode(text)))
    }
}

// ---------------------------------------------------------------------------
// 시트
// ---------------------------------------------------------------------------

fn num(v: f64) -> String {
    // 왕복 가능한 최단 표기. 무한대는 xsd:double 표기 (Rust 의 inf 는 xsd:double 이 아니다)
    if v.is_infinite() {
        return if v > 0.0 { "INF" } else { "-INF" }.into();
    }
    format!("{v}")
}

fn sheet_xml(
    sheet: &Sheet,
    sheet_no: usize,
    selected: bool,
    sst: &mut Sst,
    rich: &HashMap<&str, &RichText>,
    link_rels: &mut Vec<(String, String)>,
) -> String {
    let mut s = format!("{DECL}<x:worksheet xmlns:r=\"{REL}\" xmlns:x=\"{SML}\">");
    s.push_str(&sheet_pr_xml(sheet));
    if let Some(d) = &sheet.dimension {
        let _ = write!(s, "<x:dimension ref=\"{}\"/>", esc(d));
    }
    s.push_str(&sheet_view_xml(sheet, selected));
    let mut fmt = String::new();
    if let Some(w) = sheet.default_col_width {
        let _ = write!(fmt, " defaultColWidth=\"{}\"", num(w));
    }
    let _ = write!(
        fmt,
        " defaultRowHeight=\"{}\"",
        num(sheet.default_row_height.unwrap_or(16.5))
    );
    if sheet.default_row_custom {
        fmt.push_str(" customHeight=\"1\"");
    }
    // 개요 수준의 최댓값 (한셀·Excel 이 개요 단추 칸을 그리는 데 쓴다)
    let max_row_level = sheet
        .rows
        .iter()
        .map(|r| r.outline_level)
        .max()
        .unwrap_or(0);
    let max_col_level = sheet
        .cols
        .iter()
        .map(|c| c.outline_level)
        .max()
        .unwrap_or(0);
    if max_row_level > 0 {
        let _ = write!(fmt, " outlineLevelRow=\"{max_row_level}\"");
    }
    if max_col_level > 0 {
        let _ = write!(fmt, " outlineLevelCol=\"{max_col_level}\"");
    }
    let _ = write!(s, "<x:sheetFormatPr{fmt}/>");

    if !sheet.cols.is_empty() {
        s.push_str("<x:cols>");
        for c in &sheet.cols {
            let _ = write!(s, "<x:col min=\"{}\" max=\"{}\"", c.min, c.max);
            if let Some(w) = c.width {
                let _ = write!(s, " width=\"{}\"", num(w));
            }
            if let Some(st) = c.style.filter(|&st| st != 0) {
                let _ = write!(s, " style=\"{st}\"");
            }
            if c.hidden {
                s.push_str(" hidden=\"1\"");
            }
            if c.custom_width {
                s.push_str(" customWidth=\"1\"");
            }
            if c.outline_level > 0 {
                let _ = write!(s, " outlineLevel=\"{}\"", c.outline_level);
            }
            if c.collapsed {
                s.push_str(" collapsed=\"1\"");
            }
            s.push_str("/>");
        }
        s.push_str("</x:cols>");
    }

    // 행 정보와 셀을 행 번호로 묶는다
    let row_info: HashMap<u32, &crate::model::RowInfo> =
        sheet.rows.iter().map(|r| (r.index, r)).collect();
    let mut rows: Vec<u32> = sheet.cells.iter().map(|c| c.r.row + 1).collect();
    rows.extend(sheet.rows.iter().map(|r| r.index));
    rows.sort_unstable();
    rows.dedup();

    s.push_str("<x:sheetData>");
    let mut ci = 0;
    for rn in rows {
        let mut attrs = format!(" r=\"{rn}\"");
        if let Some(info) = row_info.get(&rn) {
            if let Some(st) = info.style {
                let _ = write!(attrs, " s=\"{st}\" customFormat=\"1\"");
            }
            if let Some(h) = info.height {
                let _ = write!(attrs, " ht=\"{}\"", num(h));
            }
            if info.custom_height {
                attrs.push_str(" customHeight=\"1\"");
            }
            if info.hidden {
                attrs.push_str(" hidden=\"1\"");
            }
            if info.outline_level > 0 {
                let _ = write!(attrs, " outlineLevel=\"{}\"", info.outline_level);
            }
            if info.collapsed {
                attrs.push_str(" collapsed=\"1\"");
            }
        }
        let start = ci;
        while ci < sheet.cells.len() && sheet.cells[ci].r.row + 1 == rn {
            ci += 1;
        }
        if start == ci {
            let _ = write!(s, "<x:row{attrs}/>");
            continue;
        }
        let _ = write!(s, "<x:row{attrs}>");
        for c in &sheet.cells[start..ci] {
            let _ = write!(s, "<x:c r=\"{}\"", c.r);
            if c.style != 0 {
                let _ = write!(s, " s=\"{}\"", c.style);
            }
            let f = c.formula.as_ref().map(formula_xml).unwrap_or_default();
            match &c.value {
                CellValue::Empty if f.is_empty() => s.push_str("/>"),
                CellValue::Empty => {
                    let _ = write!(s, ">{f}</x:c>");
                }
                CellValue::Number(n) => {
                    let _ = write!(s, ">{f}<x:v>{}</x:v></x:c>", num(*n));
                }
                CellValue::Bool(b) => {
                    let _ = write!(s, " t=\"b\">{f}<x:v>{}</x:v></x:c>", *b as u8);
                }
                CellValue::Error(e) => {
                    let _ = write!(s, " t=\"e\">{f}<x:v>{}</x:v></x:c>", esc(e));
                }
                CellValue::Date(d) => {
                    let _ = write!(s, " t=\"d\">{f}<x:v>{}</x:v></x:c>", esc(d));
                }
                CellValue::Text(t) if !f.is_empty() => {
                    let _ = write!(
                        s,
                        " t=\"str\">{f}<x:v>{}</x:v></x:c>",
                        esc(&crate::text::xstring_encode(t))
                    );
                }
                CellValue::Text(t) => {
                    let i = sst.add(t, rich.get(t.as_str()).copied());
                    let _ = write!(s, " t=\"s\"><x:v>{i}</x:v></x:c>");
                }
            }
        }
        s.push_str("</x:row>");
    }
    s.push_str("</x:sheetData>");
    s.push_str(&protection_xml(sheet));
    s.push_str(&auto_filter_xml(sheet));

    if !sheet.merges.is_empty() {
        let _ = write!(s, "<x:mergeCells count=\"{}\">", sheet.merges.len());
        for m in &sheet.merges {
            let _ = write!(s, "<x:mergeCell ref=\"{m}\"/>");
        }
        s.push_str("</x:mergeCells>");
    }
    let (cf, cf14) = conditional_formats_xml(sheet, sheet_no);
    s.push_str(&cf);
    s.push_str(&data_validations_xml(sheet));
    if !sheet.hyperlinks.is_empty() {
        s.push_str("<x:hyperlinks>");
        for h in &sheet.hyperlinks {
            let _ = write!(s, "<x:hyperlink ref=\"{}\"", esc(&h.range));
            if let Some(t) = &h.target {
                let id = format!("rId{}", link_rels.len() + 1);
                let _ = write!(s, " r:id=\"{id}\"");
                link_rels.push((id, t.clone()));
            }
            if let Some(l) = &h.location {
                let _ = write!(s, " location=\"{}\"", esc(l));
            }
            if let Some(t) = &h.tooltip {
                let _ = write!(s, " tooltip=\"{}\"", esc(t));
            }
            if let Some(d) = &h.display {
                let _ = write!(s, " display=\"{}\"", esc(d));
            }
            s.push_str("/>");
        }
        s.push_str("</x:hyperlinks>");
    }
    // 프린터 설정 파트는 pageSetup 의 r:id 로 가리킨다 (시트 관계 rIdP<n>)
    let printer = sheet
        .part_links
        .iter()
        .position(|l| l.rel_type.ends_with("/printerSettings"))
        .map(|k| format!("rIdP{}", k + 1));
    s.push_str(&page_xml(&sheet.page, printer.as_deref()));
    if !sheet.shapes.is_empty() {
        s.push_str("<x:drawing r:id=\"rIdDr\"/>");
    }
    if !sheet.comments.is_empty() {
        s.push_str("<x:legacyDrawing r:id=\"rIdVml\"/>");
    }
    // 표 (CT_Worksheet 에서 extLst 바로 앞). 관계 rIdT<n>
    if !sheet.tables.is_empty() {
        let _ = write!(s, "<x:tableParts count=\"{}\">", sheet.tables.len());
        for j in 0..sheet.tables.len() {
            let _ = write!(s, "<x:tablePart r:id=\"rIdT{}\"/>", j + 1);
        }
        s.push_str("</x:tableParts>");
    }
    // 시트 확장 (CT_Worksheet 의 마지막 자식)
    if !cf14.is_empty() {
        let _ = write!(s, "<x:extLst>{cf14}</x:extLst>");
    }
    s.push_str("</x:worksheet>");
    s
}

/// 셀 수식 (`x:f`). 공유 수식은 셀마다 펼쳐 쓴다. 배열 수식은 `t="array" ref`, 데이터 표는 `t="dataTable"` 과
/// 입력 — 한셀이 쓰는 속성 순서(ref, dt2D, dtr, r1, r2)로, 2변수 표에는 `dtr` 을 쓰지 않는다 (수식 대조 실험)
fn formula_xml(f: &crate::model::Formula) -> String {
    use crate::model::FormulaKind;
    let range = f
        .range
        .as_deref()
        .map(|r| format!(" ref=\"{}\"", esc(r)))
        .unwrap_or_default();
    match (f.kind, &f.data_table) {
        (FormulaKind::Array, _) => format!("<x:f t=\"array\"{range}>{}</x:f>", esc(&f.text)),
        (FormulaKind::DataTable, Some(d)) => {
            let mut s = format!("<x:f t=\"dataTable\"{range}");
            if d.two_d {
                s.push_str(" dt2D=\"1\"");
            } else if d.row_input {
                s.push_str(" dtr=\"1\"");
            }
            for (name, v) in [("r1", &d.r1), ("r2", &d.r2)] {
                if let Some(v) = v {
                    let _ = write!(s, " {name}=\"{}\"", esc(v));
                }
            }
            for (name, on) in [("del1", d.del1), ("del2", d.del2)] {
                if on {
                    let _ = write!(s, " {name}=\"1\"");
                }
            }
            s.push_str("/>");
            s
        }
        _ if f.text.is_empty() => String::new(),
        _ => format!("<x:f>{}</x:f>", esc(&f.text)),
    }
}

/// `sheetProtection` (CT_Worksheet 에서 sheetData 다음)
fn protection_xml(sheet: &Sheet) -> String {
    let Some(p) = &sheet.protection else {
        return String::new();
    };
    let mut s = String::from("<x:sheetProtection");
    if let Some(pw) = &p.password {
        let _ = write!(s, " password=\"{}\"", esc(pw));
    }
    for (k, v) in &p.flags {
        let _ = write!(s, " {k}=\"{}\"", *v as u8);
    }
    s.push_str("/>");
    s
}

/// `autoFilter` (CT_Worksheet 에서 sheetProtection 다음)
fn auto_filter_xml(sheet: &Sheet) -> String {
    sheet
        .auto_filter
        .as_ref()
        .map(auto_filter_xml_of)
        .unwrap_or_default()
}

/// `autoFilter` 요소 (시트와 표가 함께 쓴다)
fn auto_filter_xml_of(af: &AutoFilter) -> String {
    let mut s = format!("<x:autoFilter ref=\"{}\"", esc(&af.range));
    if af.columns.is_empty() {
        s.push_str("/>");
        return s;
    }
    s.push('>');
    for c in &af.columns {
        let _ = write!(s, "<x:filterColumn colId=\"{}\">", c.col_id);
        if !c.values.is_empty() || c.blank {
            s.push_str("<x:filters");
            if c.blank {
                s.push_str(" blank=\"1\"");
            }
            s.push('>');
            for v in &c.values {
                let _ = write!(s, "<x:filter val=\"{}\"/>", esc(v));
            }
            s.push_str("</x:filters>");
        } else if !c.custom.is_empty() {
            s.push_str("<x:customFilters");
            if c.custom_and {
                s.push_str(" and=\"1\"");
            }
            s.push('>');
            for (op, v) in &c.custom {
                s.push_str("<x:customFilter");
                if op != "equal" {
                    let _ = write!(s, " operator=\"{}\"", esc(op));
                }
                let _ = write!(s, " val=\"{}\"/>", esc(v));
            }
            s.push_str("</x:customFilters>");
        } else if let Some((top, percent, val)) = c.top10 {
            s.push_str("<x:top10");
            if !top {
                s.push_str(" top=\"0\"");
            }
            if percent {
                s.push_str(" percent=\"1\"");
            }
            let _ = write!(s, " val=\"{}\"", num(val));
            if let Some(v) = c.filter_val {
                let _ = write!(s, " filterVal=\"{}\"", num(v));
            }
            s.push_str("/>");
        } else if let Some(t) = &c.dynamic {
            let _ = write!(s, "<x:dynamicFilter type=\"{}\"", esc(t));
            if let Some(v) = c.dynamic_val {
                let _ = write!(s, " val=\"{}\"", num(v));
            }
            if let Some(v) = c.dynamic_max_val {
                let _ = write!(s, " maxVal=\"{}\"", num(v));
            }
            s.push_str("/>");
        }
        s.push_str("</x:filterColumn>");
    }
    s.push_str("</x:autoFilter>");
    s
}

fn sheet_pr_xml(sheet: &Sheet) -> String {
    let p = &sheet.page;
    let mut inner = String::new();
    if let Some(c) = &sheet.tab_color {
        let _ = write!(inner, "<x:tabColor{}/>", color_attrs(c));
    }
    if sheet.summary_above || sheet.summary_left {
        let _ = write!(
            inner,
            "<x:outlinePr summaryBelow=\"{}\" summaryRight=\"{}\"/>",
            !sheet.summary_above as u8, !sheet.summary_left as u8
        );
    }
    if p.auto_page_breaks.is_some() || p.fit_to_page.is_some() {
        inner.push_str("<x:pageSetUpPr");
        if let Some(v) = p.auto_page_breaks {
            let _ = write!(inner, " autoPageBreaks=\"{}\"", v as u8);
        }
        if let Some(v) = p.fit_to_page {
            let _ = write!(inner, " fitToPage=\"{}\"", v as u8);
        }
        inner.push_str("/>");
    }
    let mut code = sheet
        .code_name
        .as_ref()
        .map(|c| format!(" codeName=\"{}\"", esc(c)))
        .unwrap_or_default();
    if sheet.filter_mode {
        code.push_str(" filterMode=\"1\"");
    }
    match (code.is_empty(), inner.is_empty()) {
        (true, true) => String::new(),
        (_, true) => format!("<x:sheetPr{code}/>"),
        _ => format!("<x:sheetPr{code}>{inner}</x:sheetPr>"),
    }
}

fn sheet_view_xml(sheet: &Sheet, selected: bool) -> String {
    let v = &sheet.view;
    let mut a = String::new();
    let flag = |a: &mut String, name: &str, on: bool, default: bool| {
        if on != default {
            let _ = write!(a, " {name}=\"{}\"", on as u8);
        }
    };
    flag(&mut a, "showFormulas", v.show_formulas, false);
    flag(&mut a, "showGridLines", sheet.show_grid_lines, true);
    flag(&mut a, "showRowColHeaders", v.show_row_col_headers, true);
    flag(&mut a, "showZeros", v.show_zeros, true);
    flag(&mut a, "tabSelected", selected, false);
    if let Some(x) = &v.view {
        let _ = write!(a, " view=\"{}\"", esc(x));
    }
    if let Some(x) = &v.top_left_cell {
        let _ = write!(a, " topLeftCell=\"{}\"", esc(x));
    }
    for (name, val) in [
        ("zoomScale", v.zoom_scale),
        ("zoomScaleNormal", v.zoom_scale_normal),
        ("zoomScaleSheetLayoutView", v.zoom_scale_sheet_layout_view),
    ] {
        if let Some(z) = val {
            let _ = write!(a, " {name}=\"{z}\"");
        }
    }
    let mut inner = String::new();
    if let Some(p) = &v.pane {
        inner.push_str("<x:pane");
        if p.x_split != 0.0 {
            let _ = write!(inner, " xSplit=\"{}\"", num(p.x_split));
        }
        if p.y_split != 0.0 {
            let _ = write!(inner, " ySplit=\"{}\"", num(p.y_split));
        }
        for (name, val) in [
            ("topLeftCell", &p.top_left_cell),
            ("activePane", &p.active_pane),
            ("state", &p.state),
        ] {
            if let Some(x) = val {
                let _ = write!(inner, " {name}=\"{}\"", esc(x));
            }
        }
        inner.push_str("/>");
    }
    if v.active_cell.is_some() || v.sqref.is_some() {
        inner.push_str("<x:selection");
        if let Some(p) = v.pane.as_ref().and_then(|p| p.active_pane.as_ref()) {
            let _ = write!(inner, " pane=\"{}\"", esc(p));
        }
        if let Some(x) = &v.active_cell {
            let _ = write!(inner, " activeCell=\"{}\"", esc(x));
        }
        if let Some(x) = &v.sqref {
            let _ = write!(inner, " sqref=\"{}\"", esc(x));
        }
        inner.push_str("/>");
    }
    if inner.is_empty() {
        format!("<x:sheetViews><x:sheetView{a} workbookViewId=\"0\"/></x:sheetViews>")
    } else {
        format!(
            "<x:sheetViews><x:sheetView{a} workbookViewId=\"0\">{inner}</x:sheetView></x:sheetViews>"
        )
    }
}

/// docProps/core.xml — 속성이 하나도 없으면 쓰지 않는다
fn core_xml(p: &crate::model::DocProperties) -> Option<String> {
    if p.is_empty() {
        return None;
    }
    let mut s = format!(
        "{DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">"
    );
    for (tag, v) in [
        ("dc:title", &p.title),
        ("dc:subject", &p.subject),
        ("dc:creator", &p.creator),
        ("cp:keywords", &p.keywords),
        ("dc:description", &p.description),
        ("cp:lastModifiedBy", &p.last_modified_by),
        ("cp:revision", &p.revision),
        ("cp:lastPrinted", &p.last_printed),
        ("cp:category", &p.category),
        ("cp:contentStatus", &p.content_status),
    ] {
        if let Some(v) = v {
            let _ = write!(s, "<{tag}>{}</{tag}>", esc(v));
        }
    }
    for (tag, v) in [
        ("dcterms:created", &p.created),
        ("dcterms:modified", &p.modified),
    ] {
        if let Some(v) = v {
            let _ = write!(s, "<{tag} xsi:type=\"dcterms:W3CDTF\">{}</{tag}>", esc(v));
        }
    }
    s.push_str("</cp:coreProperties>");
    Some(s)
}

/// 여백(인치). 한셀은 읽을 때 HWPUNIT(1/7200 인치)로 **잘라** 들고 있으므로, 정수 HWPUNIT 바로 아래에
/// 있는 값(레거시·f32 를 거친 값)은 한 단위를 잃는다 (관측: 0.51152777… → 3682). 그런 값은 그 정수보다
/// 0.01 HWPUNIT 위로 써서, 자르든 반올림하든 f32 로 읽든 같은 단위가 되게 한다.
fn margin(v: f64) -> String {
    let h = (v * 7200.0).round();
    let below = |x: f64| x * 7200.0 < h;
    if (h - v * 7200.0).abs() < 0.05 && (below(v) || below(v as f32 as f64)) {
        num((h + 0.01) / 7200.0)
    } else {
        num(v)
    }
}

/// printOptions, pageMargins, pageSetup, headerFooter, rowBreaks, colBreaks (CT_Worksheet 순서).
/// `printer` = 프린터 설정 파트로 가는 관계 ID
fn page_xml(p: &PageLayout, printer: Option<&str>) -> String {
    let mut s = String::new();
    let b = |v: bool| v as u8;
    if let Some(o) = &p.print_options {
        let _ = write!(
            s,
            "<x:printOptions horizontalCentered=\"{}\" verticalCentered=\"{}\" headings=\"{}\" gridLines=\"{}\"/>",
            b(o.horizontal_centered),
            b(o.vertical_centered),
            b(o.headings),
            b(o.grid_lines)
        );
    }
    let m = p.margins.clone().unwrap_or(PageMargins {
        left: 0.7,
        right: 0.7,
        top: 0.75,
        bottom: 0.75,
        header: 0.3,
        footer: 0.3,
    });
    let _ = write!(
        s,
        "<x:pageMargins left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\" header=\"{}\" footer=\"{}\"/>",
        margin(m.left),
        margin(m.right),
        margin(m.top),
        margin(m.bottom),
        margin(m.header),
        margin(m.footer)
    );
    if let Some(u) = &p.setup {
        s.push_str("<x:pageSetup");
        for (name, val) in [
            ("paperSize", u.paper_size),
            ("scale", u.scale),
            ("firstPageNumber", u.first_page_number),
            ("fitToWidth", u.fit_to_width),
            ("fitToHeight", u.fit_to_height),
        ] {
            if let Some(v) = val {
                let _ = write!(s, " {name}=\"{v}\"");
            }
        }
        if let Some(o) = &u.page_order {
            let _ = write!(s, " pageOrder=\"{}\"", esc(o));
        }
        if let Some(o) = &u.orientation {
            let _ = write!(s, " orientation=\"{}\"", esc(o));
        }
        let _ = write!(
            s,
            " usePrinterDefaults=\"{}\" blackAndWhite=\"{}\" draft=\"{}\"",
            b(u.use_printer_defaults),
            b(u.black_and_white),
            b(u.draft),
        );
        if let Some(c) = &u.cell_comments {
            let _ = write!(s, " cellComments=\"{}\"", esc(c));
        }
        let _ = write!(s, " useFirstPageNumber=\"{}\"", b(u.use_first_page_number));
        if let Some(e) = &u.errors {
            let _ = write!(s, " errors=\"{}\"", esc(e));
        }
        for (name, val) in [
            ("horizontalDpi", u.horizontal_dpi),
            ("verticalDpi", u.vertical_dpi),
            ("copies", u.copies),
        ] {
            if let Some(v) = val {
                let _ = write!(s, " {name}=\"{v}\"");
            }
        }
        if let Some(id) = printer {
            let _ = write!(s, " r:id=\"{id}\"");
        }
        s.push_str("/>");
    } else if let Some(id) = printer {
        let _ = write!(s, "<x:pageSetup r:id=\"{id}\"/>");
    }
    if let Some(h) = &p.header_footer {
        let _ = write!(
            s,
            "<x:headerFooter differentOddEven=\"{}\" differentFirst=\"{}\" scaleWithDoc=\"{}\" alignWithMargins=\"{}\">",
            b(h.different_odd_even),
            b(h.different_first),
            b(h.scale_with_doc),
            b(h.align_with_margins)
        );
        for (name, val) in [
            ("oddHeader", &h.odd_header),
            ("oddFooter", &h.odd_footer),
            ("evenHeader", &h.even_header),
            ("evenFooter", &h.even_footer),
            ("firstHeader", &h.first_header),
            ("firstFooter", &h.first_footer),
        ] {
            if let Some(t) = val {
                let _ = write!(s, "<x:{name}>{}</x:{name}>", esc(t));
            }
        }
        s.push_str("</x:headerFooter>");
    }
    for (name, list, max) in [
        ("rowBreaks", &p.row_breaks, 16383),
        ("colBreaks", &p.col_breaks, 1048575),
    ] {
        if !list.is_empty() {
            let _ = write!(
                s,
                "<x:{name} count=\"{0}\" manualBreakCount=\"{0}\">",
                list.len()
            );
            for id in list {
                let _ = write!(s, "<x:brk id=\"{id}\" max=\"{max}\" man=\"1\"/>");
            }
            let _ = write!(s, "</x:{name}>");
        }
    }
    s
}

// ---------------------------------------------------------------------------
// 패키지
// ---------------------------------------------------------------------------

/// `workbookView`. 활성 탭은 선택된 시트를 따른다. 원본에 창 정보가 있으면 한셀처럼 위치·크기를 늘 쓴다
fn workbook_view_xml(v: Option<&crate::model::WorkbookView>, active: usize, n: usize) -> String {
    let mut s = String::from("<x:workbookView");
    if let Some(v) = v {
        for (name, on) in [
            ("showHorizontalScroll", v.show_horizontal_scroll),
            ("showVerticalScroll", v.show_vertical_scroll),
            ("showSheetTabs", v.show_sheet_tabs),
        ] {
            if !on {
                let _ = write!(s, " {name}=\"0\"");
            }
        }
        let _ = write!(
            s,
            " xWindow=\"{}\" yWindow=\"{}\" windowWidth=\"{}\" windowHeight=\"{}\"",
            v.x_window, v.y_window, v.window_width, v.window_height
        );
        if v.tab_ratio != 600 {
            let _ = write!(s, " tabRatio=\"{}\"", v.tab_ratio);
        }
        if v.first_sheet != 0 && (v.first_sheet as usize) < n {
            let _ = write!(s, " firstSheet=\"{}\"", v.first_sheet);
        }
    }
    let _ = write!(s, " activeTab=\"{active}\"/>");
    s
}

/// 통합 문서를 XML `.cell`(또는 `.xlsx`) 바이트로 쓴다.
pub fn write(wb: &Workbook, flavor: Flavor) -> Result<Vec<u8>> {
    let worksheets: Vec<&Sheet> = wb
        .sheets
        .iter()
        .filter(|s| s.kind == SheetKind::Worksheet)
        .collect();

    // 리치 텍스트: 같은 글자의 원본 공유 문자열에서 런을 가져온다
    let rich: HashMap<&str, &RichText> = wb
        .shared_strings
        .iter()
        .filter(|r| !r.runs.is_empty())
        .map(|r| (r.text.as_str(), r))
        .collect();

    let mut sst = Sst::new();
    let mut sheet_parts: Vec<(String, Vec<(String, String)>)> = Vec::new();
    // 선택된 시트 (없으면 처음 보이는 시트)
    let first_visible = worksheets
        .iter()
        .position(|s| s.view.tab_selected)
        .or_else(|| {
            worksheets
                .iter()
                .position(|s| s.state == SheetState::Visible)
        })
        .unwrap_or(0);
    for (i, sh) in worksheets.iter().enumerate() {
        let mut links = Vec::new();
        let xml = sheet_xml(sh, i + 1, i == first_visible, &mut sst, &rich, &mut links);
        sheet_parts.push((xml, links));
    }
    // 그리기 파트 이름: 원본 그대로 옮기는 파트(차트의 userShapes 등)와 겹치지 않게
    let mut next_drawing = 1;
    let drawing_names: Vec<Option<String>> = worksheets
        .iter()
        .map(|sh| {
            if sh.shapes.is_empty() {
                return None;
            }
            loop {
                let n = format!("xl/drawings/drawing{next_drawing}.xml");
                next_drawing += 1;
                if !wb.raw_parts.iter().any(|p| p.name == n) {
                    return Some(n);
                }
            }
        })
        .collect();
    // 그리기 파트 내용과 관계, 그리고 모델로 쓰는 차트 파트 (레거시에서 읽은 차트) — 콘텐츠 형식에 넣어야 하므로 먼저
    let mdw = crate::units::max_digit_width(wb);
    let mut chart_parts: Vec<(String, String)> = Vec::new();
    let mut drawings: Vec<Option<(String, Vec<Rel>)>> = Vec::new();
    for (sh, name) in worksheets.iter().zip(&drawing_names) {
        drawings.push(
            name.as_ref()
                .map(|_| drawing_xml(sh, mdw, &wb.raw_parts, &mut chart_parts)),
        );
    }

    // 원본 그대로 옮겨 쓰는 파트. Excel 형식에는 매크로 프로젝트를 쓰지 않는다
    let is_macro =
        |l: &PartLink| l.rel_type.ends_with("/vbaProject") || l.rel_type.ends_with("/vbsProject");
    let wb_links: Vec<&PartLink> = wb
        .part_links
        .iter()
        .filter(|l| flavor == Flavor::Cell || !is_macro(l))
        .collect();
    let skipped: Vec<String> = wb
        .part_links
        .iter()
        .filter(|l| flavor == Flavor::Xlsx && is_macro(l))
        .flat_map(|l| [l.part.clone(), crate::package::rels_part_for(&l.part)])
        .collect();
    let raw_parts: Vec<&RawPart> = wb
        .raw_parts
        .iter()
        .filter(|p| !skipped.contains(&p.name))
        .collect();
    let raw_has = |n: &str| wb.raw_parts.iter().any(|p| p.name == n);
    // 레거시 원본의 매크로 프로젝트: `_VBS_PROJECT_W` 스트림 하나짜리 복합 문서 (한셀과 같은 파트 이름)
    let vbs_part = match &wb.hancom.vbs_project {
        Some(st)
            if flavor == Flavor::Cell
                && !wb
                    .part_links
                    .iter()
                    .any(|l| l.rel_type == crate::ns::REL_HANCOM_VBS_PROJECT)
                && !raw_has("xl/vbsProject.bin") =>
        {
            Some(crate::cfb_write::write_streams(&[(
                crate::ns::VBS_PROJECT_STREAM.trim_start_matches('/'),
                st,
            )])?)
        }
        _ => None,
    };
    // 표 파트 이름: XML 원본의 파트는 그대로 (참), 모델로 쓰는 표는 새 이름 (거짓)
    let mut next_table = 1;
    let mut table_names: Vec<Vec<(String, bool)>> = Vec::new();
    for sh in &worksheets {
        let mut names = Vec::new();
        for t in &sh.tables {
            match &t.part {
                Some(p) if raw_has(p) => names.push((p.clone(), true)),
                _ => loop {
                    let n = format!("xl/tables/table{next_table}.xml");
                    next_table += 1;
                    if !raw_has(&n) {
                        names.push((n, false));
                        break;
                    }
                },
            }
        }
        table_names.push(names);
    }

    let main_ct = match flavor {
        Flavor::Cell => "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        Flavor::Xlsx => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
        }
    };
    let mut ct = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"{main_ct}\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>"
    );
    for i in 0..worksheets.len() {
        let _ = write!(
            ct,
            "<Override PartName=\"/xl/worksheets/sheet{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
            i + 1
        );
    }
    if worksheets.iter().any(|s| !s.comments.is_empty()) {
        ct.push_str("<Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/>");
    }
    for (i, sh) in worksheets.iter().enumerate() {
        if !sh.comments.is_empty() {
            let _ = write!(
                ct,
                "<Override PartName=\"/xl/comments{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml\"/>",
                i + 1
            );
        }
    }
    for i in 0..wb.external_links.len() {
        let _ = write!(
            ct,
            "<Override PartName=\"/xl/externalLinks/externalLink{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml\"/>",
            i + 1
        );
    }
    for name in drawing_names.iter().flatten() {
        let _ = write!(
            ct,
            "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/>"
        );
    }
    for (name, _) in &chart_parts {
        let _ = write!(
            ct,
            "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\"/>"
        );
    }
    for (n, _) in table_names.iter().flatten().filter(|(_, raw)| !raw) {
        let _ = write!(
            ct,
            "<Override PartName=\"/{n}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/>"
        );
    }
    if vbs_part.is_some() {
        let _ = write!(
            ct,
            "<Override PartName=\"/xl/vbsProject.bin\" ContentType=\"{}\"/>",
            crate::ns::CT_HANCOM_VBS_PROJECT
        );
    }
    for p in &raw_parts {
        if let Some(t) = &p.content_type
            && !p.name.ends_with(".rels")
        {
            let _ = write!(
                ct,
                "<Override PartName=\"/{}\" ContentType=\"{}\"/>",
                esc(&p.name),
                esc(t)
            );
        }
    }
    if wb.theme.as_ref().is_some_and(|t| t.source.is_some()) {
        ct.push_str("<Override PartName=\"/xl/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>");
    }
    if !wb.properties.is_empty() {
        ct.push_str("<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>");
    }
    ct.push_str("</Types>");

    let core = core_xml(&wb.properties);
    let root_rels = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>{}</Relationships>",
        if core.is_some() {
            "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>"
        } else {
            ""
        }
    );
    let app = format!(
        "{DECL}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>rcell</Application>{}<AppVersion>{}</AppVersion></Properties>",
        wb.total_time
            .map(|t| format!("<TotalTime>{t}</TotalTime>"))
            .unwrap_or_default(),
        env!("CARGO_PKG_VERSION")
    );

    // workbook.xml + rels
    let protection = wb.protection.as_ref().map_or(String::new(), |p| {
        let mut s = String::from("<x:workbookProtection");
        if let Some(pw) = &p.password {
            let _ = write!(s, " workbookPassword=\"{}\"", esc(pw));
        }
        if p.lock_structure {
            s.push_str(" lockStructure=\"1\"");
        }
        if p.lock_windows {
            s.push_str(" lockWindows=\"1\"");
        }
        s.push_str("/>");
        s
    });
    let code_name = wb
        .code_name
        .as_ref()
        .map(|c| format!(" codeName=\"{}\"", esc(c)))
        .unwrap_or_default();
    let mut wbx = format!(
        "{DECL}<x:workbook xmlns:r=\"{REL}\" xmlns:x=\"{SML}\"><x:fileVersion appName=\"rcell\"/><x:workbookPr date1904=\"{}\"{code_name}/>{protection}<x:bookViews>{}</x:bookViews><x:sheets>",
        wb.date1904 as u8,
        workbook_view_xml(wb.view.as_ref(), first_visible, worksheets.len())
    );
    let mut wb_rels = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (i, sh) in worksheets.iter().enumerate() {
        let state = match sh.state {
            SheetState::Visible => "",
            SheetState::Hidden => " state=\"hidden\"",
            SheetState::VeryHidden => " state=\"veryHidden\"",
        };
        let _ = write!(
            wbx,
            "<x:sheet name=\"{}\" sheetId=\"{}\"{state} r:id=\"rId{}\"/>",
            esc(&sh.name),
            i + 1,
            i + 1
        );
        let _ = write!(
            wb_rels,
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{}.xml\"/>",
            i + 1,
            i + 1
        );
    }
    wbx.push_str("</x:sheets>");
    let n = worksheets.len();
    let _ = write!(
        wb_rels,
        "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/>",
        n + 1,
        n + 2
    );
    let theme = wb.theme.as_ref().and_then(|t| t.source.as_deref());
    if theme.is_some() {
        wb_rels.push_str("<Relationship Id=\"rIdTheme\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>");
    }
    // 외부 연결: 순서가 수식의 [n] 번호다 (CT_Workbook 에서 definedNames 앞)
    if !wb.external_links.is_empty() {
        wbx.push_str("<x:externalReferences>");
        for i in 0..wb.external_links.len() {
            let id = n + 3 + i;
            let _ = write!(wbx, "<x:externalReference r:id=\"rId{id}\"/>");
            let _ = write!(
                wb_rels,
                "<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink\" Target=\"externalLinks/externalLink{}.xml\"/>",
                i + 1
            );
        }
        wbx.push_str("</x:externalReferences>");
    }
    // 원본 그대로의 통합 문서 수준 파트 (관계 rIdX<n>)
    for (k, l) in wb_links.iter().enumerate() {
        let _ = write!(
            wb_rels,
            "<Relationship Id=\"rIdX{}\" Type=\"{}\" Target=\"/{}\"/>",
            k + 1,
            esc(&l.rel_type),
            esc(&l.part)
        );
    }
    if vbs_part.is_some() {
        let _ = write!(
            wb_rels,
            "<Relationship Id=\"rIdVbs\" Type=\"{}\" Target=\"vbsProject.bin\"/>",
            crate::ns::REL_HANCOM_VBS_PROJECT
        );
    }
    wb_rels.push_str("</Relationships>");
    let names: Vec<_> = wb
        .defined_names
        .iter()
        .filter(|d| !d.formula.is_empty())
        .collect();
    if !names.is_empty() {
        wbx.push_str("<x:definedNames>");
        for d in names {
            let _ = write!(wbx, "<x:definedName name=\"{}\"", esc(&d.name));
            if let Some(l) = d.local_sheet_id {
                let _ = write!(wbx, " localSheetId=\"{l}\"");
            }
            if d.hidden {
                wbx.push_str(" hidden=\"1\"");
            }
            if let Some(c) = &d.comment {
                let _ = write!(wbx, " comment=\"{}\"", esc(c));
            }
            let _ = write!(wbx, ">{}</x:definedName>", esc(&d.formula));
        }
        wbx.push_str("</x:definedNames>");
    }
    match wb.hancom.calc_id {
        Some(id) => {
            let _ = write!(
                wbx,
                "<x:calcPr xmlns:mc=\"{MC}\" xmlns:hs=\"{HS}\" mc:Ignorable=\"hs\" hs:hclCalcId=\"{id}\"/>"
            );
        }
        None => wbx.push_str("<x:calcPr/>"),
    }
    // 피벗 캐시 (CT_Workbook 에서 calcPr 다음)
    let caches: Vec<(u32, usize)> = wb_links
        .iter()
        .enumerate()
        .filter_map(|(k, l)| Some((l.cache_id?, k + 1)))
        .collect();
    if !caches.is_empty() {
        wbx.push_str("<x:pivotCaches>");
        for (id, k) in caches {
            let _ = write!(wbx, "<x:pivotCache cacheId=\"{id}\" r:id=\"rIdX{k}\"/>");
        }
        wbx.push_str("</x:pivotCaches>");
    }
    wbx.push_str("</x:workbook>");

    // ZIP
    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, body: &[u8]| -> Result<()> {
        z.start_file(name, opts)?;
        z.write_all(body)?;
        Ok(())
    };
    put("[Content_Types].xml", ct.as_bytes())?;
    put("_rels/.rels", root_rels.as_bytes())?;
    put("docProps/app.xml", app.as_bytes())?;
    if let Some(x) = &core {
        put("docProps/core.xml", x.as_bytes())?;
    }
    put("xl/workbook.xml", wbx.as_bytes())?;
    put("xl/_rels/workbook.xml.rels", wb_rels.as_bytes())?;
    put("xl/styles.xml", styles_xml(&wb.styles, flavor).as_bytes())?;
    if let Some(t) = theme {
        put("xl/theme/theme1.xml", t)?;
    }
    put("xl/sharedStrings.xml", sst.xml().as_bytes())?;
    for (i, (xml, links)) in sheet_parts.iter().enumerate() {
        put(&format!("xl/worksheets/sheet{}.xml", i + 1), xml.as_bytes())?;
        let notes = !worksheets[i].comments.is_empty();
        if !links.is_empty()
            || notes
            || drawing_names[i].is_some()
            || !table_names[i].is_empty()
            || !worksheets[i].part_links.is_empty()
        {
            let mut r = format!(
                "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
            );
            if let (Some(name), Some((xml, rels))) = (&drawing_names[i], drawings[i].take()) {
                let _ = write!(
                    r,
                    "<Relationship Id=\"rIdDr\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"/{name}\"/>"
                );
                put(name, xml.as_bytes())?;
                if !rels.is_empty() {
                    let mut dr = format!(
                        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
                    );
                    for (id, ty, target, external) in rels {
                        let (t, mode) = if external {
                            (target, " TargetMode=\"External\"")
                        } else {
                            (format!("/{target}"), "")
                        };
                        let _ = write!(
                            dr,
                            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{mode}/>",
                            esc(&id),
                            esc(&ty),
                            esc(&t)
                        );
                    }
                    dr.push_str("</Relationships>");
                    put(&crate::package::rels_part_for(name), dr.as_bytes())?;
                }
            }
            for (id, target) in links {
                let _ = write!(
                    r,
                    "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
                    esc(target)
                );
            }
            for (j, (name, raw)) in table_names[i].iter().enumerate() {
                let _ = write!(
                    r,
                    "<Relationship Id=\"rIdT{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/table\" Target=\"/{}\"/>",
                    j + 1,
                    esc(name)
                );
                if !raw {
                    put(name, table_xml(&worksheets[i].tables[j]).as_bytes())?;
                }
            }
            for (k, l) in worksheets[i].part_links.iter().enumerate() {
                let _ = write!(
                    r,
                    "<Relationship Id=\"rIdP{}\" Type=\"{}\" Target=\"/{}\"/>",
                    k + 1,
                    esc(&l.rel_type),
                    esc(&l.part)
                );
            }
            if notes {
                let _ = write!(
                    r,
                    "<Relationship Id=\"rIdCmt\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments\" Target=\"../comments{0}.xml\"/><Relationship Id=\"rIdVml\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing\" Target=\"../drawings/vmlDrawing{0}.vml\"/>",
                    i + 1
                );
                put(
                    &format!("xl/comments{}.xml", i + 1),
                    comments_xml(worksheets[i]).as_bytes(),
                )?;
                put(
                    &format!("xl/drawings/vmlDrawing{}.vml", i + 1),
                    comments_vml(worksheets[i], i + 1).as_bytes(),
                )?;
            }
            r.push_str("</Relationships>");
            put(
                &format!("xl/worksheets/_rels/sheet{}.xml.rels", i + 1),
                r.as_bytes(),
            )?;
        }
    }
    for (i, link) in wb.external_links.iter().enumerate() {
        let mut x = format!(
            "{DECL}<x:externalLink xmlns:r=\"{REL}\" xmlns:x=\"{SML}\"><x:externalBook r:id=\"rId1\"><x:sheetNames>"
        );
        for name in &link.sheet_names {
            let _ = write!(x, "<x:sheetName val=\"{}\"/>", esc(name));
        }
        x.push_str("</x:sheetNames>");
        if !link.sheet_data.is_empty() {
            x.push_str("<x:sheetDataSet>");
            for sd in &link.sheet_data {
                let _ = write!(x, "<x:sheetData sheetId=\"{}\"", sd.sheet_id);
                if sd.refresh_error {
                    x.push_str(" refreshError=\"1\"");
                }
                if sd.cells.is_empty() {
                    x.push_str("/>");
                    continue;
                }
                x.push('>');
                let mut row = None;
                for (r, v) in &sd.cells {
                    if row != Some(r.row) {
                        if row.is_some() {
                            x.push_str("</x:row>");
                        }
                        let _ = write!(x, "<x:row r=\"{}\">", r.row + 1);
                        row = Some(r.row);
                    }
                    let (t, text) = match v {
                        CellValue::Number(n) => ("", num(*n)),
                        CellValue::Bool(b) => (" t=\"b\"", (*b as u8).to_string()),
                        CellValue::Error(e) => (" t=\"e\"", esc(e)),
                        CellValue::Text(s) | CellValue::Date(s) => (" t=\"str\"", esc(s)),
                        CellValue::Empty => {
                            let _ = write!(x, "<x:cell r=\"{r}\"/>");
                            continue;
                        }
                    };
                    let _ = write!(x, "<x:cell r=\"{r}\"{t}><x:v>{text}</x:v></x:cell>");
                }
                x.push_str("</x:row></x:sheetData>");
            }
            x.push_str("</x:sheetDataSet>");
        }
        x.push_str("</x:externalBook></x:externalLink>");
        put(
            &format!("xl/externalLinks/externalLink{}.xml", i + 1),
            x.as_bytes(),
        )?;
        let r = format!(
            "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{}\" Target=\"{}\" TargetMode=\"External\"/></Relationships>",
            if link.path_missing {
                "http://schemas.microsoft.com/office/2006/relationships/xlExternalLinkPath/xlPathMissing"
            } else {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath"
            },
            esc(&link.target)
        );
        put(
            &format!("xl/externalLinks/_rels/externalLink{}.xml.rels", i + 1),
            r.as_bytes(),
        )?;
    }
    // 모델로 쓴 차트 파트
    for (name, xml) in &chart_parts {
        put(name, xml.as_bytes())?;
    }
    // 원본 그대로의 파트 (차트·그림·표·피벗·매크로 프로젝트 …와 그 관계 파일)
    for p in &raw_parts {
        put(&p.name, &p.data)?;
    }
    if let Some(v) = &vbs_part {
        put("xl/vbsProject.bin", v)?;
    }
    let bytes = z.finish()?.into_inner();
    // 구조 잠금 문서의 .xlsx 는 한셀처럼 기본 암호로 감싼다 (xlsx 대조 실험: 한셀의 .xlsx 저장은 바이너리처럼
    // EncryptionInfo·EncryptedPackage 복합 문서 — docs/format/other-formats.md)
    if flavor == Flavor::Xlsx && wb.protection.as_ref().is_some_and(|p| p.lock_structure) {
        return Ok(crate::binary::encrypt::wrap(&bytes));
    }
    Ok(bytes)
}

/// 표 파트 (`x:table`) — 모델로 쓸 때 (바이너리·레거시 원본, XML 원본은 파트를 그대로 옮긴다)
fn table_xml(t: &Table) -> String {
    let mut s = format!(
        "{DECL}<x:table xmlns:x=\"{SML}\" id=\"{}\" name=\"{}\" displayName=\"{}\" ref=\"{}\"",
        t.id,
        esc(&t.name),
        esc(&t.display_name),
        esc(&t.range)
    );
    if let Some(v) = &t.table_type {
        let _ = write!(s, " tableType=\"{}\"", esc(v));
    }
    if t.header_rows != 1 {
        let _ = write!(s, " headerRowCount=\"{}\"", t.header_rows);
    }
    if t.totals_rows != 0 {
        let _ = write!(s, " totalsRowCount=\"{}\"", t.totals_rows);
    }
    if !t.totals_shown {
        s.push_str(" totalsRowShown=\"0\"");
    }
    for (name, v) in [
        "headerRowDxfId",
        "dataDxfId",
        "totalsRowDxfId",
        "tableBorderDxfId",
        "headerRowBorderDxfId",
        "totalsRowBorderDxfId",
    ]
    .iter()
    .zip(t.dxfs)
    {
        if let Some(v) = v {
            let _ = write!(s, " {name}=\"{v}\"");
        }
    }
    for (name, v) in ["headerRowCellStyle", "dataCellStyle", "totalsRowCellStyle"]
        .iter()
        .zip(&t.cell_styles)
    {
        if let Some(v) = v {
            let _ = write!(s, " {name}=\"{}\"", esc(v));
        }
    }
    if let Some(c) = &t.comment {
        let _ = write!(s, " comment=\"{}\"", esc(c));
    }
    if let Some(c) = t.connection_id {
        let _ = write!(s, " connectionId=\"{c}\"");
    }
    s.push('>');
    if let Some(af) = &t.auto_filter {
        s.push_str(&auto_filter_xml_of(af));
    }
    let _ = write!(s, "<x:tableColumns count=\"{}\">", t.columns.len());
    for c in &t.columns {
        let _ = write!(
            s,
            "<x:tableColumn id=\"{}\" name=\"{}\"",
            c.id,
            esc(&c.name)
        );
        if let Some(v) = &c.unique_name {
            let _ = write!(s, " uniqueName=\"{}\"", esc(v));
        }
        if let Some(v) = &c.totals_function {
            let _ = write!(s, " totalsRowFunction=\"{}\"", esc(v));
        }
        if let Some(v) = &c.totals_label {
            let _ = write!(s, " totalsRowLabel=\"{}\"", esc(v));
        }
        if let Some(v) = c.query_table_field_id {
            let _ = write!(s, " queryTableFieldId=\"{v}\"");
        }
        for (name, v) in ["headerRowDxfId", "dataDxfId", "totalsRowDxfId"]
            .iter()
            .zip(c.dxfs)
        {
            if let Some(v) = v {
                let _ = write!(s, " {name}=\"{v}\"");
            }
        }
        if c.calculated_formula.is_none() && c.totals_formula.is_none() {
            s.push_str("/>");
            continue;
        }
        s.push('>');
        if let Some(f) = &c.calculated_formula {
            let _ = write!(
                s,
                "<x:calculatedColumnFormula>{}</x:calculatedColumnFormula>",
                esc(f)
            );
        }
        if let Some(f) = &c.totals_formula {
            let _ = write!(s, "<x:totalsRowFormula>{}</x:totalsRowFormula>", esc(f));
        }
        s.push_str("</x:tableColumn>");
    }
    s.push_str("</x:tableColumns>");
    if let Some(st) = &t.style {
        s.push_str("<x:tableStyleInfo");
        if let Some(n) = &st.name {
            let _ = write!(s, " name=\"{}\"", esc(n));
        }
        let _ = write!(
            s,
            " showFirstColumn=\"{}\" showLastColumn=\"{}\" showRowStripes=\"{}\" showColumnStripes=\"{}\"/>",
            st.first_column as u8,
            st.last_column as u8,
            st.row_stripes as u8,
            st.column_stripes as u8
        );
    }
    s.push_str("</x:table>");
    s
}

const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const DML: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const DML_CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// 그리기 관계: (ID, 종류, 대상, 외부)
pub(crate) type Rel = (String, String, String, bool);

const CHART_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";

/// 그리기 파트와 그 관계. XML 원본의 앵커는 그대로, 나머지는 모델로 쓴다. 모델의 차트(레거시에서 읽은 것)는
/// 차트 파트를 새로 만들어 `chart_parts` 에 (파트 이름, XML) 로 더한다. 바이너리 쓰기도 쓴다.
pub(crate) fn drawing_xml(
    sheet: &Sheet,
    mdw: u32,
    raw_parts: &[crate::model::RawPart],
    chart_parts: &mut Vec<(String, String)>,
) -> (String, Vec<Rel>) {
    use crate::model::ShapeKind;
    let mut s = format!(
        "{DECL}<xdr:wsDr xmlns:xdr=\"{XDR}\" xmlns:a=\"{DML}\" xmlns:r=\"{REL}\" xmlns:c=\"{DML_CHART}\">"
    );
    let mut rels: Vec<Rel> = Vec::new();
    // 번호 없는 차트 도형의 cNvPr id: 한셀처럼 2 부터, 다른 도형의 번호와 겹치지 않게
    fn ids(list: &[crate::model::Shape], out: &mut Vec<u32>) {
        for x in list {
            out.push(x.id);
            ids(&x.children, out);
        }
    }
    let mut used = Vec::new();
    ids(&sheet.shapes, &mut used);
    let mut next_id = 2;
    for sh in &sheet.shapes {
        match (&sh.raw, &sh.chart) {
            (Some(raw), _) => {
                s.push_str(raw);
                for r in &sh.raw_rels {
                    if !rels.iter().any(|x| x.0 == r.0) {
                        rels.push(r.clone());
                    }
                }
            }
            (None, Some(chart)) if sh.kind == ShapeKind::Chart => {
                let id = if sh.id != 0 {
                    sh.id
                } else {
                    while used.contains(&next_id) {
                        next_id += 1;
                    }
                    used.push(next_id);
                    next_id
                };
                let name = next_chart_name(raw_parts, chart_parts);
                let rid = format!("rIdChart{}", rels.len() + 1);
                rels.push((rid.clone(), CHART_REL.into(), name.clone(), false));
                chart_parts.push((name, crate::chart_xml::chart_xml(chart)));
                s.push_str(&chart_anchor_xml(sh, id, &rid, sheet, mdw));
            }
            (None, _) => s.push_str(&anchor_xml(sh, &mut rels)),
        }
    }
    s.push_str("</xdr:wsDr>");
    (s, rels)
}

/// 원본 그대로 옮기는 파트·이미 쓴 차트와 겹치지 않는 다음 차트 파트 이름
fn next_chart_name(raw_parts: &[crate::model::RawPart], made: &[(String, String)]) -> String {
    (made.len() + 1..)
        .map(|n| format!("xl/charts/chart{n}.xml"))
        .find(|n| !raw_parts.iter().any(|p| &p.name == n) && !made.iter().any(|m| &m.0 == n))
        .unwrap_or_default()
}

/// 칸 기준 앵커의 시트 안 절대 위치와 크기 (EMU): 앞 열·행의 픽셀 합 + 칸 안 오프셋
fn anchor_rect(sheet: &Sheet, mdw: u32, a: &crate::model::Anchor) -> (i64, i64, i64, i64) {
    use crate::units::{EMU_PER_PX, row_height_px, sheet_col_px};
    let col_x = |c: u32| {
        (0..c)
            .map(|i| i64::from(sheet_col_px(sheet, i, mdw)))
            .sum::<i64>()
            * EMU_PER_PX
    };
    let def_row = i64::from(row_height_px(sheet.default_row_height.unwrap_or(15.0)));
    // 높이가 따로 있는 행만 기본 높이와의 차이를 더한다 (행 수가 많아도 빠르게)
    let row_y = |r: u32| {
        let extra: i64 = sheet
            .rows
            .iter()
            .filter(|x| x.index >= 1 && x.index - 1 < r)
            .map(|x| {
                let px = if x.hidden {
                    0
                } else {
                    x.height.map_or(def_row, |h| i64::from(row_height_px(h)))
                };
                px - def_row
            })
            .sum();
        (i64::from(r) * def_row + extra) * EMU_PER_PX
    };
    let x = col_x(a.from.col) + a.from.col_off;
    let y = row_y(a.from.row) + a.from.row_off;
    let (x2, y2) = match &a.to {
        Some(t) => (col_x(t.col) + t.col_off, row_y(t.row) + t.row_off),
        None => {
            let (cx, cy) = a.ext.unwrap_or((0, 0));
            (x + cx, y + cy)
        }
    };
    (x, y, (x2 - x).max(0), (y2 - y).max(0))
}

/// 모델 차트의 앵커: 한셀이 레거시 차트를 읽어 쓰는 꼴 (`*.legacy.as.cell` 의 그리기 파트) —
/// `graphicFrame fPublished="0"`, `cNvPr id name`, 시트 안 절대 위치 `xdr:xfrm`, `c:chart r:id`,
/// `clientData fLocksWithSheet="1" fPrintsWithSheet="1"`
fn chart_anchor_xml(
    sh: &crate::model::Shape,
    id: u32,
    rid: &str,
    sheet: &Sheet,
    mdw: u32,
) -> String {
    use crate::model::AnchorKind;
    let a = &sh.anchor;
    let marker = |tag: &str, m: &crate::model::Marker| {
        format!(
            "<xdr:{tag}><xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{tag}>",
            m.col, m.col_off, m.row, m.row_off
        )
    };
    let tag = match a.kind {
        AnchorKind::TwoCell => "twoCellAnchor",
        AnchorKind::OneCell => "oneCellAnchor",
        AnchorKind::Absolute => "absoluteAnchor",
    };
    let mut s = format!("<xdr:{tag}");
    if let Some(e) = &a.edit_as {
        let _ = write!(s, " editAs=\"{}\"", esc(e));
    }
    s.push('>');
    let (x, y, cx, cy) = match a.kind {
        AnchorKind::Absolute => {
            let (x, y) = a.pos.unwrap_or((0, 0));
            let (cx, cy) = a.ext.unwrap_or((0, 0));
            let _ = write!(s, "<xdr:pos x=\"{x}\" y=\"{y}\"/>");
            (x, y, cx, cy)
        }
        _ => {
            s.push_str(&marker("from", &a.from));
            anchor_rect(sheet, mdw, a)
        }
    };
    if let Some(t) = &a.to {
        s.push_str(&marker("to", t));
    }
    if a.kind != AnchorKind::TwoCell {
        let (ex, ey) = a.ext.unwrap_or((cx, cy));
        let _ = write!(s, "<xdr:ext cx=\"{ex}\" cy=\"{ey}\"/>");
    }
    let _ = write!(
        s,
        "<xdr:graphicFrame fPublished=\"0\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{id}\" name=\"{}\"{}{}/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"{DML_CHART}\"><c:chart r:id=\"{rid}\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData fLocksWithSheet=\"1\" fPrintsWithSheet=\"1\"/></xdr:{tag}>",
        esc(&sh.name),
        sh.descr
            .as_ref()
            .map(|d| format!(" descr=\"{}\"", esc(d)))
            .unwrap_or_default(),
        if sh.hidden { " hidden=\"1\"" } else { "" }
    );
    s
}

/// 모델로 앵커를 쓴다 (레거시에서 읽은 도형·글상자·그림). 그림은 파트가 없으면, 차트는 내용이 없으면 쓰지 않는다
/// (내용이 있는 차트는 [`chart_anchor_xml`]).
fn anchor_xml(sh: &crate::model::Shape, rels: &mut Vec<Rel>) -> String {
    use crate::model::{AnchorKind, ShapeKind};
    let picture = sh.kind == ShapeKind::Picture && sh.target.is_some();
    if !picture
        && !matches!(
            sh.kind,
            ShapeKind::Shape | ShapeKind::Connector | ShapeKind::Group
        )
    {
        return String::new();
    }
    let a = &sh.anchor;
    let marker = |tag: &str, m: &crate::model::Marker| {
        format!(
            "<xdr:{tag}><xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{tag}>",
            m.col, m.col_off, m.row, m.row_off
        )
    };
    let mut s = String::new();
    let tag = match a.kind {
        AnchorKind::TwoCell => "twoCellAnchor",
        AnchorKind::OneCell => "oneCellAnchor",
        AnchorKind::Absolute => "absoluteAnchor",
    };
    let _ = write!(s, "<xdr:{tag}");
    if let Some(e) = &a.edit_as {
        let _ = write!(s, " editAs=\"{}\"", esc(e));
    }
    s.push('>');
    match a.kind {
        AnchorKind::Absolute => {
            let (x, y) = a.pos.unwrap_or((0, 0));
            let _ = write!(s, "<xdr:pos x=\"{x}\" y=\"{y}\"/>");
        }
        _ => s.push_str(&marker("from", &a.from)),
    }
    if let Some(t) = &a.to {
        s.push_str(&marker("to", t));
    }
    if a.kind != AnchorKind::TwoCell {
        let (cx, cy) = a.ext.unwrap_or((0, 0));
        let _ = write!(s, "<xdr:ext cx=\"{cx}\" cy=\"{cy}\"/>");
    }
    s.push_str(&object_xml(sh, rels));
    // xdr:clientData: 시트 보호와 함께 잠금·인쇄 (없으면 참)
    s.push_str("<xdr:clientData");
    for (name, v) in [
        ("fLocksWithSheet", sh.locks_with_sheet),
        ("fPrintsWithSheet", sh.prints_with_sheet),
    ] {
        if let Some(v) = v {
            let _ = write!(s, " {name}=\"{}\"", v as u8);
        }
    }
    s.push_str("/>");
    let _ = write!(s, "</xdr:{tag}>");
    s
}

fn object_xml(sh: &crate::model::Shape, rels: &mut Vec<Rel>) -> String {
    use crate::model::ShapeKind;
    let mut s = String::new();
    let (el, nv, cnv) = match sh.kind {
        ShapeKind::Connector => ("cxnSp", "nvCxnSpPr", "cNvCxnSpPr"),
        ShapeKind::Group => ("grpSp", "nvGrpSpPr", "cNvGrpSpPr"),
        ShapeKind::Picture => ("pic", "nvPicPr", "cNvPicPr"),
        _ => ("sp", "nvSpPr", "cNvSpPr"),
    };
    let _ = write!(
        s,
        "<xdr:{el}><xdr:{nv}><xdr:cNvPr id=\"{}\" name=\"{}\"{}{}/>{}</xdr:{nv}>",
        sh.id,
        esc(&sh.name),
        sh.descr
            .as_ref()
            .map(|d| format!(" descr=\"{}\"", esc(d)))
            .unwrap_or_default(),
        if sh.hidden { " hidden=\"1\"" } else { "" },
        if sh.locks.is_empty() {
            format!("<xdr:{cnv}/>")
        } else {
            // 잠금: 도형 a:spLocks, 연결선 a:cxnSpLocks, 그림 a:picLocks, 그룹 a:grpSpLocks
            let tag = match sh.kind {
                ShapeKind::Connector => "cxnSpLocks",
                ShapeKind::Picture => "picLocks",
                ShapeKind::Group => "grpSpLocks",
                _ => "spLocks",
            };
            let attrs: String = sh
                .locks
                .iter()
                .map(|l| format!(" {}=\"1\"", esc(l)))
                .collect();
            format!("<xdr:{cnv}><a:{tag}{attrs}/></xdr:{cnv}>")
        }
    );
    if let (ShapeKind::Picture, Some(t)) = (sh.kind, &sh.target) {
        const IMAGE: &str =
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
        let id = match rels.iter().find(|r| &r.2 == t) {
            Some(r) => r.0.clone(),
            None => {
                let id = format!("rIdImg{}", rels.len() + 1);
                rels.push((id.clone(), IMAGE.into(), t.clone(), false));
                id
            }
        };
        let _ = write!(
            s,
            "<xdr:blipFill><a:blip r:embed=\"{id}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill>"
        );
    }
    let pr = if sh.kind == ShapeKind::Group {
        "grpSpPr"
    } else {
        "spPr"
    };
    let _ = write!(s, "<xdr:{pr}>");
    if let Some(x) = &sh.xfrm {
        let _ = write!(
            s,
            "<a:xfrm{}{}{}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/>{}</a:xfrm>",
            if x.rot != 0 {
                format!(" rot=\"{}\"", x.rot)
            } else {
                String::new()
            },
            if x.flip_h { " flipH=\"1\"" } else { "" },
            if x.flip_v { " flipV=\"1\"" } else { "" },
            x.x,
            x.y,
            x.cx,
            x.cy,
            if sh.kind == ShapeKind::Group {
                format!(
                    "<a:chOff x=\"{}\" y=\"{}\"/><a:chExt cx=\"{}\" cy=\"{}\"/>",
                    x.x, x.y, x.cx, x.cy
                )
            } else {
                String::new()
            }
        );
    }
    if sh.kind != ShapeKind::Group {
        // 모양이 없으면 쓰지 않는다 (한셀도 그렇게 쓴다). 사용자 정의 경로는 아직 모델에 없어 사각형으로.
        if let Some(g) = sh.geometry.as_deref() {
            let prst = if g == "custom" { "rect" } else { g };
            let _ = write!(
                s,
                "<a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom>",
                esc(prst)
            );
        }
        s.push_str(&paint_xml(sh.fill.as_ref()));
        if let Some(l) = &sh.line {
            s.push_str("<a:ln");
            if let Some(w) = l.width {
                let _ = write!(s, " w=\"{w}\"");
            }
            for (name, v) in [("cap", &l.cap), ("cmpd", &l.compound)] {
                if let Some(v) = v {
                    let _ = write!(s, " {name}=\"{}\"", esc(v));
                }
            }
            s.push('>');
            s.push_str(&paint_xml(l.fill.as_ref()));
            if let Some(d) = &l.dash {
                let _ = write!(s, "<a:prstDash val=\"{}\"/>", esc(d));
            }
            if let Some(j) = &l.join {
                let _ = write!(s, "<a:{}/>", esc(j));
            }
            for (tag, end) in [("headEnd", &l.head), ("tailEnd", &l.tail)] {
                if let Some(e) = end {
                    let _ = write!(s, "<a:{tag}");
                    for (name, v) in [("type", &e.kind), ("w", &e.w), ("len", &e.len)] {
                        if let Some(v) = v {
                            let _ = write!(s, " {name}=\"{}\"", esc(v));
                        }
                    }
                    s.push_str("/>");
                }
            }
            s.push_str("</a:ln>");
        }
        if let Some(sd) = &sh.shadow {
            let _ = write!(s, "<a:effectLst><a:outerShdw");
            if sd.blur != 0 {
                let _ = write!(s, " blurRad=\"{}\"", sd.blur);
            }
            let _ = write!(s, " dist=\"{}\" dir=\"{}\"", sd.dist, sd.dir);
            if let Some(a) = &sd.align {
                let _ = write!(s, " algn=\"{}\"", esc(a));
            }
            let _ = write!(
                s,
                " rotWithShape=\"{}\">{}</a:outerShdw></a:effectLst>",
                sd.rot_with_shape as u8,
                draw_color_xml(&sd.color)
            );
        }
    }
    let _ = write!(s, "</xdr:{pr}>");
    if sh.kind == ShapeKind::Group {
        for c in &sh.children {
            s.push_str(&object_xml(c, rels));
        }
    } else if let Some(t) = &sh.text {
        s.push_str("<xdr:txBody><a:bodyPr");
        for (name, v) in [
            ("vertOverflow", &t.vert_overflow),
            ("horzOverflow", &t.horz_overflow),
            ("anchor", &t.anchor),
            ("wrap", &t.wrap),
            ("vert", &t.vert),
        ] {
            if let Some(v) = v {
                let _ = write!(s, " {name}=\"{}\"", esc(v));
            }
        }
        for (i, name) in ["lIns", "tIns", "rIns", "bIns"].iter().enumerate() {
            if let Some(v) = t.insets[i] {
                let _ = write!(s, " {name}=\"{v}\"");
            }
        }
        s.push_str("/><a:lstStyle/>");
        for p in &t.paragraphs {
            s.push_str("<a:p>");
            if let Some(al) = &p.align {
                let _ = write!(s, "<a:pPr algn=\"{}\"/>", esc(al));
            }
            for r in &p.runs {
                if r.text == "\n" {
                    s.push_str("<a:br/>");
                    continue;
                }
                s.push_str("<a:r><a:rPr lang=\"ko-KR\"");
                if let Some(v) = r.size {
                    let _ = write!(s, " sz=\"{v}\"");
                }
                if let Some(v) = r.bold {
                    let _ = write!(s, " b=\"{}\"", v as u8);
                }
                if let Some(v) = r.italic {
                    let _ = write!(s, " i=\"{}\"", v as u8);
                }
                if let Some(v) = &r.underline {
                    let _ = write!(s, " u=\"{}\"", esc(v));
                }
                s.push('>');
                if let Some(c) = &r.color {
                    let _ = write!(s, "<a:solidFill>{}</a:solidFill>", draw_color_xml(c));
                }
                if let Some(f) = &r.latin {
                    let _ = write!(s, "<a:latin typeface=\"{}\"/>", esc(f));
                }
                if let Some(f) = &r.ea {
                    let _ = write!(s, "<a:ea typeface=\"{}\"/>", esc(f));
                }
                let _ = write!(s, "</a:rPr><a:t>{}</a:t></a:r>", esc(&r.text));
            }
            s.push_str("</a:p>");
        }
        s.push_str("</xdr:txBody>");
    }
    let _ = write!(s, "</xdr:{el}>");
    s
}

fn paint_xml(p: Option<&crate::model::Paint>) -> String {
    match p {
        Some(crate::model::Paint::None) => "<a:noFill/>".into(),
        Some(crate::model::Paint::Solid(c)) => {
            format!("<a:solidFill>{}</a:solidFill>", draw_color_xml(c))
        }
        _ => String::new(),
    }
}

fn draw_color_xml(c: &crate::model::DrawColor) -> String {
    let (kind, val) = c.value.split_once(':').unwrap_or(("srgb", &c.value));
    let el = match kind {
        "scheme" => "schemeClr",
        "sys" => "sysClr",
        "prst" => "prstClr",
        _ => "srgbClr",
    };
    let mods: String = c
        .mods
        .iter()
        .map(|(k, v)| format!("<a:{k} val=\"{v}\"/>"))
        .collect();
    if mods.is_empty() {
        format!("<a:{el} val=\"{}\"/>", esc(val))
    } else {
        format!("<a:{el} val=\"{}\">{mods}</a:{el}>", esc(val))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_and_drops_invalid_controls() {
        assert_eq!(esc("a<b>&\"c\u{1}"), "a&lt;b&gt;&amp;&quot;c");
    }

    #[test]
    fn dxf_border_writes_only_given_edges() {
        let b = Border {
            left: Some(BorderEdge {
                style: Some("thin".into()),
                color: None,
            }),
            right: Some(BorderEdge {
                style: None,
                color: Some(Color {
                    auto: true,
                    ..Default::default()
                }),
            }),
            horizontal: Some(BorderEdge {
                style: Some("double".into()),
                color: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            dxf_border_xml(&b),
            "<x:border><x:left style=\"thin\"/><x:right><x:color auto=\"1\"/></x:right><x:horizontal style=\"double\"/></x:border>"
        );
    }

    #[test]
    fn table_style_elements_in_type_order() {
        use crate::model::{TableStyle, TableStyleElement, TableStyles};
        let el = |kind: &str, dxf| TableStyleElement {
            kind: kind.into(),
            size: 1,
            dxf_id: Some(dxf),
        };
        let t = TableStyles {
            default_table_style: Some("T".into()),
            default_pivot_style: None,
            styles: vec![TableStyle {
                name: "T".into(),
                pivot: false,
                table: true,
                elements: vec![el("pageFieldLabels", 1), el("wholeTable", 0)],
            }],
        };
        let x = table_styles_xml(&t);
        assert!(x.contains("<x:tableStyle name=\"T\" pivot=\"0\" table=\"1\" count=\"2\">"));
        assert!(x.find("wholeTable").unwrap() < x.find("pageFieldLabels").unwrap());
    }
}
