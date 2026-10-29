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

// ---------------------------------------------------------------------------
// 읽기: 차트 부분 스트림 → 차트 모델 (한셀이 .xls 를 열어 XML 로 쓰는 차트의 꼴로 만든 뒤 XML 차트 리더로 읽는다)
// ---------------------------------------------------------------------------

/// 수식의 시트 이름과 글꼴: ixti → 시트 이름, 문서 기본 글꼴 (이름, 1/100 pt)
pub(crate) struct ReadCtx<'a> {
    pub xti_names: &'a [Option<String>],
    pub font: (String, u32),
    pub palette: &'a [u32],
}

#[derive(Default)]
struct RSeries {
    sdt_x: u16,
    name_ref: Option<String>,
    name_lit: Option<String>,
    name_text: Option<String>,
    val: Option<(Option<String>, u16)>,
    cat: Option<Option<String>>,
    fill: Option<u16>,
    line: Option<(i16, u16)>,
}

#[derive(Default)]
struct RAxis {
    kind: u16,
    tlt: u8,
    shown: bool,
    gridlines: bool,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// BIFF8 차트 수식 (PtgRef3d / PtgArea3d 하나) → `시트!$A$1:$B$2`
fn ref_text(t: &[u8], ctx: &ReadCtx) -> Option<String> {
    let u = |k: usize| -> Option<u16> { Some(u16::from_le_bytes([*t.get(k)?, *t.get(k + 1)?])) };
    let cell = |v: u16, row: u16| -> String {
        let c = crate::model::CellRef {
            row: row as u32,
            col: (v & 0x3FFF) as u32,
        }
        .to_string();
        let split = c.find(|ch: char| ch.is_ascii_digit()).unwrap_or(c.len());
        let (l, d) = c.split_at(split);
        format!(
            "{}{l}{}{d}",
            if v & 0x4000 != 0 { "" } else { "$" },
            if v & 0x8000 != 0 { "" } else { "$" }
        )
    };
    let sheet = |ixti: u16| -> Option<String> {
        let n = ctx.xti_names.get(ixti as usize)?.as_deref()?;
        Some(super::formula::quote_sheet(n))
    };
    match *t.first()? {
        0x3A | 0x5A | 0x7A => Some(format!("{}!{}", sheet(u(1)?)?, cell(u(5)?, u(3)?))),
        0x3B | 0x5B | 0x7B => Some(format!(
            "{}!{}:{}",
            sheet(u(1)?)?,
            cell(u(7)?, u(3)?),
            cell(u(9)?, u(5)?)
        )),
        _ => None,
    }
}

/// PtgStr (cch u8, 플래그, 글자)
fn ptg_str(t: &[u8]) -> Option<String> {
    if *t.first()? != 0x17 {
        return None;
    }
    Some(xl_string(t.get(2..)?, *t.get(1)? as usize))
}

/// 플래그 + 글자 (플래그 1 이면 UTF-16)
fn xl_string(d: &[u8], cch: usize) -> String {
    match d.first() {
        Some(f) if f & 1 != 0 => {
            let u: Vec<u16> = d
                .get(1..1 + cch * 2)
                .unwrap_or_default()
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&u)
        }
        _ => d
            .get(1..1 + cch)
            .unwrap_or_default()
            .iter()
            .map(|&b| b as char)
            .collect(),
    }
}

/// 글자 속성 (`a:defRPr` / `a:rPr`): 크기, 보통, 검정, 글꼴
fn rich_props(tag: &str, sz: u32, face: &str) -> String {
    let face = esc(face);
    format!(
        r#"<{tag} sz="{sz}" b="0" i="0" u="none"><a:solidFill><a:srgbClr val="000000"/></a:solidFill><a:latin typeface="{face}"/><a:ea typeface="{face}"/><a:cs typeface="{face}"/><a:sym typeface="{face}"/></{tag}>"#
    )
}

const BODY_PR: &str = r#"<a:bodyPr rot="0" vert="horz" wrap="none" lIns="0" tIns="0" rIns="0" bIns="0" anchor="ctr" anchorCtr="1"/>"#;
const LN_TAIL: &str = r#"<a:prstDash val="solid"/><a:round/><a:headEnd w="med" len="med"/><a:tailEnd w="med" len="med"/></a:ln>"#;
const DLBLS: &str = r#"<c:dLbls><c:delete val="0"/><c:showLegendKey val="0"/><c:showVal val="0"/><c:showCatName val="0"/><c:showSerName val="0"/><c:showPercent val="0"/><c:showBubbleSize val="0"/><c:showLeaderLines val="0"/></c:dLbls>"#;

/// 팔레트 색 번호 → `rrggbb`
fn palette_hex(icv: u16, palette: &[u32]) -> String {
    let rgb = match icv {
        8..=63 => palette.get(icv as usize - 8).copied().unwrap_or(0),
        _ => 0,
    };
    format!("{rgb:06x}")
}

/// 차트 부분 스트림의 해석 결과
#[derive(Default)]
struct Parsed {
    series: Vec<RSeries>,
    axes: Vec<RAxis>,
    kind: String,
    bar: (i16, u16),
    vary: bool,
    three_d: bool,
    legend: Option<u8>,
    title: Option<String>,
    labels: std::collections::BTreeMap<(u16, u16), String>,
    numbers: std::collections::BTreeMap<(u16, u16), f64>,
}

fn u16_at(d: &[u8], k: usize) -> u16 {
    d.get(k..k + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .unwrap_or(0)
}

fn parse(recs: &[(u16, Vec<u8>)], ctx: &ReadCtx) -> Parsed {
    let mut p = Parsed::default();
    let mut stack: Vec<u16> = Vec::new();
    let mut last = 0u16;
    let mut axis_line = 0u16;
    let mut text_buf: Option<String> = None;
    let mut si_index = 0u16;
    for (rt, d) in recs.iter().skip(1) {
        let rt = *rt;
        let parent = stack.last().copied();
        let in_series_format =
            parent == Some(0x1006) && stack.len() >= 2 && stack[stack.len() - 2] == 0x1003;
        match rt {
            0x1033 => {
                stack.push(last);
                continue;
            }
            0x1034 => {
                if stack.pop() == Some(0x1025)
                    && let Some(t) = text_buf.take()
                {
                    p.title = Some(t);
                }
                continue;
            }
            0x1003 => p.series.push(RSeries {
                sdt_x: u16_at(d, 0),
                ..Default::default()
            }),
            0x1051 if parent == Some(0x1003) && d.len() >= 8 => {
                if let Some(s) = p.series.last_mut() {
                    let cce = u16_at(d, 6) as usize;
                    let toks = d.get(8..8 + cce).unwrap_or_default();
                    let f = ref_text(toks, ctx);
                    match d[0] {
                        0 => {
                            s.name_ref = f;
                            s.name_lit = ptg_str(toks);
                        }
                        1 => s.val = Some((f, u16_at(d, 4))),
                        2 if d[1] != 0 => s.cat = Some(f),
                        _ => {}
                    }
                }
            }
            0x100D if parent == Some(0x1003) && d.len() >= 3 => {
                if let Some(s) = p.series.last_mut() {
                    s.name_text = Some(xl_string(&d[3..], d[2] as usize));
                }
            }
            0x100D if parent == Some(0x1025) && d.len() >= 3 => {
                text_buf = Some(xl_string(&d[3..], d[2] as usize));
            }
            0x1027 if parent == Some(0x1025) => {
                if u16_at(d, 0) != 1 {
                    text_buf = None;
                }
            }
            0x100A if in_series_format => {
                if let Some(s) = p.series.last_mut()
                    && u16_at(d, 10) & 1 == 0
                {
                    s.fill = Some(u16_at(d, 12));
                }
            }
            0x1007 if in_series_format => {
                if let Some(s) = p.series.last_mut()
                    && u16_at(d, 8) & 1 == 0
                {
                    s.line = Some((u16_at(d, 6) as i16, u16_at(d, 10)));
                }
            }
            0x101D => p.axes.push(RAxis {
                kind: u16_at(d, 0),
                tlt: 3,
                ..Default::default()
            }),
            0x101E if parent == Some(0x101D) => {
                if let Some(a) = p.axes.last_mut() {
                    a.tlt = d.get(2).copied().unwrap_or(3);
                }
            }
            0x1021 if parent == Some(0x101D) => axis_line = u16_at(d, 0),
            0x1007 if parent == Some(0x101D) => {
                if let Some(a) = p.axes.last_mut() {
                    match axis_line {
                        0 => a.shown = u16_at(d, 8) & 0x0004 != 0,
                        1 => a.gridlines = true,
                        _ => {}
                    }
                }
            }
            0x1014 => p.vary = u16_at(d, 16) & 1 != 0,
            0x1017 => {
                p.kind = if u16_at(d, 4) & 1 != 0 { "barh" } else { "bar" }.into();
                p.bar = (u16_at(d, 0) as i16, u16_at(d, 4));
            }
            0x1018 => p.kind = "line".into(),
            0x101A => p.kind = "area".into(),
            0x1019 => p.kind = "pie".into(),
            0x101B => p.kind = "scatter".into(),
            0x103A => p.three_d = true,
            0x1015 => p.legend = Some(d.get(16).copied().unwrap_or(3)),
            0x1065 => si_index = u16_at(d, 0),
            0x0204 if d.len() >= 8 => {
                let n = u16_at(d, 6) as usize;
                p.labels
                    .insert((u16_at(d, 0), u16_at(d, 2)), xl_string(&d[8..], n));
            }
            0x0203 if si_index == 1 => {
                if let Some(b) = d.get(6..14) {
                    let mut a = [0u8; 8];
                    a.copy_from_slice(b);
                    p.numbers
                        .insert((u16_at(d, 0), u16_at(d, 2)), f64::from_le_bytes(a));
                }
            }
            _ => {}
        }
        last = rt;
    }
    p
}

/// 차트 부분 스트림 (BOF … EOF 의 레코드들) → 차트 XML (한셀이 .xls 를 열어 XML 로 쓰는 꼴: 차트 영역 흰 채우기 +
/// 검정 1pt 선, 그림 영역 없음, 축 글자 = 문서 기본 글꼴, 제목 10pt, 3차원 회전은 기본 15·20)
pub(crate) fn read_xml(recs: &[(u16, Vec<u8>)], ctx: &ReadCtx) -> Option<String> {
    let p = parse(recs, ctx);
    if p.kind.is_empty() {
        return None;
    }
    let (face, axis_sz) = (&ctx.font.0, ctx.font.1);
    let kind = if p.kind == "barh" {
        "bar"
    } else {
        p.kind.as_str()
    };
    let scatter = kind == "scatter";
    let mut x = String::new();
    x.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><c:chartSpace xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:date1904 val="0"/><c:roundedCorners val="0"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:c14="http://schemas.microsoft.com/office/drawing/2007/8/2/chart" Requires="c14"><c14:style val="102"/></mc:Choice><mc:Fallback><c:style val="2"/></mc:Fallback></mc:AlternateContent><c:chart>"#);
    if let Some(t) = &p.title {
        x.push_str(&format!(
            r#"<c:title><c:tx><c:rich>{BODY_PR}<a:p><a:pPr algn="l">{}</a:pPr><a:r>{}<a:t>{}</a:t></a:r><a:endParaRPr/></a:p></c:rich></c:tx><c:layout><c:manualLayout><c:xMode val="edge"/><c:yMode val="edge"/></c:manualLayout></c:layout><c:overlay val="0"/></c:title>"#,
            rich_props("a:defRPr", 1000, face),
            rich_props("a:rPr", 1000, face),
            esc(t)
        ));
    }
    x.push_str(r#"<c:autoTitleDeleted val="0"/>"#);
    if p.three_d {
        x.push_str(r#"<c:view3D><c:rAngAx val="1"/><c:rotX val="15"/><c:rotY val="20"/><c:perspective val="30"/><c:depthPercent val="100"/></c:view3D>"#);
    }
    x.push_str("<c:plotArea><c:layout/>");
    let tag = match (kind, p.three_d) {
        ("bar", true) => "c:bar3DChart",
        ("bar", false) => "c:barChart",
        ("line", _) => "c:lineChart",
        ("area", _) => "c:areaChart",
        ("pie", _) => "c:pieChart",
        _ => "c:scatterChart",
    };
    x.push_str(&format!("<{tag}>"));
    match kind {
        "bar" => {
            let dir = if p.bar.1 & 1 != 0 { "bar" } else { "col" };
            let grp = if p.bar.1 & 4 != 0 {
                "percentStacked"
            } else if p.bar.1 & 2 != 0 {
                "stacked"
            } else {
                "clustered"
            };
            x.push_str(&format!(
                r#"<c:barDir val="{dir}"/><c:grouping val="{grp}"/>"#
            ));
        }
        "line" | "area" => x.push_str(r#"<c:grouping val="standard"/>"#),
        "scatter" => x.push_str(r#"<c:scatterStyle val="marker"/>"#),
        _ => {}
    }
    x.push_str(&format!(r#"<c:varyColors val="{}"/>"#, u8::from(p.vary)));
    let n_rows = p
        .labels
        .keys()
        .chain(p.numbers.keys())
        .map(|k| k.0 + 1)
        .max()
        .unwrap_or(0);
    // 값 참조가 없는 계열(#REF! 등)은 한셀이 버린다
    let series: Vec<(usize, &RSeries)> = p
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| s.val.as_ref().is_some_and(|v| v.0.is_some()))
        .collect();
    for (i, (orig, s)) in series.iter().enumerate() {
        x.push_str(&format!(r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/>"#));
        if let Some(f) = &s.name_ref {
            x.push_str(&format!(
                r#"<c:tx><c:strRef><c:f>{}</c:f><c:strCache><c:ptCount val="1"/>"#,
                esc(f)
            ));
            if let Some(t) = &s.name_text {
                x.push_str(&format!(r#"<c:pt idx="0"><c:v>{}</c:v></c:pt>"#, esc(t)));
            }
            x.push_str("</c:strCache></c:strRef></c:tx>");
        } else if let Some(t) = s.name_lit.as_ref().or(s.name_text.as_ref()) {
            x.push_str(&format!("<c:tx><c:v>{}</c:v></c:tx>", esc(t)));
        }
        x.push_str("<c:spPr>");
        if let Some(icv) = s.fill {
            x.push_str(&format!(
                r#"<a:solidFill><a:srgbClr val="{}"/></a:solidFill>"#,
                palette_hex(icv, ctx.palette)
            ));
        }
        match s.line {
            Some((we, icv)) => {
                let w = match we {
                    1 => 25400,
                    2 => 38100,
                    -1 => 3175,
                    _ => 12700,
                };
                x.push_str(&format!(
                    r#"<a:ln w="{w}" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:srgbClr val="{}"/></a:solidFill>{LN_TAIL}"#,
                    palette_hex(icv, ctx.palette)
                ));
            }
            None => x.push_str(&format!(
                r#"<a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:prstClr val="black"/></a:solidFill>{LN_TAIL}"#
            )),
        }
        x.push_str("</c:spPr>");
        match kind {
            "bar" => x.push_str(r#"<c:invertIfNegative val="0"/>"#),
            "line" => x.push_str("<c:marker/>"),
            "pie" => x.push_str(r#"<c:explosion val="0"/>"#),
            _ => {}
        }
        if kind != "bar" {
            x.push_str(DLBLS);
        }
        let col = *orig as u16;
        if let Some(f) = &s.cat {
            let (open, close) = if scatter {
                ("<c:xVal>", "</c:xVal>")
            } else {
                ("<c:cat>", "</c:cat>")
            };
            x.push_str(open);
            let pts: Vec<(u16, &String)> = (0..n_rows)
                .filter_map(|r| p.labels.get(&(r, col)).map(|v| (r, v)))
                .collect();
            let f = esc(f.as_deref().unwrap_or(""));
            if s.sdt_x == 3 {
                x.push_str(&format!(
                    r#"<c:strRef><c:f>{f}</c:f><c:strCache><c:ptCount val="{n_rows}"/>"#
                ));
                for (r, v) in &pts {
                    x.push_str(&format!(r#"<c:pt idx="{r}"><c:v>{}</c:v></c:pt>"#, esc(v)));
                }
                x.push_str("</c:strCache></c:strRef>");
            } else {
                x.push_str(&format!(
                    r#"<c:numRef><c:f>{f}</c:f><c:numCache><c:formatCode>G/표준</c:formatCode><c:ptCount val="{n_rows}"/>"#
                ));
                for (r, v) in &pts {
                    x.push_str(&format!(r#"<c:pt idx="{r}"><c:v>{}</c:v></c:pt>"#, esc(v)));
                }
                x.push_str("</c:numCache></c:numRef>");
            }
            x.push_str(close);
        }
        if let Some((f, ifmt)) = &s.val {
            let (open, close) = if scatter {
                ("<c:yVal>", "</c:yVal>")
            } else {
                ("<c:val>", "</c:val>")
            };
            let code = match ifmt {
                0 => "G/표준",
                n => crate::numfmt::builtin(*n as u32).unwrap_or("G/표준"),
            };
            let code = esc(code);
            x.push_str(&format!(
                r#"{open}<c:numRef><c:f>{}</c:f><c:numCache><c:formatCode>{code}</c:formatCode><c:ptCount val="{n_rows}"/>"#,
                esc(f.as_deref().unwrap_or(""))
            ));
            for r in 0..n_rows {
                if let Some(v) = p.numbers.get(&(r, col)) {
                    x.push_str(&format!(
                        r#"<c:pt idx="{r}" formatCode="{code}"><c:v>{v}</c:v></c:pt>"#
                    ));
                }
            }
            x.push_str(&format!("</c:numCache></c:numRef>{close}"));
        }
        if matches!(kind, "line" | "scatter") {
            x.push_str(r#"<c:smooth val="0"/>"#);
        }
        x.push_str("</c:ser>");
    }
    match kind {
        "bar" => {
            x.push_str(r#"<c:gapWidth val="150"/>"#);
            if p.three_d {
                x.push_str(r#"<c:gapDepth val="150"/><c:shape val="box"/>"#);
            } else {
                x.push_str(&format!(r#"<c:overlap val="{}"/>"#, -(p.bar.0 as i32)));
            }
        }
        "line" => x.push_str(r#"<c:marker val="1"/><c:smooth val="0"/>"#),
        "pie" => x.push_str(r#"<c:firstSliceAng val="0"/>"#),
        _ => {}
    }
    if kind != "pie" {
        x.push_str(r#"<c:axId val="0"/><c:axId val="1"/>"#);
        if p.three_d {
            x.push_str(r#"<c:axId val="2"/>"#);
        }
    }
    x.push_str(&format!("</{tag}>"));
    if kind != "pie" {
        let tx_pr = format!(
            r#"<c:txPr>{BODY_PR}<a:p><a:pPr algn="l">{}</a:pPr><a:endParaRPr/></a:p></c:txPr>"#,
            rich_props("a:defRPr", axis_sz, face)
        );
        for a in &p.axes {
            let tlt = match a.tlt {
                0 => "none",
                1 => "low",
                2 => "high",
                _ => "nextTo",
            };
            let del = u8::from(!a.shown);
            let grid = if a.gridlines {
                "<c:majorGridlines/>"
            } else {
                ""
            };
            // 누적 막대(fStacked)면 한셀은 축마다 0% 표시 형식을 붙인다 (원본에 연결 안 함)
            let stacked = kind == "bar" && p.bar.1 & 2 != 0;
            let pct = if stacked {
                r#"<c:numFmt formatCode="0%" sourceLinked="0"/>"#
            } else {
                ""
            };
            let common = |id: u16, pos: &str, cross: u16| -> String {
                format!(
                    r#"<c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="{pos}"/><c:crossAx val="{cross}"/><c:delete val="{del}"/>{grid}"#
                )
            };
            let ticks = format!(
                r#"<c:majorTickMark val="out"/><c:minorTickMark val="none"/><c:tickLblPos val="{tlt}"/>{tx_pr}<c:crosses val="autoZero"/>"#
            );
            match (a.kind, scatter) {
                (0, true) => x.push_str(&format!(
                    r#"<c:valAx>{}<c:numFmt formatCode="General" sourceLinked="1"/>{ticks}<c:crossBetween val="between"/></c:valAx>"#,
                    common(0, "b", 1)
                )),
                (0, false) => x.push_str(&format!(
                    r#"<c:catAx>{}{pct}{ticks}<c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/><c:tickLblSkip val="1"/><c:tickMarkSkip val="1"/><c:noMultiLvlLbl val="0"/></c:catAx>"#,
                    common(0, "b", 1)
                )),
                (1, _) => x.push_str(&format!(
                    r#"<c:valAx>{}{}{ticks}<c:crossBetween val="between"/></c:valAx>"#,
                    common(1, "l", 0),
                    if stacked {
                        pct
                    } else {
                        r#"<c:numFmt formatCode="G/표준" sourceLinked="1"/>"#
                    }
                )),
                _ => x.push_str(&format!(
                    r#"<c:serAx>{}{pct}{ticks}<c:tickMarkSkip val="1"/></c:serAx>"#,
                    common(2, "b", 1)
                )),
            }
        }
    }
    x.push_str(&format!(
        r#"<c:spPr><a:noFill/><a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:noFill/>{LN_TAIL}</c:spPr></c:plotArea>"#
    ));
    if let Some(w) = p.legend {
        let pos = match w {
            0 => "b",
            1 => "tr",
            2 => "t",
            4 => "l",
            _ => "r",
        };
        x.push_str(&format!(
            r#"<c:legend><c:legendPos val="{pos}"/><c:layout/><c:overlay val="0"/><c:spPr><a:solidFill><a:srgbClr val="ffffff"/></a:solidFill></c:spPr></c:legend>"#
        ));
    }
    x.push_str(r#"<c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart>"#);
    x.push_str(&format!(
        r#"<c:spPr><a:solidFill><a:srgbClr val="ffffff"/></a:solidFill><a:ln w="12700" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:srgbClr val="000000"/></a:solidFill>{LN_TAIL}</c:spPr>"#
    ));
    x.push_str(r#"<c:extLst><c:ext uri="CC8EB2C9-7E31-499d-B8F2-F6CE61031016"><ho:hncChartStyle xmlns:ho="http://schemas.haansoft.com/office/8.0" layoutIndex="-1" colorIndex="0" styleIndex="0"/></c:ext></c:extLst></c:chartSpace>"#);
    Some(x)
}

/// [`read_xml`] → 차트 모델
pub(crate) fn read(recs: &[(u16, Vec<u8>)], ctx: &ReadCtx) -> Option<Chart> {
    let xml = read_xml(recs, ctx)?;
    let root = crate::xml::parse_raw(&xml, "xl/charts/chart.xml").ok()?;
    Some(crate::parser::chart::parse_chart(&root))
}

/// 칸의 표시 형식 글 (한셀이 차트 캐시에 쓰는 꼴 — `General` 은 `G/표준`, 따옴표·`` 를 벗기고 `[빨강]`)
fn cell_format_code(wb: &Workbook, cell: Option<&Cell>) -> String {
    let id = cell
        .and_then(|c| wb.styles.cell_xfs.get(c.style as usize))
        .map_or(0, |x| x.num_fmt_id);
    if id == 0 {
        return "G/표준".into();
    }
    let code = wb
        .styles
        .num_fmts
        .iter()
        .find(|f| f.id == id)
        .map(|f| f.code.clone())
        .or_else(|| crate::numfmt::builtin(id).map(str::to_string))
        .unwrap_or_else(|| "General".into());
    super::vtchart_map::chart_format_code(&code)
}

/// .xls 에서 읽은 차트의 값 캐시 표시 형식: 한셀은 값 범위의 칸 서식을 쓴다 (캐시 전체는 첫 칸, 점마다는 그 칸)
pub(crate) fn apply_cell_formats(wb: &mut Workbook) {
    let mut fixes: Vec<(usize, usize, usize, Vec<String>)> = Vec::new();
    for (si, sheet) in wb.sheets.iter().enumerate() {
        for (k, shape) in sheet.shapes.iter().enumerate() {
            let Some(ch) = &shape.chart else { continue };
            for (i, s) in ch.series.iter().enumerate() {
                let Some(cells) = s
                    .val
                    .as_ref()
                    .and_then(|v| v.formula.as_deref())
                    .and_then(|f| range_cells(wb, si, f))
                else {
                    continue;
                };
                fixes.push((
                    si,
                    k,
                    i,
                    cells.iter().map(|c| cell_format_code(wb, *c)).collect(),
                ));
            }
        }
    }
    for (si, k, i, codes) in fixes {
        let Some(cache) = wb.sheets[si].shapes[k]
            .chart
            .as_mut()
            .and_then(|c| c.series.get_mut(i))
            .and_then(|s| s.val.as_mut())
            .and_then(|v| v.cache.as_mut())
        else {
            continue;
        };
        if let Some(first) = codes.first() {
            cache.format_code = Some(first.clone());
        }
        for p in &mut cache.points {
            if let Some(c) = codes.get(p.idx as usize) {
                p.format_code = Some(c.clone());
            }
        }
    }
}

/// 캐시가 없는 차트 계열 참조(이름·항목·값)에 시트 칸의 값으로 캐시를 채운 사본 — 한셀은 캐시가 없는 차트를 저장할 때
/// 칸 값을 쓴다 (레거시 차트의 데이터 표·계열 이름, .xls 의 값 캐시). 채울 것이 없으면 None
pub(crate) fn fill_caches(wb: &Workbook) -> Option<Workbook> {
    fn missing(d: Option<&ChartData>) -> bool {
        d.is_some_and(|d| d.cache.is_none() && d.formula.is_some())
    }
    fn needs(s: &crate::model::Shape) -> bool {
        s.chart.as_ref().is_some_and(|c| {
            c.series.iter().any(|x| {
                missing(x.cat.as_ref())
                    || missing(x.val.as_ref())
                    || matches!(&x.tx, Some(ChartTx::Ref(r)) if r.cache.is_none() && r.formula.is_some())
            })
        }) || s.children.iter().any(needs)
    }
    if !wb.sheets.iter().any(|s| s.shapes.iter().any(needs)) {
        return None;
    }
    let mut out = wb.clone();
    for si in 0..out.sheets.len() {
        let mut shapes = std::mem::take(&mut out.sheets[si].shapes);
        fn fill(shapes: &mut [crate::model::Shape], wb: &Workbook, si: usize) {
            for sh in shapes {
                fill(&mut sh.children, wb, si);
                let Some(ch) = sh.chart.as_mut() else {
                    continue;
                };
                for s in &mut ch.series {
                    if let Some(ChartTx::Ref(r)) = &mut s.tx
                        && r.cache.is_none()
                        && let Some(cells) =
                            r.formula.as_deref().and_then(|f| range_cells(wb, si, f))
                    {
                        r.cache = Some(cache_of(wb, &cells, false));
                    }
                    for d in [s.cat.as_mut(), s.val.as_mut()].into_iter().flatten() {
                        if d.cache.is_none()
                            && let Some(cells) =
                                d.formula.as_deref().and_then(|f| range_cells(wb, si, f))
                        {
                            d.cache = Some(cache_of(wb, &cells, d.kind.starts_with("num")));
                        }
                    }
                    // 요약 칸 (레거시 차트의 데이터 표가 쓴다): 이름, 항목 글, 값
                    if s.name.is_none()
                        && let Some(ChartTx::Ref(r)) = &s.tx
                    {
                        s.name = r
                            .cache
                            .as_ref()
                            .and_then(|c| c.points.first())
                            .map(|p| p.value.clone());
                    }
                    if s.categories.is_empty()
                        && let Some(d) = &s.cat
                    {
                        s.categories = d
                            .dense()
                            .into_iter()
                            .map(Option::unwrap_or_default)
                            .collect();
                    }
                    if s.values.is_empty()
                        && let Some(d) = &s.val
                    {
                        s.values = d
                            .dense()
                            .into_iter()
                            .map(|v| v.and_then(|t| t.trim().parse().ok()))
                            .collect();
                    }
                }
            }
        }
        fill(&mut shapes, wb, si);
        out.sheets[si].shapes = shapes;
    }
    Some(out)
}

/// 칸들 → 차트 캐시 (수 캐시는 칸 서식의 표시 형식, 점마다도)
fn cache_of(wb: &Workbook, cells: &[Option<&Cell>], numeric: bool) -> crate::model::ChartCache {
    let mut points = Vec::new();
    for (i, c) in cells.iter().enumerate() {
        let Some(c) = c.filter(|c| !c.value.is_empty()) else {
            continue;
        };
        let value = match &c.value {
            CellValue::Number(x) if numeric => format!("{x}"),
            _ => crate::csv::shown(wb, c),
        };
        points.push(crate::model::ChartPoint {
            idx: i as u32,
            format_code: numeric.then(|| cell_format_code(wb, Some(c))),
            value,
        });
    }
    crate::model::ChartCache {
        format_code: numeric.then(|| cell_format_code(wb, cells.iter().flatten().next().copied())),
        pt_count: Some(cells.len() as u32),
        points,
    }
}
