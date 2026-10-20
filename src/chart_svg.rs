// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 차트를 SVG 로 그린다 (뷰어). 3차원 차트도 평면으로 그린다.
//!
//! 막대(세로·가로, 묶음·누적·100% 누적), 꺾은선, 영역, 원형·도넛, 분산형을 그린다.
//! 색은 계열에 지정된 색, 없으면 테마 강조색 1–6 을 돌려 쓴다 (Office 기본과 같은 순서).

use std::fmt::Write as _;

use crate::model::{Chart, DrawColor};
use crate::theme::Theme;
use crate::writer::esc;

/// 차트 하나를 `w × h` 픽셀 SVG 로.
pub fn chart_svg(
    ch: &Chart,
    w: f64,
    h: f64,
    theme: &Theme,
    color: &dyn Fn(&DrawColor) -> Option<String>,
) -> String {
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.1} {h:.1}\" font-family=\"'맑은 고딕',sans-serif\" font-size=\"10\">"
    );
    let palette = |i: usize| -> String {
        theme
            .colors
            .get(4 + i % 6)
            .map(|c| format!("#{}", &c[c.len().saturating_sub(6)..]))
            .unwrap_or_else(|| "#4472C4".into())
    };
    let series_color = |i: usize| -> String {
        ch.series
            .get(i)
            .and_then(|x| x.color.as_ref())
            .and_then(color)
            .unwrap_or_else(|| palette(i))
    };

    // 제목
    let mut top = 6.0;
    if let Some(t) = &ch.title {
        let _ = write!(
            s,
            "<text x=\"{:.1}\" y=\"18\" text-anchor=\"middle\" font-size=\"13\" font-weight=\"bold\">{}</text>",
            w / 2.0,
            esc(t)
        );
        top = 28.0;
    }
    // 범례 (오른쪽 또는 아래)
    let pie = matches!(ch.kind.as_str(), "pie" | "doughnut");
    let legend_items: Vec<(String, String)> = if pie {
        let first = ch.series.first();
        first
            .map(|x| {
                (0..x.values.len())
                    .map(|i| {
                        (
                            x.categories
                                .get(i)
                                .cloned()
                                .unwrap_or_else(|| (i + 1).to_string()),
                            palette(i),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        ch.series
            .iter()
            .enumerate()
            .map(|(i, x)| {
                (
                    x.name.clone().unwrap_or_else(|| format!("계열{}", i + 1)),
                    series_color(i),
                )
            })
            .collect()
    };
    let mut right = w - 8.0;
    let mut bottom = h - 6.0;
    match ch.legend.as_deref() {
        Some("b") | Some("t") if !legend_items.is_empty() => {
            bottom -= 16.0;
            let mut x = 10.0;
            for (name, col) in &legend_items {
                let _ = write!(
                    s,
                    "<rect x=\"{x:.1}\" y=\"{:.1}\" width=\"8\" height=\"8\" fill=\"{col}\"/><text x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                    bottom + 6.0,
                    x + 11.0,
                    bottom + 14.0,
                    esc(name)
                );
                x += 20.0 + name.chars().count() as f64 * 7.0;
            }
        }
        Some(_) if !legend_items.is_empty() => {
            let lw = legend_items
                .iter()
                .map(|(n, _)| n.chars().count())
                .max()
                .unwrap_or(0) as f64
                * 7.0
                + 22.0;
            right -= lw.min(w * 0.35);
            let mut y = top + 6.0;
            for (name, col) in &legend_items {
                let _ = write!(
                    s,
                    "<rect x=\"{:.1}\" y=\"{y:.1}\" width=\"8\" height=\"8\" fill=\"{col}\"/><text x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                    right + 8.0,
                    right + 19.0,
                    y + 8.0,
                    esc(name)
                );
                y += 14.0;
            }
        }
        _ => {}
    }

    if pie {
        let Some(x) = ch.series.first() else {
            s.push_str("</svg>");
            return s;
        };
        let vals: Vec<f64> = x.values.iter().map(|v| v.unwrap_or(0.0).max(0.0)).collect();
        let total: f64 = vals.iter().sum();
        let (cx, cy) = ((8.0 + right) / 2.0, (top + bottom) / 2.0);
        let r = ((right - 8.0).min(bottom - top) / 2.0 - 4.0).max(4.0);
        if total <= 0.0 {
            let _ = write!(
                s,
                "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"none\" stroke=\"#bbb\"/>"
            );
        } else {
            let mut a0 = -std::f64::consts::FRAC_PI_2;
            for (i, v) in vals.iter().enumerate() {
                let a1 = a0 + v / total * std::f64::consts::TAU;
                let (x0, y0) = (cx + r * a0.cos(), cy + r * a0.sin());
                let (x1, y1) = (cx + r * a1.cos(), cy + r * a1.sin());
                let large = if a1 - a0 > std::f64::consts::PI { 1 } else { 0 };
                let _ = write!(
                    s,
                    "<path d=\"M{cx:.1},{cy:.1} L{x0:.1},{y0:.1} A{r:.1},{r:.1} 0 {large} 1 {x1:.1},{y1:.1} Z\" fill=\"{}\" stroke=\"#fff\"/>",
                    palette(i)
                );
                a0 = a1;
            }
            if ch.kind == "doughnut" {
                let _ = write!(
                    s,
                    "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{:.1}\" fill=\"#fff\"/>",
                    r * 0.5
                );
            }
        }
        s.push_str("</svg>");
        return s;
    }

    // 축이 있는 차트
    let horizontal = ch.bar_dir.as_deref() == Some("bar") && ch.kind == "bar";
    let grouping = ch.grouping.as_deref().unwrap_or("clustered");
    let stacked = matches!(grouping, "stacked" | "percentStacked")
        && matches!(ch.kind.as_str(), "bar" | "area" | "line");
    let percent = grouping == "percentStacked";
    let n_cat = ch
        .series
        .iter()
        .map(|x| x.values.len().max(x.categories.len()))
        .max()
        .unwrap_or(0)
        .max(1);
    let cats: Vec<String> = ch
        .series
        .iter()
        .find(|x| !x.categories.is_empty())
        .map(|x| x.categories.clone())
        .unwrap_or_else(|| (1..=n_cat).map(|i| i.to_string()).collect());
    let val = |si: usize, ci: usize| {
        ch.series[si]
            .values
            .get(ci)
            .copied()
            .flatten()
            .unwrap_or(0.0)
    };
    let (mut lo, mut hi) = (0.0f64, 0.0f64);
    for ci in 0..n_cat {
        if stacked {
            let (mut pos, mut neg) = (0.0, 0.0);
            for si in 0..ch.series.len() {
                let v = val(si, ci);
                if v >= 0.0 { pos += v } else { neg += v }
            }
            if percent {
                hi = hi.max(1.0);
                lo = lo.min(if neg < 0.0 { -1.0 } else { 0.0 });
            } else {
                hi = hi.max(pos);
                lo = lo.min(neg);
            }
        } else {
            for si in 0..ch.series.len() {
                hi = hi.max(val(si, ci));
                lo = lo.min(val(si, ci));
            }
        }
    }
    if hi == lo {
        hi = lo + 1.0;
    }
    let (lo, hi, step) = nice_scale(lo, hi);

    // 그림 영역
    let label_w = 42.0;
    let (px0, py0, px1, py1) = if horizontal {
        (8.0 + label_w + 20.0, top + 4.0, right, bottom - 16.0)
    } else {
        (8.0 + label_w, top + 4.0, right, bottom - 18.0)
    };
    let (pw, ph) = ((px1 - px0).max(1.0), (py1 - py0).max(1.0));
    // 값 → 좌표
    let vpos = |v: f64| -> f64 {
        let t = (v - lo) / (hi - lo);
        if horizontal {
            px0 + t * pw
        } else {
            py1 - t * ph
        }
    };
    // 눈금선과 값 눈금
    let mut v = lo;
    while v <= hi + step * 1e-9 {
        let p = vpos(v);
        let label = if percent {
            format!("{:.0}%", v * 100.0)
        } else {
            tick_text(v)
        };
        if horizontal {
            let _ = write!(
                s,
                "<line x1=\"{p:.1}\" y1=\"{py0:.1}\" x2=\"{p:.1}\" y2=\"{py1:.1}\" stroke=\"#d9d9d9\"/><text x=\"{p:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{label}</text>",
                py1 + 12.0
            );
        } else {
            let _ = write!(
                s,
                "<line x1=\"{px0:.1}\" y1=\"{p:.1}\" x2=\"{px1:.1}\" y2=\"{p:.1}\" stroke=\"#d9d9d9\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{label}</text>",
                px0 - 4.0,
                p + 3.5
            );
        }
        v += step;
    }
    // 항목 축
    let band = if horizontal { ph } else { pw } / n_cat as f64;
    for (ci, c) in cats.iter().enumerate().take(n_cat) {
        let mid = if horizontal {
            py0 + band * (ci as f64 + 0.5)
        } else {
            px0 + band * (ci as f64 + 0.5)
        };
        if horizontal {
            let _ = write!(
                s,
                "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>",
                px0 - 4.0,
                mid + 3.5,
                esc(c)
            );
        } else {
            let _ = write!(
                s,
                "<text x=\"{mid:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>",
                py1 + 13.0,
                esc(c)
            );
        }
    }
    let base = vpos(0.0f64.clamp(lo, hi));
    if horizontal {
        let _ = write!(
            s,
            "<line x1=\"{base:.1}\" y1=\"{py0:.1}\" x2=\"{base:.1}\" y2=\"{py1:.1}\" stroke=\"#888\"/>"
        );
    } else {
        let _ = write!(
            s,
            "<line x1=\"{px0:.1}\" y1=\"{base:.1}\" x2=\"{px1:.1}\" y2=\"{base:.1}\" stroke=\"#888\"/>"
        );
    }

    let ns = ch.series.len().max(1);
    match ch.kind.as_str() {
        "line" | "area" | "radar" | "scatter" => {
            for si in 0..ch.series.len() {
                let col = series_color(si);
                let mut acc = vec![0.0; n_cat];
                if stacked {
                    for (ci, a) in acc.iter_mut().enumerate() {
                        let total: f64 = (0..ch.series.len()).map(|k| val(k, ci).abs()).sum();
                        let below: f64 = (0..si).map(|k| val(k, ci)).sum();
                        *a = if percent && total > 0.0 {
                            below / total
                        } else {
                            below
                        };
                    }
                }
                let pts: Vec<(f64, f64)> = (0..n_cat)
                    .map(|ci| {
                        let total: f64 = (0..ch.series.len()).map(|k| val(k, ci).abs()).sum();
                        let mut y = val(si, ci);
                        if percent && total > 0.0 {
                            y /= total;
                        }
                        (px0 + band * (ci as f64 + 0.5), vpos(acc[ci] + y))
                    })
                    .collect();
                let path: Vec<String> = pts.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
                if ch.kind == "area" {
                    let back: Vec<String> = (0..n_cat)
                        .rev()
                        .map(|ci| {
                            format!("{:.1},{:.1}", px0 + band * (ci as f64 + 0.5), vpos(acc[ci]))
                        })
                        .collect();
                    let _ = write!(
                        s,
                        "<polygon points=\"{} {}\" fill=\"{col}\" fill-opacity=\"0.8\"/>",
                        path.join(" "),
                        back.join(" ")
                    );
                } else if ch.kind == "scatter" {
                    for (x, y) in &pts {
                        let _ = write!(
                            s,
                            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3\" fill=\"{col}\"/>"
                        );
                    }
                } else {
                    let _ = write!(
                        s,
                        "<polyline points=\"{}\" fill=\"none\" stroke=\"{col}\" stroke-width=\"2\"/>",
                        path.join(" ")
                    );
                }
            }
        }
        _ => {
            // 막대
            let gap = band * 0.25;
            let inner = band - 2.0 * gap;
            for ci in 0..n_cat {
                let total: f64 = (0..ch.series.len()).map(|k| val(k, ci).abs()).sum();
                let (mut pos, mut neg) = (0.0, 0.0);
                for si in 0..ch.series.len() {
                    let mut v = val(si, ci);
                    if percent && total > 0.0 {
                        v /= total;
                    }
                    let (a, b) = if stacked {
                        if v >= 0.0 {
                            pos += v;
                            (pos - v, pos)
                        } else {
                            neg += v;
                            (neg - v, neg)
                        }
                    } else {
                        (0.0, v)
                    };
                    let (p0, p1) = (vpos(a.clamp(lo, hi)), vpos(b.clamp(lo, hi)));
                    let (off, thick) = if stacked {
                        (gap, inner)
                    } else {
                        (gap + inner / ns as f64 * si as f64, inner / ns as f64)
                    };
                    let col = if ch.vary_colors && ch.series.len() == 1 && !stacked {
                        palette(ci)
                    } else {
                        series_color(si)
                    };
                    if horizontal {
                        let y = py0 + band * ci as f64 + off;
                        let _ = write!(
                            s,
                            "<rect x=\"{:.1}\" y=\"{y:.1}\" width=\"{:.1}\" height=\"{thick:.1}\" fill=\"{col}\"/>",
                            p0.min(p1),
                            (p1 - p0).abs()
                        );
                    } else {
                        let x = px0 + band * ci as f64 + off;
                        let _ = write!(
                            s,
                            "<rect x=\"{x:.1}\" y=\"{:.1}\" width=\"{thick:.1}\" height=\"{:.1}\" fill=\"{col}\"/>",
                            p0.min(p1),
                            (p1 - p0).abs()
                        );
                    }
                }
            }
        }
    }
    s.push_str("</svg>");
    s
}

/// 보기 좋은 눈금: (최소, 최대, 간격)
fn nice_scale(lo: f64, hi: f64) -> (f64, f64, f64) {
    let range = hi - lo;
    let raw = range / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag
        * if norm <= 1.0 {
            1.0
        } else if norm <= 2.0 {
            2.0
        } else if norm <= 5.0 {
            5.0
        } else {
            10.0
        };
    ((lo / step).floor() * step, (hi / step).ceil() * step, step)
}

fn tick_text(v: f64) -> String {
    if v.abs() >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if v.fract().abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        format!("{}", (v * 100.0).round() / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_scales() {
        assert_eq!(nice_scale(0.0, 1.0), (0.0, 1.0, 0.2));
        assert_eq!(nice_scale(0.0, 87.0), (0.0, 100.0, 20.0));
        assert_eq!(nice_scale(-30.0, 120.0), (-50.0, 150.0, 50.0));
    }
}
