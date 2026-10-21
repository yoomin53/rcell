// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 차트 파트 쓰기 검증.
//!
//! 한셀이 레거시 차트를 읽어 XML 로 쓴 차트 파트(`*.legacy.as.cell` 의 `xl/charts/chartN.xml`)를
//! 모델로 읽고 [`rcell::chart_xml::chart_xml`] 로 다시 써서 원본과 XML 트리로 비교한다
//! (요소 이름·순서, 속성, 글; 숫자는 값으로). 한셀 XML 차트(`hs/*.hs.cell`, 한셀이 저장한 `X.cell`)도
//! 같은 방법으로 돌려 보고 차이를 보고만 한다.
//!
//! ```text
//! RCELL_CORPUS=C:\rcell-corpus cargo test --release --test chart_xml -- --nocapture
//! ```

use std::path::{Path, PathBuf};

use rcell::model::{
    Chart, ChartAxis, ChartCache, ChartColor, ChartData, ChartFill, ChartFont, ChartLayout,
    ChartLine, ChartManualLayout, ChartPoint, ChartRich, ChartSeries, ChartShapeProps, ChartTitle,
    ChartTx,
};
use rcell::xml::{Element, Node};

fn files_with_suffix(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_lowercase().ends_with(suffix))
        })
        .collect();
    files.sort();
    files
}

/// 패키지의 차트 파트들: (파트 이름, XML 원문)
fn chart_parts(path: &Path) -> Vec<(String, String)> {
    let bytes = std::fs::read(path).expect("파일");
    let Ok(mut pkg) = rcell::Package::from_bytes(bytes) else {
        return Vec::new();
    };
    let mut names: Vec<String> = pkg
        .part_names()
        .iter()
        .filter(|n| {
            n.strip_prefix("xl/charts/chart")
                .and_then(|r| r.strip_suffix(".xml"))
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        })
        .cloned()
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|n| pkg.read_string(&n).ok().flatten().map(|s| (n, s)))
        .collect()
}

fn qname(e: &Element) -> String {
    format!(
        "{}{}",
        e.prefix
            .as_deref()
            .map_or(String::new(), |p| format!("{p}:")),
        e.name
    )
}

/// 같은 값인지: 글이 같거나, 둘 다 숫자면 값이 같다
fn same_value(a: &str, b: &str) -> bool {
    a == b
        || matches!(
            (a.trim().parse::<f64>(), b.trim().parse::<f64>()),
            (Ok(x), Ok(y)) if x == y
        )
}

fn own_text(e: &Element) -> String {
    e.children
        .iter()
        .filter_map(|n| match n {
            Node::Text(t) => Some(t.as_str()),
            Node::Element(_) => None,
        })
        .collect()
}

/// 두 트리의 차이 (경로와 내용). 네임스페이스 선언은 비교하지 않는다 (요소·속성은 URI 로 본다).
fn tree_diffs(a: &Element, b: &Element, path: &str, out: &mut Vec<String>) {
    let here = format!("{path}/{}", qname(a));
    if a.ns != b.ns || a.name != b.name {
        out.push(format!("{path}: 요소 {} ≠ {}", qname(a), qname(b)));
        return;
    }
    let key = |x: &rcell::xml::Attr| (x.ns.clone(), x.name.clone());
    for x in &a.attrs {
        match b.attrs.iter().find(|y| key(y) == key(x)) {
            None => out.push(format!(
                "{here}: 속성 {} 은 원본에 없다 (={:?})",
                x.name, x.value
            )),
            Some(y) if !same_value(&x.value, &y.value) => {
                out.push(format!("{here}/@{}: {:?} ≠ {:?}", x.name, x.value, y.value))
            }
            _ => {}
        }
    }
    for y in &b.attrs {
        if !a.attrs.iter().any(|x| key(x) == key(y)) {
            out.push(format!(
                "{here}: 속성 {} 이 빠졌다 (원본 {:?})",
                y.name, y.value
            ));
        }
    }
    let (ta, tb) = (own_text(a), own_text(b));
    if !same_value(&ta, &tb) {
        out.push(format!("{here}: 글 {ta:?} ≠ {tb:?}"));
    }
    let ca: Vec<&Element> = a.elements().collect();
    let cb: Vec<&Element> = b.elements().collect();
    if ca.len() != cb.len() {
        let names = |v: &[&Element]| v.iter().map(|e| qname(e)).collect::<Vec<_>>().join(" ");
        out.push(format!(
            "{here}: 자식 {}개 ≠ {}개\n    rcell: {}\n    원본:  {}",
            ca.len(),
            cb.len(),
            names(&ca),
            names(&cb)
        ));
    }
    for (x, y) in ca.iter().zip(cb.iter()) {
        tree_diffs(x, y, &here, out);
    }
}

/// 요약 필드를 지운다 (레거시 변환처럼 자세한 필드만 채운 모델)
fn clear_summary(c: &mut Chart) {
    c.title = None;
    c.cat_axis_title = None;
    c.val_axis_title = None;
    for s in c
        .series
        .iter_mut()
        .chain(c.more_groups.iter_mut().flat_map(|g| g.series.iter_mut()))
    {
        s.name = None;
        s.name_ref = None;
        s.cat_ref = None;
        s.val_ref = None;
        s.categories.clear();
        s.values.clear();
        s.color = None;
    }
}

fn summary_of(c: &Chart) -> String {
    format!(
        "{:?} {:?} {:?} {:?}",
        c.title,
        c.cat_axis_title,
        c.val_axis_title,
        c.series
            .iter()
            .map(|s| (
                &s.name,
                &s.name_ref,
                &s.cat_ref,
                &s.val_ref,
                &s.categories,
                &s.values,
                &s.color
            ))
            .collect::<Vec<_>>()
    )
}

/// 차트 파트 하나: 읽고 → 모델 → 다시 쓰고 → 비교. (차이, 바이트가 처음 다른 곳 — 같으면 None)
fn roundtrip(xml: &str) -> (Vec<String>, Option<String>) {
    let orig = rcell::xml::parse_raw(xml, "chart").expect("원본 XML");
    let chart = rcell::parser::chart::parse_chart(&orig);
    let out = rcell::chart_xml::chart_xml(&chart);
    let back = rcell::xml::parse_raw(&out, "rcell").expect("rcell 이 쓴 XML");
    let mut diffs = Vec::new();
    tree_diffs(&back, &orig, "", &mut diffs);
    // 요약 필드는 자세한 필드에서 다시 만들 수 있다 (파서와 같은 규칙)
    let mut again = chart.clone();
    clear_summary(&mut again);
    again.refresh_summary();
    if again != chart {
        diffs.push(format!(
            "refresh_summary 가 파서의 요약과 다르다:\n    파서: {:?}\n    다시: {:?}",
            summary_of(&chart),
            summary_of(&again)
        ));
    }
    let byte_diff = (out != xml).then(|| {
        let at = out
            .char_indices()
            .zip(xml.chars())
            .find(|((_, a), b)| a != b)
            .map_or(out.len().min(xml.len()), |((i, _), _)| i);
        let around = |s: &str| {
            let from = s[..at.min(s.len())]
                .char_indices()
                .rev()
                .nth(40)
                .map_or(0, |(i, _)| i);
            s[from..].chars().take(100).collect::<String>()
        };
        format!(
            "바이트 {at}:\n    rcell: {}\n    원본:  {}",
            around(&out),
            around(xml)
        )
    });
    (diffs, byte_diff)
}

struct Tally {
    parts: usize,
    exact: usize,
    bytes: usize,
    report: Vec<String>,
    byte_report: Vec<String>,
}

fn run(files: &[PathBuf]) -> Tally {
    let mut t = Tally {
        parts: 0,
        exact: 0,
        bytes: 0,
        report: Vec::new(),
        byte_report: Vec::new(),
    };
    for f in files {
        for (name, xml) in chart_parts(f) {
            let (diffs, byte_diff) = roundtrip(&xml);
            t.parts += 1;
            t.exact += diffs.is_empty() as usize;
            t.bytes += byte_diff.is_none() as usize;
            if !diffs.is_empty() {
                t.report.push(format!(
                    "{} {name}: 차이 {}\n  {}",
                    f.display(),
                    diffs.len(),
                    diffs.join("\n  ")
                ));
            } else if let Some(b) = byte_diff {
                t.byte_report
                    .push(format!("{} {name}: 트리는 같고 {b}", f.display()));
            }
        }
    }
    t
}

/// 한셀의 레거시 차트 읽기 결과는 모두 똑같이 다시 쓴다.
#[test]
fn legacy_chart_parts_are_rewritten_exactly() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let files: Vec<PathBuf> = ["craft-charts", "craft-charts2", "templates"]
        .iter()
        .flat_map(|d| files_with_suffix(&dir.join(d), ".legacy.as.cell"))
        .collect();
    let t = run(&files);
    eprintln!(
        "레거시 차트 (한셀 읽기): 파트 {} — 트리 일치 {}, 바이트 일치 {}",
        t.parts, t.exact, t.bytes
    );
    for r in &t.byte_report {
        eprintln!("{r}");
    }
    assert!(t.parts > 0, "차트 파트가 없다");
    assert!(t.report.is_empty(), "{}", t.report.join("\n\n"));
}

/// (경로, 값) 목록
type Found<T> = Vec<(String, T)>;

/// 차트 안의 도형 속성과 글 상자 (경로와 함께)
fn props_and_texts(c: &Chart) -> (Found<ChartShapeProps>, Found<ChartRich>) {
    let mut sp: Found<ChartShapeProps> = Vec::new();
    let mut tx: Found<ChartRich> = Vec::new();
    let mut add_sp = |p: &str, x: &Option<ChartShapeProps>| {
        if let Some(x) = x {
            sp.push((p.to_string(), x.clone()));
        }
    };
    add_sp("chartSpace", &c.sp_pr);
    add_sp("plotArea", &c.plot_sp_pr);
    add_sp("legend", &c.legend_props.sp_pr);
    let mut titles = vec![("title".to_string(), c.title_full.clone())];
    for (i, a) in c.axes.iter().enumerate() {
        add_sp(&format!("{}[{i}]", a.kind), &a.sp_pr);
        for g in [&a.major_gridlines, &a.minor_gridlines]
            .into_iter()
            .flatten()
        {
            add_sp(&format!("{}[{i}]/gridlines", a.kind), &g.sp_pr);
        }
        titles.push((format!("{}[{i}]/title", a.kind), a.title.clone()));
        if let Some(r) = &a.tx_pr {
            tx.push((format!("{}[{i}]/txPr", a.kind), r.clone()));
        }
    }
    for (i, s) in c.series.iter().enumerate() {
        add_sp(&format!("ser[{i}]"), &s.sp_pr);
        if let Some(m) = &s.marker {
            add_sp(&format!("ser[{i}]/marker"), &m.sp_pr);
        }
    }
    for (p, t) in titles {
        let Some(t) = t else { continue };
        add_sp(&p, &t.sp_pr);
        if let Some(ChartTx::Rich(r)) = &t.tx {
            tx.push((format!("{p}/rich"), r.clone()));
        }
    }
    if let Some(r) = &c.legend_props.tx_pr {
        tx.push(("legend/txPr".into(), r.clone()));
    }
    (sp, tx)
}

/// 한셀의 레거시 차트 읽기 결과의 선·글 상자·글꼴은 모두 `ChartLine::hcell`·`ChartRich::hcell`·`ChartFont::hcell`
/// 의 꼴이다 (값만 다르다) — 레거시 변환이 이 생성자로 만들면 된다는 근거.
#[test]
fn legacy_chart_lines_and_texts_are_hcell_form() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let (mut lines, mut texts, mut bad) = (0, 0, Vec::new());
    for f in ["craft-charts", "craft-charts2", "templates"]
        .iter()
        .flat_map(|d| files_with_suffix(&dir.join(d), ".legacy.as.cell"))
    {
        for (name, xml) in chart_parts(&f) {
            let c = rcell::parser::chart::parse_chart(&rcell::xml::parse_raw(&xml, "c").unwrap());
            let (sp, tx) = props_and_texts(&c);
            for (p, x) in sp {
                let Some(l) = &x.line else { continue };
                lines += 1;
                let (Some(w), Some(fill)) = (l.width, l.fill.clone()) else {
                    bad.push(format!("{} {name} {p}: 선 {l:?}", f.display()));
                    continue;
                };
                if *l != ChartLine::hcell(w, fill) {
                    bad.push(format!("{} {name} {p}: 선 {l:?}", f.display()));
                }
            }
            for (p, r) in tx {
                texts += 1;
                let font = r.paras.first().and_then(|q| q.def_font.clone());
                let ok = font.as_ref().is_some_and(|ft| {
                    let rgb = match &ft.fill {
                        Some(ChartFill::Solid(c)) => c.value.strip_prefix("srgb:").unwrap_or(""),
                        _ => "",
                    };
                    let hcell_font = ChartFont::hcell(
                        ft.size.unwrap_or(0),
                        ft.bold == Some(true),
                        ft.italic == Some(true),
                        rgb,
                        ft.latin.as_deref().unwrap_or(""),
                    );
                    let text = r.paras[0].runs.first().map(|x| x.text.as_str());
                    *ft == hcell_font
                        && r == ChartRich::hcell(
                            hcell_font,
                            text,
                            r.vert.as_deref() == Some("vert270"),
                        )
                });
                if !ok {
                    bad.push(format!("{} {name} {p}: 글 {r:?}", f.display()));
                }
            }
        }
    }
    eprintln!("레거시 차트의 선 {lines}개, 글 상자 {texts}개");
    assert!(lines > 0 && texts > 0);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// 한셀이 저장한 XML 차트 (원본이 XML): 보고만 한다.
#[test]
fn hancell_xml_chart_parts_report() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let only_cell = |d: &Path| -> Vec<PathBuf> {
        files_with_suffix(d, ".cell")
            .into_iter()
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                !(n.ends_with(".legacy.cell")
                    || n.ends_with(".bin.cell")
                    || n.ends_with(".as.cell")
                    || n.ends_with(".hs.cell"))
            })
            .collect()
    };
    for (label, files) in [
        (
            "templates/hs",
            files_with_suffix(&dir.join("templates/hs"), ".hs.cell"),
        ),
        ("craft-charts/*.cell", only_cell(&dir.join("craft-charts"))),
        (
            "craft-charts2/*.cell",
            only_cell(&dir.join("craft-charts2")),
        ),
        (
            "craft-charts/hs",
            files_with_suffix(&dir.join("craft-charts/hs"), ".hs.cell"),
        ),
    ] {
        let t = run(&files);
        eprintln!(
            "{label}: 파트 {} — 트리 일치 {}, 바이트 일치 {}",
            t.parts, t.exact, t.bytes
        );
        for r in t.report.iter().chain(&t.byte_report) {
            eprintln!("{r}");
        }
    }
}

/// 모델에서 바로 만든 차트 (레거시에서 바꾼 차트처럼): 한셀 꼴로 쓰고, 다시 읽으면 같은 모델이다.
#[test]
fn model_built_chart_is_written_in_hancell_form() {
    let font = |sz: u32, bold: bool| ChartFont::hcell(sz, bold, false, "000000", "함초롬돋움");
    let data = |kind: &str, f: &str, fmt: Option<&str>, vals: &[&str], per_point: bool| ChartData {
        kind: kind.into(),
        formula: Some(f.into()),
        cache: Some(ChartCache {
            format_code: fmt.map(str::to_string),
            pt_count: Some(vals.len() as u32),
            points: vals
                .iter()
                .enumerate()
                .map(|(i, v)| ChartPoint {
                    idx: i as u32,
                    format_code: if per_point {
                        fmt.map(str::to_string)
                    } else {
                        None
                    },
                    value: v.to_string(),
                })
                .collect(),
        }),
    };
    let mut ch = Chart {
        kind: "bar".into(),
        bar_dir: Some("col".into()),
        grouping: Some("clustered".into()),
        legend: Some("b".into()),
        date1904: Some(false),
        rounded_corners: Some(false),
        style: Some(2),
        c14_style: Some(102),
        title_full: Some(ChartTitle {
            tx: Some(ChartTx::Rich(ChartRich::hcell(
                font(1200, true),
                Some("매출 & 이익"),
                false,
            ))),
            layout: Some(ChartLayout {
                manual: Some(ChartManualLayout {
                    x_mode: Some("edge".into()),
                    y_mode: Some("edge".into()),
                    x: Some(0.3),
                    ..Default::default()
                }),
            }),
            overlay: Some(false),
            sp_pr: Some(ChartShapeProps {
                fill: Some(ChartFill::None),
                line: None,
            }),
            tx_pr: None,
        }),
        auto_title_deleted: Some(false),
        series: vec![ChartSeries {
            idx: Some(0),
            order: Some(0),
            tx: Some(ChartTx::Ref(data(
                "strRef",
                "Sheet1!$B$1",
                None,
                &["yoomin53"],
                false,
            ))),
            sp_pr: Some(ChartShapeProps {
                fill: Some(ChartFill::Solid(ChartColor::srgb("73B1D1"))),
                line: Some(ChartLine::hcell(9525, ChartFill::None)),
            }),
            invert_if_negative: Some(false),
            cat: Some(data(
                "strRef",
                "Sheet1!$A$2:$A$3",
                None,
                &["1월", ""],
                false,
            )),
            val: Some(data(
                "numRef",
                "Sheet1!$B$2:$B$3",
                Some("G/표준"),
                &["10", "20.5"],
                true,
            )),
            ..Default::default()
        }],
        axes: vec![ChartAxis {
            kind: "valAx".into(),
            id: 1,
            title: Some(ChartTitle {
                tx: Some(ChartTx::Rich(ChartRich::hcell(
                    font(1000, true),
                    Some("값"),
                    true,
                ))),
                layout: Some(ChartLayout::default()),
                overlay: Some(false),
                ..Default::default()
            }),
            tx_pr: Some(ChartRich::hcell(font(1000, false), None, false)),
            ..Default::default()
        }],
        plot_vis_only: Some(true),
        disp_blanks_as: Some("gap".into()),
        ..Default::default()
    };
    // 요약 필드(뷰어·비교용)는 자세한 필드에서 만든다
    ch.refresh_summary();
    assert_eq!(ch.title.as_deref(), Some("매출 & 이익"));
    assert_eq!(ch.val_axis_title.as_deref(), Some("값"));
    let s = &ch.series[0];
    assert_eq!(s.name.as_deref(), Some("yoomin53"));
    assert_eq!(s.categories, ["1월", ""]);
    assert_eq!(s.values, [Some(10.0), Some(20.5)]);
    assert_eq!(
        s.color.as_ref().map(|c| c.value.as_str()),
        Some("srgb:73B1D1")
    );

    let xml = rcell::chart_xml::chart_xml(&ch);
    assert!(xml.starts_with(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><c:chartSpace xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:date1904 val="0"/><c:roundedCorners val="0"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:c14="http://schemas.microsoft.com/office/drawing/2007/8/2/chart" Requires="c14"><c14:style val="102"/></mc:Choice><mc:Fallback><c:style val="2"/></mc:Fallback></mc:AlternateContent><c:chart><c:title><c:tx><c:rich><a:bodyPr rot="0" vert="horz" wrap="none" lIns="0" tIns="0" rIns="0" bIns="0" anchor="ctr" anchorCtr="1"/><a:p><a:pPr algn="l"><a:defRPr sz="1200" b="1" i="0" u="none"><a:solidFill><a:srgbClr val="000000"/></a:solidFill><a:latin typeface="함초롬돋움"/><a:ea typeface="함초롬돋움"/><a:cs typeface="함초롬돋움"/><a:sym typeface="함초롬돋움"/></a:defRPr></a:pPr><a:r><a:rPr sz="1200" b="1" i="0" u="none">"#
    ), "{xml}");
    // 한셀 꼴의 선, 소문자 RGB, f32 배치 비율, 이스케이프, 빈 값, 세로 축 제목
    assert!(xml.contains(r#"<a:solidFill><a:srgbClr val="73b1d1"/></a:solidFill><a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:noFill/><a:prstDash val="solid"/><a:round/><a:headEnd w="med" len="med"/><a:tailEnd w="med" len="med"/></a:ln>"#), "{xml}");
    assert!(
        xml.contains(r#"<c:x val="0.30000001192092895508"/>"#),
        "{xml}"
    );
    assert!(
        xml.contains("<a:t>매출 &amp; 이익</a:t></a:r><a:endParaRPr/></a:p>"),
        "{xml}"
    );
    assert!(xml.contains(r#"<c:pt idx="1"><c:v/></c:pt>"#), "{xml}");
    assert!(
        xml.contains(r#"<c:pt idx="1" formatCode="G/표준"><c:v>20.5</c:v></c:pt>"#),
        "{xml}"
    );
    assert!(xml.contains(r#"<c:valAx><c:axId val="1"/><c:scaling/><c:title><c:tx><c:rich><a:bodyPr vert="vert270" wrap="none""#), "{xml}");
    assert!(xml.contains(r#"<c:legend><c:legendPos val="b"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart></c:chartSpace>"#), "{xml}");

    // 다시 읽으면 같은 모델 (배치 비율만 f32 를 거친다), 다시 쓰면 바이트까지 같다
    let back = rcell::parser::chart::parse_chart(&rcell::xml::parse_raw(&xml, "t").unwrap());
    let mut expect = ch.clone();
    if let Some(ChartLayout { manual: Some(m) }) =
        expect.title_full.as_mut().and_then(|t| t.layout.as_mut())
    {
        m.x = Some(0.3f32 as f64);
    }
    assert_eq!(back, expect);
    assert_eq!(rcell::chart_xml::chart_xml(&back), xml);
}
