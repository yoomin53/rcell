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

/// `xdr:wsDr` → 그리기 개체 목록. 모르는 개체(`xdr:contentPart` 등)를 담은 앵커와 `mc:AlternateContent` 로 감싼
/// 앵커도 버리지 않고 원문 그대로 보관한다 (모델은 분기 안의 첫 앵커로 채운다)
pub fn parse_drawing(root: &Element, rels: &[Relationship]) -> Vec<Shape> {
    root.elements()
        .filter_map(|e| {
            // mc:AlternateContent: 분기 안의 첫 앵커로 모델을, 원문은 감싼 요소째로
            let a = if e.is(crate::ns::MC, "AlternateContent") {
                e.elements()
                    .flat_map(|b| b.elements())
                    .find(|x| anchor_kind(x).is_some())?
            } else {
                e
            };
            let kind = anchor_kind(a)?;
            let anchor = Anchor {
                kind,
                edit_as: a.attr("editAs").map(str::to_string),
                from: a.child_ns(XDR, "from").map(marker).unwrap_or_default(),
                to: a.child_ns(XDR, "to").map(marker),
                ext: a.child_ns(XDR, "ext").map(|x| (num(x, "cx"), num(x, "cy"))),
                pos: a.child_ns(XDR, "pos").map(|x| (num(x, "x"), num(x, "y"))),
            };
            let obj = a.elements().find(|c| {
                c.ns.as_deref() == Some(XDR)
                    && matches!(
                        c.name.as_str(),
                        "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp"
                    )
            });
            let mut shape = obj.map(|o| object(o, rels)).unwrap_or_else(|| Shape {
                kind: ShapeKind::Other,
                ..Default::default()
            });
            shape.anchor = anchor;
            if let Some(cd) = a.child_ns(XDR, "clientData") {
                shape.locks_with_sheet = cd.attr_bool("fLocksWithSheet");
                shape.prints_with_sheet = cd.attr_bool("fPrintsWithSheet");
            }
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

fn anchor_kind(e: &Element) -> Option<AnchorKind> {
    if e.ns.as_deref() != Some(XDR) {
        return None;
    }
    Some(match e.name.as_str() {
        "twoCellAnchor" => AnchorKind::TwoCell,
        "oneCellAnchor" => AnchorKind::OneCell,
        "absoluteAnchor" => AnchorKind::Absolute,
        _ => return None,
    })
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
    // 잠금: cNvSpPr/a:spLocks, cNvCxnSpPr/a:cxnSpLocks, cNvPicPr/a:picLocks, cNvGrpSpPr/a:grpSpLocks
    if let Some(nv) = e
        .elements()
        .find(|c| c.ns.as_deref() == Some(XDR) && c.name.starts_with("nv"))
    {
        for c in nv
            .elements()
            .filter(|c| c.name.starts_with("cNv") && c.name != "cNvPr")
        {
            for l in c
                .elements()
                .filter(|l| l.ns.as_deref() == Some(A) && l.name.ends_with("Locks"))
            {
                s.locks.extend(
                    l.attrs
                        .iter()
                        .filter(|a| a.ns.is_none() && crate::xml::parse_bool(&a.value))
                        .map(|a| a.name.clone()),
                );
            }
        }
        s.locks.sort();
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
        s.line = p.child_ns(A, "ln").map(outline);
        s.shadow = p
            .child_ns(A, "effectLst")
            .and_then(|l| l.child_ns(A, "outerShdw"))
            .map(|o| crate::model::ShapeShadow {
                dist: num(o, "dist"),
                dir: num(o, "dir"),
                blur: num(o, "blurRad"),
                align: o.attr("algn").map(str::to_string),
                rot_with_shape: o.attr_bool("rotWithShape").unwrap_or(true),
                color: o.elements().next().map(color).unwrap_or_default(),
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

/// `a:ln`: 두께, 채우기, 점선, 끝 모양·겹선·이음, 화살표
fn outline(ln: &Element) -> Outline {
    let end = |n: &str| {
        ln.child_ns(A, n).map(|e| crate::model::LineEnd {
            kind: e.attr("type").map(str::to_string),
            w: e.attr("w").map(str::to_string),
            len: e.attr("len").map(str::to_string),
        })
    };
    Outline {
        width: ln.attr("w").and_then(|v| v.parse().ok()),
        fill: paint(ln),
        dash: ln
            .child_ns(A, "prstDash")
            .and_then(|d| d.attr("val"))
            .map(str::to_string),
        cap: ln.attr("cap").map(str::to_string),
        compound: ln.attr("cmpd").map(str::to_string),
        join: ln
            .elements()
            .find(|c| {
                c.ns.as_deref() == Some(A) && matches!(c.name.as_str(), "round" | "bevel" | "miter")
            })
            .map(|c| c.name.clone()),
        head: end("headEnd"),
        tail: end("tailEnd"),
    }
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
        body.vert_overflow = b.attr("vertOverflow").map(str::to_string);
        body.horz_overflow = b.attr("horzOverflow").map(str::to_string);
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
