// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 함수 번호 표.
//!
//! 0–367 번은 공개 명세 [MS-XLS] 2.5.198.17 (Ftab) 의 BIFF8 번호와 같다. 368–484 는 [MS-XLSB] Ftab 의
//! Excel 2007 확장 번호로, 바이너리 .cell 이 그대로 쓴다. 레거시 .cell 은 이 함수들을 BIFF8 처럼 사용자
//! 함수(255)와 이름(`_xlfn.…` NAME 또는 추가 기능 EXTERNNAME)으로 쓰고, 480(IFERROR)만 번호로 쓴다.
//! 1904 SUBTOTALA, 1907 CONVERTK 는 한셀 전용 번호다 (함수 대조 실험, `docs/format/cell-legacy.md`).

/// 함수 이름과 고정 인자 수 (`None` 이면 가변 인자 — PtgFuncVar 로만 쓰인다).
pub fn lookup(index: u16) -> Option<(&'static str, Option<u8>)> {
    let (name, argc): (&str, i8) = match index {
        0 => ("COUNT", -1),
        1 => ("IF", -1),
        2 => ("ISNA", 1),
        3 => ("ISERROR", 1),
        4 => ("SUM", -1),
        5 => ("AVERAGE", -1),
        6 => ("MIN", -1),
        7 => ("MAX", -1),
        8 => ("ROW", -1),
        9 => ("COLUMN", -1),
        10 => ("NA", 0),
        11 => ("NPV", -1),
        12 => ("STDEV", -1),
        13 => ("DOLLAR", -1),
        14 => ("FIXED", -1),
        15 => ("SIN", 1),
        16 => ("COS", 1),
        17 => ("TAN", 1),
        18 => ("ATAN", 1),
        19 => ("PI", 0),
        20 => ("SQRT", 1),
        21 => ("EXP", 1),
        22 => ("LN", 1),
        23 => ("LOG10", 1),
        24 => ("ABS", 1),
        25 => ("INT", 1),
        26 => ("SIGN", 1),
        27 => ("ROUND", 2),
        28 => ("LOOKUP", -1),
        29 => ("INDEX", -1),
        30 => ("REPT", 2),
        31 => ("MID", 3),
        32 => ("LEN", 1),
        33 => ("VALUE", 1),
        34 => ("TRUE", 0),
        35 => ("FALSE", 0),
        36 => ("AND", -1),
        37 => ("OR", -1),
        38 => ("NOT", 1),
        39 => ("MOD", 2),
        40 => ("DCOUNT", 3),
        41 => ("DSUM", 3),
        42 => ("DAVERAGE", 3),
        43 => ("DMIN", 3),
        44 => ("DMAX", 3),
        45 => ("DSTDEV", 3),
        46 => ("VAR", -1),
        47 => ("DVAR", 3),
        48 => ("TEXT", 2),
        49 => ("LINEST", -1),
        50 => ("TREND", -1),
        51 => ("LOGEST", -1),
        52 => ("GROWTH", -1),
        56 => ("PV", -1),
        57 => ("FV", -1),
        58 => ("NPER", -1),
        59 => ("PMT", -1),
        60 => ("RATE", -1),
        61 => ("MIRR", 3),
        62 => ("IRR", -1),
        63 => ("RAND", 0),
        64 => ("MATCH", -1),
        65 => ("DATE", 3),
        66 => ("TIME", 3),
        67 => ("DAY", 1),
        68 => ("MONTH", 1),
        69 => ("YEAR", 1),
        70 => ("WEEKDAY", -1),
        71 => ("HOUR", 1),
        72 => ("MINUTE", 1),
        73 => ("SECOND", 1),
        74 => ("NOW", 0),
        75 => ("AREAS", 1),
        76 => ("ROWS", 1),
        77 => ("COLUMNS", 1),
        78 => ("OFFSET", -1),
        82 => ("SEARCH", -1),
        83 => ("TRANSPOSE", 1),
        86 => ("TYPE", 1),
        97 => ("ATAN2", 2),
        98 => ("ASIN", 1),
        99 => ("ACOS", 1),
        100 => ("CHOOSE", -1),
        101 => ("HLOOKUP", -1),
        102 => ("VLOOKUP", -1),
        105 => ("ISREF", 1),
        109 => ("LOG", -1),
        111 => ("CHAR", 1),
        112 => ("LOWER", 1),
        113 => ("UPPER", 1),
        114 => ("PROPER", 1),
        115 => ("LEFT", -1),
        116 => ("RIGHT", -1),
        117 => ("EXACT", 2),
        118 => ("TRIM", 1),
        119 => ("REPLACE", 4),
        120 => ("SUBSTITUTE", -1),
        121 => ("CODE", 1),
        124 => ("FIND", -1),
        125 => ("CELL", -1),
        126 => ("ISERR", 1),
        127 => ("ISTEXT", 1),
        128 => ("ISNUMBER", 1),
        129 => ("ISBLANK", 1),
        130 => ("T", 1),
        131 => ("N", 1),
        140 => ("DATEVALUE", 1),
        141 => ("TIMEVALUE", 1),
        142 => ("SLN", 3),
        143 => ("SYD", 4),
        144 => ("DDB", -1),
        148 => ("INDIRECT", -1),
        162 => ("CLEAN", 1),
        163 => ("MDETERM", 1),
        164 => ("MINVERSE", 1),
        165 => ("MMULT", 2),
        167 => ("IPMT", -1),
        168 => ("PPMT", -1),
        169 => ("COUNTA", -1),
        183 => ("PRODUCT", -1),
        184 => ("FACT", 1),
        189 => ("DPRODUCT", 3),
        190 => ("ISNONTEXT", 1),
        193 => ("STDEVP", -1),
        194 => ("VARP", -1),
        195 => ("DSTDEVP", 3),
        196 => ("DVARP", 3),
        197 => ("TRUNC", -1),
        198 => ("ISLOGICAL", 1),
        199 => ("DCOUNTA", 3),
        204 => ("USDOLLAR", -1),
        205 => ("FINDB", -1),
        206 => ("SEARCHB", -1),
        207 => ("REPLACEB", 4),
        208 => ("LEFTB", -1),
        209 => ("RIGHTB", -1),
        210 => ("MIDB", 3),
        211 => ("LENB", 1),
        212 => ("ROUNDUP", 2),
        213 => ("ROUNDDOWN", 2),
        214 => ("ASC", 1),
        215 => ("DBCS", 1),
        216 => ("RANK", -1),
        219 => ("ADDRESS", -1),
        220 => ("DAYS360", -1),
        221 => ("TODAY", 0),
        222 => ("VDB", -1),
        227 => ("MEDIAN", -1),
        228 => ("SUMPRODUCT", -1),
        229 => ("SINH", 1),
        230 => ("COSH", 1),
        231 => ("TANH", 1),
        232 => ("ASINH", 1),
        233 => ("ACOSH", 1),
        234 => ("ATANH", 1),
        235 => ("DGET", 3),
        244 => ("INFO", 1),
        247 => ("DB", -1),
        252 => ("FREQUENCY", 2),
        261 => ("ERROR.TYPE", 1),
        269 => ("AVEDEV", -1),
        270 => ("BETADIST", -1),
        271 => ("GAMMALN", 1),
        272 => ("BETAINV", -1),
        273 => ("BINOMDIST", 4),
        274 => ("CHIDIST", 2),
        275 => ("CHIINV", 2),
        276 => ("COMBIN", 2),
        277 => ("CONFIDENCE", 3),
        278 => ("CRITBINOM", 3),
        279 => ("EVEN", 1),
        280 => ("EXPONDIST", 3),
        281 => ("FDIST", 3),
        282 => ("FINV", 3),
        283 => ("FISHER", 1),
        284 => ("FISHERINV", 1),
        285 => ("FLOOR", 2),
        286 => ("GAMMADIST", 4),
        287 => ("GAMMAINV", 3),
        288 => ("CEILING", 2),
        289 => ("HYPGEOMDIST", 4),
        290 => ("LOGNORMDIST", 3),
        291 => ("LOGINV", 3),
        292 => ("NEGBINOMDIST", 3),
        293 => ("NORMDIST", 4),
        294 => ("NORMSDIST", 1),
        295 => ("NORMINV", 3),
        296 => ("NORMSINV", 1),
        297 => ("STANDARDIZE", 3),
        298 => ("ODD", 1),
        299 => ("PERMUT", 2),
        300 => ("POISSON", 3),
        301 => ("TDIST", 3),
        302 => ("WEIBULL", 4),
        303 => ("SUMXMY2", 2),
        304 => ("SUMX2MY2", 2),
        305 => ("SUMX2PY2", 2),
        306 => ("CHITEST", 2),
        307 => ("CORREL", 2),
        308 => ("COVAR", 2),
        309 => ("FORECAST", 3),
        310 => ("FTEST", 2),
        311 => ("INTERCEPT", 2),
        312 => ("PEARSON", 2),
        313 => ("RSQ", 2),
        314 => ("STEYX", 2),
        315 => ("SLOPE", 2),
        316 => ("TTEST", 4),
        317 => ("PROB", -1),
        318 => ("DEVSQ", -1),
        319 => ("GEOMEAN", -1),
        320 => ("HARMEAN", -1),
        321 => ("SUMSQ", -1),
        322 => ("KURT", -1),
        323 => ("SKEW", -1),
        324 => ("ZTEST", -1),
        325 => ("LARGE", 2),
        326 => ("SMALL", 2),
        327 => ("QUARTILE", 2),
        328 => ("PERCENTILE", 2),
        329 => ("PERCENTRANK", -1),
        330 => ("MODE", -1),
        331 => ("TRIMMEAN", 2),
        332 => ("TINV", 2),
        336 => ("CONCATENATE", -1),
        337 => ("POWER", 2),
        342 => ("RADIANS", 1),
        343 => ("DEGREES", 1),
        344 => ("SUBTOTAL", -1),
        345 => ("SUMIF", -1),
        346 => ("COUNTIF", 2),
        347 => ("COUNTBLANK", 1),
        350 => ("ISPMT", 4),
        351 => ("DATEDIF", 3),
        352 => ("DATESTRING", 1),
        353 => ("NUMBERSTRING", 2),
        354 => ("ROMAN", -1),
        358 => ("GETPIVOTDATA", -1),
        359 => ("HYPERLINK", -1),
        360 => ("PHONETIC", 1),
        361 => ("AVERAGEA", -1),
        362 => ("MAXA", -1),
        363 => ("MINA", -1),
        364 => ("STDEVPA", -1),
        365 => ("VARPA", -1),
        366 => ("STDEVA", -1),
        367 => ("VARA", -1),
        // ---- Excel 2007 확장 번호 ([MS-XLSB] Ftab, 바이너리 .cell 이 그대로 쓴다; 한셀 확인: 바이너리 대조 실험) ----
        150 => ("CALL", -1),
        368 => ("BAHTTEXT", 1),
        369 => ("THAIDAYOFWEEK", 1),
        370 => ("THAIDIGIT", 1),
        371 => ("THAIMONTHOFYEAR", 1),
        372 => ("THAINUMSOUND", 1),
        373 => ("THAINUMSTRING", 1),
        374 => ("THAISTRINGLENGTH", 1),
        375 => ("ISTHAIDIGIT", 1),
        376 => ("ROUNDBAHTDOWN", 1),
        377 => ("ROUNDBAHTUP", 1),
        378 => ("THAIYEAR", 1),
        379 => ("RTD", -1),
        380 => ("CUBEVALUE", -1),
        381 => ("CUBEMEMBER", -1),
        382 => ("CUBEMEMBERPROPERTY", 3),
        383 => ("CUBERANKEDMEMBER", -1),
        384 => ("HEX2BIN", -1),
        385 => ("HEX2DEC", 1),
        386 => ("HEX2OCT", -1),
        387 => ("DEC2BIN", -1),
        388 => ("DEC2HEX", -1),
        389 => ("DEC2OCT", -1),
        390 => ("OCT2BIN", -1),
        391 => ("OCT2HEX", -1),
        392 => ("OCT2DEC", 1),
        393 => ("BIN2DEC", 1),
        394 => ("BIN2OCT", -1),
        395 => ("BIN2HEX", -1),
        396 => ("IMSUB", 2),
        397 => ("IMDIV", 2),
        398 => ("IMPOWER", 2),
        399 => ("IMABS", 1),
        400 => ("IMSQRT", 1),
        401 => ("IMLN", 1),
        402 => ("IMLOG2", 1),
        403 => ("IMLOG10", 1),
        404 => ("IMSIN", 1),
        405 => ("IMCOS", 1),
        406 => ("IMEXP", 1),
        407 => ("IMARGUMENT", 1),
        408 => ("IMCONJUGATE", 1),
        409 => ("IMAGINARY", 1),
        410 => ("IMREAL", 1),
        411 => ("COMPLEX", -1),
        412 => ("IMSUM", -1),
        413 => ("IMPRODUCT", -1),
        414 => ("SERIESSUM", 4),
        415 => ("FACTDOUBLE", 1),
        416 => ("SQRTPI", 1),
        417 => ("QUOTIENT", 2),
        418 => ("DELTA", -1),
        419 => ("GESTEP", -1),
        420 => ("ISEVEN", 1),
        421 => ("ISODD", 1),
        422 => ("MROUND", 2),
        423 => ("ERF", -1),
        424 => ("ERFC", 1),
        425 => ("BESSELJ", 2),
        426 => ("BESSELK", 2),
        427 => ("BESSELY", 2),
        428 => ("BESSELI", 2),
        429 => ("XIRR", -1),
        430 => ("XNPV", 3),
        431 => ("PRICEMAT", -1),
        432 => ("YIELDMAT", -1),
        433 => ("INTRATE", -1),
        434 => ("RECEIVED", -1),
        435 => ("DISC", -1),
        436 => ("PRICEDISC", -1),
        437 => ("YIELDDISC", -1),
        438 => ("TBILLEQ", 3),
        439 => ("TBILLPRICE", 3),
        440 => ("TBILLYIELD", 3),
        441 => ("PRICE", -1),
        442 => ("YIELD", -1),
        443 => ("DOLLARDE", 2),
        444 => ("DOLLARFR", 2),
        445 => ("NOMINAL", 2),
        446 => ("EFFECT", 2),
        447 => ("CUMPRINC", 6),
        448 => ("CUMIPMT", 6),
        449 => ("EDATE", 2),
        450 => ("EOMONTH", 2),
        451 => ("YEARFRAC", -1),
        452 => ("COUPDAYBS", -1),
        453 => ("COUPDAYS", -1),
        454 => ("COUPDAYSNC", -1),
        455 => ("COUPNCD", -1),
        456 => ("COUPNUM", -1),
        457 => ("COUPPCD", -1),
        458 => ("DURATION", -1),
        459 => ("MDURATION", -1),
        460 => ("ODDLPRICE", -1),
        461 => ("ODDLYIELD", -1),
        462 => ("ODDFPRICE", -1),
        463 => ("ODDFYIELD", -1),
        464 => ("RANDBETWEEN", 2),
        465 => ("WEEKNUM", -1),
        466 => ("AMORDEGRC", -1),
        467 => ("AMORLINC", -1),
        468 => ("CONVERT", 3),
        469 => ("ACCRINT", -1),
        470 => ("ACCRINTM", -1),
        471 => ("WORKDAY", -1),
        472 => ("NETWORKDAYS", -1),
        473 => ("GCD", -1),
        474 => ("MULTINOMIAL", -1),
        475 => ("LCM", -1),
        476 => ("FVSCHEDULE", 2),
        477 => ("CUBEKPIMEMBER", -1),
        478 => ("CUBESET", -1),
        479 => ("CUBESETCOUNT", 1),
        480 => ("IFERROR", 2),
        481 => ("COUNTIFS", -1),
        482 => ("SUMIFS", -1),
        483 => ("AVERAGEIF", -1),
        484 => ("AVERAGEIFS", -1),
        // ---- 한셀 전용 번호 (관측: 바이너리 .cell) ----
        1904 => ("SUBTOTALA", -1),
        1907 => ("CONVERTK", 4),
        _ => return None,
    };
    Some((name, (argc >= 0).then_some(argc as u8)))
}

/// 함수 이름 → (번호, 고정 인자 수). [`lookup`] 의 역 (대소문자 무시).
pub fn by_name(name: &str) -> Option<(u16, Option<u8>)> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static MAP: OnceLock<HashMap<&'static str, (u16, Option<u8>)>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        (0..2048u16)
            .filter_map(|i| lookup(i).map(|(n, a)| (n, (i, a))))
            .collect()
    });
    map.get(name.to_ascii_uppercase().as_str()).copied()
}

/// 수식 토큰의 클래스 ([MS-XLSB] 2.5.97.88 Ptg 의 type 2비트): 참조, 값, 배열
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgClass {
    Ref,
    Value,
    Array,
}

/// 함수의 반환 클래스와 인자 클래스. 인자 목록 밖의 인자는 `cycle` 번째부터 끝까지를 되풀이한다.
#[derive(Debug, Clone, Copy)]
pub struct Signature {
    pub ret: ArgClass,
    params: &'static [ArgClass],
    cycle: usize,
}

impl Signature {
    /// `i` 번째(0부터) 인자의 클래스
    pub fn param(&self, i: usize) -> ArgClass {
        let p = self.params;
        if p.is_empty() {
            return ArgClass::Value;
        }
        if i < p.len() {
            return p[i];
        }
        let c = self.cycle.min(p.len() - 1);
        p[c + (i - c) % (p.len() - c)]
    }
}

/// 함수가 인자를 어떤 클래스로 받는지 (참조를 받는 함수 — 합계·찾기·조건 범위 등 — 는 참조, 배열 연산 함수는
/// 배열, 나머지는 값). 한셀이 바이너리에 쓰는 토큰 클래스와 같다 (함수 대조 실험: `IF` 의 둘째·셋째 인자는 참조,
/// `SUM`·`MATCH`·`VLOOKUP`·`SUMIF`·`COUNTIF`·`INDEX` 의 범위는 참조, `SUMPRODUCT` 는 배열)
pub fn signature(index: u16) -> Signature {
    use ArgClass::{Array as A, Ref as R, Value as V};
    let s = |ret, params: &'static [ArgClass], cycle| Signature { ret, params, cycle };
    match index {
        // 참조 목록 (집계)
        0
        | 4..=7
        | 12
        | 36
        | 37
        | 46
        | 169
        | 183
        | 193
        | 194
        | 227
        | 269
        | 318..=323
        | 361..=367 => s(V, &[R], 0),
        1 => s(R, &[V, R, R], 1),          // IF
        8 | 9 | 75 | 105 => s(V, &[R], 0), // ROW, COLUMN, AREAS, ISREF
        11 => s(V, &[V, R], 1),            // NPV
        28 => s(V, &[V, R, R], 1),         // LOOKUP
        29 => s(R, &[R, V, V, V], 1),      // INDEX
        40..=45 | 47 | 189 | 195 | 196 | 199 | 235 => s(V, &[R, V, R], 2), // D 함수
        49 | 51 => s(A, &[A, A, V, V], 2), // LINEST, LOGEST
        50 | 52 => s(A, &[A, A, A, V], 3), // TREND, GROWTH
        61 => s(V, &[R, V, V], 1),         // MIRR
        62 => s(V, &[R, V], 1),            // IRR
        64 => s(V, &[V, R, V], 2),         // MATCH
        76 | 77 => s(V, &[A], 0),          // ROWS, COLUMNS
        78 => s(R, &[R, V, V, V, V], 1),   // OFFSET
        83 => s(A, &[A], 0),               // TRANSPOSE
        100 => s(R, &[V, R], 1),           // CHOOSE
        101 | 102 => s(V, &[V, R, V, V], 2), // HLOOKUP, VLOOKUP
        125 => s(V, &[V, R], 1),           // CELL
        130 | 131 => s(V, &[R], 0),        // T, N
        148 => s(R, &[V, V], 1),           // INDIRECT
        163 => s(V, &[A], 0),              // MDETERM
        164 => s(A, &[A], 0),              // MINVERSE
        165 => s(A, &[A, A], 1),           // MMULT
        216 => s(V, &[V, R, V], 2),        // RANK
        228 => s(V, &[A], 0),              // SUMPRODUCT
        303..=308 | 310..=315 => s(V, &[A, A], 1),
        309 => s(V, &[V, A, A], 2),          // FORECAST
        316 | 317 => s(V, &[A, A, V, V], 2), // TTEST, PROB
        324 => s(V, &[R, V, V], 1),          // ZTEST
        325..=328 | 331 => s(V, &[R, V], 1), // LARGE, SMALL, QUARTILE, PERCENTILE, TRIMMEAN
        329 => s(V, &[R, V, V], 1),          // PERCENTRANK
        330 => s(V, &[A], 0),                // MODE
        344 | 1904 => s(V, &[V, R], 1),      // SUBTOTAL, SUBTOTALA
        345 | 483 => s(V, &[R, V, R], 2),    // SUMIF, AVERAGEIF
        346 => s(V, &[R, V], 1),             // COUNTIF
        347 => s(V, &[R], 0),                // COUNTBLANK
        481 => s(V, &[R, V], 0),             // COUNTIFS (범위, 조건) 반복
        482 | 484 => s(V, &[R, R, V], 1),    // SUMIFS, AVERAGEIFS: 합계 범위 + (범위, 조건) 반복
        _ => s(V, &[], 0),
    }
}

/// 다시 계산할 때마다 값이 바뀔 수 있는 함수 (수식 앞에 tAttrVolatile, 셀은 fAlwaysCalc)
pub fn is_volatile(index: u16) -> bool {
    // RAND, NOW, OFFSET, CELL, INDIRECT, TODAY, INFO, RANDBETWEEN
    matches!(index, 63 | 74 | 78 | 125 | 148 | 221 | 244 | 464)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_lookup_and_classes() {
        assert_eq!(by_name("sum"), Some((4, None)));
        assert_eq!(by_name("IFERROR"), Some((480, Some(2))));
        assert_eq!(by_name("NOPE"), None);
        let iff = signature(1);
        assert_eq!(iff.ret, ArgClass::Ref);
        assert_eq!(iff.param(0), ArgClass::Value);
        assert_eq!(iff.param(2), ArgClass::Ref);
        let sumifs = signature(482);
        assert_eq!(sumifs.param(0), ArgClass::Ref);
        assert_eq!(sumifs.param(3), ArgClass::Ref);
        assert_eq!(sumifs.param(4), ArgClass::Value);
        assert_eq!(signature(24).param(0), ArgClass::Value);
        assert!(is_volatile(221) && !is_volatile(4));
    }

    #[test]
    fn known_indices() {
        assert_eq!(lookup(4), Some(("SUM", None)));
        assert_eq!(lookup(27), Some(("ROUND", Some(2))));
        assert_eq!(lookup(480), Some(("IFERROR", Some(2))));
        assert_eq!(lookup(449), Some(("EDATE", Some(2))));
        assert_eq!(lookup(482), Some(("SUMIFS", None)));
        assert_eq!(lookup(1907), Some(("CONVERTK", Some(4))));
        assert_eq!(lookup(9999), None);
    }
}
