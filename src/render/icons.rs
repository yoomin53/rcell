// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 조건부 서식 아이콘 (16×16 SVG). 모양·색은 한셀 화면 갈무리(아이콘 실험: 아이콘 집합 20개)에서 잰 것:
//! 칸 왼쪽 3px, 세로 가운데. 번호 0 이 가장 낮은 단계 (빨강·아래쪽).

use std::fmt::Write as _;

const RED: &str = "#F24E4E";
const RED_ARROW: &str = "#F95151";
const YELLOW: &str = "#FFD927";
const GREEN: &str = "#5DBC77";
const GREEN_ARROW: &str = "#61BC66";
const GRAY: &str = "#BFBFBF";
const BLACK: &str = "#333333";
const BLUE: &str = "#266EB1";
const LIGHT: &str = "#B9B9B9";
const GOLD: &str = "#FFC000";

/// 위를 가리키는 화살표를 `deg` 만큼 돌린다 (시계 방향). 한셀 화살표는 상자보다 조금 크다: 머리 15px·8줄,
/// 대 5px·10줄 (18px 높이)
fn arrow(deg: f64, color: &str) -> String {
    format!(
        "<path transform=\"rotate({deg} 8.5 8.5)\" fill=\"{color}\" d=\"M8.5,-0.5L16,7.5L11,7.5L11,17.5L6,17.5L6,7.5L1,7.5Z\"/>"
    )
}

fn circle(color: &str, r: f64) -> String {
    format!("<circle cx=\"8\" cy=\"8\" r=\"{r}\" fill=\"{color}\"/>")
}

/// 별 (가운데 8,8, 바깥 반지름 7.5)
fn star_path() -> String {
    let mut d = String::new();
    for k in 0..10 {
        let r = if k % 2 == 0 { 7.5 } else { 3.2 };
        let a = std::f64::consts::PI * (k as f64 / 5.0 - 0.5);
        let _ = write!(
            d,
            "{}{:.2},{:.2}",
            if k == 0 { "M" } else { "L" },
            8.0 + r * a.cos(),
            8.5 + r * a.sin()
        );
    }
    d.push('Z');
    d
}

/// 아이콘 집합 `set` 의 `idx` 번째 (0 이 가장 낮다) 아이콘 몸통 (viewBox 0 0 16 16)
fn body(set: &str, idx: usize) -> String {
    let pick = |list: Vec<String>| {
        let k = idx.min(list.len() - 1);
        list.into_iter().nth(k).unwrap_or_default()
    };
    match set {
        "3Arrows" => pick(vec![
            arrow(180.0, RED_ARROW),
            arrow(90.0, YELLOW),
            arrow(0.0, GREEN_ARROW),
        ]),
        "3ArrowsGray" => pick(vec![
            arrow(180.0, GRAY),
            arrow(90.0, GRAY),
            arrow(0.0, GRAY),
        ]),
        "4Arrows" => pick(vec![
            arrow(180.0, RED_ARROW),
            arrow(135.0, YELLOW),
            arrow(45.0, YELLOW),
            arrow(0.0, GREEN_ARROW),
        ]),
        "4ArrowsGray" => pick(vec![
            arrow(180.0, GRAY),
            arrow(135.0, GRAY),
            arrow(45.0, GRAY),
            arrow(0.0, GRAY),
        ]),
        "5Arrows" => pick(vec![
            arrow(180.0, RED_ARROW),
            arrow(135.0, YELLOW),
            arrow(90.0, YELLOW),
            arrow(45.0, YELLOW),
            arrow(0.0, GREEN_ARROW),
        ]),
        "5ArrowsGray" => pick(vec![
            arrow(180.0, GRAY),
            arrow(135.0, GRAY),
            arrow(90.0, GRAY),
            arrow(45.0, GRAY),
            arrow(0.0, GRAY),
        ]),
        "3Flags" => {
            let c = [RED, YELLOW, GREEN][idx.min(2)];
            format!(
                "<path d=\"M4,1.5V15\" stroke=\"#7F7F7F\" stroke-width=\"1.2\"/><path d=\"M4.6,2L14,5.2L4.6,9Z\" fill=\"{c}\"/>"
            )
        }
        "3TrafficLights2" => {
            let c = [RED, YELLOW, GREEN][idx.min(2)];
            format!(
                "<rect x=\"0.5\" y=\"0.5\" width=\"15\" height=\"15\" rx=\"3\" fill=\"{BLACK}\"/>{}",
                circle(c, 5.5)
            )
        }
        "4TrafficLights" => circle([BLACK, RED, YELLOW, GREEN][idx.min(3)], 7.5),
        "4RedToBlack" => circle([BLACK, "#C0C0C0", "#FF8B8B", "#FF2525"][idx.min(3)], 7.5),
        "3Signs" => match idx {
            0 => format!("<path d=\"M8,0.5L15.5,8L8,15.5L0.5,8Z\" fill=\"{RED}\"/>"),
            1 => format!("<path d=\"M8,1.5L15.5,14.5L0.5,14.5Z\" fill=\"{YELLOW}\"/>"),
            _ => circle(GREEN, 7.5),
        },
        "3Symbols" => {
            let (c, mark) = match idx {
                0 => (
                    RED,
                    "<path d=\"M5,5L11,11M11,5L5,11\" stroke=\"#fff\" stroke-width=\"2\"/>",
                ),
                1 => (
                    YELLOW,
                    "<path d=\"M8,3.5V9.5\" stroke=\"#fff\" stroke-width=\"2\"/><circle cx=\"8\" cy=\"12\" r=\"1.2\" fill=\"#fff\"/>",
                ),
                _ => (
                    GREEN,
                    "<path d=\"M4.5,8.3L7,10.8L11.8,5.5\" fill=\"none\" stroke=\"#fff\" stroke-width=\"2\"/>",
                ),
            };
            format!("{}{mark}", circle(c, 7.5))
        }
        "3Symbols2" => match idx {
            0 => format!(
                "<path d=\"M3.5,3.5L12.5,12.5M12.5,3.5L3.5,12.5\" stroke=\"{RED}\" stroke-width=\"3\"/>"
            ),
            1 => format!(
                "<path d=\"M8,1.5V10\" stroke=\"{YELLOW}\" stroke-width=\"3\"/><circle cx=\"8\" cy=\"13.5\" r=\"1.6\" fill=\"{YELLOW}\"/>"
            ),
            _ => format!(
                "<path d=\"M2.5,8.5L6.5,12.5L13.5,4\" fill=\"none\" stroke=\"{GREEN}\" stroke-width=\"3\"/>"
            ),
        },
        "3Stars" => {
            let d = star_path();
            match idx {
                0 => format!(
                    "<path d=\"{d}\" fill=\"none\" stroke=\"#E6E6E6\" stroke-width=\"0.8\"/>"
                ),
                1 => format!(
                    "<clipPath id=\"rcis\"><rect width=\"8\" height=\"16\"/></clipPath><path d=\"{d}\" fill=\"none\" stroke=\"#E6E6E6\" stroke-width=\"0.8\"/><path d=\"{d}\" fill=\"{GOLD}\" clip-path=\"url(#rcis)\"/>"
                ),
                _ => format!("<path d=\"{d}\" fill=\"{GOLD}\"/>"),
            }
        }
        "3Triangles" => match idx {
            0 => format!("<path d=\"M2,5H14L8,11Z\" fill=\"{RED_ARROW}\"/>"),
            1 => format!("<rect x=\"2\" y=\"6.5\" width=\"12\" height=\"3\" fill=\"{YELLOW}\"/>"),
            _ => format!("<path d=\"M2,11H14L8,5Z\" fill=\"{GREEN_ARROW}\"/>"),
        },
        "5Boxes" => {
            let mut s = String::new();
            for (k, (x, y)) in [(1.0, 9.0), (9.0, 9.0), (1.0, 1.0), (9.0, 1.0)]
                .iter()
                .enumerate()
            {
                let c = if k < idx { BLUE } else { "#C0C0C0" };
                let _ = write!(
                    s,
                    "<rect x=\"{x}\" y=\"{y}\" width=\"6.5\" height=\"6.5\" fill=\"{c}\"/>"
                );
            }
            s
        }
        "4Rating" | "5Rating" => {
            let on = if set == "4Rating" { idx + 1 } else { idx };
            let mut s = String::new();
            // 막대 4개 (폭 4px, 높이 7·11·15·18px, 아래 끝이 상자 아래 +1px)
            for (k, hgt) in [7.0, 11.0, 15.0, 18.0].iter().enumerate() {
                let c = if k < on { BLUE } else { LIGHT };
                let _ = write!(
                    s,
                    "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"3.6\" height=\"{hgt:.1}\" fill=\"{c}\"/>",
                    0.2 + 4.2 * k as f64,
                    17.0 - hgt
                );
            }
            s
        }
        "5Quarters" => {
            let ring = format!(
                "<circle cx=\"8\" cy=\"8\" r=\"6.8\" fill=\"#fff\" stroke=\"{BLACK}\" stroke-width=\"1.2\"/>"
            );
            let pie = match idx {
                0 => String::new(),
                1 => format!("<path d=\"M8,8V1.2A6.8,6.8 0 0 1 14.8,8Z\" fill=\"{BLACK}\"/>"),
                2 => format!("<path d=\"M8,1.2A6.8,6.8 0 0 1 8,14.8Z\" fill=\"{BLACK}\"/>"),
                3 => format!("<path d=\"M8,8V1.2A6.8,6.8 0 1 1 1.2,8Z\" fill=\"{BLACK}\"/>"),
                _ => circle(BLACK, 7.4),
            };
            format!("{ring}{pie}")
        }
        // 3TrafficLights1 과 모르는 집합: 빨강·노랑·초록 원
        _ => {
            let n = crate::cf_eval::icon_count(set).max(2);
            let k = idx.min(n - 1);
            let c = if k == 0 {
                RED
            } else if k == n - 1 {
                GREEN
            } else {
                YELLOW
            };
            circle(c, 7.5)
        }
    }
}

/// 칸 안 아이콘 (절대 위치 SVG). `top` 은 칸 위에서의 px
pub(super) fn icon_svg(set: &str, idx: usize, top: f64) -> String {
    format!(
        "<svg class=\"rc-icon\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" style=\"top:{top:.0}px\">{}</svg>",
        body(set, idx)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_set_draws_each_level() {
        for set in [
            "3Arrows",
            "3ArrowsGray",
            "3Flags",
            "3TrafficLights1",
            "3TrafficLights2",
            "3Signs",
            "3Symbols",
            "3Symbols2",
            "3Stars",
            "3Triangles",
            "4Arrows",
            "4ArrowsGray",
            "4RedToBlack",
            "4Rating",
            "4TrafficLights",
            "5Arrows",
            "5ArrowsGray",
            "5Rating",
            "5Quarters",
            "5Boxes",
        ] {
            for k in 0..crate::cf_eval::icon_count(set) {
                assert!(icon_svg(set, k, 0.0).contains("<"), "{set} {k}");
            }
        }
        // 0 은 낮은 단계: 3Arrows 는 아래쪽 빨강
        assert!(icon_svg("3Arrows", 0, 0.0).contains(RED_ARROW));
        assert!(icon_svg("3Arrows", 2, 0.0).contains(GREEN_ARROW));
    }
}
