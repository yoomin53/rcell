// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그리기 묶음 → 장치 해상도 비트맵. 한셀 PDF 는 차트를 600 dpi 장치 해상도 그림(RGB + 알파)으로 인쇄하므로
//! (docs/format/print.md), 차트의 그리기 목록을 같은 크기의 그림으로 그려 넣는다.
//! 윤곽 채우기·선은 zeno (256단계 안티앨리어싱, 순수 Rust), 글자는 글꼴 파일의 글리프 외곽선.
//! 그림·밑줄처럼 못 그리는 것이 있으면 None — 그때는 벡터로 그린다.

use zeno::{Cap, Command, Fill, Join, Mask, Point, Stroke};

use super::pdf::FontSet;
use super::pdfimage::PdfImage;
use crate::render::page::{Op, Seg};

/// 그림 한 장의 최대 픽셀 수 (넘으면 벡터로)
const MAX_PIXELS: i64 = 64_000_000;

/// 미리 곱한 알파 RGBA 화면
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u8>,
    clips: Vec<[i32; 4]>,
}

impl Canvas {
    fn clip(&self) -> [i32; 4] {
        self.clips.last().copied().unwrap_or([0, 0, self.w, self.h])
    }

    /// 알파 마스크 `m` (자리 `at`: 왼쪽·위·너비·높이) 를 색 `c` 로 칠한다
    fn blend(&mut self, m: &[u8], at: (i32, i32, i32, i32), c: [u8; 3]) {
        let [cx0, cy0, cx1, cy1] = self.clip();
        let (left, top, mw, mh) = at;
        let (x0, y0) = (left.max(cx0), top.max(cy0));
        let (x1, y1) = ((left + mw).min(cx1), (top + mh).min(cy1));
        for y in y0..y1 {
            let row = ((y - top) * mw) as usize;
            for x in x0..x1 {
                let a = m[row + (x - left) as usize] as u32;
                if a == 0 {
                    continue;
                }
                let i = ((y * self.w + x) * 4) as usize;
                let ia = 255 - a;
                for (k, v) in c.iter().enumerate() {
                    self.px[i + k] =
                        ((*v as u32 * a + self.px[i + k] as u32 * ia + 127) / 255) as u8;
                }
                self.px[i + 3] = ((255 * a + self.px[i + 3] as u32 * ia + 127) / 255) as u8;
            }
        }
    }

    fn fill(&mut self, cmds: &[Command], color: [u8; 3], even_odd: bool) {
        let style = if even_odd {
            Fill::EvenOdd
        } else {
            Fill::NonZero
        };
        let (m, pl) = Mask::new(cmds).style(style).render();
        self.blend(
            &m,
            (pl.left, pl.top, pl.width as i32, pl.height as i32),
            color,
        );
    }

    fn stroke(
        &mut self,
        cmds: &[Command],
        width: f64,
        color: [u8; 3],
        dash: &[f64],
        caps: (u8, u8),
    ) {
        let d: Vec<f32> = dash.iter().map(|v| *v as f32).collect();
        let mut st = Stroke::new(width as f32);
        st.cap(match caps.0 {
            1 => Cap::Round,
            2 => Cap::Square,
            _ => Cap::Butt,
        })
        .join(match caps.1 {
            1 => Join::Round,
            2 => Join::Bevel,
            _ => Join::Miter,
        })
        .miter_limit(10.0);
        if d.len() >= 2 {
            st.dash(&d, 0.0);
        }
        let (m, pl) = Mask::new(cmds).style(st).render();
        self.blend(
            &m,
            (pl.left, pl.top, pl.width as i32, pl.height as i32),
            color,
        );
    }
}

/// 경로 조각 → zeno 명령 (점마다 `map`)
fn commands(segs: &[Seg], map: impl Fn(f64, f64) -> (f64, f64)) -> Vec<Command> {
    let p = |x: f64, y: f64| {
        let (a, b) = map(x, y);
        Point::new(a as f32, b as f32)
    };
    segs.iter()
        .map(|s| match *s {
            Seg::Move(x, y) => Command::MoveTo(p(x, y)),
            Seg::Line(x, y) => Command::LineTo(p(x, y)),
            Seg::Cubic(a, b, c, d, e, f) => Command::CurveTo(p(a, b), p(c, d), p(e, f)),
            Seg::Close => Command::Close,
        })
        .collect()
}

/// `ops` 를 상자 `(x0, y0, x1, y1)` (장치 단위, 정수 픽셀로 맞춘다) 크기의 그림으로. 못 그리는 것이 있으면 None
pub fn render(
    ops: &[Op],
    (bx0, by0, bx1, by1): (f64, f64, f64, f64),
    fonts: &FontSet,
) -> Option<PdfImage> {
    let (ox, oy) = (bx0.round(), by0.round());
    let (w, h) = ((bx1 - bx0).round() as i32, (by1 - by0).round() as i32);
    if w <= 0 || h <= 0 || w as i64 * h as i64 > MAX_PIXELS {
        return None;
    }
    let mut cv = Canvas {
        w,
        h,
        px: vec![0; (w * h * 4) as usize],
        clips: Vec::new(),
    };
    let to = |x: f64, y: f64| (x - ox, y - oy);
    for op in ops {
        match op {
            Op::Path {
                segs,
                fill,
                stroke,
                even_odd,
                caps,
                hatch,
            } => {
                if hatch.is_some() {
                    return None;
                }
                let cmds = commands(segs, to);
                if let Some(c) = fill {
                    cv.fill(&cmds, *c, *even_odd);
                }
                if let Some((sw, c, dash)) = stroke {
                    cv.stroke(&cmds, *sw, *c, dash, caps.unwrap_or((0, 0)));
                }
            }
            Op::Rect {
                x0,
                y0,
                x1,
                y1,
                color,
            } => {
                let segs = [
                    Seg::Move(*x0, *y0),
                    Seg::Line(*x1, *y0),
                    Seg::Line(*x1, *y1),
                    Seg::Line(*x0, *y1),
                    Seg::Close,
                ];
                cv.fill(&commands(&segs, to), *color, false);
            }
            Op::Line {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
                dash,
            } => {
                let segs = [Seg::Move(*x0, *y0), Seg::Line(*x1, *y1)];
                cv.stroke(&commands(&segs, to), *width, *color, dash, (0, 0));
            }
            Op::Text {
                x,
                y,
                size,
                font,
                color,
                text,
                underline,
                strike,
                stretch,
                angle,
            } => {
                if *underline || *strike {
                    return None;
                }
                let glyphs = fonts.glyph_paths(font, text, *size, *stretch)?;
                let (s, c) = angle.to_radians().sin_cos();
                let map = |gx: f64, gy: f64| (x + gx * c - gy * s - ox, y + gx * s + gy * c - oy);
                for g in &glyphs {
                    if !g.is_empty() {
                        // 글리프 겹침은 비영 규칙 (TrueType)
                        cv.fill(&commands(g, map), *color, false);
                    }
                }
            }
            Op::Clip { x0, y0, x1, y1 } => {
                let cur = cv.clip();
                let r = [
                    ((x0 - ox).round() as i32).max(cur[0]),
                    ((y0 - oy).round() as i32).max(cur[1]),
                    ((x1 - ox).round() as i32).min(cur[2]),
                    ((y1 - oy).round() as i32).min(cur[3]),
                ];
                cv.clips.push(r);
            }
            Op::Unclip => {
                cv.clips.pop();
            }
            Op::Image { .. } | Op::Raster { .. } => return None,
        }
    }
    // 미리 곱한 알파 → 곧은 RGB + 알파 (한셀 PDF 는 JPEG + 알파 마스크 — 여기서는 무손실)
    let n = (w * h) as usize;
    let mut rgb = Vec::with_capacity(n * 3);
    let mut alpha = Vec::with_capacity(n);
    let mut opaque = true;
    for p in cv.px.as_chunks::<4>().0 {
        let a = p[3] as u32;
        if a == 255 {
            rgb.extend_from_slice(&p[..3]);
        } else {
            // 투명한 곳은 흰색
            for v in &p[..3] {
                rgb.push(
                    (*v as u32 * 255 + a / 2)
                        .checked_div(a)
                        .map_or(255, |x| x.min(255)) as u8,
                );
            }
        }
        opaque &= a == 255;
        alpha.push(a as u8);
    }
    Some(PdfImage {
        width: w as u32,
        height: h as u32,
        components: 3,
        dct: false,
        data: rgb,
        alpha: (!opaque).then_some(alpha),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::FontDb;

    #[test]
    fn fills_and_strokes_into_a_transparent_bitmap() {
        let db = FontDb::new();
        let fonts = FontSet::new(&db);
        let ops = vec![
            Op::Rect {
                x0: 102.0,
                y0: 102.0,
                x1: 106.0,
                y1: 106.0,
                color: [255, 0, 0],
            },
            Op::Line {
                x0: 100.0,
                y0: 109.5,
                x1: 110.0,
                y1: 109.5,
                width: 1.0,
                color: [0, 0, 255],
                dash: Vec::new(),
            },
        ];
        let img = render(&ops, (100.0, 100.0, 110.0, 110.0), &fonts).unwrap();
        assert_eq!((img.width, img.height), (10, 10));
        let alpha = img.alpha.as_ref().unwrap();
        // 칠한 곳은 불투명 빨강, 빈 곳은 투명
        let at = |x: usize, y: usize| (y * 10 + x, alpha[y * 10 + x]);
        let (i, a) = at(3, 3);
        assert_eq!(a, 255);
        assert_eq!(&img.data[i * 3..i * 3 + 3], &[255, 0, 0]);
        assert_eq!(at(8, 3).1, 0);
        // 파란 선은 아래 한 줄
        let (j, a) = at(5, 9);
        assert_eq!(a, 255);
        assert_eq!(&img.data[j * 3..j * 3 + 3], &[0, 0, 255]);
        // 못 그리는 것(그림)이 있으면 None
        let none = render(
            &[Op::Image {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                data: std::sync::Arc::new(Vec::new()),
            }],
            (0.0, 0.0, 4.0, 4.0),
            &fonts,
        );
        assert!(none.is_none());
    }
}
