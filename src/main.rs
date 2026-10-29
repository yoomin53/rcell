// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! rcell 명령줄 도구.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use rcell::model::Sheet;
use rcell::scan::{ScanReport, scan_package};

#[derive(Parser)]
#[command(
    name = "rcell",
    version,
    about = "한컴 셀(.cell) 파일 분석 도구 — by yoomin53"
)]
struct Cli {
    /// 암호가 걸린 문서의 암호 (생략하면 보호만 걸린 문서의 기본 암호를 쓴다)
    #[arg(long, global = true)]
    password: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

/// --password 로 받은 문서 암호
static PASSWORD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 파일 바이트 (암호화된 OLE2 면 풀어서 안의 패키지)
fn read_plain(path: &Path) -> rcell::Result<Vec<u8>> {
    rcell::decrypt_if_needed(std::fs::read(path)?, PASSWORD.get().map(String::as_str))
}

/// 문서 열기 (암호를 받았으면 그것으로)
fn open_doc(path: &Path) -> rcell::Result<rcell::Workbook> {
    match PASSWORD.get() {
        Some(p) => rcell::open_with_password(path, p),
        None => rcell::open(path),
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// 문서 요약 (작성 프로그램, 시트, 셀 수, 한컴 확장 사용 여부)
    Info { file: PathBuf },
    /// 전체 문서 모델을 JSON 으로 출력
    Dump {
        file: PathBuf,
        /// 셀 목록 생략 (구조만 보기)
        #[arg(long)]
        no_cells: bool,
    },
    /// 시트를 CSV 로 출력 (한셀의 CSV 저장과 같은 행·열 범위·표시 글·따옴표, UTF-8)
    Csv {
        file: PathBuf,
        /// 시트 이름 또는 0부터 시작하는 번호 (기본: 첫 시트)
        #[arg(long, short)]
        sheet: Option<String>,
    },
    /// 패키지 파트와 콘텐츠 타입 목록
    Parts { file: PathBuf },
    /// 한컴 확장 어휘 스캔 (파일 또는 폴더 여러 개). 모델링 안 된 항목이 다음 분석 대상
    Scan {
        paths: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 컨테이너 항목 목록 (ZIP 파트 또는 OLE2 스트림) — 리버스 엔지니어링용
    Streams { file: PathBuf },
    /// 시트를 HTML 로 그린다 (한셀 화면처럼: 표시 형식, 글꼴, 채우기, 테두리, 병합)
    Html {
        file: PathBuf,
        /// 출력 파일 (생략하면 표준 출력)
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// 처음 보일 시트 (이름 또는 0부터 시작하는 번호, 생략하면 선택된 시트)
        #[arg(long)]
        sheet: Option<String>,
        /// 행·열 머리글을 그리지 않는다
        #[arg(long)]
        no_headers: bool,
        /// 빈 칸으로라도 채울 최소 크기 (px, 예: 1600x900). 0x0 이면 쓰는 범위만
        #[arg(long)]
        fill: Option<String>,
    },
    /// 형식 변환: XML/바이너리/레거시 .cell → XML .cell. 출력 이름이 .xlsx 면 Excel, .bin.cell·.xlsb 면
    /// 바이너리 .cell (한셀 xlCellb), .legacy.cell 이면 레거시 .cell (한셀 xlCell8), .csv·.txt·.prn 이면 활성 시트를
    /// 텍스트로 (한셀처럼 코드 페이지 949). 입력이 .csv·.txt·.tsv·.prn 이면 한셀처럼 텍스트 파일을 연다
    Convert {
        input: PathBuf,
        output: PathBuf,
        /// 저장 형식 (cell, xlsx, bin, legacy, xls, csv, txt, prn) — 생략하면 출력 이름으로 정한다
        #[arg(long)]
        format: Option<String>,
        /// 바이너리 저장의 BrtAbsPath15 에 쓸 폴더 (한셀은 저장한 폴더를 쓴다; 생략하면 빈 문자열)
        #[arg(long)]
        abs_path: Option<String>,
        /// 바이너리 저장에서 한셀이 잃는 값도 똑같이 잃는다 (한셀과 같은 결과를 볼 때)
        #[arg(long)]
        exact: bool,
    },
    /// 편집 대본(JSON — docs/editing.md)을 실행해 저장한다. 편집 뒤 수식을 다시 계산한다
    Edit {
        input: PathBuf,
        /// 편집 대본 ({"edits":[{"op":"set","cell":"B2","input":"=A1*2"}, …]})
        script: PathBuf,
        output: PathBuf,
        /// 저장 형식 (cell, xlsx, bin) — 생략하면 출력 이름으로 정한다
        #[arg(long)]
        format: Option<String>,
        /// 다시 계산하지 않는다
        #[arg(long)]
        no_recalc: bool,
    },
    /// 바이너리 .cell 두 개를 레코드 단위로 비교 (A: rcell 이 쓴 것, B: 한셀이 쓴 것) — 리버스 엔지니어링용
    Bindiff {
        a: PathBuf,
        b: PathBuf,
        /// 파트마다 보여 줄 차이 수
        #[arg(long, default_value_t = 6)]
        samples: usize,
    },
    /// 두 레거시 .cell 의 레코드 비교 (종류·순서·내용) — 레거시 쓰기 검증용 (A: rcell, B: 한셀)
    RecDiff {
        a: PathBuf,
        b: PathBuf,
        /// 출력할 최대 차이 수
        #[arg(long, default_value_t = 40)]
        max: usize,
        /// 요약 한 줄만
        #[arg(long)]
        summary: bool,
        /// SST 문자열 차례를 보지 않는다 (문자열을 내용 차례로 맞춘 뒤 비교)
        #[arg(long)]
        ignore_sst_order: bool,
        /// 한셀이 저장할 때마다 달라지는 값을 보지 않는다: SST 차례, TXO 첫 u16 의 0x0200, 차트 VtString 의 pad
        #[arg(long)]
        ignore_volatile: bool,
    },
    /// 두 문서의 의미 비교 (저장 형식이 달라도 됨). 차이가 있으면 종료 코드 1
    Diff {
        a: PathBuf,
        b: PathBuf,
        /// 셀 서식 비교 생략
        #[arg(long)]
        no_styles: bool,
        /// 출력할 최대 차이 수
        #[arg(long, default_value_t = 50)]
        max: usize,
        /// 한컴 글꼴 상대 크기·장평·자간·위치 비교 생략 (한셀의 레거시 읽기 결과와 비교할 때)
        #[arg(long)]
        ignore_font_scale: bool,
        /// 한컴 확장(글꼴 hs:*, 문단 모양) 비교 생략 (바이너리 .cell 과 비교할 때)
        #[arg(long)]
        ignore_hancom: bool,
        /// 도형의 글·사용자 정의 모양, 묶기 잠금 도형, 메모의 선 색 비교 생략 (한셀은 레거시에서 이것들을 읽지 못한다)
        #[arg(long)]
        ignore_shape_detail: bool,
        /// 차트 내용(종류·축·계열·제목·범례 …) 비교 생략 — 차트 도형의 위치는 그래도 비교한다
        #[arg(long)]
        ignore_chart: bool,
        /// 조건부 서식 비교 생략 (한셀은 레거시의 조건부 서식을 잘못 읽는다)
        #[arg(long)]
        ignore_cf: bool,
        /// 자동 필터 비교 생략 (한셀은 레거시 자동 필터의 빈 칸·백분율을 읽지 못한다)
        #[arg(long)]
        ignore_filter: bool,
        /// 사용자 정의 표 스타일 비교 생략 (한셀은 레거시의 표 스타일을 읽지 않는다)
        #[arg(long)]
        ignore_table_styles: bool,
        /// A 에 레거시 저장의 손실(`legacy::lossy::legacy_view`)을 적용한 뒤 비교 — 레거시 쓰기 왕복 검증용
        /// (A: 원본, B: rcell 이 쓴 레거시)
        #[arg(long)]
        legacy_view: bool,
    },
    /// 수식을 다시 계산해 파일의 값(한셀이 계산한 값)과 비교한다. 다르면 종료 코드 1
    Calc {
        file: PathBuf,
        /// 값이 다른 셀을 출력
        #[arg(long)]
        diff: bool,
        /// 출력할 최대 차이 수
        #[arg(long, default_value_t = 50)]
        max: usize,
        /// 다시 계산한 문서를 저장 (XML .cell, 확장자가 .xlsx 면 Excel 형식)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// ZIP 패키지의 파트를 다른 파일 내용으로 바꿔 새 파일로 저장 — 리버스 엔지니어링용
    Patch {
        input: PathBuf,
        output: PathBuf,
        /// 파트이름=로컬파일 (여러 번 지정 가능)
        #[arg(long = "part", value_name = "NAME=FILE")]
        parts: Vec<String>,
    },
    /// 레거시 .cell 시트의 그리기(OfficeArt) 도형 목록 — 리버스 엔지니어링용
    Escher { file: PathBuf },
    /// 레거시 .cell 의 한컴 차트(0x7EF0) Vt 아카이브를 `경로 = 값` 줄로 — 리버스 엔지니어링용
    Vtdump {
        file: PathBuf,
        /// 값마다 0x7EF0 레코드 안의 자리(`@0x…`)와 레코드의 스트림 위치를 붙인다
        #[arg(long)]
        offsets: bool,
    },
    /// 레코드 목록: 레거시 .cell 의 /Workbook, 또는 바이너리 .cell 의 파트(--part) — 리버스 엔지니어링용
    Records {
        file: PathBuf,
        /// 데이터 미리보기 바이트 수
        #[arg(long, default_value_t = 24)]
        bytes: usize,
        /// 이 레코드 타입만 (레거시: 16진수 예 0031, 바이너리: 10진수 예 43)
        #[arg(long)]
        only: Option<String>,
        /// 바이너리 .cell 의 파트 (기본: xl/workbook.bin)
        #[arg(long)]
        part: Option<String>,
    },
    /// 암호화된 문서(OLE2 EncryptionInfo + EncryptedPackage)를 풀어 ZIP 패키지로 저장 — 리버스 엔지니어링용
    Decrypt { input: PathBuf, output: PathBuf },
    /// 레거시 .cell 의 /Workbook 레코드를 BIFF8(.xls) 로 바꿔 쓴다 (--reverse: .xls → 레거시 레코드) — 리버스
    /// 엔지니어링용 (한셀이 같은 문서를 두 형식으로 저장한 것을 맞대 볼 때)
    Biff8 {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        reverse: bool,
    },
    /// 컨테이너 항목 하나를 16진수로 출력 — 리버스 엔지니어링용
    Hexdump {
        file: PathBuf,
        /// ZIP 파트 이름 또는 OLE2 스트림 경로 (예: /Workbook)
        entry: String,
        /// 출력할 최대 바이트 수
        #[arg(long, default_value_t = 4096)]
        max: usize,
        /// 원본 바이트를 파일로 저장
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(p) = cli.password {
        let _ = PASSWORD.set(p);
    }
    let result = match cli.cmd {
        Cmd::Info { file } => info(&file),
        Cmd::Dump { file, no_cells } => dump(&file, no_cells),
        Cmd::Csv { file, sheet } => csv(&file, sheet.as_deref()),
        Cmd::Parts { file } => parts(&file),
        Cmd::Scan { paths, json } => scan(&paths, json),
        Cmd::Streams { file } => streams(&file),
        Cmd::Records {
            file,
            bytes,
            only,
            part,
        } => records(&file, bytes, only.as_deref(), part.as_deref()),
        Cmd::Escher { file } => escher(&file),
        Cmd::Vtdump { file, offsets } => vtdump(&file, offsets),
        Cmd::Patch {
            input,
            output,
            parts,
        } => patch(&input, &output, &parts),
        Cmd::Html {
            file,
            output,
            sheet,
            no_headers,
            fill,
        } => html(
            &file,
            output.as_deref(),
            sheet.as_deref(),
            no_headers,
            fill.as_deref(),
        ),
        Cmd::Convert {
            input,
            output,
            format,
            abs_path,
            exact,
        } => convert(&input, &output, format.as_deref(), abs_path, exact),
        Cmd::Edit {
            input,
            script,
            output,
            format,
            no_recalc,
        } => edit(&input, &script, &output, format.as_deref(), no_recalc),
        Cmd::Bindiff { a, b, samples } => bindiff(&a, &b, samples),
        Cmd::RecDiff {
            a,
            b,
            max,
            summary,
            ignore_sst_order,
            ignore_volatile,
        } => rec_diff(
            &a,
            &b,
            max,
            summary,
            rcell::legacy::recdiff::Options {
                ignore_sst_order: ignore_sst_order || ignore_volatile,
                ignore_txo_flag: ignore_volatile,
                ignore_vt_pad: ignore_volatile,
            },
        ),
        Cmd::Diff {
            a,
            b,
            no_styles,
            max,
            ignore_font_scale,
            ignore_hancom,
            ignore_shape_detail,
            ignore_chart,
            ignore_cf,
            ignore_filter,
            ignore_table_styles,
            legacy_view,
        } => match diff(
            &a,
            &b,
            max,
            legacy_view,
            rcell::compare::Options {
                skip_styles: no_styles,
                ignore_font_scale,
                ignore_hancom_ext: ignore_hancom,
                ignore_shape_detail,
                ignore_chart,
                ignore_cf,
                ignore_filter,
                ignore_table_styles,
            },
        ) {
            Ok(true) => Ok(()),
            Ok(false) => return ExitCode::from(1),
            Err(e) => Err(e),
        },
        Cmd::Calc {
            file,
            diff,
            max,
            output,
        } => match calc(&file, diff, max, output.as_deref()) {
            Ok(true) => Ok(()),
            Ok(false) => return ExitCode::from(1),
            Err(e) => Err(e),
        },
        Cmd::Decrypt { input, output } => {
            read_plain(&input).and_then(|b| std::fs::write(&output, b).map_err(Into::into))
        }
        Cmd::Hexdump {
            file,
            entry,
            max,
            out,
        } => hexdump(&file, &entry, max, out.as_deref()),
        Cmd::Biff8 {
            input,
            output,
            reverse,
        } => biff8(&input, &output, reverse),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("오류: {e}");
            ExitCode::FAILURE
        }
    }
}

fn info(file: &Path) -> rcell::Result<()> {
    let wb = open_doc(file)?;
    println!("파일        : {}", file.display());
    println!(
        "작성 프로그램: {} {}",
        wb.application.as_deref().unwrap_or("?"),
        wb.app_version.as_deref().unwrap_or("")
    );
    if let Some(fv) = &wb.file_version {
        println!(
            "fileVersion : appName={} lastEdited={} rupBuild={}",
            fv.app_name.as_deref().unwrap_or("-"),
            fv.last_edited.as_deref().unwrap_or("-"),
            fv.rup_build.as_deref().unwrap_or("-")
        );
    }
    if let Some(id) = wb.hancom.calc_id {
        println!("한셀 계산 ID : {id}");
    }
    let ext_fonts = wb
        .styles
        .fonts
        .iter()
        .filter(|f| f.hancom.is_some())
        .count();
    let ext_xfs = wb
        .styles
        .cell_xfs
        .iter()
        .filter(|x| x.hancom.is_some())
        .count();
    println!(
        "스타일      : 글꼴 {} (한컴 확장 {}), 셀 서식 {} (한컴 확장 {})",
        wb.styles.fonts.len(),
        ext_fonts,
        wb.styles.cell_xfs.len(),
        ext_xfs
    );
    println!("공유 문자열 : {}", wb.shared_strings.len());
    println!("시트        : {}", wb.sheets.len());
    for (i, s) in wb.sheets.iter().enumerate() {
        let used = s
            .used_range()
            .map(|r| r.to_string())
            .unwrap_or_else(|| "-".into());
        let formulas = s.cells.iter().filter(|c| c.formula.is_some()).count();
        println!(
            "  [{i}] {:<20} 사용 범위 {:<10} 셀 {:>6}  수식 {:>4}  병합 {:>3}  스파크라인 그룹 {}",
            s.name,
            used,
            s.cells.len(),
            formulas,
            s.merges.len(),
            s.sparkline_groups
        );
    }
    Ok(())
}

fn dump(file: &Path, no_cells: bool) -> rcell::Result<()> {
    let mut wb = open_doc(file)?;
    if no_cells {
        for s in &mut wb.sheets {
            s.cells.clear();
        }
    }
    let json = serde_json::to_string_pretty(&wb).map_err(|e| rcell::Error::InvalidValue {
        context: "json".into(),
        value: e.to_string(),
    })?;
    println!("{json}");
    Ok(())
}

fn pick_sheet<'a>(wb: &'a rcell::Workbook, sel: Option<&str>) -> rcell::Result<&'a Sheet> {
    let found = match sel {
        None => wb.sheets.first(),
        Some(s) => wb
            .sheet_by_name(s)
            .or_else(|| s.parse::<usize>().ok().and_then(|i| wb.sheets.get(i))),
    };
    found.ok_or_else(|| rcell::Error::InvalidValue {
        context: "시트".into(),
        value: sel.unwrap_or("(첫 시트)").into(),
    })
}

fn csv(file: &Path, sel: Option<&str>) -> rcell::Result<()> {
    use std::io::Write as _;
    let wb = open_doc(file)?;
    let sheet = pick_sheet(&wb, sel)?;
    let text = rcell::csv::sheet_text(&wb, sheet, rcell::csv::Delimiter::Comma);
    std::io::stdout().write_all(text.as_bytes())?;
    Ok(())
}

fn parts(file: &Path) -> rcell::Result<()> {
    let pkg = rcell::open_package(read_plain(file)?)?;
    for name in pkg.part_names() {
        println!("{:<45} {}", name, pkg.content_type(name).unwrap_or("-"));
    }
    Ok(())
}

fn collect_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                let mut sub: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                sub.sort();
                out.extend(collect_files(&sub));
            }
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cell"))
            || paths.len() == 1
        {
            out.push(p.clone());
        }
    }
    out
}

fn scan(paths: &[PathBuf], json: bool) -> rcell::Result<()> {
    let files = collect_files(paths);
    let mut total = ScanReport::default();
    let mut failed = 0;
    for f in &files {
        let r = std::fs::read(f)
            .map_err(rcell::Error::from)
            .and_then(rcell::open_package)
            .and_then(|mut p| scan_package(&mut p));
        match r {
            Ok(rep) => total.merge(rep),
            Err(e) => {
                failed += 1;
                eprintln!("건너뜀 {}: {e}", f.display());
            }
        }
    }
    if json {
        let s = serde_json::to_string_pretty(&total).map_err(|e| rcell::Error::InvalidValue {
            context: "json".into(),
            value: e.to_string(),
        })?;
        println!("{s}");
        return Ok(());
    }
    println!(
        "파일 {}개 스캔 (실패 {failed}), 한컴 어휘 {}개, 모델링 비율 {:.0}%",
        files.len() - failed,
        total.hancom.len(),
        total.coverage() * 100.0
    );
    for e in &total.hancom {
        let vals: Vec<&str> = e.values.iter().map(String::as_str).collect();
        println!(
            "{} {:<40} {:>6}회  값: {}",
            if e.modeled { "✔" } else { "✘" },
            e.path,
            e.count,
            vals.join(", ")
        );
        if !e.unknown_values.is_empty() {
            let unk: Vec<&str> = e.unknown_values.iter().map(String::as_str).collect();
            println!("    ⚠ 알려진 어휘에 없는 값: {}", unk.join(", "));
        }
    }
    println!("\n네임스페이스:");
    for (uri, n) in &total.namespaces {
        let mark = if rcell::ns::is_hancom(uri) {
            "★"
        } else {
            " "
        };
        println!(" {mark} {uri}  ({n})");
    }
    Ok(())
}

fn streams(file: &Path) -> rcell::Result<()> {
    for (name, len) in rcell::container::list_entries(&std::fs::read(file)?)? {
        println!("{len:>10}  {name}");
    }
    Ok(())
}

fn biff8(input: &Path, output: &Path, reverse: bool) -> rcell::Result<()> {
    let bytes = std::fs::read(input)?;
    let out = if reverse {
        rcell::legacy::legacy_from_xls_file(&bytes)?
    } else {
        rcell::legacy::xls_from_legacy_file(&bytes)?
    };
    std::fs::write(output, out)?;
    Ok(())
}

fn hexdump(file: &Path, entry: &str, max: usize, out: Option<&Path>) -> rcell::Result<()> {
    let data = rcell::container::read_entry(&read_plain(file)?, entry)?;
    if let Some(p) = out {
        std::fs::write(p, &data)?;
    }
    println!("{entry}: {} 바이트", data.len());
    for (i, chunk) in data.chunks(16).take(max.div_ceil(16)).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("{:08x}  {:<47}  {ascii}", i * 16, hex.join(" "));
    }
    Ok(())
}

fn hex_preview(data: &[u8], preview: usize) -> String {
    let hex: Vec<String> = data
        .iter()
        .take(preview)
        .map(|b| format!("{b:02x}"))
        .collect();
    let more = if data.len() > preview { " …" } else { "" };
    format!("{}{more}", hex.join(" "))
}

/// 시트(BOF 단위)마다 MSODRAWING 을 이어 붙여 OfficeArt 도형과 OBJ 종류를 보여 준다.
fn vtdump(file: &Path, offsets: bool) -> rcell::Result<()> {
    let bytes = std::fs::read(file)?;
    for (i, d) in rcell::legacy::chart_dumps_with(&bytes, offsets)?
        .iter()
        .enumerate()
    {
        println!("=== 차트 {i}");
        print!("{d}");
    }
    Ok(())
}

fn escher(file: &Path) -> rcell::Result<()> {
    let bytes = std::fs::read(file)?;
    let stream = rcell::container::read_entry(&bytes, "/Workbook")?;
    let mut sheet = 0;
    let mut art = Vec::new();
    let mut objs: Vec<(u16, u16)> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let flush = |sheet: usize,
                 art: &mut Vec<u8>,
                 objs: &mut Vec<(u16, u16)>,
                 texts: &mut Vec<String>| {
        if art.is_empty() {
            return;
        }
        println!("== 시트 {sheet} (OBJ {})", objs.len());
        let mut k = 0;
        fn show(
            s: &rcell::legacy::escher::EscherShape,
            depth: usize,
            k: &mut usize,
            objs: &[(u16, u16)],
        ) {
            let obj = objs.get(*k).copied();
            *k += 1;
            let props: Vec<String> = s
                .props
                .iter()
                .map(|(p, v, d)| {
                    if d.is_empty() {
                        format!("{p:#06x}={v:#x}")
                    } else {
                        format!("{p:#06x}=[{}]", d.len())
                    }
                })
                .collect();
            println!(
                "{}type={} spid={:#x} flags={:#x} obj={:?} name={:?} anchor={:?} (flag {}) child={:?} tb={}\n{}  {}",
                "  ".repeat(depth),
                s.shape_type,
                s.spid,
                s.flags,
                obj,
                s.string(0x0380),
                s.anchor,
                s.anchor_flag,
                s.child_anchor,
                s.has_textbox,
                "  ".repeat(depth),
                props.join(" ")
            );
            for c in &s.children {
                show(c, depth + 1, k, objs);
            }
        }
        for s in rcell::legacy::escher::parse(art) {
            show(&s, 0, &mut k, objs);
        }
        for t in texts.iter() {
            println!("  TXO {t:?}");
        }
        art.clear();
        objs.clear();
        texts.clear();
    };
    for r in rcell::legacy::biff::records(&stream) {
        match r.rtype {
            0x0809 => {
                flush(sheet, &mut art, &mut objs, &mut texts);
                sheet += 1;
            }
            0x00EC => art.extend_from_slice(r.data),
            0x005D if r.data.len() >= 8 => objs.push((
                u16::from_le_bytes([r.data[4], r.data[5]]),
                u16::from_le_bytes([r.data[6], r.data[7]]),
            )),
            0x01B6 => texts.push(
                r.data
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => {}
        }
    }
    flush(sheet, &mut art, &mut objs, &mut texts);
    Ok(())
}

fn records(
    file: &Path,
    preview: usize,
    only: Option<&str>,
    part: Option<&str>,
) -> rcell::Result<()> {
    let bytes = read_plain(file)?;
    if rcell::detect(&bytes) == rcell::FileKind::Zip {
        let part = part.unwrap_or("xl/workbook.bin");
        let data = rcell::container::read_entry(&bytes, part)?;
        let only = only.and_then(|s| s.parse::<u32>().ok());
        for r in rcell::binary::biff12::records(&data) {
            if only.is_some_and(|t| t != r.rtype) {
                continue;
            }
            println!(
                "{:06x} {:>5} {:<24} {:>5}  {}",
                r.offset,
                r.rtype,
                r.name(),
                r.data.len(),
                hex_preview(r.data, preview)
            );
        }
        return Ok(());
    }
    let stream = rcell::container::read_entry(&bytes, "/Workbook")?;
    let only = only.and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok());
    for r in rcell::legacy::biff::records(&stream) {
        if only.is_some_and(|t| t != r.rtype) {
            continue;
        }
        let hex: Vec<String> = r
            .data
            .iter()
            .take(preview)
            .map(|b| format!("{b:02x}"))
            .collect();
        let more = if r.data.len() > preview { " …" } else { "" };
        println!(
            "{:06x} {:04x} {:<14} {:>5}  {}{more}",
            r.offset,
            r.rtype,
            r.name(),
            r.data.len(),
            hex.join(" ")
        );
    }
    Ok(())
}

fn patch(input: &Path, output: &Path, parts: &[String]) -> rcell::Result<()> {
    let mut reps = Vec::new();
    for p in parts {
        let (name, file) = p
            .split_once('=')
            .ok_or_else(|| rcell::Error::InvalidValue {
                context: "--part".into(),
                value: p.clone(),
            })?;
        reps.push((name.to_string(), std::fs::read(file)?));
    }
    let out = rcell::container::replace_zip_parts(&std::fs::read(input)?, &reps)?;
    std::fs::write(output, out)?;
    Ok(())
}

fn convert(
    input: &Path,
    output: &Path,
    format: Option<&str>,
    abs_path: Option<String>,
    exact: bool,
) -> rcell::Result<()> {
    let format = match format.map(str::to_ascii_lowercase).as_deref() {
        None => rcell::SaveFormat::from_path(output),
        Some("cell" | "xml") => rcell::SaveFormat::Cell,
        Some("xlsx") => rcell::SaveFormat::Xlsx,
        Some("bin" | "binary" | "xlsb") => rcell::SaveFormat::Binary,
        Some("legacy") => rcell::SaveFormat::Legacy,
        Some("csv") => rcell::SaveFormat::Csv,
        Some("txt" | "tsv" | "text") => rcell::SaveFormat::Text,
        Some("prn") => rcell::SaveFormat::Prn,
        Some("xls") => rcell::SaveFormat::Xls,
        Some(other) => {
            return Err(rcell::Error::InvalidValue {
                context: "--format (cell, xlsx, bin, legacy, xls, csv, txt, prn)".into(),
                value: other.into(),
            });
        }
    };
    let wb = open_doc(input)?;
    match format {
        rcell::SaveFormat::Binary => {
            let out = rcell::binary::writer::write_report(
                &wb,
                &rcell::binary::writer::Options { abs_path, exact },
            )?;
            for w in &out.warnings {
                eprintln!("경고: {w}");
            }
            std::fs::write(output, out.bytes)?;
            Ok(())
        }
        rcell::SaveFormat::Legacy => {
            let (bytes, report) = rcell::legacy::write_with_options(
                &wb,
                &rcell::legacy::WriteOptions::for_path(output),
            )?;
            for w in &report.warnings {
                eprintln!("경고: {w}");
            }
            std::fs::write(output, bytes)?;
            Ok(())
        }
        _ => rcell::save_as(&wb, output, format),
    }
}

fn save_format(output: &Path, format: Option<&str>) -> rcell::Result<rcell::SaveFormat> {
    Ok(match format.map(str::to_ascii_lowercase).as_deref() {
        None => rcell::SaveFormat::from_path(output),
        Some("cell" | "xml") => rcell::SaveFormat::Cell,
        Some("xlsx") => rcell::SaveFormat::Xlsx,
        Some("bin" | "binary" | "xlsb") => rcell::SaveFormat::Binary,
        Some(other) => {
            return Err(rcell::Error::InvalidValue {
                context: "--format (cell, xlsx, bin)".into(),
                value: other.into(),
            });
        }
    })
}

fn edit(
    input: &Path,
    script: &Path,
    output: &Path,
    format: Option<&str>,
    no_recalc: bool,
) -> rcell::Result<()> {
    let format = save_format(output, format)?;
    let mut wb = open_doc(input)?;
    let text = std::fs::read_to_string(script)?;
    let script: rcell::edit::Script =
        serde_json::from_str(&text).map_err(|e| rcell::Error::InvalidValue {
            context: format!("편집 대본 {}", script.display()),
            value: e.to_string(),
        })?;
    let mut failed = 0;
    for (i, e) in script.edits.iter().enumerate() {
        if let Err(err) = rcell::edit::apply(&mut wb, e) {
            failed += 1;
            eprintln!("편집 #{i} 실패: {err}");
        }
    }
    if !no_recalc {
        let opts = rcell::calc::CalcOptions {
            file_name: output
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string),
            ..Default::default()
        };
        rcell::calc::recalculate_with(&mut wb, &opts);
    }
    rcell::save_as(&wb, output, format)?;
    eprintln!(
        "편집 {}개 (실패 {failed}) → {}",
        script.edits.len(),
        output.display()
    );
    Ok(())
}

fn bindiff(a: &Path, b: &Path, samples: usize) -> rcell::Result<()> {
    let rep = rcell::binary::parity::compare(&std::fs::read(a)?, &std::fs::read(b)?)?;
    print!("{}", rcell::binary::parity::format(&rep, samples));
    Ok(())
}

fn rec_diff(
    a: &Path,
    b: &Path,
    max: usize,
    summary: bool,
    opts: rcell::legacy::recdiff::Options,
) -> rcell::Result<()> {
    let sa = rcell::container::read_entry(&std::fs::read(a)?, "/Workbook")?;
    let sb = rcell::container::read_entry(&std::fs::read(b)?, "/Workbook")?;
    let r = rcell::legacy::recdiff::compare_with(&sa, &sb, opts);
    if summary {
        println!("{}", r.summary());
        return Ok(());
    }
    for line in r.lines.iter().take(max) {
        println!("{line}");
    }
    if r.lines.len() > max {
        println!("… 외 {}개", r.lines.len() - max);
    }
    println!("{}", r.summary());
    Ok(())
}

/// 같으면 true.
fn diff(
    a: &Path,
    b: &Path,
    max: usize,
    legacy_view: bool,
    opts: rcell::compare::Options,
) -> rcell::Result<bool> {
    let mut wa = open_doc(a)?;
    if legacy_view {
        wa = rcell::legacy::lossy::legacy_view(&wa);
    }
    let wb = open_doc(b)?;
    let diffs = rcell::compare::compare(&wa, &wb, opts);
    for d in diffs.iter().take(max) {
        println!("{d}");
    }
    if diffs.len() > max {
        println!("… 외 {}개", diffs.len() - max);
    }
    println!("차이 {}개", diffs.len());
    Ok(diffs.is_empty())
}

/// 다시 계산해 파일의 값과 비교한다. 휘발성 셀(NOW·RAND …) 말고 다른 값이 없으면 true.
fn calc(file: &Path, show: bool, max: usize, output: Option<&Path>) -> rcell::Result<bool> {
    use rcell::calc::CellStatus;
    let mut wb = open_doc(file)?;
    let opts = rcell::calc::CalcOptions {
        file_name: file.file_name().map(|n| n.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let (results, report) = rcell::calc::evaluate(&wb, &opts);
    let (mut same, mut differ, mut volatile_differ, mut no_cache) =
        (0usize, 0usize, 0usize, 0usize);
    let mut lines = Vec::new();
    for r in &results {
        match rcell::calc::matches_cached(&wb, r) {
            None => no_cache += 1,
            Some(true) => same += 1,
            Some(false) => {
                if r.status == CellStatus::Volatile {
                    volatile_differ += 1;
                } else {
                    differ += 1;
                }
                let sheet = wb
                    .sheets
                    .get(r.sheet)
                    .map(|s| s.name.as_str())
                    .unwrap_or("?");
                lines.push(format!(
                    "{sheet}!{}  ={}  계산 {:?}  파일 {:?}  [{:?}]",
                    r.cell, r.formula, r.value, r.cached, r.status
                ));
            }
        }
    }
    println!(
        "수식 셀 {}: 같음 {same}, 다름 {differ} (휘발성 다름 {volatile_differ}), 파일 값 없음 {no_cache}",
        report.formula_cells
    );
    println!(
        "계산 {}, 휘발성 {}, 지원 안 함 {}, 해석 실패 {}, 순환 {}",
        report.computed, report.volatile, report.unsupported, report.parse_errors, report.cycles
    );
    if !report.unsupported_functions.is_empty() {
        let fns: Vec<String> = report
            .unsupported_functions
            .iter()
            .map(|(k, v)| format!("{k}({v})"))
            .collect();
        println!("지원하지 않는 함수: {}", fns.join(", "));
    }
    if show {
        for l in lines.iter().take(max) {
            println!("{l}");
        }
        if lines.len() > max {
            println!("… 외 {}개", lines.len() - max);
        }
    }
    if let Some(out) = output {
        rcell::calc::recalculate_with(&mut wb, &opts);
        rcell::save(&wb, out)?;
    }
    Ok(differ == 0)
}

fn html(
    file: &Path,
    output: Option<&Path>,
    sheet: Option<&str>,
    no_headers: bool,
    fill: Option<&str>,
) -> rcell::Result<()> {
    let wb = open_doc(file)?;
    let title = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let active_sheet = sheet.and_then(|s| {
        s.parse::<usize>()
            .ok()
            .or_else(|| wb.sheets.iter().position(|x| x.name == s))
    });
    let mut opts = rcell::render::Options {
        active_sheet,
        headers: !no_headers,
        ..Default::default()
    };
    if let Some(f) = fill {
        let (w, h) = f
            .split_once(['x', 'X'])
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
            .ok_or_else(|| rcell::Error::InvalidValue {
                context: "--fill (너비x높이)".into(),
                value: f.into(),
            })?;
        opts.fill = (w, h);
    }
    // 스파크라인 등 모델에 없는 그리기 정보는 원본에서 다시 읽는다 (XML 문서만)
    let extras = rcell::render::Extras::from_bytes(&std::fs::read(file)?);
    let page = rcell::render::workbook_html_with(&wb, &title, opts, &extras);
    match output {
        Some(p) => std::fs::write(p, page)?,
        None => println!("{page}"),
    }
    Ok(())
}
