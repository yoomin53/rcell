// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 `.cell` → [`Workbook`] 변환.
//!
//! 레코드 의미는 BIFF8 공개 명세 [MS-XLS] 를 따르되, 한셀 레거시(BOF 버전 0x1001)의
//! 차이는 한셀이 같은 문서를 XML 형식으로 저장한 결과와 대조해 확정했다.
//! 차이 목록: `docs/format/cell-legacy.md`.

use std::collections::HashMap;

use super::biff::{self, Record};
use super::escher;
use super::formula::{self, Context, Decoded};
use crate::error::{Error, Result};
use crate::model::{
    AbstractNum, Alignment, Anchor, AnchorKind, Border, BorderEdge, Cell, CellRange, CellRef,
    CellValue, CfRule, Cfvo, ColInfo, Color, ColorScale, Comment, CommentShape, ConditionalFormat,
    DYNAMIC_FILTER_TYPES, DataBar, DefinedName, DrawColor, DrawingPara, DrawingRun, ExternalLink,
    FILTER_OPERATORS, Fill, FilterColumn, Font, Formula, FormulaKind, GradientFill, GradientStop,
    HancomFontExt, HancomXfExt, HeaderFooter, Hyperlink, IconSet, Marker, NamedStyle, NumFmt,
    Outline, PageMargins, PageSetup, Paint, Pane, ParaShape, Protection, RichText, RowInfo,
    SHEET_PROTECTION_FLAGS, Shape, ShapeKind, Sheet, SheetKind, SheetProtection, SheetState,
    Styles, TableStyle, TableStyles, TextBody, TextRun, Workbook, WorkbookProtection, Xf,
};
use crate::vocab;

/// 한셀 레거시 BOF 버전.
pub const HANCELL_BIFF_VERSION: u16 = 0x1001;
/// 시트 크기 (한셀 2024: 1,048,576행 × 16,384열)
const MAX_ROWS: u32 = 1_048_576;
const MAX_COLS: u32 = 16_384;

/// SUPBOOK 종류
#[derive(Clone, Copy)]
enum Book {
    Internal,
    AddIn,
    /// 외부 문서: `Workbook::external_links` 번호 (1부터)
    External(usize),
}

/// SUPBOOK 가상 경로 ([MS-XLS] 2.5.277 VirtualPath) → 관계 Target (URI) 꼴.
/// U+0001 로 시작하면 부호화된 경로다: U+0001 드라이브 문자(`@` 는 UNC),
/// U+0002 같은 드라이브의 루트, U+0003 하위 폴더, U+0004 상위 폴더.
fn virtual_path(p: &str) -> String {
    fn push(c: char, out: &mut String) {
        if c.is_ascii_alphanumeric() || "-._~/:()!$&'*+,;=@".contains(c) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    let mut out = String::new();
    let Some(rest) = p.strip_prefix('\u{1}') else {
        for c in p.chars() {
            push(if c == '\\' { '/' } else { c }, &mut out);
        }
        return out;
    };
    let mut it = rest.chars();
    while let Some(c) = it.next() {
        match c {
            '\u{1}' => match it.next() {
                Some('@') => out.push_str("file://"),
                Some(d) => {
                    out.push_str("file:///");
                    out.push(d);
                    out.push_str(":/");
                }
                None => {}
            },
            '\u{2}' | '\u{3}' => out.push('/'),
            '\u{4}' => out.push_str("../"),
            '\u{5}' => {
                // 긴 볼륨 이름: 길이 문자 + 이름
                let n = it.next().map_or(0, |c| c as usize);
                out.push_str("file:///");
                for c in it.by_ref().take(n) {
                    push(c, &mut out);
                }
                out.push('/');
            }
            '\u{6}'..='\u{8}' => {}
            c => push(c, &mut out),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 바이트 읽기
// ---------------------------------------------------------------------------

struct Rd<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Rd { b, pos: 0 }
    }
    fn left(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        let s = self.take(2)?;
        Some(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.take(4)?;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// 한셀 레거시 문자열: 플래그 값과 상관없이 UTF-16LE.
    fn utf16(&mut self, cch: usize) -> Option<String> {
        Some(crate::text::utf16le(self.take(cch * 2)?))
    }
}

/// CF 레코드 DXFN 의 표준 부분 ([MS-XLS] DXFN) 에서 0x088D 표에 없을 수 있는 것들
#[derive(Default)]
struct Dxfn {
    num_fmt: Option<String>,
    /// 내장 표시 형식 번호 (DXFNumIFmt)
    num_fmt_id: Option<u32>,
    alignment: Option<Alignment>,
    protection: Option<Protection>,
    /// CFEX 의 XFExtNoFRT 에만 있는 그라데이션
    gradient: Option<GradientFill>,
}

/// DXFN: 플래그 u32 + u16, 그다음 플래그가 켠 블록만 순서대로 — 표시 형식(가변), 글꼴 118, 맞춤 8, 테두리 8,
/// 무늬 4, 보호 2. 한컴 확장(글꼴 이름·RGB 색 등)은 표준 블록 뒤에 붙는다 (관측).
fn parse_dxfn(d: &[u8]) -> Option<Dxfn> {
    let mut r = Rd::new(d);
    let flags = r.u32()?;
    let flags2 = r.u16()?;
    let mut out = Dxfn::default();
    if flags & 0x0200_0000 != 0 {
        if flags2 & 0x0001 != 0 {
            // DXFNumUsr: cb u16 (자신 포함), cch u16, 플래그 u8, 문자 (플래그 0 이면 1바이트씩)
            let cb = r.u16()? as usize;
            let mut nr = Rd::new(r.take(cb.checked_sub(2)?)?);
            let cch = nr.u16()? as usize;
            let wide = nr.u8()? & 0x01 != 0;
            let code = if wide {
                nr.utf16(cch)
            } else {
                nr.take(cch).map(|b| b.iter().map(|&c| c as char).collect())
            };
            out.num_fmt = code.filter(|c| !c.is_empty());
        } else {
            // DXFNumIFmt: 예약 u8, 내장 번호 u8 (차등 서식 대조 실험 3 — `#,##0` → 3)
            let b = r.take(2)?;
            out.num_fmt_id = Some(b[1] as u32);
        }
    }
    if flags & 0x0400_0000 != 0 {
        r.take(118)?;
    }
    if flags & 0x0800_0000 != 0 {
        // DXFALC: 가로 3비트·줄 바꿈·세로 3비트, 회전 u8, 들여쓰기 4비트·축소·병합·읽기 순서 2비트, 예약 u8,
        // iIndent i32. 플래그의 낮은 비트들이 '지정 안 함'(Ninch) 이다.
        // 한셀은 가로·세로 맞춤 번호를 표준보다 1 크게 쓴다 — 가로 일반 1 … 가운데 맞춤 7, 분산은 8 이 3비트에서
        // 넘쳐 0, 세로 위쪽 1 … 분산 5 (맞춤 대조 실험). 셀 서식(XF) 의 맞춤은 표준 그대로다
        let b = r.take(8)?;
        let mut a = Alignment::default();
        if flags & 0x01 == 0 {
            a.horizontal = H_ALIGN
                .get(((b[0] & 7) as usize + 7) % 8)
                .map(|s| s.to_string());
        }
        if flags & 0x02 == 0 {
            a.vertical = (((b[0] >> 4) & 7) as usize)
                .checked_sub(1)
                .and_then(|v| V_ALIGN.get(v))
                .map(|s| s.to_string());
        }
        if flags & 0x04 == 0 {
            a.wrap_text = b[0] & 0x08 != 0;
        }
        if flags & 0x08 == 0 {
            a.text_rotation = Some(b[1] as i32);
        }
        if flags & 0x20 == 0 {
            a.indent = Some((b[2] & 0x0F) as u32).filter(|&i| i != 0);
        }
        if flags & 0x40 == 0 {
            a.shrink_to_fit = b[2] & 0x10 != 0;
        }
        if flags & 0x8000_0000 == 0 {
            a.reading_order = Some(((b[2] >> 6) & 3) as u32).filter(|&o| o != 0);
        }
        out.alignment = Some(a);
    }
    if flags & 0x1000_0000 != 0 {
        r.take(8)?;
    }
    if flags & 0x2000_0000 != 0 {
        r.take(4)?;
    }
    if flags & 0x4000_0000 != 0 {
        let p = r.u16()?;
        out.protection = Some(Protection {
            locked: (flags & 0x0100 == 0).then_some(p & 0x01 != 0),
            hidden: (flags & 0x0200 == 0).then_some(p & 0x02 != 0),
        });
    }
    Some(out)
}

/// ExtPropGradientFill ([MS-XLS] XFExtGradient): 종류 u32 (0 직선, 1 경로), 각도·왼쪽·오른쪽·위·아래 f64,
/// 정지점 수 u32, 정지점마다 색 종류 u16 + 값 u32 + 위치 f64 + 밝기 f64 (한셀 확인: 그라데이션 대조 실험)
fn ext_gradient(r: &mut Rd) -> Option<GradientFill> {
    let ty = r.u32()?;
    let mut g = GradientFill {
        path: ty == 1,
        degree: r.f64()?,
        left: r.f64()?,
        right: r.f64()?,
        top: r.f64()?,
        bottom: r.f64()?,
        stops: Vec::new(),
    };
    let n = r.u32()?;
    for _ in 0..n.min(256) {
        let kind = r.u16()?;
        let v = r.u32()?;
        let position = r.f64()?;
        let tint = r.f64()?;
        let tint = (tint != 0.0).then_some(tint);
        let color = match kind {
            0 => Color {
                auto: true,
                ..Default::default()
            },
            1 => Color {
                indexed: Some(v),
                tint,
                ..Default::default()
            },
            3 => Color {
                theme: Some(v),
                tint,
                ..Default::default()
            },
            _ => Color {
                rgb: Some(format!(
                    "ff{:02x}{:02x}{:02x}",
                    v & 0xFF,
                    (v >> 8) & 0xFF,
                    (v >> 16) & 0xFF
                )),
                tint,
                ..Default::default()
            },
        };
        g.stops.push(GradientStop { position, color });
    }
    Some(g)
}

/// CFEX 의 DXFN12 안 XFExtNoFRT (예약 u16 0, 0xFFFF, 0, cexts u16, ExtProp 들) 에서 그라데이션(종류 6) 을 찾는다.
/// 앞의 DXFN 은 한컴 확장 길이를 알 수 없어 머리 모양으로 찾는다
fn cfex_gradient(d: &[u8]) -> Option<GradientFill> {
    (0..d.len().saturating_sub(12)).find_map(|i| {
        let w = &d[i..];
        if w[..6] != [0, 0, 0xFF, 0xFF, 0, 0] {
            return None;
        }
        let n = u16::from_le_bytes([w[6], w[7]]);
        if n == 0 || n > 32 {
            return None;
        }
        let mut r = Rd::new(&w[8..]);
        for _ in 0..n {
            let ty = r.u16()?;
            let cb = r.u16()? as usize;
            let v = r.take(cb.checked_sub(4)?)?;
            if ty == 6 {
                return ext_gradient(&mut Rd::new(v));
            }
        }
        None
    })
}

/// CF 레코드의 차등 서식 번호: 수식 바로 앞 u32. DXFN 에 테두리 블록이 있으면 번호와 수식 사이에 한컴 안쪽 선
/// 확장이 온다 — 가로·세로 선마다 있음 u8, 있으면 선 종류 u8 (1–13) + RGBA 4바이트, 곧 2·7·12 바이트 (관측:
/// 안쪽 선 대조 실험). 검은 선은 색 바이트가 모두 0 이라 더 짧은 (틀린) 나눔도 구조가 맞는다 — 틀린 후보는 언제나
/// 참 후보보다 짧으므로, 번호가 가리키는 차등 서식(0x088D)의 안쪽 선과 맞는 후보 중 가장 긴 것, 없으면 가장 긴
/// 후보를 고른다.
fn cf_dxf_id(d: &[u8], fstart: usize, has_border: bool, dxfs: &[crate::model::Dxf]) -> Option<u32> {
    let id_at = |end: usize| -> Option<u32> {
        let x = d.get(end.checked_sub(4)?..end)?;
        Some(u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
    };
    if !has_border {
        return id_at(fstart);
    }
    let ndxf = dxfs.len();
    let plausible =
        |id: u32| id == u32::MAX || (id as usize) < ndxf || (ndxf == 0 && id < 0x1_0000);
    // 구조가 맞는 후보: (번호, [가로, 세로] 선 종류)
    let cands: Vec<(u32, [Option<u8>; 2])> = [2usize, 7, 12]
        .into_iter()
        .filter_map(|l| {
            let ext = d.get(fstart.checked_sub(l)?..fstart)?;
            let mut i = 0;
            let mut lines = [None; 2];
            for line in &mut lines {
                match *ext.get(i)? {
                    0 => i += 1,
                    1 => {
                        let ty = *ext.get(i + 1)?;
                        if !(1..=13).contains(&ty) {
                            return None;
                        }
                        *line = Some(ty);
                        i += 6;
                    }
                    _ => return None,
                }
            }
            if i != l {
                return None;
            }
            Some((id_at(fstart - l).filter(|&id| plausible(id))?, lines))
        })
        .collect();
    let style = |e: &Option<BorderEdge>| {
        e.as_ref()
            .and_then(|e| e.style.as_deref())
            .and_then(|s| BORDER_STYLES.iter().position(|x| *x == s))
            .map(|p| p as u8)
    };
    let agrees = |c: &&(u32, [Option<u8>; 2])| {
        dxfs.get(c.0 as usize)
            .and_then(|x| x.border.as_ref())
            .is_some_and(|b| style(&b.horizontal) == c.1[0] && style(&b.vertical) == c.1[1])
    };
    cands
        .iter()
        .rev()
        .find(agrees)
        .or(cands.last())
        .map(|c| c.0)
}

fn bad(what: &str) -> Error {
    Error::InvalidValue {
        context: "레거시 .cell".into(),
        value: what.into(),
    }
}

// ---------------------------------------------------------------------------
// 논리 레코드 (CONTINUE 병합)
// ---------------------------------------------------------------------------

pub(crate) struct Logical {
    pub(crate) rtype: u16,
    pub(crate) offset: usize,
    pub(crate) data: Vec<u8>,
}

pub(crate) fn logical_records(stream: &[u8]) -> Vec<Logical> {
    let mut out: Vec<Logical> = Vec::new();
    for Record {
        offset,
        rtype,
        data,
    } in biff::records(stream)
    {
        if rtype == 0x003C
            && let Some(last) = out.last_mut()
        {
            last.data.extend_from_slice(data);
            continue;
        }
        out.push(Logical {
            rtype,
            offset,
            data: data.to_vec(),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// 색
// ---------------------------------------------------------------------------

/// BIFF8 기본 팔레트 (색 번호 8–63), [MS-XLS] 2.5.161.
pub(crate) const DEFAULT_PALETTE: [u32; 56] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000,
    0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366,
    0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00,
    0x00FFFF, 0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99,
    0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900,
    0xFF6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366,
    0x333399, 0x333333,
];

/// 한셀 확장 영역의 COLORREF(0x00BBGGRR). `0xFFFFFFFF` = 지정 안 함, `0xDFFFFFFF` = 자동.
fn colorref(v: u32) -> Option<Color> {
    match v {
        0xFFFF_FFFF => None,
        0xDFFF_FFFF => Some(Color {
            indexed: Some(64),
            ..Default::default()
        }),
        // 색인 65 (시스템 전경): 피벗 표 서식의 테두리 — XML `indexed="65"` 가 레거시에서 이 값이고, 한셀의 레거시 읽기는
        // 다시 색인 65 로 읽는다 (피벗 실험 pa…: 한셀의 바이너리 저장은 같은 값의 아래 세 바이트를 RGB 020080 으로 쓴다)
        0x8080_0002 => Some(Color {
            indexed: Some(65),
            ..Default::default()
        }),
        _ => {
            let r = v & 0xFF;
            let g = (v >> 8) & 0xFF;
            let b = (v >> 16) & 0xFF;
            Some(Color {
                rgb: Some(format!("ff{r:02x}{g:02x}{b:02x}")),
                ..Default::default()
            })
        }
    }
}

fn palette_color(icv: u16, palette: &[u32]) -> Option<Color> {
    match icv {
        8..=63 => palette.get(icv as usize - 8).map(|rgb| Color {
            rgb: Some(format!("ff{rgb:06x}")),
            ..Default::default()
        }),
        64 | 0x7FFF => Some(Color {
            auto: true,
            ..Default::default()
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// FONT
// ---------------------------------------------------------------------------

fn parse_font(data: &[u8], palette: &[u32]) -> Option<Font> {
    let mut r = Rd::new(data);
    let height = r.u16()?;
    let grbit = r.u16()?;
    let icv = r.u16()?;
    let bls = r.u16()?;
    let sss = r.u16()?;
    let uls = r.u8()?;
    let family = r.u8()?;
    let charset = r.u8()?;
    let _reserved = r.u8()?;
    let cch = r.u8()? as usize;
    let _flag = r.u8()?;
    let name = r.utf16(cch)?;

    let mut font = Font {
        name: Some(name),
        size: Some(height as f64 / 20.0),
        bold: bls >= 700,
        italic: grbit & 0x02 != 0,
        strike: grbit & 0x08 != 0,
        outline: grbit & 0x10 != 0,
        shadow: grbit & 0x20 != 0,
        condense: grbit & 0x40 != 0,
        extend: grbit & 0x80 != 0,
        underline: match uls {
            0 => None,
            1 => Some("single".into()),
            2 => Some("double".into()),
            0x21 => Some("singleAccounting".into()),
            0x22 => Some("doubleAccounting".into()),
            _ => Some("single".into()),
        },
        vert_align: match sss {
            1 => Some("superscript".into()),
            2 => Some("subscript".into()),
            _ => None,
        },
        family: (family != 0).then_some(family as u32),
        charset: Some(charset as u32),
        color: palette_color(icv, palette),
        ..Default::default()
    };

    // 한셀 확장: 플래그 1 + 78바이트
    if r.u8() == Some(1) && r.left() >= 78 {
        let ext = Rd::new(r.take(78)?);
        font.hancom = parse_font_ext(ext, &mut font);
    }
    Some(font)
}

/// FONT 확장 78바이트. 오프셋은 docs/format/cell-legacy.md 의 표를 따른다.
fn parse_font_ext(mut e: Rd, font: &mut Font) -> Option<HancomFontExt> {
    let size100 = e.u32()?;
    let flags = e.u16()?;
    let ul_type = e.u16()?;
    let _w8 = e.u16()?;
    let ul_shape = e.u32()?;
    let outline = e.u32()?;
    let shadow = e.u32()?;
    let sh_x = e.i32()?;
    let sh_y = e.i32()?;
    let so_type = e.u32()?;
    let diacsym = e.u32()?;
    let so_shape = e.u32()?;
    let text_color = e.u32()?;
    let ul_color = e.u32()?;
    let shade = e.u32()?;
    let sh_color = e.u32()?;
    let rel_size = e.i32()?;
    let ratio = e.i32()?;
    let spacing = e.i32()?;
    let offset = e.i32()?;
    let so_color = e.u32()?;

    // 1/100 pt 크기가 더 정밀하다 (7.5pt 같은 값)
    font.size = Some(size100 as f64 / 100.0);
    if text_color != 0xFFFF_FFFF
        && let Some(c) = colorref(text_color)
    {
        font.color = Some(c);
    }

    let pick = |v: &'static [&'static str], i: u32| v.get(i as usize).map(|s| s.to_string());
    let mut h = HancomFontExt {
        size: Some(rel_size),
        ratio: Some(ratio),
        spacing: Some(spacing),
        offset: Some(offset),
        emboss: (flags & 0x01 != 0).then_some(true),
        engrave: (flags & 0x02 != 0).then_some(true),
        use_kerning: (flags & 0x10 != 0).then_some(true),
        superscript: (font.vert_align.as_deref() == Some("superscript")).then_some(true),
        subscript: (font.vert_align.as_deref() == Some("subscript")).then_some(true),
        ..Default::default()
    };
    if ul_type != 0 {
        // 밑줄 위치: 1 = 아래, 3 = 위(윗줄). 윗줄은 표준 밑줄(x:u)이 아니다 (한셀 XML 과 같게).
        if ul_type != 1 {
            font.underline = None;
        }
        h.underline_type = Some(ul_type.to_string());
        h.underline_shape = pick(vocab::LINE_SHAPE, ul_shape);
        h.underline_color = colorref(ul_color);
    }
    if so_type != 0 {
        h.strikeout_type = Some(so_type.to_string());
        h.strikeout_shape = pick(vocab::LINE_SHAPE, so_shape);
        h.strikeout_color = colorref(so_color);
    }
    if outline != 0 {
        h.outline_type = pick(vocab::OUTLINE_TYPE, outline);
        // 한셀은 외곽선 종류가 있으면 (실선이 아니어도) ECMA 외곽선(x:outline)도 켠다.
        // FONT 의 grbit 0x10 은 실선일 때만 켜져 있다 (글꼴 대조 실험)
        font.outline = true;
    }
    if shadow != 0 {
        h.shadow_type = pick(vocab::SHADOW_TYPE, shadow);
        h.shadow_color = colorref(sh_color);
        h.shadow_offset_x = Some(sh_x);
        h.shadow_offset_y = Some(sh_y);
    }
    if diacsym != 0 {
        h.diacsym_mark = pick(vocab::DIACSYM_MARK, diacsym);
    }
    h.shade_color = colorref(shade);
    Some(h)
}

// ---------------------------------------------------------------------------
// XF
// ---------------------------------------------------------------------------

struct RawXf {
    style: bool,
    parent: u16,
    xf: Xf,
}

pub(crate) const BORDER_STYLES: [&str; 14] = [
    "none",
    "thin",
    "medium",
    "dashed",
    "dotted",
    "thick",
    "double",
    "hair",
    "mediumDashed",
    "dashDot",
    "mediumDashDot",
    "dashDotDot",
    "mediumDashDotDot",
    "slantDashDot",
];

pub(crate) const PATTERNS: [&str; 19] = [
    "none",
    "solid",
    "mediumGray",
    "darkGray",
    "lightGray",
    "darkHorizontal",
    "darkVertical",
    "darkDown",
    "darkUp",
    "darkGrid",
    "darkTrellis",
    "lightHorizontal",
    "lightVertical",
    "lightDown",
    "lightUp",
    "lightGrid",
    "lightTrellis",
    "gray125",
    "gray0625",
];

pub(crate) const H_ALIGN: [&str; 8] = [
    "general",
    "left",
    "center",
    "right",
    "fill",
    "justify",
    "centerContinuous",
    "distributed",
];
pub(crate) const V_ALIGN: [&str; 5] = ["top", "center", "bottom", "justify", "distributed"];

struct XfParts {
    raw: RawXf,
    fill: Fill,
    border: Border,
    /// 확장 개수가 3이면 붙는 문단 모양 참조 (0xA011 표의 번호)
    para_ref: Option<u32>,
}

fn parse_xf(data: &[u8], palette: &[u32]) -> Option<XfParts> {
    let mut r = Rd::new(data);
    let ifnt = r.u16()?;
    let ifmt = r.u16()?;
    let prot = r.u16()?;
    let align = r.u8()?;
    let trot = r.u8()?;
    let indent_b = r.u8()?;
    let used = r.u8()?;
    let border1 = r.u32()?;
    let border2 = r.u32()?;
    let colors = r.u16()?;

    // 한셀 확장: u16 개수 + COLORREF 8개 (+ 개수 3이면 u32 하나 더)
    let mut ext = [0xFFFF_FFFFu32; 8];
    let mut para_ref = None;
    if let Some(n) = r.u16() {
        // 개수 3: 문단 모양 번호(u32)가 색 8개 앞에 온다 (관측)
        if n >= 3 {
            para_ref = r.u32();
        }
        for slot in ext.iter_mut() {
            match r.u32() {
                Some(v) => *slot = v,
                None => break,
            }
        }
    }

    let style = prot & 0x0004 != 0;
    let alc = (align & 0x07) as usize;
    let alcv = ((align >> 4) & 0x07) as usize;
    let alignment = Alignment {
        horizontal: H_ALIGN.get(alc).map(|s| s.to_string()),
        vertical: V_ALIGN.get(alcv).map(|s| s.to_string()),
        wrap_text: align & 0x08 != 0,
        shrink_to_fit: indent_b & 0x10 != 0,
        text_rotation: (trot != 0).then_some(trot as i32),
        indent: ((indent_b & 0x0F) != 0).then_some((indent_b & 0x0F) as u32),
        reading_order: ((indent_b >> 6) != 0).then_some((indent_b >> 6) as u32),
    };

    let edge = |dg: u32, icv: u32, rgb: u32| -> Option<BorderEdge> {
        if dg == 0 {
            // 선 없는 가장자리에도 한셀은 색을 남긴다 (XML 의 `<x:left><x:color …/></x:left>`, 10절)
            return colorref(rgb)
                .filter(|_| rgb != 0xFFFF_FFFF)
                .map(|c| BorderEdge {
                    style: None,
                    color: Some(c),
                });
        }
        Some(BorderEdge {
            style: BORDER_STYLES.get(dg as usize).map(|s| s.to_string()),
            color: colorref(rgb)
                .filter(|c| c.rgb.is_some() || c.indexed == Some(65))
                .or_else(|| palette_color(icv as u16, palette)),
        })
    };
    let grbit_diag = border1 >> 30;
    let border = Border {
        left: edge(border1 & 0xF, (border1 >> 16) & 0x7F, ext[3]),
        right: edge((border1 >> 4) & 0xF, (border1 >> 23) & 0x7F, ext[5]),
        top: edge((border1 >> 8) & 0xF, border2 & 0x7F, ext[2]),
        bottom: edge((border1 >> 12) & 0xF, (border2 >> 7) & 0x7F, ext[4]),
        diagonal: edge((border2 >> 21) & 0xF, (border2 >> 14) & 0x7F, ext[6]),
        vertical: None,
        horizontal: None,
        diagonal_up: grbit_diag & 0x2 != 0,
        diagonal_down: grbit_diag & 0x1 != 0,
    };

    let fls = (border2 >> 26) as usize;
    let icv_fore = colors & 0x7F;
    let icv_back = (colors >> 7) & 0x7F;
    let fill = if fls == 0 {
        Fill {
            pattern_type: Some("none".into()),
            ..Default::default()
        }
    } else {
        Fill {
            pattern_type: PATTERNS.get(fls).map(|s| s.to_string()),
            fg_color: colorref(ext[1])
                .filter(|c| c.rgb.is_some())
                .or_else(|| palette_color(icv_fore, palette)),
            bg_color: colorref(ext[0]).or_else(|| palette_color(icv_back, palette)),
            gradient: None,
        }
    };

    let xf = Xf {
        num_fmt_id: ifmt as u32,
        font_id: ifnt as u32,
        fill_id: 0,
        border_id: 0,
        xf_id: None,
        apply_number_format: Some(used & 0x04 != 0),
        apply_font: Some(used & 0x08 != 0),
        apply_alignment: Some(used & 0x10 != 0),
        apply_border: Some(used & 0x20 != 0),
        apply_fill: Some(used & 0x40 != 0),
        apply_protection: Some(used & 0x80 != 0),
        alignment: Some(alignment),
        protection: Some(Protection {
            locked: Some(prot & 0x0001 != 0),
            hidden: Some(prot & 0x0002 != 0),
        }),
        // BIFF8 XF: 0x0004 fStyle, 0x0008 f123Prefix (= quotePrefix)
        quote_prefix: prot & 0x0008 != 0,
        hancom: None,
    };
    Some(XfParts {
        raw: RawXf {
            style,
            parent: prot >> 4,
            xf,
        },
        fill,
        border,
        para_ref,
    })
}

// ---------------------------------------------------------------------------
// 문자열
// ---------------------------------------------------------------------------

/// SST 문자열. BIFF8 의 XLUnicodeRichExtendedString 과 다르다 (관측):
/// `cch u16, flags u8, UTF-16 문자[cch]`, flags 비트 0 이 서 있으면 문자 **뒤에**
/// `cRun u16` 과 `(ich u16, ifnt u16) × cRun` 이 온다.
fn read_sst_string(r: &mut Rd, fonts: &[Font]) -> Option<RichText> {
    let cch = r.u16()? as usize;
    let flags = r.u8()?;
    let text = r.utf16(cch)?;
    let runs = if flags & 0x01 != 0 {
        r.u16()? as usize
    } else {
        0
    };
    let mut rt = RichText {
        text: text.clone(),
        runs: Vec::new(),
    };
    if runs > 0 {
        // FormatRun: ich u16, ifnt u16
        let mut marks = Vec::with_capacity(runs);
        for _ in 0..runs {
            let ich = r.u16()? as usize;
            let ifnt = r.u16()? as usize;
            marks.push((ich, ifnt));
        }
        let chars: Vec<char> = text.chars().collect();
        // 첫 런 앞부분은 셀 글꼴
        let mut segments: Vec<(usize, usize, Option<usize>)> = Vec::new();
        let mut start = 0;
        let mut cur: Option<usize> = None;
        for &(ich, ifnt) in &marks {
            if ich > start {
                segments.push((start, ich.min(chars.len()), cur));
            }
            start = ich.min(chars.len());
            cur = Some(ifnt);
        }
        if start < chars.len() {
            segments.push((start, chars.len(), cur));
        }
        rt.runs = segments
            .into_iter()
            .map(|(a, b, f)| TextRun {
                text: chars[a..b].iter().collect(),
                font: f.and_then(|i| fonts.get(i).cloned()),
            })
            .collect();
    }
    Some(rt)
}

// ---------------------------------------------------------------------------
// 통합 문서
// ---------------------------------------------------------------------------

struct BoundSheet {
    offset: u32,
    state: SheetState,
    kind: SheetKind,
    name: String,
}

struct PendingName {
    name: String,
    local: Option<u32>,
    hidden: bool,
    rgce: Vec<u8>,
    /// 설명 글 (XML `comment`)
    comment: Option<String>,
}

/// 레거시 `/Workbook` 스트림을 통합 문서 모델로 읽는다.
pub fn read_workbook_stream(stream: &[u8]) -> Result<Workbook> {
    let recs = logical_records(stream);
    let first = recs.first().ok_or_else(|| bad("빈 Workbook 스트림"))?;
    if first.rtype != 0x0809 || first.data.len() < 4 {
        return Err(bad("BOF 레코드가 없습니다"));
    }
    let version = u16::from_le_bytes([first.data[0], first.data[1]]);
    if version != HANCELL_BIFF_VERSION {
        return Err(bad(&format!(
            "BIFF 버전 0x{version:04x} — 한셀 레거시(0x1001)가 아닙니다 (.xls 는 아직 미지원)"
        )));
    }

    let mut wb = Workbook {
        application: Some("Cell".into()),
        ..Default::default()
    };
    let mut palette: Vec<u32> = DEFAULT_PALETTE.to_vec();
    let mut fonts: Vec<Font> = Vec::new();
    let mut formats: Vec<NumFmt> = Vec::new();
    let mut xfs: Vec<XfParts> = Vec::new();
    let mut styles_named: Vec<(u16, String, Option<u32>, Option<u32>)> = Vec::new();
    let mut sheets: Vec<BoundSheet> = Vec::new();
    let mut sst: Vec<RichText> = Vec::new();
    let mut supbooks: Vec<Book> = Vec::new();
    // SUPBOOK 별 EXTERNNAME (추가 기능 함수 이름, 외부 문서의 이름)
    let mut supbook_names: Vec<Vec<String>> = Vec::new();
    let mut xti: Vec<(u16, u16, u16)> = Vec::new();
    let mut pending_names: Vec<PendingName> = Vec::new();
    let mut parashapes: Vec<ParaShape> = Vec::new();
    let mut dxf_extra: Vec<(u32, Dxfn)> = Vec::new();
    let mut dxfs: Vec<crate::model::Dxf> = Vec::new();
    let mut table_styles: Option<TableStyles> = None;
    let mut abstract_nums: Vec<AbstractNum> = Vec::new();
    // XFEXT 의 그라데이션 채우기 (XF 번호별)
    let mut xf_gradients: HashMap<u16, GradientFill> = HashMap::new();
    let mut para_refs: Vec<u32> = Vec::new();
    // MSODRAWINGGROUP: OfficeArtDggContainer (그림 저장소)
    let mut dgg: Vec<u8> = Vec::new();
    // 표 (0xB005…0xB00D), 그리고 요약 행 수식 (표 번호, 열 번호, 토큰열) — 표를 모두 읽은 뒤 글로 바꾼다
    let mut tables: Vec<crate::model::Table> = Vec::new();
    let mut table_formulas: Vec<(usize, usize, Vec<u8>)> = Vec::new();

    // 전역 부분: 첫 EOF 까지
    for rec in recs.iter().skip(1) {
        let d = &rec.data[..];
        let mut r = Rd::new(d);
        match rec.rtype {
            0x000A => break,
            0x0022 => wb.date1904 = r.u16() == Some(1),
            0x003D if wb.view.is_none() => {
                // WINDOW1 (BIFF8 그대로): xWn i16, yWn i16, dxWn u16, dyWn u16, grbit u16 (0x08 가로 스크롤,
                // 0x10 세로 스크롤, 0x20 시트 탭), itabCur u16, itabFirst u16, ctabSel u16, wTabRatio u16
                let (Some(x), Some(y), Some(w), Some(h), Some(f), Some(cur), Some(first)) = (
                    r.u16(),
                    r.u16(),
                    r.u16(),
                    r.u16(),
                    r.u16(),
                    r.u16(),
                    r.u16(),
                ) else {
                    continue;
                };
                let _sel = r.u16();
                wb.view = Some(crate::model::WorkbookView {
                    x_window: x as i16 as i32,
                    y_window: y as i16 as i32,
                    window_width: w as u32,
                    window_height: h as u32,
                    tab_ratio: r.u16().map_or(600, u32::from),
                    first_sheet: first as u32,
                    active_tab: cur as u32,
                    show_horizontal_scroll: f & 0x08 != 0,
                    show_vertical_scroll: f & 0x10 != 0,
                    show_sheet_tabs: f & 0x20 != 0,
                });
            }
            0x01BA => {
                // 전역의 CODENAME 은 통합 문서의 코드 이름 (시트 것은 시트 BOF 뒤에 있다)
                let cch = r.u16().unwrap_or(0) as usize;
                let _flag = r.u8();
                wb.code_name = r.utf16(cch).filter(|s| !s.is_empty());
            }
            0x00EB => dgg.extend_from_slice(d),
            // 표 (한컴): 0xB005 = BrtBeginList, 0xB00B = BrtBeginListCol (+ 요약 행 수식), 0xB00C = BrtTableStyleClient,
            // 0xB00D 끝 — 바이너리 표 파트와 같은 꼴이다. 어느 시트의 표인지는 시트의 0xB00A 가 정한다 (표 대조 실험)
            0xB005 => tables.extend(crate::binary::reader::begin_list(d)),
            0xB00B => {
                if let Some((c, totals)) = crate::binary::reader::list_col(d, true)
                    && let Some(i) = tables.len().checked_sub(1)
                {
                    if let Some(rgce) = totals {
                        table_formulas.push((i, tables[i].columns.len(), rgce));
                    }
                    tables[i].columns.push(c);
                }
            }
            0xB00C => {
                if let Some(t) = tables.last_mut() {
                    t.style = crate::binary::reader::table_style_client(d);
                }
            }
            // 통합 문서 보호: PROTECT (구조), WINDOWPROTECT (창), PASSWORD
            0x0012 | 0x0019 | 0x0013 => {
                let v = r.u16().unwrap_or(0);
                if v != 0 {
                    let p = wb
                        .protection
                        .get_or_insert_with(WorkbookProtection::default);
                    match rec.rtype {
                        0x0012 => p.lock_structure = true,
                        0x0019 => p.lock_windows = true,
                        _ => p.password = Some(format!("{v:04X}")),
                    }
                }
            }
            0xA001 => parashapes.push(parse_parashape(d)),
            0xA011 => {
                let _kind = r.u32();
                para_refs.push(r.u32().unwrap_or(0));
            }
            0x0092 => {
                if let Some(n) = r.u16() {
                    palette.clear();
                    for _ in 0..n {
                        let Some(v) = r.u32() else { break };
                        palette.push(((v & 0xFF) << 16) | (v & 0xFF00) | ((v >> 16) & 0xFF));
                    }
                }
            }
            0x088D => {
                // 차등 서식 표 (한컴 DXF FRT): FRT 머리 12바이트 뒤 XLSB BrtDXF 와 같은 XFProp 목록
                if let Some(x) = d.get(12..) {
                    dxfs.push(crate::binary::reader::parse_dxf(x, true));
                }
            }
            0x088E => {
                // TABLESTYLES (FRT 머리 12): cts u32 (기본 제공 144 + 사용자 정의 수), 기본 표·피벗 스타일 이름
                // 길이 u16 ×2, 이름 두 개 (UTF-16, 플래그 바이트 없음)
                r.take(12);
                let _cts = r.u32();
                let (Some(n1), Some(n2)) = (r.u16(), r.u16()) else {
                    continue;
                };
                table_styles = Some(TableStyles {
                    default_table_style: r.utf16(n1 as usize),
                    default_pivot_style: r.utf16(n2 as usize),
                    styles: Vec::new(),
                });
            }
            0x088F => {
                // TABLESTYLE (FRT 머리 12): 플래그 u16 (0x02 피벗, 0x04 표), ctse u32, 이름 길이 u16, 이름
                r.take(12);
                let (Some(flags), Some(_ctse), Some(n)) = (r.u16(), r.u32(), r.u16()) else {
                    continue;
                };
                let name = r.utf16(n as usize).unwrap_or_default();
                table_styles
                    .get_or_insert_with(Default::default)
                    .styles
                    .push(TableStyle {
                        name,
                        pivot: flags & 0x02 != 0,
                        table: flags & 0x04 != 0,
                        elements: Vec::new(),
                    });
            }
            0x0890 => {
                // TABLESTYLEELEMENT (FRT 머리 12): 종류 u32, 줄무늬 폭 u32, 차등 서식(0x088D 순서) 번호 u32
                r.take(12);
                if let (Some(t), Some(size), Some(dxf)) = (r.u32(), r.u32(), r.u32())
                    && let Some(e) = crate::binary::reader::table_style_element(t, size, dxf)
                    && let Some(ts) = table_styles.as_mut().and_then(|x| x.styles.last_mut())
                {
                    ts.elements.push(e);
                }
            }
            0xA020 => {
                // 한컴 문단 번호 정의: 종류 u8, 시작 번호 u16, 길이 u16, 플래그 u8, 번호 모양 UTF-16 (`^1.`).
                // XML 의 abstractNumType 은 종류 + 1 (관측: 0 → 1), lvlText 는 `^` 를 `%5e` 로 쓴다
                let (Some(kind), Some(start), Some(n), Some(_flag)) =
                    (r.u8(), r.u16(), r.u16(), r.u8())
                else {
                    continue;
                };
                let text = r.utf16(n as usize).unwrap_or_default();
                abstract_nums.push(AbstractNum {
                    num_type: kind as u32 + 1,
                    id: abstract_nums.len() as u32,
                    start_num: start as u32,
                    lvl_text: text.replace('%', "%25").replace('^', "%5e"),
                });
            }
            0x087D => {
                // XFEXT (FRT 머리 12): 예약 u16, ixfe u16, 예약 u16, cexts u16, ExtProp(종류 u16, 크기 u16(머리 포함),
                // 값) × cexts. 한셀은 그라데이션 채우기(종류 6)만 여기에 쓴다 (색은 XF 의 한컴 확장에 있다)
                r.take(12);
                let (Some(_), Some(ixfe), Some(_), Some(n)) = (r.u16(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                for _ in 0..n {
                    let (Some(ty), Some(cb)) = (r.u16(), r.u16()) else {
                        break;
                    };
                    let Some(v) = r.take((cb as usize).saturating_sub(4)) else {
                        break;
                    };
                    if ty == 6
                        && let Some(g) = ext_gradient(&mut Rd::new(v))
                    {
                        xf_gradients.insert(ixfe, g);
                    }
                }
            }
            0xA101 => {
                // 한셀 계산 엔진 번호 (XML 의 calcPr/@hs:hclCalcId). 1 은 값이 없는 것으로 본다
                if let Some(v) = r.u32().filter(|&v| v != 1) {
                    wb.hancom.calc_id = Some(v);
                }
            }
            0x0031 => {
                if fonts.len() == 4 {
                    // BIFF 규칙: 글꼴 번호 4 는 비워 둔다
                    let phantom = fonts[0].clone();
                    fonts.push(phantom);
                }
                fonts.push(parse_font(d, &palette).ok_or_else(|| bad("FONT"))?);
            }
            0x041E => {
                let id = r.u16().ok_or_else(|| bad("FORMAT"))?;
                let cch = r.u16().unwrap_or(0) as usize;
                let _flag = r.u8();
                let code = r.utf16(cch).unwrap_or_default();
                formats.push(NumFmt {
                    id: id as u32,
                    code,
                });
            }
            0x00E0 => xfs.push(parse_xf(d, &palette).ok_or_else(|| bad("XF"))?),
            0x0293 => {
                let ixfe = r.u16().unwrap_or(0);
                if ixfe & 0x8000 != 0 {
                    // 내장: istyBuiltIn u8, iLevel u8 (0xFF 없음)
                    let builtin = r.u8().unwrap_or(0);
                    let lvl = r.u8().unwrap_or(0xFF);
                    styles_named.push((
                        ixfe & 0x0FFF,
                        builtin_style_name(builtin),
                        Some(builtin as u32),
                        (lvl != 0xFF && matches!(builtin, 1 | 2)).then_some(lvl as u32),
                    ));
                } else {
                    let cch = r.u16().unwrap_or(0) as usize;
                    let _flag = r.u8();
                    let name = r.utf16(cch).unwrap_or_default();
                    styles_named.push((ixfe & 0x0FFF, name, None, None));
                }
            }
            0x0085 => {
                let offset = r.u32().ok_or_else(|| bad("BOUNDSHEET"))?;
                let hs = r.u8().unwrap_or(0);
                let dt = r.u8().unwrap_or(0);
                let cch = r.u8().unwrap_or(0) as usize;
                let _flag = r.u8();
                let name = r.utf16(cch).unwrap_or_default();
                sheets.push(BoundSheet {
                    offset,
                    state: match hs & 0x03 {
                        1 => SheetState::Hidden,
                        2 => SheetState::VeryHidden,
                        _ => SheetState::Visible,
                    },
                    kind: match dt {
                        0 => SheetKind::Worksheet,
                        2 => SheetKind::Chartsheet,
                        other => SheetKind::Other(format!("biff-dt-{other}")),
                    },
                    name,
                });
            }
            0x00FC => {
                let _total = r.u32();
                let unique = r.u32().unwrap_or(0) as usize;
                for _ in 0..unique {
                    match read_sst_string(&mut r, &fonts) {
                        Some(s) => sst.push(s),
                        None => break,
                    }
                }
            }
            0x01AE => {
                // SUPBOOK: 0x0401 = 이 문서, 0x3A01 = 추가 기능, 그 밖 = 외부 문서 (경로 + 시트 이름)
                let ctab = r.u16().unwrap_or(0);
                let marker = r.u16().unwrap_or(0);
                supbook_names.push(Vec::new());
                supbooks.push(match marker {
                    0x0401 => Book::Internal,
                    0x3A01 => Book::AddIn,
                    cch => {
                        let _flag = r.u8();
                        let path = r.utf16(cch as usize).unwrap_or_default();
                        let mut sheet_names = Vec::new();
                        for _ in 0..ctab {
                            let Some(n) = r.u16() else { break };
                            let _flag = r.u8();
                            match r.utf16(n as usize) {
                                Some(s) => sheet_names.push(s),
                                None => break,
                            }
                        }
                        wb.external_links.push(ExternalLink {
                            target: virtual_path(&path),
                            sheet_names,
                            sheet_data: Vec::new(),
                            // 부호화된 가상 경로(U+0001 …)가 아니면 경로가 없는 연결 (한셀 해석)
                            path_missing: !path.starts_with(''),
                        });
                        Book::External(wb.external_links.len())
                    }
                });
            }
            0x0023 => {
                // EXTERNNAME (한컴): 옵션 u16, 예약 u32, cch u16, UTF-16 이름 (BIFF8 과 달리 플래그 바이트 없음).
                // 바로 앞 SUPBOOK 의 이름 목록에 붙는다 — PtgNameX 의 이름 번호(1부터)가 가리킨다
                r.take(6);
                let cch = r.u16().unwrap_or(0) as usize;
                let name = r.utf16(cch).unwrap_or_default();
                if let Some(v) = supbook_names.last_mut() {
                    v.push(name);
                }
            }
            0x0017 => {
                let n = r.u16().unwrap_or(0);
                for _ in 0..n {
                    let (Some(a), Some(b), Some(c)) = (r.u16(), r.u16(), r.u16()) else {
                        break;
                    };
                    xti.push((a, b, c));
                }
            }
            0x0018 => {
                let grbit = r.u16().unwrap_or(0);
                let _key = r.u8();
                let cch = r.u8().unwrap_or(0) as usize;
                let cce = r.u16().unwrap_or(0) as usize;
                let _res = r.u16();
                let itab = r.u16().unwrap_or(0);
                // 사용자 메뉴·설명·도움말·상태 표시줄 글의 길이 (u8 × 4) — 글은 수식 뒤에 (flag u8 + UTF-16)
                let lens: Vec<usize> = (0..4).map(|_| r.u8().unwrap_or(0) as usize).collect();
                let _flag = r.u8();
                let mut name = r.utf16(cch).unwrap_or_default();
                if grbit & 0x0020 != 0 {
                    // 내장 이름 (Print_Area 등): 첫 문자가 번호
                    name = builtin_name(name.chars().next().map(|c| c as u32).unwrap_or(0));
                }
                let rgce = r.take(cce).map(<[u8]>::to_vec).unwrap_or_default();
                let mut texts = lens.iter().map(|&n| {
                    (n > 0)
                        .then(|| {
                            r.u8()?;
                            r.utf16(n)
                        })
                        .flatten()
                });
                let comment = texts.nth(1).flatten();
                pending_names.push(PendingName {
                    name,
                    local: (itab != 0).then(|| itab as u32 - 1),
                    hidden: grbit & 0x0001 != 0,
                    rgce,
                    comment,
                });
            }
            _ => {}
        }
    }

    // 스타일 모델 구성: 셀 XF 와 스타일 XF 를 나눈다.
    let mut cell_map: HashMap<u16, u32> = HashMap::new();
    let mut style_map: HashMap<u16, u32> = HashMap::new();
    let mut styles = Styles {
        fonts,
        num_fmts: formats,
        dxfs,
        table_styles,
        abstract_nums,
        from_legacy: true,
        ..Default::default()
    };
    let mut fill_index: Vec<(String, u32)> = Vec::new();
    let mut border_index: Vec<(String, u32)> = Vec::new();
    for (i, parts) in xfs.iter().enumerate() {
        if parts.raw.style {
            style_map.insert(i as u16, styles.cell_style_xfs.len() as u32);
            styles.cell_style_xfs.push(parts.raw.xf.clone());
        }
    }
    for (i, mut parts) in xfs.into_iter().enumerate() {
        if let Some(g) = xf_gradients.remove(&(i as u16)) {
            parts.fill = Fill {
                gradient: Some(g),
                ..Default::default()
            };
        }
        let fill_key = format!("{:?}", parts.fill);
        let fill_id = match fill_index.iter().find(|(k, _)| *k == fill_key) {
            Some((_, id)) => *id,
            None => {
                let id = styles.fills.len() as u32;
                styles.fills.push(parts.fill);
                fill_index.push((fill_key, id));
                id
            }
        };
        let border_key = format!("{:?}", parts.border);
        let border_id = match border_index.iter().find(|(k, _)| *k == border_key) {
            Some((_, id)) => *id,
            None => {
                let id = styles.borders.len() as u32;
                styles.borders.push(parts.border);
                border_index.push((border_key, id));
                id
            }
        };
        let mut xf = parts.raw.xf;
        xf.fill_id = fill_id;
        xf.border_id = border_id;
        if let Some(k) = parts.para_ref.filter(|&k| k != u32::MAX) {
            let idx = para_refs
                .get(k as usize)
                .copied()
                .unwrap_or(k.saturating_add(1)) as usize;
            if let Some(ps) = parashapes.get(idx) {
                attach_parashape(&mut xf, ps.clone());
            }
        }
        if parts.raw.style {
            let idx = style_map[&(i as u16)] as usize;
            styles.cell_style_xfs[idx].fill_id = fill_id;
            styles.cell_style_xfs[idx].border_id = border_id;
        } else {
            xf.xf_id = style_map.get(&parts.raw.parent).copied();
            cell_map.insert(i as u16, styles.cell_xfs.len() as u32);
            styles.cell_xfs.push(xf);
        }
    }
    styles.cell_styles = styles_named
        .into_iter()
        .map(|(ixfe, name, builtin, i_level)| NamedStyle {
            name,
            xf_id: style_map.get(&ixfe).copied().unwrap_or(0),
            builtin_id: builtin,
            i_level,
        })
        .collect();
    wb.styles = styles;
    wb.shared_strings = sst;

    // 3차원 참조용 XTI → 시트 이름
    let sheet_names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
    let xti_sheets: Vec<Option<String>> = xti
        .iter()
        .map(|&(book, first, last)| {
            // 외부 문서는 `[n]시트` (n = 외부 연결 번호, 1부터)
            let (book, names) = match supbooks.get(book as usize)? {
                Book::Internal => (None, &sheet_names),
                &Book::External(n) => {
                    let link = &wb.external_links[n - 1];
                    (Some((n, link.file_name())), &link.sheet_names)
                }
                Book::AddIn => return None,
            };
            let a = names.get(first as usize)?;
            let b = if first == last {
                None
            } else {
                Some(names.get(last as usize)?.as_str())
            };
            Some(formula::sheet_ref_prefix(
                book.as_ref().map(|(n, f)| (*n, f.as_str())),
                a,
                b,
            ))
        })
        .collect();
    // 차트 항목·계열 수식의 XTI → 내부 시트 번호 (vtchart::Sheets)
    let chart_xti: Vec<Option<u16>> = xti
        .iter()
        .map(|&(book, first, last)| {
            (matches!(supbooks.get(book as usize), Some(Book::Internal)) && first == last)
                .then_some(first)
        })
        .collect();
    // XTI 별 외부 이름: 추가 기능 함수는 이름 그대로, 외부 문서 이름은 `[n]!이름`
    let xti_names: Vec<Vec<String>> = xti
        .iter()
        .map(|&(book, _, _)| {
            let names = supbook_names
                .get(book as usize)
                .cloned()
                .unwrap_or_default();
            match supbooks.get(book as usize) {
                Some(&Book::External(n)) => names.iter().map(|x| format!("[{n}]!{x}")).collect(),
                _ => names,
            }
        })
        .collect();
    let name_list: Vec<String> = pending_names.iter().map(|n| n.name.clone()).collect();
    let table_names = crate::binary::reader::table_names(tables.iter());
    for (i, col, rgce) in table_formulas {
        let ctx = Context {
            xti_sheets: &xti_sheets,
            names: &name_list,
            xti_names: &xti_names,
            base: CellRef::default(),
            xlsb: false,
            row_shift: 0,
            tables: &table_names,
        };
        if let Ok(Decoded::Text(t)) = formula::decode(&rgce, &ctx)
            && let Some(c) = tables[i].columns.get_mut(col)
        {
            c.totals_formula = Some(t);
        }
    }

    // 수식이 없는 NAME 은 정의되지 않은 이름(=XYZ)이나 _xlfn. 함수 이름 자리다.
    wb.defined_names = pending_names
        .iter()
        .filter(|n| !n.rgce.is_empty() && !n.name.starts_with("_xlfn."))
        .map(|n| {
            let ctx = Context {
                xti_sheets: &xti_sheets,
                names: &name_list,
                xti_names: &xti_names,
                base: CellRef::default(),
                xlsb: false,
                row_shift: 0,
                tables: &table_names,
            };
            let formula = match formula::decode(&n.rgce, &ctx) {
                Ok(Decoded::Text(t)) => t,
                Ok(Decoded::SharedRef(_) | Decoded::TableRef(_)) => String::new(),
                Err(e) => format!("#ERR({})", e.0),
            };
            DefinedName {
                name: n.name.clone(),
                local_sheet_id: n.local,
                hidden: n.hidden,
                formula,
                comment: n.comment.clone(),
            }
        })
        .collect();

    // 그림 저장소 → xl/media 파트 (도형이 쓰지 않는 것은 시트를 읽은 뒤 버린다)
    for (i, b) in escher::blip_store(&dgg).into_iter().enumerate() {
        if let Some(b) = b {
            let (ext, ct) = crate::image::ext_and_type(b.format);
            wb.raw_parts.push(crate::model::RawPart {
                name: format!("{MEDIA_PREFIX}{}.{ext}", i + 1),
                content_type: Some(ct.into()),
                data: b.data,
            });
        }
    }

    // 시트. 다른 시트 범위의 이름은 시트 이름을 붙여 읽는다. 차트 레코드는 (시트 번호, 레코드들) — 시트의 앞
    // 도형들이 차례로 그 차트다
    let name_scopes: Vec<Option<u32>> = pending_names.iter().map(|n| n.local).collect();
    let sheet_names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
    let mut pending_charts: Vec<(usize, Vec<Vec<u8>>)> = Vec::new();
    for bs in sheets {
        let mut sheet = Sheet {
            name: bs.name.clone(),
            state: bs.state,
            kind: bs.kind.clone(),
            part: format!("/Workbook@{:#x}", bs.offset),
            show_grid_lines: true,
            ..Default::default()
        };
        sheet.sheet_id = wb.sheets.len() as u32 + 1;
        if bs.kind == SheetKind::Worksheet {
            let start = recs
                .iter()
                .position(|r| r.offset == bs.offset as usize)
                .ok_or_else(|| bad("BOUNDSHEET 위치에 BOF 가 없습니다"))?;
            let local_names =
                formula::names_for_sheet(&name_list, &name_scopes, &sheet_names, wb.sheets.len());
            let ctx_sheets = (
                &xti_sheets,
                &local_names,
                &xti_names,
                &table_names[..],
                &tables[..],
            );
            let mut charts = Vec::new();
            read_sheet(
                &recs[start..],
                &mut sheet,
                &wb,
                &cell_map,
                ctx_sheets,
                &mut dxf_extra,
                &mut charts,
            )?;
            if !charts.is_empty() {
                pending_charts.push((wb.sheets.len(), charts));
            }
        }
        wb.sheets.push(sheet);
    }
    attach_charts(&mut wb, &pending_charts, &chart_xti);
    // CF 의 DXFN 에만 남은 차등 서식 표시 형식·맞춤, 그리고 보호 (한셀은 0x088D 표에 표시 형식·맞춤을 쓰지 않고
    // 보호 값은 틀리게 쓴다 — 차등 서식 대조 실험)
    let mut next_id = wb
        .styles
        .num_fmts
        .iter()
        .map(|n| n.id)
        .max()
        .unwrap_or(163)
        .max(163)
        + 1;
    for (i, x) in dxf_extra {
        let Some(d) = wb.styles.dxfs.get_mut(i as usize) else {
            continue;
        };
        if let Some(code) = x.num_fmt
            && d.num_fmt.is_none()
        {
            d.num_fmt = Some(NumFmt { id: next_id, code });
            next_id += 1;
        }
        if let Some(id) = x.num_fmt_id
            && d.num_fmt.is_none()
            && let Some(code) = crate::numfmt::builtin(id)
        {
            d.num_fmt = Some(NumFmt {
                id,
                code: code.to_string(),
            });
        }
        if d.alignment.is_none() {
            d.alignment = x.alignment;
        }
        if x.protection.is_some() {
            d.protection = x.protection;
        }
        if x.gradient.is_some() {
            d.fill.get_or_insert_with(Default::default).gradient = x.gradient;
        }
    }
    // XTI 차례: 한셀은 레거시에서 읽은 XTI 표를 그대로 들고 있다가 저장한다 — 쓰기가 쓰이는 XTI 를 이 차례로 먼저 둔다
    wb.legacy_xti = xti
        .iter()
        .filter_map(|&(sb, a, b)| match supbooks.get(sb as usize)? {
            Book::Internal => Some((None, a, b)),
            &Book::External(n) => Some((Some(n), a, b)),
            Book::AddIn => None,
        })
        .collect();
    prune_external_links(&mut wb);
    let mut used: Vec<String> = Vec::new();
    fn targets(shapes: &[Shape], used: &mut Vec<String>) {
        for s in shapes {
            used.extend(s.target.clone());
            targets(&s.children, used);
        }
    }
    for s in &wb.sheets {
        targets(&s.shapes, &mut used);
    }
    wb.raw_parts
        .retain(|p| !p.name.starts_with(MEDIA_PREFIX) || used.contains(&p.name));
    Ok(wb)
}

/// 한컴 차트 레코드(0x7EF0)를 차트 모델로 바꿔 그 차트 도형에 붙인다. 수식의 ixti 는 원본 범위면 시트 순서 번호,
/// 항목·계열이면 XTI 번호이고 (`xti`: XTI → 내부 시트 번호, docs/format/cell-legacy-chart.md 2절), 캐시의 표시
/// 형식·빈 칸은 참조한 셀에서 온다. 읽을 수 없는 레코드의 도형은 앵커만 남는다.
fn attach_charts(wb: &mut Workbook, pending: &[(usize, Vec<Vec<u8>>)], xti: &[Option<u16>]) {
    if pending.is_empty() {
        return;
    }
    let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
    let mut mapped = Vec::new();
    {
        let book = super::vtchart_map::Book {
            sheets: &wb.sheets,
            styles: &wb.styles,
        };
        for (si, records) in pending {
            for (k, data) in records.iter().enumerate() {
                let sheets = super::vtchart::Sheets {
                    names: &names,
                    xti: Some(xti),
                };
                if let Ok(rec) = super::vtchart::parse_with(data, sheets) {
                    mapped.push((*si, k, super::vtchart_map::map(&rec, &book)));
                }
            }
        }
    }
    for (si, k, chart) in mapped {
        if let Some(s) = wb.sheets.get_mut(si).and_then(|s| s.shapes.get_mut(k))
            && s.kind == ShapeKind::Chart
        {
            s.chart = Some(chart);
        }
    }
}

/// 한셀은 레거시 문서를 열 때 어떤 수식·이름도 쓰지 않는 외부 연결(SUPBOOK)을 버리고
/// 남은 연결 번호를 앞으로 당긴다 (관측). 수식의 `[n]` 도 같이 고친다.
pub(crate) fn prune_external_links(wb: &mut Workbook) {
    use crate::formula_text::map_external_books;
    if wb.external_links.is_empty() {
        return;
    }
    let mut used = vec![false; wb.external_links.len() + 1];
    let mut mark = |text: &str| {
        map_external_books(text, |n| {
            if let Some(u) = used.get_mut(n) {
                *u = true;
            }
            None
        });
    };
    for d in &wb.defined_names {
        mark(&d.formula);
    }
    for c in wb.sheets.iter().flat_map(|s| &s.cells) {
        if let Some(f) = &c.formula {
            mark(&f.text);
        }
    }
    if used[1..].iter().all(|&u| u) {
        return;
    }
    // 옛 번호 → 새 번호
    let mut renum = vec![0usize; used.len()];
    let mut next = 0;
    for (n, &u) in used.iter().enumerate().skip(1) {
        if u {
            next += 1;
            renum[n] = next;
        }
    }
    let map = |n: usize| renum.get(n).copied().filter(|&m| m != 0);
    for d in &mut wb.defined_names {
        d.formula = map_external_books(&d.formula, map);
    }
    for c in wb.sheets.iter_mut().flat_map(|s| &mut s.cells) {
        if let Some(f) = &mut c.formula {
            f.text = map_external_books(&f.text, map);
        }
    }
    wb.legacy_xti = wb
        .legacy_xti
        .iter()
        .filter_map(|&(b, x, y)| match b {
            None => Some((None, x, y)),
            Some(n) => map(n).map(|m| (Some(m), x, y)),
        })
        .collect();
    let mut n = 0;
    wb.external_links.retain(|_| {
        n += 1;
        used[n]
    });
}

fn builtin_style_name(id: u8) -> String {
    match id {
        0 => "Normal",
        1 => "RowLevel",
        2 => "ColLevel",
        3 => "Comma",
        4 => "Currency",
        5 => "Percent",
        6 => "Comma [0]",
        7 => "Currency [0]",
        8 => "Hyperlink",
        9 => "Followed Hyperlink",
        _ => "Style",
    }
    .to_string()
}

pub(crate) fn builtin_name(code: u32) -> String {
    match code {
        0x00 => "_xlnm.Consolidate_Area",
        0x01 => "_xlnm.Auto_Open",
        0x02 => "_xlnm.Auto_Close",
        0x03 => "_xlnm.Extract",
        0x04 => "_xlnm.Database",
        0x05 => "_xlnm.Criteria",
        0x06 => "_xlnm.Print_Area",
        0x07 => "_xlnm.Print_Titles",
        0x08 => "_xlnm.Recorder",
        0x09 => "_xlnm.Data_Form",
        0x0A => "_xlnm.Auto_Activate",
        0x0B => "_xlnm.Auto_Deactivate",
        0x0C => "_xlnm.Sheet_Title",
        0x0D => "_xlnm._FilterDatabase",
        _ => "_xlnm.Unknown",
    }
    .to_string()
}

/// RK 값 ([MS-XLS] 2.5.217).
fn rk_value(rk: u32) -> f64 {
    let v = if rk & 0x02 != 0 {
        ((rk as i32) >> 2) as f64
    } else {
        f64::from_bits(((rk & 0xFFFF_FFFC) as u64) << 32)
    };
    if rk & 0x01 != 0 { v / 100.0 } else { v }
}

/// 시트 하나. `charts` 에는 한컴 차트(0x7EF0) 레코드를 나온 순서대로 담는다 — 시트의 앞 도형들이 그 차트다
/// 시트 수식에 필요한 통합 문서 정보: XTI 별 시트, 이름, XTI 별 외부 이름, 표 이름, 표
type SheetRefs<'a> = (
    &'a Vec<Option<String>>,
    &'a Vec<String>,
    &'a Vec<Vec<String>>,
    &'a [formula::TableName],
    &'a [crate::model::Table],
);

fn read_sheet(
    recs: &[Logical],
    sheet: &mut Sheet,
    wb: &Workbook,
    cell_map: &HashMap<u16, u32>,
    (xti_sheets, names, xti_names, list_names, lists): SheetRefs,
    dxf_extra: &mut Vec<(u32, Dxfn)>,
    charts: &mut Vec<Vec<u8>>,
) -> Result<()> {
    let style = |ixfe: u16| cell_map.get(&ixfe).copied().unwrap_or(0);
    let mut pending_string_cell: Option<usize> = None;
    // 공유 수식: 기준 셀 → (공유 토큰열, 기준 셀, 공유 번호 si)
    let mut shared: HashMap<(u32, u32), (Vec<u8>, CellRef, u32)> = HashMap::new();
    // 활성 창 번호 (PANE 이 없으면 왼쪽 위 = 3)
    let mut active_pane: u8 = 3;
    // SELECTION 이 활성 창 것과 맞았는지 (안 맞으면 마지막 것을 쓴다)
    let mut sel_matched = false;
    let mut window_flags: u16 = 0;
    // 메모: OBJ(ftCmo, 종류 0x19) 번호 → 바로 뒤 TXO 의 내용. NOTE 가 번호로 가리킨다
    let mut last_note_obj: Option<u16> = None;
    let mut note_texts: HashMap<u16, RichText> = HashMap::new();
    // 메모마다 NOTE 의 idObj (메모 상자는 그 번호의 OBJ 와 짝인 OfficeArt 도형)
    let mut note_objs: Vec<u16> = Vec::new();
    // 그리기: MSODRAWING 을 이어 붙인 OfficeArt 스트림, OBJ 목록(종류, 번호, TXO 내용)
    let mut art: Vec<u8> = Vec::new();
    let mut objs: Vec<ObjRec> = Vec::new();
    // 한컴 차트(0x7EF0)의 앵커. 레코드 내용은 모든 시트를 읽은 뒤 차트 모델로 바꾼다 (참조한 셀이 필요하다)
    let mut chart_anchors: Vec<(u16, [u32; 8])> = Vec::new();
    // 조건부 서식: CONDFMT 번호(nID) → 목록 위치, (nID, icf) → (서식, 규칙) 위치, 다음 CF12 가 붙을 서식
    let mut cf_by_id: HashMap<u16, usize> = HashMap::new();
    let mut cf_rules: HashMap<(u16, u16), (usize, usize)> = HashMap::new();
    let mut cf_current: Option<(u16, u16)> = None;
    let mut cf12_target: Option<usize> = None;
    // 배열 수식의 왼쪽 위 셀 (범위 안 다른 셀의 PtgExp 가 가리킨다)
    let mut array_masters: std::collections::HashSet<(u32, u32)> = Default::default();
    // 수식 셀의 토큰열 (공유 수식은 기준 셀의 토큰) — 행 반복(0xB002) 때 다시 해석한다
    let mut tokens_at: HashMap<(u32, u32), Vec<u8>> = HashMap::new();
    // 시트 보호: PROTECT, OBJPROTECT, SCENPROTECT, PASSWORD, FEATHDR 의 EnhancedProtection
    let (mut prot, mut prot_obj, mut prot_scen, mut prot_pw) = (false, false, false, 0u16);
    let mut prot_enh: Option<u32> = None;
    // 자동 필터: AUTOFILTER12 의 범위 (_FilterDatabase 이름이 없을 때 쓴다)
    let mut af_ref: Option<String> = None;
    // 마지막 AUTOFILTER12 가 동적 필터면 (열, 읽은 숫자 조건 수) — 뒤따르는 ContinueFrt12 의 val·maxVal
    let mut af12_dynamic: Option<(u32, usize)> = None;

    for rec in recs.iter().skip(1) {
        let d = &rec.data[..];
        let mut r = Rd::new(d);
        match rec.rtype {
            0x000A => break,
            0x0225 => {
                // 플래그 비트 0 (fUnsynced): 기본 높이를 직접 정했다
                sheet.default_row_custom = r.u16().is_some_and(|f| f & 1 != 0);
                if let Some(h) = r.u16() {
                    sheet.default_row_height = Some(h as f64 / 20.0);
                }
            }
            0x0055 => {
                if let Some(w) = r.u16() {
                    sheet.default_col_width = Some(w as f64);
                }
            }
            0x0200 => {
                let (Some(r1), Some(r2), Some(c1), Some(c2)) = (r.u32(), r.u32(), r.u16(), r.u16())
                else {
                    continue;
                };
                if r2 > r1 && c2 > c1 {
                    sheet.dimension = Some(
                        CellRange {
                            start: CellRef {
                                row: r1,
                                col: c1 as u32,
                            },
                            end: CellRef {
                                row: r2 - 1,
                                col: c2 as u32 - 1,
                            },
                        }
                        .to_string(),
                    );
                }
            }
            // COLINFO, 0xB00E = 257번째 열부터의 COLINFO (한컴, 같은 배치)
            0x007D | 0xB00E => {
                let (Some(c1), Some(c2), Some(w), Some(ixfe), Some(flags)) =
                    (r.u16(), r.u16(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                sheet.cols.push(ColInfo {
                    min: c1 as u32 + 1,
                    max: (c2 as u32 + 1).min(MAX_COLS),
                    width: Some(w as f64 / 256.0),
                    style: Some(style(ixfe)),
                    hidden: flags & 0x0001 != 0,
                    custom_width: flags & 0x0002 != 0,
                    // BIFF8 COLINFO: 비트 8–10 개요 수준, 0x1000 접힘
                    outline_level: ((flags >> 8) & 0x07) as u8,
                    collapsed: flags & 0x1000 != 0,
                });
            }
            0x0208 => {
                let Some(row) = r.u32() else { continue };
                r.take(4);
                let height = r.u16().unwrap_or(0);
                r.take(4);
                let flags = r.u32().unwrap_or(0);
                let custom = flags & 0x40 != 0;
                let hidden = flags & 0x20 != 0;
                let has_style = flags & 0x80 != 0;
                // BIFF8 ROW: 비트 0–2 개요 수준, 0x10 접힘
                let level = (flags & 0x07) as u8;
                let collapsed = flags & 0x10 != 0;
                // 높이는 지정(0x40)이 아니어도 그 행의 실제 높이다 (한셀 XML 의 ht, 그리기 앵커 계산에 쓰인다)
                let default_twips = sheet.default_row_height.map(|h| (h * 20.0).round() as u16);
                if custom
                    || hidden
                    || has_style
                    || level > 0
                    || collapsed
                    || (height != 0 && Some(height) != default_twips)
                {
                    sheet.rows.push(RowInfo {
                        index: row + 1,
                        height: (height != 0).then_some(height as f64 / 20.0),
                        custom_height: custom,
                        hidden,
                        style: has_style.then(|| style(((flags >> 16) & 0x0FFF) as u16)),
                        outline_level: level,
                        collapsed,
                    });
                }
            }
            0xB002 => {
                // 한컴 행 반복: first 행(속성·셀)을 first+1..=last 행에 그대로 되풀이한다.
                // 행 상대 참조는 옮긴 행 수만큼 따라 움직인다.
                let (Some(first), Some(last)) = (r.u32(), r.u32()) else {
                    continue;
                };
                if last <= first || last >= MAX_ROWS {
                    continue;
                }
                let info = sheet
                    .rows
                    .iter()
                    .rev()
                    .find(|ri| ri.index == first + 1)
                    .cloned();
                let src: Vec<Cell> = sheet
                    .cells
                    .iter()
                    .filter(|c| c.r.row == first)
                    .cloned()
                    .collect();
                for row in first + 1..=last {
                    if let Some(ri) = &info {
                        sheet.rows.push(RowInfo {
                            index: row + 1,
                            ..ri.clone()
                        });
                    }
                    for c in &src {
                        let mut copy = c.clone();
                        copy.r.row = row;
                        if let Some(tokens) = tokens_at.get(&(first, c.r.col)) {
                            let ctx = Context {
                                xti_sheets,
                                names,
                                xti_names,
                                base: copy.r,
                                xlsb: false,
                                row_shift: (row - first) as i64,
                                tables: list_names,
                            };
                            if let Ok(Decoded::Text(t)) = formula::decode(tokens, &ctx) {
                                copy.formula = Some(Formula {
                                    text: t,
                                    ..Default::default()
                                });
                            }
                        }
                        sheet.cells.push(copy);
                    }
                }
            }
            0x00FD | 0x0203 | 0x027E | 0x0201 | 0x0205 | 0x0204 => {
                let (Some(row), Some(col), Some(ixfe)) = (r.u32(), r.u16(), r.u16()) else {
                    continue;
                };
                let value = match rec.rtype {
                    0x00FD => r
                        .u32()
                        .and_then(|i| wb.shared_strings.get(i as usize))
                        .map(|s| CellValue::Text(s.text.clone()))
                        .unwrap_or_default(),
                    0x0203 => r.f64().map(CellValue::Number).unwrap_or_default(),
                    0x027E => r
                        .u32()
                        .map(|v| CellValue::Number(rk_value(v)))
                        .unwrap_or_default(),
                    0x0205 => {
                        let v = r.u8().unwrap_or(0);
                        if r.u8() == Some(1) {
                            CellValue::Error(formula::error_text(v).into())
                        } else {
                            CellValue::Bool(v != 0)
                        }
                    }
                    0x0204 => {
                        let cch = r.u16().unwrap_or(0) as usize;
                        let _flag = r.u8();
                        CellValue::Text(r.utf16(cch).unwrap_or_default())
                    }
                    _ => CellValue::Empty,
                };
                sheet.cells.push(Cell {
                    r: CellRef {
                        row,
                        col: col as u32,
                    },
                    style: style(ixfe),
                    value,
                    formula: None,
                });
            }
            0x00BD => {
                let (Some(row), Some(first)) = (r.u32(), r.u16()) else {
                    continue;
                };
                let n = (r.left().saturating_sub(2)) / 6;
                for k in 0..n {
                    let (Some(ixfe), Some(rk)) = (r.u16(), r.u32()) else {
                        break;
                    };
                    sheet.cells.push(Cell {
                        r: CellRef {
                            row,
                            col: first as u32 + k as u32,
                        },
                        style: style(ixfe),
                        value: CellValue::Number(rk_value(rk)),
                        formula: None,
                    });
                }
            }
            0x00BE => {
                let (Some(row), Some(first)) = (r.u32(), r.u16()) else {
                    continue;
                };
                let n = (r.left().saturating_sub(2)) / 2;
                for k in 0..n {
                    let Some(ixfe) = r.u16() else { break };
                    sheet.cells.push(Cell {
                        r: CellRef {
                            row,
                            col: first as u32 + k as u32,
                        },
                        style: style(ixfe),
                        value: CellValue::Empty,
                        formula: None,
                    });
                }
            }
            0x0006 => {
                let (Some(row), Some(col), Some(ixfe)) = (r.u32(), r.u16(), r.u16()) else {
                    continue;
                };
                let Some(val) = r.take(8) else { continue };
                let _grbit = r.u16();
                let _chn = r.u32();
                let cce = r.u16().unwrap_or(0) as usize;
                let rgce = r.take(cce).unwrap_or_default();
                let at = CellRef {
                    row,
                    col: col as u32,
                };
                let value = if val[6] == 0xFF && val[7] == 0xFF {
                    match val[0] {
                        0 => CellValue::Text(String::new()), // STRING 레코드가 뒤따른다
                        1 => CellValue::Bool(val[2] != 0),
                        2 => CellValue::Error(formula::error_text(val[2]).into()),
                        _ => CellValue::Text(String::new()),
                    }
                } else {
                    CellValue::Number(f64::from_le_bytes(val.try_into().unwrap()))
                };
                let ctx = Context {
                    xti_sheets,
                    names,
                    xti_names,
                    base: at,
                    xlsb: false,
                    row_shift: 0,
                    tables: list_names,
                };
                let formula = match formula::decode(rgce, &ctx) {
                    Ok(Decoded::Text(t)) => {
                        tokens_at.insert((row, col as u32), rgce.to_vec());
                        Some(Formula {
                            text: t,
                            ..Default::default()
                        })
                    }
                    // 배열 수식·데이터 표 범위 안의 다른 셀: 수식은 왼쪽 위 셀에만 둔다 (XML 과 같다)
                    Ok(Decoded::SharedRef(master))
                        if master != at && array_masters.contains(&(master.row, master.col)) =>
                    {
                        None
                    }
                    Ok(Decoded::TableRef(master)) if master != at => None,
                    // 데이터 표의 왼쪽 위: 뒤따르는 TABLE 이 채운다
                    Ok(Decoded::TableRef(_)) => Some(Formula::default()),
                    Ok(Decoded::SharedRef(master)) => {
                        let si = shared.get(&(master.row, master.col)).map(|s| s.2);
                        let text = shared
                            .get(&(master.row, master.col))
                            .and_then(|(tokens, _, _)| {
                                tokens_at.insert((row, col as u32), tokens.clone());
                                let c = Context {
                                    xti_sheets,
                                    names,
                                    xti_names,
                                    base: at,
                                    xlsb: false,
                                    row_shift: 0,
                                    tables: list_names,
                                };
                                match formula::decode(tokens, &c) {
                                    Ok(Decoded::Text(t)) => Some(t),
                                    _ => None,
                                }
                            })
                            .unwrap_or_default();
                        // 공유 수식의 종속 셀 (기준 셀은 SHRFMLA 에서 고친다)
                        Some(Formula {
                            text,
                            kind: if si.is_some() {
                                FormulaKind::Shared
                            } else {
                                FormulaKind::Normal
                            },
                            shared_index: si,
                            ..Default::default()
                        })
                    }
                    Err(e) => Some(Formula {
                        text: format!("#ERR({})", e.0),
                        ..Default::default()
                    }),
                };
                let is_string = val[6] == 0xFF && val[7] == 0xFF && val[0] == 0;
                sheet.cells.push(Cell {
                    r: at,
                    style: style(ixfe),
                    value,
                    formula,
                });
                pending_string_cell = is_string.then(|| sheet.cells.len() - 1);
            }
            // 이 시트의 표 (전역 0xB005… 의 표 번호), 이어지는 0xB006·0xB007(열 수)은 그 표의 자동 필터 — 필터 조건은
            // 레거시에 남지 않는다 (표 대조 실험)
            0xB00A => {
                if let Some(id) = r.u32()
                    && let Some(t) = lists.iter().find(|t| t.id == id)
                {
                    sheet.tables.push(t.clone());
                }
            }
            0xB006 => {
                if let Some(t) = sheet.tables.last_mut()
                    && let Some(mut rg) = CellRange::parse(&t.range)
                {
                    rg.end.row = rg.end.row.saturating_sub(t.totals_rows);
                    t.auto_filter = Some(crate::model::AutoFilter {
                        range: rg.to_string(),
                        columns: Vec::new(),
                        sort_state: None,
                    });
                }
            }
            0x0221 => {
                // ARRAY: rwFirst u32, rwLast u32, colFirst u16, colLast u16, grbit u16, chn u32, cce u16 + 토큰.
                // 직전 FORMULA(PtgExp 가 자기 자신)가 왼쪽 위 셀이다 (수식 대조 실험)
                let (Some(r1), Some(r2), Some(c1), Some(c2)) = (r.u32(), r.u32(), r.u16(), r.u16())
                else {
                    continue;
                };
                r.take(6);
                let cce = r.u16().unwrap_or(0) as usize;
                let Some(tokens) = r.take(cce) else { continue };
                let start = CellRef {
                    row: r1,
                    col: c1 as u32,
                };
                let ctx = Context {
                    xti_sheets,
                    names,
                    xti_names,
                    base: start,
                    xlsb: false,
                    row_shift: 0,
                    tables: list_names,
                };
                if let Some(cell) = sheet.cells.last_mut()
                    && cell.r == start
                    && let Ok(Decoded::Text(t)) = formula::decode(tokens, &ctx)
                {
                    array_masters.insert((r1, c1 as u32));
                    cell.formula = Some(Formula {
                        text: t,
                        kind: FormulaKind::Array,
                        range: Some(
                            CellRange {
                                start,
                                end: CellRef {
                                    row: r2,
                                    col: c2 as u32,
                                },
                            }
                            .to_string(),
                        ),
                        ..Default::default()
                    });
                }
            }
            0x0236 => {
                // TABLE (한컴 변형): rwFirst u32, rwLast u32, colFirst u16, colLast u16, grbit u16 (0x04 행 입력,
                // 0x08 2변수), 입력 1 (rw u32, col u16), 입력 2 (rw u32, col u16 — 없으면 FF) — 직전 FORMULA
                // (PtgTbl 이 자기 자신)가 왼쪽 위 셀이다 (수식 대조 실험)
                let (Some(r1), Some(r2), Some(c1), Some(c2), Some(grbit)) =
                    (r.u32(), r.u32(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                let mut input = || -> Option<String> {
                    let (row, col) = (r.u32()?, r.u16()?);
                    (row != u32::MAX && col != u16::MAX).then(|| {
                        CellRef {
                            row,
                            col: col as u32,
                        }
                        .to_string()
                    })
                };
                let (in1, in2) = (input(), input());
                let start = CellRef {
                    row: r1,
                    col: c1 as u32,
                };
                if let Some(cell) = sheet.cells.last_mut()
                    && cell.r == start
                {
                    cell.formula = Some(Formula {
                        kind: FormulaKind::DataTable,
                        range: Some(
                            CellRange {
                                start,
                                end: CellRef {
                                    row: r2,
                                    col: c2 as u32,
                                },
                            }
                            .to_string(),
                        ),
                        data_table: Some(crate::model::DataTableInput {
                            two_d: grbit & 0x08 != 0,
                            row_input: grbit & 0x04 != 0,
                            r1: in1,
                            r2: in2,
                            del1: grbit & 0x10 != 0,
                            del2: grbit & 0x20 != 0,
                        }),
                        ..Default::default()
                    });
                }
            }
            0x04BC => {
                // SHRFMLA: 범위 + 공유 토큰열. 직전 FORMULA 가 기준 셀이다.
                let (Some(r1), Some(r2), Some(c1), Some(c2)) = (r.u32(), r.u32(), r.u16(), r.u16())
                else {
                    continue;
                };
                let _res = r.u16();
                let cce = r.u16().unwrap_or(0) as usize;
                if let Some(tokens) = r.take(cce) {
                    let si = shared.len() as u32;
                    shared.insert(
                        (r1, c1 as u32),
                        (
                            tokens.to_vec(),
                            CellRef {
                                row: r1,
                                col: c1 as u32,
                            },
                            si,
                        ),
                    );
                    let range = crate::model::CellRange {
                        start: CellRef {
                            row: r1,
                            col: c1 as u32,
                        },
                        end: CellRef {
                            row: r2,
                            col: c2 as u32,
                        },
                    }
                    .to_string();
                    // 직전 수식 셀을 다시 해석
                    if let Some(cell) = sheet.cells.last_mut()
                        && cell.formula.as_ref().is_some_and(|f| f.text.is_empty())
                    {
                        let ctx = Context {
                            xti_sheets,
                            names,
                            xti_names,
                            base: cell.r,
                            xlsb: false,
                            row_shift: 0,
                            tables: list_names,
                        };
                        if let Ok(Decoded::Text(t)) = formula::decode(tokens, &ctx) {
                            tokens_at.insert((cell.r.row, cell.r.col), tokens.to_vec());
                            // 공유 수식의 기준 셀: XML 의 `t="shared" ref=… si=…` 와 같게
                            cell.formula = Some(Formula {
                                text: t,
                                kind: FormulaKind::Shared,
                                shared_index: Some(si),
                                range: Some(range),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            0x0207 => {
                let cch = r.u16().unwrap_or(0) as usize;
                let _flag = r.u8();
                let s = r.utf16(cch).unwrap_or_default();
                if let Some(i) = pending_string_cell.take() {
                    sheet.cells[i].value = CellValue::Text(s);
                }
            }
            0x00E5 => {
                let n = r.u16().unwrap_or(0);
                for _ in 0..n {
                    let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                        (r.u32(), r.u32(), r.u16(), r.u16())
                    else {
                        break;
                    };
                    sheet.merges.push(CellRange {
                        start: CellRef {
                            row: r1,
                            col: c1 as u32,
                        },
                        end: CellRef {
                            row: r2,
                            col: c2 as u32,
                        },
                    });
                }
            }
            0x023E => {
                // WINDOW2: grbit u16, rwTop u32, colLeft u16, icvHdr u32, 예약 u16,
                // wScaleNormal u16 (14), wScaleSLV u16 (16) — BIFF8 과 두 확대율 순서가 다르다 (관측)
                let Some(grbit) = r.u16() else { continue };
                window_flags = grbit;
                sheet.show_grid_lines = grbit & 0x0002 != 0;
                let v = &mut sheet.view;
                v.show_formulas = grbit & 0x0001 != 0;
                v.show_row_col_headers = grbit & 0x0004 != 0;
                v.show_zeros = grbit & 0x0010 != 0;
                v.right_to_left = grbit & 0x0040 != 0;
                v.tab_selected = grbit & 0x0200 != 0;
                if grbit & 0x0800 != 0 {
                    v.view = Some("pageBreakPreview".into());
                }
                if let (Some(row), Some(col)) = (r.u32(), r.u16()) {
                    v.top_left_cell = Some(
                        CellRef {
                            row,
                            col: col as u32,
                        }
                        .to_string(),
                    );
                }
                // icvHdr: fDefaultHdr(0x20)가 꺼져 있으면 눈금선 색
                let icv = r.u32();
                if grbit & 0x0020 == 0 {
                    v.grid_color = icv;
                }
                r.take(2);
                let normal = r.u16().filter(|&z| z != 0);
                let slv = r.u16().filter(|&z| z != 0);
                v.zoom_scale_normal = normal.map(u32::from);
                v.zoom_scale_sheet_layout_view = slv.map(u32::from);
            }
            0x01BE => {
                // DV: 플래그 u32 (XLSB BrtDVal 과 같은 비트), 입력 제목·오류 제목·입력·오류 (cch u16, flag u8, UTF-16),
                // 수식 1·2 (cce u16, 예약 u16, 토큰), 범위 (n u16, (rw u32 ×2, col u16 ×2) × n)
                let Some(flags) = r.u32() else { continue };
                let mut strs: Vec<Option<String>> = (0..4)
                    .map(|_| {
                        let n = r.u16().unwrap_or(0) as usize;
                        let _flag = r.u8();
                        r.utf16(n).filter(|s| !s.is_empty() && s != "\0")
                    })
                    .collect();
                let mut tokens = Vec::new();
                for _ in 0..2 {
                    let cce = r.u16().unwrap_or(0) as usize;
                    r.take(2);
                    tokens.push(r.take(cce).unwrap_or_default().to_vec());
                }
                let n = r.u16().unwrap_or(0);
                let mut refs = Vec::new();
                for _ in 0..n {
                    let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                        (r.u32(), r.u32(), r.u16(), r.u16())
                    else {
                        break;
                    };
                    refs.push(CellRange {
                        start: CellRef {
                            row: r1,
                            col: c1 as u32,
                        },
                        end: CellRef {
                            row: r2,
                            col: c2 as u32,
                        },
                    });
                }
                let base = refs.first().map(|x| x.start).unwrap_or_default();
                let dec = |t: &[u8]| {
                    if t.is_empty() {
                        return None;
                    }
                    let ctx = Context {
                        xti_sheets,
                        names,
                        xti_names,
                        base,
                        xlsb: false,
                        row_shift: 0,
                        tables: list_names,
                    };
                    match formula::decode(t, &ctx) {
                        Ok(Decoded::Text(x)) => Some(x),
                        _ => None,
                    }
                };
                // 레거시 문자열 순서: 입력 제목, 오류 제목, 입력, 오류 → (오류 제목, 오류, 입력 제목, 입력)
                let mut ordered = vec![
                    strs[1].take(),
                    strs[3].take(),
                    strs[0].take(),
                    strs[2].take(),
                ];
                let sqref = refs
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                sheet
                    .data_validations
                    .push(crate::binary::reader::dv_from_flags(
                        flags,
                        sqref,
                        &mut ordered,
                        dec(&tokens[0]),
                        dec(&tokens[1]),
                    ));
            }
            0x01B0 | 0x0879 => {
                // CONDFMT / CONDFMT12(FRT 머리 12바이트 뒤): ccf u16, (nID << 1) u16, 경계 Ref8 (행 u32),
                // 범위 (n u16, Ref8 × n)
                if rec.rtype == 0x0879 {
                    r.take(12);
                }
                let (Some(_ccf), Some(id)) = (r.u16(), r.u16()) else {
                    continue;
                };
                r.take(12);
                let n = r.u16().unwrap_or(0);
                let mut refs = Vec::new();
                for _ in 0..n {
                    let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                        (r.u32(), r.u32(), r.u16(), r.u16())
                    else {
                        break;
                    };
                    refs.push(
                        CellRange {
                            start: CellRef {
                                row: r1,
                                col: c1 as u32,
                            },
                            end: CellRef {
                                row: r2,
                                col: c2 as u32,
                            },
                        }
                        .to_string(),
                    );
                }
                let nid = id >> 1;
                cf_by_id.insert(nid, sheet.conditional_formats.len());
                sheet.conditional_formats.push(ConditionalFormat {
                    sqref: refs.join(" "),
                    rules: Vec::new(),
                });
                cf_current = Some((nid, 0));
                cf12_target = (rec.rtype == 0x0879).then(|| sheet.conditional_formats.len() - 1);
            }
            0x01B1 => {
                // CF: ct u8 (1 값, 2 수식), cp u8 (연산자), cce1 u16, cce2 u16, DXFN(가변, 한컴 확장),
                // 끝에서 차등 서식 번호 u32 + 수식 1·2
                let Some((nid, icf)) = cf_current else {
                    continue;
                };
                let Some(&fi) = cf_by_id.get(&nid) else {
                    continue;
                };
                let (Some(ct), Some(cp), Some(c1), Some(c2)) = (r.u8(), r.u8(), r.u16(), r.u16())
                else {
                    continue;
                };
                let (c1, c2) = (c1 as usize, c2 as usize);
                let Some(fstart) = d.len().checked_sub(c1 + c2) else {
                    continue;
                };
                // 차등 서식 번호 u32 는 수식 바로 앞. DXFN 에 테두리 블록(플래그 0x10000000)이 있으면
                // 그 사이에 한컴 안쪽 선 확장이 온다 ([`cf_dxf_id`])
                let has_border = d.get(6..10).is_some_and(|x| {
                    u32::from_le_bytes([x[0], x[1], x[2], x[3]]) & 0x1000_0000 != 0
                });
                let dxf = cf_dxf_id(d, fstart, has_border, &wb.styles.dxfs);
                let base = CellRange::parse(
                    sheet.conditional_formats[fi]
                        .sqref
                        .split(' ')
                        .next()
                        .unwrap_or(""),
                )
                .map(|x| x.start)
                .unwrap_or_default();
                let dec = |t: &[u8]| {
                    let ctx = Context {
                        xti_sheets,
                        names,
                        xti_names,
                        base,
                        xlsb: false,
                        row_shift: 0,
                        tables: list_names,
                    };
                    match formula::decode(t, &ctx) {
                        Ok(Decoded::Text(x)) => x,
                        _ => String::new(),
                    }
                };
                // DXFN 의 표준 부분에만 있는 표시 형식·맞춤·보호
                if let (Some(id), Some(x)) = (
                    dxf.filter(|&x| x != u32::MAX),
                    d.get(6..).and_then(parse_dxfn),
                ) {
                    dxf_extra.push((id, x));
                }
                let mut formulas = vec![dec(&d[fstart..fstart + c1])];
                if c2 > 0 {
                    formulas.push(dec(&d[fstart + c1..]));
                }
                const OPS: [&str; 9] = [
                    "",
                    "between",
                    "notBetween",
                    "equal",
                    "notEqual",
                    "greaterThan",
                    "lessThan",
                    "greaterThanOrEqual",
                    "lessThanOrEqual",
                ];
                let cf = &mut sheet.conditional_formats[fi];
                cf.rules.push(CfRule {
                    kind: if ct == 1 { "cellIs" } else { "expression" }.into(),
                    priority: cf.rules.len() as u32 + 1,
                    dxf_id: dxf.filter(|&x| x != u32::MAX),
                    operator: (ct == 1)
                        .then(|| OPS.get(cp as usize).map(|s| s.to_string()))
                        .flatten()
                        .filter(|s| !s.is_empty()),
                    formulas: formulas.into_iter().filter(|f| !f.is_empty()).collect(),
                    ..Default::default()
                });
                cf_rules.insert((nid, icf), (fi, cf.rules.len() - 1));
                cf_current = Some((nid, icf + 1));
            }
            0x087B => {
                // CFEX: FRT 머리(ref8 은 16비트 행), fIsCF12 u32, nID u16,
                // CF12 가 아니면 icf u16, cp u8, 틀 u8, 우선순위 u16, 플래그 u8 (0x02 멈춤) … 끝 16바이트 = 틀 매개 변수
                r.take(12);
                let (Some(is12), Some(nid)) = (r.u32(), r.u16()) else {
                    continue;
                };
                if is12 != 0 {
                    cf12_target = cf_by_id.get(&nid).copied();
                    continue;
                }
                let (Some(icf), Some(_cp), Some(tmpl), Some(pri), Some(flags)) =
                    (r.u16(), r.u8(), r.u8(), r.u16(), r.u8())
                else {
                    continue;
                };
                let params = d.get(d.len().saturating_sub(16)..).unwrap_or_default();
                if let Some(&(fi, ri)) = cf_rules.get(&(nid, icf)) {
                    let rule = &mut sheet.conditional_formats[fi].rules[ri];
                    rule.priority = pri as u32;
                    rule.stop_if_true = flags & 0x02 != 0;
                    apply_cf_template(rule, tmpl as u32, params);
                    // 차등 서식의 그라데이션은 여기 DXFN12 의 확장에만 있다
                    if let (Some(id), Some(g)) = (rule.dxf_id, cfex_gradient(d)) {
                        dxf_extra.push((
                            id,
                            Dxfn {
                                gradient: Some(g),
                                ..Default::default()
                            },
                        ));
                    }
                }
            }
            0x087A => {
                // CF12: FRT 머리 12, ct u8, cp u8, cce1 u16, cce2 u16, cbdxf u32 + DXFN12, 수식 1·2,
                // 활성 수식(cce u16 + 2), 플래그 u8, 우선순위 u16, 틀 u16, 매개 변수(cb u8 + 16), 종류별 내용
                let Some(fi) = cf12_target else { continue };
                r.take(12);
                let (Some(ct), Some(_cp), Some(c1), Some(c2), Some(cbdxf)) =
                    (r.u8(), r.u8(), r.u16(), r.u16(), r.u32())
                else {
                    continue;
                };
                r.take(cbdxf as usize + c1 as usize + c2 as usize + 4);
                let (Some(_flags), Some(pri), Some(tmpl), Some(cbp)) =
                    (r.u8(), r.u16(), r.u16(), r.u8())
                else {
                    continue;
                };
                let params = r.take(cbp as usize).unwrap_or_default().to_vec();
                r.take(2); // 내용 길이
                let mut rule = CfRule {
                    priority: pri as u32,
                    ..Default::default()
                };
                // 기준 값의 수식 (기준 셀 = 범위의 왼쪽 위)
                let base = CellRange::parse(
                    sheet.conditional_formats[fi]
                        .sqref
                        .split(' ')
                        .next()
                        .unwrap_or(""),
                )
                .map(|x| x.start)
                .unwrap_or_default();
                let dec = |t: &[u8]| {
                    let ctx = Context {
                        xti_sheets,
                        names,
                        xti_names,
                        base,
                        xlsb: false,
                        row_shift: 0,
                        tables: list_names,
                    };
                    match formula::decode(t, &ctx) {
                        Ok(Decoded::Text(x)) => Some(x),
                        _ => None,
                    }
                };
                match ct {
                    3 => {
                        rule.kind = "colorScale".into();
                        rule.color_scale = legacy_color_scale(&mut r, &dec);
                    }
                    4 => {
                        rule.kind = "dataBar".into();
                        rule.data_bar = legacy_data_bar(&mut r, &dec);
                    }
                    6 => {
                        rule.kind = "iconSet".into();
                        rule.icon_set = legacy_icon_set(&mut r, &dec);
                        // Excel 2010 아이콘 집합은 x14 구역의 규칙이다
                        rule.x14 = rule.icon_set.as_ref().is_some_and(|i| {
                            crate::model::X14_ONLY_ICON_SETS.contains(&i.set.as_str())
                        });
                    }
                    _ => {
                        rule.kind = "expression".into();
                        apply_cf_template(&mut rule, tmpl as u32, &params);
                    }
                }
                sheet.conditional_formats[fi].rules.push(rule);
            }
            0x00EC => art.extend_from_slice(d),
            0x7EF0 => {
                // 한컴 차트: u32, u16, 차트 번호 u16, 그다음 OfficeArtClientAnchor 와 같은 22바이트 앵커
                r.take(8);
                let (Some(flag), Some(c1), Some(dx1), Some(r1), Some(dy1)) =
                    (r.u16(), r.u16(), r.u16(), r.u32(), r.u16())
                else {
                    continue;
                };
                let (Some(c2), Some(dx2), Some(r2), Some(dy2)) =
                    (r.u16(), r.u16(), r.u32(), r.u16())
                else {
                    continue;
                };
                chart_anchors.push((
                    flag,
                    [
                        c1 as u32, dx1 as u32, r1, dy1 as u32, c2 as u32, dx2 as u32, r2,
                        dy2 as u32,
                    ],
                ));
                charts.push(rec.data.clone());
            }
            0x005D => {
                // OBJ: 첫 하위 레코드 ftCmo = ft u16 (0x15), cb u16, ot u16, id u16, …
                let (Some(ft), Some(_cb), Some(ot), Some(id)) =
                    (r.u16(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                let grbit = r.u16().unwrap_or(0x0011);
                last_note_obj = (ft == 0x15 && ot == 0x19).then_some(id);
                objs.push(ObjRec {
                    ot,
                    grbit,
                    id,
                    text: None,
                    align: (1, 0),
                });
            }
            0x01B6 => {
                // TXO (한컴): 머리 10바이트 (4 가로 맞춤 u8, 5 세로 맞춤 u8), cch u16, flag u8, UTF-16
                // (CONTINUE 없이 바로), 뒤에 (ich u16, ifnt u16, 예약 u32) 런
                let align = (
                    d.get(4).copied().unwrap_or(1),
                    d.get(5).copied().unwrap_or(0),
                );
                r.take(10);
                let cch = r.u16().unwrap_or(0) as usize;
                let _flag = r.u8();
                let text = r.utf16(cch).unwrap_or_default();
                let mut runs = Vec::new();
                while let (Some(ich), Some(ifnt)) = (r.u16(), r.u16()) {
                    runs.push((ich, ifnt));
                    r.take(4);
                }
                if let Some(o) = objs.last_mut() {
                    o.text = Some((text.clone(), runs));
                    o.align = align;
                }
                let Some(id) = last_note_obj.take() else {
                    continue;
                };
                let mut r = Rd::new(d);
                r.take(13 + cch * 2);
                let font = match (r.u16(), r.u16()) {
                    (Some(0), Some(ifnt)) => wb.styles.fonts.get(ifnt as usize).cloned(),
                    _ => None,
                };
                let runs = match font {
                    Some(f) if !text.is_empty() => vec![TextRun {
                        text: text.clone(),
                        font: Some(f),
                    }],
                    _ => Vec::new(),
                };
                note_texts.insert(id, RichText { text, runs });
            }
            0x001C => {
                // NOTE: rw u32, col u16, grbit u16 (0x0002 표시), idObj u16, 작성자 (cch u16, flag u8, UTF-16)
                let (Some(row), Some(col), Some(grbit), Some(id)) =
                    (r.u32(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                let cch = r.u16().unwrap_or(0) as usize;
                let _flag = r.u8();
                sheet.comments.push(Comment {
                    cell: CellRef {
                        row,
                        col: col as u32,
                    },
                    author: r.utf16(cch).unwrap_or_default(),
                    text: note_texts.remove(&id).unwrap_or_default(),
                    visible: grbit & 0x0002 != 0,
                    ..Default::default()
                });
                note_objs.push(id);
            }
            0x0041 => {
                // PANE: x u16, y u32, rwTop u32, colLeft u16, pnnAct u16.
                // 고정(WINDOW2 0x0008)이면 x·y 는 열·행 수, 아니면 1/20 pt
                let (Some(x), Some(y), Some(row), Some(col), Some(pnn)) =
                    (r.u16(), r.u32(), r.u32(), r.u16(), r.u16())
                else {
                    continue;
                };
                active_pane = pnn as u8;
                let frozen = window_flags & 0x0008 != 0;
                let no_split = window_flags & 0x0100 != 0;
                sheet.view.pane = Some(Pane {
                    x_split: x as f64,
                    y_split: y as f64,
                    top_left_cell: Some(
                        CellRef {
                            row,
                            col: col as u32,
                        }
                        .to_string(),
                    ),
                    active_pane: pane_name(pnn as u32),
                    state: Some(
                        match (frozen, no_split) {
                            (true, true) => "frozen",
                            (true, false) => "frozenSplit",
                            _ => "split",
                        }
                        .into(),
                    ),
                });
            }
            0x001B => {
                // HORIZONTALPAGEBREAKS: n u16, (rw u32, colStart u16, colEnd u16) × n
                let n = r.u16().unwrap_or(0);
                for _ in 0..n {
                    let (Some(row), Some(_), Some(_)) = (r.u32(), r.u16(), r.u16()) else {
                        break;
                    };
                    sheet.page.row_breaks.push(row);
                }
            }
            0x001A => {
                // VERTICALPAGEBREAKS: n u16, (col u16, rwStart u16, rwEnd u32) × n (관측: 8바이트)
                let n = r.u16().unwrap_or(0);
                for _ in 0..n {
                    let (Some(col), Some(_), Some(_)) = (r.u16(), r.u16(), r.u32()) else {
                        break;
                    };
                    sheet.page.col_breaks.push(col as u32);
                }
            }
            0x00A0 => {
                // SCL: 현재 확대율 = num / den
                if let (Some(n), Some(d)) = (r.u16(), r.u16())
                    && d != 0
                {
                    sheet.view.zoom_scale = Some((n as u32 * 100 + d as u32 / 2) / d as u32);
                }
            }
            0x001D => {
                // SELECTION: pnn u8, rwAct u32, colAct u16, irefAct u16, cref u16, (rw u32 ×2, col u16 ×2) × cref
                let (Some(pnn), Some(row), Some(col), Some(_iref), Some(n)) =
                    (r.u8(), r.u32(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                // 틀이 나뉘면 창마다 하나씩 있다: 활성 창(PANE)의 것, 없으면 왼쪽 위(3)
                if pnn != active_pane && sel_matched {
                    continue;
                }
                sel_matched |= pnn == active_pane;
                sheet.view.active_cell = Some(
                    CellRef {
                        row,
                        col: col as u32,
                    }
                    .to_string(),
                );
                let mut refs = Vec::new();
                for _ in 0..n {
                    let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                        (r.u32(), r.u32(), r.u16(), r.u16())
                    else {
                        break;
                    };
                    refs.push(
                        CellRange {
                            start: CellRef {
                                row: r1,
                                col: c1 as u32,
                            },
                            end: CellRef {
                                row: r2,
                                col: c2 as u32,
                            },
                        }
                        .to_string(),
                    );
                }
                sheet.view.sqref = Some(refs.join(" "));
            }
            0x0862 => {
                // SHEETEXT: frt 머리 12 + cb u32 + icvPlain u32 (팔레트), 한컴은 뒤에 COLORREF 를 붙인다
                r.take(16);
                let icv = r.u32().unwrap_or(0x7F) & 0x7F;
                sheet.tab_color = match r.u32() {
                    Some(v) => colorref(v),
                    None if icv != 0x7F => palette_color(icv as u16, &DEFAULT_PALETTE),
                    None => None,
                };
            }
            0x01BA => {
                // CODENAME: cch u16, flag u8, UTF-16
                let cch = r.u16().unwrap_or(0) as usize;
                let _flag = r.u8();
                sheet.code_name = r.utf16(cch);
            }
            0x0081 => {
                // WSBOOL: 0x0001 자동 페이지 나누기 표시, 0x0040 요약 행 아래, 0x0080 요약 열 오른쪽,
                // 0x0100 페이지에 맞추기
                if let Some(g) = r.u16() {
                    sheet.page.auto_page_breaks = Some(g & 0x0001 != 0);
                    sheet.summary_above = g & 0x0040 == 0;
                    sheet.summary_left = g & 0x0080 == 0;
                    sheet.page.fit_to_page = Some(g & 0x0100 != 0);
                }
            }
            0x0012 => prot = r.u16().is_some_and(|v| v != 0),
            0x0063 => prot_obj = r.u16().is_some_and(|v| v != 0),
            0x00DD => prot_scen = r.u16().is_some_and(|v| v != 0),
            0x0013 => prot_pw = r.u16().unwrap_or(0),
            0x0867 => {
                // FEATHDR: FrtHeader 12바이트, isf u16 (2 = 보호), 예약 u8, cbHdrData u32,
                // EnhancedProtection u32 (비트 = "허용", objects 부터 selectUnlockedCells 까지)
                r.take(12);
                if r.u16() == Some(2) {
                    r.take(5);
                    prot_enh = r.u32();
                }
            }
            0x009B => sheet.filter_mode = true,
            0x009D => {
                sheet.auto_filter.get_or_insert_with(Default::default);
            }
            0x009E => {
                // AUTOFILTER: iEntry u16, grbit u16 (wJoin 0–1 (0 = and), fSimple1/2, fTopN 0x10,
                // fTop 0x20, fPercent 0x40, wTopN 비트 7–15), DOPER 10바이트 ×2, 이어서 문자열들
                let (Some(entry), Some(grbit)) = (r.u16(), r.u16()) else {
                    continue;
                };
                let dopers = [r.take(10), r.take(10)];
                let mut fc = FilterColumn {
                    col_id: entry as u32,
                    ..Default::default()
                };
                if grbit & 0x10 != 0 {
                    fc.top10 = Some((grbit & 0x20 != 0, grbit & 0x40 != 0, (grbit >> 7) as f64));
                    // 첫 DOPER = 기준값 (filterVal, 한셀은 RK 로 쓴다 — 필터 대조 실험)
                    fc.filter_val = dopers[0].and_then(|d| match d[0] {
                        0x02 => Some(rk_value(u32::from_le_bytes([d[2], d[3], d[4], d[5]]))),
                        0x04 => d[2..10].try_into().ok().map(f64::from_le_bytes),
                        _ => None,
                    });
                } else {
                    for d in dopers.into_iter().flatten() {
                        let op = FILTER_OPERATORS.get(d[1] as usize).copied().unwrap_or("");
                        let op = if op.is_empty() { "equal" } else { op };
                        let val = match d[0] {
                            0x02 => Some(formula::number_text(rk_value(u32::from_le_bytes([
                                d[2], d[3], d[4], d[5],
                            ])))),
                            0x04 => Some(formula::number_text(f64::from_le_bytes(
                                d[2..10].try_into().unwrap_or_default(),
                            ))),
                            // 문자열: cch 는 DOPER 의 6번째 바이트, 내용은 레코드 끝에 (한컴: 플래그 u8 + UTF-16)
                            0x06 => {
                                let cch = d[6] as usize;
                                let _flag = r.u8();
                                r.utf16(cch)
                            }
                            0x08 => Some(if d[3] != 0 {
                                "#ERR".into()
                            } else if d[2] != 0 {
                                "TRUE".into()
                            } else {
                                "FALSE".into()
                            }),
                            0x0C => Some(String::new()),
                            0x0E => Some(" ".into()),
                            _ => None,
                        };
                        if let Some(v) = val {
                            fc.custom.push((op.to_string(), v));
                        }
                    }
                    fc.custom_and = fc.custom.len() == 2 && grbit & 0x03 == 0;
                }
                sheet
                    .auto_filter
                    .get_or_insert_with(Default::default)
                    .columns
                    .push(fc);
            }
            0x009F => {
                // 한컴 확장: 값 목록 필터의 값 하나. iEntry u16, 종류 u8 (0 문자열, 0x0C 빈 칸),
                // 문자열이면 cch u16, 플래그 u8, UTF-16. 바로 앞 AUTOFILTER 의 조건을 대신한다.
                let Some(fc) = sheet
                    .auto_filter
                    .as_mut()
                    .and_then(|f| f.columns.last_mut())
                else {
                    continue;
                };
                if fc.values.is_empty() && !fc.blank {
                    fc.custom.clear();
                    fc.custom_and = false;
                }
                r.take(2);
                match r.u8() {
                    Some(0x0C) => fc.blank = true,
                    Some(_) => {
                        let cch = r.u16().unwrap_or(0) as usize;
                        let _flag = r.u8();
                        if let Some(v) = r.utf16(cch) {
                            fc.values.push(v);
                        }
                    }
                    None => {}
                }
            }
            0x087E => {
                // AUTOFILTER12: FrtRefHeader (rt, grbitFrt, Ref8U: rwFirst, rwLast, colFirst, colLast),
                // iEntry u16, fHideArrow u32, ft u32, cft u32 (동적 필터 종류), cCriteria u32, …
                r.take(4);
                let (Some(r1), Some(r2), Some(c1), Some(c2)) = (r.u16(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                af_ref = Some(
                    CellRange {
                        start: CellRef {
                            row: r1 as u32,
                            col: c1 as u32,
                        },
                        end: CellRef {
                            row: r2 as u32,
                            col: c2 as u32,
                        },
                    }
                    .to_string(),
                );
                let (Some(entry), Some(_hide), Some(_ft), Some(cft)) =
                    (r.u16(), r.u32(), r.u32(), r.u32())
                else {
                    continue;
                };
                let dynamic = DYNAMIC_FILTER_TYPES
                    .get(cft as usize)
                    .filter(|s| cft != 0 && !s.is_empty());
                af12_dynamic = None;
                if let (Some(t), Some(fc)) = (
                    dynamic,
                    sheet
                        .auto_filter
                        .as_mut()
                        .and_then(|f| f.columns.iter_mut().find(|c| c.col_id == entry as u32)),
                ) {
                    fc.custom.clear();
                    fc.custom_and = false;
                    fc.dynamic = Some(t.to_string());
                    af12_dynamic = Some((entry as u32, 0));
                }
            }
            0x087F => {
                // ContinueFrt12: 바로 앞 AUTOFILTER12 의 조건(AF12Criteria) — FrtRefHeader 12 + DOPER 10.
                // 동적 필터면 숫자 DOPER 가 차례로 val, maxVal (평균은 하나, 기간은 >= 시작 · < 끝)
                let Some((col, n)) = af12_dynamic else {
                    continue;
                };
                let Some(dp) = d.get(12..22).filter(|dp| dp[0] == 0x04) else {
                    continue;
                };
                let v = f64::from_le_bytes(dp[2..10].try_into().unwrap_or_default());
                if let Some(fc) = sheet
                    .auto_filter
                    .as_mut()
                    .and_then(|f| f.columns.iter_mut().find(|c| c.col_id == col))
                {
                    match n {
                        0 => fc.dynamic_val = Some(v).filter(|&v| v != 0.0),
                        1 => fc.dynamic_max_val = Some(v).filter(|&v| v != 0.0),
                        _ => {}
                    }
                }
                af12_dynamic = Some((col, n + 1));
            }
            0x002A | 0x002B | 0x0083 | 0x0084 => {
                let on = r.u16().unwrap_or(0) != 0;
                let o = sheet
                    .page
                    .print_options
                    .get_or_insert_with(Default::default);
                match rec.rtype {
                    0x002A => o.headings = on,
                    0x002B => o.grid_lines = on,
                    0x0083 => o.horizontal_centered = on,
                    _ => o.vertical_centered = on,
                }
            }
            0x0026..=0x0029 => {
                let Some(v) = r.f64().map(hwp_inch) else {
                    continue;
                };
                let m = sheet.page.margins.get_or_insert(PageMargins {
                    left: 0.75,
                    right: 0.75,
                    top: 1.0,
                    bottom: 1.0,
                    header: 0.5,
                    footer: 0.5,
                });
                match rec.rtype {
                    0x0026 => m.left = v,
                    0x0027 => m.right = v,
                    0x0028 => m.top = v,
                    _ => m.bottom = v,
                }
            }
            0x0014 | 0x0015 => {
                // HEADER / FOOTER: 비어 있으면 길이 0, 아니면 cch u16, flag u8, UTF-16
                let Some(cch) = r.u16() else { continue };
                let _flag = r.u8();
                let text = canonical_hf_fonts(&r.utf16(cch as usize).unwrap_or_default());
                let h = sheet
                    .page
                    .header_footer
                    .get_or_insert_with(default_header_footer);
                if rec.rtype == 0x0014 {
                    h.odd_header = Some(text);
                } else {
                    h.odd_footer = Some(text);
                }
            }
            0x089C => {
                // HeaderFooter FRT: frt 머리 12 + guid 16 + flags u16 + cch u16 ×4 + 문자열 4개
                r.take(28);
                let Some(flags) = r.u16() else { continue };
                let h = sheet
                    .page
                    .header_footer
                    .get_or_insert_with(default_header_footer);
                h.different_odd_even = flags & 0x0001 != 0;
                h.different_first = flags & 0x0002 != 0;
                h.scale_with_doc = flags & 0x0004 != 0;
                h.align_with_margins = flags & 0x0008 != 0;
                r.take(8);
                let mut texts = Vec::new();
                for _ in 0..4 {
                    let Some(cch) = r.u16() else { break };
                    let _flag = r.u8();
                    texts.push(
                        r.utf16(cch as usize)
                            .filter(|t| !t.is_empty())
                            .map(|t| canonical_hf_fonts(&t)),
                    );
                }
                texts.resize(4, None);
                h.even_header = texts[0].take();
                h.even_footer = texts[1].take();
                h.first_header = texts[2].take();
                h.first_footer = texts[3].take();
            }
            0x00A1 => {
                // SETUP
                let (Some(paper), Some(scale), Some(start), Some(fit_w), Some(fit_h), Some(g)) =
                    (r.u16(), r.u16(), r.u16(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                let (res, vres) = (r.u16(), r.u16());
                let (hdr, ftr) = (r.f64().map(hwp_inch), r.f64().map(hwp_inch));
                let copies = r.u16();
                let no_pls = g & 0x0004 != 0;
                sheet.page.setup = Some(PageSetup {
                    paper_size: Some(paper as u32),
                    scale: Some(scale as u32),
                    first_page_number: Some(start as u32),
                    fit_to_width: Some(fit_w as u32),
                    fit_to_height: Some(fit_h as u32),
                    orientation: Some(
                        if g & 0x0040 != 0 {
                            "default"
                        } else if g & 0x0002 != 0 {
                            "portrait"
                        } else {
                            "landscape"
                        }
                        .into(),
                    ),
                    page_order: (g & 0x0001 != 0).then(|| "overThenDown".into()),
                    use_printer_defaults: !no_pls,
                    black_and_white: g & 0x0008 != 0,
                    draft: g & 0x0010 != 0,
                    use_first_page_number: g & 0x0080 != 0,
                    cell_comments: crate::model::comments_flag(g & 0x0020 != 0, g & 0x0200 != 0),
                    errors: crate::model::errors_flag((g >> 10) & 3),
                    horizontal_dpi: res.map(u32::from),
                    vertical_dpi: vres.map(u32::from),
                    copies: copies.map(u32::from),
                });
                let m = sheet.page.margins.get_or_insert(PageMargins {
                    left: 0.75,
                    right: 0.75,
                    top: 1.0,
                    bottom: 1.0,
                    header: 0.5,
                    footer: 0.5,
                });
                if let Some(v) = hdr {
                    m.header = v;
                }
                if let Some(v) = ftr {
                    m.footer = v;
                }
            }
            0x01B8 => {
                if let Some(h) = parse_hlink(d) {
                    sheet.hyperlinks.push(h);
                }
            }
            0x0800 => {
                // HLINKTOOLTIP ([MS-XLS]): frtRefHeaderNoGrbit = rt u16 + Ref8U (행·열 u16 × 4) 10바이트, 그 뒤
                // wzTooltip (UTF-16, NUL 끝). 한셀은 이 레코드를 쓰지도 읽지도 않는다 (하이퍼링크 대조 실험) — 다른
                // 프로그램이 쓴 것을 위해 범위가 같은 하이퍼링크(없으면 마지막)에 붙인다
                let (Some(_rt), Some(r1), Some(r2), Some(c1), Some(c2)) =
                    (r.u16(), r.u16(), r.u16(), r.u16(), r.u16())
                else {
                    continue;
                };
                let range = CellRange {
                    start: CellRef {
                        row: r1 as u32,
                        col: c1 as u32,
                    },
                    end: CellRef {
                        row: r2 as u32,
                        col: c2 as u32,
                    },
                }
                .to_string();
                let tip = crate::text::utf16le_nul(d.get(10..).unwrap_or_default());
                let i = sheet
                    .hyperlinks
                    .iter()
                    .rposition(|h| h.range == range)
                    .or_else(|| sheet.hyperlinks.len().checked_sub(1));
                if let Some(h) = i.and_then(|i| sheet.hyperlinks.get_mut(i)) {
                    h.tooltip = Some(tip);
                }
            }
            _ => {}
        }
    }
    sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
    let mdw = crate::units::max_digit_width(wb);
    // 한셀은 차트를 다른 그리기 개체보다 먼저 둔다 (관측)
    let mut shapes: Vec<Shape> = chart_anchors
        .iter()
        .map(|&(_, a)| Shape {
            anchor: legacy_anchor(a, 0, sheet, mdw),
            kind: ShapeKind::Chart,
            ..Default::default()
        })
        .collect();
    // 메모 상자: OBJ 번호 → 모양
    let mut note_shapes: HashMap<u16, CommentShape> = HashMap::new();
    if !art.is_empty() {
        let tree = escher::parse(&art);
        let mut k = 0;
        shapes.extend(
            tree.iter()
                .filter_map(|e| legacy_shape(e, &objs, &mut k, sheet, wb, mdw, &mut note_shapes)),
        );
        // 메모 도형의 이름과 그리기 차례 (VML 의 도형 id·도형 차례와 같은 뜻): (OBJ 번호, 이름)
        let mut notes: Vec<(u16, Option<String>)> = Vec::new();
        fn walk(
            e: &escher::EscherShape,
            objs: &[ObjRec],
            k: &mut usize,
            out: &mut Vec<(u16, Option<String>)>,
        ) {
            let obj = objs.get(*k);
            *k += 1;
            for c in &e.children {
                walk(c, objs, k, out);
            }
            if let Some(o) = obj.filter(|o| o.ot == 0x19) {
                out.push((o.id, e.string(0x0380)));
            }
        }
        let mut k = 0;
        for e in &tree {
            walk(e, &objs, &mut k, &mut notes);
        }
        for (z, (id, name)) in notes.into_iter().enumerate() {
            let Some(ci) = note_objs.iter().position(|n| *n == id) else {
                continue;
            };
            if let Some(c) = sheet.comments.get_mut(ci) {
                c.shape_name = name;
                c.z_order = Some(z as u32);
            }
        }
    }
    sheet.shapes = shapes;
    for (c, id) in sheet.comments.iter_mut().zip(&note_objs) {
        c.shape = note_shapes.remove(id);
    }
    if prot {
        let mut states = [false; 16];
        states[0] = true;
        states[1] = prot_obj;
        states[2] = prot_scen;
        if let Some(e) = prot_enh {
            for (i, s) in states.iter_mut().enumerate().skip(1) {
                *s = e & (1 << (i - 1)) == 0;
            }
        } else {
            for (s, (_, d)) in states.iter_mut().zip(SHEET_PROTECTION_FLAGS).skip(3) {
                *s = d;
            }
        }
        sheet.protection = Some(SheetProtection::from_states(
            (prot_pw != 0).then(|| format!("{prot_pw:04X}")),
            states,
        ));
    }
    if let Some(af) = sheet.auto_filter.as_mut() {
        // 범위는 그 시트의 숨은 이름 _xlnm._FilterDatabase 에서 (없으면 AUTOFILTER12 의 범위)
        let idx = wb.sheets.len() as u32;
        af.range = wb
            .defined_names
            .iter()
            .find(|d| d.name == "_xlnm._FilterDatabase" && d.local_sheet_id == Some(idx))
            .map(|d| d.formula.rsplit('!').next().unwrap_or("").replace('$', ""))
            .or(af_ref)
            .unwrap_or_default();
    }
    Ok(())
}

/// 레거시 앵커의 열 안 오프셋 → 픽셀: dx 는 (열 픽셀 − 1)의 1/32768
fn anchor_col_px(sheet: &Sheet, c: u32, dx: u32, mdw: u32) -> u32 {
    let eff = crate::units::sheet_col_px(sheet, c, mdw).saturating_sub(1) as f64;
    (dx as f64 * eff / 32768.0).round() as u32
}

/// 레거시 앵커의 행 안 오프셋 → 픽셀: dy 는 (행 픽셀 − 1)의 1/8192
fn anchor_row_px(sheet: &Sheet, r: u32, dy: u32) -> u32 {
    let eff = crate::units::sheet_row_px(sheet, r).saturating_sub(1) as f64;
    (dy as f64 * eff / 8192.0).round() as u32
}

/// 레거시 메모 상자 (OfficeArt 도형 + OBJ 의 TXO) → 모델. 앵커는 그리기와 같은 단위를 VML 처럼 픽셀로 (메모 대조
/// 실험: 한셀의 레거시 읽기와 같은 `x:Anchor`), 채우기 0x0181 (없으면 흰색), 선 색 0x01C0·두께 0x01CB (EMU, 없으면
/// 검정·9525), 그림자 0x023F (0x02, 사용 0x20000), 셀 고정은 앵커 플래그 (0 옮김·크기, 2 옮김, 3 고정).
/// TXO 의 맞춤: 가로 1 왼쪽, 2 가운데, 3 오른쪽, 5 양쪽, 7 균등 / 세로 0 위, 1 가운데, 2 아래, 3 양쪽, 4 균등
fn legacy_comment_shape(
    e: &escher::EscherShape,
    a: [u32; 8],
    align: (u8, u8),
    sheet: &Sheet,
    mdw: u32,
) -> CommentShape {
    let [c1, dx1, r1, dy1, c2, dx2, r2, dy2] = a;
    let hex = |v: u32| {
        let c = art_color(v);
        format!(
            "#{}",
            c.value.trim_start_matches("srgb:").to_ascii_lowercase()
        )
    };
    let shadow = matches!(e.prop(0x023F), Some(b) if b & 0x0002_0000 != 0 && b & 0x02 != 0);
    CommentShape {
        anchor: [
            c1,
            anchor_col_px(sheet, c1, dx1, mdw),
            r1,
            anchor_row_px(sheet, r1, dy1),
            c2,
            anchor_col_px(sheet, c2, dx2, mdw),
            r2,
            anchor_row_px(sheet, r2, dy2),
        ],
        fill_color: hex(e.prop(0x0181).unwrap_or(0x00FF_FFFF)),
        stroke_color: hex(e.prop(0x01C0).unwrap_or(0)),
        stroke_weight: e.prop(0x01CB).unwrap_or(9525) as f64 / 12700.0,
        shadow,
        move_with_cells: e.anchor_flag != 3,
        size_with_cells: e.anchor_flag == 0,
        text_halign: match align.0 {
            2 => Some("Center".into()),
            3 => Some("Right".into()),
            5 => Some("Justify".into()),
            7 => Some("Distributed".into()),
            _ => None,
        },
        text_valign: match align.1 {
            1 => Some("Center".into()),
            2 => Some("Bottom".into()),
            3 => Some("Justify".into()),
            4 => Some("Distributed".into()),
            _ => None,
        },
        // 도형 이름 0x0380 — 한셀은 VML 의 v:shape id 를 여기에 쓰고 다시 읽는다 (메모 이름 실험: `메모 7`, `foo`, `메모 0`)
        name: e.string(0x0380).filter(|s| !s.is_empty()),
    }
}

/// 레거시 앵커 → 모델. dx 는 (열 픽셀 − 1)의 1/32768, dy 는 (행 픽셀 − 1)의 1/8192 이고
/// 픽셀 단위로 반올림한다 (한셀 대조, docs/format/cell-legacy.md).
pub(crate) fn legacy_anchor(a: [u32; 8], flag: u16, sheet: &Sheet, mdw: u32) -> Anchor {
    let [c1, dx1, r1, dy1, c2, dx2, r2, dy2] = a;
    let col_off =
        |c: u32, dx: u32| anchor_col_px(sheet, c, dx, mdw) as i64 * crate::units::EMU_PER_PX;
    let row_off = |r: u32, dy: u32| anchor_row_px(sheet, r, dy) as i64 * crate::units::EMU_PER_PX;
    Anchor {
        kind: AnchorKind::TwoCell,
        edit_as: match flag {
            2 => Some("oneCell".into()),
            3 => Some("absolute".into()),
            _ => None,
        },
        from: Marker {
            col: c1,
            col_off: col_off(c1, dx1),
            row: r1,
            row_off: row_off(r1, dy1),
        },
        to: Some(Marker {
            col: c2,
            col_off: col_off(c2, dx2),
            row: r2,
            row_off: row_off(r2, dy2),
        }),
        ext: None,
        pos: None,
    }
}

/// CFEX/CF12 의 틀(CFTemp)과 매개 변수(16바이트)로 규칙 종류를 정한다
fn apply_cf_template(rule: &mut CfRule, tmpl: u32, params: &[u8]) {
    let p0 = params.first().copied().unwrap_or(0);
    let p16 = |i: usize| {
        params
            .get(i..i + 2)
            .map(|x| u16::from_le_bytes([x[0], x[1]]))
            .unwrap_or(0)
    };
    // 글 규칙의 글자는 수식의 첫 문자열 상수
    let literal = |f: &[String]| {
        f.first().and_then(|s| {
            let a = s.find('"')?;
            let b = s[a + 1..].find('"')?;
            Some(s[a + 1..a + 1 + b].to_string())
        })
    };
    match tmpl {
        0x00 => {}
        0x01 => rule.kind = "expression".into(),
        0x05 => {
            rule.kind = "top10".into();
            rule.bottom = p0 & 0x01 == 0;
            rule.percent = p0 & 0x02 != 0;
            rule.rank = Some(p16(1) as u32);
            rule.formulas.clear();
        }
        0x08 => {
            let (kind, op) = match p16(0) {
                1 => ("notContainsText", "notContains"),
                2 => ("beginsWith", "beginsWith"),
                3 => ("endsWith", "endsWith"),
                _ => ("containsText", "containsText"),
            };
            rule.kind = kind.into();
            rule.operator = Some(op.into());
            rule.text = literal(&rule.formulas);
        }
        0x09 => rule.kind = "containsBlanks".into(),
        0x0A => rule.kind = "notContainsBlanks".into(),
        0x0B => rule.kind = "containsErrors".into(),
        0x0C => rule.kind = "notContainsErrors".into(),
        0x0F..=0x18 => {
            const PERIODS: [&str; 10] = [
                "today",
                "tomorrow",
                "yesterday",
                "last7Days",
                "lastMonth",
                "nextMonth",
                "thisWeek",
                "nextWeek",
                "lastWeek",
                "thisMonth",
            ];
            rule.kind = "timePeriod".into();
            rule.time_period = PERIODS.get((tmpl - 0x0F) as usize).map(|s| s.to_string());
        }
        0x19 | 0x1A | 0x1D | 0x1E => {
            rule.kind = "aboveAverage".into();
            rule.above_average = matches!(tmpl, 0x1A | 0x1E).then_some(false);
            rule.equal_average = matches!(tmpl, 0x1D | 0x1E);
            rule.std_dev = (p16(0) > 0).then(|| p16(0) as u32);
            rule.formulas.clear();
        }
        0x1B => {
            rule.kind = "duplicateValues".into();
            rule.formulas.clear();
        }
        0x07 | 0x1C => {
            rule.kind = "uniqueValues".into();
            rule.formulas.clear();
        }
        _ => {}
    }
}

/// 기준 값 수식의 토큰열 → 글
type CfvoDecoder<'a> = dyn Fn(&[u8]) -> Option<String> + 'a;

/// CFVO (레거시): 종류 u8 (1 num, 2 min, 3 max, 4 percent, 5 percentile, 7 formula), 수식 cce u16 + 토큰,
/// 수식이 없고 최소·최대가 아니면 숫자 f64. 한셀은 x14 구역에서 읽은 값(`xm:f`)을 수식(PtgInt 등)으로 쓴다
/// (x14 조건부 서식 실험 — 3Stars 의 `percent` 값 0·33·67 이 `1e 00 00` …)
fn legacy_cfvo(r: &mut Rd, dec: &CfvoDecoder) -> Option<Cfvo> {
    let ty = r.u8()?;
    let cce = r.u16()? as usize;
    let tokens = r.take(cce)?;
    let num = if cce > 0 || matches!(ty, 2 | 3) {
        None
    } else {
        r.f64()
    };
    let kind = match ty {
        1 => "num",
        2 => "min",
        3 => "max",
        4 => "percent",
        5 => "percentile",
        6 | 7 => "formula",
        _ => "num",
    };
    Some(Cfvo {
        kind: kind.into(),
        val: if cce > 0 {
            dec(tokens)
        } else {
            num.filter(|_| matches!(ty, 1 | 4 | 5))
                .map(formula::number_text)
        },
        gte: true,
    })
}

/// CFColor: 종류 u32 (2 = RGB, 3 = 테마), 값 4바이트, 밝기 f64. 레거시는 테마 색을 테마 번호 + 밝기로 지킨다
/// (XML 은 한셀이 RGB 로 바꿔 쓴다: 테마 5, 밝기 -0.25 → `ffeb5800` — x14 조건부 서식 실험)
fn legacy_cf_color(r: &mut Rd) -> Option<Color> {
    let ty = r.u32()?;
    let v = r.take(4)?;
    let tint = r.f64().filter(|t| *t != 0.0);
    match ty {
        2 => Some(Color {
            rgb: Some(format!("ff{:02x}{:02x}{:02x}", v[0], v[1], v[2])),
            ..Default::default()
        }),
        3 => Some(Color {
            theme: Some(v[0] as u32),
            tint,
            ..Default::default()
        }),
        _ => Some(Color {
            auto: true,
            ..Default::default()
        }),
    }
}

/// CFGradient: 예약 u8, 보간 수 u8, 색 수 u8, 플래그 u8, (CFVO + 영역 f64) × n, (위치 f64 + CFColor) × m
fn legacy_color_scale(r: &mut Rd, dec: &CfvoDecoder) -> Option<ColorScale> {
    r.take(1);
    let (ni, nc, _f) = (r.u8()?, r.u8()?, r.u8()?);
    let mut cs = ColorScale::default();
    for _ in 0..ni {
        cs.cfvos.push(legacy_cfvo(r, dec)?);
        r.take(8);
    }
    for _ in 0..nc {
        r.take(8);
        cs.colors.extend(legacy_cf_color(r));
    }
    Some(cs)
}

/// CFDatabar: 예약 u8, 플래그 u8 (0x01 오른쪽에서 왼쪽, 0x02 값 숨김), 최소·최대 길이 u8, CFColor, CFVO × 2.
/// x14 확장 가운데 레거시에 남는 것은 길이와 방향(오른쪽에서 왼쪽만)뿐이다 — 테두리·음수 색·축은 저장되지 않는다
/// (x14 조건부 서식 실험. 한셀은 다시 읽을 때 방향도 버리고 `negativeBarColorSameAsPositive="1"
/// axisPosition="none"` 을 채운다)
fn legacy_data_bar(r: &mut Rd, dec: &CfvoDecoder) -> Option<DataBar> {
    r.take(1);
    let (flags, min, max) = (r.u8()?, r.u8()?, r.u8()?);
    let color = legacy_cf_color(r);
    let mut cfvos = Vec::new();
    for _ in 0..2 {
        cfvos.extend(legacy_cfvo(r, dec));
    }
    let ext = (flags & 0x01 != 0).then(|| crate::model::DataBarExt {
        cfvos: cfvos.clone(),
        direction: Some("rightToLeft".into()),
        ..Default::default()
    });
    Some(DataBar {
        cfvos,
        color,
        show_value: flags & 0x02 == 0,
        min_length: Some(min as u32).filter(|&v| v != 10),
        max_length: Some(max as u32).filter(|&v| v != 90),
        ext,
    })
}

/// CFMultistate: 예약 u8, 아이콘 수 u8, 집합 번호 u8, 플래그 u8 (0x01 아이콘만, 0x04 거꾸로),
/// (CFVO + fEqual u8 + 예약 u32) × n. 사용자 지정 아이콘은 저장되지 않는다 (x14 조건부 서식 실험)
fn legacy_icon_set(r: &mut Rd, dec: &CfvoDecoder) -> Option<IconSet> {
    r.take(1);
    let (n, set, flags) = (r.u8()?, r.u8()?, r.u8()?);
    let mut cfvos = Vec::new();
    for _ in 0..n {
        let mut c = legacy_cfvo(r, dec)?;
        c.gte = r.u8()? != 0;
        r.take(4);
        cfvos.push(c);
    }
    const SETS: [&str; 20] = [
        "3Arrows",
        "3ArrowsGray",
        "3Flags",
        "3TrafficLights1",
        "3TrafficLights2",
        "3Signs",
        "3Symbols",
        "3Symbols2",
        "4Arrows",
        "4ArrowsGray",
        "4RedToBlack",
        "4Rating",
        "4TrafficLights",
        "5Arrows",
        "5ArrowsGray",
        "5Rating",
        "5Quarters",
        "3Stars",
        "3Triangles",
        "5Boxes",
    ];
    Some(IconSet {
        set: SETS
            .get(set as usize)
            .unwrap_or(&"3TrafficLights1")
            .to_string(),
        cfvos,
        show_value: flags & 0x01 == 0,
        reverse: flags & 0x04 != 0,
        percent: true,
        custom_icons: Vec::new(),
    })
}

/// OBJ 레코드 요약: 종류(ot)와 뒤따른 TXO 의 내용·런
struct ObjRec {
    ot: u16,
    /// ftCmo 플래그 (0x0001 잠금, 0x0010 인쇄)
    grbit: u16,
    /// ftCmo 의 개체 번호 (NOTE 의 idObj 가 가리킨다)
    id: u16,
    /// TXO 의 가로·세로 맞춤 (메모 상자의 x:TextHAlign·x:TextVAlign)
    align: (u8, u8),
    text: Option<(String, Vec<(u16, u16)>)>,
}

/// OfficeArt 도형 하나 → 모델 (OBJ 는 문서 순서로 하나씩 짝짓는다). 메모(ot 0x19)는 건너뛴다.
#[allow(clippy::too_many_arguments)]
fn legacy_shape(
    e: &escher::EscherShape,
    objs: &[ObjRec],
    k: &mut usize,
    sheet: &Sheet,
    wb: &Workbook,
    mdw: u32,
    notes: &mut HashMap<u16, CommentShape>,
) -> Option<Shape> {
    let obj = objs.get(*k);
    *k += 1;
    let ot = obj.map_or(0x1E, |o| o.ot);
    let children: Vec<Shape> = e
        .children
        .iter()
        .filter_map(|c| legacy_shape(c, objs, k, sheet, wb, mdw, notes))
        .collect();
    // 0x19 메모 (NOTE 가 따로 읽는다 — 여기서는 상자의 모양만), 0x14 콤보 상자 (자동 필터의 단추)
    if ot == 0x19 {
        if let (Some(o), Some(a)) = (obj, e.anchor) {
            notes.insert(o.id, legacy_comment_shape(e, a, o.align, sheet, mdw));
        }
        return None;
    }
    if ot == 0x14 {
        return None;
    }
    let kind = if e.is_group() {
        ShapeKind::Group
    } else {
        match ot {
            0x05 => ShapeKind::Chart,
            0x08 => ShapeKind::Picture,
            0x01 => ShapeKind::Connector,
            _ => ShapeKind::Shape,
        }
    };
    let anchor = e
        .anchor
        .map(|a| legacy_anchor(a, e.anchor_flag, sheet, mdw));
    let prop = |pid: u16| e.prop(pid);
    // 채우기: 0x01BF 의 fUseFilled(0x100000)/fFilled(0x10), 색 0x0181 (COLORREF, 기본 흰색)
    let filled = match prop(0x01BF) {
        Some(b) if b & 0x0010_0000 != 0 => b & 0x10 != 0,
        _ => true,
    };
    let fill = if kind == ShapeKind::Group || kind == ShapeKind::Chart {
        None
    } else if filled {
        Some(Paint::Solid(art_color(prop(0x0181).unwrap_or(0x00FF_FFFF))))
    } else {
        Some(Paint::None)
    };
    // 선: 0x01FF 의 fUseLine(0x80000)/fLine(0x8), 색 0x01C0 (기본 검정), 두께 0x01CB (EMU, 기본 9525)
    let lined = match prop(0x01FF) {
        Some(b) if b & 0x0008_0000 != 0 => b & 0x08 != 0,
        _ => true,
    };
    // 선: 점선 0x01CE (MSOLINEDASHING), 겹선 0x01CD, 이음 0x01D6 (없으면 한셀은 miter 로 읽는다), 끝 0x01D7 (없으면
    // flat), 화살표 0x01D0·0x01D1 (종류) + 0x01D2–0x01D5 (너비·길이) — 도형 대조 실험
    let arrow = |kind: u16, w: u16, len: u16| {
        let size = |v: Option<u32>| {
            match v.unwrap_or(1) {
                0 => "sm",
                2 => "lg",
                _ => "med",
            }
            .to_string()
        };
        Some(crate::model::LineEnd {
            kind: match prop(kind).unwrap_or(0) {
                1 => Some("triangle".into()),
                2 => Some("stealth".into()),
                3 => Some("diamond".into()),
                4 => Some("oval".into()),
                5 => Some("arrow".into()),
                _ => None,
            },
            w: Some(size(prop(w))),
            len: Some(size(prop(len))),
        })
    };
    let line = (kind != ShapeKind::Group && kind != ShapeKind::Chart).then(|| Outline {
        width: Some(prop(0x01CB).unwrap_or(9525) as i64),
        fill: Some(if lined {
            Paint::Solid(art_color(prop(0x01C0).unwrap_or(0)))
        } else {
            Paint::None
        }),
        dash: Some(
            match prop(0x01CE).unwrap_or(0) {
                1 => "sysDash",
                2 => "sysDot",
                3 => "sysDashDot",
                4 => "sysDashDotDot",
                5 => "dot",
                6 => "dash",
                7 => "lgDash",
                8 => "dashDot",
                9 => "lgDashDot",
                10 => "lgDashDotDot",
                _ => "solid",
            }
            .into(),
        ),
        cap: Some(
            match prop(0x01D7).unwrap_or(2) {
                0 => "rnd",
                1 => "sq",
                _ => "flat",
            }
            .into(),
        ),
        compound: Some(
            match prop(0x01CD).unwrap_or(0) {
                1 => "dbl",
                2 => "thickThin",
                3 => "thinThick",
                4 => "tri",
                _ => "sng",
            }
            .into(),
        ),
        join: Some(
            match prop(0x01D6).unwrap_or(1) {
                0 => "bevel",
                2 => "round",
                _ => "miter",
            }
            .into(),
        ),
        head: arrow(0x01D0, 0x01D2, 0x01D3),
        tail: arrow(0x01D1, 0x01D4, 0x01D5),
    });
    // 그림자: 0x023F (0x02 + 사용 0x20000), 색 0x0201 (없으면 회색), 불투명도 0x0204 (16.16), 거리 0x0205·0x0206 (EMU,
    // 없으면 25400), 기준점 0x0210·0x0211 (16.16, ±0.5 = 왼쪽·위/오른쪽·아래 → algn). 흐림은 저장되지 않는다
    let fixed = |pid: u16| prop(pid).map(|v| v as i32 as f64 / 65536.0);
    let shadow =
        matches!(prop(0x023F), Some(b) if b & 0x0002_0000 != 0 && b & 0x02 != 0).then(|| {
            let (ox, oy) = (
                prop(0x0205).map_or(25400, |v| v as i32) as f64,
                prop(0x0206).map_or(25400, |v| v as i32) as f64,
            );
            let mut color = art_color(prop(0x0201).unwrap_or(0x0080_8080));
            if let Some(a) = fixed(0x0204).filter(|&a| a < 1.0) {
                color
                    .mods
                    .push(("alpha".into(), (a * 100_000.0).round() as i64));
            }
            let side = |v: Option<f64>, lo: &'static str, hi: &'static str| match v {
                Some(x) if x < -0.25 => lo,
                Some(x) if x > 0.25 => hi,
                _ => "",
            };
            let (h, v) = (side(fixed(0x0210), "l", "r"), side(fixed(0x0211), "t", "b"));
            let dir = (oy.atan2(ox).to_degrees().rem_euclid(360.0) * 60000.0).round() as i64;
            crate::model::ShapeShadow {
                dist: ox.hypot(oy).round() as i64,
                dir: dir % 21_600_000,
                blur: 0,
                align: Some(if h.is_empty() && v.is_empty() {
                    "ctr".to_string()
                } else {
                    format!("{v}{h}")
                }),
                rot_with_shape: false,
                color,
            }
        });
    // 잠금: 0x007F 비트 0–8 (묶기, 조절점, 글, 꼭짓점, 자르기, 선택, 위치, 비율, 회전), 0x033F 비트 3 (모양 종류).
    // 한셀은 글상자를 늘 회전 잠금으로 읽는다 (도형 대조 실험)
    let mut locks: Vec<String> = Vec::new();
    if let Some(v) = prop(0x007F) {
        const NAMES: [&str; 9] = [
            "noGrp",
            "noAdjustHandles",
            "noTextEdit",
            "noEditPoints",
            "noCrop",
            "noSelect",
            "noMove",
            "noChangeAspect",
            "noRot",
        ];
        for (i, n) in NAMES.iter().enumerate() {
            if v & (1 << i) != 0 && v & (1 << (i + 16)) != 0 {
                locks.push(n.to_string());
            }
        }
    }
    if matches!(prop(0x033F), Some(v) if v & 0x08 != 0 && v & 0x80000 != 0) {
        locks.push("noChangeShapeType".into());
    }
    // (글이 없는 글상자에는 붙이지 않는다 — 한셀 .xls 열기 대조)
    let has_text = obj
        .and_then(|o| o.text.as_ref())
        .is_some_and(|(t, _)| !t.is_empty());
    if e.has_textbox && has_text && kind == ShapeKind::Shape && !locks.iter().any(|l| l == "noRot")
    {
        locks.push("noRot".into());
    }
    // 종류마다 XML 에 있는 잠금만 (a:picLocks·a:cxnSpLocks 에는 noTextEdit 가 없고, noCrop 은 그림만, 그룹은 다섯 가지)
    locks.retain(|l| match kind {
        ShapeKind::Picture => l != "noTextEdit",
        ShapeKind::Connector => l != "noTextEdit" && l != "noCrop",
        ShapeKind::Group => matches!(
            l.as_str(),
            "noGrp" | "noSelect" | "noRot" | "noChangeAspect" | "noMove"
        ),
        _ => l != "noCrop",
    });
    locks.sort();
    let text = obj.and_then(|o| o.text.as_ref()).map(|(t, runs)| {
        let font = runs
            .first()
            .and_then(|&(_, ifnt)| wb.styles.fonts.get(ifnt as usize));
        TextBody {
            anchor: prop(0x0087).map(|a| {
                match a {
                    1 | 4 => "ctr",
                    2 | 5 => "b",
                    _ => "t",
                }
                .into()
            }),
            wrap: prop(0x0085).map(|w| if w == 2 { "none" } else { "square" }.into()),
            vert: None,
            // 레거시에는 글 넘침이 없다 — 한셀은 늘 자름으로 읽는다
            vert_overflow: Some("clip".into()),
            horz_overflow: Some("clip".into()),
            insets: [
                prop(0x0081).map(i64::from),
                prop(0x0082).map(i64::from),
                prop(0x0083).map(i64::from),
                prop(0x0084).map(i64::from),
            ],
            paragraphs: t
                .split('\n')
                .map(|line| DrawingPara {
                    align: None,
                    runs: (!line.is_empty())
                        .then(|| DrawingRun {
                            text: line.to_string(),
                            size: font
                                .and_then(|f| f.size)
                                .map(|s| (s * 100.0).round() as u32),
                            bold: font.map(|f| f.bold),
                            italic: font.map(|f| f.italic),
                            color: font
                                .and_then(|f| f.color.as_ref())
                                .and_then(|c| c.rgb.as_deref())
                                .filter(|h| h.len() >= 6)
                                .map(|h| DrawColor {
                                    value: format!(
                                        "srgb:{}",
                                        h[h.len() - 6..].to_ascii_uppercase()
                                    ),
                                    mods: Vec::new(),
                                }),
                            latin: font.and_then(|f| f.name.clone()),
                            ..Default::default()
                        })
                        .into_iter()
                        .collect(),
                })
                .collect(),
        }
    });
    let hidden = matches!(prop(0x03BF), Some(b) if b & 0x0002_0000 != 0 && b & 0x02 != 0);
    Some(Shape {
        anchor: anchor.unwrap_or_default(),
        kind,
        id: e.spid & 0x3FF,
        name: e.string(0x0380).unwrap_or_default(),
        descr: e.string(0x0381),
        hidden,
        // 묶음 안 도형은 자식 앵커(묶음 안 좌표), 묶음은 자식 좌표 범위를 위치·크기로 (한셀 .xls 는 묶음 안 도형에 이것만 둔다)
        xfrm: e
            .child_anchor
            .or(if e.is_group() { e.group_rect } else { None })
            .map(|a| crate::model::Xfrm {
                x: a[0] as i64,
                y: a[1] as i64,
                cx: (a[2] - a[0]) as i64,
                cy: (a[3] - a[1]) as i64,
                ..Default::default()
            }),
        // 사용자 정의 모양(0)은 꼭짓점(0x0145)이 있을 때만. 없으면 한셀처럼 모양 없음
        geometry: (kind != ShapeKind::Group
            && kind != ShapeKind::Chart
            && !(e.shape_type == 0 && e.complex(0x0145).is_none()))
        .then(|| {
            match e.shape_type {
                0 => "custom",
                1 | 202 => "rect",
                2 => "roundRect",
                3 => "ellipse",
                4 => "diamond",
                5 => "triangle",
                6 => "rtTriangle",
                7 => "parallelogram",
                8 => "trapezoid",
                9 => "hexagon",
                10 => "octagon",
                11 => "plus",
                12 => "star5",
                13 => "rightArrow",
                20 => "line",
                32 => "straightConnector1",
                _ => "rect",
            }
            .into()
        }),
        fill,
        line,
        shadow,
        locks,
        // OBJ ftCmo: fLocked 0x0001, fPrint 0x0010
        locks_with_sheet: obj.map(|o| o.grbit & 0x0001 != 0),
        prints_with_sheet: obj.map(|o| o.grbit & 0x0010 != 0),
        text,
        children,
        // 그림: pib(0x0104) = 그림 저장소 번호 (1부터) → 전역에서 꺼내 둔 xl/media 파트
        target: (kind == ShapeKind::Picture)
            .then(|| prop(0x0104))
            .flatten()
            .and_then(|pib| {
                let prefix = format!("{}{pib}.", MEDIA_PREFIX);
                wb.raw_parts
                    .iter()
                    .find(|p| p.name.starts_with(&prefix))
                    .map(|p| p.name.clone())
            }),
        raw: None,
        raw_rels: Vec::new(),
        chart: None,
    })
}

/// 레거시 그림 저장소에서 꺼낸 그림의 파트 이름 앞부분 (뒤에 pib 와 확장자)
const MEDIA_PREFIX: &str = "xl/media/image";

/// OfficeArt COLORREF (R, G, B, 플래그) → DrawingML 색
fn art_color(v: u32) -> DrawColor {
    let (r, g, b) = (v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF);
    DrawColor {
        value: format!("srgb:{r:02X}{g:02X}{b:02X}"),
        mods: Vec::new(),
    }
}

/// 머리글·바닥글의 글꼴 코드를 한셀이 XML 에 쓰는 꼴로: `&"이름, REGULAR "` → `&"이름,Regular"`.
/// 레거시에는 글꼴 모양이 대문자 + 공백으로 기록된다 (관측).
pub(crate) fn canonical_hf_fonts(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("&\"") {
        out.push_str(&rest[..i + 2]);
        rest = &rest[i + 2..];
        let Some(end) = rest.find('"') else { break };
        let spec = &rest[..end];
        match spec.split_once(',') {
            Some((name, style)) => {
                let style = style.trim();
                let canon = ["Regular", "Bold", "Italic", "Bold Italic"]
                    .into_iter()
                    .find(|c| c.eq_ignore_ascii_case(style))
                    .unwrap_or(style);
                out.push_str(name.trim());
                out.push(',');
                out.push_str(canon);
            }
            None => out.push_str(spec),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// 한셀은 레거시의 여백(인치)을 HWPUNIT(1/7200 인치) 정수로 **잘라** 들이고 f32 로 다시 쓴다 (관측:
/// 머리글 여백 0.51180553 → 3684 → 0.51166666).
fn hwp_inch(v: f64) -> f64 {
    ((v * 7200.0).trunc() / 7200.0) as f32 as f64
}

/// 창 번호 (pnn) → `pane` 의 activePane 값
fn pane_name(pnn: u32) -> Option<String> {
    Some(
        match pnn {
            0 => "bottomRight",
            1 => "topRight",
            2 => "bottomLeft",
            3 => "topLeft",
            _ => return None,
        }
        .into(),
    )
}

fn default_header_footer() -> HeaderFooter {
    HeaderFooter {
        scale_with_doc: true,
        align_with_margins: true,
        ..Default::default()
    }
}

/// HLINK: Ref(행 u32) + CLSID + 하이퍼링크 객체 ([MS-OSHARED] 2.3.7.1).
fn parse_hlink(d: &[u8]) -> Option<Hyperlink> {
    let mut r = Rd::new(d);
    let r1 = r.u32()?;
    let r2 = r.u32()?;
    let c1 = r.u16()?;
    let c2 = r.u16()?;
    r.take(16)?; // CLSID_StdHlink
    let _ver = r.u32()?;
    let flags = r.u32()?;
    let read_hstr = |r: &mut Rd| -> Option<String> {
        let n = r.u32()? as usize;
        let s = r.utf16(n)?;
        Some(s.trim_end_matches('\0').to_string())
    };
    let mut h = Hyperlink {
        range: CellRange {
            start: CellRef {
                row: r1,
                col: c1 as u32,
            },
            end: CellRef {
                row: r2,
                col: c2 as u32,
            },
        }
        .to_string(),
        ..Default::default()
    };
    if flags & 0x10 != 0 {
        h.display = read_hstr(&mut r);
    }
    if flags & 0x80 != 0 {
        let _frame = read_hstr(&mut r);
    }
    if flags & 0x01 != 0 {
        if flags & 0x100 != 0 {
            h.target = read_hstr(&mut r);
        } else {
            let clsid = r.take(16)?;
            match (clsid[0], clsid[1]) {
                // URL 모니커 {79EAC9E0-BAF9-11CE-8C82-00AA004BA90B}
                (0xE0, 0xC9) => {
                    let len = r.u32()? as usize;
                    h.target = Some(crate::text::utf16le_nul(r.take(len)?));
                }
                // 파일 모니커 {00000303-0000-0000-C000-000000000046}
                (0x03, 0x03) => h.target = file_moniker(&mut r),
                // 모르는 모니커: 뒤의 위치를 찾을 수 없다
                _ => return Some(h),
            }
        }
    }
    if flags & 0x08 != 0
        && let Some(loc) = read_hstr(&mut r)
    {
        h.location = Some(loc);
    }
    Some(h)
}

/// 파일 모니커 ([MS-OSHARED] FileMoniker): cAnti u16, ansiLength u32, ansiPath (NUL 끝), endServer u16, 판 u16,
/// 예약 16 + 4, cbUnicodePathSize u32 (0 이 아니면 cbUnicodePathBytes u32, usKeyValue u16, UTF-16 경로).
/// 한셀은 드라이브나 UNC 로 시작하지 않는 경로를 `.\` 를 붙인 상대 경로로 읽는다 (하이퍼링크 대조 실험:
/// `file:///C:/docs/book.xlsx` → `.\file:///C:/docs/book.xlsx`), `cAnti` 만큼 `..\` 를 붙인다
fn file_moniker(r: &mut Rd) -> Option<String> {
    let anti = r.u16()? as usize;
    let n = r.u32()? as usize;
    let ansi = r.take(n)?;
    r.take(2 + 2 + 16 + 4)?;
    let unicode = match r.u32()? {
        0 => None,
        _ => {
            let bytes = r.u32()? as usize;
            r.take(2)?;
            Some(crate::text::utf16le(r.take(bytes)?))
        }
    };
    let path = unicode.unwrap_or_else(|| {
        let end = ansi.iter().position(|&b| b == 0).unwrap_or(ansi.len());
        String::from_utf8_lossy(&ansi[..end]).into_owned()
    });
    let absolute = path.get(1..2) == Some(":") || path.starts_with("\\\\");
    Some(match anti {
        0 if !absolute => format!(".\\{path}"),
        k => format!("{}{path}", "..\\".repeat(k)),
    })
}

/// 0xA001 문단 모양 레코드 (37바이트, 문단 번호가 있으면 41바이트).
/// 열거값은 한글 문단 모양 번호를 따른다 (docs/format/cell-legacy.md).
fn parse_parashape(d: &[u8]) -> ParaShape {
    let mut r = Rd::new(d);
    let mut p = ParaShape::default();
    let nz = |v: i32| (v != 0).then_some(v);
    p.left_margin = r.i32().and_then(nz);
    p.right_margin = r.i32().and_then(nz);
    p.indentation = r.i32().and_then(nz);
    p.prev_spacing = r.i32().and_then(nz);
    p.next_spacing = r.i32().and_then(nz);
    p.line_spacing = r.i32();
    p.line_spacing_type = r.u8().and_then(|v| {
        vocab::LINE_SPACING_TYPE
            .get(v as usize)
            .map(|s| s.to_string())
    });
    let _align = r.u8();
    p.break_latin_word = r
        .u8()
        .map(|v| if v == 0 { "keepWord" } else { "breakWord" }.to_string());
    p.break_non_latin_word = r
        .u8()
        .map(|v| if v == 0 { "keepWord" } else { "breakWord" }.to_string());
    p.condense = r.i32();
    p.text_alignment = r
        .u16()
        .and_then(|v| vocab::TEXT_ALIGNMENT.get(v as usize).map(|s| s.to_string()));
    let _b34 = r.u8();
    let num_type = r.u8().unwrap_or(0);
    p.font_line_height = r.u8().and_then(|v| (v != 0).then_some(true));
    if num_type != 0 {
        p.abstract_num_type = Some(num_type as i32);
        p.abstract_num_id = r.i32();
    }
    p
}

/// 문단 모양을 셀 서식에 붙이고, 한셀처럼 여백에서 들여쓰기 단계를 계산한다
/// (들여쓰기 1단계 = 1134).
fn attach_parashape(xf: &mut Xf, ps: ParaShape) {
    if let Some(a) = xf.alignment.as_mut() {
        let margin = match a.horizontal.as_deref() {
            Some("left") => ps.left_margin,
            Some("right") => ps.right_margin,
            _ => None,
        };
        if let Some(m) = margin.filter(|&m| m > 0) {
            a.indent = Some(((m as f64) / 1134.0).round() as u32);
        }
    }
    xf.hancom = Some(HancomXfExt {
        apply_extension: true,
        parashape: Some(ps),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxfn_alignment_is_one_above_the_standard() {
        // 가로·세로·줄 바꿈·회전 지정 (플래그 낮은 4비트 0), 맞춤 블록 (0x08000000)
        let mut d = 0x0800_00F0u32.to_le_bytes().to_vec();
        d.extend_from_slice(&[0, 0]);
        // 가로 2 = 왼쪽, 줄 바꿈, 세로 3 = 아래, 90도
        d.extend_from_slice(&[0x3A, 90, 0, 0, 0, 0, 0, 0]);
        let a = parse_dxfn(&d).unwrap().alignment.unwrap();
        assert_eq!(a.horizontal.as_deref(), Some("left"));
        assert_eq!(a.vertical.as_deref(), Some("bottom"));
        assert!(a.wrap_text);
        assert_eq!(a.text_rotation, Some(90));
        // 분산(7 + 1)은 3비트에서 넘쳐 0 이 된다
        let mut d = 0x0800_00FEu32.to_le_bytes().to_vec();
        d.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let a = parse_dxfn(&d).unwrap().alignment.unwrap();
        assert_eq!(a.horizontal.as_deref(), Some("distributed"));
        assert_eq!(a.vertical, None);
    }

    /// 차등 서식 22개, 번호 10 에만 안쪽 선 (가로 h, 세로 v)
    fn dxf_table(h: Option<&str>, v: Option<&str>) -> Vec<crate::model::Dxf> {
        let edge = |s: Option<&str>| {
            s.map(|s| BorderEdge {
                style: Some(s.into()),
                color: None,
            })
        };
        let mut t = vec![crate::model::Dxf::default(); 22];
        t[10].border = Some(Border {
            horizontal: edge(h),
            vertical: edge(v),
            ..Default::default()
        });
        t
    }

    fn cf_tail(id: u32, ext: &[u8]) -> (Vec<u8>, usize) {
        let mut d = vec![0xAA; 8];
        d.extend_from_slice(&id.to_le_bytes());
        d.extend_from_slice(ext);
        let fstart = d.len();
        d.extend_from_slice(&[0x1D, 1]);
        (d, fstart)
    }

    #[test]
    fn cf_dxf_id_skips_inner_border_extension() {
        // 안쪽 가로(이중)·세로(점선) 선 확장 12바이트
        let t = dxf_table(Some("double"), Some("dotted"));
        let (d, f) = cf_tail(10, &[1, 6, 0x61, 0x82, 0xD6, 0, 1, 4, 0, 0, 0xFF, 0]);
        assert_eq!(cf_dxf_id(&d, f, true, &t), Some(10));
        // 검은 이중 가로 + 점선 세로: 색 바이트 0 이라 7바이트 나눔도 구조가 맞는다
        let (d, f) = cf_tail(10, &[1, 6, 0, 0, 0, 0, 1, 4, 0, 0, 0xFF, 0]);
        assert_eq!(cf_dxf_id(&d, f, true, &t), Some(10));
        // 검은 가는 가로선만: 2바이트 나눔이 선 종류 1 을 번호로 읽게 된다 — 표와 맞는 쪽, 없으면 긴 쪽
        let t = dxf_table(Some("thin"), None);
        let (d, f) = cf_tail(10, &[1, 1, 0, 0, 0, 0, 0]);
        assert_eq!(cf_dxf_id(&d, f, true, &t), Some(10));
        assert_eq!(cf_dxf_id(&d, f, true, &[]), Some(10));
        // 안쪽 선이 없으면 확장은 2바이트
        let (d, f) = cf_tail(3, &[0, 0]);
        assert_eq!(cf_dxf_id(&d, f, true, &t), Some(3));
        let mut d = vec![0xAA, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&[0, 0]);
        let f = d.len();
        d.extend_from_slice(&[0x1D, 1]);
        assert_eq!(cf_dxf_id(&d, f, true, &t), Some(1));
    }

    /// x14 조건부 서식 실험에서 한셀이 쓴 CF12 의 기준 값: 수식으로 쓴 값(PtgInt), 종류 7 = 수식, 숫자 f64
    #[test]
    fn legacy_cfvo_values() {
        let dec = |t: &[u8]| match t {
            [0x1e, v, 0] => Some(v.to_string()),
            [0x44, ..] => Some("$H$12".to_string()),
            _ => None,
        };
        let mut b = vec![4, 3, 0, 0x1e, 33, 0];
        b.extend_from_slice(&[7, 7, 0, 0x44, 0x0b, 0, 0, 0, 7, 0]);
        b.push(5);
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&10.0f64.to_le_bytes());
        b.extend_from_slice(&[2, 0, 0]);
        let mut r = Rd::new(&b);
        let v: Vec<_> = (0..4)
            .map(|_| {
                let c = legacy_cfvo(&mut r, &dec).unwrap();
                (c.kind, c.val)
            })
            .collect();
        assert_eq!(
            v,
            [
                ("percent".into(), Some("33".into())),
                ("formula".into(), Some("$H$12".into())),
                ("percentile".into(), Some("10".into())),
                ("min".into(), None)
            ]
        );
        assert_eq!(r.left(), 0);
    }

    #[test]
    fn xfext_gradient() {
        let mut v = 0u32.to_le_bytes().to_vec(); // 직선
        for x in [45.0f64, 0.0, 0.0, 0.0, 0.0] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&[2, 0, 0xFF, 0, 0, 0]); // RGB 빨강
        v.extend_from_slice(&0.5f64.to_le_bytes());
        v.extend_from_slice(&0.0f64.to_le_bytes());
        let g = ext_gradient(&mut Rd::new(&v)).unwrap();
        assert!(!g.path);
        assert_eq!(g.degree, 45.0);
        assert_eq!(g.stops[0].position, 0.5);
        assert_eq!(g.stops[0].color.rgb.as_deref(), Some("ffff0000"));
    }
}
