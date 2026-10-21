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

/// 레거시로 저장했다가 다시 연 모습
pub fn legacy_view(wb: &Workbook) -> Workbook {
    let mut w = wb.clone();
    // 창 잠금: 한셀은 WINDOWPROTECT 를 늘 0 으로 쓴다
    if let Some(p) = &mut w.protection {
        p.lock_windows = false;
    }
    // 외부 값 캐시(XCT/CRN)는 쓰지 않는다; 수식·이름이 쓰지 않는 외부 연결은 다시 열 때 버린다
    for l in &mut w.external_links {
        l.sheet_data.clear();
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
    // 외곽선 종류(한컴)가 있으면 ECMA 외곽선도 켜진 것으로 읽힌다 (실선이 아니어도)
    for f in &mut w.styles.fonts {
        if f.hancom
            .as_ref()
            .and_then(|h| h.outline_type.as_deref())
            .is_some_and(|t| t != "none")
        {
            f.outline = true;
        }
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
        // 선의 두께·모양이 지정되지 않은 도형은 기본값(9525 EMU, 실선)으로 읽힌다
        fn lines(v: &mut [crate::model::Shape]) {
            for s in v {
                // 앵커 오프셋은 정수 픽셀로 올려 저장된다
                let snap = |off: &mut i64| {
                    *off = super::art::anchor_px(*off) * crate::units::EMU_PER_PX;
                };
                snap(&mut s.anchor.from.col_off);
                snap(&mut s.anchor.from.row_off);
                if let Some(t) = &mut s.anchor.to {
                    snap(&mut t.col_off);
                    snap(&mut t.row_off);
                }
                // 그림의 채우기·선은 한셀처럼 '없음' 으로 쓴다
                if s.kind == ShapeKind::Picture {
                    s.fill.get_or_insert(Paint::None);
                    s.line.get_or_insert(crate::model::Outline {
                        width: None,
                        fill: Some(Paint::None),
                        dash: None,
                    });
                }
                if let Some(l) = &mut s.line {
                    l.width.get_or_insert(9525);
                    l.dash.get_or_insert_with(|| "solid".into());
                    if l.fill.is_none() {
                        l.fill = Some(Paint::None);
                    }
                }
                lines(&mut s.children);
            }
        }
        lines(&mut s.shapes);
        if let Some(m) = &mut s.page.margins {
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
    super::reader::prune_external_links(&mut w);
    w
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
