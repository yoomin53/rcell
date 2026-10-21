// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 차등 서식 표 0x088D 의 내용 쓰기 — `binary::reader::parse_dxf` 의 반대.
//!
//! FRT 머리 뒤에 `u32 (3)`, `cprops u16`, XFProp(`종류 u16, 크기 u16 (머리 포함), 값`)을 종류 번호 순서로
//! 쓴다 (관측: 차등 서식 대조 실험). 한셀의 다른 점: 글꼴 크기(0x24)는 1/100 pt, 채우기·글꼴 색의 XFPropColor 는
//! `05 00 00 00` + RGBA, 테두리 색은 `05 ff 00 00` + RGBA, 자동 색은 RGB 값 `FF FF FF DF`. 표시 형식과 맞춤은
//! 이 표에 쓰지 않고(CF 의 DXFN 에만 있다) 그라데이션은 무늬 0x28 만 남긴다. 보호는 0x2C 값 0 하나로 쓴다.

use super::reader::{BORDER_STYLES, PATTERNS};
use super::writer::{Buf, rgb_of};
use crate::model::{BorderEdge, Color, Dxf, Workbook};

/// XFPropColor (8바이트): RGB 종류(0x05 = 유효 + 종류 2), 색 번호, 밝기 0, RGBA (자동 = FF FF FF DF)
pub(crate) fn prop_color(b: &mut Buf, c: Option<&Color>, wb: &Workbook, icv: u8) {
    b.u8(0x05).u8(icv).u16(0);
    match c {
        Some(c) if c.auto || c.indexed == Some(64) => {
            b.bytes(&[0xFF, 0xFF, 0xFF, 0xDF]);
        }
        Some(c) => {
            let rgb = rgb_of(c, wb).unwrap_or(0);
            b.u8((rgb >> 16) as u8)
                .u8((rgb >> 8) as u8)
                .u8(rgb as u8)
                .u8(0);
        }
        None => {
            b.bytes(&[0xFF, 0xFF, 0xFF, 0xDF]);
        }
    }
}

fn prop(out: &mut Vec<(u16, Vec<u8>)>, ty: u16, v: Vec<u8>) {
    out.push((ty, v));
}

/// 0x088D 의 FRT 머리 뒤 내용
pub(crate) fn dxf_props(d: &Dxf, wb: &Workbook) -> Vec<u8> {
    let mut props: Vec<(u16, Vec<u8>)> = Vec::new();
    if let Some(f) = &d.fill {
        let pattern = if f.gradient.is_some() {
            Some(0x28u8)
        } else {
            f.pattern_type
                .as_deref()
                .and_then(|p| PATTERNS.iter().position(|x| *x == p))
                .map(|p| p as u8)
        };
        if let Some(p) = pattern {
            prop(&mut props, 0x00, vec![p]);
        }
        if f.gradient.is_none() {
            if let Some(c) = &f.fg_color {
                let mut b = Buf::new();
                prop_color(&mut b, Some(c), wb, 0);
                prop(&mut props, 0x01, b.take());
            }
            if let Some(c) = &f.bg_color {
                let mut b = Buf::new();
                prop_color(&mut b, Some(c), wb, 0);
                prop(&mut props, 0x02, b.take());
            }
        }
    }
    if let Some(f) = &d.font
        && let Some(c) = &f.color
    {
        let mut b = Buf::new();
        prop_color(&mut b, Some(c), wb, 0);
        prop(&mut props, 0x05, b.take());
    }
    if let Some(bd) = &d.border {
        let edge = |e: &BorderEdge| -> Vec<u8> {
            let mut b = Buf::new();
            prop_color(&mut b, e.color.as_ref(), wb, 0xFF);
            let dg = e
                .style
                .as_deref()
                .and_then(|s| BORDER_STYLES.iter().position(|x| *x == s))
                .unwrap_or(0) as u16;
            b.u16(dg);
            b.take()
        };
        for (ty, e) in [
            (0x06u16, &bd.top),
            (0x07, &bd.bottom),
            (0x08, &bd.left),
            (0x09, &bd.right),
            (0x0A, &bd.diagonal),
            (0x0B, &bd.vertical),
            (0x0C, &bd.horizontal),
        ] {
            if let Some(e) = e {
                prop(&mut props, ty, edge(e));
            }
        }
        if bd.diagonal_up {
            prop(&mut props, 0x0D, vec![1]);
        }
        if bd.diagonal_down {
            prop(&mut props, 0x0E, vec![1]);
        }
    }
    if let Some(f) = &d.font {
        let mut any = false;
        if f.bold {
            prop(&mut props, 0x19, 700u16.to_le_bytes().to_vec());
            any = true;
        }
        if let Some(u) = &f.underline {
            let v: u16 = match u.as_str() {
                "double" => 2,
                "singleAccounting" => 0x21,
                "doubleAccounting" => 0x22,
                "none" => 0,
                _ => 1,
            };
            prop(&mut props, 0x1A, v.to_le_bytes().to_vec());
            any = true;
        }
        if let Some(v) = &f.vert_align {
            let x: u16 = match v.as_str() {
                "superscript" => 1,
                "subscript" => 2,
                _ => 0,
            };
            prop(&mut props, 0x1B, x.to_le_bytes().to_vec());
            any = true;
        }
        for (on, ty) in [
            (f.italic, 0x1Cu16),
            (f.strike, 0x1D),
            (f.outline, 0x1E),
            (f.shadow, 0x1F),
            (f.condense, 0x20),
            (f.extend, 0x21),
        ] {
            if on {
                prop(&mut props, ty, vec![1]);
                any = true;
            }
        }
        if let Some(s) = f.size {
            prop(
                &mut props,
                0x24,
                ((s * 100.0).round() as u32).to_le_bytes().to_vec(),
            );
            any = true;
        }
        if f.color.is_some() {
            any = true;
        }
        if !any || f.name.is_some() {
            // 글꼴이 있다는 표시만 남는 차등 서식: 글꼴 이름(0x18, `cch u16` + UTF-16)
            let name = f.name.as_deref().unwrap_or("");
            let mut b = Buf::new();
            b.u16(super::writer::units(name) as u16).utf16(name);
            prop(&mut props, 0x18, b.take());
        }
    }
    if d.protection.is_some() {
        prop(&mut props, 0x2C, vec![0]);
    }
    // 한셀의 순서: 채우기(0x00–0x04), 글꼴(색 0x05, 0x18–0x25), 테두리(0x06–0x0E), 맞춤, 표시 형식, 보호 (관측)
    let group = |t: u16| match t {
        0x00..=0x04 => 0,
        0x05 | 0x18..=0x25 => 1,
        0x06..=0x0E => 2,
        0x0F..=0x17 => 3,
        0x26..=0x2A => 4,
        _ => 5,
    };
    props.sort_by_key(|p| (group(p.0), p.0));
    let mut b = Buf::new();
    b.u32(3).u16(props.len() as u16);
    for (ty, v) in props {
        b.u16(ty).u16((v.len() + 4) as u16).bytes(&v);
    }
    b.take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Border, Fill, Font};

    fn hex(b: &[u8]) -> String {
        b.iter()
            .map(|x| format!("{x:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn matches_hancell_dxf_props() {
        let wb = Workbook::default();
        let italic = Dxf {
            font: Some(Font {
                italic: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            hex(&dxf_props(&italic, &wb)),
            "03 00 00 00 01 00 1c 00 05 00 01"
        );
        let fill = Dxf {
            fill: Some(Fill {
                pattern_type: Some("darkGrid".into()),
                fg_color: Some(Color {
                    rgb: Some("ff00ff00".into()),
                    ..Default::default()
                }),
                bg_color: Some(Color {
                    rgb: Some("ffffb689".into()),
                    ..Default::default()
                }),
                gradient: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            hex(&dxf_props(&fill, &wb)),
            "03 00 00 00 03 00 00 00 05 00 09 01 00 0c 00 05 00 00 00 00 ff 00 00 02 00 0c 00 05 00 00 00 ff b6 89 00"
        );
        let border = Dxf {
            border: Some(Border {
                left: Some(BorderEdge {
                    style: Some("thin".into()),
                    color: Some(Color {
                        rgb: Some("ffff0000".into()),
                        ..Default::default()
                    }),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            hex(&dxf_props(&border, &wb)),
            "03 00 00 00 01 00 08 00 0e 00 05 ff 00 00 ff 00 00 00 01 00"
        );
    }
}
