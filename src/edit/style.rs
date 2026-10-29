// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 셀 서식 바꾸기: 칸의 서식(Xf)을 복사해 바꾼 뒤, 같은 서식이 이미 있으면 그 번호를 쓰고 없으면 새로 더한다.
//! 글꼴·채우기·테두리·표시 형식도 같은 것이 있으면 다시 쓴다.

use std::collections::HashMap;

use serde::Deserialize;

use crate::model::{Alignment, Border, BorderEdge, Color, Fill, Font, NumFmt, Styles, Xf};

/// 서식 바꾸기 — 값이 있는 항목만 바꾼다
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct FormatPatch {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub font_name: Option<String>,
    /// 글자 크기 (pt)
    pub font_size: Option<f64>,
    /// 글자 색 `RRGGBB` (빈 글 = 자동)
    pub font_color: Option<String>,
    /// 채우기 색 `RRGGBB` (빈 글 = 채우기 없음)
    pub fill: Option<String>,
    /// `general`/`left`/`center`/`right`/`justify`/`distributed`/`centerContinuous`/`fill`
    pub h_align: Option<String>,
    /// `top`/`center`/`bottom`/`justify`/`distributed`
    pub v_align: Option<String>,
    pub wrap: Option<bool>,
    /// 표시 형식 코드
    pub num_fmt: Option<String>,
}

impl FormatPatch {
    pub fn is_empty(&self) -> bool {
        *self == FormatPatch::default()
    }
}

/// 테두리 적용 방식
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BorderPreset {
    /// 모든 칸의 네 변
    All,
    /// 범위 바깥 테두리
    Outline,
    /// 범위 안쪽 선
    Inside,
    /// 테두리 없음 (대각선 포함)
    None,
    Left,
    Top,
    Right,
    Bottom,
}

/// `RRGGBB` → 색 (불투명 ARGB)
pub fn rgb(hex: &str) -> Option<Color> {
    let h = hex.trim().trim_start_matches('#');
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| Color {
        rgb: Some(format!("FF{}", h.to_ascii_uppercase())),
        ..Default::default()
    })
}

/// 표시 형식 코드 → 번호: 한셀 내장 번호 → 문서의 사용자 형식 → 새 번호 (164 이상)
pub fn num_fmt_id(styles: &mut Styles, code: &str) -> u32 {
    if code.eq_ignore_ascii_case("general") {
        return 0;
    }
    if let Some(id) = (0..200u32).find(|&i| {
        crate::numfmt::builtin(i) == Some(code) && !styles.num_fmts.iter().any(|n| n.id == i)
    }) {
        return id;
    }
    if let Some(n) = styles.num_fmts.iter().find(|n| n.code == code) {
        return n.id;
    }
    let id = styles
        .num_fmts
        .iter()
        .map(|n| n.id + 1)
        .max()
        .unwrap_or(0)
        .max(164);
    styles.num_fmts.push(NumFmt {
        id,
        code: code.to_string(),
    });
    id
}

fn find_or_push<T: std::fmt::Debug>(list: &mut Vec<T>, item: T) -> u32 {
    let key = format!("{item:?}");
    if let Some(i) = list.iter().position(|x| format!("{x:?}") == key) {
        return i as u32;
    }
    list.push(item);
    (list.len() - 1) as u32
}

/// 셀 서식 하나를 더한다 (같은 것이 있으면 그 번호)
pub fn intern_xf(styles: &mut Styles, xf: Xf) -> u32 {
    find_or_push(&mut styles.cell_xfs, xf)
}

/// 글꼴 하나를 더한다 (같은 것이 있으면 그 번호)
pub fn intern_font(styles: &mut Styles, f: crate::model::Font) -> u32 {
    find_or_push(&mut styles.fonts, f)
}

/// 테두리 하나를 더한다 (같은 것이 있으면 그 번호)
pub fn intern_border(styles: &mut Styles, b: Border) -> u32 {
    find_or_push(&mut styles.borders, b)
}

/// 바꾼 서식 번호를 기억해 같은 서식을 여러 칸에 줄 때 한 번만 계산한다
#[derive(Default)]
pub struct Restyler {
    done: HashMap<u32, u32>,
}

impl Restyler {
    /// `old` 서식에 `f` 를 적용한 서식 번호
    pub fn apply(
        &mut self,
        styles: &mut Styles,
        old: u32,
        f: &dyn Fn(&mut Styles, &mut Xf),
    ) -> u32 {
        if let Some(&n) = self.done.get(&old) {
            return n;
        }
        let mut xf = styles.xf(old).cloned().unwrap_or_default();
        f(styles, &mut xf);
        let n = intern_xf(styles, xf);
        self.done.insert(old, n);
        n
    }
}

/// 서식 바꾸기를 Xf 에 적용한다
pub fn patch_xf(styles: &mut Styles, xf: &mut Xf, p: &FormatPatch) {
    let font_change = p.bold.is_some()
        || p.italic.is_some()
        || p.underline.is_some()
        || p.strike.is_some()
        || p.font_name.is_some()
        || p.font_size.is_some()
        || p.font_color.is_some();
    if font_change {
        let mut font: Font = styles
            .fonts
            .get(xf.font_id as usize)
            .cloned()
            .unwrap_or_default();
        if let Some(b) = p.bold {
            font.bold = b;
        }
        if let Some(i) = p.italic {
            font.italic = i;
        }
        if let Some(u) = p.underline {
            font.underline = u.then(|| "single".to_string());
        }
        if let Some(s) = p.strike {
            font.strike = s;
        }
        if let Some(n) = &p.font_name {
            font.name = Some(n.clone());
        }
        if let Some(s) = p.font_size {
            font.size = Some(s);
        }
        if let Some(c) = &p.font_color {
            font.color = rgb(c).or(Some(Color {
                auto: true,
                ..Default::default()
            }));
        }
        xf.font_id = find_or_push(&mut styles.fonts, font);
        xf.apply_font = Some(true);
    }
    if let Some(c) = &p.fill {
        let cur = styles.fills.get(xf.fill_id as usize).cloned();
        let patterned = cur.as_ref().is_some_and(|f| {
            f.gradient.is_none()
                && !matches!(
                    f.pattern_type.as_deref(),
                    None | Some("none") | Some("solid")
                )
        });
        let fill = match rgb(c) {
            // 무늬 채우기는 무늬를 두고 바탕색만 바꾼다 (한셀 Interior.Color — 채우기 대조 실험)
            Some(color) if patterned => Fill {
                bg_color: Some(color),
                ..cur.unwrap_or_default()
            },
            Some(color) => Fill {
                pattern_type: Some("solid".into()),
                fg_color: Some(color),
                bg_color: Some(Color {
                    indexed: Some(64),
                    ..Default::default()
                }),
                gradient: None,
            },
            None => Fill {
                pattern_type: Some("none".into()),
                ..Default::default()
            },
        };
        xf.fill_id = find_or_push(&mut styles.fills, fill);
        xf.apply_fill = Some(true);
    }
    if p.h_align.is_some() || p.v_align.is_some() || p.wrap.is_some() {
        let a = xf.alignment.get_or_insert_with(Alignment::default);
        if let Some(h) = &p.h_align {
            a.horizontal = Some(h.clone());
        }
        if let Some(v) = &p.v_align {
            a.vertical = Some(v.clone());
        }
        if let Some(w) = p.wrap {
            a.wrap_text = w;
        }
        xf.apply_alignment = Some(true);
    }
    if let Some(code) = &p.num_fmt {
        xf.num_fmt_id = num_fmt_id(styles, code);
        xf.apply_number_format = Some(true);
    }
}

/// 선 굵기·모양 이름 (`thin`, `medium`, `thick`, `dashed`, `dotted`, `double`, `hair` …) → 변
/// 색을 주지 않으면 `color` 가 None — 적용할 때 그 변의 원래 색(선이 없어도 색이 있으면)을 쓰고, 없으면 자동 색.
/// 한셀은 문서에서 읽은 기본 테두리의 색(왼쪽·오른쪽·아래 검정, 위 자동)을 그대로 둔다 (테두리 대조 실험)
pub fn edge(style: &str, color: Option<&str>) -> BorderEdge {
    BorderEdge {
        style: Some(style.to_string()),
        color: color.and_then(rgb),
    }
}

/// 테두리 변 넷의 새 값 (None = 그대로, Some(None) = 지움)
#[derive(Debug, Clone, Default)]
pub struct EdgeSet {
    pub left: Option<Option<BorderEdge>>,
    pub right: Option<Option<BorderEdge>>,
    pub top: Option<Option<BorderEdge>>,
    pub bottom: Option<Option<BorderEdge>>,
    /// 대각선까지 지운다
    pub clear_diagonal: bool,
}

impl EdgeSet {
    pub fn is_empty(&self) -> bool {
        self.left.is_none()
            && self.right.is_none()
            && self.top.is_none()
            && self.bottom.is_none()
            && !self.clear_diagonal
    }
}

/// 범위 안 칸 하나(범위의 첫/끝 행·열인가)에 줄 변들
pub fn edges_for(
    preset: BorderPreset,
    e: &BorderEdge,
    first_row: bool,
    last_row: bool,
    first_col: bool,
    last_col: bool,
) -> EdgeSet {
    let on = || Some(Some(e.clone()));
    let mut s = EdgeSet::default();
    match preset {
        BorderPreset::All => {
            s.left = on();
            s.right = on();
            s.top = on();
            s.bottom = on();
        }
        BorderPreset::Outline => {
            if first_col {
                s.left = on();
            }
            if last_col {
                s.right = on();
            }
            if first_row {
                s.top = on();
            }
            if last_row {
                s.bottom = on();
            }
        }
        BorderPreset::Inside => {
            if !first_col {
                s.left = on();
            }
            if !last_col {
                s.right = on();
            }
            if !first_row {
                s.top = on();
            }
            if !last_row {
                s.bottom = on();
            }
        }
        BorderPreset::None => {
            s.left = Some(None);
            s.right = Some(None);
            s.top = Some(None);
            s.bottom = Some(None);
            s.clear_diagonal = true;
        }
        BorderPreset::Left if first_col => s.left = on(),
        BorderPreset::Right if last_col => s.right = on(),
        BorderPreset::Top if first_row => s.top = on(),
        BorderPreset::Bottom if last_row => s.bottom = on(),
        _ => {}
    }
    s
}

/// Xf 의 테두리에 변들을 적용한다
pub fn patch_border(styles: &mut Styles, xf: &mut Xf, set: &EdgeSet) {
    let mut b: Border = styles
        .borders
        .get(xf.border_id as usize)
        .cloned()
        .unwrap_or_default();
    let put = |slot: &mut Option<BorderEdge>, v: &Option<Option<BorderEdge>>| {
        if let Some(v) = v {
            let old_color = slot.as_ref().and_then(|e| e.color.clone());
            *slot = v.clone().map(|mut e| {
                if e.color.is_none() {
                    e.color = Some(old_color.unwrap_or(Color {
                        auto: true,
                        ..Default::default()
                    }));
                }
                e
            });
        }
    };
    put(&mut b.left, &set.left);
    put(&mut b.right, &set.right);
    put(&mut b.top, &set.top);
    put(&mut b.bottom, &set.bottom);
    if set.clear_diagonal {
        b.diagonal = None;
        b.diagonal_up = false;
        b.diagonal_down = false;
    }
    xf.border_id = find_or_push(&mut styles.borders, b);
    xf.apply_border = Some(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats_reuse_builtin_and_custom_ids() {
        let mut st = Styles::default();
        assert_eq!(num_fmt_id(&mut st, "General"), 0);
        assert_eq!(num_fmt_id(&mut st, "0.00%"), 10);
        assert_eq!(num_fmt_id(&mut st, "yyyy-mm-dd"), 14);
        let a = num_fmt_id(&mut st, "0.000");
        assert_eq!(a, 164);
        assert_eq!(num_fmt_id(&mut st, "0.000"), 164);
        assert_eq!(num_fmt_id(&mut st, "#,##0\"원\""), 165);
    }

    #[test]
    fn same_style_is_shared() {
        let mut st = Styles {
            fonts: vec![Font::default()],
            fills: vec![Fill::default()],
            borders: vec![Border::default()],
            cell_xfs: vec![Xf::default()],
            ..Default::default()
        };
        let p = FormatPatch {
            bold: Some(true),
            fill: Some("FFFF00".into()),
            ..Default::default()
        };
        let mut r = Restyler::default();
        let a = r.apply(&mut st, 0, &|s, x| patch_xf(s, x, &p));
        let mut r2 = Restyler::default();
        let b = r2.apply(&mut st, 0, &|s, x| patch_xf(s, x, &p));
        assert_eq!(a, b);
        assert_eq!(st.cell_xfs.len(), 2);
        assert!(st.fonts[st.cell_xfs[a as usize].font_id as usize].bold);
    }
}
