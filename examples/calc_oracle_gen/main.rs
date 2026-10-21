// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 계산 엔진 대조 실험 문서를 만든다.
//!
//! 분류마다 XML .cell 하나: 자료 시트(`Data`, `Hid`, `S1`–`S3`)와 경우 시트(`T`). `T` 의 B열에 함수 ×
//! 인수 경우마다 수식 셀 하나를 두고 캐시 값은 `<v>0</v>` 로 채운다 (한셀의 바이너리 저장은 값 없는 수식
//! 셀에서 실패한다). 한셀로 열어 `Application.CalculateFull` 로 다시 계산해 저장하면 셀마다 한셀의 계산
//! 값이 남는다 (`tools/probe/calc-oracle.ps1`). `tests/calc_oracle.rs` 가 rcell 의 계산과 비교한다.
//!
//! ```text
//! cargo run --release --example calc_oracle_gen -- <출력 폴더>
//! powershell -File tools\probe\calc-oracle.ps1 -InDir <출력 폴더> -OutDir <결과 폴더>
//! RCELL_CALC_ORACLE=<결과 폴더> cargo test --release --test calc_oracle -- --nocapture
//! ```
//!
//! 자료는 모두 이 파일이 지어낸 값이다.

mod cases;

use std::path::PathBuf;

use rcell::model::{
    Cell, CellRef, CellValue, DefinedName, Formula, FormulaKind, RowInfo, Sheet, Workbook,
};

pub use cases::Case;

/// 자료 값: 숫자, 글, 논리, 오류, 빈 칸
#[derive(Clone)]
enum D {
    N(f64),
    S(&'static str),
    B(bool),
    E(&'static str),
    Blank,
}

fn put(cells: &mut Vec<Cell>, at: &str, v: D) {
    let value = match v {
        D::N(n) => CellValue::Number(n),
        D::S(s) => CellValue::Text(s.into()),
        D::B(b) => CellValue::Bool(b),
        D::E(e) => CellValue::Error(e.into()),
        D::Blank => return,
    };
    cells.push(Cell {
        r: CellRef::parse(at).expect("셀 주소"),
        style: 0,
        value,
        formula: None,
    });
}

fn column(cells: &mut Vec<Cell>, col: &str, row0: u32, vals: &[D]) {
    for (i, v) in vals.iter().enumerate() {
        put(cells, &format!("{col}{}", row0 + i as u32), v.clone());
    }
}

fn nums(v: &[f64]) -> Vec<D> {
    v.iter().map(|&x| D::N(x)).collect()
}

fn texts(v: &[&'static str]) -> Vec<D> {
    v.iter().map(|&x| D::S(x)).collect()
}

fn sheet(name: &str, mut cells: Vec<Cell>) -> Sheet {
    cells.sort_by_key(|c| (c.r.row, c.r.col));
    Sheet {
        name: name.into(),
        cells,
        show_grid_lines: true,
        ..Default::default()
    }
}

/// 자료 시트 `Data` (배치는 `cases.rs` 머리 설명)
fn data_sheet() -> Sheet {
    use D::*;
    let mut c = Vec::new();
    column(&mut c, "A", 1, &nums(&[3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 3.0]));
    column(
        &mut c,
        "B",
        1,
        &nums(&[2.5, -1.25, 0.0, 7.0, 10.5, -3.0, 4.75, 100.0, 0.001, 8.0]),
    );
    column(
        &mut c,
        "C",
        1,
        &[N(1.0), S("abc"), B(true), Blank, S("5"), N(2.5), B(false), S(""), N(0.0), N(-4.0)],
    );
    column(
        &mut c,
        "D",
        1,
        &texts(&["apple", "Banana", "cherry", "date", "Elder", "fig", "grape", "apple", "kiwi", "lemon"]),
    );
    column(
        &mut c,
        "E",
        1,
        &nums(&[45000.0, 45031.5, 45100.25, 44927.0, 45292.0, 45657.0, 36526.0, 40000.0, 43831.0, 46314.0]),
    );
    column(&mut c, "F", 1, &nums(&[10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0]));
    column(
        &mut c,
        "G",
        1,
        &texts(&["r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10"]),
    );
    column(
        &mut c,
        "H",
        1,
        &[N(1.0), E("#N/A"), N(3.0), E("#DIV/0!"), N(5.0), E("#VALUE!"), N(7.0), E("#NUM!"), N(9.0), E("#REF!")],
    );
    column(&mut c, "I", 1, &texts(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]));
    column(
        &mut c,
        "J",
        1,
        &[B(true), B(false), B(true), B(true), B(false), B(false), B(true), B(false), B(true), B(true)],
    );
    column(&mut c, "K", 1, &nums(&[2.0, 4.0, 5.0, 4.0, 5.0, 7.0, 8.0, 9.0, 10.0, 12.0]));
    column(&mut c, "L", 1, &nums(&[1.5, 2.25, 3.0, 4.5, 6.0, 7.5, 9.0, 10.25, 12.0, 15.0]));
    // 행렬: M1:O3 정칙, M5:N6 정칙, M8:N9 특이
    for (r, row) in [[2.0, 1.0, 1.0], [1.0, 3.0, 2.0], [1.0, 0.0, 0.0]].iter().enumerate() {
        for (k, v) in row.iter().enumerate() {
            put(&mut c, &format!("{}{}", ["M", "N", "O"][k], r + 1), N(*v));
        }
    }
    for (r, row) in [[4.0, 7.0], [2.0, 6.0]].iter().enumerate() {
        for (k, v) in row.iter().enumerate() {
            put(&mut c, &format!("{}{}", ["M", "N"][k], r + 5), N(*v));
        }
    }
    for (r, row) in [[1.0, 2.0], [2.0, 4.0]].iter().enumerate() {
        for (k, v) in row.iter().enumerate() {
            put(&mut c, &format!("{}{}", ["M", "N"][k], r + 8), N(*v));
        }
    }
    // 목록(데이터베이스) Q1:T11 과 조건 표 V1:W3, Y1:Y2, V5:V6
    column(
        &mut c,
        "Q",
        1,
        &texts(&["Name", "Kim", "Lee", "Park", "Choi", "Jung", "Kang", "Cho", "Yoon", "Jang", "Lim"]),
    );
    column(
        &mut c,
        "R",
        1,
        &texts(&["Dept", "Sales", "IT", "Sales", "HR", "IT", "Sales", "HR", "IT", "Sales", "HR"]),
    );
    column(&mut c, "S", 1, &[S("Age")]);
    column(&mut c, "S", 2, &nums(&[34.0, 28.0, 45.0, 39.0, 31.0, 29.0, 52.0, 41.0, 36.0, 25.0]));
    column(&mut c, "T", 1, &[S("Score")]);
    column(&mut c, "T", 2, &nums(&[88.0, 92.0, 75.0, 81.0, 95.0, 67.0, 90.0, 72.0, 85.0, 78.0]));
    column(&mut c, "V", 1, &[S("Dept"), S("Sales"), S("IT")]);
    column(&mut c, "W", 1, &[S("Age"), S(">30")]);
    column(&mut c, "Y", 1, &[S("Score"), S(">=85")]);
    column(&mut c, "V", 5, &[S("Name"), S("K")]);
    column(&mut c, "W", 5, &[S("Age"), S("<30")]);
    // 현금 흐름 AA1:AA6, 날짜 AB1:AB6
    column(&mut c, "AA", 1, &nums(&[-1000.0, 300.0, 400.0, 400.0, 300.0, 200.0]));
    column(&mut c, "AB", 1, &nums(&[44927.0, 45000.0, 45100.0, 45200.0, 45300.0, 45400.0]));
    // 확률 AC1:AD4, 구간 AE1:AE3, 휴일 AF1:AF2
    column(&mut c, "AC", 1, &nums(&[1.0, 2.0, 3.0, 4.0]));
    column(&mut c, "AD", 1, &nums(&[0.1, 0.2, 0.3, 0.4]));
    column(&mut c, "AE", 1, &nums(&[2.0, 4.0, 6.0]));
    column(&mut c, "AF", 1, &nums(&[45292.0, 45306.0]));
    // 복소수 글 AG1:AG4, 진법 글 AH1:AH3
    column(&mut c, "AG", 1, &texts(&["3+4i", "1-i", "-2.5j", "2"]));
    column(&mut c, "AH", 1, &[S("101"), S("FF"), N(777.0)]);
    // 한 행 자료 A12:J12 (HLOOKUP·가로 찾기)
    for (k, v) in [10.0, 20.0, 30.0, 40.0, 50.0].iter().enumerate() {
        put(&mut c, &format!("{}12", ["A", "B", "C", "D", "E"][k]), N(*v));
    }
    for (k, v) in ["x1", "x2", "x3", "x4", "x5"].iter().enumerate() {
        put(&mut c, &format!("{}13", ["A", "B", "C", "D", "E"][k]), S(v));
    }
    sheet("Data", c)
}

/// 숨긴 행(3, 6)과 부분합 셀이 있는 시트 `Hid`
fn hidden_sheet() -> (Sheet, Vec<(String, String)>) {
    let mut c = Vec::new();
    column(&mut c, "A", 1, &nums(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]));
    column(&mut c, "C", 1, &nums(&[1.5, -2.0, 3.25, 4.0, 0.0, 6.5, 7.0, -8.0, 9.0, 10.0]));
    let mut s = sheet("Hid", c);
    s.rows = [3u32, 6]
        .iter()
        .map(|&i| RowInfo {
            index: i,
            hidden: true,
            ..Default::default()
        })
        .collect();
    // B5·B10 은 부분합 수식 (다른 부분합이 겹쳐 세지 않는지)
    let formulas = vec![
        ("B5".to_string(), "SUBTOTAL(9,A1:A4)".to_string()),
        ("B10".to_string(), "SUBTOTAL(9,A6:A9)".to_string()),
        ("B2".to_string(), "A2*2".to_string()),
    ];
    (s, formulas)
}

fn small_sheet(name: &'static str, vals: [f64; 3]) -> Sheet {
    let mut c = Vec::new();
    column(&mut c, "A", 1, &nums(&vals));
    put(&mut c, "B1", D::S(name));
    sheet(name, c)
}

fn formula_cell(at: CellRef, text: &str, array: Option<&str>) -> Cell {
    Cell {
        r: at,
        style: 0,
        value: CellValue::Number(0.0),
        formula: Some(Formula {
            text: text.into(),
            kind: if array.is_some() {
                FormulaKind::Array
            } else {
                FormulaKind::Normal
            },
            shared_index: None,
            range: array.map(str::to_string),
        }),
    }
}

/// 분류 하나의 통합 문서
fn build(cases: &[Case], date1904: bool) -> Workbook {
    let (mut hid, hid_formulas) = hidden_sheet();
    for (at, f) in hid_formulas {
        hid.cells.push(formula_cell(CellRef::parse(&at).unwrap(), &f, None));
    }
    hid.cells.sort_by_key(|c| (c.r.row, c.r.col));
    // 경우 시트: B열에 차례로. 배열 수식은 h×w 칸을 차지하고 그다음 행부터 이어 간다.
    let mut cells = Vec::new();
    let mut row = 0u32;
    for case in cases {
        let at = CellRef { row, col: 1 };
        match case.array {
            None => {
                cells.push(formula_cell(at, &case.f, None));
                row += 1;
            }
            Some((h, w)) => {
                let end = CellRef {
                    row: row + h - 1,
                    col: 1 + w - 1,
                };
                let rg = if h == 1 && w == 1 {
                    at.to_string()
                } else {
                    format!("{at}:{end}")
                };
                cells.push(formula_cell(at, &case.f, Some(&rg)));
                for r in 0..h {
                    for k in 0..w {
                        if r == 0 && k == 0 {
                            continue;
                        }
                        cells.push(Cell {
                            r: CellRef {
                                row: row + r,
                                col: 1 + k,
                            },
                            style: 0,
                            value: CellValue::Number(0.0),
                            formula: None,
                        });
                    }
                }
                row += h;
            }
        }
    }
    let t = sheet("T", cells);
    let sheets = vec![
        t,
        data_sheet(),
        hid,
        small_sheet("S1", [1.0, 2.0, 3.0]),
        small_sheet("S2", [10.0, 20.0, 30.0]),
        small_sheet("S3", [100.0, 200.0, 300.0]),
    ];
    let name = |n: &str, f: &str, local: Option<u32>| DefinedName {
        name: n.into(),
        local_sheet_id: local,
        hidden: false,
        formula: f.into(),
    };
    Workbook {
        date1904,
        sheets,
        defined_names: vec![
            name("Nums", "Data!$A$1:$A$10", None),
            name("Rate", "0.05", None),
            name("Two", "1+1", None),
            name("Greeting", "\"hello\"", None),
            name("Arr", "{1,2,3}", None),
            name("Local", "Data!$B$2", None),
            name("Local", "Data!$B$1", Some(0)),
            name("DataTop", "Data!$A$1", None),
            name("Both", "Data!$A$1:$B$3", None),
        ],
        ..Default::default()
    }
}

/// 사용: `<출력 폴더> [--only 분류] [--chunk N]` — `--chunk` 는 한 문서에 경우 N개씩 나눠 쓴다
/// (한셀을 죽이는 경우를 좁힐 때).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out) = args.first().map(PathBuf::from) else {
        eprintln!("사용: cargo run --release --example calc_oracle_gen -- <출력 폴더> [--only 분류] [--chunk N]");
        std::process::exit(2);
    };
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let only = opt("--only");
    let chunk: usize = opt("--chunk").and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
    std::fs::create_dir_all(&out).expect("출력 폴더");
    let mut total = 0;
    for (name, cases, date1904) in cases::catalog() {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        for (i, part) in cases.chunks(chunk.max(1)).enumerate() {
            let wb = build(part, date1904);
            let file = if chunk == usize::MAX {
                format!("{name}.cell")
            } else {
                format!("{name}-{i:03}.cell")
            };
            rcell::save(&wb, out.join(file)).expect("저장");
        }
        println!("{name}: {} 경우", cases.len());
        total += cases.len();
    }
    println!("모두 {total} 경우");
}
