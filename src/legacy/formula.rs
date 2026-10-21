// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 `.cell` 수식 토큰(Ptg) 디코더.
//!
//! 토큰 체계는 공개 명세 [MS-XLS] 2.5.198 의 BIFF8 Ptg 와 같다. 한셀 레거시의 차이:
//! - 행 번호가 u32 (PtgRef 6바이트, PtgArea 12바이트, PtgRef3d 8바이트 …)
//! - PtgStr 의 길이가 u16, 뒤따르는 플래그가 1이면 UTF-16, 0이면 1바이트 문자
//! - 한셀 확장 함수 번호 (예: IFERROR = 480) — [`super::ftab`]
//!
//! 출력은 한셀이 XML 형식에 쓰는 수식 원문과 같은 꼴(`=` 없음)이다.

use super::ftab;
use crate::model::CellRef;

/// 최대 행/열 (한셀 2024: 1,048,576 행 × 16,384 열).
const MAX_ROW: u32 = 1_048_575;
const MAX_COL: u16 = 16_383;

/// 디코딩에 필요한 문서 정보.
pub struct Context<'a> {
    /// EXTERNSHEET 의 XTI 별 시트 부분 (`ixti` → [`sheet_ref_prefix`] 결과, 따옴표 포함)
    pub xti_sheets: &'a [Option<String>],
    /// NAME 레코드 순서대로의 정의된 이름 (PtgName 은 1부터)
    pub names: &'a [String],
    /// XTI 별 외부 이름(EXTERNNAME) 목록, PtgNameX 의 이름 번호는 1부터. 추가 기능이면 함수 이름 그대로,
    /// 외부 문서면 `[n]!이름` 꼴로 미리 만들어 둔다
    pub xti_names: &'a [Vec<String>],
    /// 수식이 있는 셀 (상대 토큰 PtgRefN/PtgAreaN 의 기준)
    pub base: CellRef,
    /// 바이너리 OOXML(XLSB) 토큰이면 true. PtgStr 이 플래그 없는 UTF-16 이다.
    pub xlsb: bool,
    /// 행 반복(0xB002)으로 복사된 행: 행 상대 참조를 이만큼 옮긴다
    pub row_shift: i64,
}

/// 디코딩 실패. 알 수 없는 토큰이나 잘린 데이터.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError(pub String);

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.pos + n > self.b.len() {
            return Err(DecodeError(format!(
                "수식 데이터가 잘림 (위치 {})",
                self.pos
            )));
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f64(&mut self) -> Result<f64, DecodeError> {
        let s = self.take(8)?;
        Ok(f64::from_le_bytes(s.try_into().unwrap()))
    }
}

/// 열 필드: 하위 14비트 열 번호, 비트 14 = 열 상대, 비트 15 = 행 상대.
fn split_col(c: u16) -> (u16, bool, bool) {
    (c & 0x3FFF, c & 0x4000 != 0, c & 0x8000 != 0)
}

fn col_text(col: u16, abs: bool) -> String {
    format!(
        "{}{}",
        if abs { "$" } else { "" },
        CellRef::col_name(col as u32)
    )
}

fn row_text(row: u32, abs: bool) -> String {
    format!("{}{}", if abs { "$" } else { "" }, row + 1)
}

/// 행 상대 참조면 행을 옮긴다 (열 전체 참조는 그대로).
fn shifted(row: u32, colf: u16, shift: i64) -> u32 {
    if shift != 0 && colf & 0x8000 != 0 && row < MAX_ROW {
        (row as i64 + shift).clamp(0, MAX_ROW as i64) as u32
    } else {
        row
    }
}

fn ref_text(row: u32, colf: u16) -> String {
    let (col, col_rel, row_rel) = split_col(colf);
    format!("{}{}", col_text(col, !col_rel), row_text(row, !row_rel))
}

fn area_text(r1: u32, r2: u32, c1f: u16, c2f: u16) -> String {
    let (c1, c1_rel, r1_rel) = split_col(c1f);
    let (c2, c2_rel, r2_rel) = split_col(c2f);
    if r1 == 0 && r2 >= MAX_ROW {
        // 열 전체 (A:A)
        return format!("{}:{}", col_text(c1, !c1_rel), col_text(c2, !c2_rel));
    }
    if c1 == 0 && c2 >= MAX_COL {
        // 행 전체 (1:1)
        return format!("{}:{}", row_text(r1, !r1_rel), row_text(r2, !r2_rel));
    }
    format!("{}:{}", ref_text(r1, c1f), ref_text(r2, c2f))
}

/// 상대 토큰(PtgRefN 등)의 행/열 오프셋을 기준 셀에 더한다.
fn rel_row(base: u32, raw: u32, rel: bool) -> u32 {
    if rel {
        (base as i64 + raw as i32 as i64).rem_euclid(MAX_ROW as i64 + 1) as u32
    } else {
        raw
    }
}

fn rel_col(base: u32, raw: u16, rel: bool) -> u16 {
    if rel {
        let off = ((raw as i16) << 2) >> 2; // 14비트 부호 확장
        (base as i64 + off as i64).rem_euclid(MAX_COL as i64 + 1) as u16
    } else {
        raw & 0x3FFF
    }
}

fn with_flags(col: u16, col_rel: bool, row_rel: bool) -> u16 {
    col | if col_rel { 0x4000 } else { 0 } | if row_rel { 0x8000 } else { 0 }
}

/// 시트 이름을 수식에 쓸 꼴로 (공백·기호가 있으면 작은따옴표).
/// PtgName 이 가리키는 이름 목록을 시트 하나에 맞춘다: 다른 시트 범위의 이름은 `시트!이름` 으로 쓴다 — 그 시트에서
/// 이름만 쓰면 자기 시트의 같은 이름이 되기 때문이다 (수식 대조 실험: Sheet2 의 `=Sheet1!L`, 두 시트에 범위 이름 L).
/// `scopes[i]` 는 i 번째 이름의 시트 번호(0부터, 없으면 통합 문서 범위)
pub fn names_for_sheet(
    names: &[String],
    scopes: &[Option<u32>],
    sheet_names: &[String],
    sheet: usize,
) -> Vec<String> {
    names
        .iter()
        .zip(scopes.iter().chain(std::iter::repeat(&None)))
        .map(|(n, scope)| match scope.map(|s| s as usize) {
            Some(s) if s != sheet => match sheet_names.get(s) {
                Some(sh) => format!("{}!{n}", quote_sheet(sh)),
                None => n.clone(),
            },
            _ => n.clone(),
        })
        .collect()
}

pub fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && CellRef::parse(name).is_none();
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// 3차원 참조의 시트 부분 (`!` 앞, 필요하면 따옴표로 감싼다).
/// 외부 문서는 `[n]` 이 앞에 붙고, 그 문서 파일 이름에 따옴표가 필요하면 함께 감싼다
/// (한셀 관측: `'[1]Intro'!` ← "Auto Expense Report.hcdt", `[2]Intro!` ← "RecoveredExternalLink1").
pub fn sheet_ref_prefix(book: Option<(usize, &str)>, first: &str, last: Option<&str>) -> String {
    let needs = |s: &str| quote_sheet(s) != s;
    let quote =
        book.is_some_and(|(_, file)| needs(file)) || needs(first) || last.is_some_and(needs);
    let mut s = match book {
        Some((n, _)) => format!("[{n}]{first}"),
        None => first.to_string(),
    };
    if let Some(l) = last {
        s.push(':');
        s.push_str(l);
    }
    if quote {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        s
    }
}

/// XTI 번호 → `시트!` (`Context::xti_sheets` 에는 [`sheet_ref_prefix`] 결과가 들어 있다)
fn sheet_prefix(ctx: &Context, ixti: u16) -> String {
    match ctx.xti_sheets.get(ixti as usize).and_then(|s| s.as_deref()) {
        Some(prefix) => format!("{prefix}!"),
        None => "#REF!".to_string(),
    }
}

/// 레거시는 Excel 2007 에서 새로 생긴 함수도 `_xlfn.` 이름으로 쓴다. 그중 OOXML 에서 접두 없이 쓰는
/// 함수(한셀 XML 도 접두를 뗀다, 함수 대조 실험)는 접두를 뗀다.
fn native_name(name: String) -> String {
    const NATIVE: [&str; 5] = ["AVERAGEIF", "AVERAGEIFS", "COUNTIFS", "SUMIFS", "IFERROR"];
    match name.strip_prefix("_xlfn.") {
        Some(rest) if NATIVE.contains(&rest) => rest.to_string(),
        _ => name,
    }
}

/// 수식 원문에 쓰는 숫자 표기 (한셀 XML 과 같은 꼴: 큰 수는 `1.5E+20`).
pub fn number_text(v: f64) -> String {
    let a = v.abs();
    if a != 0.0 && !(1e-5..1e15).contains(&a) {
        let s = format!("{v:E}"); // 1.5E20
        match s.split_once('E') {
            Some((m, e)) => {
                let e: i32 = e.parse().unwrap_or(0);
                format!("{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
            }
            None => s,
        }
    } else {
        format!("{v}")
    }
}

pub fn error_text(code: u8) -> &'static str {
    match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        0x2A => "#N/A",
        0x2B => "#GETTING_DATA",
        _ => "#ERR!",
    }
}

fn pop(stack: &mut Vec<String>) -> Result<String, DecodeError> {
    stack
        .pop()
        .ok_or_else(|| DecodeError("수식 스택이 비었음".into()))
}

fn pop_args(stack: &mut Vec<String>, n: usize) -> Result<Vec<String>, DecodeError> {
    if stack.len() < n {
        return Err(DecodeError(format!("함수 인자 부족 ({n}개 필요)")));
    }
    Ok(stack.split_off(stack.len() - n))
}

/// 결과: 수식 원문, 또는 공유 수식 참조(PtgExp)면 그 기준 셀.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    Text(String),
    /// PtgExp — 공유 수식/배열 수식의 기준 셀
    SharedRef(CellRef),
    /// PtgTbl — 데이터 표의 왼쪽 위 셀 (레거시 TABLE 0x0236 이 뜻을 담는다)
    TableRef(CellRef),
}

/// 수식 토큰열을 원문으로 바꾼다.
pub fn decode(rgce: &[u8], ctx: &Context) -> Result<Decoded, DecodeError> {
    let mut r = Reader { b: rgce, pos: 0 };
    let mut stack: Vec<String> = Vec::new();
    while r.pos < rgce.len() {
        let ptg = r.u8()?;
        // 피연산자 토큰은 클래스(참조/값/배열)에 따라 0x20·0x40·0x60 이 더해진다.
        let base = if (0x20..0x80).contains(&ptg) {
            (ptg & 0x1F) | 0x20
        } else {
            ptg
        };
        match base {
            0x01 => {
                // PtgExp: BIFF8 = 행 + 열, XLSB = 행만 (열은 공유 범위로 찾는다 → 이 셀의 열을 넣어 둔다)
                let row = r.u32()?;
                let col = if ctx.xlsb {
                    ctx.base.col
                } else {
                    r.u16()? as u32
                };
                return Ok(Decoded::SharedRef(CellRef { row, col }));
            }
            0x02 => {
                // PtgTbl: 행 u32 + 열 u16 (레거시. 바이너리의 데이터 표 셀은 수식 없이 값만 쓴다)
                let row = r.u32()?;
                let col = r.u16()? as u32;
                return Ok(Decoded::TableRef(CellRef { row, col }));
            }
            0x03..=0x11 => {
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                let op = match base {
                    0x03 => "+",
                    0x04 => "-",
                    0x05 => "*",
                    0x06 => "/",
                    0x07 => "^",
                    0x08 => "&",
                    0x09 => "<",
                    0x0A => "<=",
                    0x0B => "=",
                    0x0C => ">=",
                    0x0D => ">",
                    0x0E => "<>",
                    0x0F => " ",
                    0x10 => ",",
                    _ => ":",
                };
                stack.push(format!("{a}{op}{b}"));
            }
            0x12 => {
                let a = pop(&mut stack)?;
                stack.push(format!("+{a}"));
            }
            0x13 => {
                let a = pop(&mut stack)?;
                stack.push(format!("-{a}"));
            }
            0x14 => {
                let a = pop(&mut stack)?;
                stack.push(format!("{a}%"));
            }
            0x15 => {
                let a = pop(&mut stack)?;
                stack.push(format!("({a})"));
            }
            0x16 => stack.push(String::new()),
            0x17 => {
                let cch = r.u16()? as usize;
                let wide = ctx.xlsb || r.u8()? & 1 == 1;
                let s = if wide {
                    crate::text::utf16le(r.take(cch * 2)?)
                } else {
                    r.take(cch)?.iter().map(|&b| b as char).collect()
                };
                stack.push(format!("\"{}\"", s.replace('"', "\"\"")));
            }
            0x19 => {
                let flags = r.u8()?;
                let data = r.u16()?;
                if flags & 0x04 != 0 {
                    // tAttrChoose: 점프 표 (data+1)개 u16
                    r.take((data as usize + 1) * 2)?;
                } else if flags & 0x10 != 0 {
                    let a = pop(&mut stack)?;
                    stack.push(format!("SUM({a})"));
                }
                // tAttrIf/Goto/Semi/Space/Baxcel 은 원문에 영향 없음
            }
            0x1C => {
                let e = r.u8()?;
                stack.push(error_text(e).into());
            }
            0x1D => {
                let b = r.u8()?;
                stack.push(if b != 0 { "TRUE" } else { "FALSE" }.into());
            }
            0x1E => {
                let v = r.u16()?;
                stack.push(v.to_string());
            }
            0x1F => {
                let v = r.f64()?;
                stack.push(number_text(v));
            }
            0x20 => {
                // PtgArray — 상수 배열 내용은 토큰열 뒤(rgcb)에 있다. 아직 미지원.
                r.take(7)?;
                stack.push("{…}".into());
            }
            0x21 => {
                let idx = r.u16()?;
                let (name, argc) = ftab::lookup(idx)
                    .ok_or_else(|| DecodeError(format!("알 수 없는 함수 번호 {idx}")))?;
                let argc = argc.ok_or_else(|| {
                    DecodeError(format!("가변 인자 함수 {name} 가 고정 인자로 쓰임"))
                })?;
                let args = pop_args(&mut stack, argc as usize)?;
                stack.push(format!("{name}({})", args.join(",")));
            }
            0x22 => {
                let argc = r.u8()? & 0x7F;
                let idx = r.u16()? & 0x7FFF;
                let mut args = pop_args(&mut stack, argc as usize)?;
                let name = if idx == 255 {
                    // 사용자 정의/추가 기능 함수: 첫 인자가 이름
                    if args.is_empty() {
                        return Err(DecodeError("사용자 함수 이름 없음".into()));
                    }
                    native_name(args.remove(0))
                } else {
                    ftab::lookup(idx)
                        .map(|(n, _)| n.to_string())
                        .ok_or_else(|| DecodeError(format!("알 수 없는 함수 번호 {idx}")))?
                };
                stack.push(format!("{name}({})", args.join(",")));
            }
            0x23 => {
                let idx = r.u32()?;
                let name = ctx
                    .names
                    .get((idx as usize).wrapping_sub(1))
                    .cloned()
                    .unwrap_or_else(|| format!("#NAME{idx}"));
                stack.push(name);
            }
            0x24 => {
                let row = r.u32()?;
                let col = r.u16()?;
                stack.push(ref_text(shifted(row, col, ctx.row_shift), col));
            }
            0x25 => {
                let r1 = r.u32()?;
                let r2 = r.u32()?;
                let c1 = r.u16()?;
                let c2 = r.u16()?;
                let full = r1 == 0 && r2 >= MAX_ROW;
                let (r1, r2) = if full {
                    (r1, r2)
                } else {
                    (
                        shifted(r1, c1, ctx.row_shift),
                        shifted(r2, c2, ctx.row_shift),
                    )
                };
                stack.push(area_text(r1, r2, c1, c2));
            }
            0x26 | 0x27 => {
                // PtgMemArea / PtgMemErr: 뒤따르는 하위 식의 길이만 담는다
                r.take(4)?;
                r.u16()?;
            }
            0x28 | 0x29 => {
                r.u16()?;
            }
            0x2A => {
                r.take(6)?;
                stack.push("#REF!".into());
            }
            0x2B => {
                r.take(12)?;
                stack.push("#REF!".into());
            }
            0x2C => {
                let row = r.u32()?;
                let colf = r.u16()?;
                let (_, col_rel, row_rel) = split_col(colf);
                let rr = rel_row(ctx.base.row, row, row_rel);
                let cc = rel_col(ctx.base.col, colf & 0x3FFF, col_rel);
                stack.push(ref_text(rr, with_flags(cc, col_rel, row_rel)));
            }
            0x2D => {
                let r1 = r.u32()?;
                let r2 = r.u32()?;
                let c1f = r.u16()?;
                let c2f = r.u16()?;
                let (_, c1r, r1r) = split_col(c1f);
                let (_, c2r, r2r) = split_col(c2f);
                let a = rel_row(ctx.base.row, r1, r1r);
                let b = rel_row(ctx.base.row, r2, r2r);
                let c = rel_col(ctx.base.col, c1f & 0x3FFF, c1r);
                let d = rel_col(ctx.base.col, c2f & 0x3FFF, c2r);
                stack.push(area_text(
                    a,
                    b,
                    with_flags(c, c1r, r1r),
                    with_flags(d, c2r, r2r),
                ));
            }
            0x39 => {
                let ixti = r.u16()?;
                let idx = r.u32()?;
                let name = ctx
                    .xti_names
                    .get(ixti as usize)
                    .and_then(|v| v.get((idx as usize).wrapping_sub(1)))
                    .cloned()
                    .unwrap_or_else(|| format!("#NAMEX{idx}"));
                stack.push(name);
            }
            0x3A => {
                let ixti = r.u16()?;
                let row = r.u32()?;
                let col = r.u16()?;
                let row = shifted(row, col, ctx.row_shift);
                stack.push(format!("{}{}", sheet_prefix(ctx, ixti), ref_text(row, col)));
            }
            0x3B => {
                let ixti = r.u16()?;
                let r1 = r.u32()?;
                let r2 = r.u32()?;
                let c1 = r.u16()?;
                let c2 = r.u16()?;
                let (r1, r2) = if r1 == 0 && r2 >= MAX_ROW {
                    (r1, r2)
                } else {
                    (
                        shifted(r1, c1, ctx.row_shift),
                        shifted(r2, c2, ctx.row_shift),
                    )
                };
                stack.push(format!(
                    "{}{}",
                    sheet_prefix(ctx, ixti),
                    area_text(r1, r2, c1, c2)
                ));
            }
            0x3C => {
                let ixti = r.u16()?;
                r.take(6)?;
                stack.push(format!("{}#REF!", sheet_prefix(ctx, ixti)));
            }
            0x3D => {
                let ixti = r.u16()?;
                r.take(12)?;
                stack.push(format!("{}#REF!", sheet_prefix(ctx, ixti)));
            }
            other => {
                return Err(DecodeError(format!(
                    "알 수 없는 토큰 0x{other:02x} (위치 {})",
                    r.pos - 1
                )));
            }
        }
    }
    match stack.len() {
        1 => Ok(Decoded::Text(stack.pop().unwrap())),
        0 => Ok(Decoded::Text(String::new())),
        n => Err(DecodeError(format!("수식 스택에 {n}개 남음"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context<'static> {
        Context {
            xti_sheets: &[],
            names: &[],
            xti_names: &[],
            base: CellRef::default(),
            xlsb: false,
            row_shift: 0,
        }
    }

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    fn text(s: &str) -> String {
        match decode(&hex(s), &ctx()).unwrap() {
            Decoded::Text(t) => t,
            d => panic!("{d:?}"),
        }
    }

    // 아래 토큰열은 한셀 2024 가 저장한 레거시 파일에서 관측한 그대로다.
    #[test]
    fn arithmetic_and_refs() {
        assert_eq!(
            text(
                "44 00 00 00 00 00 c0 44 01 00 00 00 00 c0 44 02 00 00 00 00 c0 05 03 44 03 00 00 00 00 c0 44 04 00 00 00 00 c0 06 04"
            ),
            "A1+A2*A3-A4/A5"
        );
        assert_eq!(
            text("44 00 00 00 00 00 00 44 01 00 00 00 00 40 03 44 02 00 00 00 00 80 03"),
            "$A$1+A$2+$A3"
        );
    }

    #[test]
    fn functions_and_strings() {
        assert_eq!(
            text("25 00 00 00 00 04 00 00 00 00 c0 00 c0 19 10 00 00"),
            "SUM(A1:A5)"
        );
        assert_eq!(
            text(
                "44 00 00 00 00 00 c0 1e 05 00 0d 19 02 0c 00 17 02 00 01 6c d0 e4 b2 19 08 0f 00 17 02 00 01 91 c7 e4 b2 19 08 03 00 42 03 01 00"
            ),
            "IF(A1>5,\"크다\",\"작다\")"
        );
        assert_eq!(
            text("1e 01 00 1e 00 00 06 1e 00 00 41 e0 01"),
            "IFERROR(1/0,0)"
        );
        assert_eq!(
            text("44 00 00 00 00 01 c0 17 01 00 00 2d 44 01 00 00 00 01 c0 42 03 50 01"),
            "CONCATENATE(B1,\"-\",B2)"
        );
    }

    #[test]
    fn whole_rows_columns_and_numbers() {
        assert_eq!(
            text("25 00 00 00 00 ff ff 0f 00 00 40 00 40 19 10 00 00"),
            "SUM(A:A)"
        );
        assert_eq!(
            text("25 00 00 00 00 00 00 00 00 00 80 ff bf 19 10 00 00"),
            "SUM(1:1)"
        );
        assert_eq!(number_text(1.5e20), "1.5E+20");
        assert_eq!(number_text(-5.0), "-5");
        assert_eq!(number_text(7.8), "7.8");
    }

    #[test]
    fn shared_formula_pointer_biff8_and_xlsb() {
        let mut c = Context {
            xti_sheets: &[],
            names: &[],
            xti_names: &[],
            base: CellRef { row: 28, col: 8 },
            xlsb: false,
            row_shift: 0,
        };
        // BIFF8 변형: 행 u32 + 열 u16
        let d = decode(&hex("01 1b 00 00 00 08 00"), &c).unwrap();
        assert_eq!(d, Decoded::SharedRef(CellRef { row: 27, col: 8 }));
        // XLSB: 행 u32 만 (열은 이 셀의 열로 두고 공유 범위로 찾는다)
        c.xlsb = true;
        let d = decode(&hex("01 1b 00 00 00"), &c).unwrap();
        assert_eq!(d, Decoded::SharedRef(CellRef { row: 27, col: 8 }));
    }

    #[test]
    fn row_shift_moves_only_row_relative_references() {
        let c = Context {
            xti_sheets: &[],
            names: &[],
            xti_names: &[],
            base: CellRef::default(),
            xlsb: false,
            row_shift: 2,
        };
        // A1(행·열 상대) + $B$1(절대)
        let d = decode(&hex("44 00 00 00 00 00 c0 44 00 00 00 00 01 00 03"), &c).unwrap();
        assert_eq!(d, Decoded::Text("A3+$B$1".into()));
    }

    #[test]
    fn three_d_reference() {
        let sheets = [Some("Sheet2".to_string())];
        let c = Context {
            xti_sheets: &sheets,
            names: &[],
            xti_names: &[],
            base: CellRef::default(),
            xlsb: false,
            row_shift: 0,
        };
        let d = decode(&hex("5a 00 00 00 00 00 00 00 c0 1e 02 00 05"), &c).unwrap();
        assert_eq!(d, Decoded::Text("Sheet2!A1*2".into()));
        assert_eq!(quote_sheet("첫 시트"), "'첫 시트'");
        assert_eq!(quote_sheet("둘째"), "둘째");
    }
}
