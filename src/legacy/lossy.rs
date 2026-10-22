// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 형식이 담지 못하는 것 — 레거시로 쓴 뒤 다시 읽으면 바뀌는 값을 모델에 미리 적용한다.
//!
//! [`legacy_view`] 는 통합 문서를 "레거시로 저장했다가 다시 연 모습"으로 바꾼다. 왕복 시험
//! (`tests/real_samples.rs`)은 원본에 이것을 적용한 것과 rcell 이 쓴 레거시를 다시 읽은 것을 비교한다.
//! 하나하나가 한셀의 레거시 저장·읽기와 같은 손실이다 (`docs/format/cell-legacy.md` 8절, 10절).

use crate::model::{Paint, ShapeKind, Workbook};

/// 한셀처럼 여백을 HWPUNIT(1/7200 인치)로 잘라 f32 로 쓰고(쓰기), 다시 잘라 읽는다(읽기)
fn hwp_round_trip(v: f64) -> f64 {
    let written = ((v * 7200.0).trunc() / 7200.0) as f32 as f64;
    ((written * 7200.0).trunc() / 7200.0) as f32 as f64
}

/// 머리글·바닥글 구역(`&L`·`&C`·`&R`)마다 글꼴 코드가 없으면 기본 글꼴 코드를 넣는다 (한셀의 레거시 저장)
pub(crate) fn with_default_fonts(s: &str, font: &str) -> String {
    let code = format!("&\"{font},Regular\"");
    let mut out = String::with_capacity(s.len() + 32);
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut at_section_start = true;
    // 구역 코드가 하나도 없으면 문자열 전체가 가운데 구역이다
    while i < chars.len() {
        if chars[i] == '&' && i + 1 < chars.len() && matches!(chars[i + 1], 'L' | 'C' | 'R') {
            out.push('&');
            out.push(chars[i + 1]);
            i += 2;
            at_section_start = true;
            continue;
        }
        if at_section_start {
            let starts_font = chars[i] == '&' && chars.get(i + 1) == Some(&'"');
            if !starts_font {
                out.push_str(&code);
            }
            at_section_start = false;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 그리기 개체 하나(자식까지)를 레거시로 저장했다가 다시 연 모습 (`art.rs` 의 쓰기와 읽기의 `legacy_shape`):
/// 앵커 오프셋은 정수 픽셀로 올리고, 선은 기본값(9525 EMU, 실선, 끝 flat, 겹선 sng, 이음 — 지정 없음은 둥글게,
/// 그림은 뾰족하게)으로 채우고, 그림자는 자른 x·y 오프셋에서 다시 계산한 거리·방향과 회색 기본 색·불투명도만
/// 남기고(흐림 없음), 잠금은 0x007F·0x033F 에 담기는 것만 남기고 글 상자에 회전 잠금을 더하고, 시트와 함께
/// 잠금은 늘 켜고, 글 상자의 맞춤·줄 바꿈·여백은 버리고 글 넘침은 자름이 된다
fn shape_view(s: &mut crate::model::Shape) {
    let snap = |off: &mut i64| {
        *off = super::art::anchor_px(*off) * crate::units::EMU_PER_PX;
    };
    snap(&mut s.anchor.from.col_off);
    snap(&mut s.anchor.from.row_off);
    if let Some(t) = &mut s.anchor.to {
        snap(&mut t.col_off);
        snap(&mut t.row_off);
    }
    let has_text = super::art::shape_has_text(s);
    let (group, picture) = (s.kind == ShapeKind::Group, s.kind == ShapeKind::Picture);
    // 채우기가 지정되지 않으면: 연결선은 없음으로 쓰고(도형 대조 실험 0x01BF = 0x00100000), 다른 도형은 읽을 때
    // 흰색이다
    if !group && !picture && s.fill.is_none() {
        s.fill = Some(if s.kind == ShapeKind::Connector {
            Paint::None
        } else {
            Paint::Solid(crate::model::DrawColor {
                value: "srgb:FFFFFF".into(),
                mods: Vec::new(),
            })
        });
    }
    // 그림의 채우기·선은 한셀처럼 '없음' 으로 쓴다
    if picture {
        s.fill.get_or_insert(Paint::None);
        s.line.get_or_insert(crate::model::Outline {
            fill: Some(Paint::None),
            ..Default::default()
        });
    }
    if !group && s.line.is_none() {
        // 선이 지정되지 않으면 읽을 때 검정 실선이다
        s.line = Some(crate::model::Outline {
            fill: Some(Paint::Solid(crate::model::DrawColor {
                value: "srgb:000000".into(),
                mods: Vec::new(),
            })),
            ..Default::default()
        });
    }
    if let Some(l) = &mut s.line {
        l.width.get_or_insert(9525);
        l.dash.get_or_insert_with(|| "solid".into());
        if l.fill.is_none() {
            l.fill = Some(Paint::None);
        }
        l.cap.get_or_insert_with(|| "flat".into());
        l.compound.get_or_insert_with(|| "sng".into());
        if picture {
            l.join = Some("miter".into());
        } else {
            l.join.get_or_insert_with(|| "round".into());
        }
    }
    // 그림자는 그림·그룹에 쓰지 않는다
    if group || picture {
        s.shadow = None;
    }
    if let Some(sh) = &mut s.shadow {
        let (ox, oy) = super::art::shadow_offsets(sh.dist, sh.dir);
        let (ox, oy) = (ox as f64, oy as f64);
        sh.dist = ox.hypot(oy).round() as i64;
        sh.dir =
            ((oy.atan2(ox).to_degrees().rem_euclid(360.0) * 60000.0).round() as i64) % 21_600_000;
        sh.blur = 0;
        sh.rot_with_shape = false;
        sh.align = Some(sh.align.clone().unwrap_or_else(|| "b".into()));
        let alpha = sh
            .color
            .mods
            .iter()
            .find(|(k, _)| k == "alpha")
            .map(|(_, v)| *v)
            .filter(|&v| v < 100_000)
            .map(|v| {
                let fixed = (v as f64 / 100_000.0 * 65536.0).round();
                (fixed / 65536.0 * 100_000.0).round() as i64
            });
        if !sh.color.value.starts_with("srgb:") {
            sh.color.value = "srgb:808080".into();
        }
        sh.color.mods = alpha
            .map(|a| ("alpha".to_string(), a))
            .into_iter()
            .collect();
    }
    // 잠금: 0x007F 의 아홉 가지 (연결선·그림은 글 잠금이 늘 켜진다) + 0x033F 의 모양 종류, 글 상자는 회전 잠금
    const NAMES: [&str; 9] = [
        "noGrp",
        "noAdjustHandles",
        "noTextEdit",
        "noEditPoints",
        "noCrop",
        "noSelect",
        "noMove",
        "noChangeAspect",
        "noRot",
    ];
    let mut locks: Vec<String> = NAMES
        .iter()
        .filter(|n| s.locks.iter().any(|l| l == *n))
        .map(|n| n.to_string())
        .collect();
    if s.locks.iter().any(|l| l == "noChangeShapeType") {
        locks.push("noChangeShapeType".into());
    }
    if has_text && s.kind == ShapeKind::Shape && !locks.iter().any(|l| l == "noRot") {
        locks.push("noRot".into());
    }
    locks.retain(|l| match s.kind {
        ShapeKind::Picture => l != "noTextEdit",
        ShapeKind::Connector => l != "noTextEdit" && l != "noCrop",
        ShapeKind::Group => matches!(
            l.as_str(),
            "noGrp" | "noSelect" | "noRot" | "noChangeAspect" | "noMove"
        ),
        _ => l != "noCrop",
    });
    locks.sort();
    s.locks = locks;
    s.locks_with_sheet = Some(true);
    s.prints_with_sheet = Some(s.prints_with_sheet.unwrap_or(true));
    // 사용자 정의 모양은 꼭짓점을 쓰지 않아 모양이 사라진다. 읽기가 모르는 모양 번호는 사각형으로 읽힌다
    const KNOWN: [&str; 15] = [
        "rect",
        "roundRect",
        "ellipse",
        "diamond",
        "triangle",
        "rtTriangle",
        "parallelogram",
        "trapezoid",
        "hexagon",
        "octagon",
        "plus",
        "star5",
        "rightArrow",
        "line",
        "straightConnector1",
    ];
    match s.geometry.as_deref() {
        Some("custom") => s.geometry = None,
        Some(g) if !group && !KNOWN.contains(&g) => s.geometry = Some("rect".into()),
        _ => {}
    }
    if has_text {
        if let Some(t) = &mut s.text {
            t.anchor = None;
            t.wrap = None;
            t.vert = None;
            t.insets = [None; 4];
            t.vert_overflow = Some("clip".into());
            t.horz_overflow = Some("clip".into());
        } else {
            s.text = Some(crate::model::TextBody {
                vert_overflow: Some("clip".into()),
                horz_overflow: Some("clip".into()),
                ..Default::default()
            });
        }
    } else {
        s.text = None;
    }
    for c in &mut s.children {
        shape_view(c);
    }
}

/// 레거시로 저장했다가 다시 연 모습
/// 차등 서식을 레거시에 남는 꼴로 (`dxf::dxf_props`·`cf::dxf_bytes` 의 규칙, 차등 서식 대조 실험 1–3): 글꼴 크기는
/// 정수 pt, 위 첨자는 한컴 `hs:superscript` 만, ECMA 외곽선·그림자·condense·extend 대신 한컴 종류, 한컴 밑줄·취소선
/// 종류가 BIFF 밑줄·취소선이 된다. 대각선 테두리, 무늬 `none`, 꺼진 보호(`locked="0"`), 남는 속성이 없는 글꼴·테두리·
/// 채우기는 없어진다. 내장 번호의 표시 형식은 내장 코드로 읽힌다
fn dxf_view(d: &mut crate::model::Dxf, wb: &Workbook) {
    // 테마·색 번호는 RGB 로 저장된다
    let rgb = |c: &mut Option<crate::model::Color>| {
        if let Some(x) = c
            && !x.auto
            && x.rgb.is_none()
            && let Some(v) = super::writer::rgb_of(x, wb)
        {
            *x = crate::model::Color {
                rgb: Some(format!("ff{v:06x}")),
                ..Default::default()
            };
        }
    };
    if let Some(f) = &mut d.font {
        rgb(&mut f.color);
    }
    if let Some(fl) = &mut d.fill {
        rgb(&mut fl.fg_color);
        rgb(&mut fl.bg_color);
    }
    if let Some(b) = &mut d.border {
        for e in [
            &mut b.left,
            &mut b.right,
            &mut b.top,
            &mut b.bottom,
            &mut b.vertical,
            &mut b.horizontal,
        ]
        .into_iter()
        .flatten()
        {
            rgb(&mut e.color);
        }
    }
    if let Some(f) = &mut d.font {
        let h = f.hancom.clone();
        let h = h.as_ref();
        let hs_ul = h.and_then(|h| h.underline_type.clone());
        let hs_strike = h
            .and_then(|h| h.strikeout_type.as_deref())
            .is_some_and(|t| t != "0");
        let outline = h.and_then(|h| h.outline_type.clone());
        let shadow = h.and_then(|h| h.shadow_type.clone());
        let superscript = h.and_then(|h| h.superscript) == Some(true);
        // 0x088D 에 남는 속성이 하나라도 있나 (dxf_props 와 같은 조건)
        let visible = f.name.as_deref().is_some_and(|n| !n.is_empty())
            || f.bold
            || f.underline.is_some()
            || hs_ul.is_some()
            || superscript
            || f.italic
            || f.strike
            || hs_strike
            || outline.is_some()
            || shadow.is_some()
            || f.size.is_some()
            || f.color.is_some();
        if !visible {
            d.font = None;
        } else {
            f.size = f.size.map(|s| s.max(0.0).floor());
            f.vert_align = superscript.then(|| "superscript".to_string());
            f.outline = outline.as_deref() == Some("solid");
            f.shadow = shadow.is_some_and(|s| s != "none");
            f.condense = false;
            f.extend = false;
            if f.underline.as_deref() == Some("none") {
                f.underline = None;
            }
            if f.underline.is_none() && hs_ul.as_deref() == Some("1") {
                f.underline = Some("single".into());
            }
            if hs_strike {
                f.strike = true;
            }
        }
    }
    if let Some(b) = &mut d.border {
        b.diagonal = None;
        b.diagonal_up = false;
        b.diagonal_down = false;
        let some = |e: &Option<crate::model::BorderEdge>| e.is_some();
        if ![
            &b.left,
            &b.right,
            &b.top,
            &b.bottom,
            &b.vertical,
            &b.horizontal,
        ]
        .into_iter()
        .any(some)
        {
            d.border = None;
        }
    }
    if let Some(fl) = &mut d.fill
        && fl.gradient.is_none()
        && fl.pattern_type.as_deref() == Some("none")
    {
        fl.pattern_type = None;
        if fl.fg_color.is_none() && fl.bg_color.is_none() {
            d.fill = None;
        }
    }
    if let Some(p) = &d.protection
        && !p.locked.unwrap_or(true)
        && p.hidden != Some(true)
    {
        d.protection = None;
    }
    if let Some(n) = &mut d.num_fmt
        && n.id < 164
        && let Some(code) = crate::numfmt::builtin(n.id)
    {
        n.code = code.to_string();
    }
}

pub fn legacy_view(wb: &Workbook) -> Workbook {
    // 셀이 없는 메모 자리에는 빈 셀이 생긴다 (writer::with_comment_cells)
    let mut w = super::writer::with_comment_cells(wb).unwrap_or_else(|| wb.clone());
    // 창 잠금: 한셀은 WINDOWPROTECT 를 늘 0 으로 쓴다
    if let Some(p) = &mut w.protection {
        p.lock_windows = false;
    }
    // 외부 값 캐시(XCT/CRN)는 쓰지 않는다; 수식·이름이 쓰지 않는 외부 연결은 다시 열 때 버린다
    for l in &mut w.external_links {
        l.sheet_data.clear();
    }
    // 분류·상태(DocumentSummaryInformation)는 저장되지 않는다 (관측: 문서 속성 실험의 한셀 레거시도 SummaryInformation
    // 하나뿐)
    w.properties.category = None;
    w.properties.content_status = None;
    // 원본 그대로 옮기는 패키지 파트(피벗·연결·사용자 지정 XML·프린터 설정 …)는 레거시에 없다 — 매크로
    // 프로젝트만 `_VBS_PROJECT_W` 스트림으로 남는다
    w.part_links
        .retain(|l| l.rel_type == crate::ns::REL_HANCOM_VBS_PROJECT);
    for s in &mut w.sheets {
        s.part_links.clear();
        // pageSetup 이 없으면 한셀의 기본 SETUP (writer: 맞춤 0·0, 세로)
        s.page.setup.get_or_insert(crate::model::PageSetup {
            paper_size: Some(9),
            scale: Some(100),
            first_page_number: Some(1),
            fit_to_width: Some(0),
            fit_to_height: Some(0),
            orientation: Some("portrait".into()),
            use_printer_defaults: true,
            horizontal_dpi: Some(0),
            vertical_dpi: Some(0),
            copies: Some(1),
            ..Default::default()
        });
    }
    for xf in w
        .styles
        .cell_xfs
        .iter_mut()
        .chain(w.styles.cell_style_xfs.iter_mut())
    {
        // 줄 간격 atLeast 는 betweenLines 와 같은 값으로 저장된다
        if let Some(p) = xf.hancom.as_mut().and_then(|h| h.parashape.as_mut())
            && p.line_spacing_type.as_deref() == Some("atLeast")
        {
            p.line_spacing_type = Some("betweenLines".into());
        }
        // 글자 회전은 저장되지 않는다 (한셀의 레거시 저장은 XF 의 trot 를 늘 0 으로 쓴다)
        if let Some(a) = &mut xf.alignment {
            a.text_rotation = None;
        }
    }
    // 외곽선 종류(한컴)가 있으면 ECMA 외곽선도 켜진 것으로 읽힌다 (실선이 아니어도).
    // 한컴 취소선 종류만 있어도 BIFF 취소선으로 저장된다 (밑줄은 한컴 밑줄 종류를 따라 다시 읽힌다)
    for f in &mut w.styles.fonts {
        let set = |v: Option<&str>| v.is_some_and(|t| t != "none" && t != "0");
        let h = f.hancom.as_ref();
        let (outline, strike) = (
            set(h.and_then(|h| h.outline_type.as_deref())),
            set(h.and_then(|h| h.strikeout_type.as_deref())),
        );
        if outline {
            f.outline = true;
        }
        if strike {
            f.strike = true;
        }
    }
    let theme_wb = w.clone();
    for d in &mut w.styles.dxfs {
        dxf_view(d, &theme_wb);
    }
    let font = wb
        .styles
        .fonts
        .first()
        .and_then(|f| f.name.clone())
        .unwrap_or_else(|| "맑은 고딕".into());
    for s in &mut w.sheets {
        // 자동 페이지 나누기 끔과 간단히 인쇄(draft)는 저장되지 않는다
        s.page.auto_page_breaks = s.page.auto_page_breaks.map(|_| true);
        if let Some(p) = &mut s.page.setup {
            p.draft = false;
        }
        // 요약 행·열 위치: 한셀은 WSBOOL 을 0x04C1 로 쓴다
        s.summary_above = false;
        s.summary_left = false;
        // 차트 내용(0x7EF0 Vt 아카이브)은 아직 쓰지 않는다
        s.shapes.retain(|x| x.kind != ShapeKind::Chart);
        for x in &mut s.shapes {
            shape_view(x);
        }
        // 메모 상자: 셀과 함께 옮기지 않으면 크기도 바꾸지 않는다 (앵커 플래그 3). 자리의 픽셀 오프셋은 열·행
        // 단위(1/32768·1/8192)로 바꿨다 다시 읽은 값 — 숨긴 행·열 안의 오프셋은 0 (한셀 레거시도 같다: 메모 실험의
        // 숨긴 7행)
        let mdw = crate::units::max_digit_width(wb);
        let sheet_copy = s.clone();
        for c in &mut s.comments {
            if let Some(b) = &mut c.shape {
                if !b.move_with_cells {
                    b.size_with_cells = false;
                }
                let a = &mut b.anchor;
                for (cell, off, unit, col) in [
                    (a[0], 1, 32768.0, true),
                    (a[2], 3, 8192.0, false),
                    (a[4], 5, 32768.0, true),
                    (a[6], 7, 8192.0, false),
                ] {
                    let px = if col {
                        crate::units::sheet_col_px(&sheet_copy, cell, mdw)
                    } else {
                        crate::units::sheet_row_px(&sheet_copy, cell)
                    };
                    let eff = px.saturating_sub(1) as f64;
                    a[off] = if eff <= 0.0 {
                        0
                    } else {
                        let max = if col { 0x7FFF } else { 0x1FFF };
                        let d = ((a[off] as f64 * unit / eff).round() as i64).clamp(0, max);
                        (d as f64 * eff / unit).round() as u32
                    };
                }
            }
        }
        // 하이퍼링크의 화면 설명은 저장되지 않는다 (HLINKTOOLTIP 을 쓰지 않는다). `file:` 주소는 파일 모니커의
        // 경로로 저장되어 상대 경로 `.\file:…` 로 읽힌다 (하이퍼링크 대조 실험)
        for h in &mut s.hyperlinks {
            h.tooltip = None;
            if let Some(t) = h.target.as_mut().filter(|t| t.starts_with("file:")) {
                *t = format!(".\\{t}");
            }
        }
        // 데이터 표의 지운 입력 표시(del1·del2)는 저장되지 않는다 (수식 대조 실험: TABLE grbit 에 없다)
        for c in &mut s.cells {
            if let Some(d) = c.formula.as_mut().and_then(|f| f.data_table.as_mut()) {
                d.del1 = false;
                d.del2 = false;
            }
        }
        // 표: 자동 필터 조건과 계산 열 수식은 저장되지 않는다 (8절)
        for t in &mut s.tables {
            if let Some(af) = &mut t.auto_filter {
                af.columns.clear();
            }
            for c in &mut t.columns {
                c.calculated_formula = None;
            }
        }
        // 조건부 서식의 x14 확장: 데이터 막대는 방향(오른쪽에서 왼쪽)만 CFDatabar 에 남고, 사용자 지정 아이콘은
        // 저장되지 않는다 (6.3절)
        for cf in &mut s.conditional_formats {
            for r in &mut cf.rules {
                // 확장이 붙은 막대는 확장의 기준 값이 본 기준 값으로 저장된다 (writer::rule_cfvos)
                let cfvos: Option<Vec<crate::model::Cfvo>> = r
                    .data_bar
                    .as_ref()
                    .filter(|d| d.ext.as_ref().is_some_and(|e| e.cfvos.len() >= 2))
                    .map(|_| {
                        super::writer::rule_cfvos(r)
                            .into_iter()
                            .map(|(c, _)| c)
                            .collect()
                    });
                if let Some(d) = &mut r.data_bar {
                    if let Some(c) = cfvos {
                        d.cfvos = c;
                    }
                    let rtl = d
                        .ext
                        .as_ref()
                        .is_some_and(|e| e.direction.as_deref() == Some("rightToLeft"));
                    d.ext = rtl.then(|| crate::model::DataBarExt {
                        cfvos: d.cfvos.clone(),
                        direction: Some("rightToLeft".into()),
                        ..Default::default()
                    });
                }
                if let Some(i) = &mut r.icon_set {
                    i.custom_icons.clear();
                }
            }
        }
        // 레거시에서 읽은 여백은 이미 한셀이 읽은 HWPUNIT 값이라 그대로 다시 읽힌다 (writer::margin)
        if let Some(m) = s.page.margins.as_mut().filter(|_| !wb.styles.from_legacy) {
            for v in [
                &mut m.left,
                &mut m.right,
                &mut m.top,
                &mut m.bottom,
                &mut m.header,
                &mut m.footer,
            ] {
                *v = hwp_round_trip(*v);
            }
        }
        if let Some(h) = &mut s.page.header_footer {
            for t in [&mut h.odd_header, &mut h.odd_footer] {
                if let Some(x) = t.as_mut().filter(|x| !x.is_empty()) {
                    *x = with_default_fonts(x, &font);
                }
            }
            for t in [
                &mut h.even_header,
                &mut h.even_footer,
                &mut h.first_header,
                &mut h.first_footer,
            ] {
                if let Some(x) = t.as_mut().filter(|x| !x.is_empty()) {
                    *x = with_default_fonts(x, &font);
                }
            }
        }
    }
    // 구조적 참조는 PtgList 로 저장되어 읽기의 꼴로 바뀐다: 표 이름만은 `표[]`, `[#Headers],[#Data]` 는
    // `[#Data],[#Headers]` 차례 (formula::structured_ref)
    let names: Vec<String> = w
        .sheets
        .iter()
        .flat_map(|s| &s.tables)
        .map(|t| t.display_name.clone())
        .filter(|n| !n.is_empty())
        .collect();
    if !names.is_empty() {
        for s in &mut w.sheets {
            for c in &mut s.cells {
                if let Some(f) = &mut c.formula {
                    f.text = canonical_structured_refs(&f.text, &names);
                }
            }
            for t in &mut s.tables {
                for c in &mut t.columns {
                    if let Some(f) = &mut c.totals_formula {
                        *f = canonical_structured_refs(f, &names);
                    }
                }
            }
        }
    }
    super::reader::prune_external_links(&mut w);
    w
}

/// 수식 글의 구조적 참조를 레거시 읽기의 꼴로 ([`legacy_view`]). 따옴표 안은 건드리지 않는다
pub(crate) fn canonical_structured_refs(text: &str, tables: &[String]) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 4);
    let mut i = 0;
    let word = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '.';
    while i < c.len() {
        if c[i] == '"' {
            // 문자열 상수 그대로
            out.push('"');
            i += 1;
            while i < c.len() {
                out.push(c[i]);
                if c[i] == '"' {
                    if c.get(i + 1) == Some(&'"') {
                        out.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if word(c[i]) && (i == 0 || !word(c[i - 1])) {
            let start = i;
            while i < c.len() && word(c[i]) {
                i += 1;
            }
            let w: String = c[start..i].iter().collect();
            out.push_str(&w);
            let is_table = tables.iter().any(|t| t.eq_ignore_ascii_case(&w));
            if is_table
                && c.get(i) != Some(&'[')
                && c.get(i) != Some(&'(')
                && c.get(i) != Some(&'!')
            {
                out.push_str("[]");
            }
            if is_table && c.get(i) == Some(&'[') {
                let from = "[[#Headers],[#Data]";
                let n = from.chars().count();
                let head: String = c[i..].iter().take(n).collect();
                if head.eq_ignore_ascii_case(from) {
                    out.push_str("[[#Data],[#Headers]");
                    i += n;
                }
            }
            continue;
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_codes_per_section() {
        assert_eq!(
            with_default_fonts("&L왼쪽&C&\"굴림,Bold\"가운데&R&P / &N", "맑은 고딕"),
            "&L&\"맑은 고딕,Regular\"왼쪽&C&\"굴림,Bold\"가운데&R&\"맑은 고딕,Regular\"&P / &N"
        );
        assert_eq!(
            with_default_fonts("&C&D &T", "맑은 고딕"),
            "&C&\"맑은 고딕,Regular\"&D &T"
        );
    }
}
