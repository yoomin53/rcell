// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 렌더러의 화면 배치(열 너비·위치, 행 높이·위치)를 한셀 COM 으로 잰 값과 대조한다.
//!
//! `tools/probe/geometry-oracle.ps1 -InDir <폴더> -Out <정답.tsv>` 로 만든 정답이 필요하다
//! (첫 줄 `#dir` 이 문서 폴더). 한셀은 열 너비를 1/20 pt 로 반올림해 돌려주고 행 높이는 1/20 pt 다.
//! 같은 문서의 바이너리·레거시 판(`X.bin.cell`, `X.legacy.cell`)이 옆에 있으면 그것도 같은 정답과 대조한다.
//!
//! ```text
//! RCELL_GEOMETRY_ORACLE=C:\…\geometry-templates.tsv cargo test --release --test render_geometry -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct SheetOracle {
    /// 열(1부터) → (Left, Width) pt
    cols: BTreeMap<u32, (f64, f64)>,
    /// 행(1부터) → (Top, Height) pt
    rows: BTreeMap<u32, (f64, f64)>,
}

fn load(tsv: &str) -> (Option<PathBuf>, BTreeMap<(String, String), SheetOracle>) {
    let mut dir = None;
    let mut out: BTreeMap<(String, String), SheetOracle> = BTreeMap::new();
    for line in tsv.lines() {
        let p: Vec<&str> = line.split('\t').collect();
        if p.first() == Some(&"#dir") {
            dir = p.get(1).map(PathBuf::from);
            continue;
        }
        if p.len() < 6 {
            continue;
        }
        let key = (p[0].to_string(), p[1].to_string());
        let (Ok(i), Ok(a), Ok(b)) = (
            p[3].parse::<u32>(),
            p[4].parse::<f64>(),
            p[5].parse::<f64>(),
        ) else {
            continue;
        };
        let o = out.entry(key).or_default();
        match p[2] {
            "c" => {
                o.cols.insert(i, (a, b));
            }
            "r" => {
                o.rows.insert(i, (a, b));
            }
            _ => {}
        }
    }
    (dir, out)
}

/// 문서 하나와 정답의 차이: (열 수, 행 수, 다른 것들)
fn compare(
    path: &Path,
    name: &str,
    oracle: &BTreeMap<(String, String), SheetOracle>,
) -> (usize, usize, Vec<String>) {
    let wb = match rcell::open(path) {
        Ok(w) => w,
        Err(e) => return (0, 0, vec![format!("{}: 열지 못함 {e}", path.display())]),
    };
    let mut bad = Vec::new();
    let (mut nc, mut nr) = (0, 0);
    for ((file, sheet_name), o) in oracle.iter().filter(|((f, _), _)| f == name) {
        let Some(sheet) = wb.sheet_by_name(sheet_name) else {
            bad.push(format!("{file} {sheet_name}: 시트 없음"));
            continue;
        };
        let max_c = o.cols.keys().max().copied().unwrap_or(0);
        let max_r = o.rows.keys().max().copied().unwrap_or(0);
        let g = rcell::render::sheet_geometry(&wb, sheet, max_r, max_c);
        // 열: 한셀 안의 너비(px)를 pt 로 → 1/20 pt 반올림과 비교, 위치는 누적을 반올림
        let mut x = 0.0;
        for (&c, &(left, width)) in &o.cols {
            let w_px = g.col_px[(c - 1) as usize];
            let tw = |px: f64| (px * 15.0).round() / 20.0;
            let (my_left, my_w) = (tw(x), tw(w_px));
            x += w_px;
            nc += 1;
            if (my_w - width).abs() > 0.026 || (my_left - left).abs() > 0.026 {
                bad.push(format!(
                    "{file} {sheet_name} 열 {c}: rcell {my_left}/{my_w} pt, 한셀 {left}/{width} pt"
                ));
            }
        }
        let mut y = 0.0;
        for (&r, &(top, height)) in &o.rows {
            let h = g.row_pt[(r - 1) as usize];
            let my_top = y;
            y += h;
            nr += 1;
            if (h - height).abs() > 0.026 || (my_top - top).abs() > 0.03 {
                bad.push(format!(
                    "{file} {sheet_name} 행 {r}: rcell {my_top:.2}/{h} pt, 한셀 {top}/{height} pt"
                ));
            }
        }
    }
    (nc, nr, bad)
}

#[test]
fn layout_matches_hancell_com_geometry() {
    let Some(tsv_path) = std::env::var_os("RCELL_GEOMETRY_ORACLE") else {
        eprintln!("RCELL_GEOMETRY_ORACLE 미설정 — 건너뜀");
        return;
    };
    let tsv = std::fs::read_to_string(&tsv_path).expect("정답 TSV");
    let (dir, oracle) = load(&tsv);
    let dir = dir
        .or_else(|| std::env::var_os("RCELL_CORPUS").map(|c| PathBuf::from(c).join("templates")))
        .expect("#dir 줄 또는 RCELL_CORPUS");
    // 정답이 있는 XML 판 (레거시 판의 정답은 그 판을 잴 때 쓴다)
    let files: Vec<String> = {
        let mut f: Vec<String> = oracle
            .keys()
            .map(|(f, _)| f.clone())
            .filter(|f| !f.contains(".legacy.") && !f.contains(".bin."))
            .collect();
        f.dedup();
        f
    };
    let has = |f: &str| oracle.keys().any(|(k, _)| k == f);
    let mut all_bad = Vec::new();
    let (mut docs, mut cols, mut rows) = (0, 0, 0);
    for name in &files {
        let stem = name.trim_end_matches(".cell");
        // XML 원본과 같은 문서의 다른 형식: 그 판을 잰 정답이 있으면 그것, 없으면 XML 판의 정답 (한셀 화면은 같다)
        for variant in [name.to_string(), format!("{stem}.bin.cell"), format!("{stem}.legacy.cell")] {
            let path = dir.join(&variant);
            if !path.exists() {
                continue;
            }
            let key = if has(&variant) { variant.as_str() } else { name.as_str() };
            let (c, r, bad) = compare(&path, key, &oracle);
            docs += 1;
            cols += c;
            rows += r;
            all_bad.extend(bad.into_iter().map(|b| format!("[{variant}] {b}")));
        }
    }
    eprintln!(
        "문서 {docs}개, 열 {cols}개, 행 {rows}개 대조 — 다른 것 {}개",
        all_bad.len()
    );
    let lim: usize = std::env::var("RCELL_GEOMETRY_SHOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    for b in all_bad.iter().take(lim) {
        eprintln!("  {b}");
    }
    assert!(all_bad.is_empty());
}
