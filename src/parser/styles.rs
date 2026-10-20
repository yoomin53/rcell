// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! `xl/styles.xml` 파서 (한컴 `hs:` 글꼴·문단 확장 포함).

use std::collections::BTreeMap;

use crate::model::{
    AbstractNum, Alignment, Border, BorderEdge, Color, Dxf, Fill, Font, GradientFill, GradientStop,
    HancomFontExt, HancomXfExt, NamedStyle, NumFmt, ParaShape, Protection, Styles, TableStyle,
    TableStyleElement, TableStyles, Xf,
};
use crate::ns;
use crate::xml::{Element, parse_bool};

pub fn parse_styles(root: &Element) -> Styles {
    let mut st = Styles::default();
    if let Some(e) = root.sml_child("numFmts") {
        st.num_fmts = e
            .sml_children("numFmt")
            .filter_map(|n| {
                Some(NumFmt {
                    id: n.attr_u32("numFmtId")?,
                    code: n.attr("formatCode").unwrap_or("").to_string(),
                })
            })
            .collect();
    }
    if let Some(e) = root.sml_child("fonts") {
        st.fonts = e.sml_children("font").map(parse_font).collect();
    }
    if let Some(e) = root.sml_child("fills") {
        st.fills = e.sml_children("fill").map(parse_fill).collect();
    }
    if let Some(e) = root.sml_child("borders") {
        st.borders = e.sml_children("border").map(parse_border).collect();
    }
    if let Some(e) = root.sml_child("cellStyleXfs") {
        st.cell_style_xfs = e.sml_children("xf").map(parse_xf).collect();
    }
    if let Some(e) = root.sml_child("cellXfs") {
        st.cell_xfs = e.sml_children("xf").map(parse_xf).collect();
    }
    if let Some(e) = root.sml_child("cellStyles") {
        st.cell_styles = e
            .sml_children("cellStyle")
            .map(|c| NamedStyle {
                name: c.attr("name").unwrap_or("").to_string(),
                xf_id: c.attr_u32("xfId").unwrap_or(0),
                builtin_id: c.attr_u32("builtinId"),
                i_level: c.attr_u32("iLevel"),
            })
            .collect();
    }
    if let Some(e) = root.sml_child("dxfs") {
        st.dxfs = e.sml_children("dxf").map(parse_dxf).collect();
    }
    if let Some(e) = root.sml_child("tableStyles") {
        st.table_styles = Some(TableStyles {
            default_table_style: e.attr("defaultTableStyle").map(str::to_string),
            default_pivot_style: e.attr("defaultPivotStyle").map(str::to_string),
            styles: e
                .sml_children("tableStyle")
                .map(|t| TableStyle {
                    name: t.attr("name").unwrap_or("").to_string(),
                    pivot: t.attr_bool("pivot").unwrap_or(true),
                    table: t.attr_bool("table").unwrap_or(true),
                    elements: t
                        .sml_children("tableStyleElement")
                        .map(|x| TableStyleElement {
                            kind: x.attr("type").unwrap_or("").to_string(),
                            size: x.attr_u32("size").unwrap_or(1),
                            dxf_id: x.attr_u32("dxfId"),
                        })
                        .collect(),
                })
                .collect(),
        });
    }
    if let Some(e) = root.sml_child("abstractNums") {
        st.abstract_nums = e
            .sml_children("abstractNum")
            .map(|a| AbstractNum {
                num_type: a.attr_u32("abstractNumType").unwrap_or(0),
                id: a.attr_u32("abstractNumId").unwrap_or(0),
                start_num: a.attr_u32("startNum").unwrap_or(0),
                lvl_text: a.attr("lvlText").unwrap_or("").to_string(),
            })
            .collect();
    }
    st
}

fn parse_dxf(e: &Element) -> Dxf {
    Dxf {
        font: e.sml_child("font").map(parse_font),
        fill: e.sml_child("fill").map(parse_fill),
        border: e.sml_child("border").map(parse_border),
        num_fmt: e.sml_child("numFmt").and_then(|n| {
            Some(NumFmt {
                id: n.attr_u32("numFmtId")?,
                code: n.attr("formatCode").unwrap_or("").to_string(),
            })
        }),
        alignment: e.sml_child("alignment").map(parse_alignment),
        protection: e.sml_child("protection").map(parse_protection),
        hancom: parse_hancom_xf_ext(e),
    }
}

fn parse_protection(p: &Element) -> Protection {
    Protection {
        locked: p.attr_bool("locked"),
        hidden: p.attr_bool("hidden"),
    }
}

pub fn parse_color(e: &Element) -> Color {
    Color {
        rgb: e.attr("rgb").map(str::to_string),
        theme: e.attr_u32("theme"),
        tint: e.attr_f64("tint"),
        indexed: e.attr_u32("indexed"),
        auto: e.attr_bool("auto").unwrap_or(false),
    }
}

/// `<x:b/>` 처럼 val 이 생략되면 참인 불리언 요소.
fn flag(e: &Element) -> bool {
    e.attr("val").map(parse_bool).unwrap_or(true)
}

/// `x:font` 또는 공유 문자열 런의 `x:rPr` 를 해석한다.
pub fn parse_font(e: &Element) -> Font {
    let mut f = Font::default();
    let mut hancom: Option<HancomFontExt> = None;
    for c in e.elements() {
        let ns_uri = c.ns.as_deref().unwrap_or("");
        if ns::is_sml(ns_uri) {
            match c.name.as_str() {
                "name" | "rFont" => f.name = c.attr("val").map(str::to_string),
                "sz" => f.size = c.attr_f64("val"),
                "b" => f.bold = flag(c),
                "i" => f.italic = flag(c),
                "strike" => f.strike = flag(c),
                "dstrike" => f.double_strike = flag(c),
                "outline" => f.outline = flag(c),
                "shadow" => f.shadow = flag(c),
                "condense" => f.condense = flag(c),
                "extend" => f.extend = flag(c),
                "u" => {
                    let v = c.attr("val").unwrap_or("single");
                    f.underline = (v != "none").then(|| v.to_string());
                }
                "color" => f.color = Some(parse_color(c)),
                "vertAlign" => f.vert_align = c.attr("val").map(str::to_string),
                "family" => f.family = c.attr_u32("val"),
                "charset" => f.charset = c.attr_u32("val"),
                "scheme" => f.scheme = c.attr("val").map(str::to_string),
                _ => {}
            }
        } else if ns_uri == ns::HS {
            apply_hancom_font(hancom.get_or_insert_with(Default::default), c);
        }
    }
    // hs:extension="1" 만 있고 자식이 없는 경우도 확장 글꼴로 표시한다.
    if hancom.is_none() && e.attr_ns(ns::HS, "extension").is_some_and(parse_bool) {
        hancom = Some(HancomFontExt::default());
    }
    f.hancom = hancom;
    f
}

fn int_val(c: &Element) -> Option<i32> {
    c.attr("val").and_then(|v| v.trim().parse().ok())
}

fn str_val(c: &Element) -> Option<String> {
    c.attr("val").map(str::to_string)
}

fn apply_hancom_font(h: &mut HancomFontExt, c: &Element) {
    match c.name.as_str() {
        "size" => h.size = int_val(c),
        "ratio" => h.ratio = int_val(c),
        "spacing" => h.spacing = int_val(c),
        "offset" => h.offset = int_val(c),
        "underlineType" => h.underline_type = str_val(c),
        "underlineShape" => h.underline_shape = str_val(c),
        "underlineColor" => h.underline_color = Some(parse_color(c)),
        "strikeoutType" => h.strikeout_type = str_val(c),
        "strikeoutShape" => h.strikeout_shape = str_val(c),
        "strikeoutColor" => h.strikeout_color = Some(parse_color(c)),
        "outlineType" => h.outline_type = str_val(c),
        "shadowType" => h.shadow_type = str_val(c),
        "shadowColor" => h.shadow_color = Some(parse_color(c)),
        "shadowOffsetX" => h.shadow_offset_x = int_val(c),
        "shadowOffsetY" => h.shadow_offset_y = int_val(c),
        "shadeColor" => h.shade_color = Some(parse_color(c)),
        "emboss" => h.emboss = Some(flag(c)),
        "engrave" => h.engrave = Some(flag(c)),
        "superscript" => h.superscript = Some(flag(c)),
        "subscript" => h.subscript = Some(flag(c)),
        "useFontSpace" => h.use_font_space = Some(flag(c)),
        "useKerning" => h.use_kerning = Some(flag(c)),
        "diacsymMark" => h.diacsym_mark = str_val(c),
        other => {
            h.other.insert(other.to_string(), describe_unknown(c));
        }
    }
}

/// 모르는 요소를 `val` 또는 속성 나열로 요약한다.
fn describe_unknown(c: &Element) -> String {
    if let Some(v) = c.attr("val") {
        return v.to_string();
    }
    c.attrs
        .iter()
        .map(|a| format!("{}={}", a.name, a.value))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_fill(e: &Element) -> Fill {
    let mut f = Fill::default();
    if let Some(p) = e.sml_child("patternFill") {
        f.pattern_type = p.attr("patternType").map(str::to_string);
        f.fg_color = p.sml_child("fgColor").map(parse_color);
        f.bg_color = p.sml_child("bgColor").map(parse_color);
    }
    f.gradient = e.sml_child("gradientFill").map(|g| GradientFill {
        path: g.attr("type") == Some("path"),
        degree: g.attr_f64("degree").unwrap_or(0.0),
        left: g.attr_f64("left").unwrap_or(0.0),
        right: g.attr_f64("right").unwrap_or(0.0),
        top: g.attr_f64("top").unwrap_or(0.0),
        bottom: g.attr_f64("bottom").unwrap_or(0.0),
        stops: g
            .sml_children("stop")
            .map(|s| GradientStop {
                position: s.attr_f64("position").unwrap_or(0.0),
                color: s.sml_child("color").map(parse_color).unwrap_or_default(),
            })
            .collect(),
    });
    f
}

/// 있는 선은 스타일이 없어도 남긴다: 차등 서식에서 없는 선은 '그대로 둠', 스타일 없는 선은 '선 없음'
fn parse_edge(e: Option<&Element>) -> Option<BorderEdge> {
    let e = e?;
    Some(BorderEdge {
        style: e.attr("style").map(str::to_string),
        color: e.sml_child("color").map(parse_color),
    })
}

fn parse_border(e: &Element) -> Border {
    Border {
        left: parse_edge(e.sml_child("left").or_else(|| e.sml_child("start"))),
        right: parse_edge(e.sml_child("right").or_else(|| e.sml_child("end"))),
        top: parse_edge(e.sml_child("top")),
        bottom: parse_edge(e.sml_child("bottom")),
        diagonal: parse_edge(e.sml_child("diagonal")),
        vertical: parse_edge(e.sml_child("vertical")),
        horizontal: parse_edge(e.sml_child("horizontal")),
        diagonal_up: e.attr_bool("diagonalUp").unwrap_or(false),
        diagonal_down: e.attr_bool("diagonalDown").unwrap_or(false),
    }
}

fn parse_xf(e: &Element) -> Xf {
    let mut xf = Xf {
        num_fmt_id: e.attr_u32("numFmtId").unwrap_or(0),
        font_id: e.attr_u32("fontId").unwrap_or(0),
        fill_id: e.attr_u32("fillId").unwrap_or(0),
        border_id: e.attr_u32("borderId").unwrap_or(0),
        xf_id: e.attr_u32("xfId"),
        apply_number_format: e.attr_bool("applyNumberFormat"),
        apply_font: e.attr_bool("applyFont"),
        apply_fill: e.attr_bool("applyFill"),
        apply_border: e.attr_bool("applyBorder"),
        apply_alignment: e.attr_bool("applyAlignment"),
        apply_protection: e.attr_bool("applyProtection"),
        quote_prefix: e.attr_bool("quotePrefix").unwrap_or(false),
        ..Default::default()
    };
    xf.alignment = e.sml_child("alignment").map(parse_alignment);
    xf.protection = e.sml_child("protection").map(parse_protection);
    xf.hancom = parse_hancom_xf_ext(e);
    xf
}

fn parse_alignment(a: &Element) -> Alignment {
    Alignment {
        horizontal: a.attr("horizontal").map(str::to_string),
        vertical: a.attr("vertical").map(str::to_string),
        wrap_text: a.attr_bool("wrapText").unwrap_or(false),
        shrink_to_fit: a.attr_bool("shrinkToFit").unwrap_or(false),
        text_rotation: a.attr("textRotation").and_then(|v| v.trim().parse().ok()),
        indent: a.attr_u32("indent"),
        reading_order: a.attr_u32("readingOrder"),
    }
}

/// `x:xf` / `x:dxf` 에 붙은 `hs:` 속성과 자식.
fn parse_hancom_xf_ext(e: &Element) -> Option<HancomXfExt> {
    let mut ext = HancomXfExt::default();
    let mut has_ext = false;
    for a in e.attrs.iter().filter(|a| a.ns.as_deref() == Some(ns::HS)) {
        has_ext = true;
        if a.name == "applyExtension" {
            ext.apply_extension = parse_bool(&a.value);
        } else {
            ext.other.insert(a.name.clone(), a.value.clone());
        }
    }
    for c in e.elements().filter(|c| c.ns.as_deref() == Some(ns::HS)) {
        has_ext = true;
        if c.name == "parashape" {
            ext.parashape = Some(parse_parashape(c));
        } else if c.name == "accountUnderline" {
            ext.account_underline = hs_attr(c, "shape")
                .or_else(|| c.attr("val"))
                .map(str::to_string);
        } else {
            ext.other.insert(c.name.clone(), describe_unknown(c));
        }
    }
    has_ext.then_some(ext)
}

/// `hs:parashape` 의 속성은 샘플에서 `hs:` 접두사가 붙어 있었지만,
/// 접두사 없는 형태도 받아들인다.
fn hs_attr<'a>(e: &'a Element, name: &str) -> Option<&'a str> {
    e.attr_ns(ns::HS, name).or_else(|| e.attr(name))
}

fn parse_parashape(e: &Element) -> ParaShape {
    let int = |n: &str| hs_attr(e, n).and_then(|v| v.trim().parse().ok());
    let s = |n: &str| hs_attr(e, n).map(str::to_string);
    let known = [
        "lineSpacingType",
        "lineSpacing",
        "prevSpacing",
        "nextSpacing",
        "breakLatinWord",
        "breakNonLatinWord",
        "condense",
        "textAlignment",
        "fontLineHeight",
        "leftMargin",
        "rightMargin",
        "indentation",
        "abstractNumId",
        "abstractNumType",
    ];
    let other: BTreeMap<String, String> = e
        .attrs
        .iter()
        .filter(|a| !known.contains(&a.name.as_str()))
        .map(|a| (a.name.clone(), a.value.clone()))
        .collect();
    ParaShape {
        line_spacing_type: s("lineSpacingType"),
        line_spacing: int("lineSpacing"),
        prev_spacing: int("prevSpacing"),
        next_spacing: int("nextSpacing"),
        break_latin_word: s("breakLatinWord"),
        break_non_latin_word: s("breakNonLatinWord"),
        condense: int("condense"),
        text_alignment: s("textAlignment"),
        font_line_height: hs_attr(e, "fontLineHeight").map(parse_bool),
        left_margin: int("leftMargin"),
        right_margin: int("rightMargin"),
        indentation: int("indentation"),
        abstract_num_id: int("abstractNumId"),
        abstract_num_type: int("abstractNumType"),
        other,
    }
}
