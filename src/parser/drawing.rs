// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그리기 파트 (`xl/drawings/drawingN.xml`, DrawingML SpreadsheetDrawing).
//!
//! XML `.cell` 과 바이너리 `.cell` 이 같은 XML 그리기 파트를 쓴다 (관측). 앵커 요소마다
//! [`Shape`] 하나를 만들고, 저장 때 그대로 다시 쓸 수 있도록 원래 XML 도 보관한다.

use crate::model::{
    Anchor, AnchorKind, DrawColor, DrawingPara, DrawingRun, Marker, Outline, Paint, Shape,
    ShapeKind, TextBody, Xfrm,
};
use crate::package::Relationship;
use crate::xml::Element;

pub const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
pub const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// `xdr:wsDr` → 그리기 개체 목록
pub fn parse_drawing(root: &Element, rels: &[Relationship]) -> Vec<Shape> {
    root.elements()
        .filter_map(|e| {
            let kind = match e.name.as_str() {
                "twoCellAnchor" => AnchorKind::TwoCell,
                "oneCellAnchor" => AnchorKind::OneCell,
                "absoluteAnchor" => AnchorKind::Absolute,
                _ => return None,
            };
            if e.ns.as_deref() != Some(XDR) {
                return None;
            }
            let anchor = Anchor {
                kind,
                edit_as: e.attr("editAs").map(str::to_string),
                from: e.child_ns(XDR, "from").map(marker).unwrap_or_default(),
                to: e.child_ns(XDR, "to").map(marker),
                ext: e.child_ns(XDR, "ext").map(|x| (num(x, "cx"), num(x, "cy"))),
                pos: e.child_ns(XDR, "pos").map(|x| (num(x, "x"), num(x, "y"))),
            };
            let obj = e.elements().find(|c| {
                c.ns.as_deref() == Some(XDR)
                    && matches!(
                        c.name.as_str(),
                        "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp"
                    )
            })?;
            let mut shape = object(obj, rels);
            shape.anchor = anchor;
            shape.raw = Some(e.to_xml());
            for id in rel_ids(e) {
                if let Some(r) = rels.iter().find(|r| r.id == id)
                    && !shape.raw_rels.iter().any(|x| x.0 == id)
                {
                    shape
                        .raw_rels
                        .push((id, r.rel_type.clone(), r.target.clone(), r.external));
                }
            }
            Some(shape)
        })
        .collect()
}

fn num(e: &Element, name: &str) -> i64 {
    e.attr(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn child_num(e: &Element, name: &str) -> i64 {
    e.child_ns(XDR, name)
        .and_then(|x| x.text().trim().parse().ok())
        .unwrap_or(0)
}

fn marker(e: &Element) -> Marker {
    Marker {
        col: child_num(e, "col").max(0) as u32,
        col_off: child_num(e, "colOff"),
        row: child_num(e, "row").max(0) as u32,
        row_off: child_num(e, "rowOff"),
    }
}

/// 하위 트리의 관계 ID (`r:id`, `r:embed`, `r:link`)
fn rel_ids(e: &Element) -> Vec<String> {
    let mut out = Vec::new();
    e.walk(&mut |x| {
        for a in &x.attrs {
            if a.ns.as_deref().is_some_and(crate::ns::is_rel)
                && matches!(a.name.as_str(), "id" | "embed" | "link")
            {
                out.push(a.value.clone());
            }
        }
    });
    out
}

/// sp / cxnSp / pic / graphicFrame / grpSp
fn object(e: &Element, rels: &[Relationship]) -> Shape {
    let mut s = Shape {
        kind: match e.name.as_str() {
            "cxnSp" => ShapeKind::Connector,
            "pic" => ShapeKind::Picture,
            "grpSp" => ShapeKind::Group,
            "graphicFrame" => ShapeKind::Other,
            _ => ShapeKind::Shape,
        },
        ..Default::default()
    };
    // nvSpPr / nvCxnSpPr / nvPicPr / nvGraphicFramePr / nvGrpSpPr → cNvPr
    if let Some(nv) = e
        .elements()
        .find(|c| c.ns.as_deref() == Some(XDR) && c.name.starts_with("nv"))
        && let Some(c) = nv.child_ns(XDR, "cNvPr")
    {
        s.id = c.attr_u32("id").unwrap_or(0);
        s.name = c.attr("name").unwrap_or("").to_string();
        s.descr = c.attr("descr").map(str::to_string);
        s.hidden = c.attr_bool("hidden").unwrap_or(false);
    }
    let sp_pr = e
        .child_ns(XDR, "spPr")
        .or_else(|| e.child_ns(XDR, "grpSpPr"));
    if let Some(p) = sp_pr {
        s.xfrm = p.child_ns(A, "xfrm").map(xfrm);
        if let Some(g) = p.child_ns(A, "prstGeom") {
            s.geometry = g.attr("prst").map(str::to_string);
        } else if let Some(g) = p.child_ns(A, "custGeom") {
            // 경로가 하나도 없는 사용자 정의 모양은 그려지지 않는다 → 모양 없음과 같다
            let has_path = g.child_ns(A, "pathLst").is_some_and(|l| {
                l.elements()
                    .any(|p| p.is(A, "path") && p.elements().next().is_some())
            });
            s.geometry = has_path.then(|| "custom".into());
        }
        s.fill = paint(p);
        s.line = p.child_ns(A, "ln").map(|ln| Outline {
            width: ln.attr("w").and_then(|v| v.parse().ok()),
            fill: paint(ln),
            dash: ln
                .child_ns(A, "prstDash")
                .and_then(|d| d.attr("val"))
                .map(str::to_string),
        });
    }
    if s.kind == ShapeKind::Other {
        // graphicFrame: xdr:xfrm + a:graphic/a:graphicData (차트면 c:chart r:id)
        s.xfrm = e.child_ns(XDR, "xfrm").map(xfrm);
        let mut chart = None;
        e.walk(&mut |x| {
            if x.is(C, "chart") {
                chart = x.rel_id().map(str::to_string);
            }
        });
        if let Some(id) = chart {
            s.kind = ShapeKind::Chart;
            s.target = rels.iter().find(|r| r.id == id).map(|r| r.target.clone());
        }
    }
    if s.kind == ShapeKind::Picture {
        let mut embed = None;
        e.walk(&mut |x| {
            if x.is(A, "blip") {
                embed = x
                    .attrs
                    .iter()
                    .find(|a| a.name == "embed" && a.ns.as_deref().is_some_and(crate::ns::is_rel))
                    .map(|a| a.value.clone());
            }
        });
        s.target = embed.and_then(|id| rels.iter().find(|r| r.id == id).map(|r| r.target.clone()));
    }
    s.text = e.child_ns(XDR, "txBody").map(text_body);
    if s.kind == ShapeKind::Group {
        s.children = e
            .elements()
            .filter(|c| {
                c.ns.as_deref() == Some(XDR)
                    && matches!(
                        c.name.as_str(),
                        "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp"
                    )
            })
            .map(|c| object(c, rels))
            .collect();
    }
    s
}

fn xfrm(x: &Element) -> Xfrm {
    let off = x.child_ns(A, "off");
    let ext = x.child_ns(A, "ext");
    Xfrm {
        x: off.map_or(0, |o| num(o, "x")),
        y: off.map_or(0, |o| num(o, "y")),
        cx: ext.map_or(0, |o| num(o, "cx")),
        cy: ext.map_or(0, |o| num(o, "cy")),
        rot: num(x, "rot"),
        flip_h: x.attr_bool("flipH").unwrap_or(false),
        flip_v: x.attr_bool("flipV").unwrap_or(false),
    }
}

/// 채우기 요소 (noFill / solidFill / gradFill / pattFill / blipFill)
fn paint(p: &Element) -> Option<Paint> {
    for c in p.elements().filter(|c| c.ns.as_deref() == Some(A)) {
        match c.name.as_str() {
            "noFill" => return Some(Paint::None),
            "solidFill" => {
                return Some(Paint::Solid(
                    c.elements().next().map(color).unwrap_or_default(),
                ));
            }
            "gradFill" | "pattFill" | "blipFill" | "grpFill" => {
                return Some(Paint::Other(c.name.clone()));
            }
            _ => {}
        }
    }
    None
}

/// srgbClr / schemeClr / sysClr / prstClr (+ 변형)
pub fn color(c: &Element) -> DrawColor {
    let val = c.attr("val").unwrap_or("");
    let value = match c.name.as_str() {
        "srgbClr" => format!("srgb:{}", val.to_ascii_uppercase()),
        "schemeClr" => format!("scheme:{val}"),
        "sysClr" => format!("sys:{val}"),
        "prstClr" => format!("prst:{val}"),
        other => format!("{other}:{val}"),
    };
    DrawColor {
        value,
        mods: c
            .elements()
            .map(|m| {
                (
                    m.name.clone(),
                    m.attr("val").and_then(|v| v.parse().ok()).unwrap_or(0),
                )
            })
            .collect(),
    }
}

fn text_body(t: &Element) -> TextBody {
    let mut body = TextBody::default();
    if let Some(b) = t.child_ns(A, "bodyPr") {
        body.anchor = b.attr("anchor").map(str::to_string);
        body.wrap = b.attr("wrap").map(str::to_string);
        body.vert = b.attr("vert").map(str::to_string);
        for (i, n) in ["lIns", "tIns", "rIns", "bIns"].iter().enumerate() {
            body.insets[i] = b.attr(n).and_then(|v| v.parse().ok());
        }
    }
    for p in t.elements().filter(|c| c.is(A, "p")) {
        let mut para = DrawingPara {
            align: p
                .child_ns(A, "pPr")
                .and_then(|pp| pp.attr("algn"))
                .map(str::to_string),
            runs: Vec::new(),
        };
        for r in p.elements() {
            if r.ns.as_deref() != Some(A) {
                continue;
            }
            let text = match r.name.as_str() {
                "r" | "fld" => r.child_ns(A, "t").map(|t| t.text()).unwrap_or_default(),
                "br" => "\n".to_string(),
                _ => continue,
            };
            let mut run = DrawingRun {
                text,
                ..Default::default()
            };
            if let Some(rp) = r.child_ns(A, "rPr") {
                run.size = rp.attr_u32("sz");
                run.bold = rp.attr_bool("b");
                run.italic = rp.attr_bool("i");
                run.underline = rp.attr("u").map(str::to_string);
                run.color = rp
                    .child_ns(A, "solidFill")
                    .and_then(|f| f.elements().next())
                    .map(color);
                run.latin = rp
                    .child_ns(A, "latin")
                    .and_then(|l| l.attr("typeface"))
                    .map(str::to_string);
                run.ea = rp
                    .child_ns(A, "ea")
                    .and_then(|l| l.attr("typeface"))
                    .map(str::to_string);
            }
            para.runs.push(run);
        }
        body.paragraphs.push(para);
    }
    body
}
