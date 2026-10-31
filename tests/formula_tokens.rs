// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 토큰열 대조: 한셀이 같은 문서를 바이너리·레거시로 저장해 쓴 수식 토큰열과, rcell 이 XML 원본을 읽어
//! 쓴 토큰열을 칸마다 바이트로 견준다 (캐시 값·서식 번호는 보지 않는다).
//!
//! - 바이너리: `BrtFmla*` 의 `rgce`+`rgcb`, `BrtArrFmla`·`BrtShrFmla` 레코드 (xl/worksheets/sheetN.bin)
//! - 레거시: `FORMULA` (0x0006) 의 `grbit..rgce`, `SHRFMLA`(0x04BC)·`ARRAY`(0x0221)·`TABLE`(0x0236) 레코드
//!
//! 짝은 `X.cell` (XML) · `X.bin.cell` · `X.legacy.cell` 세 벌이다. 폴더는 `RCELL_TOKENS_DIR`, 없으면 `RCELL_CORPUS`
//! 의 문서. `RCELL_TOKENS_DUMP=<파일>` 이면 다른 칸을 `서식\t문서\t시트\t칸\t수식\t한셀\trcell` 줄로 적는다.
//!
//! ```text
//! RCELL_TOKENS_DIR=<폴더> RCELL_TOKENS_DUMP=<파일> cargo test --release --test formula_tokens -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rcell::binary::biff12;
use rcell::legacy::biff;

/// 칸 열쇠 (시트 순번, 행, 열) → 토큰열
type Tokens = BTreeMap<(usize, u32, u32), Vec<u8>>;

fn u16_at(d: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?]) as usize)
}

fn u32_at(d: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes([
        *d.get(at)?,
        *d.get(at + 1)?,
        *d.get(at + 2)?,
        *d.get(at + 3)?,
    ]) as usize)
}

fn hex(d: &[u8]) -> String {
    d.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 바이너리 파트 이름의 시트 순번 (`xl/worksheets/sheet3.bin` → 3)
fn sheet_no(name: &str) -> Option<usize> {
    name.strip_prefix("xl/worksheets/sheet")?
        .strip_suffix(".bin")?
        .parse()
        .ok()
}

/// BIFF12 토큰 하나의 바이트 수 (모르는 토큰은 1)
fn ptg_size12(rgce: &[u8], i: usize) -> usize {
    let p = rgce[i];
    if p >= 0x20 {
        return match 0x20 | (p & 0x1F) {
            0x20 => 15,
            0x21 => 3,
            0x22 => 4,
            0x23 => 5,
            0x24 | 0x26..=0x28 | 0x2A | 0x2C => 7,
            0x25 | 0x2B | 0x2D => 13,
            0x29 | 0x2E | 0x2F => 3,
            0x39 | 0x3A | 0x3C => 7 + 2 * (0x20 | (p & 0x1F) != 0x39) as usize,
            0x3B | 0x3D => 15,
            _ => 1,
        };
    }
    match p {
        1 | 2 => 5,
        0x17 => 3 + 2 * u16_at(rgce, i + 1).unwrap_or(0),
        0x18 => 6,
        0x19 => match rgce.get(i + 1) {
            Some(t) if t & 4 != 0 => 4 + 4 * (u16_at(rgce, i + 2).unwrap_or(0) + 1),
            _ => 4,
        },
        0x1C | 0x1D => 2,
        0x1E => 3,
        0x1F => 9,
        _ => 1,
    }
}

/// 한셀이 초기화하지 않은 바이트를 0 으로: 배열 상수의 오류 칸 뒤 채움 3바이트는 앞서 쓴 다른 수식의 찌꺼기다
/// (같은 수식을 쓰는 문서마다 다르다). `t` = grbit u16 + cce u32 + rgce + cb u32 + rgcb
fn mask_pads12(t: &mut [u8]) {
    let Some(cce) = u32_at(t, 2) else { return };
    let rgce: Vec<u8> = t.get(6..6 + cce).unwrap_or_default().to_vec();
    let mut at = 6 + cce + 4;
    let mut i = 0;
    while i < rgce.len() {
        let p = rgce[i];
        let base = 0x20 | (p & 0x1F);
        if p >= 0x20 && base == 0x26 {
            // PtgExtraMem: 개수 u32 + 개수 × 16
            at += 4 + 16 * u32_at(t, at).unwrap_or(0);
        } else if p >= 0x20 && base == 0x20 {
            let (Some(r), Some(c)) = (u32_at(t, at), u32_at(t, at + 4)) else {
                return;
            };
            at += 8;
            for _ in 0..r * c {
                match t.get(at).copied() {
                    Some(0) => at += 9,
                    Some(1) => at += 3 + 2 * u16_at(t, at + 1).unwrap_or(0),
                    Some(2) => at += 2,
                    Some(4) => {
                        if let Some(pad) = t.get_mut(at + 2..at + 5) {
                            pad.fill(0);
                        }
                        at += 5;
                    }
                    _ => return,
                }
            }
        }
        i += ptg_size12(&rgce, i);
    }
}

/// BIFF12 수식 (cce u32, rgce, cb u32, rgcb) 의 전체 바이트 길이
fn bin_formula_len(d: &[u8], at: usize) -> Option<usize> {
    let cce = u32_at(d, at)?;
    let cb = u32_at(d, at + 4 + cce)?;
    Some(4 + cce + 4 + cb)
}

fn bin_tokens(bytes: &[u8]) -> Tokens {
    let mut out = Tokens::new();
    let Ok(mut pkg) = rcell::open_package(bytes.to_vec()) else {
        return out;
    };
    let names: Vec<String> = pkg.part_names().to_vec();
    for name in names {
        let Some(no) = sheet_no(&name) else {
            continue;
        };
        let Ok(Some(part)) = pkg.read_bytes(&name) else {
            continue;
        };
        let mut row = 0u32;
        for r in biff12::records(&part) {
            let d = r.data;
            match r.rtype {
                0 => row = u32_at(d, 0).unwrap_or(0) as u32,
                // BrtFmlaString·Num·Bool·Error: col, style, 값, grbit u16, 수식
                8..=11 => {
                    let Some(col) = u32_at(d, 0) else { continue };
                    let val = match r.rtype {
                        9 => 8,
                        8 => match u32_at(d, 8) {
                            Some(n) => 4 + n * 2,
                            None => continue,
                        },
                        _ => 1,
                    };
                    let at = 8 + val;
                    let Some(len) = bin_formula_len(d, at + 2) else {
                        continue;
                    };
                    // 값 형은 캐시 값의 형이라 보지 않는다: 토큰열 앞에 grbit 만
                    let mut t = d[at..at + 2].to_vec();
                    t.extend_from_slice(&d[at + 2..at + 2 + len]);
                    mask_pads12(&mut t);
                    out.insert((no, row, col as u32), t);
                }
                // BrtArrFmla: RfX 16, flags u8, 수식 / BrtShrFmla: RfX 16, 수식
                426 | 427 => {
                    let (Some(r0), Some(c0)) = (u32_at(d, 0), u32_at(d, 8)) else {
                        continue;
                    };
                    let mut t = vec![0xFF, 0xFF];
                    t.extend_from_slice(&(r.rtype as u16).to_le_bytes());
                    t.extend_from_slice(d);
                    out.insert((no, r0 as u32 + 0x8000_0000, c0 as u32), t);
                }
                _ => {}
            }
        }
    }
    out
}

fn legacy_tokens(bytes: &[u8]) -> Tokens {
    let mut out = Tokens::new();
    let Ok(stream) = rcell::container::read_entry(bytes, "/Workbook") else {
        return out;
    };
    let mut sheet = 0usize;
    for r in biff::records(&stream) {
        let d = r.data;
        match r.rtype {
            0x0809 => sheet += 1, // 첫 BOF 는 통합 문서 전체 — 첫 시트가 1
            // FORMULA (한셀 레거시: 행 u32): row, col u16, ixfe u16, 값 8, grbit u16, chn u32, cce u16, rgce (+ 배열 상수 덩어리)
            0x0006 => {
                let (Some(row), Some(col)) = (u32_at(d, 0), u16_at(d, 4)) else {
                    continue;
                };
                if d.len() > 16 {
                    out.insert((sheet - 1, row as u32, col as u32), d[16..].to_vec());
                }
            }
            // SHRFMLA·ARRAY: 첫 칸 (RefU) 으로 열쇠. TABLE 도 같다
            0x04BC | 0x0221 | 0x0236 => {
                let (Some(r0), Some(c0)) = (u32_at(d, 0), u16_at(d, 8)) else {
                    continue;
                };
                let mut t = vec![0xFF, 0xFF];
                t.extend_from_slice(&r.rtype.to_le_bytes());
                t.extend_from_slice(d);
                out.insert((sheet - 1, r0 as u32 + 0x8000_0000, c0 as u32), t);
            }
            _ => {}
        }
    }
    out
}

/// 문서의 짝: (이름, XML, 바이너리, 레거시)
fn triples() -> Vec<(String, PathBuf, Option<PathBuf>, Option<PathBuf>)> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("RCELL_TOKENS_DIR") {
        dirs.push(PathBuf::from(d));
    } else if let Some(root) = std::env::var_os("RCELL_CORPUS") {
        let root = PathBuf::from(root);
        for d in ["", "templates", "craft", "craft-charts", "craft-charts2"] {
            dirs.push(root.join(d));
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.ends_with(".cell")
                        && !n.starts_with("~$")
                        && !n.ends_with(".bin.cell")
                        && !n.ends_with(".legacy.cell")
                })
            })
            .collect();
        v.sort();
        for xml in v {
            let stem = xml.file_name().unwrap().to_string_lossy().into_owned();
            let stem = stem.trim_end_matches(".cell").to_string();
            let sib = |ext: &str| {
                let p = xml.with_file_name(format!("{stem}{ext}"));
                p.exists().then_some(p)
            };
            let (bin, leg) = (sib(".bin.cell"), sib(".legacy.cell"));
            if bin.is_some() || leg.is_some() {
                out.push((stem, xml, bin, leg));
            }
        }
    }
    out
}

/// 칸의 수식 글 (설명용)
fn formula_text(wb: &rcell::Workbook, sheet: usize, row: u32, col: u32) -> String {
    wb.sheets
        .get(sheet)
        .and_then(|s| {
            s.cells
                .iter()
                .find(|c| c.r.row == row && c.r.col == col)
                .and_then(|c| c.formula.as_ref().map(|f| f.text.clone()))
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct Tally {
    total: usize,
    same: usize,
    only_hcell: usize,
    only_rcell: usize,
    differ: usize,
}

#[allow(clippy::too_many_arguments)]
fn compare(
    kind: &str,
    book: &str,
    wb: &rcell::Workbook,
    hc: &Tokens,
    rc: &Tokens,
    base: usize,
    tally: &mut Tally,
    dump: &mut String,
) {
    let mut keys: Vec<_> = hc.keys().chain(rc.keys()).copied().collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        tally.total += 1;
        let (a, b) = (hc.get(&k), rc.get(&k));
        let all = std::env::var_os("RCELL_TOKENS_ALL").is_some();
        if a == b {
            tally.same += 1;
            if !all {
                continue;
            }
        } else {
            match (a, b) {
                (Some(_), None) => tally.only_hcell += 1,
                (None, Some(_)) => tally.only_rcell += 1,
                _ => tally.differ += 1,
            }
        }
        let (sheet, row, col) = (k.0.saturating_sub(base), k.1 & 0x7FFF_FFFF, k.2);
        let text = formula_text(wb, sheet, row, col);
        dump.push_str(&format!(
            "{kind}\t{book}\t{sheet}\tr{row}c{col}\t{text}\t{}\t{}\n",
            a.map(|x| hex(x)).unwrap_or_else(|| "-".into()),
            b.map(|x| hex(x)).unwrap_or_else(|| "-".into()),
        ));
    }
}

#[test]
fn formula_tokens_match_hancell() {
    let list = triples();
    if list.is_empty() {
        eprintln!("RCELL_TOKENS_DIR·RCELL_CORPUS 미설정 — 건너뜀");
        return;
    }
    let (mut tb, mut tl) = (Tally::default(), Tally::default());
    let mut dump = String::new();
    for (name, xml, bin, leg) in &list {
        let Ok(wb) = rcell::open(xml) else { continue };
        // 시트 순번: 바이너리는 워크시트만 세므로 차트 시트가 앞에 있으면 어긋난다 — 워크시트 차례로 맞춘다
        if let Some(bin) = bin {
            let hc = bin_tokens(&std::fs::read(bin).unwrap());
            if let Ok(out) = rcell::binary::writer::write_report(
                &wb,
                &rcell::binary::writer::Options {
                    abs_path: None,
                    exact: false,
                },
            ) {
                let rc = bin_tokens(&out.bytes);
                compare("bin", name, &wb, &hc, &rc, 1, &mut tb, &mut dump);
            }
        }
        if let Some(leg) = leg {
            let hc = legacy_tokens(&std::fs::read(leg).unwrap());
            if let Ok(bytes) = rcell::legacy::write(&wb) {
                let rc = legacy_tokens(&bytes);
                compare("legacy", name, &wb, &hc, &rc, 1, &mut tl, &mut dump);
            }
        }
    }
    let pct = |t: &Tally| 100.0 * t.same as f64 / t.total.max(1) as f64;
    eprintln!(
        "바이너리 수식 {}: 같음 {} ({:.2}%), 한셀에만 {}, rcell 에만 {}, 다름 {}",
        tb.total,
        tb.same,
        pct(&tb),
        tb.only_hcell,
        tb.only_rcell,
        tb.differ
    );
    eprintln!(
        "레거시 수식 {}: 같음 {} ({:.2}%), 한셀에만 {}, rcell 에만 {}, 다름 {}",
        tl.total,
        tl.same,
        pct(&tl),
        tl.only_hcell,
        tl.only_rcell,
        tl.differ
    );
    if let Some(p) = std::env::var_os("RCELL_TOKENS_DUMP").map(PathBuf::from) {
        let _ = std::fs::write(&p, dump);
        eprintln!("다른 칸을 {} 에 적었다", Path::new(&p).display());
    }
}
