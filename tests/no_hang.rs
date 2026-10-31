// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 멈춤·충돌 방지: 한셀이 멈추거나 (POISSON.DIST(1E5,1E5,TRUE) 등) 원격 호출 오류로 죽는 입력, 그리고 같은
//! 꼴의 극단값 입력에서 rcell 은 멈추거나 패닉하지 않고 (값이나 오류 값을) 돌려줘야 한다.
//!
//! 입력마다 따로 스레드를 띄워 제한 시간 안에 끝나는지, 패닉하지 않는지 본다 (흑상자 실험 기록: docs/format/calc.md).

use std::sync::mpsc;
use std::time::Duration;

use rcell::model::CellRef;

/// 한 입력의 제한 시간 (느린 CI 도 넉넉히)
const LIMIT: Duration = Duration::from_secs(20);

/// 한셀이 멈추거나 죽은 입력
const HANG_IN_HCELL: &[&str] = &[
    "=POISSON.DIST(1E5,1E5,TRUE)",
    "=POISSON.DIST(1E5,1E5,FALSE)",
    "=POISSON(1E5,1E5,TRUE)",
    "=NUMBERSTRING(1E300,1)",
    "=NUMBERSTRING(1E300,2)",
    "=NUMBERSTRING(1E300,3)",
    "=ISOWEEKNUM(1)",
    "=GAMMA.INV(1,1,1)",
    "=ACCRINT(\"abc\",DATE(2020,1,1),DATE(2020,6,1),0.05,1000,2)",
    "=ACCRINT(DATE(2020,1,1),\"abc\",DATE(2020,6,1),0.05,1000,2)",
    "=ACCRINT(DATE(2020,1,1),DATE(2020,3,1),0,0.05,1000,2)",
    "=ACCRINT(0,DATE(2020,3,1),DATE(2020,6,1),0.05,1000,2)",
    "=ACCRINT(DATE(2020,1,1),DATE(2020,3,1),DATE(2020,6,1),0.05,1000,2,0)",
    "=ACCRINT(DATE(2020,1,1),DATE(2020,3,1),DATE(2020,6,1),0.05,1000,2,4,1)",
];

/// 같은 꼴의 극단값 (무한 반복·거대 할당·재귀 깊이 위험)
const EXTREME: &[&str] = &[
    "=POISSON.DIST(1E12,100,TRUE)",
    "=POISSON.DIST(1E9,1E9,TRUE)",
    "=POISSON.DIST(2147483648,1,TRUE)",
    "=BINOM.DIST(1E9,2E9,0.5,TRUE)",
    "=BINOM.DIST(1E15,1E16,0.5,FALSE)",
    "=BINOM.INV(1E9,0.5,0.5)",
    "=BINOM.DIST.RANGE(1E9,0.5,1,1E9)",
    "=NEGBINOM.DIST(1E9,1E9,0.5,TRUE)",
    "=HYPGEOM.DIST(1E9,2E9,1E9,3E9,TRUE)",
    "=CRITBINOM(1E12,0.5,0.5)",
    "=COMBIN(1E9,5E8)",
    "=COMBINA(1E9,5E8)",
    "=PERMUT(1E9,5E8)",
    "=PERMUTATIONA(1E9,5E8)",
    "=FACT(1E9)",
    "=FACT(170)",
    "=FACTDOUBLE(1E9)",
    "=MULTINOMIAL(1E9,1E9)",
    "=GAMMA(1E300)",
    "=GAMMA(-1E300)",
    "=GAMMALN(1E300)",
    "=GAMMA.DIST(1E300,1E300,1,TRUE)",
    "=GAMMA.INV(0.5,1E300,1E300)",
    "=CHISQ.DIST(1E300,1E10,TRUE)",
    "=CHISQ.INV(0.5,1E10)",
    "=CHISQ.INV.RT(0.5,1E300)",
    "=T.DIST(1E300,1E10,TRUE)",
    "=T.INV(0.5,1E300)",
    "=F.DIST(1E300,1E10,1E10,TRUE)",
    "=F.INV(0.5,1E300,1E300)",
    "=BETA.DIST(0.5,1E300,1E300,TRUE)",
    "=BETA.INV(0.5,1E300,1E300)",
    "=WEIBULL.DIST(1E300,1E300,1,TRUE)",
    "=LOGNORM.INV(0.5,1E300,1E300)",
    "=NORM.S.INV(1E-300)",
    "=CONFIDENCE.T(0.05,1,1E300)",
    "=REPT(\"x\",1E9)",
    "=REPT(\"xyz\",32767)",
    "=REPT(\"x\",32768)",
    "=NUMBERSTRING(1E15,1)",
    "=NUMBERSTRING(-1E300,1)",
    "=ROMAN(3999)",
    "=ROMAN(1E300)",
    "=ARABIC(REPT(\"M\",30000))",
    "=BASE(1E300,2)",
    "=BASE(1E15,2,255)",
    "=DEC2BIN(1E300)",
    "=DECIMAL(REPT(\"1\",255),2)",
    "=RANDARRAY(1E9,1E9)",
    "=SEQUENCE(1E9,1E9)",
    "=SEQUENCE(1E5,1E5)",
    "=MUNIT(1E9)",
    "=MMULT(SEQUENCE(500,500),SEQUENCE(500,500))",
    "=ROW(1:1048576)",
    "=SUM(ROW(1:1048576)*COLUMN(A:XFD))",
    "=SUMPRODUCT(ROW(1:1048576),COLUMN(A:XFD))",
    "=TRANSPOSE(ROW(1:1048576))",
    "=INDEX(A:XFD,1048576,16384)",
    "=MDETERM(SEQUENCE(300,300))",
    "=MINVERSE(SEQUENCE(300,300))",
    "=SUMXMY2(ROW(1:1048576),ROW(1:1048576))",
    "=FREQUENCY(ROW(1:100000),ROW(1:100000))",
    "=LARGE(ROW(1:1048576),1048576)",
    "=PERCENTILE(ROW(1:1048576),0.5)",
    "=RANK(1,ROW(1:1048576))",
    "=TEXTJOIN(\",\",TRUE,ROW(1:100000))",
    "=CONCAT(ROW(1:100000))",
    "=SUBSTITUTE(REPT(\"a\",30000),\"a\",REPT(\"b\",30000))",
    "=FIND(REPT(\"a\",30000),REPT(\"a\",30000)&\"b\")",
    "=SEARCH(REPT(\"a?\",4000),REPT(\"a\",8000)&\"b\")",
    "=SEARCH(REPT(\"*a\",50),REPT(\"a\",5000)&\"b\")",
    "=COUNTIF(A1:A100,REPT(\"*a\",50))",
    "=2^1E9",
    "=(-2)^1E9",
    "=(-2)^0.5",
    "=0^0",
    "=1E308*10",
    "=1E-324/1E300",
    "=MOD(1E300,7)",
    "=MOD(1,1E-300)",
    "=QUOTIENT(1E300,1E-300)",
    "=INT(-1E300)",
    "=ROUND(1E300,-1E9)",
    "=ROUND(1E300,1E9)",
    "=TEXT(1E300,\"0.00000000000000000000000000000000000000000\")",
    "=TEXT(123,REPT(\"0\",300))",
    "=TEXT(1E300,\"#,##0\")",
    "=TEXT(123456789,REPT(\"0,\",100))",
    "=DOLLAR(1E300,1E9)",
    "=FIXED(1E300,1E9)",
    "=DATE(1E9,1E9,1E9)",
    "=DATE(-1E9,1,1)",
    "=EDATE(1,1E9)",
    "=EOMONTH(1,-1E9)",
    "=WORKDAY(1,1E9)",
    "=WORKDAY.INTL(1,-1E9,1)",
    "=NETWORKDAYS(1,1E9)",
    "=NETWORKDAYS.INTL(1,1E9,1)",
    "=DAYS360(1,1E9)",
    "=YEARFRAC(1,1E9,1)",
    "=WEEKNUM(1E9)",
    "=ISOWEEKNUM(1E9)",
    "=ACCRINTM(1,1E9,0.05)",
    "=DISC(1,1E9,1,1)",
    "=COUPNUM(1,1E9,2)",
    "=COUPNCD(1,1E9,2)",
    "=COUPPCD(1,1E9,2)",
    "=COUPDAYS(1,1E9,2)",
    "=COUPDAYBS(1,1E9,2)",
    "=COUPDAYSNC(1,1E9,2)",
    "=PRICE(1,1E9,0.05,0.05,100,2)",
    "=YIELD(1,1E9,0.05,95,100,2)",
    "=YIELD(DATE(2020,1,1),DATE(2030,1,1),0.05,1E-300,100,2)",
    "=YIELD(DATE(2020,1,1),DATE(2030,1,1),0.05,1E300,100,2)",
    "=RATE(1E9,-1,1)",
    "=RATE(1E300,-1,1,0,0,1E300)",
    "=RATE(60,-100,1E300)",
    "=IRR({-1,1E300})",
    "=IRR({-1E300,1})",
    "=XIRR({-100,1E300},{1,2})",
    "=XIRR({-100,110},{1,1E9})",
    "=XNPV(1E300,{-100,110},{1,2})",
    "=NPV(1E300,1,2,3)",
    "=NPER(1E-300,-1,1)",
    "=PMT(1E300,1E9,1)",
    "=FV(1E-300,1E9,1)",
    "=IPMT(0.1,1E9,1E9,1)",
    "=CUMIPMT(0.1,1E9,1,1,1E9,0)",
    "=CUMPRINC(0.1,1E9,1,1,1E9,0)",
    "=AMORDEGRC(1,1,1E9,1,1,0.1)",
    "=AMORLINC(1,1,1E9,1,1,0.1)",
    "=VDB(1,1,1E7,0,1E7)",
    "=DB(1,1,1E9,1)",
    "=SLN(1,1,1E-300)",
    "=SYD(1,1,1E9,1E9)",
    "=DDB(1,1,1E9,1E9)",
    "=MIRR({-1,1},1E300,1E300)",
    "=EFFECT(1E300,1E9)",
    "=NOMINAL(1E300,1E9)",
    "=RRI(1E9,1,1E300)",
    "=PDURATION(1E-300,1,1E300)",
    "=LET(a,SEQUENCE(1E5,1E5),a)",
    "=SORT(SEQUENCE(1E6))",
    "=UNIQUE(SEQUENCE(1E6))",
    "=FILTER(SEQUENCE(1E6),SEQUENCE(1E6)>5)",
    "=TOCOL(SEQUENCE(1000,1000))",
    "=TAKE(SEQUENCE(1000,1000),1E9)",
    "=DROP(SEQUENCE(10),-1E9)",
    "=EXPAND(1,1E9,1E9)",
    "=HSTACK(SEQUENCE(1E6),SEQUENCE(1E6))",
    "=WRAPROWS(SEQUENCE(100),1E9)",
    "=REDUCE(0,SEQUENCE(1E6),LAMBDA(a,b,a+b))",
    "=SCAN(0,SEQUENCE(1E5),LAMBDA(a,b,a+b))",
    "=BYROW(SEQUENCE(1E5,3),LAMBDA(r,SUM(r)))",
    "=MAKEARRAY(1E4,1E4,LAMBDA(r,c,r+c))",
    "=OFFSET(A1,1E9,1E9)",
    "=OFFSET(A1,0,0,1E9,1E9)",
    "=INDIRECT(\"A1:XFD1048576\")",
    "=SUM(INDIRECT(\"A1:XFD1048576\"))",
    "=SUM(A:XFD)",
    "=A1:XFD1048576",
    "=ROWS(A1:XFD1048576)",
    "=COUNTA(A1:XFD1048576)",
    "=VLOOKUP(1,A1:XFD1048576,16384,FALSE)",
    "=MATCH(1,A:A,0)",
    "=XLOOKUP(1,A:A,B:B)",
    "=SUMIFS(A:A,B:B,\">0\",C:C,\"<5\")",
    "=SUMPRODUCT((A:A>0)*(B:B<5))",
    "=SUBTOTAL(9,A:XFD)",
    "=AGGREGATE(14,6,ROW(1:1048576),1)",
    "=INT(1/0)",
    "=A1",
    "=A1+1",
    "=SUM(A1:A1)+A1",
    "=ROW(A1)+A1",
    "=INDIRECT(\"A1\")",
    "=OFFSET(A1,0,0)",
    "=INDEX(A1,1,1)",
    "=CHOOSE(1,A1)",
    "=IF(TRUE,A1)",
    "=LET(x,A1,x)",
    "=LAMBDA(x,x)(A1)",
    "=SUBTOTAL(9,A1)",
    "=AGGREGATE(9,6,A1)",
    "=SUMIF(A1:A1,\">0\")",
    "=SUMIF(A1,\">0\",A1)",
    "=COUNTIF(A1,A1)",
    "=VLOOKUP(A1,A1:A1,1)",
    "=MATCH(A1,A1,0)",
    "=TEXT(A1,\"0\")",
    "=ISFORMULA(A1)",
    "=FORMULATEXT(A1)",
    "=CELL(\"contents\",A1)",
    "=NOW()+A1",
    "=RAND()+A1",
];

/// 시트 한 장에 입력을 넣고 계산해 값까지 읽는다
fn run(formula: &str) {
    let mut wb = rcell::edit::new_workbook();
    // 일부 입력은 값을 필요로 하므로 주변 칸을 채운다
    for (a1, t) in [("B1", "1"), ("B2", "2"), ("B3", "3")] {
        let _ = rcell::edit::set_input(&mut wb, 0, CellRef::parse(a1).unwrap(), t);
    }
    // 입력이 거부되어도 (한셀이 거부하는 꼴) 패닉만 아니면 된다
    if rcell::edit::set_input(&mut wb, 0, CellRef::parse("A1").unwrap(), formula).is_err() {
        return;
    }
    let _ = rcell::calc::recalculate(&mut wb);
    let _ = rcell::calc::evaluate_cell(&wb, 0, CellRef::parse("A1").unwrap());
}

/// 제한 시간 안에 패닉 없이 끝나는지: Err(이유) 면 실패
fn guarded(formula: &'static str) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let t0 = std::time::Instant::now();
    // 스레드 안 패닉은 join 으로 알아낸다. 멈춘 스레드는 남겨 둔 채 실패를 알린다
    let handle = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let r = std::panic::catch_unwind(|| run(formula));
            let _ = tx.send(r.is_ok());
        })
        .expect("스레드");
    match rx.recv_timeout(LIMIT) {
        Ok(true) => {
            let _ = handle.join();
            // 느린 입력은 알린다 (1초 넘게)
            if t0.elapsed() > Duration::from_secs(1) {
                eprintln!("느림 {:.1}초: {formula}", t0.elapsed().as_secs_f64());
            }
            Ok(())
        }
        Ok(false) => Err(format!("패닉: {formula}")),
        Err(_) => Err(format!("{}초 안에 끝나지 않음: {formula}", LIMIT.as_secs())),
    }
}

fn check(list: &[&'static str]) {
    let bad: Vec<String> = list.iter().filter_map(|f| guarded(f).err()).collect();
    assert!(
        bad.is_empty(),
        "멈춤·패닉 {}건:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn hancell_hang_inputs_do_not_hang() {
    check(HANG_IN_HCELL);
}

#[test]
fn extreme_inputs_do_not_hang_or_panic() {
    check(EXTREME);
}

/// 긴 기간의 CUMIPMT·CUMPRINC 닫힌 꼴이 항을 하나씩 더한 값과 (오차 안에서) 같다
#[test]
fn cum_closed_form_matches_loop() {
    let wb = rcell::edit::new_workbook();
    let at = CellRef::parse("A1").unwrap();
    let ev = |f: &str| match rcell::calc::evaluate_formula(&wb, 0, at, f).unwrap() {
        rcell::model::CellValue::Number(x) => x,
        v => panic!("{f}: {v:?}"),
    };
    for (rate, t) in [(0.000004, 0), (0.000001, 1)] {
        for start in [1, 5] {
            // 항 수: 한계(500,000) 이하는 합, 이상은 닫힌 꼴
            let (n, e1, e2) = (800_000, 499_990 + start, 500_000 + start);
            let f = |name: &str, e: i64| format!("={name}({rate},{n},1000000,{start},{e},{t})");
            // 합: 마지막 항들만 따로 더해 같은 구간을 닫힌 꼴로 맞춘다
            for (name, ipmt) in [("CUMIPMT", "IPMT"), ("CUMPRINC", "PPMT")] {
                let tail: f64 = (e1 + 1..=e2)
                    .map(|k| ev(&format!("={ipmt}({rate},{k},{n},1000000,0,{t})")))
                    .sum();
                let loop_sum = ev(&f(name, e1)) + tail;
                let closed = ev(&f(name, e2));
                let tol = 1e-6 * closed.abs().max(1.0);
                assert!(
                    (loop_sum - closed).abs() <= tol,
                    "{name} rate={rate} t={t} start={start}: 합 {loop_sum} 닫힌 꼴 {closed}"
                );
            }
        }
    }
}

/// 항이 아주 많은 분포 함수의 닫힌 꼴이 항을 하나씩 더하는 쪽과 이어진다 (한계 2,000,000 의 앞뒤)
#[test]
fn distribution_closed_forms_continue_the_loop() {
    let wb = rcell::edit::new_workbook();
    let at = CellRef::parse("A1").unwrap();
    let ev = |f: &str| match rcell::calc::evaluate_formula(&wb, 0, at, f).unwrap() {
        rcell::model::CellValue::Number(x) => x,
        v => panic!("{f}: {v:?}"),
    };
    // POISSON: 합(x = 2,000,000) + 다음 항 = 닫힌 꼴(x = 2,000,001)
    let a = ev("=POISSON.DIST(2000000,2000000,TRUE)");
    let pm = ev("=POISSON.DIST(2000001,2000000,FALSE)");
    let b = ev("=POISSON.DIST(2000001,2000000,TRUE)");
    assert!((a + pm - b).abs() < 1e-6, "POISSON {a} + {pm} != {b}");
    // BINOM.DIST: 이웃한 누적의 차 = 정규 밀도에 가깝다
    let (n, p) = (4_000_100.0f64, 0.5f64);
    let c0 = ev("=BINOM.DIST(2000000,4000100,0.5,TRUE)");
    let c1 = ev("=BINOM.DIST(2000001,4000100,0.5,TRUE)");
    let sd = (n * p * (1.0 - p)).sqrt();
    let z = (2_000_001.0 - n * p) / sd;
    let pdf = (-0.5 * z * z).exp() / (sd * (2.0 * std::f64::consts::PI).sqrt());
    assert!(
        ((c1 - c0) - pdf).abs() < 0.01 * pdf,
        "BINOM {c0} {c1} 차 {} 밀도 {pdf}",
        c1 - c0
    );
    // BINOM.INV·CRITBINOM: 중앙값 근처
    for f in ["=BINOM.INV(4000100,0.5,0.5)", "=CRITBINOM(4000100,0.5,0.5)"] {
        let k = ev(f);
        assert!((k - 2_000_050.0).abs() <= 1.0, "{f} = {k}");
    }
    // HYPGEOM.DIST: 0 과 1 사이, 평균 위는 절반을 넘는다
    let h = ev("=HYPGEOM.DIST(2000051,4000100,4000100,8000200,TRUE)");
    assert!((0.5..=1.0).contains(&h), "HYPGEOM {h}");
}

/// 한셀이 멈추는 입력을 담은 문서를 네 서식으로 저장하고 다시 열어 계산해도 패닉·멈춤이 없다
#[test]
fn hang_inputs_round_trip_files() {
    use rcell::SaveFormat;
    let dir = std::env::temp_dir().join(format!("rcell-nohang-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (tx, rx) = mpsc::channel();
    let d2 = dir.clone();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let r = std::panic::catch_unwind(|| {
                let mut wb = rcell::edit::new_workbook();
                let mut n = 0;
                for f in HANG_IN_HCELL.iter().chain(&EXTREME[..40]) {
                    let at = CellRef { row: n, col: 0 };
                    if rcell::edit::set_input(&mut wb, 0, at, f).is_ok() {
                        n += 1;
                    }
                }
                rcell::calc::recalculate(&mut wb);
                for (name, fmt) in [
                    ("a.cell", SaveFormat::Cell),
                    ("a.xlsx", SaveFormat::Xlsx),
                    ("a.bin.cell", SaveFormat::Binary),
                    ("a.legacy.cell", SaveFormat::Legacy),
                ] {
                    let p = d2.join(name);
                    rcell::save_as(&wb, &p, fmt).unwrap();
                    let mut w2 = rcell::open(&p).unwrap();
                    rcell::calc::recalculate(&mut w2);
                }
            });
            let _ = tx.send(r.is_ok());
        })
        .unwrap();
    let ok = rx.recv_timeout(LIMIT * 3);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(ok, Ok(true), "저장·열기·계산이 패닉했거나 시간이 넘었다");
}
