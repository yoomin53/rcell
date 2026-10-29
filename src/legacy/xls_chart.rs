// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! `.xls` (BIFF8) 의 차트 부분 스트림 ([MS-XLS] 2.1.7.20.1 차트 시트 부분 스트림 꼴, BOF dt 0x20 … EOF).
//!
//! 한셀의 `.xls` 저장은 시트의 차트(레거시 0x7EF0)마다 OfficeArt 도형 + OBJ(ot 5) 뒤에 이 부분 스트림을 둔다.
//! 레코드는 몇 가지 틀뿐이다 — 차트 영역·축·범례의 서식은 늘 같고, 종류·묶음·3차원·계열(참조·캐시·채우기·선)·축
//! (눈금·이름표·지움·주 눈금선)·범례 자리·제목만 바뀐다 (한셀이 차트 문서 17개 · 차트 23개를 `.xls` 로 저장한 것).
//! [`write`] 는 차트 모델로 그 레코드들을 만든다. 규칙: docs/format/other-formats.md 5.4절.

use crate::model::{
    Cell, CellValue, Chart, ChartAxis, ChartData, ChartFill, ChartSeries, ChartShapeProps, ChartTx,
    Workbook,
};

type Recs = Vec<(u16, Vec<u8>)>;
/// 계열의 (항목 글, 값)
type SeriesData = (Vec<Option<String>>, Vec<Option<f64>>);

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap_or(0))
        .collect()
}

struct W(Recs);

impl W {
    fn r(&mut self, rt: u16, d: Vec<u8>) {
        self.0.push((rt, d));
    }
    fn h(&mut self, rt: u16, s: &str) {
        self.0.push((rt, hex(s)));
    }
    fn begin(&mut self) {
        self.0.push((0x1033, Vec::new()));
    }
    fn end(&mut self) {
        self.0.push((0x1034, Vec::new()));
    }
}

/// 수식 글 (`Sheet1!$C$2:$C$6`, `'시트 1'!$A$1`) → (시트 이름, 첫 칸 (행, 열, 행 상대, 열 상대), 끝 칸)
type At = (u32, u32, bool, bool);

fn parse_ref(f: &str) -> Option<(Option<String>, At, At)> {
    let f = f.trim().trim_start_matches('=');
    let (sheet, rest) = match f.rfind('!') {
        Some(i) => {
            let s = &f[..i];
            let s = s
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .map(|s| s.replace("''", "'"))
                .unwrap_or_else(|| s.to_string());
            (Some(s), &f[i + 1..])
        }
        None => (None, f),
    };
    let cell = |t: &str| -> Option<At> {
        let col_rel = !t.starts_with('$');
        let t = t.trim_start_matches('$');
        let split = t.find(|c: char| c.is_ascii_digit() || c == '$')?;
        let (letters, digits) = t.split_at(split);
        let row_rel = !digits.starts_with('$');
        let digits = digits.trim_start_matches('$');
        let r = crate::model::CellRef::parse(&format!("{letters}{digits}"))?;
        Some((r.row, r.col, row_rel, col_rel))
    };
    let (a, b) = match rest.split_once(':') {
        Some((a, b)) => (cell(a)?, cell(b)?),
        None => {
            let c = cell(rest)?;
            (c, c)
        }
    };
    Some((sheet, a, b))
}

/// BIFF8 열 칸: 열 14비트 + 열 상대 0x4000 + 행 상대 0x8000
fn col_field(c: &At) -> u16 {
    (c.1 as u16 & 0x3FFF) | if c.3 { 0x4000 } else { 0 } | if c.2 { 0x8000 } else { 0 }
}

/// 참조 수식 → PtgRef3d / PtgArea3d (참조형 0x3A / 0x3B)
fn ref_tokens(f: &str, xti: &dyn Fn(Option<&str>) -> Option<u16>) -> Option<Vec<u8>> {
    let (sheet, a, b) = parse_ref(f)?;
    let ixti = xti(sheet.as_deref())?;
    let mut t = Vec::new();
    if a == b {
        t.push(0x3A);
        t.extend_from_slice(&ixti.to_le_bytes());
        t.extend_from_slice(&(a.0 as u16).to_le_bytes());
        t.extend_from_slice(&col_field(&a).to_le_bytes());
    } else {
        t.push(0x3B);
        t.extend_from_slice(&ixti.to_le_bytes());
        t.extend_from_slice(&(a.0 as u16).to_le_bytes());
        t.extend_from_slice(&(b.0 as u16).to_le_bytes());
        t.extend_from_slice(&col_field(&a).to_le_bytes());
        t.extend_from_slice(&col_field(&b).to_le_bytes());
    }
    Some(t)
}

/// BRAI: id u8, rt u8 (1 글·없음, 2 참조), 플래그 u16 (fCustomIfmt), ifmt u16, cce u16 + 토큰
fn brai(id: u8, rt: u8, grbit: u16, tokens: &[u8]) -> Vec<u8> {
    let mut d = vec![id, rt];
    d.extend_from_slice(&grbit.to_le_bytes());
    d.extend_from_slice(&0u16.to_le_bytes());
    d.extend_from_slice(&(tokens.len() as u16).to_le_bytes());
    d.extend_from_slice(tokens);
    d
}

/// 내장 표시 형식 번호 (1–49, 일반은 0)
fn builtin_id(code: &str) -> Option<u16> {
    if code.eq_ignore_ascii_case("General") || code == "G/표준" {
        return Some(0);
    }
    (1..50)
        .find(|&i| crate::numfmt::builtin(i) == Some(code))
        .map(|i| i as u16)
}

/// UTF-16 글 (플래그 1)
fn wide(s: &str) -> (usize, Vec<u8>) {
    let u: Vec<u16> = s.encode_utf16().take(255).collect();
    let mut b = Vec::with_capacity(u.len() * 2);
    for c in &u {
        b.extend_from_slice(&c.to_le_bytes());
    }
    (u.len(), b)
}

/// SeriesText: 예약 u16, cch u8, 플래그 1, UTF-16
fn series_text(s: &str) -> Vec<u8> {
    let (n, b) = wide(s);
    let mut d = vec![0, 0, n as u8, 1];
    d.extend_from_slice(&b);
    d
}

/// 단색 채우기 → RGB
fn solid_rgb(p: Option<&ChartShapeProps>) -> Option<u32> {
    match p?.fill.as_ref()? {
        ChartFill::Solid(c) => super::vtbuild::srgb_value(c),
        _ => None,
    }
}

/// 선 → (RGB, 두께 EMU). 채우기 없는 선은 None
fn line_rgb(p: Option<&ChartShapeProps>) -> Option<(u32, i64)> {
    let l = p?.line.as_ref()?;
    match l.fill.as_ref()? {
        ChartFill::Solid(c) => Some((super::vtbuild::srgb_value(c)?, l.width.unwrap_or(9525))),
        _ => None,
    }
}

/// LineFormat: rgb (r, g, b, 0), lns u16 (0 실선), we i16 (−1 가는 선, 0 보통, 1 중간, 2 굵게), 플래그 u16, icv u16
fn line_format(rgb: u32, we: i16, flags: u16, icv: u16) -> Vec<u8> {
    let mut d = vec![(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 0, 0, 0];
    d.extend_from_slice(&we.to_le_bytes());
    d.extend_from_slice(&flags.to_le_bytes());
    d.extend_from_slice(&icv.to_le_bytes());
    d
}

/// AreaFormat: 앞 색, 뒤 색 (rgb + 0), 무늬 u16 (1 단색), 플래그 u16, icv 앞·뒤
fn area_format(fore: u32, back: u32, flags: u16, icv_f: u16, icv_b: u16) -> Vec<u8> {
    let mut d = vec![
        (fore >> 16) as u8,
        (fore >> 8) as u8,
        fore as u8,
        0,
        (back >> 16) as u8,
        (back >> 8) as u8,
        back as u8,
        0,
    ];
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&flags.to_le_bytes());
    d.extend_from_slice(&icv_f.to_le_bytes());
    d.extend_from_slice(&icv_b.to_le_bytes());
    d
}

/// 축의 눈금 표시: none 0, in 1, out 2, cross 3
fn tick_mark(v: Option<&str>, default: u8) -> u8 {
    match v {
        Some("none") => 0,
        Some("in") => 1,
        Some("out") => 2,
        Some("cross") => 3,
        _ => default,
    }
}

/// 축 하나 (축 종류 0 항목 1 값 2 계열)
fn axis(w: &mut W, ax: Option<&ChartAxis>, kind: u16, scatter_x: bool) {
    w.r(0x101D, {
        let mut d = kind.to_le_bytes().to_vec();
        d.extend_from_slice(&[0; 16]);
        d
    });
    w.begin();
    if kind == 1 {
        let mut d = vec![0u8; 40];
        d.extend_from_slice(&0x000Fu16.to_le_bytes());
        w.r(0x101F, d);
    } else {
        w.h(0x0856, "56 08 00 00 64 00 02 00 00 00 00 00");
        if scatter_x {
            w.h(0x1020, "01 00 00 00 01 01 01 00");
        } else {
            w.h(0x1020, "01 00 01 00 01 00 01 00");
        }
        w.h(
            0x1062,
            "00 00 00 00 01 00 00 00 01 00 00 00 00 00 00 00 6f 00",
        );
    }
    // Tick: 주·보조 눈금, 이름표 자리 (none 0, low 1, high 2, nextTo 3), 배경 1 (투명), … 플래그 2, icv 8
    let major = tick_mark(ax.and_then(|a| a.major_tick_mark.as_deref()), 2);
    let minor = tick_mark(ax.and_then(|a| a.minor_tick_mark.as_deref()), 0);
    let tlt = match ax.and_then(|a| a.tick_lbl_pos.as_deref()) {
        Some("none") => 0,
        Some("low") => 1,
        Some("high") => 2,
        _ => 3,
    };
    let mut t = vec![major, minor, tlt, 1];
    t.extend_from_slice(&[0; 20]);
    t.extend_from_slice(&[2, 0, 8, 0, 0, 0]);
    w.r(0x101E, t);
    // 축 선 (지운 축은 플래그 0, 보이면 자동 + 축 보임 5) — 회색 808080, 가는 선
    let shown = !ax.and_then(|a| a.delete).unwrap_or(false);
    let flags = if shown { 5 } else { 0 };
    w.h(0x1021, "00 00");
    w.r(0x1007, line_format(0x80_8080, -1, flags, 0x17));
    if ax.is_some_and(|a| a.major_gridlines.is_some()) {
        w.h(0x1021, "01 00");
        w.r(0x1007, line_format(0x80_8080, -1, flags, 0x17));
    }
    w.end();
}

/// 참조 범위의 칸들 (행 우선). 시트 이름이 없으면 차트가 있는 시트
fn range_cells<'a>(wb: &'a Workbook, si: usize, f: &str) -> Option<Vec<Option<&'a Cell>>> {
    let (sheet, a, b) = parse_ref(f)?;
    let sheet = match sheet {
        Some(n) => wb.sheets.iter().find(|s| s.name == n)?,
        None => wb.sheets.get(si)?,
    };
    let mut v = Vec::new();
    for row in a.0.min(b.0)..=a.0.max(b.0) {
        for col in a.1.min(b.1)..=a.1.max(b.1) {
            v.push(sheet.cell(crate::model::CellRef { row, col }));
        }
    }
    Some(v)
}

/// 계열의 값: 캐시가 있으면 캐시, 없으면 참조하는 시트의 칸 (한셀 — 캐시 없는 차트는 칸 값을 쓴다)
fn texts(wb: &Workbook, si: usize, d: Option<&ChartData>) -> Vec<Option<String>> {
    let Some(d) = d else { return Vec::new() };
    if d.cache.is_none()
        && let Some(cells) = d.formula.as_deref().and_then(|f| range_cells(wb, si, f))
    {
        return cells
            .into_iter()
            .map(|c| {
                c.filter(|c| !c.value.is_empty())
                    .map(|c| crate::csv::shown(wb, c))
            })
            .collect();
    }
    d.dense()
}

fn numbers(wb: &Workbook, si: usize, d: Option<&ChartData>) -> Vec<Option<f64>> {
    let Some(d) = d else { return Vec::new() };
    if d.cache.is_none()
        && let Some(cells) = d.formula.as_deref().and_then(|f| range_cells(wb, si, f))
    {
        return cells
            .into_iter()
            .map(|c| match c.map(|c| &c.value) {
                Some(CellValue::Number(x)) => Some(*x),
                _ => None,
            })
            .collect();
    }
    d.dense()
        .into_iter()
        .map(|v| v.and_then(|v| v.trim().parse().ok()))
        .collect()
}

/// 차트 하나의 부분 스트림 (BOF … EOF). `si` 는 차트가 있는 시트, `xti` 는 수식의 시트 이름(없으면 그 시트) → ixti
pub(crate) fn write(
    ch: &Chart,
    wb: &Workbook,
    si: usize,
    xti: &dyn Fn(Option<&str>) -> Option<u16>,
) -> Recs {
    let mut w = W(Vec::new());
    w.h(0x0809, "00 06 20 00 bb 0d cc 07 00 00 00 00 06 00 00 00");
    w.r(0x0014, Vec::new());
    w.r(0x0015, Vec::new());
    w.h(0x0083, "00 00");
    w.h(0x0084, "00 00");
    let mut setup = vec![0u8; 42];
    setup[20] = 3;
    w.r(0x00A1, setup);
    w.r(0x1002, vec![0; 16]);
    w.begin();
    w.h(0x00A0, "01 00 01 00");
    w.h(0x1064, "00 00 01 00 00 00 01 00");
    // 차트 영역: 자동 선·채우기
    w.h(0x1032, "00 00 02 00");
    w.begin();
    w.h(0x1007, "00 00 00 00 00 00 00 00 09 00 08 00");
    w.h(0x100A, "ff ff ff 00 00 00 00 00 01 00 01 00 09 00 08 00");
    w.end();

    let series: &[ChartSeries] = &ch.series;
    let scatter = ch.kind == "scatter";
    let data: Vec<SeriesData> = series
        .iter()
        .map(|s| {
            (
                texts(wb, si, s.cat.as_ref()),
                numbers(wb, si, s.val.as_ref()),
            )
        })
        .collect();
    for (i, s) in series.iter().enumerate() {
        let n_cat = data[i].0.len();
        let n_val = data[i].1.len();
        let text_cats = s.cat.as_ref().is_some_and(|c| c.kind.starts_with("str"));
        let mut d = Vec::new();
        for v in [
            if text_cats { 3u16 } else { 1 },
            1,
            n_cat.max(n_val) as u16,
            n_val as u16,
            1,
            0,
        ] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        w.r(0x1003, d);
        w.begin();
        // 이름: 참조면 BRAI 참조 + SeriesText (캐시), 글이면 BRAI 의 PtgStr + SeriesText, 없으면 빈 BRAI
        match &s.tx {
            Some(ChartTx::Ref(r)) => {
                let tok = r.formula.as_deref().and_then(|f| ref_tokens(f, xti));
                match tok {
                    Some(t) => w.r(0x1051, brai(0, 2, 0, &t)),
                    None => w.r(0x1051, brai(0, 1, 0, &[])),
                }
                let name = texts(wb, si, Some(r))
                    .into_iter()
                    .next()
                    .flatten()
                    .or_else(|| s.name.clone())
                    .unwrap_or_default();
                w.r(0x100D, series_text(&name));
            }
            Some(ChartTx::Value(v)) => {
                let (n, b) = wide(v);
                let mut t = vec![0x17, n as u8, 1];
                t.extend_from_slice(&b);
                w.r(0x1051, brai(0, 1, 0, &t));
                w.r(0x100D, series_text(v));
            }
            _ => w.r(0x1051, brai(0, 1, 0, &[])),
        }
        let data = |d: Option<&ChartData>| -> Option<Vec<u8>> {
            d?.formula.as_deref().and_then(|f| ref_tokens(f, xti))
        };
        // 값·항목: 참조 (수식이 깨졌으면 토큰 없이), 항목이 없으면 종류 0. 값의 표시 형식은 캐시의 내장 형식 번호
        let ifmt = s
            .val
            .as_ref()
            .and_then(|v| v.cache.as_ref())
            .and_then(|c| c.format_code.as_deref())
            .and_then(builtin_id)
            .unwrap_or(0);
        match &s.val {
            Some(v) => {
                let mut b = brai(1, 2, 1, &data(Some(v)).unwrap_or_default());
                b[4..6].copy_from_slice(&ifmt.to_le_bytes());
                w.r(0x1051, b);
            }
            None => w.r(0x1051, brai(1, 1, 0, &[])),
        }
        match &s.cat {
            Some(c) => w.r(
                0x1051,
                brai(
                    2,
                    2,
                    u16::from(!text_cats),
                    &data(Some(c)).unwrap_or_default(),
                ),
            ),
            None => w.r(0x1051, brai(2, 0, 0, &[])),
        }
        w.r(0x1051, brai(3, 1, 0, &[]));
        let mut df = vec![0xFF, 0xFF];
        df.extend_from_slice(&(i as u16).to_le_bytes());
        df.extend_from_slice(&(i as u16).to_le_bytes());
        df.extend_from_slice(&[0, 0]);
        w.r(0x1006, df);
        w.begin();
        w.h(0x105F, "00 00");
        if let Some((rgb, width)) = line_rgb(s.sp_pr.as_ref()) {
            let we = match width {
                w if w <= 12700 => 0,
                w if w <= 25400 => 1,
                _ => 2,
            };
            w.r(
                0x1007,
                line_format(rgb, we, 0, super::writer::nearest_icv(rgb)),
            );
        }
        if let Some(rgb) = solid_rgb(s.sp_pr.as_ref()) {
            w.r(
                0x100A,
                area_format(rgb, 0, 0, super::writer::nearest_icv(rgb), 8),
            );
        }
        w.end();
        w.h(0x1045, "00 00");
        w.end();
    }
    w.h(0x1044, "01 00 00");
    w.h(0x1024, "02 00");
    w.h(
        0x1025,
        "02 02 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08 00 00 00 00 00",
    );
    w.begin();
    w.h(
        0x104F,
        "02 00 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00",
    );
    w.h(0x1051, "00 01 00 00 00 00 00 00");
    w.end();
    w.h(0x1046, "01 00");
    w.r(0x1041, vec![0; 18]);
    w.begin();
    w.h(
        0x104F,
        "02 00 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00",
    );
    let pie = matches!(ch.kind.as_str(), "pie" | "doughnut" | "ofPie");
    if !pie {
        let find = |k: &str| ch.axes.iter().find(|a| a.kind == k);
        if scatter {
            let vals: Vec<&ChartAxis> = ch.axes.iter().filter(|a| a.kind == "valAx").collect();
            axis(&mut w, vals.first().copied(), 0, true);
            axis(&mut w, vals.get(1).copied(), 1, false);
        } else {
            axis(&mut w, find("catAx").or_else(|| find("dateAx")), 0, false);
            axis(&mut w, find("valAx"), 1, false);
            if ch.three_d && ch.axes.iter().any(|a| a.kind == "serAx") {
                axis(&mut w, find("serAx"), 2, false);
            }
        }
    }
    w.r(0x1035, Vec::new());
    let mut cf = vec![0u8; 20];
    if ch.vary_colors {
        cf[16] = 1;
    }
    w.r(0x1014, cf);
    w.begin();
    match ch.kind.as_str() {
        "bar" => {
            let stacked = matches!(ch.grouping.as_deref(), Some("stacked" | "percentStacked"));
            let overlap = -ch.group.overlap.unwrap_or(0) as i16;
            let gap = ch.group.gap_width.unwrap_or(150) as u16;
            let mut flags = 0u16;
            if ch.bar_dir.as_deref() == Some("bar") {
                flags |= 1;
            }
            if stacked {
                flags |= 2;
            }
            if ch.grouping.as_deref() == Some("percentStacked") {
                flags |= 4;
            }
            let mut d = overlap.to_le_bytes().to_vec();
            d.extend_from_slice(&gap.to_le_bytes());
            d.extend_from_slice(&flags.to_le_bytes());
            w.r(0x1017, d);
        }
        "line" => w.h(0x1018, "00 00"),
        "area" => w.h(0x101A, "00 00"),
        "pie" => w.h(0x1019, "00 00 00 00 00 00"),
        "scatter" => w.h(0x101B, "64 00 01 00 00 00"),
        _ => w.h(0x1017, "00 00 96 00 00 00"),
    }
    w.h(0x1022, "00 00 00 00 00 00 00 00 0f 00");
    if ch.three_d {
        let v = ch.view3d.as_ref();
        let mut d = Vec::new();
        for x in [
            v.and_then(|v| v.rot_y).unwrap_or(20) as u16,
            v.and_then(|v| v.rot_x).unwrap_or(15) as u16,
            v.and_then(|v| v.perspective).unwrap_or(30) as u16,
            v.and_then(|v| v.h_percent).unwrap_or(100) as u16,
            v.and_then(|v| v.depth_percent).unwrap_or(100) as u16,
            ch.group.gap_depth.unwrap_or(150) as u16,
            0x17,
        ] {
            d.extend_from_slice(&x.to_le_bytes());
        }
        w.r(0x103A, d);
    }
    if let Some(pos) = ch.legend.as_deref() {
        // Legend: x, y, dx, dy (0x0A, 0x0A, 0x64, 0x19), 자리 (0 아래, 1 구석, 2 위, 3 오른쪽, 4 왼쪽), 간격 1, 플래그
        let (wtype, flags) = match pos {
            "b" => (0u8, 0x000Fu16),
            "tr" => (1, 0x001F),
            "t" => (2, 0x000F),
            "l" => (4, 0x001F),
            _ => (3, 0x001F),
        };
        let mut d = Vec::new();
        for v in [0x0Au32, 0x0A, 0x64, 0x19] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[wtype, 1]);
        d.extend_from_slice(&flags.to_le_bytes());
        w.r(0x1015, d);
        w.begin();
        w.h(
            0x104F,
            "05 00 02 00 0a 00 00 00 0a 00 00 00 64 00 00 00 19 00 00 00",
        );
        w.h(0x1032, "00 00 02 00");
        w.begin();
        w.h(0x1007, "00 00 00 00 05 00 00 00 09 00 08 00");
        w.end();
        w.end();
    }
    if scatter {
        w.h(0x1006, "00 00 00 00 00 00 00 00");
        w.begin();
        w.h(0x105F, "00 00");
        w.end();
    }
    w.end();
    w.end();
    // 제목
    if ch.auto_title_deleted != Some(true)
        && let Some(t) = ch.title.as_deref().filter(|t| !t.is_empty())
    {
        w.h(
            0x1025,
            "02 02 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 91 00 08 00 09 00 00 00",
        );
        w.begin();
        w.h(
            0x104F,
            "02 00 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00",
        );
        w.h(0x1051, "00 01 00 00 00 00 00 00");
        w.r(0x100D, series_text(t));
        w.h(0x1027, "01 00 00 00 00 00");
        w.end();
    }
    w.end();
    // 값 캐시: 행 = 점, 열 = 계열. 항목은 LABEL (SIIndex 2), 값은 NUMBER (SIIndex 1)
    let rows = data
        .iter()
        .map(|(c, v)| c.len().max(v.len()))
        .max()
        .unwrap_or(0);
    let mut dim = vec![0, 0, 0, 0];
    dim.extend_from_slice(&(rows as u32).to_le_bytes());
    dim.extend_from_slice(&0u16.to_le_bytes());
    dim.extend_from_slice(&(series.len() as u16).to_le_bytes());
    dim.extend_from_slice(&[0, 0]);
    w.r(0x0200, dim);
    if series.iter().any(|s| s.cat.is_some()) {
        w.h(0x1065, "02 00");
    }
    for r in 0..rows {
        for (c, (cats, _)) in data.iter().enumerate() {
            if let Some(Some(v)) = cats.get(r) {
                let (n, b) = wide(v);
                let mut d = Vec::new();
                d.extend_from_slice(&(r as u16).to_le_bytes());
                d.extend_from_slice(&(c as u16).to_le_bytes());
                d.extend_from_slice(&0u16.to_le_bytes());
                d.extend_from_slice(&(n as u16).to_le_bytes());
                d.push(1);
                d.extend_from_slice(&b);
                w.r(0x0204, d);
            }
        }
    }
    w.h(0x1065, "01 00");
    for r in 0..rows {
        for (c, (_, vals)) in data.iter().enumerate() {
            if let Some(Some(x)) = vals.get(r) {
                let mut d = Vec::new();
                d.extend_from_slice(&(r as u16).to_le_bytes());
                d.extend_from_slice(&(c as u16).to_le_bytes());
                d.extend_from_slice(&0u16.to_le_bytes());
                d.extend_from_slice(&x.to_le_bytes());
                w.r(0x0203, d);
            }
        }
    }
    w.h(0x7EF8, "ff ff ff ff");
    w.r(0x000A, Vec::new());
    w.0
}
