// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 바이너리 OOXML (`workbook.bin` 등) → [`Workbook`].
//!
//! 레코드 구조는 공개 명세 [MS-XLSB] 를 따른다. 한셀의 `xlCellb` 형식은 이 명세 그대로이며
//! 한컴 글꼴/문단 확장은 담지 않는다 (docs/format/cell-binary.md).

use std::collections::HashMap;

use super::biff12::{self, Record};
use crate::error::{Error, Result};
use crate::legacy::formula::{self, Context, Decoded};
use crate::model::{
    Alignment, AutoFilter, Border, BorderEdge, Cell, CellRange, CellRef, CellValue, CfRule, Cfvo,
    ColInfo, Color, Comment, ConditionalFormat, DYNAMIC_FILTER_TYPES, DataBar, DataValidation,
    DefinedName, ExternalLink, FILTER_OPERATORS, FileVersion, Fill, FilterColumn, Font, Formula,
    GradientFill, GradientStop, HeaderFooter, Hyperlink, IconSet, NamedStyle, NumFmt, PageMargins,
    PageSetup, Pane, PrintOptions, Protection, RichText, RowInfo, Sheet, SheetKind,
    SheetProtection, SheetState, Styles, TABLE_STYLE_TYPES, TableStyle, TableStyleElement,
    TableStyles, TextRun, Workbook, WorkbookProtection, Xf,
};
use crate::package::{Package, Relationship};

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
        let s = self.b.get(self.pos..self.pos.checked_add(n)?)?;
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
    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.take(4)?;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// XLWideString: cch u32 + UTF-16
    fn wstr(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        Some(crate::text::utf16le(self.take(n.checked_mul(2)?)?))
    }
    /// XLNullableWideString: cch 0xFFFFFFFF 이면 없음
    fn nwstr(&mut self) -> Option<Option<String>> {
        let n = self.u32()?;
        if n == 0xFFFF_FFFF {
            return Some(None);
        }
        Some(Some(crate::text::utf16le(
            self.take((n as usize).checked_mul(2)?)?,
        )))
    }
}

fn bad(what: &str) -> Error {
    Error::InvalidValue {
        context: "바이너리 .cell".into(),
        value: what.into(),
    }
}

/// 메모: `commentsN.bin` (BrtCommentAuthor 632, BrtBeginComment 635, BrtCommentText 637)
/// + 표시 여부는 XML 과 같은 VML 파트.
fn read_comments(
    pkg: &mut Package,
    srels: &[Relationship],
    fonts: &[Font],
    sheet: &mut Sheet,
) -> Result<()> {
    let Some(rel) = srels.iter().find(|r| r.rel_type.ends_with("/comments")) else {
        return Ok(());
    };
    let data = read_part(pkg, &rel.target)?;
    let mut authors: Vec<String> = Vec::new();
    for rec in biff12::records(&data) {
        let mut r = Rd::new(rec.data);
        match rec.rtype {
            632 => authors.push(r.wstr().unwrap_or_default()),
            635 => {
                // iauthor u32, RfX (rwFirst, rwLast, colFirst, colLast), guid 16
                let (Some(a), Some(row), Some(_), Some(col)) = (r.u32(), r.u32(), r.u32(), r.u32())
                else {
                    continue;
                };
                sheet.comments.push(Comment {
                    cell: CellRef { row, col },
                    author: authors.get(a as usize).cloned().unwrap_or_default(),
                    ..Default::default()
                });
            }
            637 => {
                if let (Some(c), Some(t)) = (sheet.comments.last_mut(), rich_str(&mut r, fonts)) {
                    c.text = t;
                }
            }
            _ => {}
        }
    }
    if let Some(v) = srels.iter().find(|r| r.rel_type.ends_with("/vmlDrawing"))
        && let Some(e) = pkg.read_xml(&v.target)?
    {
        crate::parser::comments::apply_vml(&e, &mut sheet.comments);
    }
    Ok(())
}

/// UncheckedSqRfX: crfx u32 + (rwFirst, rwLast, colFirst, colLast u32) × crfx → "A1:B2 C3"
fn sqrfx(r: &mut Rd) -> String {
    let n = r.u32().unwrap_or(0);
    let mut v = Vec::new();
    for _ in 0..n {
        let (Some(r1), Some(r2), Some(c1), Some(c2)) = (r.u32(), r.u32(), r.u32(), r.u32()) else {
            break;
        };
        v.push(
            CellRange {
                start: CellRef { row: r1, col: c1 },
                end: CellRef { row: r2, col: c2 },
            }
            .to_string(),
        );
    }
    v.join(" ")
}

/// FRTHeader: 플래그 u32 (0x01 fRef, 0x02 fSqref, 0x04 fFormula, 0x08 fRelID) 뒤에 그 목록들 ([MS-XLSB]).
/// 첫 범위 목록과 수식 토큰열들을 돌려준다. 수식은 `플래그 u32, cce u32, cb u32, rgce, rgcb` (관측: BrtCFVO14)
fn frt_header<'a>(r: &mut Rd<'a>) -> Option<(Option<String>, Vec<&'a [u8]>)> {
    let flags = r.u32()?;
    if flags & 0x01 != 0 {
        let n = r.u32()? as usize;
        r.take(n.checked_mul(20)?)?;
    }
    let mut sqref = None;
    if flags & 0x02 != 0 {
        for i in 0..r.u32()? {
            r.u32()?;
            let s = sqrfx(r);
            if i == 0 {
                sqref = Some(s);
            }
        }
    }
    let mut formulas = Vec::new();
    if flags & 0x04 != 0 {
        for _ in 0..r.u32()? {
            r.u32()?;
            let cce = r.u32()? as usize;
            let cb = r.u32()? as usize;
            formulas.push(r.take(cce)?);
            r.take(cb)?;
        }
    }
    if flags & 0x08 != 0 {
        r.nwstr()?;
    }
    Some((sqref, formulas))
}

/// x14 구역의 색 (BrtColor14 의 BrtColor). 한셀은 RGB 색의 색인 바이트에 빨강 값을 넣고, 색인 색은 RGB 종류에
/// 색인 바이트만 채워(RGB 0) 쓴다 (x14 조건부 서식 실험: `indexed="10"` → `05 0a 00 00 00 00 00 ff`).
/// 자동 색도 검정과 같은 바이트로 쓰므로 둘은 구별할 수 없다
fn x14_color(b: &[u8]) -> Option<Color> {
    let (kind, index, rgb) = (b[0] >> 1, b[1], &b[4..7]);
    Some(match kind {
        0 => Color {
            auto: true,
            ..Default::default()
        },
        1 => Color {
            indexed: Some(index as u32),
            ..Default::default()
        },
        3 => Color {
            theme: Some(index as u32),
            ..Default::default()
        },
        _ if index != rgb[0] && rgb == [0, 0, 0] => Color {
            indexed: Some(index as u32),
            ..Default::default()
        },
        _ => Color {
            rgb: Some(format!("ff{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])),
            ..Default::default()
        },
    })
}

/// BrtColor14 들을 데이터 막대의 색 자리(테두리, 음수 채우기, 음수 테두리, 축)에 놓는다. 한셀은 모델에 있는 색만
/// 이 순서로 쓰고 종류를 적지 않는다 (색 조합 실험: 색 조합이 달라도 BrtBeginDatabar14 는 같다) — 넷이면 모두,
/// 아니면 플래그로 쓰임새가 있는 자리(테두리 켬, 음수 색 다름, 테두리 켬 + 음수 테두리 다름, 축 있음)를 고르고,
/// 수가 맞지 않으면 음수 채우기 → 축 → 테두리 → 음수 테두리 순으로 넣고 뺀다 (Excel·한셀이 흔히 쓰는 조합:
/// 넷, 테두리·음수 채우기·축, 음수 채우기·축)
fn assign_bar_colors(colors: Vec<Color>, ext: &mut crate::model::DataBarExt) {
    let n = colors.len().min(4);
    let expected = [
        ext.border,
        !ext.negative_same_as_positive,
        ext.border && !ext.negative_border_same_as_positive,
        ext.axis_position.as_deref() != Some("none"),
    ];
    let mut on = expected;
    const PRIORITY: [usize; 4] = [1, 3, 0, 2];
    while on.iter().filter(|&&x| x).count() > n {
        if let Some(&k) = PRIORITY.iter().rev().find(|&&k| on[k]) {
            on[k] = false;
        }
    }
    while on.iter().filter(|&&x| x).count() < n {
        if let Some(&k) = PRIORITY.iter().find(|&&k| !on[k]) {
            on[k] = true;
        }
    }
    let mut it = colors.into_iter();
    for (k, slot) in [
        &mut ext.border_color,
        &mut ext.negative_fill_color,
        &mut ext.negative_border_color,
        &mut ext.axis_color,
    ]
    .into_iter()
    .enumerate()
    {
        if on[k] {
            *slot = it.next();
        }
    }
}

/// x14 구역의 규칙 하나 (BrtBeginCFRule14 … BrtEndCFRule14)
#[derive(Default)]
struct X14Rule {
    sqref: String,
    kind: u32,
    priority: u32,
    guid: [u8; 16],
    /// BrtBeginDatabar14: 길이 최소·최대, 값 표시, 방향, 축 위치, 플래그
    bar: Option<[u8; 6]>,
    /// BrtBeginIconSet14: 집합 번호, 플래그
    icons: Option<(i32, u16)>,
    cfvos: Vec<Cfvo>,
    colors: Vec<Color>,
    custom_icons: Vec<(String, u32)>,
}

/// x14 아이콘 집합 번호 (-1 은 아이콘 없음 `NoIcons`)
fn icon_set_name(v: i32) -> String {
    usize::try_from(v)
        .ok()
        .and_then(|i| ICON_SETS.get(i))
        .unwrap_or(&"NoIcons")
        .to_string()
}

/// x14 구역 규칙을 본 규칙(BrtCFRuleExt 의 GUID 로 이어진 것)에 더하거나 x14 구역 규칙으로 더한다.
/// 데이터 막대·아이콘 집합만 — 한셀은 x14 구역 수식 규칙의 수식을 바이너리에 쓰지 않는다 (x14 조건부 서식 실험)
fn merge_x14_rules(
    sheet: &mut Sheet,
    rules: Vec<X14Rule>,
    guids: &HashMap<[u8; 16], (usize, usize)>,
) {
    for x in rules {
        let target = guids.get(&x.guid).copied();
        let mut rule_icons = x.icons.map(|(set, f)| IconSet {
            set: icon_set_name(set),
            cfvos: x.cfvos.clone(),
            show_value: f & 0x02 == 0,
            reverse: f & 0x04 != 0,
            percent: true,
            custom_icons: x.custom_icons.clone(),
        });
        let ext = x.bar.map(|[_, _, _, dir, axis, f]| {
            let mut e = crate::model::DataBarExt {
                cfvos: x.cfvos.clone(),
                border: f & 0x01 != 0,
                gradient: f & 0x02 != 0,
                direction: match dir {
                    1 => Some("leftToRight".into()),
                    2 => Some("rightToLeft".into()),
                    _ => None,
                },
                negative_same_as_positive: f & 0x04 == 0,
                negative_border_same_as_positive: f & 0x08 == 0,
                axis_position: match axis {
                    1 => Some("middle".into()),
                    2 => Some("none".into()),
                    _ => None,
                },
                ..Default::default()
            };
            assign_bar_colors(x.colors.clone(), &mut e);
            e
        });
        let apply_bar = |bar: &mut DataBar| {
            if let (Some([min, max, show, ..]), Some(e)) = (x.bar, ext.clone()) {
                bar.min_length = Some(min as u32).filter(|&v| v != 10);
                bar.max_length = Some(max as u32).filter(|&v| v != 90);
                bar.show_value = show != 0;
                bar.ext = Some(e);
            }
        };
        let main = target.and_then(|(i, j)| {
            sheet
                .conditional_formats
                .get_mut(i)
                .and_then(|c| c.rules.get_mut(j))
        });
        match (x.kind, main) {
            (4, Some(rule)) => {
                if let Some(bar) = rule.data_bar.as_mut() {
                    apply_bar(bar);
                }
            }
            (6, Some(rule)) => {
                if let Some(i) = rule_icons.take() {
                    rule.icon_set = Some(i);
                }
            }
            (4 | 6, None) => {
                let mut rule = CfRule {
                    kind: if x.kind == 4 { "dataBar" } else { "iconSet" }.into(),
                    priority: x.priority,
                    x14: true,
                    icon_set: rule_icons.take(),
                    ..Default::default()
                };
                if x.kind == 4 {
                    let mut bar = DataBar {
                        cfvos: x.cfvos.clone(),
                        show_value: true,
                        ..Default::default()
                    };
                    apply_bar(&mut bar);
                    rule.data_bar = Some(bar);
                }
                sheet.conditional_formats.push(ConditionalFormat {
                    sqref: x.sqref.clone(),
                    rules: vec![rule],
                });
            }
            _ => {}
        }
    }
}

/// 아이콘 집합 번호 → 이름 ([MS-XLSB] 2.5.75 IconSet)
const ICON_SETS: [&str; 20] = [
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

/// BrtBeginCFRule 의 종류·틀·매개 변수 → 규칙 (색 눈금 등 세부는 뒤 레코드가 채운다)
#[allow(clippy::too_many_arguments)]
fn cf_rule(
    ty: u32,
    template: u32,
    dxf: u32,
    priority: u32,
    param: u32,
    flags: u16,
    text: Option<String>,
    formulas: Vec<String>,
) -> CfRule {
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
    let mut r = CfRule {
        priority,
        dxf_id: (dxf != u32::MAX).then_some(dxf),
        stop_if_true: flags & 0x02 != 0,
        formulas,
        ..Default::default()
    };
    match ty {
        1 => {
            r.kind = "cellIs".into();
            r.operator = OPS
                .get(param as usize)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
        }
        3 => r.kind = "colorScale".into(),
        4 => r.kind = "dataBar".into(),
        5 => {
            r.kind = "top10".into();
            r.rank = Some(param);
            r.bottom = flags & 0x08 != 0;
            r.percent = flags & 0x10 != 0;
        }
        6 => r.kind = "iconSet".into(),
        _ => {
            // 식 계열: 틀(CFTemp)로 종류를 정한다
            let (kind, op) = match template {
                0x08 => match param {
                    1 => ("notContainsText", "notContains"),
                    2 => ("beginsWith", "beginsWith"),
                    3 => ("endsWith", "endsWith"),
                    _ => ("containsText", "containsText"),
                },
                0x09 => ("containsBlanks", ""),
                0x0A => ("notContainsBlanks", ""),
                0x0B => ("containsErrors", ""),
                0x0C => ("notContainsErrors", ""),
                0x0F..=0x18 => ("timePeriod", ""),
                0x19 | 0x1A | 0x1D | 0x1E => ("aboveAverage", ""),
                0x1B => ("duplicateValues", ""),
                0x07 | 0x1C => ("uniqueValues", ""),
                _ => ("expression", ""),
            };
            r.kind = kind.into();
            if !op.is_empty() {
                r.operator = Some(op.into());
                r.text = text;
            }
            if kind == "timePeriod" {
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
                r.time_period = PERIODS
                    .get((template - 0x0F) as usize)
                    .map(|s| s.to_string());
            }
            if kind == "aboveAverage" {
                r.above_average = matches!(template, 0x1A | 0x1E).then_some(false);
                r.equal_average = matches!(template, 0x1D | 0x1E);
                r.std_dev = (param > 0).then_some(param);
            }
            // 이 종류들은 수식이 규칙에 딸린 것이 아니다 (한셀 XML 에 없다)
            if matches!(kind, "duplicateValues" | "uniqueValues" | "aboveAverage") {
                r.formulas.clear();
            }
        }
    }
    r
}

/// BrtDVal / 레거시 DV 의 플래그 u32 → 데이터 유효성 (문자열 순서: 오류 제목, 오류, 입력 제목, 입력)
pub(crate) fn dv_from_flags(
    flags: u32,
    sqref: String,
    s: &mut [Option<String>],
    f1: Option<String>,
    f2: Option<String>,
) -> DataValidation {
    const KINDS: [&str; 8] = [
        "none",
        "whole",
        "decimal",
        "list",
        "date",
        "time",
        "textLength",
        "custom",
    ];
    const OPS: [&str; 8] = [
        "between",
        "notBetween",
        "equal",
        "notEqual",
        "greaterThan",
        "lessThan",
        "greaterThanOrEqual",
        "lessThanOrEqual",
    ];
    let op = ((flags >> 20) & 0x0F) as usize;
    let style = (flags >> 4) & 0x07;
    DataValidation {
        sqref,
        kind: KINDS
            .get((flags & 0x0F) as usize)
            .unwrap_or(&"none")
            .to_string(),
        operator: (op != 0).then(|| OPS.get(op).unwrap_or(&"between").to_string()),
        formula1: f1,
        formula2: f2,
        allow_blank: flags & 0x0100 != 0,
        hide_drop_down: flags & 0x0200 != 0,
        show_input: flags & 0x0004_0000 != 0,
        show_error: flags & 0x0008_0000 != 0,
        error_style: match style {
            1 => Some("warning".into()),
            2 => Some("information".into()),
            _ => None,
        },
        error_title: s.first_mut().and_then(Option::take),
        error: s.get_mut(1).and_then(Option::take),
        prompt_title: s.get_mut(2).and_then(Option::take),
        prompt: s.get_mut(3).and_then(Option::take),
    }
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

fn read_part(pkg: &mut Package, name: &str) -> Result<Vec<u8>> {
    pkg.read_bytes(name)?
        .ok_or_else(|| Error::MissingPart(name.to_string()))
}

/// BrtColor (8바이트).
fn color(r: &mut Rd) -> Option<Option<Color>> {
    let b0 = r.u8()?;
    let index = r.u8()?;
    let tint = r.i16()?;
    let (red, green, blue, _a) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
    let kind = b0 >> 1;
    let tint = (tint != 0).then(|| tint as f64 / 32767.0);
    Some(match kind {
        0 => Some(Color {
            auto: true,
            ..Default::default()
        }),
        1 => Some(Color {
            indexed: Some(index as u32),
            tint,
            ..Default::default()
        }),
        2 => Some(Color {
            rgb: Some(format!("ff{red:02x}{green:02x}{blue:02x}")),
            tint,
            ..Default::default()
        }),
        3 => Some(Color {
            theme: Some(index as u32),
            tint,
            ..Default::default()
        }),
        _ => None,
    })
}

/// 셀 공통 머리 (열 u32 + 스타일 24비트).
fn cell_head(r: &mut Rd) -> Option<(u32, u32)> {
    let col = r.u32()?;
    let s = r.u32()? & 0x00FF_FFFF;
    Some((col, s))
}

fn rk_value(rk: u32) -> f64 {
    let v = if rk & 0x02 != 0 {
        ((rk as i32) >> 2) as f64
    } else {
        f64::from_bits(((rk & 0xFFFF_FFFC) as u64) << 32)
    };
    if rk & 0x01 != 0 { v / 100.0 } else { v }
}

const BORDER_STYLES: [&str; 14] = [
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
const PATTERNS: [&str; 19] = [
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
const H_ALIGN: [&str; 8] = [
    "general",
    "left",
    "center",
    "right",
    "fill",
    "justify",
    "centerContinuous",
    "distributed",
];
const V_ALIGN: [&str; 5] = ["top", "center", "bottom", "justify", "distributed"];

/// BrtColor 인데 RGB 가 유효(첫 바이트 0x01)하면 종류와 상관없이 RGB 로 (조건부 서식·차등 서식)
fn valid_rgb_color(r: &mut Rd) -> Option<Color> {
    let b = r.take(8)?;
    if b[0] & 0x01 != 0 && b[0] >> 1 != 3 {
        return Some(Color {
            rgb: Some(format!("ff{:02x}{:02x}{:02x}", b[4], b[5], b[6])),
            ..Default::default()
        });
    }
    color(&mut Rd::new(b)).flatten()
}

/// 차등 서식의 XFPropColor: 종류 0 은 자동이다. RGB 가 유효(0x01)해도 그렇다 (한셀 확인: 차등 서식 대조 실험 —
/// 글꼴 색·테두리 색 모두 `<x:color auto="1"/>`)
fn dxf_color(r: &mut Rd) -> Option<Color> {
    let b = r.take(8)?;
    // 레거시는 자동 색을 RGB 종류로 쓰고 값 FF FF FF DF 로 표시한다 (한셀의 자동 색 표지)
    if b[0] >> 1 == 0 || b[4..8] == [0xFF, 0xFF, 0xFF, 0xDF] {
        return Some(Color {
            auto: true,
            ..Default::default()
        });
    }
    valid_rgb_color(&mut Rd::new(b))
}

/// 차등 서식의 테두리 선 (XFPropBorder: XFPropColor + dg u16). dg 0 은 '선 없음' 으로 지정된 선이다
fn dxf_edge(r: &mut Rd) -> Option<crate::model::BorderEdge> {
    let color = dxf_color(r);
    let dg = r.u16()? as usize;
    Some(crate::model::BorderEdge {
        style: (dg != 0)
            .then(|| BORDER_STYLES.get(dg).map(|s| s.to_string()))
            .flatten(),
        color,
    })
}

/// BrtDXF (507): 플래그 u16 (0x8000 fNewBorder), 예약 u16, cprops u16, XFProp(종류 u16, 크기 u16(머리 포함), 값) × cprops.
/// 레거시 DXF(0x088D) 도 FRT 머리 뒤가 같다. 종류 번호는 [MS-XLSB] XFProp 그대로다 (한셀 확인: 차등 서식 대조
/// 실험 — 0x1A 밑줄 u16, 0x1C 기울임, 0x1D 취소선, 0x24 크기(1/20 pt), 0x0B·0x0C 안쪽 세로·가로 선).
/// 한셀은 차등 서식의 글꼴 이름(0x18)을 XML 에 쓰지 않고 바이너리에는 깨진 문자열을 쓴다 — 글꼴이 있다는 것만 본다.
/// 글꼴 크기(0x24)는 바이너리가 1/20 pt, 레거시가 1/100 pt 다 (`legacy`). 한셀은 차등 서식의 크기를 정수 pt 로 다룬다.
pub(crate) fn parse_dxf(d: &[u8], legacy: bool) -> crate::model::Dxf {
    use crate::model::{Alignment, Border, Dxf, Fill, Font, Protection};
    let mut r = Rd::new(d);
    r.take(4);
    let n = r.u16().unwrap_or(0);
    let mut font: Option<Font> = None;
    let mut fill: Option<Fill> = None;
    let mut border: Option<Border> = None;
    let mut align: Option<Alignment> = None;
    let mut prot: Option<Protection> = None;
    let mut fmt_id: Option<u16> = None;
    let mut fmt_code: Option<String> = None;
    for _ in 0..n {
        let (Some(ty), Some(cb)) = (r.u16(), r.u16()) else {
            break;
        };
        let Some(v) = r.take((cb as usize).saturating_sub(4)) else {
            break;
        };
        let mut x = Rd::new(v);
        let on = |x: &mut Rd| x.u8().unwrap_or(0) != 0;
        match ty {
            // 채우기: 무늬 u8 (0x28 = 그라데이션), 무늬 색, 배경색, 그라데이션(0x03)·정지점(0x04)
            0x00 => {
                let p = x.u8().unwrap_or(0);
                let f = fill.get_or_insert_with(Default::default);
                f.pattern_type = PATTERNS.get(p as usize).map(|s| s.to_string());
                if p == 0x28 {
                    f.gradient.get_or_insert_with(Default::default);
                }
            }
            0x01 => fill.get_or_insert_with(Default::default).fg_color = dxf_color(&mut x),
            0x02 => fill.get_or_insert_with(Default::default).bg_color = dxf_color(&mut x),
            0x03 => {
                // XFPropGradient: 종류 u32 (0 직선, 1 경로), 각도·왼쪽·오른쪽·위·아래 f64
                let g = fill
                    .get_or_insert_with(Default::default)
                    .gradient
                    .get_or_insert_with(Default::default);
                g.path = x.u32() == Some(1);
                g.degree = x.f64().unwrap_or(0.0);
                g.left = x.f64().unwrap_or(0.0);
                g.right = x.f64().unwrap_or(0.0);
                g.top = x.f64().unwrap_or(0.0);
                g.bottom = x.f64().unwrap_or(0.0);
            }
            0x04 => {
                // XFPropGradientStop: 예약 u16, 위치 f64, XFPropColor
                x.take(2);
                let position = x.f64().unwrap_or(0.0);
                let color = dxf_color(&mut x).unwrap_or_default();
                fill.get_or_insert_with(Default::default)
                    .gradient
                    .get_or_insert_with(Default::default)
                    .stops
                    .push(GradientStop { position, color });
            }
            0x05 => font.get_or_insert_with(Default::default).color = dxf_color(&mut x),
            // 테두리: 위, 아래, 왼쪽, 오른쪽, 대각선, 안쪽 세로, 안쪽 가로, 대각선 방향 두 가지
            0x06..=0x0C => {
                let e = dxf_edge(&mut x);
                let b = border.get_or_insert_with(Default::default);
                match ty {
                    0x06 => b.top = e,
                    0x07 => b.bottom = e,
                    0x08 => b.left = e,
                    0x09 => b.right = e,
                    0x0A => b.diagonal = e,
                    0x0B => b.vertical = e,
                    _ => b.horizontal = e,
                }
            }
            0x0D => border.get_or_insert_with(Default::default).diagonal_up = on(&mut x),
            0x0E => border.get_or_insert_with(Default::default).diagonal_down = on(&mut x),
            // 맞춤: 가로 u8, 세로 u8, 회전 u8, 들여쓰기 u16, 읽기 순서 u8, 줄 바꿈 u8, (0x15 양쪽 끝), 축소 u8
            0x0F => {
                align.get_or_insert_with(Default::default).horizontal = x
                    .u8()
                    .and_then(|v| H_ALIGN.get(v as usize))
                    .map(|s| s.to_string())
            }
            0x10 => {
                align.get_or_insert_with(Default::default).vertical = x
                    .u8()
                    .and_then(|v| V_ALIGN.get(v as usize))
                    .map(|s| s.to_string())
            }
            0x11 => {
                align.get_or_insert_with(Default::default).text_rotation = x.u8().map(i32::from)
            }
            0x12 => align.get_or_insert_with(Default::default).indent = x.u16().map(u32::from),
            0x13 => {
                align.get_or_insert_with(Default::default).reading_order = x.u8().map(u32::from)
            }
            0x14 => align.get_or_insert_with(Default::default).wrap_text = on(&mut x),
            0x16 => align.get_or_insert_with(Default::default).shrink_to_fit = on(&mut x),
            // 글꼴
            0x18 => {
                font.get_or_insert_with(Default::default);
            }
            0x19 => font.get_or_insert_with(Default::default).bold = x.u16().unwrap_or(400) >= 700,
            0x1A => {
                font.get_or_insert_with(Default::default).underline = match x.u16().unwrap_or(0) {
                    0 => None,
                    2 => Some("double".into()),
                    0x21 => Some("singleAccounting".into()),
                    0x22 => Some("doubleAccounting".into()),
                    _ => Some("single".into()),
                }
            }
            0x1B => {
                font.get_or_insert_with(Default::default).vert_align = match x.u16().unwrap_or(0) {
                    1 => Some("superscript".into()),
                    2 => Some("subscript".into()),
                    _ => None,
                }
            }
            0x1C => font.get_or_insert_with(Default::default).italic = on(&mut x),
            0x1D => font.get_or_insert_with(Default::default).strike = on(&mut x),
            0x1E => font.get_or_insert_with(Default::default).outline = on(&mut x),
            0x1F => font.get_or_insert_with(Default::default).shadow = on(&mut x),
            0x20 => font.get_or_insert_with(Default::default).condense = on(&mut x),
            0x21 => font.get_or_insert_with(Default::default).extend = on(&mut x),
            0x22 => font.get_or_insert_with(Default::default).charset = x.u8().map(u32::from),
            0x23 => font.get_or_insert_with(Default::default).family = x.u8().map(u32::from),
            0x24 => {
                font.get_or_insert_with(Default::default).size = x
                    .u32()
                    .map(|v| v as f64 / if legacy { 100.0 } else { 20.0 })
            }
            0x25 => {
                font.get_or_insert_with(Default::default).scheme = match x.u8() {
                    Some(1) => Some("major".into()),
                    Some(2) => Some("minor".into()),
                    _ => None,
                }
            }
            // 표시 형식: 0x26 형식 문자열 (u16 cch + UTF-16), 0x29 IFmt u16 (한셀은 쓰지 않는다 — Excel 의 .xlsb)
            0x26 => {
                fmt_code = x
                    .u16()
                    .and_then(|c| x.take(c as usize * 2))
                    .map(crate::text::utf16le)
            }
            0x29 => fmt_id = x.u16(),
            // 보호: 잠금, 숨김
            0x2B => prot.get_or_insert_with(Default::default).locked = Some(on(&mut x)),
            0x2C => prot.get_or_insert_with(Default::default).hidden = Some(on(&mut x)),
            _ => {}
        }
    }
    let num_fmt = (fmt_id.is_some() || fmt_code.is_some()).then(|| {
        let id = fmt_id.map_or(164, u32::from);
        NumFmt {
            id,
            code: fmt_code
                .or_else(|| crate::numfmt::builtin(id).map(str::to_string))
                .unwrap_or_default(),
        }
    });
    Dxf {
        font,
        fill,
        border,
        num_fmt,
        alignment: align,
        protection: prot,
        ..Default::default()
    }
}

/// 표 스타일 요소 하나 (바이너리 BrtTableStyleElement, 레거시 TableStyleElement 공통).
/// 종류 번호가 ST_TableStyleType 밖이면 버린다 (XML 에 쓸 수 있는 값이 없다)
pub(crate) fn table_style_element(t: u32, size: u32, dxf: u32) -> Option<TableStyleElement> {
    Some(TableStyleElement {
        kind: TABLE_STYLE_TYPES.get(t as usize)?.to_string(),
        size,
        dxf_id: Some(dxf),
    })
}

fn parse_styles(data: &[u8]) -> Styles {
    let mut st = Styles::default();
    let mut in_cell_xfs = false;
    let mut in_style_xfs = false;
    for rec in biff12::records(data) {
        let mut r = Rd::new(rec.data);
        match rec.rtype {
            507 => st.dxfs.push(parse_dxf(rec.data, false)),
            508 => {
                // BrtBeginTableStyles: cts u32, 기본 표 스타일 이름, 기본 피벗 스타일 이름 (XLWideString)
                let _cts = r.u32();
                st.table_styles = Some(TableStyles {
                    default_table_style: r.wstr(),
                    default_pivot_style: r.wstr(),
                    styles: Vec::new(),
                });
            }
            510 => {
                // BrtBeginTableStyle: 플래그 u16 (0x02 피벗, 0x04 표), ctse u32, 이름
                let (Some(flags), Some(_ctse), Some(name)) = (r.u16(), r.u32(), r.wstr()) else {
                    continue;
                };
                st.table_styles
                    .get_or_insert_with(Default::default)
                    .styles
                    .push(TableStyle {
                        name,
                        pivot: flags & 0x02 != 0,
                        table: flags & 0x04 != 0,
                        elements: Vec::new(),
                    });
            }
            512 => {
                // BrtTableStyleElement: 종류 u32, 줄무늬 폭 u32, 차등 서식 번호 u32
                if let (Some(t), Some(size), Some(dxf)) = (r.u32(), r.u32(), r.u32())
                    && let Some(e) = table_style_element(t, size, dxf)
                    && let Some(ts) = st.table_styles.as_mut().and_then(|x| x.styles.last_mut())
                {
                    ts.elements.push(e);
                }
            }
            617 => in_cell_xfs = true,
            618 => in_cell_xfs = false,
            626 => in_style_xfs = true,
            627 => in_style_xfs = false,
            43 => {
                let f = (|| -> Option<Font> {
                    let height = r.u16()?;
                    let grbit = r.u16()?;
                    let bls = r.u16()?;
                    let sss = r.u16()?;
                    let uls = r.u8()?;
                    let family = r.u8()?;
                    let charset = r.u8()?;
                    let _unused = r.u8()?;
                    let col = color(&mut r)?;
                    let scheme = r.u8()?;
                    let name = r.wstr()?;
                    Some(Font {
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
                        scheme: match scheme {
                            1 => Some("major".into()),
                            2 => Some("minor".into()),
                            _ => None,
                        },
                        color: col,
                        ..Default::default()
                    })
                })();
                st.fonts.push(f.unwrap_or_default());
            }
            44 => {
                if let (Some(id), Some(code)) = (r.u16(), r.wstr()) {
                    st.num_fmts.push(NumFmt {
                        id: id as u32,
                        code,
                    });
                }
            }
            45 => {
                // BrtFill: fls u32, 무늬 색, 배경색, 그라데이션 종류 u32 (0 직선, 1 경로), 각도·왼쪽·오른쪽·위·아래
                // f64, 정지점 수 u32 + 정지점마다 BrtColor + 위치 f64. fls 0x28 이면 그라데이션 (한셀 확인:
                // 그라데이션 대조 실험)
                let fill = (|| -> Option<Fill> {
                    let fls = r.u32()? as usize;
                    let fg = color(&mut r)?;
                    let bg = color(&mut r)?;
                    // [MS-XLSB] fls 가 0x28 이 아니면 그라데이션 칸들은 0 이고 무시한다 — 무늬 채우기는 여기서 끝
                    if fls != 0x28 {
                        return Some(Fill {
                            pattern_type: PATTERNS.get(fls).map(|s| s.to_string()),
                            fg_color: (fls != 0).then_some(fg).flatten(),
                            bg_color: (fls != 0).then_some(bg).flatten(),
                            gradient: None,
                        });
                    }
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
                        // 잘린 정지점은 버리고 읽은 데까지 남긴다
                        let (Some(c), Some(position)) = (color(&mut r), r.f64()) else {
                            break;
                        };
                        g.stops.push(GradientStop {
                            position,
                            color: c.unwrap_or_default(),
                        });
                    }
                    Some(Fill {
                        gradient: Some(g),
                        ..Default::default()
                    })
                })();
                st.fills.push(fill.unwrap_or_default());
            }
            46 => {
                let border = (|| -> Option<Border> {
                    let flags = r.u8()?;
                    let edge = |r: &mut Rd| -> Option<Option<BorderEdge>> {
                        let dg = r.u8()? as usize;
                        let _res = r.u8()?;
                        let c = color(r)?;
                        Some((dg != 0).then(|| BorderEdge {
                            style: BORDER_STYLES.get(dg).map(|s| s.to_string()),
                            color: c,
                        }))
                    };
                    let top = edge(&mut r)?;
                    let bottom = edge(&mut r)?;
                    let left = edge(&mut r)?;
                    let right = edge(&mut r)?;
                    let diagonal = edge(&mut r)?;
                    Some(Border {
                        left,
                        right,
                        top,
                        bottom,
                        diagonal,
                        vertical: None,
                        horizontal: None,
                        diagonal_down: flags & 0x01 != 0,
                        diagonal_up: flags & 0x02 != 0,
                    })
                })();
                st.borders.push(border.unwrap_or_default());
            }
            47 => {
                let xf = (|| -> Option<Xf> {
                    let parent = r.u16()?;
                    let fmt = r.u16()?;
                    let font = r.u16()?;
                    let fill = r.u16()?;
                    let border = r.u16()?;
                    let trot = r.u8()?;
                    let indent = r.u8()?;
                    let flags = r.u16()?;
                    let atr = r.u8()?;
                    let alc = (flags & 0x07) as usize;
                    let alcv = ((flags >> 3) & 0x07) as usize;
                    Some(Xf {
                        num_fmt_id: fmt as u32,
                        font_id: font as u32,
                        fill_id: fill as u32,
                        border_id: border as u32,
                        xf_id: (parent != 0xFFFF).then_some(parent as u32),
                        apply_number_format: Some(atr & 0x01 != 0),
                        apply_font: Some(atr & 0x02 != 0),
                        apply_alignment: Some(atr & 0x04 != 0),
                        apply_border: Some(atr & 0x08 != 0),
                        apply_fill: Some(atr & 0x10 != 0),
                        apply_protection: Some(atr & 0x20 != 0),
                        alignment: Some(Alignment {
                            horizontal: H_ALIGN.get(alc).map(|s| s.to_string()),
                            vertical: V_ALIGN.get(alcv).map(|s| s.to_string()),
                            wrap_text: flags & 0x40 != 0,
                            shrink_to_fit: flags & 0x100 != 0,
                            text_rotation: (trot != 0).then_some(trot as i32),
                            indent: (indent != 0).then_some(indent as u32),
                            reading_order: (((flags >> 10) & 0x03) != 0)
                                .then_some(((flags >> 10) & 0x03) as u32),
                        }),
                        protection: Some(Protection {
                            locked: Some(flags & 0x1000 != 0),
                            hidden: Some(flags & 0x2000 != 0),
                        }),
                        // BrtXF flags 0x8000 f123Prefix (= quotePrefix)
                        quote_prefix: flags & 0x8000 != 0,
                        hancom: None,
                    })
                })()
                .unwrap_or_default();
                if in_cell_xfs {
                    st.cell_xfs.push(xf);
                } else if in_style_xfs {
                    st.cell_style_xfs.push(xf);
                }
            }
            48 => {
                // BrtStyle: ixf u32, grbitObj1 u16 (0x01 fBuiltIn), iStyBuiltIn u8, iLevel u8 (0xFF 없음), 이름.
                // 한셀은 모든 스타일에 fBuiltIn 을 켜고, 사용자 스타일에는 앞 스타일의 내장 번호를 그대로 남긴다 (관측).
                // 그래서 같은 (번호, 수준)이 이미 나왔으면 사용자 스타일로 본다 (Excel 도 내장 번호 중복을 허용하지 않는다).
                if let (Some(ixf), Some(flags), Some(builtin), Some(lvl), Some(name)) =
                    (r.u32(), r.u16(), r.u8(), r.u8(), r.wstr())
                {
                    let i_level = (lvl != 0xFF).then_some(lvl as u32);
                    let dup = st
                        .cell_styles
                        .iter()
                        .any(|c| c.builtin_id == Some(builtin as u32) && c.i_level == i_level);
                    st.cell_styles.push(NamedStyle {
                        name,
                        xf_id: ixf,
                        builtin_id: (flags & 1 != 0 && !dup).then_some(builtin as u32),
                        i_level,
                    });
                }
            }
            _ => {}
        }
    }
    st
}

/// RichStr ([MS-XLSB] 2.5.125).
fn rich_str(r: &mut Rd, fonts: &[Font]) -> Option<RichText> {
    let flags = r.u8()?;
    let text = r.wstr()?;
    let mut rt = RichText {
        text: text.clone(),
        runs: Vec::new(),
    };
    if flags & 0x01 != 0 {
        let n = r.u32()? as usize;
        let mut marks = Vec::with_capacity(n);
        for _ in 0..n {
            marks.push((r.u16()? as usize, r.u16()? as usize));
        }
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        let mut cur: Option<usize> = None;
        for (ich, ifnt) in marks.into_iter().chain(std::iter::once((chars.len(), 0))) {
            let end = ich.min(chars.len());
            if end > start {
                rt.runs.push(TextRun {
                    text: chars[start..end].iter().collect(),
                    font: cur.and_then(|i| fonts.get(i).cloned()),
                });
            }
            start = end;
            cur = Some(ifnt);
        }
    }
    Some(rt)
}

/// CellParsedFormula / NameParsedFormula: cce u32 + rgce + cb u32 + rgcb
fn parsed_formula<'a>(r: &mut Rd<'a>) -> Option<&'a [u8]> {
    let cce = r.u32()? as usize;
    let rgce = r.take(cce)?;
    if r.left() >= 4 {
        let cb = r.u32()? as usize;
        r.take(cb);
    }
    Some(rgce)
}

struct BundleSheet {
    state: SheetState,
    rel_id: String,
    name: String,
}

/// 바이너리 통합 문서를 읽는다. `wb_part` 는 `xl/workbook.bin`.
pub fn read_workbook(pkg: &mut Package, wb_part: &str) -> Result<Workbook> {
    let mut wb = Workbook::default();
    let root_rels = pkg.relationships("")?;
    if let Some(app) = root_rels
        .iter()
        .find(|r| r.rel_type.ends_with("/extended-properties"))
        && let Some(props) = pkg.read_xml(&app.target)?
    {
        let field = |name: &str| {
            props
                .child_ns(crate::ns::EXTENDED_PROPS, name)
                .map(|e| e.text().trim().to_string())
        };
        wb.application = field("Application");
        wb.app_version = field("AppVersion");
        wb.total_time = field("TotalTime").and_then(|t| t.parse().ok());
    }
    wb.properties = crate::parser::read_core_properties(pkg, &root_rels)?;

    let data = read_part(pkg, wb_part)?;
    let rels = pkg.relationships(wb_part)?;
    let mut bundles: Vec<BundleSheet> = Vec::new();
    let mut names: Vec<(String, Option<u32>, bool, Vec<u8>)> = Vec::new();
    // None = 이 통합 문서, Some(관계 ID) = 외부 통합 문서
    let mut supbooks: Vec<Option<String>> = Vec::new();
    let mut xti: Vec<(u32, i32, i32)> = Vec::new();

    for rec in biff12::records(&data) {
        let mut r = Rd::new(rec.data);
        match rec.rtype {
            128 => {
                r.take(16);
                let app = r.wstr();
                let last = r.wstr();
                let lowest = r.wstr();
                let rup = r.wstr();
                wb.file_version = Some(FileVersion {
                    app_name: app,
                    last_edited: last,
                    lowest_edited: lowest,
                    rup_build: rup,
                });
            }
            153 => {
                // BrtWbProp: 플래그 u32 (0x01 1904 날짜 체계), dwThemeVersion u32, 코드 이름 XLWideString
                wb.date1904 = r.u32().is_some_and(|f| f & 0x01 != 0);
                let _theme = r.u32();
                wb.code_name = r.wstr().filter(|s| !s.is_empty());
            }
            158 if wb.view.is_none() => {
                // BrtBookView: xWn i32, yWn i32, dxWn u32, dyWn u32, iTabRatio u32, itabFirst u32,
                // itabCur u32, 플래그 u8 (0x08 가로 스크롤, 0x10 세로 스크롤, 0x20 시트 탭)
                let (Some(x), Some(y), Some(w), Some(h), Some(ratio), Some(first), Some(cur)) = (
                    r.u32(),
                    r.u32(),
                    r.u32(),
                    r.u32(),
                    r.u32(),
                    r.u32(),
                    r.u32(),
                ) else {
                    continue;
                };
                let f = r.u8().unwrap_or(0x38);
                wb.view = Some(crate::model::WorkbookView {
                    x_window: x as i32,
                    y_window: y as i32,
                    window_width: w,
                    window_height: h,
                    tab_ratio: ratio,
                    first_sheet: first,
                    active_tab: cur,
                    show_horizontal_scroll: f & 0x08 != 0,
                    show_vertical_scroll: f & 0x10 != 0,
                    show_sheet_tabs: f & 0x20 != 0,
                });
            }
            534 => {
                // BrtBookProtection: protpwdBook u16, protpwdRev u16, 플래그 u16 (1 구조, 2 창, 4 변경 내용)
                let (Some(pw), Some(_rev), Some(f)) = (r.u16(), r.u16(), r.u16()) else {
                    continue;
                };
                wb.protection = Some(WorkbookProtection {
                    lock_structure: f & 0x01 != 0,
                    lock_windows: f & 0x02 != 0,
                    password: (pw != 0).then(|| format!("{pw:04X}")),
                });
            }
            156 => {
                let (Some(hs), Some(_tab), Some(rel), Some(name)) =
                    (r.u32(), r.u32(), r.nwstr(), r.wstr())
                else {
                    continue;
                };
                bundles.push(BundleSheet {
                    state: match hs {
                        1 => SheetState::Hidden,
                        2 => SheetState::VeryHidden,
                        _ => SheetState::Visible,
                    },
                    rel_id: rel.unwrap_or_default(),
                    name,
                });
            }
            // BrtSupSelf / BrtSupSame: 이 통합 문서, BrtSupBookSrc: 외부 통합 문서
            357 | 358 => supbooks.push(None),
            355 => supbooks.push(Some(r.wstr().unwrap_or_default())),
            362 => {
                let n = r.u32().unwrap_or(0);
                for _ in 0..n {
                    let (Some(b), Some(f), Some(l)) = (r.u32(), r.u32(), r.u32()) else {
                        break;
                    };
                    xti.push((b, f as i32, l as i32));
                }
            }
            39 => {
                let flags = r.u32().unwrap_or(0);
                let _key = r.u8();
                let itab = r.u32().unwrap_or(0xFFFF_FFFF);
                let mut name = r.wstr().unwrap_or_default();
                // fBuiltin: Print_Area 등 내장 이름은 XML 처럼 _xlnm. 을 붙인다
                if flags & 0x20 != 0 && !name.starts_with("_xlnm.") {
                    name = format!("_xlnm.{name}");
                }
                let rgce = parsed_formula(&mut r).unwrap_or_default().to_vec();
                names.push((
                    name,
                    (itab != 0xFFFF_FFFF).then_some(itab),
                    flags & 0x01 != 0,
                    rgce,
                ));
            }
            _ => {}
        }
    }

    // 테마 / 스타일 / 공유 문자열
    if let Some(th) = rels.iter().find(|r| r.rel_type.ends_with("/theme"))
        && let Some(e) = pkg.read_xml(&th.target)?
    {
        let mut theme = crate::theme::parse_theme(&e);
        theme.source = pkg.read_bytes(&th.target)?;
        wb.theme = Some(theme);
    }
    if let Some(st) = rels.iter().find(|r| r.rel_type.ends_with("/styles")) {
        wb.styles = parse_styles(&read_part(pkg, &st.target)?);
    }
    if let Some(sst) = rels.iter().find(|r| r.rel_type.ends_with("/sharedStrings")) {
        let data = read_part(pkg, &sst.target)?;
        for rec in biff12::records(&data) {
            if rec.rtype == 19 {
                let mut r = Rd::new(rec.data);
                wb.shared_strings
                    .push(rich_str(&mut r, &wb.styles.fonts).unwrap_or_default());
            }
        }
    }

    // 외부 연결 (externalLinkN.bin: BrtBeginSupBook 의 관계 ID → 경로, BrtSupTabs → 시트 이름)
    let mut book_index: Vec<Option<usize>> = Vec::new(); // SupBook 번호 → 외부 연결 번호 (1부터)
    for sb in &supbooks {
        let Some(rid) = sb else {
            book_index.push(None);
            continue;
        };
        let mut link = ExternalLink::default();
        if let Some(rel) = rels.iter().find(|r| &r.id == rid) {
            let data = read_part(pkg, &rel.target)?;
            let link_rels = pkg.relationships(&rel.target)?;
            for rec in biff12::records(&data) {
                let mut r = Rd::new(rec.data);
                match rec.rtype {
                    360 => {
                        if r.u16() == Some(0)
                            && let Some(Some(id)) = r.nwstr()
                            && let Some(t) = link_rels.iter().find(|x| x.id == id)
                        {
                            link.target = t.target.clone();
                        }
                    }
                    359 => {
                        let n = r.u32().unwrap_or(0);
                        for _ in 0..n {
                            match r.wstr() {
                                Some(s) => link.sheet_names.push(s),
                                None => break,
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        wb.external_links.push(link);
        book_index.push(Some(wb.external_links.len()));
    }

    let sheet_names: Vec<String> = bundles.iter().map(|b| b.name.clone()).collect();
    let xti_sheets: Vec<Option<String>> = xti
        .iter()
        .map(|&(book, first, last)| {
            if first < 0 {
                return None;
            }
            let (book, names) = match *book_index.get(book as usize)? {
                None => (None, &sheet_names),
                Some(n) => {
                    let link = &wb.external_links[n - 1];
                    (Some((n, link.file_name())), &link.sheet_names)
                }
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
    let name_list: Vec<String> = names.iter().map(|n| n.0.clone()).collect();
    wb.defined_names = names
        .iter()
        .filter(|n| !n.3.is_empty() && !n.0.starts_with("_xlfn."))
        .map(|(name, local, hidden, rgce)| {
            let ctx = Context {
                xti_sheets: &xti_sheets,
                names: &name_list,
                xti_names: &[],
                base: CellRef::default(),
                xlsb: true,
                row_shift: 0,
            };
            let formula = match formula::decode(rgce, &ctx) {
                Ok(Decoded::Text(t)) => t,
                Ok(Decoded::SharedRef(_)) => String::new(),
                Err(e) => format!("#ERR({})", e.0),
            };
            DefinedName {
                name: name.clone(),
                local_sheet_id: *local,
                hidden: *hidden,
                formula,
            }
        })
        .collect();

    for b in bundles {
        let Some(rel) = rels.iter().find(|r| r.id == b.rel_id).cloned() else {
            continue;
        };
        let mut sheet = Sheet {
            name: b.name,
            sheet_id: wb.sheets.len() as u32 + 1,
            state: b.state,
            kind: if rel.rel_type.ends_with("/worksheet") {
                SheetKind::Worksheet
            } else if rel.rel_type.ends_with("/chartsheet") {
                SheetKind::Chartsheet
            } else {
                SheetKind::Other(rel.rel_type.clone())
            },
            part: rel.target.clone(),
            show_grid_lines: true,
            ..Default::default()
        };
        if sheet.kind == SheetKind::Worksheet {
            let data = read_part(pkg, &rel.target)?;
            let srels = pkg.relationships(&rel.target)?;
            read_sheet(&data, &mut sheet, &wb, &srels, (&xti_sheets, &name_list))?;
            read_comments(pkg, &srels, &wb.styles.fonts, &mut sheet)?;
            crate::parser::read_sheet_drawings(pkg, &srels, &mut wb.raw_parts, &mut sheet)?;
        }
        wb.sheets.push(sheet);
    }
    Ok(wb)
}

/// 자동 필터에서 지금 읽고 있는 열
fn cur_filter_column(sheet: &mut Sheet) -> Option<&mut FilterColumn> {
    sheet.auto_filter.as_mut()?.columns.last_mut()
}

fn read_sheet(
    data: &[u8],
    sheet: &mut Sheet,
    wb: &Workbook,
    rels: &[Relationship],
    (xti_sheets, names): (&Vec<Option<String>>, &Vec<String>),
) -> Result<()> {
    let rel_by_id: HashMap<&str, &Relationship> = rels.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut row: u32 = 0;
    // 페이지 나누기 목록 안: 1 = 행, 2 = 열
    let mut brk_kind = 0u8;
    // BrtSel 이 활성 창 것과 맞았는지
    let mut sel_matched = false;
    // 공유 수식: (범위, 토큰열). XLSB 의 PtgExp 는 기준 행만 가리키므로 범위로 찾는다.
    let mut shared: Vec<(CellRange, Vec<u8>)> = Vec::new();
    let find_shared = |shared: &[(CellRange, Vec<u8>)], master_row: u32, at: CellRef| {
        shared
            .iter()
            .rev()
            .find(|(rg, _)| {
                rg.start.row == master_row
                    && (rg.start.row..=rg.end.row).contains(&at.row)
                    && (rg.start.col..=rg.end.col).contains(&at.col)
            })
            .map(|(_, t)| t.clone())
    };
    // 조건부 서식 수식·값의 토큰열 → 글 (기준 셀 = 범위의 왼쪽 위)
    let decode_at = |t: &[u8], base: CellRef| -> Option<String> {
        let ctx = Context {
            xti_sheets,
            names,
            xti_names: &[],
            base,
            xlsb: true,
            row_shift: 0,
        };
        match formula::decode(t, &ctx) {
            Ok(Decoded::Text(s)) => Some(s),
            _ => None,
        }
    };
    // x14 조건부 서식: 본 규칙의 GUID(BrtCFRuleExt) → (서식, 규칙) 위치, x14 구역의 규칙들과 지금 범위
    let mut cf_guids: HashMap<[u8; 16], (usize, usize)> = HashMap::new();
    let mut x14_rules: Vec<X14Rule> = Vec::new();
    let mut x14_sqref = String::new();
    let recs: Vec<Record> = biff12::records(data);
    for rec in &recs {
        let mut r = Rd::new(rec.data);
        match rec.rtype {
            1146 => {
                // BrtCFRuleExt: FRTHeader + GUID — 바로 앞 본 규칙을 x14 구역의 같은 GUID 규칙과 잇는다
                if frt_header(&mut r).is_some()
                    && let Some(g) = r.take(16)
                    && let Some(i) = sheet.conditional_formats.len().checked_sub(1)
                    && let Some(j) = sheet.conditional_formats[i].rules.len().checked_sub(1)
                {
                    let mut guid = [0u8; 16];
                    guid.copy_from_slice(g);
                    cf_guids.insert(guid, (i, j));
                }
            }
            1046 => {
                // BrtBeginConditionalFormatting14: FRTHeader (범위), ccf u32, fPivot u32
                if let Some((Some(sq), _)) = frt_header(&mut r) {
                    x14_sqref = sq;
                }
            }
            1048 => {
                // BrtBeginCFRule14: FRTHeader, iType, iTemplate, dxfId, iPri (본 규칙과 이어지면 -1), iParam,
                // 예약 ×2 (u32), 플래그 u16, cbFmla ×3 (u32), strParam, GUID
                if frt_header(&mut r).is_none() {
                    continue;
                }
                let v: Vec<u32> = (0..7).map(|_| r.u32().unwrap_or(0)).collect();
                r.take(2 + 12);
                let _ = r.nwstr();
                let mut x = X14Rule {
                    sqref: x14_sqref.clone(),
                    kind: v[0],
                    priority: v[3],
                    ..Default::default()
                };
                if let Some(g) = r.take(16) {
                    x.guid.copy_from_slice(g);
                }
                x14_rules.push(x);
            }
            1051 => {
                // BrtBeginDatabar14: FRTHeader, 길이 최소 u8, 최대 u8, 값 표시 u8, 방향 u8 (0 context,
                // 1 leftToRight, 2 rightToLeft), 축 u8 (0 automatic, 1 middle, 2 none), 플래그 u8 (0x01 테두리,
                // 0x02 그라데이션, 0x04 음수 색 다름, 0x08 음수 테두리 색 다름) — x14 조건부 서식 실험
                if let (Some(_), Some(x)) = (frt_header(&mut r), x14_rules.last_mut())
                    && let Some(b) = r.take(6)
                {
                    let mut v = [0u8; 6];
                    v.copy_from_slice(b);
                    x.bar = Some(v);
                }
            }
            1052 => {
                // BrtBeginIconSet14: FRTHeader, 집합 i32, 플래그 u16 (0x01 사용자 지정, 0x02 아이콘만, 0x04 거꾸로)
                if let (Some(_), Some(x)) = (frt_header(&mut r), x14_rules.last_mut())
                    && let (Some(set), Some(f)) = (r.u32(), r.u16())
                {
                    x.icons = Some((set as i32, f));
                }
            }
            1050 => {
                // BrtCFVO14: FRTHeader (값의 수식), iType u32, numParam f64, fSaveGTE u32, fGTE u32, cce u32.
                // 한셀은 xm:f 로 받은 값을 수식(PtgInt 등)으로 쓴다
                let base = sqref_base(&x14_sqref);
                let Some((_, fmls)) = frt_header(&mut r) else {
                    continue;
                };
                let (Some(ty), Some(num), Some(save_gte), Some(gte)) =
                    (r.u32(), r.f64(), r.u32(), r.u32())
                else {
                    continue;
                };
                let val = fmls
                    .first()
                    .and_then(|t| decode_at(t, base))
                    .or_else(|| matches!(ty, 1 | 4 | 5).then(|| formula::number_text(num)));
                if let Some(x) = x14_rules.last_mut() {
                    x.cfvos.push(Cfvo {
                        kind: cfvo_kind(ty).into(),
                        val,
                        gte: save_gte == 0 || gte != 0,
                    });
                }
            }
            1055 => {
                // BrtColor14: FRTHeader + BrtColor
                if let (Some(_), Some(x)) = (frt_header(&mut r), x14_rules.last_mut())
                    && let Some(c) = r.take(8).and_then(x14_color)
                {
                    x.colors.push(c);
                }
            }
            1112 => {
                // BrtCFIcon: FRTHeader, 집합 i32 (-1 = 아이콘 없음), 번호 u32
                if let (Some(_), Some(x)) = (frt_header(&mut r), x14_rules.last_mut())
                    && let (Some(set), Some(id)) = (r.u32(), r.u32())
                {
                    x.custom_icons.push((icon_set_name(set as i32), id));
                }
            }
            148 => {
                if let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                    (r.u32(), r.u32(), r.u32(), r.u32())
                {
                    sheet.dimension = Some(
                        CellRange {
                            start: CellRef { row: r1, col: c1 },
                            end: CellRef { row: r2, col: c2 },
                        }
                        .to_string(),
                    );
                }
            }
            147 => {
                // BrtWsProp: 플래그 3바이트 (0x000001 자동 페이지 나누기, 0x000040 요약 행 아래,
                // 0x000080 요약 열 오른쪽, 0x000100 페이지에 맞추기, 0x010000 필터 적용 중),
                // brtcolorTab, rwSync, colSync, strName(코드 이름)
                let (Some(a), Some(b), Some(c)) = (r.u8(), r.u8(), r.u8()) else {
                    continue;
                };
                sheet.page.auto_page_breaks = Some(a & 0x01 != 0);
                sheet.summary_above = a & 0x40 == 0;
                sheet.summary_left = a & 0x80 == 0;
                sheet.page.fit_to_page = Some(b & 0x01 != 0);
                sheet.filter_mode = c & 0x01 != 0;
                sheet.tab_color = color(&mut r).flatten().filter(|c| !c.auto); // 자동 = 탭 색 없음
                r.take(8);
                sheet.code_name = r.wstr().filter(|s| !s.is_empty());
            }
            137 => {
                // BrtBeginWsView (한셀 30바이트): flags u16, xlView u32, rwTop u32, colLeft u32,
                // icvHdr u8, 예약 u8, ? u16, 현재 확대율 u16 (18), 기본 u16 (20), 페이지 나누기 미리 보기 u16 (22),
                // 페이지 레이아웃 u16 (24), iWbkView u32 — [MS-XLSB] 순서. 한셀의 XML 과는 일관되지 않다 (문서 참고)
                let Some(flags) = r.u16() else { continue };
                sheet.show_grid_lines = flags & 0x0004 != 0;
                let v = &mut sheet.view;
                v.show_formulas = flags & 0x0002 != 0;
                v.show_row_col_headers = flags & 0x0008 != 0;
                v.show_zeros = flags & 0x0010 != 0;
                v.tab_selected = flags & 0x0040 != 0;
                let xl_view = r.u32().unwrap_or(0);
                v.view = match xl_view {
                    1 => Some("pageBreakPreview".into()),
                    2 => Some("pageLayout".into()),
                    _ => None,
                };
                if let (Some(row), Some(col)) = (r.u32(), r.u32()) {
                    v.top_left_cell = Some(CellRef { row, col }.to_string());
                }
                r.take(4);
                let z = |x: Option<u16>| x.filter(|&z| z != 0).map(u32::from);
                v.zoom_scale = z(r.u16());
                v.zoom_scale_normal = z(r.u16());
                v.zoom_scale_sheet_layout_view = z(r.u16());
            }
            151 => {
                // BrtPane: xnumXSplit f64, xnumYSplit f64, rwTop u32, colLeft u32, pnnAct u32, flags u8
                let (Some(x), Some(y), Some(row), Some(col), Some(pnn), Some(f)) =
                    (r.f64(), r.f64(), r.u32(), r.u32(), r.u32(), r.u8())
                else {
                    continue;
                };
                sheet.view.pane = Some(Pane {
                    x_split: x,
                    y_split: y,
                    top_left_cell: Some(CellRef { row, col }.to_string()),
                    active_pane: pane_name(pnn),
                    state: Some(
                        match (f & 0x01 != 0, f & 0x02 != 0) {
                            (true, true) => "frozen",
                            (true, false) => "frozenSplit",
                            _ => "split",
                        }
                        .into(),
                    ),
                });
            }
            152 => {
                // BrtSel: pnn u32, rwAct u32, colAct u32, dwRfxAct u32, crfx u32 + RfX × crfx
                let (Some(pnn), Some(row), Some(col), Some(_i), Some(n)) =
                    (r.u32(), r.u32(), r.u32(), r.u32(), r.u32())
                else {
                    continue;
                };
                let active = sheet.view.pane.as_ref().and_then(|p| p.active_pane.clone());
                // 활성 창 것을 쓰고, 맞는 것이 없으면 마지막 것 (한셀은 나눈 창의 번호를 잃기도 한다)
                let hit = pane_name(pnn) == active || (active.is_none() && pnn == 3);
                if !hit && sel_matched {
                    continue;
                }
                sel_matched |= hit;
                sheet.view.active_cell = Some(CellRef { row, col }.to_string());
                let mut refs = Vec::new();
                for _ in 0..n {
                    let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                        (r.u32(), r.u32(), r.u32(), r.u32())
                    else {
                        break;
                    };
                    refs.push(
                        CellRange {
                            start: CellRef { row: r1, col: c1 },
                            end: CellRef { row: r2, col: c2 },
                        }
                        .to_string(),
                    );
                }
                sheet.view.sqref = Some(refs.join(" "));
            }
            461 => {
                // BrtBeginConditionalFormatting: ccf u32, fPivot u32, UncheckedSqRfX
                r.take(8);
                sheet.conditional_formats.push(ConditionalFormat {
                    sqref: sqrfx(&mut r),
                    rules: Vec::new(),
                });
            }
            463 => {
                // BrtBeginCFRule: iType, iTemplate, dxfId, iPri, iParam, 예약 ×2 (u32), flags u16,
                // cbFmla1..3 u32, strParam, 수식 1–3 (CFParsedFormula)
                let Some(cf) = sheet.conditional_formats.last_mut() else {
                    continue;
                };
                let base = CellRange::parse(cf.sqref.split(' ').next().unwrap_or(""))
                    .map(|r| r.start)
                    .unwrap_or_default();
                let v: Vec<u32> = (0..7).map(|_| r.u32().unwrap_or(0)).collect();
                let flags = r.u16().unwrap_or(0);
                let cb: Vec<u32> = (0..3).map(|_| r.u32().unwrap_or(0)).collect();
                let text = r.nwstr().flatten();
                let mut formulas = Vec::new();
                for &n in &cb {
                    if n == 0 {
                        continue;
                    }
                    let Some(f) = r.take(n as usize) else { break };
                    let mut fr = Rd::new(f);
                    if let Some(rgce) = parsed_formula(&mut fr) {
                        let ctx = Context {
                            xti_sheets,
                            names,
                            xti_names: &[],
                            base,
                            xlsb: true,
                            row_shift: 0,
                        };
                        formulas.push(match formula::decode(rgce, &ctx) {
                            Ok(Decoded::Text(t)) => t,
                            _ => String::new(),
                        });
                    }
                }
                cf.rules
                    .push(cf_rule(v[0], v[1], v[2], v[3], v[4], flags, text, formulas));
            }
            469 | 467 | 465 => {
                // 색 눈금 / 데이터 막대 (bLenMin u8, bLenMax u8, fShowValue u8) / 아이콘 (iSet u32, flags u16)
                let Some(rule) = sheet
                    .conditional_formats
                    .last_mut()
                    .and_then(|c| c.rules.last_mut())
                else {
                    continue;
                };
                match rec.rtype {
                    469 => rule.color_scale = Some(Default::default()),
                    467 => {
                        let (min, max, show) = (r.u8(), r.u8(), r.u8());
                        rule.data_bar = Some(DataBar {
                            min_length: min.map(u32::from).filter(|&v| v != 10),
                            max_length: max.map(u32::from).filter(|&v| v != 90),
                            show_value: show != Some(0),
                            ..Default::default()
                        });
                    }
                    _ => {
                        let set = r.u32().unwrap_or(0);
                        let f = r.u16().unwrap_or(0);
                        rule.icon_set = Some(IconSet {
                            set: ICON_SETS
                                .get(set as usize)
                                .unwrap_or(&"3TrafficLights1")
                                .to_string(),
                            cfvos: Vec::new(),
                            show_value: f & 0x02 == 0,
                            reverse: f & 0x04 != 0,
                            percent: true,
                            custom_icons: Vec::new(),
                        });
                    }
                }
            }
            471 | 564 => {
                // BrtCFVO (iType u32, numParam f64, fSaveGTE u32, fGTE u32, cbFmla u32 + 수식) / BrtColor
                let base = sheet
                    .conditional_formats
                    .last()
                    .map(|c| sqref_base(&c.sqref))
                    .unwrap_or_default();
                let Some(rule) = sheet
                    .conditional_formats
                    .last_mut()
                    .and_then(|c| c.rules.last_mut())
                else {
                    continue;
                };
                if rec.rtype == 471 {
                    let ty = r.u32().unwrap_or(0);
                    let num = r.f64().unwrap_or(0.0);
                    let (save_gte, gte) = (r.u32().unwrap_or(0), r.u32().unwrap_or(1));
                    // cbFmla u32 뒤에 CFParsedFormula — 값이 수식이면(x14 의 xm:f 에서 온 값 포함) 그 글이 값이다
                    let fml = (r.u32().unwrap_or(0) > 0)
                        .then(|| parsed_formula(&mut r))
                        .flatten()
                        .and_then(|t| decode_at(t, base));
                    let cfvo = Cfvo {
                        kind: cfvo_kind(ty).into(),
                        val: fml
                            .or_else(|| matches!(ty, 1 | 4 | 5).then(|| formula::number_text(num))),
                        gte: save_gte == 0 || gte != 0,
                    };
                    if let Some(c) = rule.color_scale.as_mut() {
                        c.cfvos.push(cfvo);
                    } else if let Some(d) = rule.data_bar.as_mut() {
                        d.cfvos.push(cfvo);
                    } else if let Some(i) = rule.icon_set.as_mut() {
                        i.cfvos.push(cfvo);
                    }
                } else {
                    let c = valid_rgb_color(&mut r);
                    if let Some(s) = rule.color_scale.as_mut() {
                        s.colors.extend(c);
                    } else if let Some(d) = rule.data_bar.as_mut() {
                        d.color = c;
                    }
                }
            }
            64 => {
                // BrtDVal: 플래그 u32, UncheckedSqRfX, 제목·내용 문자열 4개, 수식 2개 (DValFormula)
                let Some(flags) = r.u32() else { continue };
                let sqref = sqrfx(&mut r);
                let base = CellRange::parse(sqref.split(' ').next().unwrap_or(""))
                    .map(|r| r.start)
                    .unwrap_or_default();
                let mut s: Vec<Option<String>> = (0..4).map(|_| r.nwstr().flatten()).collect();
                let mut fml = || {
                    let rgce = parsed_formula(&mut r)?;
                    if rgce.is_empty() {
                        return None;
                    }
                    let ctx = Context {
                        xti_sheets,
                        names,
                        xti_names: &[],
                        base,
                        xlsb: true,
                        row_shift: 0,
                    };
                    match formula::decode(rgce, &ctx) {
                        Ok(Decoded::Text(t)) => Some(t),
                        _ => None,
                    }
                };
                let (f1, f2) = (fml(), fml());
                sheet
                    .data_validations
                    .push(dv_from_flags(flags, sqref, &mut s, f1, f2));
            }
            // 페이지 나누기: BrtBeginRwBrks(392)/BrtBeginColBrks(394) 안의 BrtBrk(396)
            392 => brk_kind = 1,
            394 => brk_kind = 2,
            393 | 395 => brk_kind = 0,
            396 => {
                // BrtBrk: id u32, min u32, max u32, fMan u32, fPivot u32
                if let Some(id) = r.u32() {
                    match brk_kind {
                        1 => sheet.page.row_breaks.push(id),
                        2 => sheet.page.col_breaks.push(id),
                        _ => {}
                    }
                }
            }
            476 => {
                // BrtMargins: f64 × 6
                let mut v = [0f64; 6];
                for x in v.iter_mut() {
                    *x = r.f64().unwrap_or(0.0);
                }
                sheet.page.margins = Some(PageMargins {
                    left: v[0],
                    right: v[1],
                    top: v[2],
                    bottom: v[3],
                    header: v[4],
                    footer: v[5],
                });
            }
            477 => {
                // BrtPrintOptions: 0x01 가로 가운데, 0x02 세로 가운데, 0x04 행·열 머리글, 0x08 눈금선
                let f = r.u16().unwrap_or(0);
                sheet.page.print_options = Some(PrintOptions {
                    horizontal_centered: f & 0x01 != 0,
                    vertical_centered: f & 0x02 != 0,
                    headings: f & 0x04 != 0,
                    grid_lines: f & 0x08 != 0,
                });
            }
            478 => {
                // BrtPageSetup: paper, scale, res, vres, copies, pageStart, fitW, fitH (u32), flags u16
                let mut v = [0u32; 8];
                for x in v.iter_mut() {
                    *x = r.u32().unwrap_or(0);
                }
                let g = r.u16().unwrap_or(0);
                sheet.page.setup = Some(PageSetup {
                    paper_size: Some(v[0]),
                    scale: Some(v[1]),
                    horizontal_dpi: Some(v[2]),
                    vertical_dpi: Some(v[3]),
                    copies: Some(v[4]),
                    first_page_number: Some(v[5]),
                    fit_to_width: Some(v[6]),
                    fit_to_height: Some(v[7]),
                    orientation: Some(
                        if g & 0x0040 != 0 {
                            "default"
                        } else if g & 0x0002 != 0 {
                            "landscape"
                        } else {
                            "portrait"
                        }
                        .into(),
                    ),
                    page_order: (g & 0x0001 != 0).then(|| "overThenDown".into()),
                    use_printer_defaults: g & 0x0004 == 0,
                    black_and_white: g & 0x0008 != 0,
                    draft: g & 0x0010 != 0,
                    use_first_page_number: g & 0x0080 != 0,
                });
            }
            479 => {
                // BrtBeginHeaderFooter: flags u16 + 머리글/바닥글 6개 (XLNullableWideString)
                let f = r.u16().unwrap_or(0);
                let mut t: Vec<Option<String>> = (0..6)
                    .map(|_| r.nwstr().flatten().filter(|s| !s.is_empty()))
                    .collect();
                sheet.page.header_footer = Some(HeaderFooter {
                    different_odd_even: f & 0x01 != 0,
                    different_first: f & 0x02 != 0,
                    scale_with_doc: f & 0x04 != 0,
                    align_with_margins: f & 0x08 != 0,
                    odd_header: t[0].take(),
                    odd_footer: t[1].take(),
                    even_header: t[2].take(),
                    even_footer: t[3].take(),
                    first_header: t[4].take(),
                    first_footer: t[5].take(),
                });
            }
            535 => {
                // BrtSheetProtection: protpwd u16, 이어서 u32 16개 — fLocked(시트 보호) 다음 15개는
                // "허용" 여부라 OOXML 속성(참 = 막힘)과 반대다
                let pw = r.u16().unwrap_or(0);
                let mut states = [false; 16];
                for (i, s) in states.iter_mut().enumerate() {
                    let v = r.u32().unwrap_or(0) != 0;
                    *s = if i == 0 { v } else { !v };
                }
                sheet.protection = Some(SheetProtection::from_states(
                    (pw != 0).then(|| format!("{pw:04X}")),
                    states,
                ));
            }
            161 => {
                // BrtBeginAFilter: RfX (rwFirst, rwLast, colFirst, colLast)
                if let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                    (r.u32(), r.u32(), r.u32(), r.u32())
                {
                    sheet.auto_filter = Some(AutoFilter {
                        range: CellRange {
                            start: CellRef { row: r1, col: c1 },
                            end: CellRef { row: r2, col: c2 },
                        }
                        .to_string(),
                        columns: Vec::new(),
                    });
                }
            }
            163 => {
                // BrtBeginFilterColumn: dwCol u32, 플래그 u16 (단추 숨김 등)
                if let (Some(af), Some(col)) = (sheet.auto_filter.as_mut(), r.u32()) {
                    af.columns.push(FilterColumn {
                        col_id: col,
                        ..Default::default()
                    });
                }
            }
            165 => {
                // BrtBeginFilters: fBlank u32, 달력 종류 u32
                if let Some(c) = cur_filter_column(sheet) {
                    c.blank = r.u32().is_some_and(|v| v != 0);
                }
            }
            167 => {
                if let (Some(v), Some(c)) = (r.wstr(), cur_filter_column(sheet)) {
                    c.values.push(v);
                }
            }
            172 => {
                if let Some(c) = cur_filter_column(sheet) {
                    c.custom_and = r.u32().is_some_and(|v| v != 0);
                }
            }
            174 => {
                // BrtCustomFilter: vts u8 (4 수, 6 문자열, 8 논리/오류, 0x0C 빈 칸, 0x0E 빈 칸 아님),
                // grbitSign u8, 값 8바이트 (문자열이면 이어서 XLWideString)
                let (Some(vts), Some(sign)) = (r.u8(), r.u8()) else {
                    continue;
                };
                let op = FILTER_OPERATORS.get(sign as usize).copied().unwrap_or("");
                let op = if op.is_empty() { "equal" } else { op };
                let val = match vts {
                    4 => r.f64().map(formula::number_text),
                    6 => {
                        r.take(8);
                        r.wstr()
                    }
                    8 => r.u8().map(|b| {
                        if b != 0 {
                            "TRUE".into()
                        } else {
                            "FALSE".into()
                        }
                    }),
                    0x0C => Some(String::new()),
                    0x0E => Some(String::new()),
                    _ => None,
                };
                if let (Some(v), Some(c)) = (val, cur_filter_column(sheet)) {
                    c.custom.push((op.to_string(), v));
                }
            }
            170 => {
                // BrtTop10Filter: 플래그 u8 (1 위, 2 백분율, 4 기준값 있음), 개수/백분율 f64, 기준값 f64
                if let (Some(f), Some(v), Some(c)) = (r.u8(), r.f64(), cur_filter_column(sheet)) {
                    c.top10 = Some((f & 0x01 != 0, f & 0x02 != 0, v));
                    c.filter_val = r.f64().filter(|_| f & 0x04 != 0);
                }
            }
            171 => {
                // BrtDynamicFilter: cft u32, 플래그 u8, 값 f64, 최댓값 f64. 한셀은 플래그 바이트를 채우지 않고 쓰므로
                // (관측: 같은 종류에 0x03·0x63·0xCC) 값은 플래그와 상관없이 0 이 아니면 있는 것으로 본다
                if let (Some(t), Some(c)) = (r.u32(), cur_filter_column(sheet)) {
                    c.dynamic = DYNAMIC_FILTER_TYPES
                        .get(t as usize)
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());
                    let _flags = r.u8();
                    c.dynamic_val = r.f64().filter(|&v| v != 0.0);
                    c.dynamic_max_val = r.f64().filter(|&v| v != 0.0);
                }
            }
            485 => {
                if let (Some(dx_gcol), Some(_dx_col), Some(dy)) = (r.u32(), r.u16(), r.u16()) {
                    sheet.default_col_width = Some(dx_gcol as f64 / 256.0);
                    sheet.default_row_height = Some(dy as f64 / 20.0);
                }
            }
            60 => {
                let (Some(c1), Some(c2), Some(w), Some(ixfe), Some(flags)) =
                    (r.u32(), r.u32(), r.u32(), r.u32(), r.u16())
                else {
                    continue;
                };
                sheet.cols.push(ColInfo {
                    min: c1 + 1,
                    max: c2 + 1,
                    width: Some(w as f64 / 256.0),
                    style: Some(ixfe),
                    hidden: flags & 0x0001 != 0,
                    custom_width: flags & 0x0002 != 0,
                    // BrtColInfo 플래그: 비트 8–10 개요 수준, 0x1000 접힘 ([MS-XLSB]).
                    // 한셀은 접힘을 0x2000 에 쓴다 — 둘 다 받는다.
                    outline_level: ((flags >> 8) & 0x07) as u8,
                    collapsed: flags & 0x3000 != 0,
                });
            }
            0 => {
                let (Some(rw), Some(ixfe), Some(h), Some(flags)) =
                    (r.u32(), r.u32(), r.u16(), r.u16())
                else {
                    continue;
                };
                row = rw;
                let custom = flags & 0x2000 != 0;
                let hidden = flags & 0x1000 != 0;
                let has_style = flags & 0x4000 != 0;
                // BrtRowHdr 플래그: 비트 8–10 개요 수준, 0x0800 접힘
                let level = ((flags >> 8) & 0x07) as u8;
                let collapsed = flags & 0x0800 != 0;
                // 높이는 지정(fUnsynced)이 아니어도 그 행의 실제 높이다 — 기본과 다르면 남긴다
                // (한셀 XML 의 ht. 버리면 한셀이 다시 계산해 그리기 앵커가 어긋난다)
                let default_twips = sheet.default_row_height.map(|d| (d * 20.0).round() as u16);
                let differs = h != 0 && Some(h) != default_twips;
                if custom || hidden || has_style || level > 0 || collapsed || differs {
                    sheet.rows.push(RowInfo {
                        index: rw + 1,
                        height: (custom || differs).then_some(h as f64 / 20.0),
                        custom_height: custom,
                        hidden,
                        style: has_style.then_some(ixfe),
                        outline_level: level,
                        collapsed,
                    });
                }
            }
            1..=7 | 62 => {
                let Some((col, style)) = cell_head(&mut r) else {
                    continue;
                };
                let value = match rec.rtype {
                    1 => CellValue::Empty,
                    2 => r
                        .u32()
                        .map(|v| CellValue::Number(rk_value(v)))
                        .unwrap_or_default(),
                    3 => r
                        .u8()
                        .map(|e| CellValue::Error(formula::error_text(e).into()))
                        .unwrap_or_default(),
                    4 => r.u8().map(|b| CellValue::Bool(b != 0)).unwrap_or_default(),
                    5 => r.f64().map(CellValue::Number).unwrap_or_default(),
                    6 => r.wstr().map(CellValue::Text).unwrap_or_default(),
                    7 => r
                        .u32()
                        .and_then(|i| wb.shared_strings.get(i as usize))
                        .map(|s| CellValue::Text(s.text.clone()))
                        .unwrap_or_default(),
                    _ => rich_str(&mut r, &wb.styles.fonts)
                        .map(|s| CellValue::Text(s.text))
                        .unwrap_or_default(),
                };
                sheet.cells.push(Cell {
                    r: CellRef { row, col },
                    style,
                    value,
                    formula: None,
                });
            }
            8..=11 => {
                let Some((col, style)) = cell_head(&mut r) else {
                    continue;
                };
                let value = match rec.rtype {
                    8 => r.wstr().map(CellValue::Text).unwrap_or_default(),
                    9 => r.f64().map(CellValue::Number).unwrap_or_default(),
                    10 => r.u8().map(|b| CellValue::Bool(b != 0)).unwrap_or_default(),
                    _ => r
                        .u8()
                        .map(|e| CellValue::Error(formula::error_text(e).into()))
                        .unwrap_or_default(),
                };
                let _grbit = r.u16();
                let at = CellRef { row, col };
                let rgce = parsed_formula(&mut r).unwrap_or_default();
                let ctx = Context {
                    xti_sheets,
                    names,
                    xti_names: &[],
                    base: at,
                    xlsb: true,
                    row_shift: 0,
                };
                let text = match formula::decode(rgce, &ctx) {
                    Ok(Decoded::Text(t)) => t,
                    Ok(Decoded::SharedRef(m)) => find_shared(&shared, m.row, at)
                        .and_then(|tok| match formula::decode(&tok, &ctx) {
                            Ok(Decoded::Text(t)) => Some(t),
                            _ => None,
                        })
                        .unwrap_or_default(),
                    Err(e) => format!("#ERR({})", e.0),
                };
                sheet.cells.push(Cell {
                    r: at,
                    style,
                    value,
                    formula: Some(Formula {
                        text,
                        ..Default::default()
                    }),
                });
            }
            427 => {
                // BrtShrFmla: RfX + formula
                if let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                    (r.u32(), r.u32(), r.u32(), r.u32())
                    && let Some(tokens) = parsed_formula(&mut r)
                {
                    // 기준 셀(직전 수식 셀)은 이 레코드보다 먼저 나오므로 다시 해석한다
                    if let Some(cell) = sheet.cells.last_mut()
                        && cell.r == (CellRef { row: r1, col: c1 })
                        && let Some(f) = cell.formula.as_mut()
                        && f.text.is_empty()
                    {
                        let ctx = Context {
                            xti_sheets,
                            names,
                            xti_names: &[],
                            base: cell.r,
                            xlsb: true,
                            row_shift: 0,
                        };
                        if let Ok(Decoded::Text(t)) = formula::decode(tokens, &ctx) {
                            f.text = t;
                        }
                    }
                    shared.push((
                        CellRange {
                            start: CellRef { row: r1, col: c1 },
                            end: CellRef { row: r2, col: c2 },
                        },
                        tokens.to_vec(),
                    ));
                }
            }
            176 => {
                if let (Some(r1), Some(r2), Some(c1), Some(c2)) =
                    (r.u32(), r.u32(), r.u32(), r.u32())
                {
                    sheet.merges.push(CellRange {
                        start: CellRef { row: r1, col: c1 },
                        end: CellRef { row: r2, col: c2 },
                    });
                }
            }
            494 => {
                let h = (|| -> Option<Hyperlink> {
                    let (r1, r2, c1, c2) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                    let rel = r.nwstr()?;
                    let location = r.wstr()?;
                    let tooltip = r.wstr()?;
                    let display = r.wstr()?;
                    Some(Hyperlink {
                        range: CellRange {
                            start: CellRef { row: r1, col: c1 },
                            end: CellRef { row: r2, col: c2 },
                        }
                        .to_string(),
                        location: (!location.is_empty()).then_some(location),
                        target: rel
                            .and_then(|id| rel_by_id.get(id.as_str()).map(|r| r.target.clone())),
                        tooltip: (!tooltip.is_empty()).then_some(tooltip),
                        display: (!display.is_empty()).then_some(display),
                    })
                })();
                if let Some(h) = h {
                    sheet.hyperlinks.push(h);
                }
            }
            _ => {}
        }
    }
    let _ = bad;
    merge_x14_rules(sheet, x14_rules, &cf_guids);
    sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
    Ok(())
}

/// 범위 목록의 첫 범위의 왼쪽 위 (조건부 서식 수식의 기준 셀)
fn sqref_base(sqref: &str) -> CellRef {
    CellRange::parse(sqref.split(' ').next().unwrap_or(""))
        .map(|r| r.start)
        .unwrap_or_default()
}

/// BrtCFVO / BrtCFVO14 의 iType: 1 num, 2 min, 3 max, 4 percent, 5 percentile, 7 formula, 8 autoMin,
/// 9 autoMax (x14 조건부 서식 실험. 6 은 관측되지 않았다)
fn cfvo_kind(ty: u32) -> &'static str {
    match ty {
        2 => "min",
        3 => "max",
        4 => "percent",
        5 => "percentile",
        6 | 7 => "formula",
        8 => "autoMin",
        9 => "autoMax",
        _ => "num",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// XFProp 하나: 종류 u16, 크기 u16 (머리 4바이트 포함), 값
    fn prop(ty: u16, v: &[u8]) -> Vec<u8> {
        let mut b = ty.to_le_bytes().to_vec();
        b.extend_from_slice(&((v.len() + 4) as u16).to_le_bytes());
        b.extend_from_slice(v);
        b
    }

    /// BrtDXF: 플래그 u16, 예약 u16, cprops u16, XFProp 들
    fn dxf(props: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![0, 0, 0, 0];
        b.extend_from_slice(&(props.len() as u16).to_le_bytes());
        for p in props {
            b.extend_from_slice(p);
        }
        b
    }

    #[test]
    fn dxf_props_follow_xlsb_numbers() {
        let d = parse_dxf(
            &dxf(&[
                // 밑줄 이중, 기울임, 취소선, 위 첨자, 14 pt (1/20 pt)
                prop(0x1A, &[2, 0]),
                prop(0x1C, &[1]),
                prop(0x1D, &[1]),
                prop(0x1B, &[1, 0]),
                prop(0x24, &280u32.to_le_bytes()),
                // 안쪽 세로 선: 점선, 파랑 / 왼쪽: 선 없음, 자동 색(한셀 표지 FF FF FF DF)
                prop(0x0B, &[5, 0, 0, 0, 0, 0, 0xFF, 0xFF, 4, 0]),
                prop(0x08, &[1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF, 0, 0]),
            ]),
            false,
        );
        let f = d.font.unwrap();
        assert_eq!(f.underline.as_deref(), Some("double"));
        assert!(f.italic && f.strike);
        assert_eq!(f.vert_align.as_deref(), Some("superscript"));
        assert_eq!(f.size, Some(14.0));
        let b = d.border.unwrap();
        let v = b.vertical.unwrap();
        assert_eq!(v.style.as_deref(), Some("dotted"));
        assert_eq!(v.color.unwrap().rgb.as_deref(), Some("ff0000ff"));
        let l = b.left.unwrap();
        assert_eq!(l.style, None);
        assert!(l.color.unwrap().auto);
        assert!(b.top.is_none());
        // 레거시 DXF 는 크기가 1/100 pt
        let d = parse_dxf(&dxf(&[prop(0x24, &1400u32.to_le_bytes())]), true);
        assert_eq!(d.font.unwrap().size, Some(14.0));
    }

    #[test]
    fn dxf_gradient_props() {
        let mut g = 1u32.to_le_bytes().to_vec(); // 경로
        for v in [0.0f64, 0.2, 0.8, 0.1, 0.9] {
            g.extend_from_slice(&v.to_le_bytes());
        }
        let mut stop = vec![0, 0];
        stop.extend_from_slice(&1.0f64.to_le_bytes());
        stop.extend_from_slice(&[5, 0, 0, 0, 0x44, 0x72, 0xC4, 0xFF]);
        let d = parse_dxf(
            &dxf(&[prop(0x00, &[0x28]), prop(0x03, &g), prop(0x04, &stop)]),
            false,
        );
        let g = d.fill.unwrap().gradient.unwrap();
        assert!(g.path);
        assert_eq!((g.left, g.right, g.top, g.bottom), (0.2, 0.8, 0.1, 0.9));
        assert_eq!(g.stops.len(), 1);
        assert_eq!(g.stops[0].position, 1.0);
        assert_eq!(g.stops[0].color.rgb.as_deref(), Some("ff4472c4"));
    }

    #[test]
    fn table_style_element_names() {
        assert_eq!(
            table_style_element(5, 2, 20).unwrap().kind,
            "firstRowStripe"
        );
        assert_eq!(
            table_style_element(26, 1, 18).unwrap().kind,
            "pageFieldLabels"
        );
        assert!(table_style_element(99, 1, 0).is_none());
    }

    /// BIFF12 레코드: 종류·길이 7비트 가변 길이 + 내용
    fn rec(t: u32, data: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        for (mut v, max) in [(t, 2), (data.len() as u32, 4)] {
            for _ in 0..max {
                let byte = (v & 0x7F) as u8;
                v >>= 7;
                if v == 0 {
                    b.push(byte);
                    break;
                }
                b.push(byte | 0x80);
            }
        }
        b.extend_from_slice(data);
        b
    }

    fn le(v: &[u32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// x14 조건부 서식 실험에서 한셀이 쓴 바이트 그대로 (데이터 막대 A1:A12, 사용자 지정 아이콘 D1:D12)
    #[test]
    fn x14_conditional_formats() {
        let guid = |n: u8| {
            let mut g = vec![0u8; 16];
            g[7] = 0x40;
            g[8] = 0x80;
            g[15] = n;
            g
        };
        let sq = |c: u32| le(&[1, 0, 11, c, c]);
        let mut s = Vec::new();
        // 본 규칙: BrtBeginConditionalFormatting, BrtBeginCFRule (데이터 막대, 우선순위 1), BrtBeginDatabar (10/90),
        // BrtCFVO 최소·최대, BrtColor, BrtEndDatabar, BrtCFRuleExt(GUID 1)
        s.extend(rec(461, &[le(&[1, 0]), sq(0)].concat()));
        let mut cf = le(&[4, 3, u32::MAX, 1, 0, 0, 0]);
        cf.extend_from_slice(&[0, 0]);
        cf.extend(le(&[0, 0, 0, u32::MAX]));
        s.extend(rec(463, &cf));
        s.extend(rec(467, &[10, 90, 1]));
        s.extend(rec(471, &[le(&[2]), vec![0; 20]].concat()));
        s.extend(rec(471, &[le(&[3]), vec![0; 20]].concat()));
        s.extend(rec(564, &[5, 0x63, 0, 0, 0x63, 0x8e, 0xc6, 0xff]));
        s.extend(rec(468, &[]));
        s.extend(rec(1146, &[vec![0; 4], guid(1)].concat()));
        s.extend(rec(464, &[]));
        s.extend(rec(462, &[]));
        // x14 구역: 범위, 규칙(GUID 1), BrtBeginDatabar14 (5/95, 오른쪽에서 왼쪽, 축 가운데, 테두리·음수 색 다름),
        // autoMin·autoMax, 색 넷
        let frt_sq = |c: u32| [le(&[2, 1, 2]), sq(c), le(&[1, 0])].concat();
        let rule14 = |kind: u32, tmpl: u32, pri: u32, n: u8| {
            let mut b = le(&[0, kind, tmpl, u32::MAX, pri, 0, 0, 0]);
            b.extend_from_slice(&[0, 0]);
            b.extend(le(&[0, 0, 0, 0]));
            b.extend(guid(n));
            b.extend(le(&[1]));
            b
        };
        s.extend(rec(1046, &frt_sq(0)));
        s.extend(rec(1048, &rule14(4, 3, u32::MAX, 1)));
        s.extend(rec(1051, &[0, 0, 0, 0, 5, 95, 1, 2, 1, 0x0d, 0]));
        for t in [8, 9] {
            s.extend(rec(1050, &[le(&[0, t]), vec![0; 20]].concat()));
        }
        for c in [
            [5, 0x1f, 0, 0, 0x1f, 0x4e, 0x79, 0xff],
            [5, 0xff, 0, 0, 0xff, 0, 0, 0xff],
            [5, 0x70, 0, 0, 0x70, 0x30, 0xa0, 0xff],
            [5, 0x40, 0, 0, 0, 0, 0, 0xff],
        ] {
            s.extend(rec(1055, &[vec![0; 4], c.to_vec()].concat()));
        }
        s.extend(rec(1156, &[]));
        s.extend(rec(1049, &[]));
        s.extend(rec(1047, &[]));
        // x14 구역에만 있는 아이콘 집합 (3Arrows, 사용자 지정, 우선순위 5): 값은 수식 (PtgInt)
        s.extend(rec(1046, &frt_sq(3)));
        s.extend(rec(1048, &rule14(6, 4, 5, 5)));
        s.extend(rec(1052, &[le(&[0, 0]), vec![1, 0]].concat()));
        for (v, ty, gte) in [(0u8, 4u32, 1u32), (0, 1, 0), (67, 4, 1)] {
            let mut b = le(&[4, 1, 2, 3, 0]);
            b.extend_from_slice(&[0x1e, v, 0]);
            b.extend(le(&[ty]));
            b.extend_from_slice(&[0; 8]);
            b.extend(le(&[1, gte, 3]));
            s.extend(rec(1050, &b));
        }
        for (set, id) in [(2u32, 0u32), (u32::MAX, 0), (17, 2)] {
            s.extend(rec(1112, &le(&[0, set, id])));
        }
        s.extend(rec(1155, &[]));
        s.extend(rec(1049, &[]));
        s.extend(rec(1047, &[]));

        let mut sheet = Sheet::default();
        let wb = Workbook::default();
        read_sheet(&s, &mut sheet, &wb, &[], (&Vec::new(), &Vec::new())).unwrap();
        let bar_rule = &sheet.conditional_formats[0].rules[0];
        let bar = bar_rule.data_bar.as_ref().unwrap();
        assert_eq!((bar.min_length, bar.max_length), (Some(5), Some(95)));
        let e = bar.ext.as_ref().unwrap();
        assert_eq!(e.cfvos[0].kind, "autoMin");
        assert_eq!(e.cfvos[1].kind, "autoMax");
        assert!(e.border && !e.gradient && !e.negative_same_as_positive);
        assert!(!e.negative_border_same_as_positive);
        assert_eq!(e.direction.as_deref(), Some("rightToLeft"));
        assert_eq!(e.axis_position.as_deref(), Some("middle"));
        assert_eq!(
            e.border_color.as_ref().unwrap().rgb.as_deref(),
            Some("ff1f4e79")
        );
        assert_eq!(e.axis_color.as_ref().unwrap().indexed, Some(64));
        let icons = &sheet.conditional_formats[1];
        assert_eq!(icons.sqref, "D1:D12");
        let r = &icons.rules[0];
        assert!(r.x14);
        assert_eq!(r.priority, 5);
        let i = r.icon_set.as_ref().unwrap();
        assert_eq!(i.set, "3Arrows");
        let vals: Vec<_> = i.cfvos.iter().map(|c| (c.val.clone(), c.gte)).collect();
        assert_eq!(
            vals,
            [
                (Some("0".into()), true),
                (Some("0".into()), false),
                (Some("67".into()), true)
            ]
        );
        assert_eq!(
            i.custom_icons,
            [
                ("3Flags".into(), 0),
                ("NoIcons".into(), 0),
                ("3Stars".into(), 2)
            ]
        );
    }

    /// 색 조합 실험: 종류 없이 쓰인 색을 플래그로 자리에 놓는다
    #[test]
    fn data_bar_color_slots() {
        let c = |v: u8| Color {
            rgb: Some(format!("ff{v:02x}{v:02x}{v:02x}")),
            ..Default::default()
        };
        let slots = |e: &crate::model::DataBarExt| {
            [
                &e.border_color,
                &e.negative_fill_color,
                &e.negative_border_color,
                &e.axis_color,
            ]
            .map(|x| x.is_some())
        };
        // 테두리 켬·음수 색 다름·음수 테두리 같음·축 자동 → 셋은 테두리·음수 채우기·축
        let mut e = crate::model::DataBarExt {
            border: true,
            ..Default::default()
        };
        assign_bar_colors(vec![c(1), c(2), c(3)], &mut e);
        assert_eq!(slots(&e), [true, true, false, true]);
        // 테두리 없음 → 둘은 음수 채우기·축 (한셀 COM 이 만드는 막대)
        let mut e = crate::model::DataBarExt {
            negative_border_same_as_positive: false,
            ..Default::default()
        };
        assign_bar_colors(vec![c(1), c(2)], &mut e);
        assert_eq!(slots(&e), [false, true, false, true]);
        // 테두리 켬인데 둘뿐 → 테두리를 먼저 뺀다
        let mut e = crate::model::DataBarExt {
            border: true,
            ..Default::default()
        };
        assign_bar_colors(vec![c(1), c(2)], &mut e);
        assert_eq!(slots(&e), [false, true, false, true]);
        let mut e = crate::model::DataBarExt::default();
        assign_bar_colors(vec![c(1), c(2), c(3), c(4)], &mut e);
        assert_eq!(slots(&e), [true; 4]);
    }
}
