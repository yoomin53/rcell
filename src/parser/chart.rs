// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 차트 파트 (`xl/charts/chartN.xml`, DrawingML Chart) → [`Chart`].
//!
//! 그림 영역의 첫 차트(종류·막대 방향·묶음 방식)와 계열(이름·항목·값의 참조와 저장된 값),
//! 제목·축 제목·범례만 읽는다. 원본 파트는 그대로 보존되므로 저장에는 쓰지 않는다.

use crate::model::{Chart, ChartSeries};
use crate::parser::drawing::{A, C, color};
use crate::xml::Element;

/// `c:chartSpace` → 차트
pub fn parse_chart(root: &Element) -> Chart {
    let mut ch = Chart::default();
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
    ch.legend = chart.child_ns(C, "legend").map(|l| {
        l.child_ns(C, "legendPos")
            .and_then(|p| p.attr("val"))
            .unwrap_or("r")
            .to_string()
    });
    let Some(plot) = chart.child_ns(C, "plotArea") else {
        return ch;
    };
    if let Some(t) = plot
        .elements()
        .find(|e| e.ns.as_deref() == Some(C) && e.name.ends_with("Chart"))
    {
        let base = t.name.trim_end_matches("Chart");
        ch.three_d = base.ends_with("3D");
        ch.kind = base.trim_end_matches("3D").to_string();
        if ch.kind == "ofPie" {
            ch.kind = "pie".into();
        }
        let val = |name: &str| {
            t.child_ns(C, name)
                .and_then(|e| e.attr("val"))
                .map(str::to_string)
        };
        ch.bar_dir = val("barDir");
        ch.grouping = val("grouping");
        ch.vary_colors = t
            .child_ns(C, "varyColors")
            .and_then(|e| e.attr_bool("val"))
            .unwrap_or(false);
        ch.series = t
            .elements()
            .filter(|e| e.is(C, "ser"))
            .map(series)
            .collect();
    }
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

fn series(s: &Element) -> ChartSeries {
    let mut out = ChartSeries::default();
    if let Some(tx) = s.child_ns(C, "tx") {
        if let Some(r) = tx.child_ns(C, "strRef") {
            out.name_ref = r.child_ns(C, "f").map(|f| f.text());
            out.name = cache(r).into_iter().flatten().next();
        } else if let Some(v) = tx.child_ns(C, "v") {
            out.name = Some(v.text());
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
    }
    out.color = s
        .child_ns(C, "spPr")
        .and_then(|p| p.child_ns(A, "solidFill"))
        .and_then(|f| f.elements().next())
        .map(color);
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
