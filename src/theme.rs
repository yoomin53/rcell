// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 테마 색과 색 해석 (테마 번호·명암(tint)·팔레트 → RGB).

use serde::Serialize;

use crate::model::Color;
use crate::xml::Element;

const DRAWINGML: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// 테마 색 12개. SpreadsheetML 의 `theme` 번호 순서
/// (0 lt1, 1 dk1, 2 lt2, 3 dk2, 4–9 accent1–6, 10 hlink, 11 folHlink).
#[derive(Debug, Clone, Serialize)]
pub struct Theme {
    pub colors: [String; 12],
    pub major_font: Option<String>,
    pub minor_font: Option<String>,
    /// 원본 테마 파트 (저장할 때 그대로 쓴다). 색·글꼴 밖의 내용(효과, 채우기 목록 …)을 잃지 않도록.
    #[serde(skip)]
    pub source: Option<Vec<u8>>,
}

impl Default for Theme {
    /// Office 기본 테마.
    fn default() -> Self {
        let c = [
            "FFFFFF", "000000", "EEECE1", "1F497D", "4F81BD", "C0504D", "9BBB59", "8064A2",
            "4BACC6", "F79646", "0000FF", "800080",
        ];
        Theme {
            colors: c.map(str::to_string),
            major_font: None,
            minor_font: None,
            source: None,
        }
    }
}

fn scheme_color(e: &Element) -> Option<String> {
    for c in e.elements() {
        if c.ns.as_deref() != Some(DRAWINGML) {
            continue;
        }
        match c.name.as_str() {
            "srgbClr" => return c.attr("val").map(|v| v.to_ascii_uppercase()),
            "sysClr" => {
                return c
                    .attr("lastClr")
                    .map(|v| v.to_ascii_uppercase())
                    .or_else(|| match c.attr("val") {
                        Some("windowText") => Some("000000".into()),
                        Some("window") => Some("FFFFFF".into()),
                        _ => None,
                    });
            }
            _ => {}
        }
    }
    None
}

/// `xl/theme/theme1.xml` 을 해석한다.
pub fn parse_theme(root: &Element) -> Theme {
    let mut theme = Theme::default();
    let mut found = None;
    root.walk(&mut |e| {
        if found.is_none() && e.is(DRAWINGML, "clrScheme") {
            found = Some(e);
        }
    });
    if let Some(scheme) = found {
        // 파일 순서(dk1 lt1 dk2 lt2 …)를 번호 순서(lt1 dk1 lt2 dk2 …)로
        let order = [
            ("lt1", 0),
            ("dk1", 1),
            ("lt2", 2),
            ("dk2", 3),
            ("accent1", 4),
            ("accent2", 5),
            ("accent3", 6),
            ("accent4", 7),
            ("accent5", 8),
            ("accent6", 9),
            ("hlink", 10),
            ("folHlink", 11),
        ];
        for (name, idx) in order {
            if let Some(v) = scheme.child_ns(DRAWINGML, name).and_then(scheme_color) {
                theme.colors[idx] = v;
            }
        }
    }
    let mut fonts = (None, None);
    root.walk(&mut |e| {
        if e.is(DRAWINGML, "majorFont") || e.is(DRAWINGML, "minorFont") {
            let pick = e
                .elements()
                .find(|c| c.name == "font" && c.attr("script") == Some("Hang"))
                .and_then(|c| c.attr("typeface"))
                .or_else(|| {
                    e.child_ns(DRAWINGML, "latin")
                        .and_then(|l| l.attr("typeface"))
                })
                .map(str::to_string);
            if e.name == "majorFont" {
                fonts.0 = pick;
            } else {
                fonts.1 = pick;
            }
        }
    });
    theme.major_font = fonts.0;
    theme.minor_font = fonts.1;
    theme
}

/// BIFF8 / SpreadsheetML 기본 색 번호 팔레트 (0–63).
pub const INDEXED: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000,
    0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000,
    0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC,
    0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF,
    0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600,
    0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
    0x333333,
];

fn hex_to_rgb(h: &str) -> Option<(u8, u8, u8)> {
    let h = h.trim_start_matches('#');
    let h = if h.len() == 8 { &h[2..] } else { h };
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// 명암(tint) 적용 — ECMA-376 18.8.19 (HLS 240 단계).
fn apply_tint((r, g, b): (u8, u8, u8), tint: f64) -> (u8, u8, u8) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let mut l = (max + min) / 2.0;
    let d = max - min;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } / 6.0;
        (h, s)
    };
    l = if tint < 0.0 {
        l * (1.0 + tint)
    } else {
        l * (1.0 - tint) + tint
    };
    let hue = |p: f64, q: f64, mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue(p, q, h + 1.0 / 3.0),
            hue(p, q, h),
            hue(p, q, h - 1.0 / 3.0),
        )
    };
    let c = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    (c(r), c(g), c(b))
}

/// 색을 `#RRGGBB` 로. 자동(auto)/알 수 없으면 `None`.
pub fn resolve(c: &Color, theme: &Theme) -> Option<String> {
    if c.auto {
        return None;
    }
    let base = if let Some(rgb) = &c.rgb {
        hex_to_rgb(rgb)?
    } else if let Some(t) = c.theme {
        hex_to_rgb(theme.colors.get(t as usize)?)?
    } else {
        let v = *INDEXED.get(c.indexed? as usize)?;
        ((v >> 16) as u8, (v >> 8) as u8, v as u8)
    };
    let (r, g, b) = match c.tint {
        Some(t) if t != 0.0 => apply_tint(base, t),
        _ => base,
    };
    Some(format!("#{r:02X}{g:02X}{b:02X}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_rgb_theme_and_tint() {
        let t = Theme::default();
        let rgb = Color {
            rgb: Some("ff6182d6".into()),
            ..Default::default()
        };
        assert_eq!(resolve(&rgb, &t).as_deref(), Some("#6182D6"));
        let accent1 = Color {
            theme: Some(4),
            ..Default::default()
        };
        assert_eq!(resolve(&accent1, &t).as_deref(), Some("#4F81BD"));
        let lighter = Color {
            theme: Some(1),
            tint: Some(0.5),
            ..Default::default()
        };
        assert_eq!(resolve(&lighter, &t).as_deref(), Some("#808080"));
    }
}
