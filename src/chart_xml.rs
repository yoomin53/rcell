// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! [`Chart`] → 차트 파트 (`xl/charts/chartN.xml`), 한셀이 쓰는 꼴 그대로.
//!
//! 모델에 있는 요소만 쓴다 (`None` 이면 쓰지 않는다). 한셀이 늘 같게 쓰는 것은 여기서 정한다
//! (근거: `*.legacy.as.cell` 77개·`hs/*.hs.cell`·한셀이 저장한 XML 차트 파트, docs/format/cell-2024.md 11절):
//!
//! - 선언 `<?xml … standalone="yes" ?>`, `c:chartSpace` 의 `xmlns:r`·`xmlns:a`·`xmlns:c` (이 순서),
//!   `mc:`·`c14:`·`ho:` 는 쓰는 요소에서 선언
//! - 요소 순서: ECMA-376 스키마 순서와 같되 한셀의 세 가지 차이를 따른다 — 축은 `axPos`·`crossAx`·`delete`
//!   순으로 `c:scaling` 바로 뒤, `c:view3D` 는 `rAngAx` 가 먼저, 차트 공간은 `c:txPr` 가 `c:spPr` 앞
//! - `c:dLbls` 는 `c:delete` 와 표시 여부를 함께 쓴다 (스키마는 둘 중 하나)
//! - `a:bodyPr` 는 `rot`·`vert` 뒤에 `wrap="none" lIns="0" tIns="0" rIns="0" bIns="0" anchor="ctr" anchorCtr="1"`,
//!   문단 끝은 빈 `<a:endParaRPr/>`
//! - 빈 요소는 닫아 쓴다 (`<c:layout/>`, `<c:v/>`, `<a:ln w="38100" cap="rnd" cmpd="sng"/>`)
//! - `a:srgbClr` 는 소문자 16진 (원본 `FFFF00` → `ffff00`)
//! - 배치 비율(`c:manualLayout` 의 `c:x` …)은 f32 로 바꾼 값을 소수 20자리로 (`0.3` → `0.30000001192092895508`)
//! - 참·거짓은 `1`/`0`
//! - 한컴 확장은 `c:ext uri="CC8EB2C9-7E31-499d-B8F2-F6CE61031016"` 안의 `ho:hncChartStyle`

use std::fmt::Write as _;

use crate::model::{
    Chart, ChartAxis, ChartData, ChartDataLabels, ChartFill, ChartFont, ChartGroup,
    ChartGroupProps, ChartLayout, ChartLine, ChartLineEnd, ChartLines, ChartMarker, ChartNumFmt,
    ChartRich, ChartSeries, ChartShapeProps, ChartSurface, ChartTitle, ChartTx,
};
use crate::ns::{HANCOM_OFFICE_8 as NS_HO, MC as NS_MC, REL as NS_R};
use crate::parser::chart::C14 as NS_C14;
use crate::parser::drawing::{A as NS_A, C as NS_C};
use crate::writer::esc;

const DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>"#;
/// 한컴 차트 확장의 `c:ext/@uri` (관측)
pub const HNC_EXT_URI: &str = "CC8EB2C9-7E31-499d-B8F2-F6CE61031016";
/// 한셀이 `a:bodyPr` 에 늘 붙이는 속성 (`rot`·`vert` 뒤)
const BODY_PR_TAIL: &str =
    r#" wrap="none" lIns="0" tIns="0" rIns="0" bIns="0" anchor="ctr" anchorCtr="1""#;

/// 차트 하나를 차트 파트 XML 로.
pub fn chart_xml(ch: &Chart) -> String {
    let mut s = String::with_capacity(8192);
    s.push_str(DECL);
    let ns = format!(r#" xmlns:r="{NS_R}" xmlns:a="{NS_A}" xmlns:c="{NS_C}""#);
    el(
        &mut s,
        "c:chartSpace",
        |s| s.push_str(&ns),
        |s| chart_space(s, ch),
    );
    s
}

fn chart_space(s: &mut String, ch: &Chart) {
    val_bool(s, "c:date1904", ch.date1904);
    val_bool(s, "c:roundedCorners", ch.rounded_corners);
    match ch.c14_style {
        Some(c14) => {
            let mc = format!(r#" xmlns:mc="{NS_MC}""#);
            el(
                s,
                "mc:AlternateContent",
                |s| s.push_str(&mc),
                |s| {
                    let choice = format!(r#" xmlns:c14="{NS_C14}" Requires="c14""#);
                    el(
                        s,
                        "mc:Choice",
                        |s| s.push_str(&choice),
                        |s| {
                            val(s, "c14:style", Some(c14));
                        },
                    );
                    el(s, "mc:Fallback", no_attrs, |s| val(s, "c:style", ch.style));
                },
            );
        }
        None => val(s, "c:style", ch.style),
    }
    el(s, "c:chart", no_attrs, |s| chart(s, ch));
    // 한셀은 차트 공간의 txPr 를 spPr 앞에 쓴다 (서식 파일의 hs 저장본)
    if let Some(r) = &ch.tx_pr {
        rich(s, "c:txPr", r);
    }
    sp_pr(s, &ch.sp_pr);
    if let Some(h) = &ch.hnc_chart_style {
        el(s, "c:extLst", no_attrs, |s| {
            el(
                s,
                "c:ext",
                |s| attr(s, "uri", Some(HNC_EXT_URI)),
                |s| {
                    let attrs = |s: &mut String| {
                        let _ = write!(s, r#" xmlns:ho="{NS_HO}""#);
                        attr(s, "layoutIndex", h.layout_index);
                        attr(s, "colorIndex", h.color_index);
                        attr(s, "styleIndex", h.style_index);
                    };
                    el(s, "ho:hncChartStyle", attrs, |_| {});
                },
            );
        });
    }
}

fn chart(s: &mut String, ch: &Chart) {
    if let Some(t) = &ch.title_full {
        title(s, t);
    }
    val_bool(s, "c:autoTitleDeleted", ch.auto_title_deleted);
    if let Some(v) = &ch.view3d {
        el(s, "c:view3D", no_attrs, |s| {
            val_bool(s, "c:rAngAx", v.r_ang_ax);
            val(s, "c:rotX", v.rot_x);
            val(s, "c:rotY", v.rot_y);
            val(s, "c:perspective", v.perspective);
            val(s, "c:depthPercent", v.depth_percent);
            // 한셀의 순서에서 위치를 관측하지 못했다 (스키마로는 rotX 다음)
            val(s, "c:hPercent", v.h_percent);
        });
    }
    surface(s, "c:floor", &ch.floor);
    surface(s, "c:sideWall", &ch.side_wall);
    surface(s, "c:backWall", &ch.back_wall);
    el(s, "c:plotArea", no_attrs, |s| {
        if let Some(l) = &ch.plot_layout {
            layout(s, l);
        }
        if !ch.kind.is_empty() {
            group(
                s,
                &Group {
                    kind: &ch.kind,
                    three_d: ch.three_d,
                    bar_dir: ch.bar_dir.as_deref(),
                    grouping: ch.grouping.as_deref(),
                    vary_colors: ch.vary_colors,
                    props: &ch.group,
                    series: &ch.series,
                },
            );
        }
        for g in &ch.more_groups {
            group(s, &Group::of(g));
        }
        for a in &ch.axes {
            axis(s, a);
        }
        sp_pr(s, &ch.plot_sp_pr);
    });
    if let Some(pos) = &ch.legend {
        let l = &ch.legend_props;
        el(s, "c:legend", no_attrs, |s| {
            val(s, "c:legendPos", Some(esc(pos)));
            if let Some(x) = &l.layout {
                layout(s, x);
            }
            val_bool(s, "c:overlay", l.overlay);
            sp_pr(s, &l.sp_pr);
            if let Some(r) = &l.tx_pr {
                rich(s, "c:txPr", r);
            }
        });
    }
    val_bool(s, "c:plotVisOnly", ch.plot_vis_only);
    val_str(s, "c:dispBlanksAs", &ch.disp_blanks_as);
}

/// 그림 영역의 차트 하나 (첫 차트는 [`Chart`] 의 필드, 나머지는 [`ChartGroup`])
struct Group<'a> {
    kind: &'a str,
    three_d: bool,
    bar_dir: Option<&'a str>,
    grouping: Option<&'a str>,
    vary_colors: bool,
    props: &'a ChartGroupProps,
    series: &'a [ChartSeries],
}

impl<'a> Group<'a> {
    fn of(g: &'a ChartGroup) -> Self {
        Group {
            kind: &g.kind,
            three_d: g.three_d,
            bar_dir: g.bar_dir.as_deref(),
            grouping: g.grouping.as_deref(),
            vary_colors: g.vary_colors,
            props: &g.props,
            series: &g.series,
        }
    }
}

/// 차트 묶음. 요소 순서는 종류마다 스키마 순서인데, 종류마다 쓰는 요소가 겹치지 않아 한 순서로 모두 맞는다.
fn group(s: &mut String, x: &Group) {
    let g = x.props;
    let name = if g.of_pie_type.is_some() {
        "c:ofPieChart".to_string()
    } else {
        format!("c:{}{}Chart", x.kind, if x.three_d { "3D" } else { "" })
    };
    el(s, &name, no_attrs, |s| {
        val_str(s, "c:radarStyle", &g.radar_style);
        val_str(s, "c:scatterStyle", &g.scatter_style);
        val_str(s, "c:ofPieType", &g.of_pie_type);
        val_bool(s, "c:wireframe", g.wireframe);
        val(s, "c:barDir", x.bar_dir.map(esc));
        val(s, "c:grouping", x.grouping.map(esc));
        // 주식형·표면형에는 varyColors 가 없다
        if !matches!(x.kind, "stock" | "surface") {
            val_bool(s, "c:varyColors", Some(x.vary_colors));
        }
        let xy = matches!(x.kind, "scatter" | "bubble");
        for r in x.series {
            ser(s, r, xy);
        }
        if let Some(d) = &g.d_lbls {
            d_lbls(s, d);
        }
        val(s, "c:gapWidth", g.gap_width);
        val(s, "c:overlap", g.overlap);
        val_str(s, "c:splitType", &g.split_type);
        val(s, "c:splitPos", g.split_pos.map(num));
        val(s, "c:secondPieSize", g.second_pie_size);
        lines(s, "c:serLines", &g.ser_lines);
        lines(s, "c:dropLines", &g.drop_lines);
        lines(s, "c:hiLowLines", &g.hi_low_lines);
        val(s, "c:gapDepth", g.gap_depth);
        val_str(s, "c:shape", &g.shape);
        val_bool(s, "c:marker", g.marker);
        val_bool(s, "c:smooth", g.smooth);
        val(s, "c:firstSliceAng", g.first_slice_ang);
        val(s, "c:holeSize", g.hole_size);
        val_bool(s, "c:bubble3D", g.bubble3d);
        val(s, "c:bubbleScale", g.bubble_scale);
        val_bool(s, "c:showNegBubbles", g.show_neg_bubbles);
        val_str(s, "c:sizeRepresents", &g.size_represents);
        for id in &g.ax_ids {
            val(s, "c:axId", Some(id));
        }
    });
}

/// 계열 (`c:ser`). 분산형·거품형은 항목·값을 `c:xVal`/`c:yVal` 로 쓴다.
fn ser(s: &mut String, x: &ChartSeries, xy: bool) {
    el(s, "c:ser", no_attrs, |s| {
        val(s, "c:idx", x.idx);
        val(s, "c:order", x.order);
        if let Some(t) = &x.tx {
            tx(s, t);
        }
        sp_pr(s, &x.sp_pr);
        val_bool(s, "c:invertIfNegative", x.invert_if_negative);
        if let Some(m) = &x.marker {
            marker(s, m);
        }
        val(s, "c:explosion", x.explosion);
        for p in &x.d_pts {
            el(s, "c:dPt", no_attrs, |s| {
                val(s, "c:idx", Some(p.idx));
                val_bool(s, "c:invertIfNegative", p.invert_if_negative);
                if let Some(m) = &p.marker {
                    marker(s, m);
                }
                val_bool(s, "c:bubble3D", p.bubble3d);
                val(s, "c:explosion", p.explosion);
                sp_pr(s, &p.sp_pr);
            });
        }
        if let Some(d) = &x.d_lbls {
            d_lbls(s, d);
        }
        let (cat, v) = if xy {
            ("c:xVal", "c:yVal")
        } else {
            ("c:cat", "c:val")
        };
        source(s, cat, &x.cat);
        source(s, v, &x.val);
        val_str(s, "c:shape", &x.shape);
        source(s, "c:bubbleSize", &x.bubble_size);
        val_bool(s, "c:bubble3D", x.bubble3d);
        val_bool(s, "c:smooth", x.smooth);
    });
}

/// 축. 한셀은 `axPos`·`crossAx`·`delete` 를 `c:scaling` 바로 뒤에 쓴다 (스키마는 `delete`, `axPos` … `crossAx`).
fn axis(s: &mut String, a: &ChartAxis) {
    el(s, &format!("c:{}", a.kind), no_attrs, |s| {
        val(s, "c:axId", Some(a.id));
        el(s, "c:scaling", no_attrs, |s| {
            val(s, "c:logBase", a.scaling.log_base.map(num));
            val_str(s, "c:orientation", &a.scaling.orientation);
            val(s, "c:max", a.scaling.max.map(num));
            val(s, "c:min", a.scaling.min.map(num));
        });
        val_str(s, "c:axPos", &a.ax_pos);
        val(s, "c:crossAx", a.cross_ax);
        val_bool(s, "c:delete", a.delete);
        lines(s, "c:majorGridlines", &a.major_gridlines);
        lines(s, "c:minorGridlines", &a.minor_gridlines);
        if let Some(t) = &a.title {
            title(s, t);
        }
        if let Some(n) = &a.num_fmt {
            num_fmt(s, n);
        }
        val_str(s, "c:majorTickMark", &a.major_tick_mark);
        val_str(s, "c:minorTickMark", &a.minor_tick_mark);
        val_str(s, "c:tickLblPos", &a.tick_lbl_pos);
        sp_pr(s, &a.sp_pr);
        if let Some(r) = &a.tx_pr {
            rich(s, "c:txPr", r);
        }
        val_str(s, "c:crosses", &a.crosses);
        val(s, "c:crossesAt", a.crosses_at.map(num));
        val_str(s, "c:crossBetween", &a.cross_between);
        val_bool(s, "c:auto", a.auto);
        val_str(s, "c:lblAlgn", &a.lbl_algn);
        val(s, "c:lblOffset", a.lbl_offset);
        val_str(s, "c:baseTimeUnit", &a.base_time_unit);
        val(s, "c:tickLblSkip", a.tick_lbl_skip);
        val(s, "c:tickMarkSkip", a.tick_mark_skip);
        val_bool(s, "c:noMultiLvlLbl", a.no_multi_lvl_lbl);
        val(s, "c:majorUnit", a.major_unit.map(num));
        val_str(s, "c:majorTimeUnit", &a.major_time_unit);
        val(s, "c:minorUnit", a.minor_unit.map(num));
        val_str(s, "c:minorTimeUnit", &a.minor_time_unit);
    });
}

fn title(s: &mut String, t: &ChartTitle) {
    el(s, "c:title", no_attrs, |s| {
        if let Some(x) = &t.tx {
            tx(s, x);
        }
        if let Some(l) = &t.layout {
            layout(s, l);
        }
        val_bool(s, "c:overlay", t.overlay);
        sp_pr(s, &t.sp_pr);
        if let Some(r) = &t.tx_pr {
            rich(s, "c:txPr", r);
        }
    });
}

/// `c:tx` (제목의 `c:rich`, 이름 참조 `c:strRef`, 글 `c:v`)
fn tx(s: &mut String, t: &ChartTx) {
    el(s, "c:tx", no_attrs, |s| match t {
        ChartTx::Rich(r) => rich(s, "c:rich", r),
        ChartTx::Ref(d) => data(s, d),
        ChartTx::Value(v) => text_el(s, "c:v", v),
    });
}

fn layout(s: &mut String, l: &ChartLayout) {
    el(s, "c:layout", no_attrs, |s| {
        let Some(m) = &l.manual else { return };
        el(s, "c:manualLayout", no_attrs, |s| {
            val_str(s, "c:layoutTarget", &m.layout_target);
            val_str(s, "c:xMode", &m.x_mode);
            val_str(s, "c:yMode", &m.y_mode);
            val_str(s, "c:wMode", &m.w_mode);
            val_str(s, "c:hMode", &m.h_mode);
            val(s, "c:x", m.x.map(ratio));
            val(s, "c:y", m.y.map(ratio));
            val(s, "c:w", m.w.map(ratio));
            val(s, "c:h", m.h.map(ratio));
        });
    });
}

fn surface(s: &mut String, tag: &str, x: &Option<ChartSurface>) {
    let Some(x) = x else { return };
    el(s, tag, no_attrs, |s| {
        val(s, "c:thickness", x.thickness);
        sp_pr(s, &x.sp_pr);
    });
}

fn lines(s: &mut String, tag: &str, l: &Option<ChartLines>) {
    let Some(l) = l else { return };
    el(s, tag, no_attrs, |s| sp_pr(s, &l.sp_pr));
}

fn marker(s: &mut String, m: &ChartMarker) {
    el(s, "c:marker", no_attrs, |s| {
        val_str(s, "c:symbol", &m.symbol);
        val(s, "c:size", m.size);
        sp_pr(s, &m.sp_pr);
    });
}

/// 데이터 레이블. 한셀은 `c:delete` 를 먼저, 표시 여부들을 함께 쓴다 (`numFmt`·`spPr`·`txPr` 위치는 스키마 순서).
fn d_lbls(s: &mut String, d: &ChartDataLabels) {
    el(s, "c:dLbls", no_attrs, |s| {
        val_bool(s, "c:delete", d.delete);
        if let Some(n) = &d.num_fmt {
            num_fmt(s, n);
        }
        sp_pr(s, &d.sp_pr);
        if let Some(r) = &d.tx_pr {
            rich(s, "c:txPr", r);
        }
        val_str(s, "c:dLblPos", &d.d_lbl_pos);
        val_bool(s, "c:showLegendKey", d.show_legend_key);
        val_bool(s, "c:showVal", d.show_val);
        val_bool(s, "c:showCatName", d.show_cat_name);
        val_bool(s, "c:showSerName", d.show_ser_name);
        val_bool(s, "c:showPercent", d.show_percent);
        val_bool(s, "c:showBubbleSize", d.show_bubble_size);
        if let Some(sep) = &d.separator {
            text_el(s, "c:separator", sep);
        }
        val_bool(s, "c:showLeaderLines", d.show_leader_lines);
    });
}

fn num_fmt(s: &mut String, n: &ChartNumFmt) {
    let attrs = |s: &mut String| {
        attr(s, "formatCode", Some(esc(&n.format_code)));
        attr(s, "sourceLinked", n.source_linked.map(b));
    };
    el(s, "c:numFmt", attrs, |_| {});
}

/// `c:cat`·`c:val` 따위 — 안에 데이터 원본 하나
fn source(s: &mut String, tag: &str, d: &Option<ChartData>) {
    let Some(d) = d else { return };
    el(s, tag, no_attrs, |s| data(s, d));
}

/// 데이터 원본: 참조는 `c:f` + 캐시(`numCache`/`strCache`), 리터럴은 값 그대로.
fn data(s: &mut String, d: &ChartData) {
    el(s, &format!("c:{}", d.kind), no_attrs, |s| {
        if let Some(f) = &d.formula {
            text_el(s, "c:f", f);
        }
        let Some(c) = &d.cache else { return };
        let body = |s: &mut String| {
            if let Some(f) = &c.format_code {
                text_el(s, "c:formatCode", f);
            }
            val(s, "c:ptCount", c.pt_count);
            for p in &c.points {
                let attrs = |s: &mut String| {
                    attr(s, "idx", Some(p.idx));
                    attr(s, "formatCode", p.format_code.as_deref().map(esc));
                };
                el(s, "c:pt", attrs, |s| text_el(s, "c:v", &p.value));
            }
        };
        match d.kind.as_str() {
            "numRef" => el(s, "c:numCache", no_attrs, body),
            "strRef" => el(s, "c:strCache", no_attrs, body),
            "multiLvlStrRef" => el(s, "c:multiLvlStrCache", no_attrs, body),
            _ => body(s),
        }
    });
}

fn sp_pr(s: &mut String, p: &Option<ChartShapeProps>) {
    let Some(p) = p else { return };
    el(s, "c:spPr", no_attrs, |s| {
        if let Some(f) = &p.fill {
            fill(s, f);
        }
        if let Some(l) = &p.line {
            line(s, l);
        }
    });
}

fn fill(s: &mut String, f: &ChartFill) {
    match f {
        ChartFill::None => s.push_str("<a:noFill/>"),
        ChartFill::Solid(c) => el(s, "a:solidFill", no_attrs, |s| {
            let (kind, v) = c.value.split_once(':').unwrap_or(("srgb", &c.value));
            let tag = match kind {
                "srgb" => "a:srgbClr".to_string(),
                "scheme" => "a:schemeClr".to_string(),
                "sys" => "a:sysClr".to_string(),
                "prst" => "a:prstClr".to_string(),
                other => format!("a:{other}"),
            };
            // 한셀은 RGB 16진을 소문자로 쓴다
            let v = if kind == "srgb" {
                v.to_ascii_lowercase()
            } else {
                v.to_string()
            };
            let attrs = |s: &mut String| {
                attr(s, "val", Some(esc(&v)));
                attr(s, "lastClr", c.last_clr.as_deref().map(esc));
            };
            el(s, &tag, attrs, |s| {
                for (k, m) in &c.mods {
                    let _ = write!(s, r#"<a:{k} val="{m}"/>"#);
                }
            });
        }),
        ChartFill::Other(xml) => s.push_str(xml),
    }
}

fn line(s: &mut String, l: &ChartLine) {
    let attrs = |s: &mut String| {
        attr(s, "w", l.width);
        attr(s, "cap", l.cap.as_deref().map(esc));
        attr(s, "cmpd", l.cmpd.as_deref().map(esc));
        attr(s, "algn", l.algn.as_deref().map(esc));
    };
    el(s, "a:ln", attrs, |s| {
        if let Some(f) = &l.fill {
            fill(s, f);
        }
        val_str(s, "a:prstDash", &l.dash);
        if let Some(j) = &l.join {
            let _ = write!(s, "<a:{j}/>");
        }
        line_end(s, "a:headEnd", &l.head_end);
        line_end(s, "a:tailEnd", &l.tail_end);
    });
}

fn line_end(s: &mut String, tag: &str, e: &Option<ChartLineEnd>) {
    let Some(e) = e else { return };
    let attrs = |s: &mut String| {
        attr(s, "type", e.kind.as_deref().map(esc));
        attr(s, "w", e.w.as_deref().map(esc));
        attr(s, "len", e.len.as_deref().map(esc));
    };
    el(s, tag, attrs, |_| {});
}

/// `c:rich`/`c:txPr`: 한셀 꼴의 `a:bodyPr` + 문단들
fn rich(s: &mut String, tag: &str, r: &ChartRich) {
    el(s, tag, no_attrs, |s| {
        let body_pr = |s: &mut String| {
            attr(s, "rot", r.rot);
            attr(s, "vert", r.vert.as_deref().map(esc));
            s.push_str(BODY_PR_TAIL);
        };
        el(s, "a:bodyPr", body_pr, |_| {});
        for p in &r.paras {
            el(s, "a:p", no_attrs, |s| {
                if p.align.is_some() || p.def_font.is_some() {
                    let algn = |s: &mut String| attr(s, "algn", p.align.as_deref().map(esc));
                    el(s, "a:pPr", algn, |s| {
                        if let Some(f) = &p.def_font {
                            font(s, "a:defRPr", f);
                        }
                    });
                }
                for run in &p.runs {
                    el(s, "a:r", no_attrs, |s| {
                        if let Some(f) = &run.font {
                            font(s, "a:rPr", f);
                        }
                        text_el(s, "a:t", &run.text);
                    });
                }
                s.push_str("<a:endParaRPr/>");
            });
        }
    });
}

fn font(s: &mut String, tag: &str, f: &ChartFont) {
    let attrs = |s: &mut String| {
        attr(s, "sz", f.size);
        attr(s, "b", f.bold.map(b));
        attr(s, "i", f.italic.map(b));
        attr(s, "u", f.underline.as_deref().map(esc));
        attr(s, "strike", f.strike.as_deref().map(esc));
        attr(s, "baseline", f.baseline);
    };
    el(s, tag, attrs, |s| {
        if let Some(x) = &f.fill {
            fill(s, x);
        }
        for (t, v) in [
            ("a:latin", &f.latin),
            ("a:ea", &f.ea),
            ("a:cs", &f.cs),
            ("a:sym", &f.sym),
        ] {
            if let Some(v) = v {
                let _ = write!(s, r#"<{t} typeface="{}"/>"#, esc(v));
            }
        }
    });
}

/// `<tag 속성>내용</tag>`, 내용이 없으면 `<tag 속성/>` (한셀은 빈 요소를 닫아 쓴다)
fn el(s: &mut String, tag: &str, attrs: impl FnOnce(&mut String), body: impl FnOnce(&mut String)) {
    let _ = write!(s, "<{tag}");
    attrs(s);
    s.push('>');
    let start = s.len();
    body(s);
    if s.len() == start {
        s.pop();
        s.push_str("/>");
    } else {
        let _ = write!(s, "</{tag}>");
    }
}

fn no_attrs(_: &mut String) {}

fn b(v: bool) -> &'static str {
    if v { "1" } else { "0" }
}

/// 실수 값 (`c:max` 등): 정수면 소수점 없이
fn num(v: f64) -> String {
    format!("{v}")
}

/// 배치 비율: 한셀은 f32 로 들고 소수 20자리로 쓴다 (관측: 0.30000001192092895508 = f32 0.3)
fn ratio(v: f64) -> String {
    format!("{:.20}", v as f32 as f64)
}

/// 글 요소. 빈 글은 `<tag/>` (한셀: 빈 칸의 캐시 값 `<c:v/>`)
fn text_el(s: &mut String, tag: &str, text: &str) {
    el(s, tag, no_attrs, |s| s.push_str(&esc(text)));
}

/// `<tag val="…"/>` (값이 있을 때만)
fn val<T: std::fmt::Display>(s: &mut String, tag: &str, v: Option<T>) {
    if let Some(v) = v {
        let _ = write!(s, r#"<{tag} val="{v}"/>"#);
    }
}

fn val_str(s: &mut String, tag: &str, v: &Option<String>) {
    val(s, tag, v.as_deref().map(esc));
}

fn val_bool(s: &mut String, tag: &str, v: Option<bool>) {
    val(s, tag, v.map(b));
}

/// ` name="…"` (값이 있을 때만)
fn attr<T: std::fmt::Display>(s: &mut String, name: &str, v: Option<T>) {
    if let Some(v) = v {
        let _ = write!(s, r#" {name}="{v}""#);
    }
}
