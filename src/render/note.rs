// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 보이는 메모의 상자와 셀로 가는 화살표 (한셀 화면 — 메모 실험: 기본·색·그림자·가운데 맞춤·먼 상자·모양 없는 VML).
//!
//! - 상자 자리는 x:Anchor 의 (열, px, 행, px) 둘. 선은 도형처럼 경계 1px 왼쪽·위 픽셀에 가운데 (0.75pt = 1px,
//!   2pt 는 가운데 ±1.3px). 채우기는 상자 안, 그림자는 오른쪽·아래로 2px 검정.
//! - 글은 상자 안 왼쪽 2px·위 2px 부터, 9pt 돋움 한 줄 15px (서식 있는 글 조각의 글꼴 그대로), 맞춤은 x:TextHAlign·
//!   x:TextVAlign.
//! - 화살표는 상자 왼쪽 위 모서리에서 셀 오른쪽 위 모서리(격자선 교차 픽셀)로 가는 1px 검정 선, 셀 쪽 끝에 길이 9px·
//!   폭 7px 채운 삼각형.
//! - 숨긴 메모는 셀 오른쪽 위 빨간 삼각형만 (CSS `rc-note`).

use std::fmt::Write as _;

use super::cell::font_css;
use super::layout::Layout;
use crate::model::{CommentShape, Sheet};
use crate::theme::Theme;
use crate::writer::esc;

/// 앵커 (칸 번호, px 오프셋) → 표 좌표 (px). 배치 범위 밖은 마지막 칸 끝에서 잇는다
fn at(lay: &Layout, col: u32, dx: u32, row: u32, dy: u32) -> (f64, f64) {
    let x = if col < lay.ncols {
        lay.col_x(col)
    } else {
        lay.col_x.last().copied().unwrap_or(0) + lay.col_w.last().copied().unwrap_or(0)
    };
    let y = if row < lay.nrows {
        lay.row_y(row)
    } else {
        lay.row_y.last().copied().unwrap_or(0) + lay.row_h.last().copied().unwrap_or(0)
    };
    ((x + dx) as f64, (y + dy) as f64)
}

/// 보이는 메모의 상자·화살표 (표 위 절대 위치). `x0`·`y0` 는 머리글 폭·높이
pub(super) fn notes_html(sheet: &Sheet, lay: &Layout, x0: u32, y0: u32, theme: &Theme) -> String {
    let mut s = String::new();
    for c in sheet.comments.iter().filter(|c| c.visible) {
        let shape = c
            .shape
            .clone()
            .unwrap_or_else(|| CommentShape::implied(c.cell));
        let a = shape.anchor;
        let (bx0, by0) = at(lay, a[0], a[1], a[2], a[3]);
        let (bx1, by1) = at(lay, a[4], a[5], a[6], a[7]);
        let (w, h) = ((bx1 - bx0).max(4.0), (by1 - by0).max(4.0));
        let (bx0, by0) = (bx0 + x0 as f64, by0 + y0 as f64);
        let sw = (shape.stroke_weight * 4.0 / 3.0).max(1.0);
        // 셀 오른쪽 위 모서리 (격자선 교차 픽셀 가운데)
        let (cx, cy) = at(lay, c.cell.col + 1, 0, c.cell.row, 0);
        let (cx, cy) = (cx + x0 as f64 - 0.5, cy + y0 as f64 - 0.5);
        // 화살표: 상자 왼쪽 위 → 셀 모서리
        let (lx, ly) = (bx0 - 0.5, by0 - 0.5);
        let (minx, miny) = (lx.min(cx) - 6.0, ly.min(cy) - 6.0);
        let (sw_box, sh_box) = ((lx - cx).abs() + 12.0, (ly - cy).abs() + 12.0);
        let len = ((cx - lx).powi(2) + (cy - ly).powi(2)).sqrt();
        if len > 1.0 {
            let (ux, uy) = ((cx - lx) / len, (cy - ly) / len);
            let (hx, hy) = (cx - ux * 9.0, cy - uy * 9.0);
            let (px, py) = (-uy * 3.5, ux * 3.5);
            let _ = write!(
                s,
                "<svg class=\"rc-note-line\" style=\"left:{minx:.1}px;top:{miny:.1}px\" width=\"{sw_box:.0}\" height=\"{sh_box:.0}\"><g transform=\"translate({:.1},{:.1})\"><path d=\"M{lx:.1},{ly:.1}L{hx:.1},{hy:.1}\" stroke=\"#000\" stroke-width=\"1\"/><path d=\"M{cx:.1},{cy:.1}L{:.1},{:.1}L{:.1},{:.1}Z\" fill=\"#000\"/></g></svg>",
                -minx,
                -miny,
                hx + px,
                hy + py,
                hx - px,
                hy - py
            );
        }
        // 상자 (그림자 → 채우기 → 선)
        let pad = sw / 2.0 + 3.0;
        let shadow = if shape.shadow {
            format!(
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"#000\" stroke=\"#000\" stroke-width=\"{sw:.2}\"/>",
                pad + 2.0,
                pad + 2.0
            )
        } else {
            String::new()
        };
        let _ = write!(
            s,
            "<svg class=\"rc-note-box\" style=\"left:{:.1}px;top:{:.1}px\" width=\"{:.1}\" height=\"{:.1}\">{shadow}<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{sw:.2}\"/></svg>",
            bx0 - 0.5 - pad,
            by0 - 0.5 - pad,
            w + pad * 2.0,
            h + pad * 2.0,
            pad,
            pad,
            esc(&shape.fill_color),
            esc(&shape.stroke_color),
        );
        // 글
        let ta = match shape.text_halign.as_deref() {
            Some("Center") => "center",
            Some("Right") => "right",
            Some("Justify") | Some("Distributed") => "justify",
            _ => "left",
        };
        let jc = match shape.text_valign.as_deref() {
            Some("Center") => "center",
            Some("Bottom") => "flex-end",
            _ => "flex-start",
        };
        let mut body = String::new();
        if c.text.runs.is_empty() {
            body.push_str(&esc(&c.text.text));
        } else {
            for run in &c.text.runs {
                let css = run
                    .font
                    .as_ref()
                    .map(|f| font_css(Some(f), theme, None))
                    .unwrap_or_default();
                let _ = write!(body, "<span style=\"{css}\">{}</span>", esc(&run.text));
            }
        }
        let _ = write!(
            s,
            "<div class=\"rc-note-text\" style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px;justify-content:{jc};text-align:{ta}\"><div>{body}</div></div>",
            bx0 + 2.0,
            by0 + 2.0,
            (w - 4.0).max(1.0),
            (h - 4.0).max(1.0)
        );
    }
    s
}
