// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! ODF 스프레드시트 `.ods` (OASIS ODF 1.2) 읽기·쓰기 — 시트, 칸 값·수식, 병합, 열 너비·숨김, 행 높이·숨김.
//!
//! 한셀의 `.ods` 저장(FileFormat 60)은 `content.xml`·`styles.xml`·`meta.xml`·`settings.xml`·매니페스트 (`mimetype`
//! 없음), 수식은 `of:=` 뒤에 A1 참조 그대로 (`[.A1]` 꼴이 아니다)와 인수 구분 `;`, 다른 시트 참조는 `#REF.` 로 잃는다.
//! 칸 서식은 표시 형식을 잃는다 (docs/format/other-formats.md 8절). rcell 은 값·수식·구조만 옮긴다.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use crate::error::{Error, Result};
use crate::model::{Cell, CellRange, CellRef, CellValue, ColInfo, Formula, RowInfo, Workbook};
use crate::xml::Element;

const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";

/// ZIP 바이트가 ODF 스프레드시트인가 (`content.xml` 이 있고 OPC `[Content_Types].xml` 이 없다)
pub fn is_ods(bytes: &[u8]) -> bool {
    let Ok(mut z) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    z.by_name("content.xml").is_ok() && z.by_name("[Content_Types].xml").is_err()
}

fn entry(bytes: &[u8], name: &str) -> Result<String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::InvalidValue {
        context: "ods".into(),
        value: e.to_string(),
    })?;
    let mut f = z
        .by_name(name)
        .map_err(|_| Error::MissingPart(name.into()))?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

/// 길이 (`1.94cm`, `0.45in`, `12pt`, `5mm`) → pt
fn length_pt(v: &str) -> Option<f64> {
    let v = v.trim();
    let (n, unit) = v.split_at(v.find(|c: char| c.is_ascii_alphabetic())?);
    let n: f64 = n.parse().ok()?;
    Some(match unit {
        "cm" => n / 2.54 * 72.0,
        "mm" => n / 25.4 * 72.0,
        "in" => n * 72.0,
        "pt" => n,
        "pc" => n * 12.0,
        "px" => n * 0.75,
        _ => return None,
    })
}

/// ODF 수식 (`of:=SUM([.A1:.A3];1)` 또는 한셀 꼴 `of:=SUM(A1:A3;1)`) → 셀 수식 글 (`=` 없이)
pub fn formula_from_odf(f: &str) -> String {
    let f = f
        .strip_prefix("of:")
        .or_else(|| f.strip_prefix("oooc:"))
        .unwrap_or(f);
    let f = f.strip_prefix('=').unwrap_or(f);
    let mut out = String::with_capacity(f.len());
    let mut chars = f.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                for d in chars.by_ref() {
                    out.push(d);
                    if d == '"' {
                        break;
                    }
                }
            }
            ';' => out.push(','),
            '[' => {
                // [시트.A1] / [.A1:.B2] / [$'시트 1'.$A$1]
                let mut r = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    r.push(d);
                }
                let parts: Vec<String> = r
                    .split(':')
                    .map(|p| {
                        let p = p.trim_start_matches('$');
                        match p.rfind('.') {
                            Some(i) if i > 0 => format!("{}!{}", &p[..i], &p[i + 1..]),
                            Some(_) => p[1..].to_string(),
                            None => p.to_string(),
                        }
                    })
                    .collect();
                out.push_str(&parts.join(":"));
            }
            // 한셀 꼴의 다른 시트 참조 `시트.A1` (괄호 없음): 이름(숫자로 시작하지 않는 낱말이나 따옴표 이름) 뒤의
            // `.` 이 칸 참조 앞이면 `!`. 한셀이 잃은 참조 `#REF.` 는 `#REF!`
            '.' if out.ends_with("#REF") => out.push('!'),
            '.' if cell_ref_ahead(chars.clone()) && sheet_name_before(&out) => out.push('!'),
            _ => out.push(c),
        }
    }
    out
}

/// 다음 글이 칸 참조 (`$A$1`, `B12`) 로 시작하는가 (`STDEV.S(` 같은 함수 이름은 아니다)
fn cell_ref_ahead(mut it: impl Iterator<Item = char>) -> bool {
    let mut c = it.next();
    if c == Some('$') {
        c = it.next();
    }
    let mut letters = 0;
    while c.is_some_and(|x| x.is_ascii_alphabetic()) {
        letters += 1;
        c = it.next();
    }
    if c == Some('$') {
        c = it.next();
    }
    let mut digits = 0;
    while c.is_some_and(|x| x.is_ascii_digit()) {
        digits += 1;
        c = it.next();
    }
    (1..=3).contains(&letters) && digits > 0 && !c.is_some_and(|x| x.is_alphanumeric() || x == '(')
}

/// 글 끝이 시트 이름인가: 따옴표로 닫혔거나, 끝의 낱말(글자·숫자·`_`)이 숫자로 시작하지 않는다
fn sheet_name_before(out: &str) -> bool {
    if out.ends_with('\'') {
        return true;
    }
    let word: String = out
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    word.chars()
        .last()
        .is_some_and(|first| !first.is_ascii_digit())
}

/// 이름 정의의 범위 주소 (`$Variables.$B$1`, `$시트.$A$1:.$B$2`) → `Variables!$B$1`
fn range_address(a: &str) -> String {
    let mut sheet = String::new();
    let parts: Vec<String> = a
        .split(':')
        .map(|p| match p.rfind('.') {
            Some(i) => {
                if i > 0 {
                    sheet = p[..i].trim_start_matches('$').to_string();
                }
                p[i + 1..].to_string()
            }
            None => p.to_string(),
        })
        .collect();
    let sheet = if sheet.is_empty() {
        String::new()
    } else {
        format!(
            "{}!",
            crate::legacy::formula::quote_sheet(sheet.trim_matches('\''))
        )
    };
    format!("{sheet}{}", parts.join(":"))
}

/// 셀 수식 글 → ODF (한셀 꼴: `of:=` + A1 참조 그대로, 인수 구분 `;`). 다른 시트 참조 `시트!A1:B2` 는 ODF 표준
/// `[시트.A1:.B2]` 로 (한셀은 `#REF.` 로 잃는다)
pub fn formula_to_odf(f: &str) -> String {
    let mut out = String::from("of:=");
    let chars: Vec<char> = f.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                    if chars[i - 1] == '"' {
                        break;
                    }
                }
                continue;
            }
            ',' => out.push(';'),
            '!' => {
                // 앞의 시트 이름 (따옴표 이름 또는 낱말) 을 out 에서 떼어 [이름.참조] 로
                let sheet_start = if out.ends_with('\'') {
                    out[..out.len() - 1].rfind('\'')
                } else {
                    let word_len: usize = out
                        .chars()
                        .rev()
                        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                        .map(char::len_utf8)
                        .sum();
                    (word_len > 0).then(|| out.len() - word_len)
                };
                let mut j = i + 1;
                while j < chars.len()
                    && (chars[j].is_ascii_alphanumeric() || chars[j] == '$' || chars[j] == ':')
                {
                    j += 1;
                }
                match sheet_start {
                    Some(s) if j > i + 1 => {
                        let sheet = out[s..].to_string();
                        out.truncate(s);
                        let r: String = chars[i + 1..j].iter().collect();
                        let r = r.replace(':', ":.");
                        out.push_str(&format!("[{sheet}.{r}]"));
                        i = j;
                        continue;
                    }
                    _ => out.push(c),
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// `PT12H30M15S` → 하루의 비율
fn duration_days(v: &str) -> Option<f64> {
    let v = v.strip_prefix("PT").or_else(|| v.strip_prefix("-PT"))?;
    let (mut h, mut m, mut s) = (0.0, 0.0, 0.0);
    let mut num = String::new();
    for c in v.chars() {
        match c {
            'H' => h = num.parse().ok()?,
            'M' => m = num.parse().ok()?,
            'S' => s = num.parse().ok()?,
            _ => {
                num.push(c);
                continue;
            }
        }
        num.clear();
    }
    Some((h * 3600.0 + m * 60.0 + s) / 86400.0)
}

/// `2024-03-15` (`T12:00:00` 이 붙을 수 있다) → 1900 날짜 체계 일련번호
fn date_serial(v: &str) -> Option<f64> {
    let (d, t) = v.split_once('T').unwrap_or((v, ""));
    let mut it = d.split('-');
    let (y, m, day): (i64, i64, i64) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    // 1899-12-30 기준 일 수 (1900-03-01 이후는 1900 체계 일련번호와 같다)
    let days_from_civil = |y: i64, m: i64, d: i64| -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    };
    let serial = (days_from_civil(y, m, day) - days_from_civil(1899, 12, 30)) as f64;
    let frac = if t.is_empty() {
        0.0
    } else {
        let mut p = t.split(':');
        let h: f64 = p.next()?.parse().ok()?;
        let mi: f64 = p.next().unwrap_or("0").parse().ok()?;
        let s: f64 = p.next().unwrap_or("0").parse().ok()?;
        (h * 3600.0 + mi * 60.0 + s) / 86400.0
    };
    Some(serial + frac)
}

fn attr<'a>(e: &'a Element, ns: &str, name: &str) -> Option<&'a str> {
    e.attr_ns(ns, name)
}

/// 칸 스타일의 속성 — 칸·문단·글자 속성의 지역 이름 → 값
type StyleProps = HashMap<String, String>;

/// 칸 스타일 목록: 이름 → (부모 이름, 자기 속성)
type RawStyles = HashMap<String, (Option<String>, StyleProps)>;

fn own_props(st: &Element) -> StyleProps {
    let mut p = StyleProps::new();
    if let Some(d) = attr(st, NS_STYLE, "data-style-name") {
        p.insert("data-style-name".into(), d.to_string());
    }
    for e in st.elements() {
        if e.is(NS_STYLE, "table-cell-properties")
            || e.is(NS_STYLE, "paragraph-properties")
            || e.is(NS_STYLE, "text-properties")
        {
            for a in &e.attrs {
                p.insert(a.name.clone(), a.value.clone());
            }
        }
    }
    p
}

/// `office:styles`·`office:automatic-styles` 의 칸 스타일을 모은다 (기본 스타일은 이름 없이 "")
fn collect_cell_styles(parent: &Element, out: &mut RawStyles) {
    for st in parent.elements() {
        if attr(st, NS_STYLE, "family") != Some("table-cell") {
            continue;
        }
        if st.is(NS_STYLE, "default-style") {
            out.insert(String::new(), (None, own_props(st)));
        } else if st.is(NS_STYLE, "style")
            && let Some(name) = attr(st, NS_STYLE, "name")
        {
            let parent = attr(st, NS_STYLE, "parent-style-name").map(str::to_string);
            out.insert(name.to_string(), (parent, own_props(st)));
        }
    }
}

/// 부모를 먼저 물려받은 속성 (기본 스타일 → 부모 → 자기)
fn resolved_props(raw: &RawStyles, name: &str) -> StyleProps {
    let mut chain = Vec::new();
    let mut cur = Some(name.to_string());
    while let Some(n) = cur.filter(|n| !chain.contains(n) && chain.len() < 16) {
        cur = raw.get(&n).and_then(|s| s.0.clone());
        chain.push(n);
    }
    let mut p = raw.get("").map(|s| s.1.clone()).unwrap_or_default();
    for n in chain.iter().rev() {
        if let Some((_, own)) = raw.get(n) {
            p.extend(own.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    p
}

/// `#rrggbb` → 색
fn odf_color(v: &str) -> Option<crate::model::Color> {
    crate::edit::style::rgb(v.trim())
}

/// ODF 테두리 `너비 모양 색` → 변. 한셀은 너비로 굵기를 고른다 (0.002cm 가는 선, 0.088cm 굵은 선 — 한셀 .ods 열기
/// 대조), 색이 없으면 검정
fn odf_border(v: &str) -> Option<crate::model::BorderEdge> {
    let mut width = None;
    let mut shape = None;
    let mut color = None;
    for t in v.split_whitespace() {
        if t.starts_with('#') {
            color = odf_color(t);
        } else if let Some(w) = length_pt(t) {
            width = Some(w);
        } else {
            shape = Some(t);
        }
    }
    let shape = shape.unwrap_or("solid");
    if shape == "none" || shape == "hidden" || width == Some(0.0) {
        return None;
    }
    let w = width.unwrap_or(0.75);
    let style = match shape {
        "double" => "double",
        "dashed" | "long-dash" => "dashed",
        "dotted" => "dotted",
        "dot-dash" => "dashDot",
        "dot-dot-dash" => "dashDotDot",
        // 한셀은 실선을 가는 선과 굵은 선으로만 연다 (1.75pt → 굵은 선 — 한셀 .ods 열기 대조)
        _ if w >= 1.25 => "thick",
        _ => "thin",
    };
    Some(crate::model::BorderEdge {
        style: Some(style.into()),
        color: color.or_else(|| odf_color("#000000")),
    })
}

/// 글자 속성 → 글꼴
fn apply_font(font: &mut crate::model::Font, p: &StyleProps) {
    let get = |k: &str| p.get(k).map(String::as_str);
    if let Some(n) = get("font-name").or(get("font-family")) {
        font.name = Some(n.trim_matches('\'').to_string());
    }
    if let Some(s) = get("font-size").and_then(|s| {
        let n = s.trim_end_matches("pt");
        n.parse::<f64>().ok().or_else(|| length_pt(s))
    }) {
        font.size = Some((s * 100.0).round() / 100.0);
    }
    if let Some(w) = get("font-weight") {
        font.bold = w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600);
    }
    if let Some(s) = get("font-style") {
        font.italic = s == "italic" || s == "oblique";
    }
    if let Some(u) = get("text-underline-style") {
        font.underline = (u != "none").then(|| {
            if get("text-underline-type") == Some("double") {
                "double".to_string()
            } else {
                "single".to_string()
            }
        });
    }
    if let Some(s) = get("text-line-through-style") {
        font.strike = s != "none";
    }
    if let Some(pos) = get("text-position") {
        font.vert_align = if pos.starts_with("super") {
            Some("superscript".into())
        } else if pos.starts_with("sub") {
            Some("subscript".into())
        } else {
            None
        };
    }
    if let Some(c) = get("color").and_then(odf_color) {
        font.color = Some(c);
    }
    // 한셀은 밑줄·취소선에 한컴 글꼴 확장(모양 solid / 이중 doubleslim, 색 = 글자 색)을 붙인다
    if font.underline.is_some() || font.strike {
        let h = font.hancom.get_or_insert_with(Default::default);
        if let Some(u) = &font.underline {
            h.underline_type = Some("1".into());
            h.underline_shape = Some(if u == "double" { "doubleslim" } else { "solid" }.into());
            h.underline_color = font.color.clone();
        }
        if font.strike {
            h.strikeout_type = Some("1".into());
            h.strikeout_shape = Some("solid".into());
            h.strikeout_color = font.color.clone();
        }
    }
}

/// 풀린 속성 → 셀 서식 (문서의 기본 셀 서식 위에)
fn props_xf(styles: &mut crate::model::Styles, p: &StyleProps, num_fmt: Option<&str>) -> u32 {
    use crate::model::{Alignment, Border, Fill};
    let get = |k: &str| p.get(k).map(String::as_str);
    let mut xf = styles.xf(0).cloned().unwrap_or_default();
    let mut font = styles
        .fonts
        .get(xf.font_id as usize)
        .cloned()
        .unwrap_or_default();
    apply_font(&mut font, p);
    xf.font_id = crate::edit::style::intern_font(styles, font);
    if let Some(c) = get("background-color").and_then(odf_color) {
        let fill = Fill {
            pattern_type: Some("solid".into()),
            fg_color: Some(c),
            bg_color: Some(crate::model::Color {
                indexed: Some(64),
                ..Default::default()
            }),
            gradient: None,
        };
        let key = format!("{fill:?}");
        xf.fill_id = match styles.fills.iter().position(|f| format!("{f:?}") == key) {
            Some(i) => i as u32,
            None => {
                styles.fills.push(fill);
                styles.fills.len() as u32 - 1
            }
        };
    }
    let all = get("border").and_then(odf_border);
    let side = |k: &str| match get(k) {
        Some(v) => odf_border(v),
        None => all.clone(),
    };
    let mut border = Border {
        left: side("border-left"),
        right: side("border-right"),
        top: side("border-top"),
        bottom: side("border-bottom"),
        ..Default::default()
    };
    // 대각선: 선 하나에 방향 둘 — 한셀은 왼쪽 위→오른쪽 아래 선의 모양을 먼저 쓴다
    let down = get("diagonal-tl-br").and_then(odf_border);
    let up = get("diagonal-bl-tr").and_then(odf_border);
    border.diagonal_down = down.is_some();
    border.diagonal_up = up.is_some();
    border.diagonal = down.or(up);
    if [
        &border.left,
        &border.right,
        &border.top,
        &border.bottom,
        &border.diagonal,
    ]
    .iter()
    .any(|e| e.is_some())
    {
        xf.border_id = crate::edit::style::intern_border(styles, border);
    }
    let mut al = xf.alignment.clone().unwrap_or(Alignment::default());
    if let Some(h) = get("text-align") {
        al.horizontal = match h {
            "start" | "left" => Some("left".into()),
            "center" => Some("center".into()),
            "end" | "right" => Some("right".into()),
            "justify" => Some("justify".into()),
            _ => None,
        };
    }
    if let Some(v) = get("vertical-align") {
        al.vertical = match v {
            "top" => Some("top".into()),
            "middle" => Some("center".into()),
            "bottom" => Some("bottom".into()),
            _ => al.vertical,
        };
    }
    if let Some(w) = get("wrap-option") {
        al.wrap_text = w == "wrap";
    }
    if let Some(p) = get("cell-protect") {
        let hidden = p.contains("formula-hidden") || p.contains("hidden-and-protected");
        let locked = p.contains("protected");
        if !locked || hidden {
            xf.protection = Some(crate::model::Protection {
                locked: Some(locked),
                hidden: Some(hidden),
            });
        }
    }
    if let Some(s) = get("shrink-to-fit") {
        al.shrink_to_fit = s == "true";
    }
    if let Some(r) =
        get("rotation-angle").and_then(|r| r.trim_end_matches("deg").parse::<f64>().ok())
    {
        let r = r.round() as i32 % 360;
        al.text_rotation = match r {
            0 => None,
            1..=90 => Some(r),
            270..=359 => Some(90 + 360 - r),
            _ => None,
        };
    }
    // 들여쓰기: 한셀은 단계마다 문단 왼쪽·오른쪽 여백 0.353cm (10pt) 를 쓴다
    let margin = ["margin-left", "margin-right"]
        .iter()
        .filter_map(|k| get(k).and_then(length_pt))
        .fold(0.0f64, f64::max);
    let h = al.horizontal.clone();
    if matches!(h.as_deref(), Some("left" | "right")) && margin > 0.0 {
        al.indent = Some((margin / 10.0).round() as u32).filter(|&i| i > 0);
    }
    // 문단 모양: 한셀은 가로 맞춤이 일반이 아닌 칸에 기본 문단 모양(줄 간격 75 betweenLines, 영문 keepWord·한글
    // breakWord, 기준선)을 붙이고 들여쓰기는 왼쪽(오른쪽 맞춤이면 오른쪽) 여백 = 단계 × 1134 (.xls 와 같다 — 한셀 .ods 열기 대조)
    let indent = al.indent.unwrap_or(0) as i32;
    if h.is_some() && !(h.as_deref() == Some("left") && indent == 0) {
        let left = h.as_deref() == Some("left");
        let right = h.as_deref() == Some("right");
        xf.hancom = Some(crate::model::HancomXfExt {
            apply_extension: true,
            parashape: Some(crate::model::ParaShape {
                line_spacing_type: Some("betweenLines".into()),
                line_spacing: Some(75),
                break_latin_word: Some("keepWord".into()),
                break_non_latin_word: Some("breakWord".into()),
                condense: Some(0),
                text_alignment: Some("baseLine".into()),
                left_margin: (left && indent > 0).then_some(indent * 1134),
                right_margin: (right && indent > 0).then_some(indent * 1134),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    xf.alignment = Some(al);
    // 표시 형식 (한셀은 ODF 표시 형식 스타일을 읽는다 — 한셀 .ods 열기 대조)
    if let Some(code) = num_fmt.filter(|c| !c.is_empty()) {
        xf.num_fmt_id = crate::edit::style::num_fmt_id(styles, code);
    }
    crate::edit::style::intern_xf(styles, xf)
}

/// 머리글·바닥글 영역의 글 → 머리글 코드 (`&"글꼴,Regular"&크기` 와 필드 `&A`·`&P`·`&N`·`&D`·`&T`·`&F`)
fn hf_text(e: &Element, text_styles: &HashMap<String, StyleProps>, out: &mut String) {
    for n in &e.children {
        match n {
            crate::xml::Node::Text(t) => out.push_str(t),
            crate::xml::Node::Element(c) if c.ns.as_deref() == Some(NS_TEXT) => {
                match c.name.as_str() {
                    "span" => {
                        if let Some(p) =
                            attr(c, NS_TEXT, "style-name").and_then(|s| text_styles.get(s))
                        {
                            let get = |k: &str| p.get(k).map(String::as_str);
                            if let Some(name) = get("font-name").or(get("font-family")) {
                                let bold = get("font-weight") == Some("bold");
                                let italic = get("font-style") == Some("italic");
                                let shape = match (bold, italic) {
                                    (true, true) => "Bold Italic",
                                    (true, false) => "Bold",
                                    (false, true) => "Italic",
                                    _ => "Regular",
                                };
                                out.push_str(&format!("&\"{name},{shape}\""));
                            }
                            if let Some(s) = get("font-size").and_then(length_pt) {
                                out.push_str(&format!("&{s}"));
                            }
                        }
                        hf_text(c, text_styles, out);
                    }
                    "sheet-name" => out.push_str("&A"),
                    "page-number" => out.push_str("&P"),
                    "page-count" => out.push_str("&N"),
                    "date" => out.push_str("&D"),
                    "time" => out.push_str("&T"),
                    "file-name" | "title" => out.push_str("&F"),
                    "s" => {
                        let n: usize = attr(c, NS_TEXT, "c")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1);
                        out.push_str(&" ".repeat(n.min(100)));
                    }
                    "line-break" => out.push('\n'),
                    "tab" => out.push('\t'),
                    _ => hf_text(c, text_styles, out),
                }
            }
            _ => {}
        }
    }
}

/// `style:header`·`style:footer` → 머리글 코드 (영역 왼쪽·가운데·오른쪽, 영역이 없으면 가운데)
fn hf_code(e: &Element, text_styles: &HashMap<String, StyleProps>) -> Option<String> {
    let mut out = String::new();
    let mut any = false;
    for (region, code) in [
        ("region-left", "&L"),
        ("region-center", "&C"),
        ("region-right", "&R"),
    ] {
        if let Some(r) = e.child_ns(NS_STYLE, region) {
            any = true;
            let mut t = String::new();
            for (i, p) in r.elements().filter(|p| p.is(NS_TEXT, "p")).enumerate() {
                if i > 0 {
                    t.push('\n');
                }
                hf_text(p, text_styles, &mut t);
            }
            if !t.is_empty() {
                out.push_str(code);
                out.push_str(&t);
            }
        }
    }
    if !any {
        let mut t = String::new();
        for (i, p) in e.elements().filter(|p| p.is(NS_TEXT, "p")).enumerate() {
            if i > 0 {
                t.push('\n');
            }
            hf_text(p, text_styles, &mut t);
        }
        if !t.is_empty() {
            out = format!("&C{t}");
        }
    }
    (!out.is_empty()).then_some(out)
}

/// 쪽 모양 (`style:page-layout`)과 마스터 쪽의 머리글·바닥글 → 시트 인쇄 설정. 한셀이 .ods 를 열 때처럼: 위·아래
/// 여백은 쪽 여백 + 머리글(바닥글) 높이 + 간격, 머리글·바닥글 여백은 쪽 여백, 용지는 늘 A4(9), 쪽 순서 overThenDown,
/// 600 dpi, 페이지에 맞춤 켬 (한셀 .ods 열기 대조)
fn page_layout(
    layout: Option<&Element>,
    master: Option<&Element>,
    text_styles: &HashMap<String, StyleProps>,
) -> crate::model::PageLayout {
    use crate::model::{HeaderFooter, PageLayout, PageMargins, PageSetup, PrintOptions};
    let props = layout.and_then(|l| l.child_ns(NS_STYLE, "page-layout-properties"));
    let hf_props = |which: &str| {
        layout
            .and_then(|l| l.child_ns(NS_STYLE, which))
            .and_then(|h| h.child_ns(NS_STYLE, "header-footer-properties"))
    };
    let len = |e: Option<&Element>, ns: &str, k: &str| {
        e.and_then(|e| e.attr_ns(ns, k))
            .and_then(length_pt)
            .map(|pt| pt / 72.0)
    };
    const NS_FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
    let (head, foot) = (hf_props("header-style"), hf_props("footer-style"));
    let top = len(props, NS_FO, "margin-top").unwrap_or(0.0);
    let bottom = len(props, NS_FO, "margin-bottom").unwrap_or(0.0);
    let head_h = len(head, NS_FO, "min-height").unwrap_or(0.0)
        + len(head, NS_FO, "margin-bottom").unwrap_or(0.0);
    let foot_h = len(foot, NS_FO, "min-height").unwrap_or(0.0)
        + len(foot, NS_FO, "margin-top").unwrap_or(0.0);
    let pa = |k: &str| props.and_then(|p| p.attr_ns(NS_STYLE, k));
    let num = |k: &str| pa(k).and_then(|v| v.trim_end_matches('%').parse::<u32>().ok());
    let centering = pa("table-centering").unwrap_or("none");
    let print = pa("print").unwrap_or("");
    let mut hf = HeaderFooter {
        scale_with_doc: true,
        align_with_margins: true,
        ..Default::default()
    };
    if let Some(m) = master {
        hf.odd_header = m
            .child_ns(NS_STYLE, "header")
            .and_then(|h| hf_code(h, text_styles));
        hf.odd_footer = m
            .child_ns(NS_STYLE, "footer")
            .and_then(|h| hf_code(h, text_styles));
    }
    PageLayout {
        auto_page_breaks: Some(true),
        fit_to_page: Some(true),
        print_options: Some(PrintOptions {
            horizontal_centered: matches!(centering, "horizontal" | "both"),
            vertical_centered: matches!(centering, "vertical" | "both"),
            headings: print.split_whitespace().any(|t| t == "headers"),
            grid_lines: print.split_whitespace().any(|t| t == "grid"),
        }),
        margins: Some(PageMargins {
            left: len(props, NS_FO, "margin-left").unwrap_or(0.75),
            right: len(props, NS_FO, "margin-right").unwrap_or(0.75),
            top: top + head_h,
            bottom: bottom + foot_h,
            header: top,
            footer: bottom,
        }),
        setup: Some(PageSetup {
            paper_size: Some(paper_code(props)),
            scale: Some(num("scale-to").unwrap_or(100)),
            first_page_number: Some(num("first-page-number").unwrap_or(1)),
            fit_to_width: Some(num("scale-to-X").unwrap_or(0)),
            fit_to_height: Some(num("scale-to-Y").unwrap_or(0)),
            orientation: Some(pa("print-orientation").unwrap_or("portrait").to_string()),
            page_order: Some(
                if pa("print-page-order") == Some("ttb") {
                    "downThenOver"
                } else {
                    "overThenDown"
                }
                .into(),
            ),
            use_printer_defaults: true,
            horizontal_dpi: Some(600),
            vertical_dpi: Some(600),
            copies: Some(1),
            ..Default::default()
        }),
        header_footer: Some(hf),
        ..Default::default()
    }
}

/// 용지 크기 (cm) → 용지 번호. 한셀은 쪽의 (높이, 너비) 를 세운 용지의 (너비, 높이) 와 맞대 보고 (그래서 세로
/// 레터는 A4 로, 가로 레터는 레터로 연다), 맞는 것이 없으면 A4(9) — 한셀 .ods 열기 대조
fn paper_code(props: Option<&Element>) -> u32 {
    const NS_FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
    let cm = |k: &str| {
        props
            .and_then(|p| p.attr_ns(NS_FO, k))
            .and_then(length_pt)
            .map(|pt| pt / 72.0 * 2.54)
    };
    let (Some(w), Some(h)) = (cm("page-height"), cm("page-width")) else {
        return 9;
    };
    PAPERS
        .iter()
        .find(|p| (p.1 - w).abs() < 0.05 && (p.2 - h).abs() < 0.05)
        .map_or(9, |p| p.0)
}

/// 용지 번호와 세운 크기 (cm)
const PAPERS: &[(u32, f64, f64)] = &[
    (1, 21.59, 27.94),
    (5, 21.59, 35.56),
    (7, 18.415, 26.67),
    (8, 29.7, 42.0),
    (9, 21.0, 29.7),
    (11, 14.8, 21.0),
    (12, 25.7, 36.4),
    (13, 18.2, 25.7),
];

/// 한셀의 .ods 열 너비: 화면 픽셀(96 dpi) = (글자 수 + 0.772342) × 숫자 폭 + 5. 숫자 폭은 기본 글꼴 크기로 (11pt 7px,
/// 10pt 6px — 한셀이 저장한 .ods 의 cm 와 다시 연 글자 수를 맞춘 식, 오차 1/256 글자 안팎)
fn digit_px(font_pt: f64) -> f64 {
    (((font_pt * 96.0 / 72.0 * 0.55).round()) - 1.0).max(4.0)
}

fn width_chars(cm: f64, font_pt: f64) -> f64 {
    let px = cm / 2.54 * 96.0;
    let w = (px - 5.0) / digit_px(font_pt) - 0.772342;
    (w.max(0.0) * 256.0).trunc() / 256.0
}

fn width_cm(chars: f64, font_pt: f64) -> f64 {
    (((chars + 0.772342) * digit_px(font_pt) + 5.0) / 96.0 * 2.54).max(0.0)
}

const NS_NUMBER: &str = "urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0";

/// 표시 형식 안의 글 → 따옴표로 묶은 글 (한셀처럼 `"-"`)
fn quoted(t: &str) -> String {
    if t.is_empty() {
        String::new()
    } else {
        format!("\"{}\"", t.replace('"', ""))
    }
}

/// `number:number` → `#,##0.00` 꼴
fn number_part(n: &Element) -> String {
    let get = |k: &str| attr(n, NS_NUMBER, k);
    let dec: usize = get("decimal-places")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let min_dec: usize = get("min-decimal-places")
        .and_then(|v| v.parse().ok())
        .unwrap_or(dec)
        .min(dec);
    let min_int: usize = get("min-integer-digits")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let grouping = get("grouping") == Some("true");
    let mut s = if grouping {
        let zeros = min_int.clamp(1, 4);
        let digits: String = "#".repeat(4 - zeros) + &"0".repeat(zeros);
        format!("{},{}", &digits[..1], &digits[1..])
    } else if min_int == 0 {
        "#".to_string()
    } else {
        "0".repeat(min_int)
    };
    if dec > 0 {
        s.push('.');
        s.push_str(&"0".repeat(min_dec));
        s.push_str(&"#".repeat(dec - min_dec));
    }
    s
}

/// ODF 표시 형식 스타일 하나 → 서식 문자열 (한 구역)
fn number_style_section(st: &Element) -> String {
    // 자리 수를 정하지 않은 number:number 하나뿐이면 일반 (LibreOffice·rcell 의 "General")
    let kids: Vec<&Element> = st.elements().collect();
    if st.is(NS_NUMBER, "number-style")
        && kids.len() == 1
        && kids[0].is(NS_NUMBER, "number")
        && attr(kids[0], NS_NUMBER, "decimal-places").is_none()
        && attr(kids[0], NS_NUMBER, "grouping").is_none()
    {
        return "General".into();
    }
    let mut out = String::new();
    for c in st.elements() {
        if c.ns.as_deref() != Some(NS_NUMBER) {
            if c.is(NS_STYLE, "text-properties")
                && let Some(col) = c.attr_ns(
                    "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0",
                    "color",
                )
            {
                let name = match col.to_ascii_lowercase().as_str() {
                    "#ff0000" => "Red",
                    "#0000ff" => "Blue",
                    "#00ff00" => "Green",
                    "#ffff00" => "Yellow",
                    "#ff00ff" => "Magenta",
                    "#00ffff" => "Cyan",
                    "#ffffff" => "White",
                    _ => "Black",
                };
                out.insert_str(0, &format!("[{name}]"));
            }
            continue;
        }
        let long = attr(c, NS_NUMBER, "style") == Some("long");
        let textual = attr(c, NS_NUMBER, "textual") == Some("true");
        match c.name.as_str() {
            "number" => out.push_str(&number_part(c)),
            "scientific-number" => {
                out.push_str(&number_part(c));
                let e: usize = attr(c, NS_NUMBER, "min-exponent-digits")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2);
                out.push_str("E+");
                out.push_str(&"0".repeat(e.max(1)));
            }
            "fraction" => {
                let w = |k: &str| {
                    attr(c, NS_NUMBER, k)
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                };
                out.push_str(&format!(
                    "# {}/{}",
                    "?".repeat(w("min-numerator-digits")),
                    "?".repeat(w("min-denominator-digits"))
                ));
            }
            "text" => {
                let t = c.text();
                if t == "%" {
                    out.push('%');
                } else {
                    out.push_str(&quoted(&t));
                }
            }
            "currency-symbol" => {
                let sym = c.text();
                let lcid = match (
                    attr(c, NS_NUMBER, "language"),
                    attr(c, NS_NUMBER, "country"),
                ) {
                    (Some("ko"), _) => "412",
                    (Some("ja"), _) => "411",
                    (Some("zh"), _) => "804",
                    (Some("en"), Some("GB")) => "809",
                    (Some("de"), _) => "407",
                    (Some("fr"), _) => "40C",
                    _ => "409",
                };
                out.push_str(&format!("[${sym}-{lcid}]"));
            }
            "year" => out.push_str(if long { "yyyy" } else { "yy" }),
            "month" => out.push_str(match (textual, long) {
                (true, true) => "mmmm",
                (true, false) => "mmm",
                (false, true) => "mm",
                (false, false) => "m",
            }),
            "day" => out.push_str(if long { "dd" } else { "d" }),
            "day-of-week" => out.push_str(if long { "dddd" } else { "ddd" }),
            "hours" => out.push_str(if long { "hh" } else { "h" }),
            "minutes" => out.push_str(if long { "mm" } else { "m" }),
            "seconds" => {
                out.push_str(if long { "ss" } else { "s" });
                let d: usize = attr(c, NS_NUMBER, "decimal-places")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                if d > 0 {
                    out.push('.');
                    out.push_str(&"0".repeat(d));
                }
            }
            "am-pm" => out.push_str("AM/PM"),
            "text-content" => out.push('@'),
            "boolean" => out.push_str("\"TRUE\";\"TRUE\";\"FALSE\""),
            _ => {}
        }
    }
    // 시간 길이: 넘침을 자르지 않으면 [h]
    if st.is(NS_NUMBER, "time-style")
        && attr(st, NS_NUMBER, "truncate-on-overflow") == Some("false")
    {
        for (a, b) in [("hh", "[hh]"), ("h", "[h]")] {
            if let Some(i) = out.find(a) {
                out.replace_range(i..i + a.len(), b);
                break;
            }
        }
    }
    out
}

/// ODF 표시 형식 스타일들 (이름 → 요소)에서 이름 하나의 서식 문자열 (조건 `style:map` 은 구역으로)
fn number_format(styles: &HashMap<String, &Element>, name: &str) -> Option<String> {
    let st = styles.get(name)?;
    let main = number_style_section(st);
    // 조건이 맞을 때 쓸 구역들 (앞 구역) — 기본 구역(main)은 조건이 모두 틀릴 때 쓴다
    let mut before: Vec<String> = Vec::new();
    let mut after: Vec<String> = Vec::new();
    for m in st.elements().filter(|e| e.is(NS_STYLE, "map")) {
        let (Some(cond), Some(apply)) = (
            attr(m, NS_STYLE, "condition"),
            attr(m, NS_STYLE, "apply-style-name"),
        ) else {
            continue;
        };
        let Some(other) = styles.get(apply) else {
            continue;
        };
        let sec = number_style_section(other);
        let c = cond.trim_start_matches("value()").trim();
        // 양수 구역(>=0)은 기본 구역 앞에, 음수 구역(<0)은 뒤에 조건 없이, 그 밖은 [조건]
        if c == ">=0" || c == ">0" {
            before.push(sec);
        } else if c == "<0" {
            after.push(sec);
        } else {
            before.push(format!("[{c}]{sec}"));
        }
    }
    if before.is_empty() && after.is_empty() {
        return Some(main).filter(|s| !s.is_empty());
    }
    before.push(main);
    before.extend(after);
    Some(before.join(";"))
}

/// 서식 문자열을 구역(`;`)으로 — 따옴표·대괄호 안의 `;` 는 넘긴다
fn format_sections(code: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut quote, mut bracket, mut esc_next) = (false, false, false);
    for c in code.chars() {
        let cur = out.last_mut().expect("구역");
        if esc_next {
            cur.push(c);
            esc_next = false;
            continue;
        }
        match c {
            '\\' if !quote => {
                esc_next = true;
                cur.push(c);
            }
            '"' => {
                quote = !quote;
                cur.push(c);
            }
            '[' if !quote => {
                bracket = true;
                cur.push(c);
            }
            ']' if !quote => {
                bracket = false;
                cur.push(c);
            }
            ';' if !quote && !bracket => out.push(String::new()),
            _ => cur.push(c),
        }
    }
    out
}

/// 서식 문자열 한 구역 → ODF 표시 형식 스타일 (`number:*-style`). 숫자·백분율·지수·통화·날짜·시간·글을 다룬다
fn number_style_xml(name: &str, code: &str, extra: &str) -> Option<String> {
    let chars: Vec<char> = code.chars().collect();
    let mut body = String::new();
    let mut color = None;
    let mut kind = "number";
    let mut i = 0;
    let mut text = String::new();
    let flush = |text: &mut String, body: &mut String| {
        if !text.is_empty() {
            body.push_str(&format!("<number:text>{}</number:text>", esc(text)));
            text.clear();
        }
    };
    let mut last_date = ' ';
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|&x| x == '"')
                    .map_or(chars.len(), |p| i + 1 + p);
                text.extend(&chars[i + 1..end.min(chars.len())]);
                i = end + 1;
            }
            '\\' => {
                if let Some(&n) = chars.get(i + 1) {
                    text.push(n);
                }
                i += 2;
            }
            '[' => {
                let end = chars[i..]
                    .iter()
                    .position(|&x| x == ']')
                    .map_or(chars.len(), |p| i + p);
                let inner: String = chars[i + 1..end].iter().collect();
                if let Some(cur) = inner.strip_prefix('$') {
                    let (sym, lcid) = cur.split_once('-').unwrap_or((cur, ""));
                    let (lang, country) = match lcid.to_ascii_uppercase().as_str() {
                        "412" => ("ko", "KR"),
                        "411" => ("ja", "JP"),
                        "804" => ("zh", "CN"),
                        "809" => ("en", "GB"),
                        "407" => ("de", "DE"),
                        "40C" => ("fr", "FR"),
                        _ => ("en", "US"),
                    };
                    flush(&mut text, &mut body);
                    body.push_str(&format!(
                        r#"<number:currency-symbol number:language="{lang}" number:country="{country}">{}</number:currency-symbol>"#,
                        esc(sym)
                    ));
                    kind = "currency";
                } else if matches!(
                    inner.to_ascii_lowercase().as_str(),
                    "h" | "hh" | "m" | "mm" | "s" | "ss"
                ) {
                    flush(&mut text, &mut body);
                    let long = inner.len() > 1;
                    let tag = match inner.to_ascii_lowercase().chars().next() {
                        Some('h') => "hours",
                        Some('m') => "minutes",
                        _ => "seconds",
                    };
                    body.push_str(&format!(
                        r#"<number:{tag}{}/>"#,
                        if long { r#" number:style="long""# } else { "" }
                    ));
                    kind = "time-elapsed";
                } else if !inner.is_empty() && inner.chars().all(|x| x.is_ascii_alphabetic()) {
                    color = Some(match inner.to_ascii_lowercase().as_str() {
                        "red" => "#ff0000",
                        "blue" => "#0000ff",
                        "green" => "#00ff00",
                        "yellow" => "#ffff00",
                        "magenta" => "#ff00ff",
                        "cyan" => "#00ffff",
                        "white" => "#ffffff",
                        _ => "#000000",
                    });
                }
                i = end + 1;
            }
            '#' | '0' | '?' | ',' | '.' if kind != "date" && kind != "time" => {
                let start = i;
                while i < chars.len() && matches!(chars[i], '#' | '0' | '?' | ',' | '.') {
                    i += 1;
                }
                let pat: String = chars[start..i].iter().collect();
                let (int, dec) = pat.split_once('.').unwrap_or((&pat, ""));
                let digits = |s: &str| s.chars().filter(|c| *c == '0').count();
                let grouping = int.contains(',');
                let mut attrs = format!(
                    r#" number:decimal-places="{}" number:min-decimal-places="{}" number:min-integer-digits="{}""#,
                    dec.chars().filter(|c| matches!(c, '0' | '#' | '?')).count(),
                    digits(dec),
                    digits(int)
                );
                if grouping {
                    attrs.push_str(r#" number:grouping="true""#);
                }
                flush(&mut text, &mut body);
                // 지수
                if i + 1 < chars.len()
                    && matches!(chars[i], 'E' | 'e')
                    && matches!(chars[i + 1], '+' | '-')
                {
                    let s = i + 2;
                    let mut j = s;
                    while j < chars.len() && chars[j] == '0' {
                        j += 1;
                    }
                    body.push_str(&format!(
                        r#"<number:scientific-number{attrs} number:min-exponent-digits="{}"/>"#,
                        j - s
                    ));
                    i = j;
                } else {
                    body.push_str(&format!("<number:number{attrs}/>"));
                }
            }
            '%' => {
                flush(&mut text, &mut body);
                body.push_str("<number:text>%</number:text>");
                if kind == "number" {
                    kind = "percentage";
                }
                i += 1;
            }
            '@' => {
                flush(&mut text, &mut body);
                body.push_str("<number:text-content/>");
                kind = "text";
                i += 1;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'a' | 'A' => {
                // AM/PM
                let rest: String = chars[i..].iter().take(5).collect();
                if rest.eq_ignore_ascii_case("AM/PM") {
                    flush(&mut text, &mut body);
                    body.push_str("<number:am-pm/>");
                    i += 5;
                    continue;
                }
                if matches!(c, 'a' | 'A') {
                    text.push(c);
                    i += 1;
                    continue;
                }
                let lc = c.to_ascii_lowercase();
                let mut j = i;
                while j < chars.len() && chars[j].to_ascii_lowercase() == lc {
                    j += 1;
                }
                let n = j - i;
                flush(&mut text, &mut body);
                let long = if n >= 2 {
                    r#" number:style="long""#
                } else {
                    ""
                };
                let el = match lc {
                    'y' => format!(
                        "<number:year{}/>",
                        if n >= 3 {
                            r#" number:style="long""#
                        } else {
                            ""
                        }
                    ),
                    'd' if n >= 4 => r#"<number:day-of-week number:style="long"/>"#.into(),
                    'd' if n == 3 => "<number:day-of-week/>".into(),
                    'd' => format!("<number:day{long}/>"),
                    'h' => format!("<number:hours{long}/>"),
                    's' => format!("<number:seconds{long}/>"),
                    // m: 시 뒤나 초 앞이면 분
                    _ => {
                        let next_s = chars[j..]
                            .iter()
                            .find(|x| x.is_ascii_alphabetic())
                            .is_some_and(|x| x.eq_ignore_ascii_case(&'s'));
                        if (last_date == 'h' || next_s) && n <= 2 {
                            format!("<number:minutes{long}/>")
                        } else if n >= 4 {
                            r#"<number:month number:style="long" number:textual="true"/>"#.into()
                        } else if n == 3 {
                            r#"<number:month number:textual="true"/>"#.into()
                        } else {
                            format!("<number:month{long}/>")
                        }
                    }
                };
                body.push_str(&el);
                last_date = if el.contains("minutes") { 'n' } else { lc };
                if kind != "time-elapsed" {
                    kind = if matches!(lc, 'y' | 'd') || el.contains("month") || kind == "date" {
                        "date"
                    } else {
                        "time"
                    };
                }
                i = j;
            }
            '_' | '*' => i += 2,
            _ => {
                text.push(c);
                i += 1;
            }
        }
    }
    flush(&mut text, &mut body);
    if body.is_empty() {
        return None;
    }
    let props = color
        .map(|c| format!(r#"<style:text-properties fo:color="{c}"/>"#))
        .unwrap_or_default();
    let (tag, attrs) = match kind {
        "percentage" => ("number:percentage-style", ""),
        "currency" => ("number:currency-style", ""),
        "date" => ("number:date-style", ""),
        "time" => ("number:time-style", ""),
        "time-elapsed" => (
            "number:time-style",
            r#" number:truncate-on-overflow="false""#,
        ),
        "text" => ("number:text-style", ""),
        _ => ("number:number-style", ""),
    };
    Some(format!(
        r#"<{tag} style:name="{name}"{attrs}>{props}{body}{extra}</{tag}>"#
    ))
}

/// 서식 문자열 → ODF 표시 형식 스타일들 (구역이 여럿이면 `style:map` 으로). 일반 서식은 None
fn number_styles_xml(name: &str, code: &str) -> Option<String> {
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return None;
    }
    let secs = format_sections(code);
    if secs.len() == 1 {
        return number_style_xml(name, &secs[0], "");
    }
    // 첫 구역은 양수(>=0 또는 [조건]), 둘째는 음수 — 마지막 구역을 바탕으로 두고 앞 구역을 조건으로 건다
    let mut out = String::new();
    let mut maps = String::new();
    let conds = [">=0", "<0"];
    let (main, rest) = if secs.len() >= 2 {
        (secs[1].clone(), vec![secs[0].clone()])
    } else {
        (secs[0].clone(), vec![])
    };
    for (k, s) in rest.iter().enumerate() {
        let sub = format!("{name}P{k}");
        out.push_str(&number_style_xml(&sub, s, "")?);
        maps.push_str(&format!(
            r#"<style:map style:condition="value(){}" style:apply-style-name="{sub}"/>"#,
            conds[k]
        ));
    }
    // 음수 구역은 부호를 스스로 쓰지 않으면 ODF 에서 '-' 가 빠지지 않는다 — 그대로 둔다
    out.push_str(&number_style_xml(name, &main, &maps)?);
    Some(out)
}

/// 이름 붙은 칸 스타일 → 셀 서식 번호 (풀어 둔 것을 기억한다)
struct CellStyles {
    raw: RawStyles,
    done: HashMap<String, u32>,
    /// 표시 형식 스타일 이름 → 서식 문자열
    formats: HashMap<String, String>,
}

impl CellStyles {
    fn xf(&mut self, styles: &mut crate::model::Styles, name: &str) -> u32 {
        if let Some(&x) = self.done.get(name) {
            return x;
        }
        let p = resolved_props(&self.raw, name);
        let code = p
            .get("data-style-name")
            .and_then(|d| self.formats.get(d))
            .map(String::as_str);
        let x = props_xf(styles, &p, code);
        self.done.insert(name.to_string(), x);
        x
    }
}

fn paragraphs(cell: &Element) -> String {
    let mut lines = Vec::new();
    for p in cell.elements().filter(|e| e.is(NS_TEXT, "p")) {
        lines.push(p.text());
    }
    lines.join("\n")
}

/// `.ods` 바이트 → 통합 문서
pub fn read(bytes: &[u8]) -> Result<Workbook> {
    let content = entry(bytes, "content.xml")?;
    let root = crate::xml::parse_raw(&content, "content.xml")?;
    // 자동 스타일: 열 너비, 행 높이
    let mut col_width: HashMap<String, f64> = Default::default();
    let mut row_height: HashMap<String, (f64, bool)> = Default::default();
    // 칸 스타일: styles.xml 의 이름·기본 스타일과 content.xml 의 자동 스타일
    let mut cell_styles = CellStyles {
        raw: RawStyles::new(),
        done: HashMap::new(),
        formats: HashMap::new(),
    };
    let sroot = entry(bytes, "styles.xml")
        .ok()
        .and_then(|s| crate::xml::parse_raw(&s, "styles.xml").ok());
    // 글자 스타일 (머리글·바닥글의 글꼴), 쪽 모양, 마스터 쪽
    let mut text_styles: HashMap<String, StyleProps> = HashMap::new();
    let mut layouts: HashMap<&str, &Element> = HashMap::new();
    let mut masters: HashMap<&str, &Element> = HashMap::new();
    if let Some(sroot) = &sroot {
        for part in ["styles", "automatic-styles"] {
            if let Some(p) = sroot.child_ns(NS_OFFICE, part) {
                collect_cell_styles(p, &mut cell_styles.raw);
                for st in p.elements() {
                    if st.is(NS_STYLE, "page-layout")
                        && let Some(n) = attr(st, NS_STYLE, "name")
                    {
                        layouts.insert(n, st);
                    } else if st.is(NS_STYLE, "style")
                        && attr(st, NS_STYLE, "family") == Some("text")
                        && let Some(n) = attr(st, NS_STYLE, "name")
                    {
                        text_styles.insert(n.to_string(), own_props(st));
                    }
                }
            }
        }
        if let Some(m) = sroot.child_ns(NS_OFFICE, "master-styles") {
            for mp in m.elements().filter(|e| e.is(NS_STYLE, "master-page")) {
                if let Some(n) = attr(mp, NS_STYLE, "name") {
                    masters.insert(n, mp);
                }
            }
        }
    }
    // 글꼴 선언: style:font-name → svg:font-family
    let mut faces: HashMap<String, String> = HashMap::new();
    for decls in [sroot.as_ref(), Some(&root)]
        .into_iter()
        .flatten()
        .filter_map(|r| r.child_ns(NS_OFFICE, "font-face-decls"))
    {
        for ff in decls.elements().filter(|e| e.is(NS_STYLE, "font-face")) {
            if let (Some(n), Some(fam)) = (
                attr(ff, NS_STYLE, "name"),
                ff.attrs
                    .iter()
                    .find(|a| a.name == "font-family")
                    .map(|a| a.value.trim_matches('\'').to_string()),
            ) {
                // 대소문자만 다른 글꼴은 먼저 선언된 이름으로 (한셀: ARIAL → Arial)
                let fam = faces
                    .values()
                    .find(|v| v.eq_ignore_ascii_case(&fam))
                    .cloned()
                    .unwrap_or(fam);
                faces.insert(n.to_string(), fam);
            }
        }
    }
    // 설정: 시트 이름 → 눈금선 (ShowGrid)
    let settings_xml = entry(bytes, "settings.xml").ok();
    let settings_root = settings_xml
        .as_deref()
        .and_then(|s| crate::xml::parse_raw(s, "settings.xml").ok());
    let settings: Option<HashMap<String, bool>> = settings_root.as_ref().map(|r| {
        let mut m = HashMap::new();
        r.walk(&mut |e| {
            if e.name == "config-item-map-entry"
                && let Some(n) = e.attrs.iter().find(|a| a.name == "name")
            {
                for c in e.elements() {
                    if c.name == "config-item"
                        && c.attrs
                            .iter()
                            .any(|a| a.name == "name" && a.value == "ShowGrid")
                    {
                        m.insert(n.value.clone(), c.text().trim() == "true");
                    }
                }
            }
        });
        m
    });
    // 표 스타일: 이름 → (마스터 쪽, 보임, 탭 색)
    let mut table_styles: HashMap<&str, (Option<&str>, bool, Option<&str>)> = HashMap::new();
    if let Some(auto) = root.child_ns(NS_OFFICE, "automatic-styles") {
        collect_cell_styles(auto, &mut cell_styles.raw);
        for st in auto.elements().filter(|e| e.is(NS_STYLE, "style")) {
            let Some(name) = attr(st, NS_STYLE, "name") else {
                continue;
            };
            if attr(st, NS_STYLE, "family") == Some("table") {
                let p = st.child_ns(NS_STYLE, "table-properties");
                let shown = p.and_then(|p| attr(p, NS_TABLE, "display")) != Some("false");
                let tab = p.and_then(|p| {
                    p.attrs
                        .iter()
                        .find(|a| a.name == "tab-color")
                        .map(|a| a.value.as_str())
                });
                table_styles.insert(name, (attr(st, NS_STYLE, "master-page-name"), shown, tab));
            }
            if let Some(p) = st.child_ns(NS_STYLE, "table-column-properties")
                && let Some(w) = attr(p, NS_STYLE, "column-width").and_then(length_pt)
            {
                col_width.insert(name.to_string(), w / 72.0 * 2.54);
            }
            if let Some(p) = st.child_ns(NS_STYLE, "table-row-properties")
                && let Some(h) = attr(p, NS_STYLE, "row-height").and_then(length_pt)
            {
                let optimal = attr(p, NS_STYLE, "use-optimal-row-height") == Some("true");
                row_height.insert(name.to_string(), (h, !optimal));
            }
        }
    }
    let spreadsheet = root
        .child_ns(NS_OFFICE, "body")
        .and_then(|b| b.child_ns(NS_OFFICE, "spreadsheet"))
        .ok_or_else(|| Error::MissingPart("office:spreadsheet".into()))?;
    {
        let mut data: HashMap<String, &Element> = HashMap::new();
        for part in [sroot.as_ref(), Some(&root)].into_iter().flatten() {
            for k in ["styles", "automatic-styles"] {
                if let Some(p) = part.child_ns(NS_OFFICE, k) {
                    for e in p.elements().filter(|e| {
                        e.ns.as_deref() == Some(NS_NUMBER) && e.name.ends_with("-style")
                    }) {
                        if let Some(n) = attr(e, NS_STYLE, "name") {
                            data.insert(n.to_string(), e);
                        }
                    }
                }
            }
        }
        for n in data.keys() {
            if let Some(code) = number_format(&data, n) {
                cell_styles.formats.insert(n.clone(), code);
            }
        }
    }
    for p in cell_styles
        .raw
        .values_mut()
        .map(|s| &mut s.1)
        .chain(text_styles.values_mut())
    {
        if let Some(fam) = p.get("font-name").and_then(|n| faces.get(n)) {
            p.insert("font-name".into(), fam.clone());
        }
    }
    let mut wb = crate::edit::new_workbook();
    wb.application = Some("ODF".into());
    // 기본 글꼴: "Default" 칸 스타일의 글자 속성 (한셀이 문서의 기본 글꼴로 삼는다)
    let base = resolved_props(&cell_styles.raw, "Default");
    if let Some(f0) = wb.styles.fonts.first_mut() {
        apply_font(f0, &base);
    }
    let font_pt = wb.styles.fonts.first().and_then(|f| f.size).unwrap_or(11.0);
    // 한셀이 .ods 를 열면 통합 문서 창은 늘 이 자리·크기다
    wb.view = Some(crate::model::WorkbookView {
        x_window: 480,
        y_window: 120,
        window_width: 16155,
        window_height: 9405,
        tab_ratio: 600,
        ..Default::default()
    });
    let template = wb.sheets[0].clone();
    wb.sheets.clear();
    for (ti, table) in spreadsheet
        .elements()
        .filter(|e| e.is(NS_TABLE, "table"))
        .enumerate()
    {
        let mut sheet = template.clone();
        sheet.name = attr(table, NS_TABLE, "name")
            .map(str::to_string)
            .unwrap_or_else(|| format!("Sheet{}", ti + 1));
        sheet.sheet_id = ti as u32 + 1;
        sheet.view.tab_selected = ti == 0;
        let ts = attr(table, NS_TABLE, "style-name").and_then(|s| table_styles.get(s));
        // 표 스타일이 없으면 한셀은 인쇄 설정을 기본값으로 둔다 (여백은 OOXML 기본값)
        if ts.is_none() {
            sheet.page = crate::model::PageLayout {
                setup: Some(crate::model::PageSetup {
                    paper_size: Some(9),
                    scale: Some(100),
                    first_page_number: Some(1),
                    fit_to_width: Some(0),
                    fit_to_height: Some(0),
                    orientation: Some("portrait".into()),
                    page_order: Some("downThenOver".into()),
                    use_printer_defaults: true,
                    horizontal_dpi: Some(0),
                    vertical_dpi: Some(0),
                    copies: Some(1),
                    ..Default::default()
                }),
                ..Default::default()
            };
        }
        // 눈금선: settings.xml 의 ShowGrid (없으면 켬), settings.xml 이 없으면 한셀은 끈다
        sheet.show_grid_lines = match &settings {
            None => false,
            Some(m) => m.get(sheet.name.as_str()).copied().unwrap_or(true),
        };
        if let Some((master, shown, tab)) = ts {
            if !shown {
                sheet.state = crate::model::SheetState::Hidden;
            }
            sheet.tab_color = tab.and_then(odf_color);
            let m = master.and_then(|m| masters.get(m)).copied();
            let l = m
                .and_then(|m| attr(m, NS_STYLE, "page-layout-name"))
                .and_then(|l| layouts.get(l))
                .copied();
            sheet.page = page_layout(l, m, &text_styles);
        }
        let mut col = 0u32;
        let mut row = 0u32;
        let mut rows_parent: Vec<&Element> = Vec::new();
        // 열의 기본 칸 스타일: (첫 열, 끝 열 다음, 이름)
        let mut col_default: Vec<(u32, u32, &str)> = Vec::new();
        for e in table.elements() {
            if e.is(NS_TABLE, "table-column")
                || e.is(NS_TABLE, "table-columns")
                || e.is(NS_TABLE, "table-column-group")
            {
                let cols: Vec<&Element> = if e.is(NS_TABLE, "table-column") {
                    vec![e]
                } else {
                    e.elements()
                        .filter(|c| c.is(NS_TABLE, "table-column"))
                        .collect()
                };
                for c in cols {
                    let n: u32 = attr(c, NS_TABLE, "number-columns-repeated")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1);
                    let width = attr(c, NS_TABLE, "style-name")
                        .and_then(|s| col_width.get(s))
                        .copied();
                    let hidden = attr(c, NS_TABLE, "visibility") == Some("collapse");
                    if let Some(s) = attr(c, NS_TABLE, "default-cell-style-name") {
                        col_default.push((col, col.saturating_add(n), s));
                    }
                    // 한셀은 .ods 의 열을 모두 너비를 정한 열로 연다
                    if (hidden || width.is_some()) && col < 16384 {
                        let max = (col + n).min(16384);
                        sheet.cols.push(ColInfo {
                            min: col + 1,
                            max,
                            width: width.map(|cm| width_chars(cm, font_pt)),
                            hidden,
                            custom_width: width.is_some(),
                            ..Default::default()
                        });
                    }
                    col = col.saturating_add(n);
                }
            } else if e.is(NS_TABLE, "table-row") {
                rows_parent.push(e);
            } else if e.is(NS_TABLE, "table-rows")
                || e.is(NS_TABLE, "table-header-rows")
                || e.is(NS_TABLE, "table-row-group")
            {
                rows_parent.extend(e.elements().filter(|r| r.is(NS_TABLE, "table-row")));
            }
        }
        for r in rows_parent {
            let n: u32 = attr(r, NS_TABLE, "number-rows-repeated")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let height = attr(r, NS_TABLE, "style-name")
                .and_then(|s| row_height.get(s))
                .copied();
            let hidden = attr(r, NS_TABLE, "visibility") == Some("collapse");
            let row_style = attr(r, NS_TABLE, "default-cell-style-name");
            let has_cells = r.elements().any(|c| {
                c.is(NS_TABLE, "table-cell")
                    && (attr(c, NS_OFFICE, "value-type").is_some()
                        || attr(c, NS_TABLE, "formula").is_some())
            });
            // 끝의 빈 행 반복 (1048576 행까지 채우는 것)은 건너뛴다
            if !has_cells && n > 1000 {
                row = row.saturating_add(n);
                continue;
            }
            for k in 0..n {
                let ri = row + k;
                if ri >= 1_048_576 {
                    break;
                }
                if let Some((h, custom)) = height.filter(|h| h.1) {
                    sheet.rows.push(RowInfo {
                        index: ri + 1,
                        // 한셀은 행 높이를 pt 정수로 버리고, 높이 0 인 행은 숨긴다 (16.75pt → 16 — 한셀 .ods 열기 대조)
                        height: Some(h.trunc()),
                        custom_height: custom,
                        hidden: hidden || h.trunc() == 0.0,
                        ..Default::default()
                    });
                } else if hidden {
                    sheet.rows.push(RowInfo {
                        index: ri + 1,
                        hidden,
                        ..Default::default()
                    });
                }
                let mut ci = 0u32;
                for c in r.elements() {
                    let covered = c.is(NS_TABLE, "covered-table-cell");
                    if !(c.is(NS_TABLE, "table-cell") || covered) {
                        continue;
                    }
                    let rep: u32 = attr(c, NS_TABLE, "number-columns-repeated")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1);
                    // 병합에 덮인 칸은 값을 버리고 스타일만 (한셀도 덮인 칸의 테두리를 둔다)
                    let vt = attr(c, NS_OFFICE, "value-type").filter(|_| !covered);
                    let formula = attr(c, NS_TABLE, "formula")
                        .filter(|_| !covered)
                        .map(formula_from_odf);
                    // 칸 스타일: 칸 → 행 기본 → 열 기본 → "Default"
                    let own_style = attr(c, NS_TABLE, "style-name");
                    let style_name = own_style
                        .or(row_style)
                        .or_else(|| {
                            col_default
                                .iter()
                                .find(|d| d.0 <= ci && ci < d.1)
                                .map(|d| d.2)
                        })
                        .unwrap_or("Default");
                    let style = cell_styles.xf(&mut wb.styles, style_name);
                    let has_value = vt.is_some() || formula.is_some();
                    if (has_value || (own_style.is_some() && style != 0)) && rep < 1000 {
                        let num =
                            |a: &str| attr(c, NS_OFFICE, a).and_then(|v| v.parse::<f64>().ok());
                        let value = match vt {
                            Some("float" | "percentage" | "currency") => {
                                num("value").map_or(CellValue::Empty, CellValue::Number)
                            }
                            Some("date") => attr(c, NS_OFFICE, "date-value")
                                .and_then(date_serial)
                                .map_or(CellValue::Empty, CellValue::Number),
                            Some("time") => attr(c, NS_OFFICE, "time-value")
                                .and_then(duration_days)
                                .map_or(CellValue::Empty, CellValue::Number),
                            Some("boolean") => {
                                CellValue::Bool(attr(c, NS_OFFICE, "boolean-value") == Some("true"))
                            }
                            Some("string") => CellValue::Text(
                                attr(c, NS_OFFICE, "string-value")
                                    .map(str::to_string)
                                    .unwrap_or_else(|| paragraphs(c)),
                            ),
                            _ => CellValue::Empty,
                        };
                        for k in 0..rep {
                            let cc = ci + k;
                            if cc >= 16384 {
                                break;
                            }
                            sheet.cells.push(Cell {
                                r: CellRef { row: ri, col: cc },
                                style,
                                value: value.clone(),
                                formula: formula.clone().map(|text| Formula {
                                    text,
                                    ..Default::default()
                                }),
                            });
                        }
                    }
                    if !covered {
                        let cs: u32 = attr(c, NS_TABLE, "number-columns-spanned")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1);
                        let rs: u32 = attr(c, NS_TABLE, "number-rows-spanned")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1);
                        if (cs > 1 || rs > 1) && k == 0 {
                            sheet.merges.push(CellRange {
                                start: CellRef { row: ri, col: ci },
                                end: CellRef {
                                    row: ri + rs - 1,
                                    col: ci + cs - 1,
                                },
                            });
                        }
                    }
                    ci = ci.saturating_add(rep);
                }
            }
            row = row.saturating_add(n);
        }
        sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
        crate::edit::structure::update_dimension(&mut sheet);
        wb.sheets.push(sheet);
    }
    // 이름 정의: 범위 (`table:named-range`) 와 식 (`table:named-expression`)
    if let Some(names) = spreadsheet.child_ns(NS_TABLE, "named-expressions") {
        for n in names.elements() {
            let Some(name) = attr(n, NS_TABLE, "name") else {
                continue;
            };
            let formula = if n.is(NS_TABLE, "named-range") {
                attr(n, NS_TABLE, "cell-range-address").map(range_address)
            } else if n.is(NS_TABLE, "named-expression") {
                attr(n, NS_TABLE, "expression").map(formula_from_odf)
            } else {
                None
            };
            if let Some(formula) = formula {
                wb.defined_names.push(crate::model::DefinedName {
                    name: name.to_string(),
                    formula,
                    ..Default::default()
                });
            }
        }
    }
    if wb.sheets.is_empty() {
        wb.sheets.push(template);
    }
    if wb
        .sheets
        .iter()
        .any(|s| s.cells.iter().any(|c| c.formula.is_some()))
    {
        crate::calc::recalculate(&mut wb);
    }
    Ok(wb)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 색 → `#rrggbb` (자동·알 수 없는 색은 None)
fn css(c: Option<&crate::model::Color>, theme: &crate::theme::Theme) -> Option<String> {
    c.and_then(|c| crate::theme::resolve(c, theme))
        .map(|s| s.to_ascii_lowercase())
}

/// 글꼴 → `style:text-properties` 의 속성 (한셀 꼴: 글자 색, 글꼴 이름(동아시아·복합 문자도), 크기, 굵게·기울임,
/// 밑줄·취소선·위첨자)
fn text_props(f: &crate::model::Font, theme: &crate::theme::Theme) -> String {
    let mut a = format!(
        r#" fo:color="{}""#,
        css(f.color.as_ref(), theme).unwrap_or_else(|| "#000000".into())
    );
    if let Some(n) = &f.name {
        let n = esc(n);
        a.push_str(&format!(
            r#" style:font-name="{n}" style:font-name-asian="{n}" style:font-name-complex="{n}""#
        ));
    }
    let size = f.size.unwrap_or(11.0);
    a.push_str(&format!(
        r#" fo:font-size="{size}pt" style:font-size-asian="{size}pt" style:font-size-complex="{size}pt""#
    ));
    if f.bold {
        a.push_str(r#" fo:font-weight="bold" style:font-weight-asian="bold" style:font-weight-complex="bold""#);
    }
    if f.italic {
        a.push_str(r#" fo:font-style="italic" style:font-style-asian="italic" style:font-style-complex="italic""#);
    }
    if let Some(u) = f.underline.as_deref().filter(|u| *u != "none") {
        let ty = if u.starts_with("double") {
            "double"
        } else {
            "single"
        };
        a.push_str(&format!(
            r#" style:text-underline-style="solid" style:text-underline-type="{ty}" style:text-underline-width="auto" style:text-underline-color="font-color""#
        ));
    }
    if f.strike {
        a.push_str(r#" style:text-line-through-style="solid""#);
    }
    match f.vert_align.as_deref() {
        Some("superscript") => a.push_str(r#" style:text-position="super 58%""#),
        Some("subscript") => a.push_str(r#" style:text-position="sub 58%""#),
        _ => {}
    }
    a
}

/// 테두리 변 → ODF 테두리. 한셀처럼 굵기로만 (가는 선 0.002cm, 중간·굵은 선 0.088cm, 이중선 0.039cm)
fn odf_edge(e: Option<&crate::model::BorderEdge>, theme: &crate::theme::Theme) -> Option<String> {
    let e = e?;
    let style = e.style.as_deref().filter(|s| *s != "none")?;
    let w = match style {
        "double" => "0.039cm double",
        "medium" | "thick" | "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" => {
            "0.088cm solid"
        }
        // 한셀은 쓸 때 점선·파선을 실선으로 잃지만 읽을 때는 dashed·dotted 를 안다
        "dashed" => "0.002cm dashed",
        "dotted" | "hair" => "0.002cm dotted",
        _ => "0.002cm solid",
    };
    let color = css(e.color.as_ref(), theme).unwrap_or_else(|| "#000000".into());
    Some(format!("{w} {color}"))
}

/// 셀 서식 → 자동 칸 스타일 (`style:style` family table-cell, 부모 "Default")
fn cell_style_xml(
    name: &str,
    wb: &Workbook,
    xf_index: u32,
    theme: &crate::theme::Theme,
    data_style: Option<&str>,
) -> String {
    let styles = &wb.styles;
    let xf = styles.xf(xf_index).cloned().unwrap_or_default();
    let al = xf.alignment.clone().unwrap_or_default();
    // 칸 속성: 세로 맞춤, 채우기, 테두리, 줄 바꿈, 맞춤 축소, 회전
    let mut cell = String::new();
    cell.push_str(match al.vertical.as_deref() {
        Some("top") => r#" style:vertical-align="top""#,
        Some("bottom") | None => r#" style:vertical-align="bottom""#,
        _ => r#" style:vertical-align="middle""#,
    });
    if let Some(f) = styles.fills.get(xf.fill_id as usize) {
        let color = match (&f.gradient, f.pattern_type.as_deref()) {
            // 그라데이션은 한셀처럼 쓰지 않는다
            (Some(_), _) => None,
            (None, None | Some("none")) => None,
            (None, Some(_)) => {
                css(f.fg_color.as_ref(), theme).or_else(|| css(f.bg_color.as_ref(), theme))
            }
        };
        if let Some(c) = color {
            cell.push_str(&format!(r#" fo:background-color="{c}""#));
        }
    }
    if let Some(b) = styles.borders.get(xf.border_id as usize) {
        for (k, e) in [
            ("top", &b.top),
            ("bottom", &b.bottom),
            ("left", &b.left),
            ("right", &b.right),
        ] {
            let v = odf_edge(e.as_ref(), theme).unwrap_or_else(|| "none".into());
            cell.push_str(&format!(r#" fo:border-{k}="{v}""#));
        }
        if let Some(d) = odf_edge(b.diagonal.as_ref(), theme) {
            if b.diagonal_down {
                cell.push_str(&format!(r#" style:diagonal-tl-br="{d}""#));
            }
            if b.diagonal_up {
                cell.push_str(&format!(r#" style:diagonal-bl-tr="{d}""#));
            }
        }
    }
    if al.wrap_text {
        cell.push_str(r#" fo:wrap-option="wrap""#);
    }
    // 셀 보호 (잠금·수식 숨김)
    let prot = xf.protection.clone().unwrap_or_default();
    let (locked, hidden) = (prot.locked != Some(false), prot.hidden == Some(true));
    if !locked || hidden {
        cell.push_str(match (locked, hidden) {
            (false, false) => r#" style:cell-protect="none""#,
            (false, true) => r#" style:cell-protect="formula-hidden""#,
            _ => r#" style:cell-protect="protected formula-hidden""#,
        });
    }
    if al.shrink_to_fit {
        cell.push_str(r#" style:shrink-to-fit="true""#);
    }
    if let Some(r) = al.text_rotation.filter(|&r| r != 0 && r != 255) {
        let deg = if r <= 90 { r } else { 360 - (r - 90) };
        cell.push_str(&format!(r#" style:rotation-angle="{deg}""#));
    }
    // 문단 속성: 가로 맞춤과 들여쓰기 (단계마다 0.353cm)
    let mut para = String::new();
    if let Some(h) = match al.horizontal.as_deref() {
        Some("left" | "fill") => Some("start"),
        Some("center" | "centerContinuous") => Some("center"),
        Some("right") => Some("end"),
        Some("justify" | "distributed") => Some("justify"),
        _ => None,
    } {
        para.push_str(&format!(r#" fo:text-align="{h}""#));
    }
    if let Some(i) = al.indent.filter(|&i| i > 0) {
        let m = i as f64 * 0.353;
        para.push_str(&format!(
            r#" fo:margin-left="{m:.6}cm" fo:margin-right="{m:.6}cm""#
        ));
    }
    let font = styles
        .fonts
        .get(xf.font_id as usize)
        .cloned()
        .unwrap_or_default();
    format!(
        r#"<style:style style:name="{name}" style:parent-style-name="Default" style:family="table-cell"{}><style:table-cell-properties{cell}/><style:paragraph-properties{para}/><style:text-properties{}/></style:style>"#,
        data_style
            .map(|d| format!(r#" style:data-style-name="{d}""#))
            .unwrap_or_default(),
        text_props(&font, theme)
    )
}

/// 머리글 코드 한 영역 → ODF 문단 내용 (`&"글꼴,모양"`·`&크기` 는 글자 스타일 span, `&A`·`&P`·`&N`·`&D`·`&T`·`&F` 는
/// 필드). 글자 스타일은 `text_styles` 에 더한다
fn hf_odf(code: &str, text_styles: &mut Vec<String>, theme_font: &str) -> String {
    let mut out = String::new();
    let mut font: Option<(String, bool, bool)> = None;
    let mut size: Option<f64> = None;
    let mut run = String::new();
    let flush = |run: &mut String,
                 out: &mut String,
                 font: &Option<(String, bool, bool)>,
                 size: Option<f64>,
                 text_styles: &mut Vec<String>| {
        if run.is_empty() {
            return;
        }
        if font.is_none() && size.is_none() {
            out.push_str(run);
        } else {
            let (name, bold, italic) =
                font.clone()
                    .unwrap_or((theme_font.to_string(), false, false));
            let mut p = format!(
                r#" style:font-name="{0}" style:font-name-asian="{0}""#,
                esc(&name)
            );
            if let Some(s) = size {
                p.push_str(&format!(
                    r#" fo:font-size="{s}pt" style:font-size-asian="{s}pt""#
                ));
            }
            if bold {
                p.push_str(r#" fo:font-weight="bold""#);
            }
            if italic {
                p.push_str(r#" fo:font-style="italic""#);
            }
            let k = match text_styles.iter().position(|x| *x == p) {
                Some(k) => k,
                None => {
                    text_styles.push(p);
                    text_styles.len() - 1
                }
            };
            out.push_str(&format!(
                r#"<text:span text:style-name="T{}">{run}</text:span>"#,
                k + 1
            ));
        }
        run.clear();
    };
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '&' || i + 1 >= chars.len() {
            match c {
                '<' => run.push_str("&lt;"),
                '>' => run.push_str("&gt;"),
                '&' => run.push_str("&amp;"),
                _ => run.push(c),
            }
            i += 1;
            continue;
        }
        let n = chars[i + 1];
        if n == '"' {
            // &"글꼴,모양"
            let end = chars[i + 2..]
                .iter()
                .position(|&x| x == '"')
                .map(|p| i + 2 + p);
            let Some(end) = end else { break };
            flush(&mut run, &mut out, &font, size, text_styles);
            let spec: String = chars[i + 2..end].iter().collect();
            let (name, shape) = spec.split_once(',').unwrap_or((&spec, "Regular"));
            let name = if name == "-" { theme_font } else { name };
            font = Some((
                name.to_string(),
                shape.contains("Bold"),
                shape.contains("Italic"),
            ));
            i = end + 1;
        } else if n.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            flush(&mut run, &mut out, &font, size, text_styles);
            size = chars[i + 1..j].iter().collect::<String>().parse().ok();
            i = j;
        } else {
            let field = match n {
                'A' => Some("<text:sheet-name>???</text:sheet-name>"),
                'P' => Some("<text:page-number>1</text:page-number>"),
                'N' => Some("<text:page-count>1</text:page-count>"),
                'D' => Some("<text:date/>"),
                'T' => Some("<text:time/>"),
                'F' => Some("<text:file-name>???</text:file-name>"),
                '&' => {
                    run.push_str("&amp;");
                    None
                }
                // 글자 모양 켜고 끄기(굵게·기울임·밑줄·취소선·위아래 첨자)는 버린다
                'B' | 'I' | 'U' | 'S' | 'E' | 'X' | 'Y' | 'G' | 'Z' => None,
                // 코드가 아니면 그대로 (한셀 서식 파일의 `&[SHEET]` 같은 글)
                _ => {
                    run.push_str("&amp;");
                    match n {
                        '<' => run.push_str("&lt;"),
                        '>' => run.push_str("&gt;"),
                        _ => run.push(n),
                    }
                    None
                }
            };
            if let Some(f) = field {
                run.push_str(f);
            }
            i += 2;
        }
    }
    flush(&mut run, &mut out, &font, size, text_styles);
    out
}

/// 머리글·바닥글 코드 → `style:header`/`style:footer` 의 영역들
fn hf_regions(code: &str, text_styles: &mut Vec<String>, theme_font: &str) -> String {
    let mut sections: [String; 3] = Default::default();
    let mut cur = 1usize;
    let mut rest = code;
    let mut buf = String::new();
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("&L") {
            sections[cur].push_str(&buf);
            buf.clear();
            cur = 0;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("&C") {
            sections[cur].push_str(&buf);
            buf.clear();
            cur = 1;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("&R") {
            sections[cur].push_str(&buf);
            buf.clear();
            cur = 2;
            rest = r;
        } else if rest.starts_with("&&") {
            buf.push_str("&&");
            rest = &rest[2..];
        } else {
            let c = rest.chars().next().unwrap_or_default();
            buf.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    sections[cur].push_str(&buf);
    let mut out = String::new();
    for (k, s) in ["region-left", "region-center", "region-right"]
        .iter()
        .zip(sections.iter())
    {
        if !s.is_empty() {
            let paras: String = s
                .split('\n')
                .map(|line| format!("<text:p>{}</text:p>", hf_odf(line, text_styles, theme_font)))
                .collect();
            out.push_str(&format!("<style:{k}>{paras}</style:{k}>"));
        }
    }
    out
}

/// 시트 인쇄 설정 → 쪽 모양과 마스터 쪽. 한셀이 읽을 때 위(아래) 여백 = 쪽 여백 + 머리글(바닥글) 높이 + 간격 이므로
/// 쪽 여백에 머리글 여백을, 머리글 높이에 나머지를 둔다 (한셀 자신은 높이 0.751cm·간격 0.25cm 로 고정해 위 여백을 잃는다)
fn page_xml(
    i: usize,
    sheet: &crate::model::Sheet,
    text_styles: &mut Vec<String>,
    theme_font: &str,
) -> (String, String) {
    let p = &sheet.page;
    let m = p.margins.clone().unwrap_or(crate::model::PageMargins {
        left: 0.7,
        right: 0.7,
        top: 0.75,
        bottom: 0.75,
        header: 0.3,
        footer: 0.3,
    });
    let cm = |inch: f64| inch * 2.54;
    let su = p.setup.clone().unwrap_or_default();
    let landscape = su.orientation.as_deref() == Some("landscape");
    let (pw, ph) = PAPERS
        .iter()
        .find(|x| Some(x.0) == su.paper_size)
        .map_or((21.0, 29.7), |x| (x.1, x.2));
    let (pw, ph) = if landscape { (ph, pw) } else { (pw, ph) };
    let mut props = format!(
        r#"fo:page-width="{pw}cm" fo:page-height="{ph}cm" style:print-orientation="{}" fo:margin-left="{:.6}cm" fo:margin-right="{:.6}cm" fo:margin-top="{:.6}cm" fo:margin-bottom="{:.6}cm" style:scale-to="{}""#,
        if landscape { "landscape" } else { "portrait" },
        cm(m.left),
        cm(m.right),
        cm(m.header),
        cm(m.footer),
        su.scale.unwrap_or(100)
    );
    // 한셀처럼 배율과 쪽 맞춤 수를 함께 쓴다 (한셀은 읽을 때 늘 쪽 맞춤을 켠다)
    let fit = p.fit_to_page == Some(true);
    props.push_str(&format!(
        r#" style:scale-to-X="{}" style:scale-to-Y="{}""#,
        su.fit_to_width.unwrap_or(fit as u32),
        su.fit_to_height.unwrap_or(fit as u32)
    ));
    let o = p.print_options.clone().unwrap_or_default();
    let centering = match (o.horizontal_centered, o.vertical_centered) {
        (true, true) => Some("both"),
        (true, false) => Some("horizontal"),
        (false, true) => Some("vertical"),
        _ => None,
    };
    if let Some(c) = centering {
        props.push_str(&format!(r#" style:table-centering="{c}""#));
    }
    let mut print = vec!["charts", "drawings", "objects", "zero-values"];
    if o.grid_lines {
        print.push("grid");
    }
    if o.headings {
        print.push("headers");
    }
    props.push_str(&format!(r#" style:print="{}""#, print.join(" ")));
    if su.page_order.as_deref() == Some("downThenOver") {
        props.push_str(r#" style:print-page-order="ttb""#);
    }
    if su.use_first_page_number
        && let Some(n) = su.first_page_number
    {
        props.push_str(&format!(r#" style:first-page-number="{n}""#));
    }
    // 머리글·바닥글 높이: 위(아래) 여백 − 머리글(바닥글) 여백 − 간격
    let block = |outer: f64, inner: f64| {
        let room = cm(outer - inner).max(0.0);
        let gap = if room >= 0.25 { 0.25 } else { 0.0 };
        (room - gap, gap)
    };
    let (hh, hg) = block(m.top, m.header);
    let (fh, fg) = block(m.bottom, m.footer);
    let layout = format!(
        r#"<style:page-layout style:name="pm{i}"><style:page-layout-properties {props}/><style:header-style><style:header-footer-properties fo:min-height="{hh:.6}cm" fo:margin-left="0cm" fo:margin-right="0cm" fo:margin-bottom="{hg}cm"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height="{fh:.6}cm" fo:margin-left="0cm" fo:margin-right="0cm" fo:margin-top="{fg}cm"/></style:footer-style></style:page-layout>"#
    );
    let hf = p.header_footer.clone().unwrap_or_default();
    let mut master =
        format!(r#"<style:master-page style:name="mp{i}" style:page-layout-name="pm{i}">"#);
    for (tag, code) in [("header", &hf.odd_header), ("footer", &hf.odd_footer)] {
        if let Some(c) = code.as_deref().filter(|c| !c.is_empty()) {
            master.push_str(&format!(
                "<style:{tag}>{}</style:{tag}>",
                hf_regions(c, text_styles, theme_font)
            ));
        }
    }
    master.push_str("</style:master-page>");
    (layout, master)
}

/// 이름 정의의 수식이 시트 범위 하나면 ODF 범위 주소 (`$'시트'.$A$1:.$B$2`)
fn odf_range(formula: &str) -> Option<String> {
    let (sheet, cells) = formula.rsplit_once('!')?;
    let sheet = sheet.trim_matches('\'').replace("''", "'");
    let parts: Vec<&str> = cells.split(':').collect();
    if parts.len() > 2
        || parts
            .iter()
            .any(|p| CellRef::parse(&p.replace('$', "")).is_none())
    {
        return None;
    }
    let q = if sheet.chars().all(|c| c.is_alphanumeric() || c == '_') {
        sheet
    } else {
        format!("'{}'", sheet.replace('\'', "''"))
    };
    let a = parts
        .iter()
        .enumerate()
        .map(|(k, p)| {
            if k == 0 {
                format!("${q}.{p}")
            } else {
                format!(".{p}")
            }
        })
        .collect::<Vec<_>>()
        .join(":");
    Some(a)
}

/// 통합 문서 → `.ods` 바이트 (한셀처럼 `mimetype` 없이 내용·스타일·메타·설정·매니페스트)
pub fn write(wb: &Workbook) -> Result<Vec<u8>> {
    // 값이 없는 수식이 있으면 다시 계산한 사본으로 쓴다 (ODS 는 수식의 값을 담는다 — 한셀은 열 때 다시 계산하지 않는다)
    let computed;
    let wb = if wb.sheets.iter().any(|s| {
        s.cells
            .iter()
            .any(|c| c.formula.is_some() && c.value.is_empty())
    }) {
        let mut w = wb.clone();
        crate::calc::recalculate(&mut w);
        computed = w;
        &computed
    } else {
        wb
    };
    const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:tableooo="http://openoffice.org/2009/table" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0""#;
    let mut auto = String::new();
    let mut body = String::new();
    let theme = wb.theme.clone().unwrap_or_default();
    let font_pt = wb.styles.font_of(0).and_then(|f| f.size).unwrap_or(11.0);
    // 셀 서식 → 자동 칸 스타일 이름 (기본 서식 0 은 열의 기본 칸 스타일 "Default")
    let mut cell_style_names: HashMap<u32, String> = HashMap::new();
    let mut data_styles: Vec<u32> = Vec::new();
    let mut col_styles: Vec<String> = Vec::new();
    let mut row_styles: Vec<String> = Vec::new();
    let style_of = |list: &mut Vec<String>,
                    key: String,
                    prefix: &str,
                    auto: &mut String,
                    make: &dyn Fn(&str) -> String|
     -> String {
        let k = match list.iter().position(|x| *x == key) {
            Some(k) => k,
            None => {
                list.push(key.clone());
                let name = format!("{prefix}{}", list.len());
                auto.push_str(&make(&name));
                list.len() - 1
            }
        };
        format!("{prefix}{}", k + 1)
    };
    // 쪽 모양·마스터 쪽 (styles.xml), 머리글 글자 스타일
    let mut layouts = String::new();
    let mut masters = String::new();
    let mut hf_styles: Vec<String> = Vec::new();
    let theme_font = wb
        .styles
        .font_of(0)
        .and_then(|f| f.name.clone())
        .unwrap_or_else(|| "맑은 고딕".into());
    for (si, sheet) in wb.sheets.iter().enumerate() {
        let i = si + 1;
        let (layout, master) = page_xml(i, sheet, &mut hf_styles, &theme_font);
        layouts.push_str(&layout);
        masters.push_str(&master);
        // 표 스타일: 마스터 쪽, 숨김, 탭 색
        let tab = css(sheet.tab_color.as_ref(), &theme)
            .map(|c| format!(r#" tableooo:tab-color="{c}""#))
            .unwrap_or_default();
        auto.push_str(&format!(
            r#"<style:style style:name="ta{i}" style:family="table" style:master-page-name="mp{i}"><style:table-properties table:display="{}"{tab}/></style:style>"#,
            sheet.state == crate::model::SheetState::Visible
        ));
        body.push_str(&format!(
            r#"<table:table table:name="{}" table:style-name="ta{i}">"#,
            esc(&sheet.name)
        ));
        let last_col = sheet.cells.iter().map(|c| c.r.col).max().unwrap_or(0).max(
            sheet
                .cols
                .iter()
                .map(|c| c.max)
                .max()
                .unwrap_or(1)
                .saturating_sub(1),
        );
        // 열은 끝(16384)까지 쓴다 — 한셀처럼, 마지막 열 묶음은 반복으로
        let mut c = 0u32;
        while c <= last_col.max(16383) {
            let info = sheet.cols.iter().find(|x| x.min <= c + 1 && c < x.max);
            let width = info
                .and_then(|i| i.width)
                .unwrap_or(sheet.default_col_width.unwrap_or(9.003_906_25));
            // 한셀은 읽을 때 글자 수를 1/256 로 버린다 — 식의 오차(1e-6cm)로 한 칸 아래가 되지 않게 조금 올린다
            let w = format!(
                "{:.6}cm",
                (width_cm(width, font_pt) * 1e6).ceil() / 1e6 + 1e-5
            );
            let name = style_of(&mut col_styles, w.clone(), "co", &mut auto, &|n| {
                format!(
                    r#"<style:style style:name="{n}" style:family="table-column"><style:table-column-properties style:column-width="{w}"/></style:style>"#
                )
            });
            let hidden = if info.is_some_and(|i| i.hidden) {
                r#" table:visibility="collapse""#
            } else {
                ""
            };
            // 마지막 쓴 열 뒤로 같은 열이 이어지면 한 번에
            let rep = if c > last_col { 16384 - c } else { 1 };
            let repeated = if rep > 1 {
                format!(r#" table:number-columns-repeated="{rep}""#)
            } else {
                String::new()
            };
            body.push_str(&format!(
                r#"<table:table-column{repeated} table:style-name="{name}"{hidden} table:default-cell-style-name="Default"/>"#
            ));
            c += rep;
        }
        let last_row = sheet
            .cells
            .iter()
            .map(|c| c.r.row)
            .chain(sheet.merges.iter().map(|m| m.end.row))
            .chain(
                sheet
                    .rows
                    .iter()
                    .filter(|r| r.height.is_some() || r.hidden)
                    .map(|r| r.index.saturating_sub(1)),
            )
            .max()
            .unwrap_or(0);
        let mut i = 0usize;
        for r in 0..=last_row {
            let info = sheet.rows.iter().find(|x| x.index == r + 1);
            let h = info
                .and_then(|i| i.height)
                .unwrap_or(sheet.default_row_height.unwrap_or(16.5));
            // 한셀처럼 높이가 있는 행은 모두 정한 높이로 (자동 맞춤 높이도)
            let custom = info.is_some_and(|i| i.custom_height || i.height.is_some());
            let key = format!("{h:.2}pt/{custom}");
            let name = style_of(&mut row_styles, key, "ro", &mut auto, &|n| {
                format!(
                    r#"<style:style style:name="{n}" style:family="table-row"><style:table-row-properties style:row-height="{:.6}cm" style:use-optimal-row-height="{}"/></style:style>"#,
                    // 한셀은 읽을 때 pt 를 정수로 버린다 — 정수 pt 가 아래로 반올림되지 않게 올린다
                    (h / 72.0 * 2.54 * 1e6).ceil() / 1e6,
                    !custom
                )
            });
            let hidden = if info.is_some_and(|i| i.hidden) {
                r#" table:visibility="collapse""#
            } else {
                ""
            };
            body.push_str(&format!(
                r#"<table:table-row table:style-name="{name}"{hidden}>"#
            ));
            // 이 행의 칸: 값·수식이 있는 칸과 병합의 첫 칸
            let mut entries: Vec<(u32, Option<&Cell>)> = Vec::new();
            while i < sheet.cells.len() && sheet.cells[i].r.row == r {
                let cell = &sheet.cells[i];
                i += 1;
                if !cell.value.is_empty() || cell.formula.is_some() || cell.style != 0 {
                    entries.push((cell.r.col, Some(cell)));
                }
            }
            for m in sheet.merges.iter().filter(|m| m.start.row == r) {
                if !entries.iter().any(|e| e.0 == m.start.col) {
                    entries.push((m.start.col, None));
                }
            }
            entries.sort_by_key(|e| e.0);
            let mut col = 0u32;
            for (at, cell) in entries {
                let here = CellRef { row: r, col: at };
                if at > col {
                    body.push_str(&format!(
                        r#"<table:table-cell table:number-columns-repeated="{}"/>"#,
                        at - col
                    ));
                }
                let covered = sheet
                    .merges
                    .iter()
                    .any(|m| m.contains(here) && m.start != here);
                // 병합에 덮인 칸도 한셀처럼 table:table-cell 로 쓴다 (한셀은 covered-table-cell 의 스타일을 버리고
                // 병합 첫 칸의 스타일을 준다 — 한셀 .ods 열기 대조)
                let tag = "table:table-cell";
                let _ = covered;
                let mut a = String::new();
                if let Some(x) = cell.map(|c| c.style).filter(|&x| x != 0) {
                    let name = match cell_style_names.get(&x) {
                        Some(n) => n.clone(),
                        None => {
                            let n = format!("ce{}", cell_style_names.len() + 1);
                            // 표시 형식: 한셀은 쓰지 않지만 읽는다 (number:*-style)
                            let id = wb.styles.xf(x).map_or(0, |xf| xf.num_fmt_id);
                            // 기본 서식에 표시 형식이 있으면 일반 서식도 밝힌다 (부모 "Default" 를 물려받지 않게)
                            let base_fmt = wb.styles.xf(0).map_or(0, |xf| xf.num_fmt_id);
                            let general = (id == 0 && base_fmt != 0).then(|| {
                                if !data_styles.contains(&u32::MAX) {
                                    auto.push_str(r#"<number:number-style style:name="NG"><number:number number:min-integer-digits="1"/></number:number-style>"#);
                                    data_styles.push(u32::MAX);
                                }
                                "NG".to_string()
                            });
                            let data = (id != 0)
                                .then(|| {
                                    let dn = format!("N{id}");
                                    if !data_styles.contains(&id) {
                                        let code = crate::numfmt::code_for(&wb.styles, id);
                                        let xml = number_styles_xml(&dn, &code)?;
                                        auto.push_str(&xml);
                                        data_styles.push(id);
                                    }
                                    Some(dn)
                                })
                                .flatten()
                                .or(general);
                            auto.push_str(&cell_style_xml(&n, wb, x, &theme, data.as_deref()));
                            cell_style_names.insert(x, n.clone());
                            n
                        }
                    };
                    a.push_str(&format!(r#" table:style-name="{name}""#));
                }
                if let Some(m) = sheet.merges.iter().find(|m| m.start == here) {
                    a.push_str(&format!(
                        r#" table:number-columns-spanned="{}" table:number-rows-spanned="{}""#,
                        m.end.col - m.start.col + 1,
                        m.end.row - m.start.row + 1
                    ));
                }
                if let Some(f) = cell.and_then(|c| c.formula.as_ref()) {
                    a.push_str(&format!(
                        r#" table:formula="{}""#,
                        esc(&formula_to_odf(&f.text))
                    ));
                }
                let empty = CellValue::Empty;
                let text = match cell.map_or(&empty, |c| &c.value) {
                    CellValue::Number(x) => {
                        a.push_str(&format!(r#" office:value-type="float" office:value="{x}""#));
                        cell.map(|c| crate::csv::shown(wb, c)).unwrap_or_default()
                    }
                    CellValue::Bool(b) => {
                        a.push_str(&format!(
                            r#" office:value-type="boolean" office:boolean-value="{b}""#
                        ));
                        if *b { "TRUE" } else { "FALSE" }.into()
                    }
                    CellValue::Text(t) => {
                        a.push_str(r#" office:value-type="string""#);
                        t.clone()
                    }
                    CellValue::Error(e) => {
                        a.push_str(r#" office:value-type="string""#);
                        e.clone()
                    }
                    CellValue::Date(d) => {
                        a.push_str(&format!(
                            r#" office:value-type="date" office:date-value="{}""#,
                            esc(d)
                        ));
                        d.clone()
                    }
                    CellValue::Empty => String::new(),
                };
                body.push_str(&format!("<{tag}{a}>"));
                if cell.is_some_and(|c| !c.value.is_empty()) {
                    for line in text.split('\n') {
                        body.push_str(&format!("<text:p>{}</text:p>", esc(line)));
                    }
                }
                body.push_str(&format!("</{tag}>"));
                col = at + 1;
            }
            body.push_str("</table:table-row>");
        }
        body.push_str("</table:table>");
    }
    // 이름 정의: 시트 범위 하나는 named-range, 나머지는 named-expression (예약 이름 _xlnm. 은 뺀다)
    let mut names = String::new();
    for n in wb.defined_names.iter().filter(|n| {
        // 한셀처럼 잃은 참조(#REF!)의 이름은 뺀다
        !n.name.starts_with("_xlnm.") && n.local_sheet_id.is_none() && !n.formula.contains("#REF!")
    }) {
        let name = esc(&n.name);
        match odf_range(&n.formula) {
            Some(a) => {
                let base = a.split(':').next().unwrap_or(&a).to_string();
                names.push_str(&format!(
                    r#"<table:named-range table:name="{name}" table:cell-range-address="{}" table:base-cell-address="{}"/>"#,
                    esc(&a),
                    esc(&base)
                ));
            }
            None => names.push_str(&format!(
                r#"<table:named-expression table:name="{name}" table:expression="{}"/>"#,
                esc(&formula_to_odf(&n.formula))
            )),
        }
    }
    if !names.is_empty() {
        body.push_str(&format!(
            "<table:named-expressions>{names}</table:named-expressions>"
        ));
    }
    // 글꼴 선언: 쓰인 글꼴 이름 모두
    let mut faces: Vec<&str> = Vec::new();
    for f in &wb.styles.fonts {
        if let Some(n) = f.name.as_deref()
            && !faces.contains(&n)
        {
            faces.push(n);
        }
    }
    let faces: String = faces
        .iter()
        .map(|n| {
            let n = esc(n);
            format!(r#"<style:font-face style:name="{n}" svg:font-family="{n}"/>"#)
        })
        .collect();
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} office:version="1.2"><office:font-face-decls>{faces}</office:font-face-decls><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"#
    );
    // 기본 칸 스타일 "Default" = 기본 셀 서식 0 (표시 형식·맞춤·테두리까지 — 한셀은 글자 속성만 쓴다)
    let default_data =
        wb.styles.xf(0).filter(|x| x.num_fmt_id != 0).and_then(|x| {
            number_styles_xml("N0", &crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        });
    let default_style = cell_style_xml(
        "Default",
        wb,
        0,
        &theme,
        default_data.as_ref().map(|_| "N0"),
    )
    .replace(r#" style:parent-style-name="Default""#, "");
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS} office:version="1.2"><office:font-face-decls>{faces}</office:font-face-decls><office:styles>{}{default_style}</office:styles><office:automatic-styles>{layouts}{}</office:automatic-styles><office:master-styles>{masters}</office:master-styles></office:document-styles>"#,
        default_data.unwrap_or_default(),
        hf_styles
            .iter()
            .enumerate()
            .map(|(k, p)| format!(r#"<style:style style:name="T{}" style:family="text"><style:text-properties{p}/></style:style>"#, k + 1))
            .collect::<String>()
    );
    let meta = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-meta {NS} office:version="1.2"><office:meta><meta:generator>rcell</meta:generator></office:meta></office:document-meta>"#
    );
    // 설정: 한셀 꼴 (시트마다 커서 위치, 활성 시트, 확대율, 행·열 머리글). 이 파트가 없으면 한셀은 눈금선을 끈다
    let mut tables = String::new();
    for sheet in &wb.sheets {
        let at = sheet
            .view
            .active_cell
            .as_deref()
            .and_then(CellRef::parse)
            .unwrap_or(CellRef { row: 0, col: 0 });
        tables.push_str(&format!(
            r#"<config:config-item-map-entry config:name="{}"><config:config-item config:name="CursorPositionX" config:type="int">{}</config:config-item><config:config-item config:name="CursorPositionY" config:type="int">{}</config:config-item><config:config-item config:name="ShowGrid" config:type="boolean">{}</config:config-item></config:config-item-map-entry>"#,
            esc(&sheet.name),
            at.col,
            at.row,
            sheet.show_grid_lines
        ));
    }
    let active = wb
        .sheets
        .iter()
        .find(|s| s.view.tab_selected)
        .or(wb.sheets.first());
    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0" office:version="1.2"><office:settings><config:config-item-set config:name="ooo:view-settings"><config:config-item-map-indexed config:name="Views"><config:config-item-map-entry><config:config-item-map-named config:name="Tables">{tables}</config:config-item-map-named><config:config-item config:name="ActiveTable" config:type="string">{}</config:config-item><config:config-item config:name="ZoomValue" config:type="int">{}</config:config-item><config:config-item config:name="PageViewZoomValue" config:type="int">100</config:config-item><config:config-item config:name="ShowPageBreakPreview" config:type="boolean">false</config:config-item><config:config-item config:name="HasColumnRowHeaders" config:type="boolean">{}</config:config-item></config:config-item-map-entry></config:config-item-map-indexed></config:config-item-set></office:settings></office:document-settings>"#,
        esc(active.map_or("", |s| s.name.as_str())),
        active.and_then(|s| s.view.zoom_scale).unwrap_or(100),
        active.is_none_or(|s| s.view.show_row_col_headers)
    );
    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/></manifest:manifest>"#;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let zerr = |e: zip::result::ZipError| Error::InvalidValue {
        context: "ods".into(),
        value: e.to_string(),
    };
    // ODF 는 첫 항목으로 압축하지 않은 mimetype 을 둔다 (한셀은 쓰지 않지만 다른 프로그램을 위해)
    z.start_file("mimetype", stored).map_err(zerr)?;
    z.write_all(b"application/vnd.oasis.opendocument.spreadsheet")?;
    for (name, data) in [
        ("META-INF/manifest.xml", manifest.to_string()),
        ("content.xml", content),
        ("styles.xml", styles),
        ("meta.xml", meta),
        ("settings.xml", settings),
    ] {
        z.start_file(name, deflated).map_err(zerr)?;
        z.write_all(data.as_bytes())?;
    }
    Ok(z.finish().map_err(zerr)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas() {
        assert_eq!(formula_from_odf("of:=SUM([.A1:.A3];1)"), "SUM(A1:A3,1)");
        assert_eq!(
            formula_from_odf("of:=IF(A1>5;\"a;b\";2)"),
            "IF(A1>5,\"a;b\",2)"
        );
        assert_eq!(
            formula_from_odf("of:=[$'Sheet 2'.$A$1]+1"),
            "'Sheet 2'!$A$1+1"
        );
        assert_eq!(
            formula_to_odf("IF(A1>5,\"a,b\",2)"),
            "of:=IF(A1>5;\"a,b\";2)"
        );
        // 한셀 꼴: 괄호 없는 다른 시트 참조, 잃은 참조, 점이 든 함수 이름과 소수
        assert_eq!(formula_from_odf("of:=Sheet2.A1*2+1.5"), "Sheet2!A1*2+1.5");
        assert_eq!(formula_from_odf("of:=STDEV.S(A1:A3)"), "STDEV.S(A1:A3)");
        assert_eq!(
            formula_from_odf("of:=CONCATENATE(#REF.;1)"),
            "CONCATENATE(#REF!,1)"
        );
        assert_eq!(range_address("$Variables.$B$1"), "Variables!$B$1");
        let f = "Sheet2!A1*2+SUM('My Sheet'!A1:B2,1)";
        let odf = formula_to_odf(f);
        assert_eq!(odf, "of:=[Sheet2.A1]*2+SUM(['My Sheet'.A1:.B2];1)");
        assert_eq!(formula_from_odf(&odf), f);
    }

    #[test]
    fn dates() {
        assert_eq!(date_serial("2024-03-15"), Some(45366.0));
        assert_eq!(date_serial("1900-03-01"), Some(61.0));
        assert_eq!(duration_days("PT12H00M00S"), Some(0.5));
    }

    #[test]
    fn round_trip() {
        let mut wb = crate::edit::new_workbook();
        for (a1, v) in [
            ("A1", "10"),
            ("A2", "20"),
            ("B1", "글"),
            ("A3", "=SUM(A1:A2)"),
            ("C1", "TRUE"),
        ] {
            crate::edit::set_input(&mut wb, 0, CellRef::parse(a1).unwrap(), v).unwrap();
        }
        wb.sheets[0].merges.push(CellRange::parse("D1:E2").unwrap());
        crate::calc::recalculate(&mut wb);
        let bytes = write(&wb).unwrap();
        assert!(is_ods(&bytes));
        let back = read(&bytes).unwrap();
        let s = &back.sheets[0];
        let at = |a: &str| s.cell(CellRef::parse(a).unwrap()).map(|c| c.value.clone());
        assert_eq!(at("A3"), Some(CellValue::Number(30.0)));
        assert_eq!(at("B1"), Some(CellValue::Text("글".into())));
        assert_eq!(at("C1"), Some(CellValue::Bool(true)));
        assert_eq!(
            s.cell(CellRef::parse("A3").unwrap())
                .and_then(|c| c.formula.as_ref())
                .map(|f| f.text.as_str()),
            Some("SUM(A1:A2)")
        );
        assert_eq!(s.merges, vec![CellRange::parse("D1:E2").unwrap()]);
    }

    #[test]
    fn cell_formats() {
        use crate::edit::style::{BorderPreset, FormatPatch};
        let mut wb = crate::edit::new_workbook();
        crate::edit::set_input(&mut wb, 0, CellRef::parse("B2").unwrap(), "글").unwrap();
        let r = CellRange::parse("B2:C3").unwrap();
        crate::edit::format(
            &mut wb,
            0,
            r,
            &FormatPatch {
                bold: Some(true),
                italic: Some(true),
                font_size: Some(14.0),
                font_color: Some("FF0000".into()),
                fill: Some("FFFF00".into()),
                h_align: Some("center".into()),
                v_align: Some("top".into()),
                wrap: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        crate::edit::border(
            &mut wb,
            0,
            r,
            BorderPreset::Outline,
            "medium",
            Some("0000FF"),
        )
        .unwrap();
        let back = read(&write(&wb).unwrap()).unwrap();
        let key = |w: &Workbook, a: &str| {
            let c = w.sheets[0]
                .cell(CellRef::parse(a).unwrap())
                .map_or(0, |c| c.style);
            crate::compare::style_key(&w.styles, c, Default::default())
        };
        // 빈 칸의 서식도 옮긴다. 중간 굵기 선은 한셀처럼 굵은 선이 된다
        let k = key(&back, "C3");
        assert!(k.contains("size=14 bold italic color=ff0000"), "{k}");
        assert!(k.contains("fill[solid/ffff00]"), "{k}");
        assert!(
            k.contains("R=thick/0000ff") && k.contains("B=thick/0000ff"),
            "{k}"
        );
        assert!(k.contains("h=center v=top wrap"), "{k}");
        assert!(key(&back, "B2").contains("para["));
    }

    #[test]
    fn number_formats_round_trip() {
        // 표시 형식은 ODF 표시 형식 스타일로 쓰고 읽어도 같은 글이 된다
        let codes = [
            "#,##0.00",
            "0.0%",
            "yyyy-mm-dd",
            "#,##0;[Red]-#,##0",
            "\"₩\"#,##0",
            "0.00E+00",
            "h:mm:ss",
            "mmmm d, yyyy",
        ];
        for code in codes {
            let mut wb = crate::edit::new_workbook();
            crate::edit::set_input(&mut wb, 0, CellRef::parse("A1").unwrap(), "45366.75").unwrap();
            crate::edit::format(
                &mut wb,
                0,
                CellRange::parse("A1").unwrap(),
                &crate::edit::style::FormatPatch {
                    num_fmt: Some(code.into()),
                    ..Default::default()
                },
            )
            .unwrap();
            let back = read(&write(&wb).unwrap()).unwrap();
            let shown = |w: &Workbook, v: f64| {
                let c = w.sheets[0].cell(CellRef::parse("A1").unwrap()).unwrap();
                let id = w.styles.xf(c.style).map_or(0, |x| x.num_fmt_id);
                let code = crate::numfmt::code_for(&w.styles, id);
                crate::numfmt::format(&CellValue::Number(v), &code, false).text
            };
            for v in [45366.75, -1234.5, 0.25] {
                assert_eq!(shown(&back, v), shown(&wb, v), "{code} {v}");
            }
        }
    }
}
