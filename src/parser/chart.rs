// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 차트 파트 (`xl/charts/chartN.xml`, DrawingML Chart) → [`Chart`].
//!
//! 요약 필드(종류·묶음 방식·제목·범례·계열의 참조와 값)는 뷰어와 비교에 쓰고, 나머지 필드는
//! 차트 파트를 한셀과 같은 꼴로 다시 쓰는 데 쓴다 ([`crate::chart_xml`]). 한셀은 읽은 요소의
//! 있고 없음을 그대로 다시 쓰므로 여기서도 요소마다 `None`/`Some` 을 지킨다.
//!
//! `mc:AlternateContent` 를 펼치지 않은 트리를 받아야 `c14:style` 을 읽는다 (펼친 트리를 받으면
//! `mc:Fallback` 의 `c:style` 만 남는다).

use crate::model::{
    Chart, ChartAxis, ChartCache, ChartColor, ChartData, ChartDataLabels, ChartDataPoint,
    ChartFill, ChartFont, ChartGroup, ChartGroupProps, ChartLayout, ChartLegendProps, ChartLine,
    ChartLineEnd, ChartLines, ChartManualLayout, ChartMarker, ChartNumFmt, ChartPara, ChartPoint,
    ChartRich, ChartRun, ChartScaling, ChartSeries, ChartShapeProps, ChartSurface, ChartTitle,
    ChartTx, ChartView3D, HncChartStyle,
};
use crate::ns;
use crate::parser::drawing::{A, C, color};
use crate::xml::Element;

/// `c14:` (Office 2010 차트 확장)
pub const C14: &str = "http://schemas.microsoft.com/office/drawing/2007/8/2/chart";

/// `c:chartSpace` → 차트
pub fn parse_chart(root: &Element) -> Chart {
    let mut ch = Chart::default();
    // c14:style 은 mc:Choice 안에 있다 — 펼치기 전에 읽는다
    for ac in root.elements().filter(|e| e.is(ns::MC, "AlternateContent")) {
        for b in ac.elements().filter(|b| b.is(ns::MC, "Choice")) {
            if let Some(s) = b.child_ns(C14, "style") {
                ch.c14_style = s.attr_u32("val");
            }
        }
    }
    let mut has_ac = false;
    root.walk(&mut |e| has_ac |= e.is(ns::MC, "AlternateContent"));
    let resolved = has_ac.then(|| {
        let mut r = root.clone();
        crate::xml::resolve_alternate_content(&mut r);
        r
    });
    let root = resolved.as_ref().unwrap_or(root);
    ch.date1904 = val_bool(root, "date1904");
    ch.rounded_corners = val_bool(root, "roundedCorners");
    ch.style = val_u32(root, "style");
    ch.tx_pr = root.child_ns(C, "txPr").map(rich);
    ch.sp_pr = root.child_ns(C, "spPr").map(shape_props);
    ch.hnc_chart_style = root
        .child_ns(C, "extLst")
        .into_iter()
        .flat_map(|l| l.elements())
        .flat_map(|x| x.elements())
        .find(|s| s.is(ns::HANCOM_OFFICE_8, "hncChartStyle"))
        .map(|s| HncChartStyle {
            layout_index: attr_i32(s, "layoutIndex"),
            color_index: attr_i32(s, "colorIndex"),
            style_index: attr_i32(s, "styleIndex"),
        });

    let Some(chart) = root.child_ns(C, "chart") else {
        return ch;
    };
    let deleted = chart
        .child_ns(C, "autoTitleDeleted")
        .and_then(|d| d.attr_bool("val"))
        .unwrap_or(false);
    ch.title = chart
        .child_ns(C, "title")
        .and_then(title_text)
        .filter(|_| !deleted);
    ch.title_full = chart.child_ns(C, "title").map(title);
    ch.auto_title_deleted = val_bool(chart, "autoTitleDeleted");
    ch.view3d = chart.child_ns(C, "view3D").map(|v| ChartView3D {
        r_ang_ax: val_bool(v, "rAngAx"),
        rot_x: val_str(v, "rotX").and_then(|x| x.parse().ok()),
        h_percent: val_u32(v, "hPercent"),
        rot_y: val_u32(v, "rotY"),
        perspective: val_u32(v, "perspective"),
        depth_percent: val_u32(v, "depthPercent"),
    });
    let surface = |name: &str| {
        chart.child_ns(C, name).map(|s| ChartSurface {
            thickness: val_u32(s, "thickness"),
            sp_pr: s.child_ns(C, "spPr").map(shape_props),
        })
    };
    ch.floor = surface("floor");
    ch.side_wall = surface("sideWall");
    ch.back_wall = surface("backWall");
    ch.legend = chart.child_ns(C, "legend").map(|l| {
        l.child_ns(C, "legendPos")
            .and_then(|p| p.attr("val"))
            .unwrap_or("r")
            .to_string()
    });
    if let Some(l) = chart.child_ns(C, "legend") {
        ch.legend_props = ChartLegendProps {
            layout: l.child_ns(C, "layout").map(layout),
            overlay: val_bool(l, "overlay"),
            sp_pr: l.child_ns(C, "spPr").map(shape_props),
            tx_pr: l.child_ns(C, "txPr").map(rich),
        };
    }
    ch.plot_vis_only = val_bool(chart, "plotVisOnly");
    ch.disp_blanks_as = val_str(chart, "dispBlanksAs");
    let Some(plot) = chart.child_ns(C, "plotArea") else {
        return ch;
    };
    ch.plot_layout = plot.child_ns(C, "layout").map(layout);
    ch.plot_sp_pr = plot.child_ns(C, "spPr").map(shape_props);
    let mut groups = plot
        .elements()
        .filter(|e| e.ns.as_deref() == Some(C) && e.name.ends_with("Chart"));
    if let Some(t) = groups.next() {
        let g = group(t);
        ch.kind = g.kind;
        ch.three_d = g.three_d;
        ch.bar_dir = g.bar_dir;
        ch.grouping = g.grouping;
        ch.vary_colors = g.vary_colors;
        ch.group = g.props;
        ch.series = g.series;
    }
    ch.more_groups = groups.map(group).collect();
    ch.axes = plot
        .elements()
        .filter(|e| {
            e.ns.as_deref() == Some(C)
                && matches!(e.name.as_str(), "catAx" | "valAx" | "serAx" | "dateAx")
        })
        .map(axis)
        .collect();
    let axis_title = |name: &str| {
        plot.elements()
            .find(|e| e.is(C, name))
            .and_then(|a| a.child_ns(C, "title"))
            .and_then(title_text)
    };
    ch.cat_axis_title = axis_title("catAx").or_else(|| axis_title("dateAx"));
    ch.val_axis_title = axis_title("valAx");
    ch
}

/// 그림 영역의 차트 하나 (`c:barChart` …)
fn group(t: &Element) -> ChartGroup {
    let base = t.name.trim_end_matches("Chart");
    let mut kind = base.trim_end_matches("3D").to_string();
    if kind == "ofPie" {
        kind = "pie".into();
    }
    let val = |name: &str| val_str(t, name);
    let lines = |name: &str| t.child_ns(C, name).map(chart_lines);
    ChartGroup {
        kind,
        three_d: base.ends_with("3D"),
        bar_dir: val("barDir"),
        grouping: val("grouping"),
        vary_colors: t
            .child_ns(C, "varyColors")
            .and_then(|e| e.attr_bool("val"))
            .unwrap_or(false),
        props: ChartGroupProps {
            radar_style: val("radarStyle"),
            scatter_style: val("scatterStyle"),
            of_pie_type: val("ofPieType"),
            wireframe: val_bool(t, "wireframe"),
            d_lbls: t.child_ns(C, "dLbls").map(data_labels),
            gap_width: val_u32(t, "gapWidth"),
            overlap: val("overlap").and_then(|v| v.trim_end_matches('%').parse().ok()),
            split_type: val("splitType"),
            split_pos: val_f64(t, "splitPos"),
            second_pie_size: val_u32(t, "secondPieSize"),
            ser_lines: lines("serLines"),
            drop_lines: lines("dropLines"),
            hi_low_lines: lines("hiLowLines"),
            gap_depth: val_u32(t, "gapDepth"),
            shape: val("shape"),
            marker: val_bool(t, "marker"),
            smooth: val_bool(t, "smooth"),
            first_slice_ang: val_u32(t, "firstSliceAng"),
            hole_size: val_u32(t, "holeSize"),
            bubble3d: val_bool(t, "bubble3D"),
            bubble_scale: val_u32(t, "bubbleScale"),
            show_neg_bubbles: val_bool(t, "showNegBubbles"),
            size_represents: val("sizeRepresents"),
            ax_ids: t
                .elements()
                .filter(|e| e.is(C, "axId"))
                .filter_map(|e| e.attr_u32("val"))
                .collect(),
        },
        series: t
            .elements()
            .filter(|e| e.is(C, "ser"))
            .map(series)
            .collect(),
    }
}

fn series(s: &Element) -> ChartSeries {
    let mut out = ChartSeries::default();
    if let Some(tx) = s.child_ns(C, "tx") {
        if let Some(r) = tx.child_ns(C, "strRef") {
            out.name_ref = r.child_ns(C, "f").map(|f| f.text());
            out.name = cache(r).into_iter().flatten().next();
            out.tx = Some(ChartTx::Ref(data(r)));
        } else if let Some(v) = tx.child_ns(C, "v") {
            out.name = Some(v.text());
            out.tx = Some(ChartTx::Value(v.text()));
        }
    }
    // 막대·선은 cat/val, 분산형은 xVal/yVal
    let cat = s.child_ns(C, "cat").or_else(|| s.child_ns(C, "xVal"));
    if let Some(c) = cat
        && let Some(r) = c.elements().next()
    {
        out.cat_ref = r.child_ns(C, "f").map(|f| f.text());
        out.categories = cache(r)
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect();
        out.cat = Some(data(r));
    }
    let val = s.child_ns(C, "val").or_else(|| s.child_ns(C, "yVal"));
    if let Some(v) = val
        && let Some(r) = v.elements().next()
    {
        out.val_ref = r.child_ns(C, "f").map(|f| f.text());
        out.values = cache(r)
            .into_iter()
            .map(|x| x.and_then(|t| t.trim().parse().ok()))
            .collect();
        out.val = Some(data(r));
    }
    out.color = s
        .child_ns(C, "spPr")
        .and_then(|p| p.child_ns(A, "solidFill"))
        .and_then(|f| f.elements().next())
        .map(color);
    out.idx = val_u32(s, "idx");
    out.order = val_u32(s, "order");
    out.sp_pr = s.child_ns(C, "spPr").map(shape_props);
    out.invert_if_negative = val_bool(s, "invertIfNegative");
    out.marker = s.child_ns(C, "marker").map(marker);
    out.explosion = val_u32(s, "explosion");
    out.d_pts = s
        .elements()
        .filter(|e| e.is(C, "dPt"))
        .map(|p| ChartDataPoint {
            idx: val_u32(p, "idx").unwrap_or(0),
            invert_if_negative: val_bool(p, "invertIfNegative"),
            marker: p.child_ns(C, "marker").map(marker),
            bubble3d: val_bool(p, "bubble3D"),
            explosion: val_u32(p, "explosion"),
            sp_pr: p.child_ns(C, "spPr").map(shape_props),
        })
        .collect();
    out.d_lbls = s.child_ns(C, "dLbls").map(data_labels);
    out.bubble_size = s
        .child_ns(C, "bubbleSize")
        .and_then(|b| b.elements().next())
        .map(data);
    out.shape = val_str(s, "shape");
    out.bubble3d = val_bool(s, "bubble3D");
    out.smooth = val_bool(s, "smooth");
    out
}

/// strRef/numRef/strLit/numLit 의 저장된 점 (`ptCount` 만큼, 없는 점은 None)
fn cache(r: &Element) -> Vec<Option<String>> {
    let c = r
        .child_ns(C, "strCache")
        .or_else(|| r.child_ns(C, "numCache"))
        .or_else(|| (r.is(C, "strLit") || r.is(C, "numLit")).then_some(r));
    let Some(c) = c else {
        return Vec::new();
    };
    let count = c
        .child_ns(C, "ptCount")
        .and_then(|p| p.attr_u32("val"))
        .unwrap_or(0) as usize;
    let mut v = vec![None; count];
    for p in c.elements().filter(|e| e.is(C, "pt")) {
        let i = p.attr_u32("idx").unwrap_or(0) as usize;
        if i >= v.len() {
            v.resize(i + 1, None);
        }
        v[i] = p.child_ns(C, "v").map(|x| x.text());
    }
    v
}

/// c:title → 글 (서식 있는 글이면 이어 붙이고, 참조면 저장된 값)
fn title_text(t: &Element) -> Option<String> {
    let tx = t.child_ns(C, "tx")?;
    if let Some(rich) = tx.child_ns(C, "rich") {
        let paras: Vec<String> = rich
            .elements()
            .filter(|p| p.is(A, "p"))
            .map(|p| {
                p.elements()
                    .filter(|r| r.is(A, "r") || r.is(A, "fld"))
                    .filter_map(|r| r.child_ns(A, "t").map(|x| x.text()))
                    .collect::<String>()
            })
            .collect();
        let s = paras.join("\n");
        return (!s.is_empty()).then_some(s);
    }
    tx.child_ns(C, "strRef")
        .and_then(|r| cache(r).into_iter().flatten().next())
}

/// 데이터 원본 (`c:numRef` …) 그대로
fn data(r: &Element) -> ChartData {
    let cache_el = match r.name.as_str() {
        "numRef" => r.child_ns(C, "numCache"),
        "strRef" => r.child_ns(C, "strCache"),
        "multiLvlStrRef" => r.child_ns(C, "multiLvlStrCache"),
        _ => Some(r),
    };
    ChartData {
        kind: r.name.clone(),
        formula: r.child_ns(C, "f").map(|f| f.text()),
        cache: cache_el.map(|c| ChartCache {
            format_code: c.child_ns(C, "formatCode").map(|f| f.text()),
            pt_count: val_u32(c, "ptCount"),
            points: c
                .elements()
                .filter(|e| e.is(C, "pt"))
                .map(|p| ChartPoint {
                    idx: p.attr_u32("idx").unwrap_or(0),
                    format_code: p.attr("formatCode").map(str::to_string),
                    value: p.child_ns(C, "v").map(|v| v.text()).unwrap_or_default(),
                })
                .collect(),
        }),
    }
}

fn title(t: &Element) -> ChartTitle {
    let tx = t.child_ns(C, "tx").and_then(|tx| {
        if let Some(r) = tx.child_ns(C, "rich") {
            Some(ChartTx::Rich(rich(r)))
        } else if let Some(r) = tx.child_ns(C, "strRef") {
            Some(ChartTx::Ref(data(r)))
        } else {
            tx.child_ns(C, "v").map(|v| ChartTx::Value(v.text()))
        }
    });
    ChartTitle {
        tx,
        layout: t.child_ns(C, "layout").map(layout),
        overlay: val_bool(t, "overlay"),
        sp_pr: t.child_ns(C, "spPr").map(shape_props),
        tx_pr: t.child_ns(C, "txPr").map(rich),
    }
}

fn layout(l: &Element) -> ChartLayout {
    ChartLayout {
        manual: l.child_ns(C, "manualLayout").map(|m| ChartManualLayout {
            layout_target: val_str(m, "layoutTarget"),
            x_mode: val_str(m, "xMode"),
            y_mode: val_str(m, "yMode"),
            w_mode: val_str(m, "wMode"),
            h_mode: val_str(m, "hMode"),
            x: val_f64(m, "x"),
            y: val_f64(m, "y"),
            w: val_f64(m, "w"),
            h: val_f64(m, "h"),
        }),
    }
}

fn axis(a: &Element) -> ChartAxis {
    let lines = |name: &str| a.child_ns(C, name).map(chart_lines);
    ChartAxis {
        kind: a.name.clone(),
        id: val_u32(a, "axId").unwrap_or(0),
        scaling: a
            .child_ns(C, "scaling")
            .map(|s| ChartScaling {
                log_base: val_f64(s, "logBase"),
                orientation: val_str(s, "orientation"),
                max: val_f64(s, "max"),
                min: val_f64(s, "min"),
            })
            .unwrap_or_default(),
        delete: val_bool(a, "delete"),
        ax_pos: val_str(a, "axPos"),
        major_gridlines: lines("majorGridlines"),
        minor_gridlines: lines("minorGridlines"),
        title: a.child_ns(C, "title").map(title),
        num_fmt: a.child_ns(C, "numFmt").map(num_fmt),
        major_tick_mark: val_str(a, "majorTickMark"),
        minor_tick_mark: val_str(a, "minorTickMark"),
        tick_lbl_pos: val_str(a, "tickLblPos"),
        sp_pr: a.child_ns(C, "spPr").map(shape_props),
        tx_pr: a.child_ns(C, "txPr").map(rich),
        cross_ax: val_u32(a, "crossAx"),
        crosses: val_str(a, "crosses"),
        crosses_at: val_f64(a, "crossesAt"),
        cross_between: val_str(a, "crossBetween"),
        auto: val_bool(a, "auto"),
        lbl_algn: val_str(a, "lblAlgn"),
        lbl_offset: val_str(a, "lblOffset").and_then(|v| v.trim_end_matches('%').parse().ok()),
        base_time_unit: val_str(a, "baseTimeUnit"),
        tick_lbl_skip: val_u32(a, "tickLblSkip"),
        tick_mark_skip: val_u32(a, "tickMarkSkip"),
        no_multi_lvl_lbl: val_bool(a, "noMultiLvlLbl"),
        major_unit: val_f64(a, "majorUnit"),
        major_time_unit: val_str(a, "majorTimeUnit"),
        minor_unit: val_f64(a, "minorUnit"),
        minor_time_unit: val_str(a, "minorTimeUnit"),
    }
}

fn num_fmt(n: &Element) -> ChartNumFmt {
    ChartNumFmt {
        format_code: n.attr("formatCode").unwrap_or("").to_string(),
        source_linked: n.attr_bool("sourceLinked"),
    }
}

fn chart_lines(e: &Element) -> ChartLines {
    ChartLines {
        sp_pr: e.child_ns(C, "spPr").map(shape_props),
    }
}

fn marker(m: &Element) -> ChartMarker {
    ChartMarker {
        symbol: val_str(m, "symbol"),
        size: val_u32(m, "size"),
        sp_pr: m.child_ns(C, "spPr").map(shape_props),
    }
}

fn data_labels(d: &Element) -> ChartDataLabels {
    ChartDataLabels {
        delete: val_bool(d, "delete"),
        num_fmt: d.child_ns(C, "numFmt").map(num_fmt),
        sp_pr: d.child_ns(C, "spPr").map(shape_props),
        tx_pr: d.child_ns(C, "txPr").map(rich),
        d_lbl_pos: val_str(d, "dLblPos"),
        show_legend_key: val_bool(d, "showLegendKey"),
        show_val: val_bool(d, "showVal"),
        show_cat_name: val_bool(d, "showCatName"),
        show_ser_name: val_bool(d, "showSerName"),
        show_percent: val_bool(d, "showPercent"),
        show_bubble_size: val_bool(d, "showBubbleSize"),
        separator: d.child_ns(C, "separator").map(|s| s.text()),
        show_leader_lines: val_bool(d, "showLeaderLines"),
    }
}

/// `c:rich` / `c:txPr`
fn rich(r: &Element) -> ChartRich {
    let body = r.child_ns(A, "bodyPr");
    ChartRich {
        rot: body
            .and_then(|b| b.attr("rot"))
            .and_then(|v| v.parse().ok()),
        vert: body.and_then(|b| b.attr("vert")).map(str::to_string),
        paras: r
            .elements()
            .filter(|p| p.is(A, "p"))
            .map(|p| {
                let ppr = p.child_ns(A, "pPr");
                ChartPara {
                    align: ppr.and_then(|x| x.attr("algn")).map(str::to_string),
                    def_font: ppr.and_then(|x| x.child_ns(A, "defRPr")).map(font),
                    runs: p
                        .elements()
                        .filter(|e| e.is(A, "r"))
                        .map(|e| ChartRun {
                            font: e.child_ns(A, "rPr").map(font),
                            text: e.child_ns(A, "t").map(|t| t.text()).unwrap_or_default(),
                        })
                        .collect(),
                }
            })
            .collect(),
    }
}

/// `a:defRPr` / `a:rPr`
fn font(f: &Element) -> ChartFont {
    let face = |name: &str| {
        f.child_ns(A, name)
            .and_then(|x| x.attr("typeface"))
            .map(str::to_string)
    };
    ChartFont {
        size: f.attr_u32("sz"),
        bold: f.attr_bool("b"),
        italic: f.attr_bool("i"),
        underline: f.attr("u").map(str::to_string),
        strike: f.attr("strike").map(str::to_string),
        baseline: attr_i32(f, "baseline"),
        fill: fill(f),
        latin: face("latin"),
        ea: face("ea"),
        cs: face("cs"),
        sym: face("sym"),
    }
}

/// `c:spPr`
fn shape_props(p: &Element) -> ChartShapeProps {
    ChartShapeProps {
        fill: fill(p),
        line: p.child_ns(A, "ln").map(line),
    }
}

/// 자식 가운데 첫 채우기 요소
fn fill(p: &Element) -> Option<ChartFill> {
    p.elements().find_map(|c| {
        if c.ns.as_deref() != Some(A) {
            return None;
        }
        match c.name.as_str() {
            "noFill" => Some(ChartFill::None),
            "solidFill" => Some(ChartFill::Solid(
                c.elements().next().map(chart_color).unwrap_or_default(),
            )),
            "gradFill" | "pattFill" | "blipFill" | "grpFill" => Some(ChartFill::Other(c.to_xml())),
            _ => None,
        }
    })
}

fn chart_color(c: &Element) -> ChartColor {
    let d = color(c);
    ChartColor {
        value: d.value,
        last_clr: c.attr("lastClr").map(str::to_string),
        mods: d.mods,
    }
}

fn line(l: &Element) -> ChartLine {
    let end = |name: &str| {
        l.child_ns(A, name).map(|e| ChartLineEnd {
            kind: e.attr("type").map(str::to_string),
            w: e.attr("w").map(str::to_string),
            len: e.attr("len").map(str::to_string),
        })
    };
    ChartLine {
        width: l.attr("w").and_then(|v| v.trim().parse().ok()),
        cap: l.attr("cap").map(str::to_string),
        cmpd: l.attr("cmpd").map(str::to_string),
        algn: l.attr("algn").map(str::to_string),
        fill: fill(l),
        dash: l
            .child_ns(A, "prstDash")
            .and_then(|d| d.attr("val"))
            .map(str::to_string),
        join: l
            .elements()
            .find(|e| {
                e.ns.as_deref() == Some(A) && matches!(e.name.as_str(), "round" | "bevel" | "miter")
            })
            .map(|e| e.name.clone()),
        head_end: end("headEnd"),
        tail_end: end("tailEnd"),
    }
}

/// `<c:name val="…"/>` 의 값
fn val_str(e: &Element, name: &str) -> Option<String> {
    e.child_ns(C, name)
        .and_then(|x| x.attr("val"))
        .map(str::to_string)
}

fn val_u32(e: &Element, name: &str) -> Option<u32> {
    e.child_ns(C, name).and_then(|x| x.attr_u32("val"))
}

fn val_f64(e: &Element, name: &str) -> Option<f64> {
    e.child_ns(C, name).and_then(|x| x.attr_f64("val"))
}

/// `CT_Boolean`: 요소가 있고 `val` 이 없으면 참 (ECMA-376 기본값)
fn val_bool(e: &Element, name: &str) -> Option<bool> {
    e.child_ns(C, name)
        .map(|x| x.attr_bool("val").unwrap_or(true))
}

fn attr_i32(e: &Element, name: &str) -> Option<i32> {
    e.attr(name).and_then(|v| v.trim().parse().ok())
}
