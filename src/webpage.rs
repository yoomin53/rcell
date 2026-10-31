// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 웹 페이지 저장 — 한셀의 `.htm`(FileFormat 44)과 `.mht`(45) 꼴 (docs/format/other-formats.md 9절).
//!
//! `.htm` 은 프레임 묶음 파일 `이름.htm` 과 `이름.files/` 폴더 (시트마다 `sheetNNN.htm`, 시트 탭 `tabs.htm`, 그림
//! `imgN.png`) 이고, `.mht` 는 같은 파일들을 MIME `multipart/related` 한 덩이로 묶는다 (본문 quoted-printable, 그림 base64).
//! 시트 페이지는 칸마다 `<td class="hcxN">` 한 칸이고 서식은 `<style>` 의 클래스(`.hcxN`)로 쓴다.
//!
//! 한셀과 같게 쓰지 못하는 것: 도형·차트는 한셀이 PNG 그림으로 그려 넣지만 rcell 은 넣지 않는다. 그림은 한셀이 PNG 로
//! 다시 그려 칸 크기로 줄이지만 rcell 은 원본 바이트를 그대로 옮긴다 (그림 디코더 없음).

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::model::{
    AnchorKind, Border, BorderEdge, Cell, Color, Font, Paint, Shape, ShapeKind, Sheet, SheetState,
    Workbook,
};

/// 만든 파일 하나: 경로(`/` 로 나눔, 기준 폴더 아래)와 내용
#[derive(Debug, Clone)]
pub struct WebFile {
    pub path: String,
    pub data: Vec<u8>,
}

/// 쪽 하나의 글 — 한셀처럼 어느 글자도 이스케이프하지 않는다 (`<`·`&` 가 그대로 나온다)
struct Pages {
    /// 프레임 묶음 (`이름.htm`)
    main: String,
    /// (파일 이름 `sheet001.htm`, 내용)
    sheets: Vec<(String, String)>,
    tabs: String,
    /// (파일 이름, 바이트)
    images: Vec<(String, Vec<u8>)>,
}

const XMLNS: &str = r#"<html xmlns:x="urn:schemas-microsoft-com:office:excel" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns="http://www.w3.org/TR/REC-html40" lang="ko">"#;

// ---------------------------------------------------------------------------
// 표시 형식 → mso-number-format
// ---------------------------------------------------------------------------

/// 서식 문자열을 한셀 안의 꼴로 (글자 사이 문자를 `\` 로 막는다). 숫자 서식은 `:` 도 막고, 날짜·시간 서식은 `/` 를 막는다.
/// 따옴표 안 글·대괄호·`_x`·`*x` 는 그대로 — 한셀 웹 저장 대조 (표시 형식 65가지)
fn canonical_code(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    // 따옴표·대괄호·`\x` 밖의 글자로 날짜·시간 서식인지 본다
    let date = {
        let mut d = false;
        let (mut q, mut b, mut i) = (false, false, 0);
        while i < chars.len() {
            let c = chars[i];
            match c {
                '"' => q = !q,
                '\\' if !q => i += 1,
                '[' if !q => b = true,
                ']' if !q => b = false,
                'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'm' | 'M' if !q && !b => d = true,
                _ => {}
            }
            i += 1;
        }
        d
    };
    let mut out = String::new();
    let mut percents = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                // 따옴표 글 그대로
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '"' {
                        break;
                    }
                    i += 1;
                }
            }
            '\\' => {
                out.push(c);
                if let Some(n) = chars.get(i + 1) {
                    out.push(*n);
                }
                i += 1;
            }
            '[' => {
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == ']' {
                        break;
                    }
                    i += 1;
                }
            }
            '_' | '*' => {
                out.push(c);
                if let Some(n) = chars.get(i + 1) {
                    out.push(*n);
                }
                i += 1;
            }
            'E' | 'e' if matches!(chars.get(i + 1), Some('+' | '-')) => {
                // 지수: E+ · E- 는 그대로
                out.push(c);
                out.push(chars[i + 1]);
                i += 1;
            }
            ';' => {
                percents = 0;
                out.push(c);
            }
            '%' => {
                percents += 1;
                if percents > 1 {
                    out.push('\\');
                }
                out.push(c);
            }
            ':' if !date => out.push_str("\\:"),
            '/' if date => {
                // AM/PM·A/P 의 `/` 는 그대로
                let rest: String = chars[i.saturating_sub(2)..(i + 3).min(chars.len())]
                    .iter()
                    .collect();
                let up = rest.to_ascii_uppercase();
                if up.contains("AM/PM") || up.contains("A/P") && !up.contains("AA/") {
                    out.push(c);
                } else {
                    out.push_str("\\/");
                }
            }
            ' ' | '-' | '+' | '=' | '<' | '>' | '&' | '\'' | '!' | '~' | '^' | '|' | '{' | '}'
            | '$' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            '\u{20a9}' => out.push_str("\"\u{20a9}\""),
            c if !c.is_ascii() && !c.is_alphabetic() => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// 한셀 안의 서식 → CSS `mso-number-format` 값 (`#,.;[]()@/` 와 `\` 앞에 `\`, `"` 는 `\0022`, 기호는 `\XXXX`)
fn mso_format(code: &str) -> String {
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return "General".into();
    }
    let mut out = String::new();
    let mut canon = canonical_code(code);
    // 조건 `[>100]` 하나뿐인 서식은 나머지를 일반으로 채운다
    if canon.starts_with('[')
        && !canon.contains(';')
        && canon
            .find(']')
            .is_some_and(|i| canon[1..i].starts_with(['<', '>', '=']))
    {
        canon.push_str(";General");
    }
    for c in canon.chars() {
        match c {
            '"' => out.push_str("\\0022"),
            '\\' => out.push_str("\\\\"),
            '#' | ',' | '.' | ';' | '[' | ']' | '(' | ')' | '@' | '/' => {
                out.push('\\');
                out.push(c);
            }
            c if !c.is_ascii() && !c.is_alphabetic() => {
                let _ = write!(out, "\\{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 칸 서식 → CSS 클래스
// ---------------------------------------------------------------------------

fn hex(c: Option<&Color>, theme: &crate::theme::Theme) -> Option<String> {
    c.and_then(|c| crate::theme::resolve(c, theme))
        .map(|s| s.to_ascii_lowercase())
}

/// 테두리 변 → `border-방향:굵기 모양 색` (없으면 None). 굵기·모양은 한셀 웹 저장 대조
fn border_css(side: &str, e: Option<&BorderEdge>, theme: &crate::theme::Theme) -> Option<String> {
    let e = e?;
    let (w, shape) = match e.style.as_deref()? {
        "none" => return None,
        "thin" => (".5pt", "solid"),
        "medium" => ("1.0pt", "solid"),
        "thick" => ("2.0pt", "solid"),
        "double" => ("2.0pt", "double"),
        "hair" => (".5pt", "hairline"),
        "dotted" => (".5pt", "dotted"),
        "dashed" => (".5pt", "dashed"),
        "dashDot" => (".5pt", "dot-dash"),
        "dashDotDot" => (".5pt", "dot-dot-dash"),
        "mediumDashed" => ("1.0pt", "dashed"),
        "mediumDashDot" => ("1.0pt", "dot-dash"),
        "mediumDashDotDot" => ("1.0pt", "dot-dot-dash"),
        "slantDashDot" => ("1.0pt", "dot-dash-slanted"),
        _ => (".5pt", "solid"),
    };
    let color = hex(e.color.as_ref(), theme).unwrap_or_else(|| "#000000".into());
    Some(format!("border-{side}:{w} {shape} {color};"))
}

/// 서식 코드 색 이름 → `#rrggbb`
fn color_name(c: &str) -> String {
    match c {
        "Red" => "#ff0000",
        "Blue" => "#0000ff",
        "Green" => "#00ff00",
        "Yellow" => "#ffff00",
        "Magenta" => "#ff00ff",
        "Cyan" => "#00ffff",
        "White" => "#ffffff",
        _ => "#000000",
    }
    .to_string()
}

/// 글꼴 클래스 (`.hclfontN`)의 속성: 글꼴과 일반 서식 (한셀 웹 저장 대조)
fn font_css_text(font: &Font, theme: &crate::theme::Theme) -> String {
    let mut s = font_props(font, theme);
    s.push_str("mso-number-format:\"General\";text-align:general;vertical-align:middle;white-space:nowrap;mso-text-control:wrap;");
    s
}

/// 글꼴의 CSS 속성 (색, 크기, 굵기, 기울임, 밑줄, 글꼴 이름)
fn font_props(font: &Font, theme: &crate::theme::Theme) -> String {
    let mut s = String::new();
    let color = hex(font.color.as_ref(), theme).unwrap_or_else(|| "#000000".into());
    let _ = write!(s, "color:{color};");
    let _ = write!(s, "font-size:{:.2}pt;", font.size.unwrap_or(11.0));
    let _ = write!(s, "font-weight:{};", if font.bold { 700 } else { 400 });
    s.push_str(if font.italic {
        "font-style:italic;"
    } else {
        "font-style:normal;"
    });
    match font.underline.as_deref().filter(|u| *u != "none") {
        Some(u) => {
            let kind = if u.starts_with("double") {
                "double"
            } else {
                "single"
            };
            let _ = write!(
                s,
                "text-decoration:underline;text-underline-style:{kind};text-underline-color:{color};"
            );
        }
        None => s.push_str("text-decoration:none;"),
    }
    let _ = write!(
        s,
        "font-family:\"{}\";",
        font.name.as_deref().unwrap_or("맑은 고딕")
    );
    s
}

/// 셀 서식 번호 → CSS (`.hcxN` 의 속성들, 한셀과 같은 차례)
/// `edges` 가 있으면 병합 범위의 오른쪽·아래 테두리를 그 서식 칸 것으로 바꿔 넣는다 (병합 파생 클래스)
fn xf_css(
    wb: &Workbook,
    xf_index: u32,
    theme: &crate::theme::Theme,
    edges: Option<(u32, u32)>,
) -> String {
    let st = &wb.styles;
    let xf = st.xf(xf_index).cloned().unwrap_or_default();
    let font: Font = st
        .fonts
        .get(xf.font_id as usize)
        .cloned()
        .unwrap_or_default();
    let mut s = font_props(&font, theme);
    let code = crate::numfmt::code_for(st, xf.num_fmt_id);
    let _ = write!(s, "mso-number-format:\"{}\";", mso_format(&code));
    if let Some(b) = st.borders.get(xf.border_id as usize) {
        let b: &Border = b;
        // 병합 파생: 오른쪽은 병합 범위 위쪽 끝 칸, 아래쪽은 왼쪽 끝 칸의 테두리
        let edge_of = |x: u32| {
            st.xf(x)
                .and_then(|x| st.borders.get(x.border_id as usize))
                .cloned()
        };
        let right = edges
            .and_then(|(r, _)| edge_of(r))
            .map_or(b.right.clone(), |e| e.right);
        let bottom = edges
            .and_then(|(_, d)| edge_of(d))
            .map_or(b.bottom.clone(), |e| e.bottom);
        for (side, e) in [
            ("top", &b.top),
            ("left", &b.left),
            ("bottom", &bottom),
            ("right", &right),
        ] {
            if let Some(x) = border_css(side, e.as_ref(), theme) {
                s.push_str(&x);
            }
        }
    }
    // 채우기: 단색은 앞색, 무늬(gray125 등)는 바탕색 (한셀 웹 저장 대조)
    if let Some(f) = st.fills.get(xf.fill_id as usize)
        && f.gradient.is_none()
    {
        let c = match f.pattern_type.as_deref() {
            Some("solid") => hex(f.fg_color.as_ref(), theme),
            Some("none") | None => None,
            Some(_) => hex(f.bg_color.as_ref(), theme),
        };
        if let Some(c) = c {
            let _ = write!(s, "background-color:{c};");
        }
    }
    let al = xf.alignment.clone().unwrap_or_default();
    let h = match al.horizontal.as_deref() {
        Some("left") => "left",
        Some("center") => "center",
        Some("right") => "right",
        Some("justify") => "justify",
        Some("centerContinuous") => "center-across",
        Some("fill") => "fill",
        _ => "general",
    };
    let v = match al.vertical.as_deref() {
        Some("top") => "top",
        Some("center" | "distributed" | "justify") => "middle",
        _ => "bottom",
    };
    let _ = write!(s, "text-align:{h};vertical-align:{v};");
    // 세로 맞춤 양쪽·균등도 줄 바꿈으로 나온다 (한셀 웹 저장 대조)
    s.push_str(
        if al.wrap_text || matches!(al.vertical.as_deref(), Some("justify" | "distributed")) {
            "white-space:normal;"
        } else {
            "white-space:nowrap;"
        },
    );
    s.push_str("mso-text-control:wrap;");
    s
}

/// 셀 서식 번호 → 클래스 번호: 기본 서식(0)은 15, 나머지는 20 + 스타일 서식 수 + 번호 (한셀 웹 저장 대조 —
/// 스타일 서식 1개면 21 + 번호, 3개면 23 + 번호)
fn class_of(wb: &Workbook, xf: u32) -> u32 {
    if xf == 0 {
        15
    } else {
        20 + wb.styles.cell_style_xfs.len().max(1) as u32 + xf
    }
}

/// 칸의 서식 번호: 칸 자신, 없으면 행 서식, 열 서식, 기본(0)
fn style_at(sheet: &Sheet, cells: &HashMap<(u32, u32), &Cell>, r: u32, c: u32) -> u32 {
    if let Some(cell) = cells.get(&(r, c)) {
        return cell.style;
    }
    let row = sheet
        .rows
        .iter()
        .find(|x| x.index == r + 1)
        .and_then(|x| x.style);
    let col = sheet
        .cols
        .iter()
        .find(|x| x.min <= c + 1 && c < x.max)
        .and_then(|x| x.style);
    row.or(col).unwrap_or(0)
}

/// 시트 쪽 CSS 에 쓰인 클래스: 번호, 서식 번호, 병합 파생이면 (오른쪽·아래 테두리를 가져올 서식)
struct UsedClass {
    cls: u32,
    xf: u32,
    edges: Option<(u32, u32)>,
}

/// 병합 파생 클래스: 번호와 (오른쪽 테두리를 가져올 서식, 아래쪽 테두리를 가져올 서식)
struct MergeClass {
    class: u32,
    edges: Option<(u32, u32)>,
}

/// 병합 칸의 파생 클래스 번호 (서식 번호 → 클래스 번호). 한셀은 병합 범위 안의 칸 서식마다 서식 표 끝(모든 서식 뒤)에
/// 복사본을 하나씩 만들고, 병합 시작 칸은 그 복사본의 클래스를 쓴다. 번호는 시트 차례대로, 시트 안에서는 병합 목록
/// 차례로, 범위 안에서는 행 우선으로 처음 나온 서식부터 매긴다 (한셀 웹 저장 28문서 병합 199곳이 모두 맞는다)
fn merge_classes(wb: &Workbook) -> HashMap<u32, MergeClass> {
    let mut map: HashMap<u32, MergeClass> = HashMap::new();
    let mut next =
        20 + wb.styles.cell_style_xfs.len().max(1) as u32 + wb.styles.cell_xfs.len() as u32;
    for sheet in &wb.sheets {
        if sheet.merges.is_empty() {
            continue;
        }
        let cells: HashMap<(u32, u32), &Cell> = sheet
            .cells
            .iter()
            .map(|c| ((c.r.row, c.r.col), c))
            .collect();
        for m in &sheet.merges {
            let area = (m.end.row - m.start.row + 1) as u64 * (m.end.col - m.start.col + 1) as u64;
            // 병합 시작 칸의 파생 클래스는 처음 그 서식으로 시작한 병합의 오른쪽 위·왼쪽 아래 칸 테두리를 쓴다
            let corner = (
                style_at(sheet, &cells, m.start.row, m.end.col),
                style_at(sheet, &cells, m.end.row, m.start.col),
            );
            let anchor = style_at(sheet, &cells, m.start.row, m.start.col);
            let mut take = |map: &mut HashMap<u32, MergeClass>, xf: u32| {
                map.entry(xf).or_insert_with(|| {
                    next += 1;
                    MergeClass {
                        class: next - 1,
                        edges: None,
                    }
                });
            };
            if area <= 100_000 {
                for r in m.start.row..=m.end.row {
                    for c in m.start.col..=m.end.col {
                        take(&mut map, style_at(sheet, &cells, r, c));
                    }
                }
            } else {
                // 아주 큰 병합은 있는 칸만 (행 우선), 나머지는 기본 서식으로 센다
                let mut inside: Vec<&Cell> = sheet
                    .cells
                    .iter()
                    .filter(|c| {
                        (m.start.row..=m.end.row).contains(&c.r.row)
                            && (m.start.col..=m.end.col).contains(&c.r.col)
                    })
                    .collect();
                inside.sort_by_key(|c| (c.r.row, c.r.col));
                take(&mut map, anchor);
                for c in inside {
                    take(&mut map, c.style);
                }
            }
            if let Some(e) = map.get_mut(&anchor)
                && e.edges.is_none()
            {
                e.edges = Some(corner);
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// 시트 쪽
// ---------------------------------------------------------------------------

/// 글꼴 비교 열쇠: 웹 쪽에 나오는 속성만 (한컴 확장은 뺀다)
fn font_key(f: &Font) -> String {
    // 한컴 확장은 기본값(크기·장평 100%, 자간·위치 0)이 아닌 것만
    let mut h = f.hancom.clone().unwrap_or_default();
    if h.size == Some(100) {
        h.size = None;
    }
    if h.ratio == Some(100) {
        h.ratio = None;
    }
    if h.spacing == Some(0) {
        h.spacing = None;
    }
    if h.offset == Some(0) {
        h.offset = None;
    }
    format!(
        "{:?}|{:?}|{}|{}|{:?}|{}|{:?}|{:?}|{:?}",
        f.name,
        f.size.map(|s| (s * 100.0).round() as i64),
        f.bold,
        f.italic,
        f.underline,
        f.strike,
        // 색을 정하지 않은 글꼴은 검정 (자동) 과 같다
        f.color.as_ref().map_or_else(
            || (Some("ff000000".to_string()), None, None, None, false),
            |c| (
                c.rgb.clone(),
                c.theme,
                c.tint.map(|t| (t * 1e6) as i64),
                c.indexed,
                c.auto
            )
        ),
        f.vert_align,
        h
    )
}

/// 글꼴 번호: 글꼴 목록에서 같은 글꼴을 합친 차례 (`hclfontN`)
struct FontNumbers {
    by_id: Vec<usize>,
    keys: Vec<String>,
}

impl FontNumbers {
    fn new(wb: &Workbook) -> FontNumbers {
        // 한셀 글꼴 표는 기본 글꼴 (맑은 고딕 11pt, 검정) 하나로 시작하고 (번호 0), 문서의 글꼴이 차례로 더해진다 — 같은 글꼴은
        // 합쳐진다. 한셀 프로세스 하나가 여러 문서를 열면 번호가 이어서 쌓이므로 문서마다 새 프로세스로 대조했다
        let default_font = Font {
            name: Some("맑은 고딕".into()),
            size: Some(11.0),
            color: Some(Color {
                rgb: Some("ff000000".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut keys: Vec<String> = vec![font_key(&default_font)];
        let by_id = wb
            .styles
            .fonts
            .iter()
            .map(|f| {
                let key = font_key(f);
                match keys.iter().position(|k| *k == key) {
                    Some(i) => i,
                    None => {
                        keys.push(key);
                        keys.len() - 1
                    }
                }
            })
            .collect();
        FontNumbers { by_id, keys }
    }
    /// 글꼴 목록 번호 → 글꼴 번호
    fn id(&self, id: u32) -> usize {
        self.by_id.get(id as usize).copied().unwrap_or(0)
    }
    /// 글꼴 값 → 글꼴 번호 (목록에 없는 글꼴은 맨 뒤에 더한 번호)
    fn of(&self, f: &Font) -> usize {
        let key = font_key(f);
        self.keys
            .iter()
            .position(|k| *k == key)
            .unwrap_or(self.keys.len())
    }
}

/// 칸 글을 `<font>` 들로: 줄마다 한 요소, 줄 바꿈은 `<br/>`
fn text_html(text: &str, class: &str) -> String {
    runs_html(&[(text.to_string(), class.to_string())])
}

/// 글 조각들(글, 클래스)을 `<font>` 들로: 조각 안의 줄마다 한 요소. 앞뒤 공백은 전체에서만 뗀다
fn runs_html(runs: &[(String, String)]) -> String {
    let mut out = String::new();
    let last = runs.len().saturating_sub(1);
    for (k, (text, class)) in runs.iter().enumerate() {
        let mut t = text.replace("\r\n", "\n").replace('\r', "\n");
        if k == 0 {
            t = t.trim_start().to_string();
        }
        if k == last {
            t = t.trim_end().to_string();
        }
        if t.is_empty() {
            // 공백뿐인 칸 글은 빈 <font> 하나 (한셀 웹 저장 대조)
            if runs.len() == 1 && !text.is_empty() {
                out.push_str(&format!("<font class=\"{class}\"></font>"));
            }
            continue;
        }
        let lines: Vec<&str> = t.split('\n').collect();
        for (i, l) in lines.iter().enumerate() {
            out.push_str(&format!("<font class=\"{class}\">{l}"));
            if i + 1 < lines.len() {
                out.push_str("<br/>\n");
            }
            out.push_str("</font>");
        }
    }
    out
}

/// 도형·차트 그림을 그릴 때 쓰는 것: 활성 시트의 배율과 (처음 쓸 때 읽는) 시스템 글꼴
struct PngCtx {
    zoom: f64,
    db: std::cell::OnceCell<crate::print::FontDb>,
}

struct SheetOut {
    html: String,
    images: Vec<(String, Vec<u8>)>,
}

/// 시트 쪽에 그릴 범위: 값·서식이 있는 칸, 병합, 그림의 시작 칸
fn extent(sheet: &Sheet) -> (u32, u32, u32) {
    let mut max_r = 0u32;
    let mut max_c = 0u32;
    let mut any = false;
    // 열 서식과 같은 서식뿐인 빈 칸은 쓴 칸으로 세지 않는다 (한셀 웹 저장 대조 — 열 서식 칸이 열 끝까지 있는 시트)
    let col_style = |col: u32| {
        sheet
            .cols
            .iter()
            .find(|x| x.min <= col + 1 && col < x.max)
            .and_then(|x| x.style)
            .unwrap_or(0)
    };
    for c in &sheet.cells {
        if !c.value.is_empty()
            || c.formula.is_some()
            || (c.style != 0 && c.style != col_style(c.r.col))
        {
            max_r = max_r.max(c.r.row);
            max_c = max_c.max(c.r.col);
            any = true;
        }
    }
    for m in &sheet.merges {
        max_r = max_r.max(m.end.row);
        max_c = max_c.max(m.end.col);
        any = true;
    }
    let _ = any;
    // 그리기 개체: 두 칸 앵커의 끝 칸 (한셀은 끝 표지의 칸까지 그린다). 열 줄(`<col>`)은 값이 있는 칸·병합과 너비를
    // 정했거나 숨기거나 개요가 있는 열 정보의 끝 열까지만 쓴다 (한셀 웹 저장 대조)
    let mut data_cols = 0u32;
    for c in &sheet.cells {
        if !c.value.is_empty() || c.formula.is_some() {
            data_cols = data_cols.max(c.r.col + 1);
        }
    }
    for m in &sheet.merges {
        data_cols = data_cols.max(m.end.col + 1);
    }
    for c in &sheet.cols {
        if c.custom_width || c.hidden || c.outline_level > 0 || c.collapsed {
            data_cols = data_cols.max(c.max.min(16384));
        }
    }
    let data_cols = data_cols.max(1);
    for sh in &sheet.shapes {
        let m = sh.anchor.to.unwrap_or(sh.anchor.from);
        max_r = max_r.max(m.row);
        max_c = max_c.max(m.col);
    }
    (max_r + 1, max_c + 1, data_cols)
}

#[allow(clippy::too_many_arguments)]
fn sheet_page(
    wb: &Workbook,
    sheet: &Sheet,
    theme: &crate::theme::Theme,
    fonts: &FontNumbers,
    merge_cls: &HashMap<u32, MergeClass>,
    png: &PngCtx,
    image_base: &mut u32,
    mht_base: Option<&str>,
) -> SheetOut {
    let (nrows, ncols, data_cols) = extent(sheet);
    // 칸 수가 터무니없이 큰 시트(멀리 떨어진 칸 하나)는 앞부분만 쓴다 — 한 쪽이 수 GB 가 되지 않게
    const MAX_CELLS: u32 = 4_000_000;
    let ncols = ncols.min(2048);
    // 열 줄(`<col>`)은 칸이 없는 끝 열 정보(너비를 정한 열)까지 쓰므로 너비는 그 열까지 잰다 (표 너비·칸은 ncols 까지)
    let data_cols = data_cols.min(2048);
    let gcols = ncols.max(data_cols);
    let nrows = nrows.min(MAX_CELLS / ncols).max(1);
    let geo = crate::render::sheet_geometry(wb, sheet, nrows, gcols);
    let col_exact: Vec<f64> = (0..gcols as usize)
        .map(|c| {
            if sheet
                .cols
                .iter()
                .any(|x| x.hidden && x.min <= c as u32 + 1 && (c as u32) < x.max)
            {
                0.0
            } else {
                geo.col_px.get(c).copied().unwrap_or(72.0)
            }
        })
        .collect();
    let col_w: Vec<u32> = col_exact.iter().map(|w| w.trunc() as u32).collect();
    // 열 너비는 트윕(1/15 px) 정수로 내림해서 더한다: 표 너비·병합 칸 너비 = 내림(트윕 합 / 15) (한셀 웹 저장 52시트 대조)
    let col_tw: Vec<f64> = col_exact
        .iter()
        .map(|w| (w * 15.0 + 1e-6).floor())
        .collect();
    let span_w = |a: u32, b: u32| -> u32 {
        (col_tw[a as usize..=b as usize].iter().sum::<f64>() / 15.0 + 1e-9).floor() as u32
    };
    // 행 높이(px) = 내림(pt × 4/3) + 1. 높이를 정하지 않은 행은 글에 맞춘 높이(값이 있는 행만)와 시트 기본 높이 가운데 큰 것.
    // (docs/format/other-formats.md 9절)
    let mut has_value: std::collections::HashSet<u32> = Default::default();
    // 행의 값 칸 가운데 가장 큰 글꼴의 한 줄 높이(pt)와 줄 간격(pt)
    let mut line_of: HashMap<u32, (f64, f64)> = HashMap::new();
    for c in &sheet.cells {
        if !c.value.is_empty() {
            has_value.insert(c.r.row);
            // 여러 줄이 되는 칸(줄 바꿈 서식이거나 글 안에 줄 바꿈)만
            let wraps = wb
                .styles
                .xf(c.style)
                .and_then(|x| x.alignment.as_ref())
                .is_some_and(|a| a.wrap_text)
                || matches!(&c.value, crate::model::CellValue::Text(t) if t.contains('\n'));
            if !wraps {
                continue;
            }
            let f = wb.styles.font_of(c.style);
            let m = crate::render::FontMetrics::new(
                f.and_then(|f| f.name.as_deref()),
                f.and_then(|f| f.size),
                f.is_some_and(|f| f.bold),
            );
            let e = line_of.entry(c.r.row).or_insert((0.0, 0.0));
            if m.row_pt > e.0 {
                *e = (m.row_pt, m.line_twips / 20.0);
            }
        }
    }
    // 행 높이: (px 소수, 더하는 px). 한 행은 내림(소수) + 더하는 값, 병합 칸은 행들의 소수 합을 내림해서 더한다
    let row_ex: Vec<(f64, u32)> = (0..nrows as usize)
        .map(|r| {
            let explicit = sheet
                .rows
                .iter()
                .find(|x| x.index == r as u32 + 1)
                .and_then(|x| x.height);
            let auto = geo
                .row_pt
                .get(r)
                .copied()
                .filter(|&h| h > 0.0 && has_value.contains(&(r as u32)));
            // 줄 바꿈으로 여러 줄이 된 행: 한셀 웹 저장은 줄마다 한 줄 높이(px)를 더한다 (6줄 → 132 = 6 × 22)
            if explicit.is_none()
                && let (Some(a), Some(&(nat, line))) = (auto, line_of.get(&(r as u32)))
                && line > 0.0
                && a > nat + 0.5 * line
            {
                let lines = ((a - nat) / line).round() as u32 + 1;
                return ((lines * (nat * 4.0 / 3.0).ceil() as u32) as f64, 0);
            }
            let pt = explicit
                .or_else(|| match (auto, sheet.default_row_height) {
                    (Some(a), Some(d)) => Some(a.max(d)),
                    (a, d) => a.or(d),
                })
                .unwrap_or(16.5);
            // 한셀 웹 저장: 내림(pt × 4/3) + 1 (12.75pt → 18, 19.5pt → 27 — 정수 px 가 되는 높이는 한 칸 더 커진다)
            (pt * 4.0 / 3.0, 1)
        })
        .collect();
    let row_h: Vec<u32> = row_ex
        .iter()
        .map(|&(e, p)| (e + 1e-9).floor() as u32 + p)
        .collect();
    let hidden_row = |r: u32| sheet.rows.iter().any(|x| x.index == r + 1 && x.hidden);
    let cell_at: HashMap<(u32, u32), &Cell> = sheet
        .cells
        .iter()
        .map(|c| ((c.r.row, c.r.col), c))
        .collect();
    // 병합: 시작 칸 → (끝 행, 끝 열), 덮인 칸
    let mut merge_end: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
    let mut covered: std::collections::HashSet<(u32, u32)> = Default::default();
    for m in &sheet.merges {
        merge_end.insert((m.start.row, m.start.col), (m.end.row, m.end.col));
        for r in m.start.row..=m.end.row {
            for c in m.start.col..=m.end.col {
                if (r, c) != (m.start.row, m.start.col) {
                    covered.insert((r, c));
                }
            }
        }
    }
    // 그림: 시작 칸 안쪽(왼쪽 위로 한 칸) → 목록
    let mut images: Vec<(String, Vec<u8>)> = Vec::new();
    let mut pics: HashMap<(u32, u32), Vec<String>> = HashMap::new();
    for sh in &sheet.shapes {
        let html = picture_span(
            wb,
            sh,
            &col_w,
            &row_h,
            png.zoom,
            image_base,
            &mut images,
            mht_base,
        )
        .or_else(|| shape_span(wb, sh, png, image_base, &mut images));
        let Some(mut html) = html else {
            continue;
        };
        // 병합에 덮인 칸이면 병합 시작 칸에 쓴다
        if covered.contains(&(html.0, html.1))
            && let Some(m) = sheet.merges.iter().find(|m| {
                (m.start.row..=m.end.row).contains(&html.0)
                    && (m.start.col..=m.end.col).contains(&html.1)
            })
        {
            html.0 = m.start.row;
            html.1 = m.start.col;
        }
        pics.entry((html.0, html.1)).or_default().push(html.2);
    }
    // 클래스: 쓰인 서식 번호 (작은 것부터)
    // 표 클래스: 첫 열의 서식 (열 서식이 없으면 기본)
    let table_xf = sheet
        .cols
        .iter()
        .min_by_key(|c| c.min)
        .filter(|c| c.min == 1)
        .and_then(|c| c.style)
        .unwrap_or(0);
    // 쓰인 클래스: (클래스 번호, 서식 번호) — 병합 시작 칸은 파생 번호
    let mut used: Vec<UsedClass> = vec![UsedClass {
        cls: class_of(wb, 0),
        xf: 0,
        edges: None,
    }];
    if table_xf != 0 {
        used.push(UsedClass {
            cls: class_of(wb, table_xf),
            xf: table_xf,
            edges: None,
        });
    }
    // 열 서식은 그 열의 칸이 없어도 클래스를 정의한다 (열 줄에 드는 열 정보까지 — 한셀 웹 저장 대조)
    for ci in sheet.cols.iter().filter(|c| c.min <= data_cols) {
        if let Some(xf) = ci.style.filter(|&x| x != 0)
            && !used.iter().any(|u| u.cls == class_of(wb, xf))
        {
            used.push(UsedClass {
                cls: class_of(wb, xf),
                xf,
                edges: None,
            });
        }
    }
    let mut body = String::new();
    // 부분 서식 글 (같은 글의 공유 문자열 조각), 조각 글꼴 번호 → 글꼴
    let rich: HashMap<&str, &crate::model::RichText> = wb
        .shared_strings
        .iter()
        .filter(|r| r.runs.len() > 1)
        .map(|r| (r.text.as_str(), r))
        .collect();
    let mut font_css: std::collections::BTreeMap<usize, Font> = Default::default();
    let style_of = |r: u32, c: u32| -> u32 { style_at(sheet, &cell_at, r, c) };
    // 표 너비는 열 너비(px 소수)의 합을 버린 값 (한셀 웹 저장 대조: 72.03 × 10 → 720)
    let total_w: u32 = span_w(0, ncols - 1);
    // 열 줄: 열 정보 하나(min–max)가 한 줄이고 열 정보가 없는 이어진 열은 한 줄 (한셀 웹 저장 대조).
    // 모든 열이 열 정보 없이 같은 너비면 쓰지 않는다
    let record_of = |c: usize| {
        sheet
            .cols
            .iter()
            .position(|x| x.min <= c as u32 + 1 && (c as u32) < x.max)
    };
    let dcw = &col_w[..(data_cols as usize).min(col_w.len())];
    let all_same = dcw.windows(2).all(|w| w[0] == w[1]) && !dcw.contains(&0);
    if !all_same {
        let mut k = 0usize;
        while k < dcw.len() {
            let mut j = k;
            while j + 1 < dcw.len() && record_of(j + 1) == record_of(k) {
                j += 1;
            }
            let w = col_w[k];
            let _ = writeln!(
                body,
                "      <col span=\"{}\" width=\"{w}\"{}/>",
                j - k + 1,
                if w == 0 { " style='display:none'" } else { "" }
            );
            k = j + 1;
        }
    }
    // 행 서식이 있는 행은 <tr> 에 클래스를 단다 (열 서식처럼 칸이 없어도 클래스를 정의한다)
    let row_style = |r: u32| {
        sheet
            .rows
            .iter()
            .find(|x| x.index == r + 1)
            .and_then(|x| x.style)
            .filter(|&x| x != 0)
    };
    let row_class = |r: u32| match row_style(r) {
        Some(xf) => format!(" class=\"hcx{}\"", class_of(wb, xf)),
        None => String::new(),
    };
    for r in 0..nrows {
        if let Some(xf) = row_style(r)
            && !used.iter().any(|u| u.cls == class_of(wb, xf))
        {
            used.push(UsedClass {
                cls: class_of(wb, xf),
                xf,
                edges: None,
            });
        }
        let hidden = hidden_row(r);
        let _ = writeln!(
            body,
            "      <tr{} height=\"{}\"{}>",
            row_class(r),
            if hidden { 0 } else { row_h[r as usize] },
            if hidden { " style='display:none'" } else { "" }
        );
        for c in 0..ncols {
            if covered.contains(&(r, c)) {
                continue;
            }
            let xf = style_of(r, c);
            let (er, ec) = merge_end.get(&(r, c)).copied().unwrap_or((r, c));
            let (cls, edges) = match merge_cls.get(&xf) {
                Some(m) if merge_end.contains_key(&(r, c)) => (m.class, m.edges),
                _ => (class_of(wb, xf), None),
            };
            if !used.iter().any(|u| u.cls == cls) {
                used.push(UsedClass { cls, xf, edges });
            }
            let w: u32 = span_w(c, ec);
            let h: u32 = if er == r {
                row_h[r as usize]
            } else {
                let sum: f64 = (r..=er).map(|k| row_ex[k as usize].0).sum();
                (sum + 1e-9).floor() as u32 + row_ex[r as usize].1
            };
            let mut attrs = format!("class=\"hcx{cls}\"");
            if ec > c {
                let _ = write!(attrs, " colspan=\"{}\"", ec - c + 1);
            }
            if er > r {
                let _ = write!(attrs, " rowspan=\"{}\"", er - r + 1);
            }
            let _ = write!(attrs, " width=\"{w}\" height=\"{h}\"");
            if w == 0 {
                attrs.push_str(" style='display:none'");
            }
            let _ = writeln!(body, "        <td {attrs}>");
            if let Some(list) = pics.get(&(r, c)) {
                for p in list {
                    body.push_str(p);
                    let _ = writeln!(
                        body,
                        "          <span class=\"hcx_t_s\" style='height:{}px; width:{}px; vertical-align:middle'>\n          </span>",
                        row_h[r as usize], col_w[c as usize]
                    );
                }
            }
            if let Some(cell) = cell_at.get(&(r, c)) {
                let txt = crate::csv::shown(wb, cell);
                // 표시 형식의 색 (`[Red]`)은 글에 `<font color>` 로 (한셀 웹 저장 대조)
                let code_color = match &cell.value {
                    crate::model::CellValue::Number(_) | crate::model::CellValue::Text(_) => {
                        let code = wb
                            .styles
                            .xf(cell.style)
                            .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
                            .unwrap_or_else(|| "General".into());
                        crate::numfmt::format(&cell.value, &code, wb.date1904).color
                    }
                    _ => None,
                };
                let font_no = wb.styles.xf(cell.style).map_or(0, |x| fonts.id(x.font_id));
                // 부분 서식이 있는 글은 조각마다 글꼴 (한셀은 같은 글의 공유 문자열 조각을 쓴다)
                let rich = match &cell.value {
                    crate::model::CellValue::Text(t) => rich.get(t.as_str()),
                    _ => None,
                };
                let inner = match rich {
                    Some(rt) => {
                        let runs: Vec<(String, String)> = rt
                            .runs
                            .iter()
                            .map(|r| {
                                let n = r.font.as_ref().map_or(font_no, |f| fonts.of(f));
                                if let Some(f) = &r.font {
                                    font_css.entry(n).or_insert_with(|| f.clone());
                                } else if let Some(f) = wb.styles.font_of(cell.style) {
                                    font_css.entry(n).or_insert_with(|| f.clone());
                                }
                                (r.text.clone(), format!("hclfont{n}"))
                            })
                            .collect();
                        runs_html(&runs)
                    }
                    None => text_html(&txt, &format!("hclfont{font_no}")),
                };
                let inner = match code_color.as_deref() {
                    Some(c) => inner.replace(
                        "<font class=",
                        &format!("<font color=\"{}\" class=", color_name(c)),
                    ),
                    None => inner,
                };
                let f = wb.styles.font_of(cell.style).cloned().unwrap_or_default();
                let wrap: Option<&str> = if inner.is_empty() {
                    None
                } else if f.strike {
                    Some("s")
                } else {
                    match f.vert_align.as_deref() {
                        Some("superscript") => Some("sup"),
                        Some("subscript") => Some("sub"),
                        _ => None,
                    }
                };
                match wrap {
                    Some(tag) => {
                        let _ = writeln!(body, "          <{tag}>");
                        body.push_str(&inner);
                        let _ = writeln!(body, "          </{tag}>");
                    }
                    None => body.push_str(&inner),
                }
                body.push_str("        </td>\n");
                continue;
            }
            body.push_str("        </td>\n");
        }
        body.push_str("      </tr>\n");
    }
    used.sort_by_key(|u| u.cls);
    let mut css = String::new();
    // 메모 글꼴: 메모마다 .hcx1, .hcx2 … (한셀 웹 저장 대조 — 메모 두 개 문서)
    let comments: Vec<&crate::model::Comment> = sheet.comments.iter().collect();
    for (i, c) in comments.iter().enumerate() {
        let f = c
            .text
            .runs
            .first()
            .and_then(|r| r.font.clone())
            .unwrap_or_default();
        let color = hex(f.color.as_ref(), theme).unwrap_or_else(|| "#000000".into());
        let _ = write!(
            css,
            ".hcx{} {{ color:{color};font-size:{:.2}pt;font-weight:{};font-style:{};text-decoration:none;font-family:\"{}\";mso-number-format:\"General\";text-align:general;vertical-align:middle;white-space:nowrap;mso-text-control:wrap; }}",
            i + 1,
            f.size.unwrap_or(9.0),
            if f.bold { 700 } else { 400 },
            if f.italic { "italic" } else { "normal" },
            f.name.as_deref().unwrap_or("굴림")
        );
    }
    for (n, f) in &font_css {
        let _ = write!(css, ".hclfont{n} {{ {} }}", font_css_text(f, theme));
    }
    for u in &used {
        let _ = write!(
            css,
            ".hcx{} {{ {} }}",
            u.cls,
            xf_css(wb, u.xf, theme, u.edges)
        );
    }
    if !comments.is_empty() {
        css.push_str(".memoa { font-size:8.00pt;font-weight:400;font-style:normal;text-decoration:none;font-family:\"굴림\"; }");
    }
    css.push_str(".hcx_t_s { display: table-cell; }body { margin:0; padding:0;}");
    let mut html = String::new();
    let _ = writeln!(html, "{XMLNS}");
    html.push_str("  <head>\n");
    if let Some(base) = mht_base {
        let _ = base;
    }
    html.push_str("    <meta charset=\"UTF-8\"/>\n    <style>\n");
    html.push_str(&css);
    html.push_str("    </style>\n  </head>\n  <body>\n");
    let _ = writeln!(
        html,
        "    <table class=\"hcx{}\" width=\"{total_w}\" cellspacing=\"0\" cellpadding=\"0\" border=\"0\" style='border-collapse:collapse; table-layout:fixed'>",
        class_of(wb, table_xf)
    );
    html.push_str(&body);
    html.push_str("    </table>\n");
    if !comments.is_empty() {
        html.push_str("    <hr align=\"left\" width=\"300\" size=\"1\">\n    </hr>\n    <div>\n");
        for (i, c) in comments.iter().enumerate() {
            let n = i + 1;
            let _ = writeln!(
                html,
                "      <div>\n        <a class=\"memoa\" href=\"#_hncmemo_{n}\">\n[{n}]        </a>\n        <span>\n          <div>"
            );
            html.push_str(&text_html(&c.text.text, &format!("hcx{n}")));
            html.push_str("          </div>\n        </span>\n      </div>\n");
        }
        html.push_str("    </div>\n");
    }
    html.push_str("  </body>\n</html>\n");
    SheetOut { html, images }
}

/// 그림 도형 → (그 칸 행, 열, `<span><img>` 글). 그림이 아니면 None
#[allow(clippy::too_many_arguments)]
fn picture_span(
    wb: &Workbook,
    sh: &Shape,
    col_w: &[u32],
    row_h: &[u32],
    zoom: f64,
    image_base: &mut u32,
    images: &mut Vec<(String, Vec<u8>)>,
    mht_base: Option<&str>,
) -> Option<(u32, u32, String)> {
    let _ = mht_base;
    if sh.kind != ShapeKind::Picture || sh.hidden {
        return None;
    }
    let target = sh.target.as_deref()?;
    let part = wb.raw_parts.iter().find(|p| p.name == target)?;
    let info = crate::image::info(&part.data)?;
    let (ext, _) = crate::image::ext_and_type(info.format);
    // 크기 (px): 두 칸 앵커는 끝 표지까지, 아니면 크기
    let a = &sh.anchor;
    let px = |emu: i64| (emu as f64 / 9525.0).round() as i64;
    let x_of = |c: u32, off: i64| -> i64 {
        col_w
            .iter()
            .take(c as usize)
            .map(|&w| w as i64)
            .sum::<i64>()
            + px(off)
    };
    let y_of = |r: u32, off: i64| -> i64 {
        row_h
            .iter()
            .take(r as usize)
            .map(|&h| h as i64)
            .sum::<i64>()
            + px(off)
    };
    let (x, y) = (
        x_of(a.from.col, a.from.col_off),
        y_of(a.from.row, a.from.row_off),
    );
    let (w, h) = match (a.kind, a.to, a.ext) {
        (AnchorKind::TwoCell, Some(t), _) => {
            (x_of(t.col, t.col_off) - x, y_of(t.row, t.row_off) - y)
        }
        (_, _, Some((cx, cy))) => (px(cx), px(cy)),
        _ => (info.width as i64, info.height as i64),
    };
    // 그림은 시작 칸 안에 쓰고, 여백은 시작 칸의 너비·높이 + 칸 안 오프셋 (한셀 웹 저장 대조 — 그림 2개)
    let (cr, cc) = (a.from.row, a.from.col);
    let cell_w = col_w.get(cc as usize).copied().unwrap_or(72) as i64;
    let cell_h = row_h.get(cr as usize).copied().unwrap_or(22) as i64;
    // 한셀은 그림도 화면 크기의 RGBA PNG 로 다시 그려 넣는다. PNG·BMP 는 그렇게 하고, 풀 수 없는 형식(JPEG)은 원본을 쓴다
    let (w, h) = (
        (w as f64 * zoom).round() as i64,
        (h as f64 * zoom).round() as i64,
    );
    *image_base += 1;
    let redrawn = if w > 0 && h > 0 && w * h <= 16_000_000 {
        crate::print::picture_png(&part.data, w as u32, h as u32)
    } else {
        None
    };
    let (name, bytes) = match redrawn {
        Some(b) => (format!("img{}.png", *image_base), b),
        None => (format!("img{}.{ext}", *image_base), part.data.clone()),
    };
    images.push((name.clone(), bytes));
    let html = format!(
        "          <span style='position:absolute; margin-top:{}px; margin-left:{}px; width:{w}px; height:{h}px'>\n            <img src=\"{name}\" height=\"{h}\" width=\"{w}\"/>\n          </span>\n",
        cell_h + px(a.from.row_off),
        cell_w + px(a.from.col_off)
    );
    let _ = (x, y);
    Some((cr, cc, html))
}

/// 도형·차트 → (시작 칸 행, 열, `<span><img>` 글). 한셀은 도형·차트를 PNG 그림으로 넣는다: 그림 크기 = 개체 크기(px, 내림)
/// (그림자가 있으면 한 줄 더), 화면 크기·여백 = 개체 크기·자리(xfrm) × 활성 시트 배율. 여백이 시작 칸의 어디에서 재는지는
/// 첫 열에 시작하는 개체만 맞는다 (docs/format/other-formats.md 9절)
fn shape_span(
    wb: &Workbook,
    sh: &Shape,
    png: &PngCtx,
    image_base: &mut u32,
    images: &mut Vec<(String, Vec<u8>)>,
) -> Option<(u32, u32, String)> {
    if sh.kind == ShapeKind::Picture {
        return None;
    }
    if sh.hidden {
        // 한셀은 숨긴 도형(텍스트 상자)도 투명한 23×27 그림으로 쓴다 (첫 칸, 여백 1)
        if !matches!(sh.kind, ShapeKind::Shape) {
            return None;
        }
        let bytes = crate::print::transparent_png(23, 27);
        *image_base += 1;
        let name = format!("img{}.png", *image_base);
        images.push((name.clone(), bytes));
        let html = format!(
            "          <span style='position:absolute; margin-top:1px; margin-left:1px; width:23px; height:27px'>
            <img src=\"{name}\" height=\"27\" width=\"23\"/>
          </span>
"
        );
        return Some((0, 0, html));
    }
    let xf = sh.xfrm.as_ref()?;
    let (w, h) = (xf.cx as f64 / 9525.0, xf.cy as f64 / 9525.0);
    // 그림 크기 = 개체 크기를 반올림한 px, 테두리선·그림자(·차트 틀)가 있으면 한 줄·한 칸 더 (한셀 웹 저장 대조)
    let (iw, ih) = ((w.round() as u32).max(1), (h.round() as u32).max(1));
    let has_line = sh
        .line
        .as_ref()
        .is_some_and(|l| !matches!(l.fill, Some(Paint::None)) && l.width != Some(0));
    // 차트 틀: 차트 영역 선을 정하지 않았거나(기본 회색 틀) 보이는 선이면
    let chart_frame = sh.chart.as_ref().is_some_and(|c| {
        c.sp_pr.as_ref().is_none_or(|sp| {
            !matches!(
                sp.line.as_ref().and_then(|l| l.fill.as_ref()),
                Some(crate::model::ChartFill::None)
            )
        })
    });
    let extra = u32::from(sh.shadow.is_some() || has_line || chart_frame);
    let (pw, ph) = (iw + extra, ih + extra);
    if pw as u64 * ph as u64 > 16_000_000 {
        return None;
    }
    let z = png.zoom;
    let (sw, shh) = ((w * z).round() as i64, (h * z).round() as i64);
    let a = &sh.anchor;
    let absolute = a.kind == AnchorKind::Absolute || a.edit_as.as_deref() == Some("absolute");
    let (ml, mt) = if absolute {
        (1, 1)
    } else {
        (
            (xf.x as f64 / 9525.0 * z).round() as i64,
            (xf.y as f64 / 9525.0 * z).round() as i64,
        )
    };
    let db = png.db.get_or_init(crate::print::FontDb::system);
    let bytes = crate::print::shape_png(wb, sh, (w, h), (pw, ph), db);
    *image_base += 1;
    let name = format!("img{}.png", *image_base);
    images.push((name.clone(), bytes));
    let html = format!(
        "          <span style='position:absolute; margin-top:{mt}px; margin-left:{ml}px; width:{sw}px; height:{shh}px'>
            <img src=\"{name}\" height=\"{shh}\" width=\"{sw}\"/>
          </span>
"
    );
    let (r, c) = if absolute {
        (0, 0)
    } else {
        (a.from.row, a.from.col)
    };
    Some((r, c, html))
}

// ---------------------------------------------------------------------------
// 묶음 쪽·시트 탭
// ---------------------------------------------------------------------------

fn pages(wb: &Workbook, files_dir: &str, mht_base: Option<&str>) -> Pages {
    let theme = wb.theme.clone().unwrap_or_default();
    let fonts = FontNumbers::new(wb);
    let mut image_base = 29000u32;
    let merge_cls = merge_classes(wb);
    // 도형·차트·그림의 크기와 여백은 활성 시트의 배율을 따른다 (한셀 웹 저장 대조: 배율 90 → 0.9배)
    let active_zoom = wb
        .sheets
        .iter()
        .find(|s| s.view.tab_selected)
        .or_else(|| wb.sheets.first())
        .and_then(|s| s.view.zoom_scale)
        .unwrap_or(100);
    let png = PngCtx {
        zoom: active_zoom as f64 / 100.0,
        db: std::cell::OnceCell::new(),
    };
    let mut sheets = Vec::new();
    let mut images = Vec::new();
    for (i, sh) in wb.sheets.iter().enumerate() {
        if sh.state != SheetState::Visible {
            continue;
        }
        let out = sheet_page(
            wb,
            sh,
            &theme,
            &fonts,
            &merge_cls,
            &png,
            &mut image_base,
            mht_base,
        );
        sheets.push((format!("sheet{:03}.htm", i + 1), out.html));
        images.extend(out.images);
    }
    let active = wb
        .sheets
        .iter()
        .position(|s| s.view.tab_selected)
        .unwrap_or(0);
    let first_visible = sheets.first().map(|s| s.0.clone()).unwrap_or_default();
    let active_file = format!("sheet{:03}.htm", active + 1);
    let frame = if sheets.iter().any(|s| s.0 == active_file) {
        active_file
    } else {
        first_visible
    };
    let mut main = String::new();
    let _ = writeln!(main, "{XMLNS}");
    main.push_str("  <head>\n    <meta name=\"Excel Workbook Frameset\"/>\n    <meta charset=\"UTF-8\"/>\n<!--[if gte mso 9]>    <xml>\n      <x:ExcelWorkbook>\n        <x:ExcelWorksheets>\n");
    for (i, sh) in wb.sheets.iter().enumerate() {
        let _ = writeln!(
            main,
            "          <x:ExcelWorksheet>\n            <x:Name>{}</x:Name>\n            <x:WorksheetSource HRef=\"{files_dir}\\sheet{:03}.htm\"/>\n          </x:ExcelWorksheet>",
            sh.name,
            i + 1
        );
    }
    let _ = writeln!(
        main,
        "        </x:ExcelWorksheets>\n        <x:ActiveSheet>{active}</x:ActiveSheet>\n      </x:ExcelWorkbook>\n    </xml>\n<![endif]-->  </head>\n  <frameset width=\"0\" border=\"0\" rows=\"*,50\" frameborder=\"0\" framespacing=\"0\">\n    <frame src=\"{files_dir}\\{frame}\" name=\"frame_sheet\"/>\n    <frame src=\"{files_dir}\\tabs.htm\" name=\"frame_tabs\"/>\n    <noframes>\n      <body>\n      </body>\n    </noframes>\n  </frameset>\n</html>"
    );
    let mut tabs = String::from(
        "<html lang=\"ko\">\n  <head>\n    <meta charset=\"UTF-8\"/>\n    <style>\nbody {margin:0px;background-color:#EAE5FA;}.tab_btn {float:left;background-color:#FFFFFF;margin:0.5px;padding:2px;border:1px solid #000000;font-size:9pt;color:#000000;}    </style>\n  </head>\n  <body>\n    <div>\n",
    );
    for (i, sh) in wb.sheets.iter().enumerate() {
        if sh.state != SheetState::Visible {
            continue;
        }
        let _ = writeln!(
            tabs,
            "      <div class=\"tab_btn\">\n        <a href=\".\\sheet{:03}.htm\" target=\"frame_sheet\">\n{}        </a>\n      </div>",
            i + 1,
            sh.name
        );
    }
    tabs.push_str("    </div>\n  </body>\n</html>\n");
    Pages {
        main,
        sheets,
        tabs,
        images,
    }
}

/// `.htm` 로 쓸 파일들: `이름.htm`, `이름.files/sheetNNN.htm`, `이름.files/tabs.htm`, `이름.files/imgN.png`.
/// 한셀과 같게 줄 끝은 `\n`
pub fn write_htm(wb: &Workbook, stem: &str) -> Vec<WebFile> {
    let dir = format!("{stem}.files");
    let p = pages(wb, &dir, None);
    let mut out = vec![WebFile {
        path: format!("{stem}.htm"),
        data: p.main.into_bytes(),
    }];
    for (n, h) in p.sheets {
        out.push(WebFile {
            path: format!("{dir}/{n}"),
            data: h.into_bytes(),
        });
    }
    out.push(WebFile {
        path: format!("{dir}/tabs.htm"),
        data: p.tabs.into_bytes(),
    });
    for (n, d) in p.images {
        out.push(WebFile {
            path: format!("{dir}/{n}"),
            data: d,
        });
    }
    out
}

/// `.htm` 을 파일로: `path` 에 묶음 쪽, 같은 폴더의 `이름.files/` 에 시트·탭·그림
pub fn save_htm(wb: &Workbook, path: &std::path::Path) -> crate::Result<()> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sheet")
        .to_string();
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    for f in write_htm(wb, &stem) {
        let to = dir.join(&f.path);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(to, f.data)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// .mht
// ---------------------------------------------------------------------------

/// quoted-printable: `=` 만 `=3D` 로, 글자(유니코드 한 글자)를 세어 73 이 되면 `=\r\n` 으로 접는다. 줄 바꿈(`\n`)은 그대로
fn qp(text: &str) -> String {
    let mut out = String::new();
    let mut len = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            out.push('\n');
            len = 0;
            continue;
        }
        if ch == '=' {
            out.push_str("=3D");
            len += 3;
        } else {
            out.push(ch);
            len += 1;
        }
        if len >= 73 {
            out.push_str("=\r\n");
            len = 0;
        }
    }
    out
}

/// base64: 72 글자씩 `\n`, 마지막 줄은 `\r\n`
fn base64_lines(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for ch in data.chunks(3) {
        let n = (ch[0] as u32) << 16
            | (*ch.get(1).unwrap_or(&0) as u32) << 8
            | *ch.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if ch.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if ch.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    let mut out = String::new();
    let chunks: Vec<&str> = s
        .as_bytes()
        .chunks(72)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();
    for (i, c) in chunks.iter().enumerate() {
        out.push_str(c);
        out.push_str(if i + 1 == chunks.len() { "\r\n" } else { "\n" });
    }
    out
}

/// 문서에서 정한 이름들 (한셀은 쓸 때마다 무작위 — rcell 은 내용에서 정해 같은 문서는 같은 파일이 된다)
fn mht_names(wb: &Workbook) -> (String, String, String) {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in &wb.sheets {
        for b in s.name.bytes().chain((s.cells.len() as u32).to_le_bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    let g = format!(
        "{:016X}{:016X}",
        h,
        h.rotate_left(23) ^ 0x9e37_79b9_7f4a_7c15
    );
    let guid = format!("{}-{}-{}", &g[0..8], &g[8..20], &g[20..32]);
    let file = format!("0000{:08x}", (h >> 16) as u32);
    let boundary = format!("----=_NextPart_{}.{}", &g[0..8], &g[8..16]);
    (guid, file, boundary)
}

/// `.mht` 바이트
pub fn write_mht(wb: &Workbook) -> Vec<u8> {
    let (guid, file, boundary) = mht_names(wb);
    let dir = format!("{file}.files");
    let p = pages(wb, &dir, None);
    let loc = |name: &str| format!("file:///C:\\{guid}\\{name}");
    // 쪽마다 머리(`<head>` 다음 줄)에 `<base>` 를 넣는다
    let with_base = |html: &str, name: &str| {
        let tag = format!("    <![if IE]><base href=\"{}\"><![endif]>\n", loc(name));
        match html.find("<head>\n") {
            Some(i) => {
                let at = i + "<head>\n".len();
                format!("{}{}{}", &html[..at], tag, &html[at..])
            }
            None => html.to_string(),
        }
    };
    let mut out = String::new();
    let _ = write!(
        out,
        "Mime-Version:1.0\r\nX-Document-Type:workbook\r\nContent-Type:multipart/related; boundary=\"{boundary}\"\r\n\r\n"
    );
    let mut part = |loc_name: &str, kind: &str, body: String| {
        let _ = write!(
            out,
            "--{boundary}\r\nContent-Location:{}\r\n",
            loc(loc_name)
        );
        out.push_str(kind);
        out.push_str("\r\n");
        out.push_str(&body);
        out.push_str("\r\n");
    };
    let html_kind = "Content-Transfer-Encoding:quoted-printable\r\nContent-Type:text/html; charset=\"UTF-8\"\r\n";
    part(
        &format!("{file}.htm"),
        html_kind,
        qp(&with_base(&p.main, &format!("{file}.htm"))),
    );
    for (n, d) in &p.images {
        let ty = n
            .rsplit('.')
            .next()
            .map(|e| match e {
                "png" => "image/png",
                "jpeg" | "jpg" => "image/jpeg",
                "gif" => "image/gif",
                "bmp" => "image/bmp",
                _ => "application/octet-stream",
            })
            .unwrap_or("application/octet-stream");
        part(
            &format!("{dir}\\{n}"),
            &format!("Content-Transfer-Encoding:base64\r\nContent-Type:{ty}\r\n"),
            base64_lines(d),
        );
    }
    for (n, h) in &p.sheets {
        part(
            &format!("{dir}\\{n}"),
            html_kind,
            qp(&with_base(h, &format!("{dir}\\{n}"))),
        );
    }
    part(
        &format!("{dir}\\tabs.htm"),
        html_kind,
        qp(&with_base(&p.tabs, &format!("{dir}\\tabs.htm"))),
    );
    let _ = write!(out, "--{boundary}--\r\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CellRef;

    #[test]
    fn mso_formats_match_hcell() {
        // 한셀 웹 저장(표시 형식 65가지)에서 얻은 값
        for (code, want) in [
            ("General", "General"),
            ("0.00", "0\\.00"),
            ("#,##0", "\\#\\,\\#\\#0"),
            ("0%", "0%"),
            ("0.00E+00", "0\\.00E+00"),
            ("\"$\"#,##0.00", "\\0022$\\0022\\#\\,\\#\\#0\\.00"),
            ("\"\u{20a9}\"#,##0", "\\0022\\20A9\\0022\\#\\,\\#\\#0"),
            ("\u{20a9}#,##0", "\\0022\\20A9\\0022\\#\\,\\#\\#0"),
            (
                "#,##0;[Red]-#,##0",
                "\\#\\,\\#\\#0\\;\\[Red\\]\\\\-\\#\\,\\#\\#0",
            ),
            ("0_);(0)", "0_\\)\\;\\\\\\(0\\\\\\)"),
            ("@", "\\@"),
            ("# ?/?", "\\#\\\\ ?\\/?"),
            ("yyyy-mm-dd", "yyyy\\\\-mm\\\\-dd"),
            ("yyyy/mm/dd", "yyyy\\\\\\/mm\\\\\\/dd"),
            ("h:mm AM/PM", "h:mm\\\\ AM\\/PM"),
            ("[h]:mm", "\\[h\\]:mm"),
            ("0.0 \"kg\"", "0\\.0\\\\ \\0022kg\\0022"),
            ("[$-409]d-mmm-yy", "\\[$-409\\]d\\\\-mmm\\\\-yy"),
            ("0:0", "0\\\\:0"),
            ("0+0", "0\\\\+0"),
            ("$#,##0", "\\\\$\\#\\,\\#\\#0"),
            ("#,##0 \u{20ac}", "\\#\\,\\#\\#0\\\\ \\\\\\20AC"),
            ("0*x0", "0*x0"),
            ("0%%", "0%\\\\%"),
            (
                "yyyy\"\u{b144}\" m\"\u{c6d4}\" d\"\u{c77c}\"",
                "yyyy\\0022\u{b144}\\0022\\\\ m\\0022\u{c6d4}\\0022\\\\ d\\0022\u{c77c}\\0022",
            ),
        ] {
            assert_eq!(mso_format(code), want, "{code}");
        }
    }

    #[test]
    fn quoted_printable_folds_like_hcell() {
        let line = "<html xmlns:x=\"urn:schemas-microsoft-com:office:excel\" xmlns:v=\"urn:schemas-microsoft-com:vml\">\n";
        let q = qp(line);
        assert!(q.starts_with(
            "<html xmlns:x=3D\"urn:schemas-microsoft-com:office:excel\" xmlns:v=3D\"urn:s=\r\n"
        ));
        assert!(q.ends_with("\n"));
    }

    #[test]
    fn writes_frameset_and_sheet() {
        let mut wb = crate::edit::new_workbook();
        crate::edit::set_input(&mut wb, 0, CellRef::parse("A1").unwrap(), "가나").unwrap();
        crate::edit::set_input(&mut wb, 0, CellRef::parse("B1").unwrap(), "1234.5").unwrap();
        let files = write_htm(&wb, "x");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["x.htm", "x.files/sheet001.htm", "x.files/tabs.htm"]);
        let sheet = String::from_utf8(files[1].data.clone()).unwrap();
        assert!(sheet.contains("<font class=\"hclfont0\">가나</font>"));
        assert!(sheet.contains("<font class=\"hclfont0\">1234.5</font>"));
        let mht = String::from_utf8(write_mht(&wb)).unwrap();
        assert!(mht.starts_with("Mime-Version:1.0\r\nX-Document-Type:workbook\r\n"));
        assert!(mht.trim_end().ends_with("--"));
    }
}
