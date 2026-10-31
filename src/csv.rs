// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! CSV·텍스트 파일 — 한셀의 텍스트 열기와 CSV(`6`)·텍스트(`-4158`, 탭 구분)·PRN(`36`, 공백 구분) 저장.
//!
//! 한셀 COM 대조 실험에서 확인한 규칙이다 (docs/format/other-formats.md 3절).
//!
//! 열기: 인코딩은 BOM(UTF-8, UTF-16 LE/BE) → BOM 이 없으면 파일 전체가 올바른 UTF-8 이면 UTF-8, 아니면 코드
//! 페이지 949. NUL 에서 글이 끝난다. 확장자가 `.csv` 면 쉼표, 그 밖(`.txt` `.tsv` `.out`)은 탭으로 나눈다.
//! 따옴표로 시작한 필드는 닫는 따옴표까지 (구분 문자·줄 바꿈 그대로, `""` 는 `"`), 닫은 뒤의 글은 이어 붙이고,
//! 닫지 않으면 파일 끝까지. 필드 중간의 따옴표는 글자. 빈 줄은 빈 행. 값은 셀 입력처럼 읽는다 (숫자·날짜·
//! 시각·백분율·논리값·오류·`=수식` — 따옴표로 싸도 같다). 시트 이름은 파일 이름.
//!
//! 저장: 활성 시트 하나를 코드 페이지 949 로, 줄마다 CRLF. 칸은 화면에 보이는 글(표시 형식을 적용한 글).
//! 셀 레코드가 있는 행만 쓰고(빈 행은 건너뛴다), 열은 CSV 는 A 열부터, 텍스트·PRN 은 셀이 있는 첫 열부터
//! 셀이 있는 마지막 열까지. 구분 문자·따옴표·줄 바꿈이 든 칸은 따옴표로 싼다.

use std::collections::HashMap;
use std::path::Path;

use crate::calc::cp949;
use crate::edit::input::{self, Input};
use crate::model::{Cell, CellRef, CellValue, Formula, Sheet, SheetKind, Workbook};

/// 칸 구분 문자
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    /// 쉼표 — CSV (`.csv`, 한셀 FileFormat 6)
    Comma,
    /// 탭 — 텍스트 (`.txt`, 한셀 FileFormat -4158 xlText)
    Tab,
    /// 공백 — PRN (`.prn`, 한셀 FileFormat 36 xlTextPrinter. Excel 과 달리 고정 폭이 아니다)
    Space,
}

impl Delimiter {
    pub fn as_char(self) -> char {
        match self {
            Delimiter::Comma => ',',
            Delimiter::Tab => '\t',
            Delimiter::Space => ' ',
        }
    }

    /// 파일 이름의 확장자로: `.csv` 쉼표, `.txt` `.tsv` `.out` 탭, `.prn` 공백
    pub fn from_path(path: &Path) -> Option<Delimiter> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "csv" => Some(Delimiter::Comma),
            // SYLK·DIF 는 한셀도 형식으로 읽지 않고 텍스트로 연다 (한 줄이 한 칸)
            "txt" | "tsv" | "out" | "slk" | "dif" => Some(Delimiter::Tab),
            "prn" => Some(Delimiter::Space),
            _ => None,
        }
    }
}

/// 따옴표로 싸지 않은 필드는 이 길이(글자)마다 잘라 나머지를 다음 행 A 열부터 (한셀 관찰 — 딱 이 길이여도 다음 행으로 넘어간다)
const MAX_PLAIN: usize = 32759;
/// 따옴표로 싼 필드의 최대 길이 (글자). 넘는 글은 버리고, 그 줄의 다음 필드는 한 행 아래로 (한셀 관찰)
const MAX_QUOTED: usize = 32767;
const MAX_ROWS: u32 = 1_048_576;
const MAX_COLS: u32 = 16_384;

/// 파일 바이트 → 글 (BOM → UTF-8 → 코드 페이지 949, NUL 에서 끝)
pub fn decode(bytes: &[u8]) -> String {
    let utf16 = |b: &[u8], be: bool| -> String {
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if be {
                    u16::from_be_bytes(*c)
                } else {
                    u16::from_le_bytes(*c)
                }
            })
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let end = b.iter().position(|&x| x == 0).unwrap_or(b.len());
        return String::from_utf8_lossy(&b[..end]).into_owned();
    }
    if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(b, false);
    }
    if let Some(b) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(b, true);
    }
    let end = bytes.iter().position(|&x| x == 0).unwrap_or(bytes.len());
    let b = &bytes[..end];
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => cp949::decode(b),
    }
}

/// 글을 칸으로 나눈다 → (행, 열, 글). 빈 필드는 빼고 돌려준다
pub fn split(text: &str, delim: Delimiter) -> Vec<(u32, u32, String)> {
    split_with(text, &[delim.as_char()])
}

/// [`split`] 의 구분 문자 여럿 판
pub fn split_with(text: &str, delims: &[char]) -> Vec<(u32, u32, String)> {
    let ch: Vec<char> = text.chars().collect();
    let n = ch.len();
    let mut out = Vec::new();
    let (mut row, mut col) = (0u32, 0u32);
    let mut i = 0;
    let is_eol = |c: char| c == '\r' || c == '\n';
    while i < n {
        // 필드 하나
        let mut buf = String::new();
        if ch[i] == '"' {
            i += 1;
            let mut len = 0usize;
            while i < n {
                if ch[i] == '"' {
                    if i + 1 < n && ch[i + 1] == '"' {
                        if len < MAX_QUOTED {
                            buf.push('"');
                        }
                        len += 1;
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                if len < MAX_QUOTED {
                    buf.push(ch[i]);
                }
                len += 1;
                i += 1;
            }
            let quoted_over = len > MAX_QUOTED;
            // 닫은 뒤의 글은 구분 문자·줄 끝까지 이어 붙인다
            while i < n && !delims.contains(&ch[i]) && !is_eol(ch[i]) {
                buf.push(ch[i]);
                i += 1;
            }
            if !buf.is_empty() {
                out.push((row, col, buf));
            }
            if quoted_over {
                row += 1;
            }
        } else {
            let start = i;
            while i < n && !delims.contains(&ch[i]) && !is_eol(ch[i]) {
                i += 1;
            }
            let mut field = &ch[start..i];
            while field.len() >= MAX_PLAIN {
                out.push((row, col, field[..MAX_PLAIN].iter().collect()));
                field = &field[MAX_PLAIN..];
                row += 1;
                col = 0;
            }
            if !field.is_empty() {
                out.push((row, col, field.iter().collect()));
            }
        }
        // 구분 문자면 다음 필드, 줄 끝이면 다음 행
        if i < n && delims.contains(&ch[i]) {
            i += 1;
            col += 1;
            if i == n {
                break;
            }
            continue;
        }
        if i < n && ch[i] == '\r' {
            i += 1;
            if i < n && ch[i] == '\n' {
                i += 1;
            }
        } else if i < n && ch[i] == '\n' {
            i += 1;
        }
        row += 1;
        col = 0;
    }
    out.retain(|(r, c, _)| *r < MAX_ROWS && *c < MAX_COLS);
    out
}

/// 시트 이름: 파일 이름에서 확장자를 뺀 것 (31자까지, 시트 이름에 쓸 수 없는 글자는 `_`)
pub fn sheet_name_for(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Sheet1");
    let name: String = stem
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .take(31)
        .collect();
    if name.is_empty() {
        "Sheet1".into()
    } else {
        name
    }
}

/// 텍스트 파일 바이트에서 통합 문서를 만든다 (한셀이 CSV·텍스트 파일을 여는 것처럼)
pub fn read(bytes: &[u8], delim: Delimiter, sheet_name: &str) -> crate::Result<Workbook> {
    read_with(bytes, &TextOptions::for_delimiter(delim), sheet_name)
}

/// 텍스트 가져오기 선택 (한셀 `Workbooks.OpenText` 와 대화 상자의 선택): 구분 문자 여럿, 따옴표 문자, 글자 열, 시작 행.
/// 한셀 관찰 (docs/format/other-formats.md 3.3절): 선택한 구분 문자 모두로 나눈다, 선택이 없으면 확장자의 기본,
/// "그 밖의 문자" 는 첫 글자만, 연속 구분 문자 합치기·열 건너뛰기·고정 폭은 한셀이 무시한다
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextOptions {
    /// 칸 구분 문자들
    pub delimiters: Vec<char>,
    /// 글자로 읽을 열 (0부터): 값 해석 없이 글 + 글자 서식 `@`
    pub text_columns: Vec<u32>,
    /// 첫 줄을 놓을 행 (0부터)
    pub start_row: u32,
}

impl TextOptions {
    pub fn for_delimiter(d: Delimiter) -> TextOptions {
        TextOptions {
            delimiters: vec![d.as_char()],
            text_columns: Vec::new(),
            start_row: 0,
        }
    }
}

/// [`read`] 에 가져오기 선택을 더한 것
pub fn read_with(bytes: &[u8], opts: &TextOptions, sheet_name: &str) -> crate::Result<Workbook> {
    let text = decode(bytes);
    let mut wb = crate::edit::new_workbook();
    wb.application = Some("rcell".into());
    let name = if sheet_name.is_empty() {
        "Sheet1"
    } else {
        sheet_name
    };
    wb.sheets[0].name = name.to_string();
    let mut styles: StyleCache = HashMap::new();
    // 칸은 파일 차례(행 우선)로 나오므로 시트에 차례로 붙인다 — 수식의 표시 형식은 앞에 읽은 칸을 본다
    let mut last: Option<(u32, u32)> = None;
    for (row, col, field) in split_with(&text, &opts.delimiters) {
        let row = row + opts.start_row;
        let r = CellRef { row, col };
        let text_col = opts.text_columns.contains(&col);
        let Some(cell) = field_cell(&mut wb, &mut styles, r, &field, None, text_col) else {
            continue;
        };
        let cells = &mut wb.sheets[0].cells;
        if last.is_some_and(|l| l >= (row, col)) {
            // 차례가 어긋나면 (생기지 않아야 한다) 자리를 찾아 넣는다
            match cells.binary_search_by(|c| (c.r.row, c.r.col).cmp(&(row, col))) {
                Ok(i) => cells[i] = cell,
                Err(i) => cells.insert(i, cell),
            }
        } else {
            cells.push(cell);
            last = Some((row, col));
        }
    }
    let sheet = &mut wb.sheets[0];
    crate::edit::structure::update_dimension(sheet);
    if sheet.cells.iter().any(|c| c.formula.is_some()) {
        crate::calc::recalculate(&mut wb);
    }
    Ok(wb)
}

/// 필드 글 → 값. 한셀이 텍스트 파일을 열 때 칸마다 하는 해석 — 셀 입력 규칙(일반 서식 칸)에 더해:
/// 256 자보다 긴 필드는 글자 서식(@)의 글, 대분수(`1 1/2`)는 수, `+`·`-` 로 시작하는 글은 수식
/// (`+1+2` → `=1+2`, 고를 수 없으면 글)
pub fn value(field: &str, date1904: bool) -> Input {
    // 256 자보다 긴 필드는 해석하지 않는다 (한셀 관찰: 257 자부터, 바이트가 아니라 글자 수)
    if field.chars().nth(LONG_TEXT).is_some() {
        return Input::Value {
            value: CellValue::Text(field.to_string()),
            num_fmt: Some(49),
            quote_prefix: false,
        };
    }
    if let Some(v) = mixed_fraction(field) {
        return v;
    }
    match input::parse(field, "General", date1904) {
        Input::Value {
            value: CellValue::Text(_),
            num_fmt: None,
            quote_prefix: false,
        } if field.len() > 1 && field.starts_with(['+', '-']) => Input::Formula(field.to_string()),
        // 대분수 꼴의 글(`1 1/2 `)에 입력 규칙이 붙이는 분수 형식은 붙이지 않는다
        Input::Value {
            value: v @ CellValue::Text(_),
            num_fmt: Some(12 | 13),
            quote_prefix: false,
        } => Input::Value {
            value: v,
            num_fmt: None,
            quote_prefix: false,
        },
        v => v,
    }
}

/// 대분수 `정수 분자/분모` (앞 공백은 괜찮고 뒤 공백·부호는 글) → 수, 분모가 10 보다 작으면 `# ?/?`(12),
/// 아니면 `# ??/??`(13)
fn mixed_fraction(field: &str) -> Option<Input> {
    let t = field.trim_start_matches(' ');
    if t.ends_with(' ') {
        return None;
    }
    let (whole, frac) = t.split_once(' ')?;
    let frac = frac.trim_start_matches(' ');
    let (n, d) = frac.split_once('/')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(n) || !digits(d) {
        return None;
    }
    let (w, n, d): (f64, f64, f64) = (whole.parse().ok()?, n.parse().ok()?, d.parse().ok()?);
    if d == 0.0 {
        return None;
    }
    Some(Input::Value {
        value: CellValue::Number(w + n / d),
        num_fmt: Some(if d < 10.0 { 12 } else { 13 }),
        quote_prefix: false,
    })
}

/// 이보다 긴 필드는 글자 서식(@)의 글
const LONG_TEXT: usize = 256;

/// (표시 형식, 글자 접두, 줄 바꿈, 더한 서식)
type StyleKey = (Option<u32>, bool, bool, String);

/// 가져오기가 칸 서식을 만들 때 쓰는 기억 (같은 서식은 한 번만)
pub(crate) type StyleCache = HashMap<StyleKey, u32>;

/// 필드 글 하나로 칸을 만든다 (텍스트 가져오기와 HTML 가져오기가 같이 쓴다): 값 해석, 수식 고르기, 줄 바꿈 맞춤.
/// `patch` 는 얹을 서식 (굵게·맞춤·채우기 …), `as_text` 면 값 해석 없이 글자 서식 글
pub(crate) fn field_cell(
    wb: &mut Workbook,
    cache: &mut StyleCache,
    r: CellRef,
    field: &str,
    patch: Option<&crate::edit::style::FormatPatch>,
    as_text: bool,
) -> Option<Cell> {
    let input = if as_text {
        if field.is_empty() {
            return None;
        }
        // 맨 앞의 '(글자 접두)는 글자 열에서도 글자로 벗긴다 (한셀 관찰)
        match field.strip_prefix('\'') {
            Some(rest) if !rest.is_empty() => Input::Value {
                value: CellValue::Text(rest.to_string()),
                num_fmt: Some(49),
                quote_prefix: true,
            },
            _ => Input::Value {
                value: CellValue::Text(field.to_string()),
                num_fmt: Some(49),
                quote_prefix: false,
            },
        }
    } else {
        value(field, wb.date1904)
    };
    let pkey = patch.map(|p| format!("{p:?}")).unwrap_or_default();
    Some(match input {
        Input::Empty => return None,
        // 함수 호출이 아닌 자리의 쉼표(합집합)는 양쪽이 참조일 때만 수식 — 숫자·글 사이 쉼표는 문법에 맞지 않아 글 (한셀 관찰)
        Input::Formula(_) if bad_union(field) => {
            let style = style_for(wb, cache, (None, false, false, pkey), patch);
            Cell {
                r,
                style,
                value: CellValue::Text(field.to_string()),
                formula: None,
            }
        }
        Input::Formula(f) => match crate::edit::normalize_formula_strict(wb, &f) {
            Ok(text) => {
                let fmt = crate::edit::formula_format(wb, 0, &text);
                let style = style_for(wb, cache, (fmt, false, false, pkey), patch);
                Cell {
                    r,
                    style,
                    value: CellValue::Empty,
                    formula: Some(Formula {
                        text,
                        ..Default::default()
                    }),
                }
            }
            // 고를 수 없는 수식은 글로
            Err(_) => {
                let style = style_for(wb, cache, (None, false, false, pkey), patch);
                Cell {
                    r,
                    style,
                    value: CellValue::Text(field.to_string()),
                    formula: None,
                }
            }
        },
        Input::Value {
            value,
            num_fmt,
            quote_prefix,
        } => {
            // 줄 바꿈(LF)이 든 글은 줄 바꿈 맞춤 (한셀 관찰 — CR 만 든 글은 그대로)
            let wrap = matches!(&value, CellValue::Text(t) if t.contains('\n'));
            let style = style_for(wb, cache, (num_fmt, quote_prefix, wrap, pkey), patch);
            Cell {
                r,
                style,
                value,
                formula: None,
            }
        }
    })
}

/// 수식 글의 합집합 쉼표(함수 인수 자리가 아닌 쉼표) 가운데 양쪽이 참조가 아닌 것이 있나. 따옴표 안은 건드리지 않는다
fn bad_union(f: &str) -> bool {
    let c: Vec<char> = f.chars().collect();
    let mut stack: Vec<bool> = Vec::new(); // 괄호마다: 함수 호출인가
    let mut i = 0;
    let delim = |ch: char| ",()+-*/^&=<>%; ".contains(ch);
    let refy = |t: &str| {
        let t = t.trim_matches('$');
        !t.is_empty()
            && (t.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '\'' || ch == '_')
                || t.contains('!'))
    };
    while i < c.len() {
        match c[i] {
            '"' => {
                i += 1;
                while i < c.len() && c[i] != '"' {
                    i += 1;
                }
            }
            '(' => stack
                .push(i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_' || c[i - 1] == '.')),
            ')' => {
                stack.pop();
            }
            ',' if stack.last().copied() != Some(true) => {
                let l: String = {
                    let mut j = i;
                    while j > 0 && !delim(c[j - 1]) {
                        j -= 1;
                    }
                    c[j..i].iter().collect()
                };
                let r: String = {
                    let mut j = i + 1;
                    while j < c.len() && !delim(c[j]) {
                        j += 1;
                    }
                    c[i + 1..j].iter().collect()
                };
                if !refy(&l) || !refy(&r) {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// 서식 바꿈 하나만 입힌 칸 서식 번호 (값 해석 없이)
pub(crate) fn styled(
    wb: &mut Workbook,
    cache: &mut StyleCache,
    patch: &crate::edit::style::FormatPatch,
) -> u32 {
    style_for(
        wb,
        cache,
        (None, false, false, format!("{patch:?}")),
        Some(patch),
    )
}

fn style_for(
    wb: &mut Workbook,
    cache: &mut StyleCache,
    key: StyleKey,
    patch: Option<&crate::edit::style::FormatPatch>,
) -> u32 {
    let (num_fmt, quote_prefix, wrap) = (key.0, key.1, key.2);
    if num_fmt.is_none() && !quote_prefix && !wrap && patch.is_none() {
        return 0;
    }
    if let Some(&s) = cache.get(&key) {
        return s;
    }
    let s = crate::edit::style::Restyler::default().apply(&mut wb.styles, 0, &|_, x| {
        if let Some(id) = num_fmt {
            x.num_fmt_id = id;
            x.apply_number_format = Some(true);
        }
        x.quote_prefix = quote_prefix;
        if wrap {
            x.alignment.get_or_insert_with(Default::default).wrap_text = true;
            x.apply_alignment = Some(true);
        }
    });
    let s = match patch {
        Some(p) => crate::edit::style::Restyler::default().apply(&mut wb.styles, s, &|st, x| {
            crate::edit::style::patch_xf(st, x, p);
        }),
        None => s,
    };
    cache.insert(key, s);
    s
}

// ---------------------------------------------------------------------------
// 저장
// ---------------------------------------------------------------------------

/// 저장할 시트: 활성 시트 (워크시트가 아니면 첫 워크시트)
pub fn active_sheet(wb: &Workbook) -> Option<usize> {
    let a = wb.view.as_ref().map_or(0, |v| v.active_tab as usize);
    if wb
        .sheets
        .get(a)
        .is_some_and(|s| s.kind == SheetKind::Worksheet)
    {
        return Some(a);
    }
    wb.sheets
        .iter()
        .position(|s| s.kind == SheetKind::Worksheet)
}

/// 칸에 보이는 글 (표시 형식을 적용한 글, 수식은 계산된 값)
pub fn shown(wb: &Workbook, cell: &Cell) -> String {
    let code = || {
        wb.styles
            .xf(cell.style)
            .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
            .unwrap_or_else(|| "General".into())
    };
    match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        CellValue::Error(e) => e.clone(),
        v @ (CellValue::Number(_) | CellValue::Text(_)) => {
            crate::numfmt::format(v, &code(), wb.date1904).text
        }
        CellValue::Date(s) => s.clone(),
    }
}

/// 시트를 텍스트로 (한셀의 CSV·텍스트·PRN 저장과 같은 행·열 범위와 따옴표, 줄 끝 CRLF). 글은 유니코드 그대로
pub fn sheet_text(wb: &Workbook, sheet: &Sheet, delim: Delimiter) -> String {
    // 한셀이 셀로 보는 것: 값·수식이 있거나, 기본이 아닌 서식이 있거나, 병합 영역 안의 칸 (서식 0 의 빈
    // `<c>` 는 세지 않는다 — 말뭉치 structure 의 A10)
    let counted = |c: &&Cell| {
        !c.value.is_empty()
            || c.formula.is_some()
            || c.style != 0
            || sheet.merges.iter().any(|m| {
                (m.start.row..=m.end.row).contains(&c.r.row)
                    && (m.start.col..=m.end.col).contains(&c.r.col)
            })
    };
    let cells: Vec<&Cell> = sheet.cells.iter().filter(counted).collect();
    let (Some(first), Some(last)) = (
        cells.iter().map(|c| c.r.col).min(),
        cells.iter().map(|c| c.r.col).max(),
    ) else {
        return String::new();
    };
    let first = if delim == Delimiter::Comma { 0 } else { first };
    let d = delim.as_char();
    let mut out = String::new();
    let mut i = 0;
    while i < cells.len() {
        let row = cells[i].r.row;
        let mut fields: Vec<String> = vec![String::new(); (last - first + 1) as usize];
        while i < cells.len() && cells[i].r.row == row {
            let c = cells[i];
            if c.r.col >= first {
                let mut t = shown(wb, c);
                // 숫자의 표시 형식에 든 원 기호(U+20A9)는 949 의 0x5C 로 (한셀 관찰 — 글 칸의 ₩ 는 `?`)
                if matches!(c.value, CellValue::Number(_)) {
                    t = t.replace('\u{20A9}', "\\");
                }
                fields[(c.r.col - first) as usize] = quote(&t, d);
            }
            i += 1;
        }
        out.push_str(&fields.join(&d.to_string()));
        out.push_str("\r\n");
    }
    out
}

fn quote(t: &str, d: char) -> String {
    if t.contains([d, '"', '\n', '\r']) {
        format!("\"{}\"", t.replace('"', "\"\""))
    } else {
        t.to_string()
    }
}

/// 통합 문서의 활성 시트를 CSV·텍스트·PRN 파일 바이트로 (코드 페이지 949)
pub fn write(wb: &Workbook, delim: Delimiter) -> crate::Result<Vec<u8>> {
    let Some(si) = active_sheet(wb) else {
        return Ok(Vec::new());
    };
    Ok(cp949::encode_lossy(&sheet_text(wb, &wb.sheets[si], delim)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(t: &str, d: Delimiter) -> Vec<(u32, u32, String)> {
        split(t, d)
    }

    #[test]
    fn split_quotes_and_lines() {
        // 한셀 관찰 (가져오기 실험 s1, s7)
        let t = "a,b,c\r\n\"x,y\",\"he said \"\"hi\"\"\",\"multi\r\nline\"\r\n\"lf\nonly\",plain\r\nragged\r\n\r\n,,\r\na\"b,\"c\"d,\"e\" f, \"g\"\r\ntrailing,\r\n\"open,x\r\nlast";
        let c = cells(t, Delimiter::Comma);
        let at = |r: u32, col: u32| {
            c.iter()
                .find(|x| x.0 == r && x.1 == col)
                .map(|x| x.2.as_str())
        };
        assert_eq!(at(1, 0), Some("x,y"));
        assert_eq!(at(1, 1), Some("he said \"hi\""));
        assert_eq!(at(1, 2), Some("multi\r\nline"));
        assert_eq!(at(2, 0), Some("lf\nonly"));
        assert_eq!(at(3, 0), Some("ragged"));
        assert_eq!(at(4, 0), None);
        assert_eq!(at(6, 0), Some("a\"b"));
        assert_eq!(at(6, 1), Some("cd"));
        assert_eq!(at(6, 2), Some("e f"));
        assert_eq!(at(6, 3), Some(" \"g\""));
        assert_eq!(at(7, 0), Some("trailing"));
        assert_eq!(at(8, 0), Some("open,x\r\nlast"));
        let s7 = cells(
            "\"a\"  ,  \"b\"\r\n\"c\"\"d\"\"\",e\r\n\"\"\"\"\r\n\"\",x\r\n",
            Delimiter::Comma,
        );
        assert_eq!(
            s7,
            vec![
                (0, 0, "a  ".into()),
                (0, 1, "  \"b\"".into()),
                (1, 0, "c\"d\"".into()),
                (1, 1, "e".into()),
                (2, 0, "\"".into()),
                (3, 1, "x".into()),
            ]
        );
        // CR 만, LF 만
        assert_eq!(cells("a\r1", Delimiter::Comma).len(), 2);
        assert_eq!(cells("a\n1\n", Delimiter::Comma)[1], (1, 0, "1".into()));
    }

    #[test]
    fn long_fields() {
        // 따옴표 없는 필드는 32759 자에서 잘려 나머지가 다음 행 A 열로
        let t = format!("long,{}\r\nend", "x".repeat(40000));
        let c = cells(&t, Delimiter::Comma);
        assert_eq!(c[1].2.len(), 32759);
        assert_eq!((c[2].0, c[2].1, c[2].2.len()), (1, 0, 7241));
        assert_eq!((c[3].0, c[3].2.as_str()), (2, "end"));
        // 따옴표로 싼 필드는 32767 자까지, 나머지는 버리고 다음 필드는 한 행 아래
        let t = format!("\"{}\",x\r\nend", "b".repeat(40000));
        let c = cells(&t, Delimiter::Comma);
        assert_eq!((c[0].0, c[0].1, c[0].2.len()), (0, 0, 32767));
        assert_eq!((c[1].0, c[1].1, c[1].2.as_str()), (1, 1, "x"));
        assert_eq!((c[2].0, c[2].1), (2, 0));
    }

    #[test]
    fn encodings() {
        let k = "이름,값\r\n";
        assert_eq!(decode(&cp949::encode_lossy(k)), k);
        assert_eq!(decode(k.as_bytes()), k);
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(k.as_bytes());
        assert_eq!(decode(&bom), k);
        let mut le = vec![0xFF, 0xFE];
        le.extend(k.encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(decode(&le), k);
        let mut be = vec![0xFE, 0xFF];
        be.extend(k.encode_utf16().flat_map(|u| u.to_be_bytes()));
        assert_eq!(decode(&be), k);
        // BOM 없는 UTF-16 은 949 로 읽혀 NUL 에서 끝난다 (한셀: `t??`)
        let nobom: Vec<u8> = k.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&nobom), "t??");
    }
}
