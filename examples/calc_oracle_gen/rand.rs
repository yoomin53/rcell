// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 무작위 수식 생성기 (한셀과 차분 시험).
//!
//! `--random N --seed S` 로 수식 N 개를 만든다. 함수마다 인수 자리의 종류(숫자·글·논리·날짜·범위·배열 …)를
//! 적은 표(`SIGS`)에서 인수를 뽑고, 깊이 3 까지 함수·연산자를 겹친다. 같은 시드는 같은 목록을 만든다
//! (`--emit 파일` 로 목록을 글로 남긴다). 자료는 `Data` 시트의 지어낸 값이다.
//!
//! 인수 종류 글자: n 숫자 · i 작은 정수 · I 정수(음수 포함) · p 확률 · e 이율 · t 글 · b 논리 · d 날짜 · x 아무 값 ·
//! X 값·범위·배열 · r 숫자 범위 · R 범위 · a 배열 · k 조건 글 · f 서식 글 · m 일치 유형 · c 열 번호 · y 0–2 ·
//! z 0–4 · w 주 번호 유형 · u 단위 글 · g 복소수 글 · q 진법 글 · M 행렬 · S 부분합 번호 · Y·D 연·일.
//! 인수 글자 뒤 `?` 는 생략할 수 있다는 뜻.

use crate::cases::Case;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn pick<'a, T: ?Sized>(&mut self, v: &[&'a T]) -> &'a T {
        v[self.below(v.len())]
    }
}

/// (이름들, 인수 종류, 돌려주는 종류 n·t·b·x)
const SIGS: &[(&str, &str, char)] = &[
    (
        "ABS ACOS ACOSH ACOT ACOTH ASIN ASINH ATAN ATANH COS COSH COT COTH CSC CSCH DEGREES EVEN EXP FACT FACTDOUBLE FISHER FISHERINV GAMMA GAMMALN GAMMALN.PRECISE INT LN LOG10 ODD RADIANS SEC SECH SIGN SIN SINH SQRT SQRTPI TAN TANH NORMSDIST NORMSINV NORM.S.INV PHI GAUSS ERF.PRECISE ERFC ERFC.PRECISE N",
        "n",
        'n',
    ),
    (
        "ATAN2 POWER MOD QUOTIENT ROUND ROUNDUP ROUNDDOWN MROUND CEILING FLOOR COMBIN COMBINA PERMUT PERMUTATIONA BITAND BITOR BITXOR",
        "n n",
        'n',
    ),
    (
        "DELTA GESTEP TRUNC LOG ERF CEILING.MATH FLOOR.MATH CEILING.PRECISE FLOOR.PRECISE",
        "n n?",
        'n',
    ),
    ("BITLSHIFT BITRSHIFT", "I i", 'n'),
    ("LEN LENB CODE UNICODE TYPE", "t", 'n'),
    ("LOWER UPPER PROPER TRIM CLEAN ASC DBCS JUNJA T", "t", 't'),
    ("VALUE DATEVALUE TIMEVALUE ARABIC", "t", 'n'),
    ("CHAR UNICHAR", "I", 't'),
    ("LEFT RIGHT LEFTB RIGHTB", "t i?", 't'),
    ("MID MIDB", "t I I", 't'),
    ("REPLACE REPLACEB", "t I I t", 't'),
    ("SUBSTITUTE", "t t t i?", 't'),
    ("FIND SEARCH FINDB SEARCHB", "t t I?", 'n'),
    ("REPT", "t i", 't'),
    ("EXACT", "t t", 'b'),
    ("TEXT", "x f", 't'),
    ("FIXED", "n i? b?", 't'),
    ("DOLLAR USDOLLAR", "n i?", 't'),
    ("NUMBERVALUE", "t t? t?", 'n'),
    ("CONCAT CONCATENATE", "x x? x?", 't'),
    ("TEXTJOIN", "t b X X?", 't'),
    ("NUMBERSTRING ROMAN", "n y", 't'),
    ("BASE", "I i i?", 't'),
    ("DECIMAL", "t i", 'n'),
    ("DEC2BIN DEC2HEX DEC2OCT", "I i?", 't'),
    ("BIN2DEC HEX2DEC OCT2DEC", "q", 'n'),
    (
        "BIN2HEX BIN2OCT HEX2BIN HEX2OCT OCT2BIN OCT2HEX",
        "q i?",
        't',
    ),
    ("DATE", "Y m D", 'n'),
    ("TIME", "I I I", 'n'),
    ("DAY MONTH YEAR HOUR MINUTE SECOND", "d", 'n'),
    ("WEEKDAY", "d y?", 'n'),
    ("WEEKNUM", "d w?", 'n'),
    ("EDATE EOMONTH", "d I", 'n'),
    ("DAYS", "d d", 'n'),
    ("DATEDIF", "d d u", 'n'),
    ("DAYS360", "d d b?", 'n'),
    ("NETWORKDAYS", "d d H?", 'n'),
    ("WORKDAY", "d I H?", 'n'),
    ("YEARFRAC", "d d z?", 'n'),
    (
        "SUM AVERAGE MAX MIN COUNT COUNTA PRODUCT MEDIAN STDEV STDEV.S STDEV.P STDEVP VAR VAR.S VAR.P VARP AVEDEV DEVSQ GEOMEAN HARMEAN KURT SKEW MODE MODE.SNGL SUMSQ AVERAGEA MAXA MINA STDEVA VARA STDEVPA VARPA",
        "X X? X?",
        'n',
    ),
    ("SUBTOTAL", "S X", 'n'),
    ("SUMPRODUCT", "X X?", 'n'),
    ("LARGE SMALL", "X I", 'n'),
    (
        "PERCENTILE PERCENTILE.INC PERCENTILE.EXC QUARTILE QUARTILE.INC QUARTILE.EXC TRIMMEAN",
        "X p",
        'n',
    ),
    ("RANK RANK.EQ RANK.AVG", "n X y?", 'n'),
    (
        "CORREL PEARSON SLOPE INTERCEPT RSQ COVAR COVARIANCE.P COVARIANCE.S STEYX",
        "r r",
        'n',
    ),
    ("FORECAST", "n r r", 'n'),
    ("COUNTIF", "X k", 'n'),
    ("SUMIF AVERAGEIF", "X k X?", 'n'),
    ("COUNTBLANK ROWS COLUMNS AREAS", "X", 'n'),
    ("COUNTIFS", "X k", 'n'),
    ("SUMIFS AVERAGEIFS MAXIFS MINIFS", "r r k", 'n'),
    ("VLOOKUP HLOOKUP", "x R c b?", 'x'),
    ("LOOKUP", "x R R?", 'x'),
    ("MATCH", "x R o?", 'n'),
    ("INDEX", "R c? c?", 'x'),
    ("CHOOSE", "c x x x?", 'x'),
    ("XLOOKUP", "x R R x?", 'x'),
    ("OFFSET", "R I I", 'x'),
    ("ROW COLUMN", "R?", 'n'),
    ("TRANSPOSE", "X", 'x'),
    ("IF", "x x? x?", 'x'),
    ("IFERROR IFNA", "x x", 'x'),
    ("AND OR XOR", "x x? x?", 'b'),
    ("NOT", "x", 'b'),
    ("IFS", "x x x? x?", 'x'),
    ("SWITCH", "x x x x? x?", 'x'),
    (
        "ISBLANK ISERR ISERROR ISNA ISNUMBER ISTEXT ISLOGICAL ISNONTEXT ISREF ISEVEN ISODD",
        "x",
        'b',
    ),
    ("ERROR.TYPE", "x", 'n'),
    ("NORM.DIST NORMDIST", "n n n b", 'n'),
    ("NORM.INV NORMINV", "p n n", 'n'),
    ("NORM.S.DIST", "n b", 'n'),
    ("T.DIST", "n i b", 'n'),
    ("T.DIST.2T T.DIST.RT", "n i", 'n'),
    ("T.INV T.INV.2T TINV", "p i", 'n'),
    ("CHISQ.DIST", "n i b", 'n'),
    ("CHISQ.INV CHISQ.INV.RT CHIINV", "p i", 'n'),
    ("F.DIST", "n i i b", 'n'),
    ("F.INV F.INV.RT FINV", "p i i", 'n'),
    ("BINOM.DIST BINOMDIST", "i i p b", 'n'),
    ("POISSON.DIST POISSON", "i n b", 'n'),
    ("EXPON.DIST EXPONDIST", "n n b", 'n'),
    ("WEIBULL.DIST WEIBULL GAMMA.DIST GAMMADIST", "n n n b", 'n'),
    ("LOGNORM.DIST", "n n n b", 'n'),
    ("LOGNORM.INV", "p n n", 'n'),
    ("STANDARDIZE", "n n n", 'n'),
    ("CONFIDENCE CONFIDENCE.NORM", "p n i", 'n'),
    ("PMT", "e i n n? b?", 'n'),
    ("FV PV", "e i n n? b?", 'n'),
    ("NPER", "e n n n? b?", 'n'),
    ("IPMT PPMT", "e i i n n? b?", 'n'),
    ("NPV", "e X X?", 'n'),
    ("IRR", "r", 'n'),
    ("SLN", "n n i", 'n'),
    ("SYD DB DDB", "n n i i", 'n'),
    ("EFFECT NOMINAL", "e i", 'n'),
    ("MIRR", "r e e", 'n'),
    ("IMABS IMREAL IMAGINARY IMARGUMENT", "g", 'n'),
    (
        "IMCONJUGATE IMSQRT IMEXP IMLN IMSIN IMCOS IMTAN IMSINH IMCOSH IMSEC IMCSC IMLOG10 IMLOG2",
        "g",
        't',
    ),
    ("IMSUM IMSUB IMPRODUCT IMDIV IMPOWER", "g g", 't'),
    ("COMPLEX", "n n u?", 't'),
    ("CONVERT", "n U U", 'n'),
    ("GCD LCM", "I I? I?", 'n'),
    ("MULTINOMIAL", "i i? i?", 'n'),
    ("SUMX2MY2 SUMX2PY2 SUMXMY2", "r r", 'n'),
    ("MDETERM", "M", 'n'),
    ("MINVERSE MUNIT", "M", 'x'),
    ("MMULT", "M M", 'x'),
    ("PROB", "r r n n?", 'n'),
    ("ADDRESS", "I I y? b? t?", 't'),
    ("CELL", "C R", 'x'),
    ("SHEET SHEETS", "X?", 'n'),
];

/// 쓰지 않는 함수 (한셀이 모르거나, 죽거나, 값이 바뀐다)
#[allow(dead_code)]
const SKIP: &[&str] = &[
    "RAND",
    "RANDBETWEEN",
    "RANDARRAY",
    "NOW",
    "TODAY",
    "INFO",
    "ISOWEEKNUM",
    "GAMMAINV",
    "TTEST",
    "WORKDAY.INTL",
];

const NUMS: &[&str] = &[
    "0",
    "1",
    "2",
    "3",
    "5",
    "7",
    "10",
    "12",
    "100",
    "-1",
    "-2",
    "-5",
    "0.5",
    "-0.5",
    "0.25",
    "1.5",
    "2.5",
    "3.14159",
    "-3.7",
    "0.1",
    "0.2",
    "0.3",
    "1E-5",
    "1E5",
    "123456.789",
    "0.000123",
    "1E+15",
    "1E+20",
    "12345678901234567",
    "-1E-7",
    "99999",
    "255",
    "1023",
    "65535",
    "6.02E23",
    "0.99",
    "1.0000001",
    "45000",
    "36526.5",
    "1E-300",
    "2^53",
    "7/3",
];
const TEXTS: &[&str] = &[
    "\"abc\"",
    "\"Hello World\"",
    "\"  pad  \"",
    "\"\"",
    "\"123\"",
    "\"1,234.5\"",
    "\"2024-03-15\"",
    "\"TRUE\"",
    "\"가나다\"",
    "\"ÄÖÜ ß\"",
    "\"a,b,c\"",
    "\"x1\"",
    "\"A1\"",
    "\"12:30\"",
    "\"1/2/2024\"",
    "\"$5\"",
    "\"50%\"",
    "\"1e3\"",
    "\"-\"",
    "\"Ab Cd eF\"",
    "\"2024년 3월 15일\"",
    "\"오전 9시\"",
    "\"Ａ１２\"",
    "\"ｈｅｌｌｏ\"",
    "\"ア\"",
    "\"tab\\t\"",
    "\"a\"\"b\"",
    "\"  12  \"",
    "\"(5)\"",
    "\"1 1/2\"",
    "\"#N/A\"",
    "\"3+4i\"",
    "\"0x1F\"",
    "\"12abc\"",
    "\"abc12\"",
    "\"TrUe\"",
    "\"😀x\"",
    "\"한글 English 123\"",
    "\"Jan 5 2024\"",
    "\"5-Mar-24\"",
];
const FMTS: &[&str] = &[
    "\"0\"",
    "\"0.00\"",
    "\"#,##0\"",
    "\"#,##0.00\"",
    "\"0.0%\"",
    "\"yyyy-mm-dd\"",
    "\"yyyy년 mm월 dd일\"",
    "\"hh:mm:ss\"",
    "\"h:mm AM/PM\"",
    "\"@\"",
    "\"General\"",
    "\"0.00E+00\"",
    "\"# ?/?\"",
    "\"ddd\"",
    "\"dddd\"",
    "\"mmm\"",
    "\"mmmm\"",
    "\"aaa\"",
    "\"aaaa\"",
    "\"[h]:mm\"",
    "\"$#,##0.00\"",
    "\"\\W#,##0\"",
    "\"0;-0;\\z\"",
    "\"00000\"",
    "\"###\"",
    "\"0.0,\"",
    "\"yy/m/d\"",
    "\"[$-412]yyyy-mm-dd\"",
    "\"A/P\"",
    "\"x\"",
    "\"\"",
    "\"[Red]0.0\"",
    "\"0.000\"",
    "\"mm/dd/yy\"",
];
const CRITS: &[&str] = &[
    "\">3\"",
    "\"<5\"",
    "\">=5\"",
    "\"<>0\"",
    "\"=1\"",
    "\"apple\"",
    "\"a*\"",
    "\"?e*\"",
    "\"<>\"",
    "\"=\"",
    "5",
    "\"5\"",
    "\">0\"",
    "\"r1\"",
    "\"*a*\"",
    "\"~*\"",
    "\">\"&3",
    "\"<=\"&Data!A2",
    "TRUE",
    "\"x\"",
    "\">=10\"",
    "\"<0\"",
    "\"b\"",
];
const UNITS: &[&str] = &[
    "\"Y\"", "\"M\"", "\"D\"", "\"MD\"", "\"YM\"", "\"YD\"", "\"y\"", "\"d\"", "\"m\"", "\"X\"",
];
const CONV: &[&str] = &[
    "\"m\"", "\"ft\"", "\"km\"", "\"mi\"", "\"C\"", "\"F\"", "\"K\"", "\"kg\"", "\"lbm\"",
    "\"in\"", "\"yd\"", "\"cm\"",
];
const COMPLEXES: &[&str] = &[
    "\"3+4i\"",
    "\"1-i\"",
    "\"-2.5j\"",
    "\"2\"",
    "\"i\"",
    "\"0\"",
    "\"1+i\"",
    "\"-i\"",
    "\"5i\"",
    "\"abc\"",
    "Data!AG1",
    "Data!AG2",
    "Data!AG3",
    "\"\"",
];
const BASES: &[&str] = &[
    "\"101\"",
    "\"1111111111\"",
    "\"1000000000\"",
    "\"FF\"",
    "\"777\"",
    "\"12\"",
    "\"G\"",
    "\"-1\"",
    "\"0\"",
    "\"7FFFFFFFFF\"",
    "\"FFFFFFFFFF\"",
    "1010",
    "777",
    "\"a\"",
    "\"\"",
    "Data!AH1",
    "Data!AH2",
    "Data!AH3",
];
const INFO_TYPES: &[&str] = &[
    "\"address\"",
    "\"row\"",
    "\"col\"",
    "\"contents\"",
    "\"type\"",
    "\"prefix\"",
    "\"width\"",
    "\"format\"",
    "\"color\"",
    "\"parentheses\"",
    "\"ROW\"",
    "\"x\"",
];

const NUM_RANGES: &[&str] = &[
    "Data!A1:A10",
    "Data!B1:B10",
    "Data!F1:F10",
    "Data!K1:K10",
    "Data!L1:L10",
];
const RANGES: &[&str] = &[
    "Data!A1:A10",
    "Data!A1:A5",
    "Data!B1:B10",
    "Data!F1:F10",
    "Data!K1:K10",
    "Data!L1:L10",
    "Data!A1:B5",
    "Data!A1:C3",
    "Data!D1:D10",
    "Data!C1:C10",
    "Data!H1:H10",
    "Data!J1:J10",
    "Data!E1:E10",
    "Data!A12:E12",
    "Data!A12:E13",
    "Data!M1:O3",
    "Data!AC1:AD4",
    "Data!AA1:AA6",
    "Data!AB1:AB6",
    "Data!AI1:AI5",
    "Data!Z1:Z3",
    "Data!G1:G10",
    "Data!I1:I10",
    "Data!A1:J10",
    "Nums",
    "Arr",
    "Data!A:A",
    "Data!1:1",
    "Data!AG1:AG4",
    "Data!D1:D3",
    "Data!C3:C6",
    "Data!H2:H4",
    "S1:S3!A1",
    "Data!B2:B2",
    "Data!A1",
];
const ARRAYS: &[&str] = &[
    "{1,2,3}",
    "{1;2;3}",
    "{1,2;3,4}",
    "{\"a\",\"b\"}",
    "{1,\"a\",TRUE}",
    "{#N/A,1}",
    "{0,0;0,0}",
    "{1,2,3,4,5}",
    "ROW(1:3)",
    "COLUMN(A:C)",
    "{-1,0,1.5}",
    "{\"x\";\"y\";\"z\"}",
    "{1,2;3,4;5,6}",
    "{TRUE,FALSE}",
    "{\"1\",\"2\"}",
    "{1,#DIV/0!,3}",
    "{5}",
    "{\"\"}",
];
const NUM_CELLS: &[&str] = &[
    "Data!A1", "Data!A2", "Data!A5", "Data!A6", "Data!B1", "Data!B2", "Data!B3", "Data!B8",
    "Data!B9", "Data!F1", "Data!F10", "Data!K3", "Data!L4", "Data!E1", "Data!E7", "Data!E10",
    "Data!C1", "Data!C6", "Data!C9", "Data!C10", "S1!A1", "S2!A2", "S3!A3", "Data!AA2", "Data!AB1",
];
const TEXT_CELLS: &[&str] = &[
    "Data!D1",
    "Data!D2",
    "Data!D5",
    "Data!D8",
    "Data!G1",
    "Data!G10",
    "Data!I3",
    "Data!I10",
    "Data!C2",
    "Data!C5",
    "Data!C8",
    "Data!AI1",
    "Data!AI4",
    "Data!AI5",
    "Data!AG1",
    "Data!AG3",
    "Data!Q1",
    "Data!R2",
    "Data!B1&\"\"",
];
const ANY_CELLS: &[&str] = &[
    "Data!C1", "Data!C2", "Data!C3", "Data!C4", "Data!C5", "Data!C6", "Data!C7", "Data!C8",
    "Data!C9", "Data!H1", "Data!H2", "Data!H4", "Data!H6", "Data!J1", "Data!J2", "Data!Z9",
    "Data!Z10", "Data!D1", "Data!A1", "Data!E3", "Data!AI3", "Data!AI4",
];
const ERRORS: &[&str] = &[
    "#N/A",
    "#DIV/0!",
    "#VALUE!",
    "#REF!",
    "#NUM!",
    "#NAME?",
    "#NULL!",
    "NA()",
    "1/0",
    "SQRT(-1)",
    "LN(0)",
    "\"a\"+1",
    "INDEX({1},2)",
    "Data!H2",
    "Data!H4",
    "Data!H6",
    "Data!H8",
    "Data!H10",
];

/// 함수 한 줄
struct Func {
    name: &'static str,
    args: Vec<(char, bool)>,
    ret: char,
}

pub struct Gen {
    rng: Rng,
    funcs: Vec<Func>,
}

impl Gen {
    pub fn new(seed: u64) -> Gen {
        let mut funcs = Vec::new();
        for (names, sig, ret) in SIGS {
            let args: Vec<(char, bool)> = sig
                .split(' ')
                .map(|t| (t.chars().next().unwrap(), t.ends_with('?')))
                .collect();
            for n in names.split(' ') {
                funcs.push(Func {
                    name: n,
                    args: args.clone(),
                    ret: *ret,
                });
            }
        }
        Gen {
            rng: Rng::new(seed),
            funcs,
        }
    }

    fn call(&mut self, want: char, depth: u32) -> String {
        // 원하는 종류를 돌려주는 함수, 15% 는 아무 값을 돌려주는 함수 (값이 어떻게 바뀌어 쓰이는지 보려고)
        let loose = self.rng.chance(15);
        let idx: Vec<usize> = (0..self.funcs.len())
            .filter(|&i| {
                let r = self.funcs[i].ret;
                want == 'x' || r == want || (loose && r == 'x')
            })
            .collect();
        let i = idx[self.rng.below(idx.len())];
        let (name, args, _) = {
            let f = &self.funcs[i];
            (f.name, f.args.clone(), f.ret)
        };
        let mut out = Vec::new();
        for (k, (cls, opt)) in args.iter().enumerate() {
            if *opt && self.rng.chance(40) && k > 0 {
                break;
            }
            out.push(self.expr(*cls, depth.saturating_sub(1)));
        }
        format!("{name}({})", out.join(","))
    }

    fn op_expr(&mut self, want: char, depth: u32) -> String {
        match want {
            'n' => {
                let a = self.expr('n', depth - 1);
                let b = self.expr('n', depth - 1);
                let op = self.rng.pick(&["+", "-", "*", "/", "^", "+", "-", "*"]);
                match self.rng.below(8) {
                    0 => format!("-{}", wrap(&a)),
                    1 => format!("{}%", wrap(&a)),
                    2 => format!("({a}{op}{b})"),
                    _ => format!("{}{op}{}", wrap(&a), wrap(&b)),
                }
            }
            't' => {
                let a = self.expr('x', depth - 1);
                let b = self.expr('x', depth - 1);
                format!("{}&{}", wrap(&a), wrap(&b))
            }
            _ => {
                let a = self.expr('x', depth - 1);
                let b = self.expr('x', depth - 1);
                let op = self.rng.pick(&["=", "<>", "<", ">", "<=", ">="]);
                format!("{}{op}{}", wrap(&a), wrap(&b))
            }
        }
    }

    pub fn expr(&mut self, cls: char, depth: u32) -> String {
        let nest = depth > 0 && self.rng.chance(55);
        match cls {
            'n' => {
                if nest {
                    if self.rng.chance(30) {
                        self.op_expr('n', depth)
                    } else {
                        self.call('n', depth)
                    }
                } else if self.rng.chance(25) {
                    self.rng.pick(NUM_CELLS).to_string()
                } else {
                    self.rng.pick(NUMS).to_string()
                }
            }
            't' => {
                if nest {
                    if self.rng.chance(30) {
                        self.op_expr('t', depth)
                    } else {
                        self.call('t', depth)
                    }
                } else if self.rng.chance(25) {
                    self.rng.pick(TEXT_CELLS).to_string()
                } else {
                    self.rng.pick(TEXTS).to_string()
                }
            }
            'b' => {
                if nest {
                    if self.rng.chance(45) {
                        self.op_expr('b', depth)
                    } else {
                        self.call('b', depth)
                    }
                } else {
                    self.rng
                        .pick(&[
                            "TRUE", "FALSE", "Data!J1", "Data!J2", "1", "0", "Data!C3", "\"TRUE\"",
                            "Data!C7",
                        ])
                        .to_string()
                }
            }
            'x' => match self.rng.below(100) {
                0..=34 => self.expr('n', depth),
                35..=54 => self.expr('t', depth),
                55..=62 => self.expr('b', depth),
                63..=70 => self.rng.pick(ERRORS).to_string(),
                71..=84 => self.rng.pick(ANY_CELLS).to_string(),
                85..=90 => self.rng.pick(RANGES).to_string(),
                91..=94 => self.rng.pick(ARRAYS).to_string(),
                95..=97 => self.expr('d', depth),
                _ => "\"\"".to_string(),
            },
            'X' => match self.rng.below(100) {
                0..=39 => self.rng.pick(RANGES).to_string(),
                40..=54 => self.rng.pick(ARRAYS).to_string(),
                55..=62 if depth > 0 => self.call('x', depth),
                _ => self.expr('x', depth),
            },
            'r' => self.rng.pick(NUM_RANGES).to_string(),
            'R' => {
                if self.rng.chance(8) {
                    self.rng.pick(ARRAYS).to_string()
                } else {
                    self.rng.pick(RANGES).to_string()
                }
            }
            'a' => self.rng.pick(ARRAYS).to_string(),
            'i' => self
                .rng
                .pick(&[
                    "0", "1", "2", "3", "4", "5", "6", "10", "12", "-1", "1.9", "2.5", "\"2\"",
                    "TRUE", "Data!A2", "Data!A5", "255",
                ])
                .to_string(),
            'I' => self
                .rng
                .pick(&[
                    "0", "1", "2", "3", "5", "10", "-1", "-3", "100", "1000", "0.5", "-0.5", "2.9",
                    "-2.9", "\"3\"", "Data!A1", "Data!A3", "1114111", "65536", "32767", "-32768",
                ])
                .to_string(),
            'p' => self
                .rng
                .pick(&[
                    "0", "1", "0.5", "0.1", "0.25", "0.75", "0.9", "0.99", "0.01", "0.001",
                    "0.999", "0.3", "-0.1", "1.5", "0.6827", "0.95", "0.05",
                ])
                .to_string(),
            'e' => self
                .rng
                .pick(&[
                    "0", "0.05", "0.1", "0.005", "-0.01", "0.08/12", "0.2", "1", "0.0001", "-0.5",
                    "0.5", "1E-10", "0.12/4",
                ])
                .to_string(),
            'd' => {
                if depth > 0 && self.rng.chance(40) {
                    let y = self.expr('Y', 0);
                    let m = self.expr('m', 0);
                    let d = self.expr('D', 0);
                    format!("DATE({y},{m},{d})")
                } else {
                    self.rng
                        .pick(&[
                            "45000",
                            "36526",
                            "0",
                            "1",
                            "60",
                            "61",
                            "-1",
                            "2958465",
                            "2958466",
                            "45658.75",
                            "Data!E1",
                            "Data!E2",
                            "Data!E5",
                            "Data!E7",
                            "Data!E10",
                            "\"2024-03-15\"",
                            "\"1/2/2024\"",
                            "\"2024년 3월 15일\"",
                            "Data!A1",
                            "\"abc\"",
                            "TRUE",
                            "44927",
                            "43831",
                            "45291",
                            "45292",
                            "\"Jan 5 2024\"",
                            "1.5",
                            "59",
                            "366",
                            "40000",
                        ])
                        .to_string()
                }
            }
            'Y' => self
                .rng
                .pick(&[
                    "2024", "1900", "1899", "1904", "2023", "2000", "2100", "9999", "10000", "0",
                    "-1", "99", "1999", "24", "1900.9", "\"2024\"",
                ])
                .to_string(),
            'm' => self
                .rng
                .pick(&[
                    "1", "2", "3", "6", "12", "13", "0", "-1", "25", "-12", "2.9", "\"3\"",
                ])
                .to_string(),
            'D' => self
                .rng
                .pick(&[
                    "1", "15", "28", "29", "30", "31", "32", "0", "-1", "60", "365", "366", "-30",
                    "1.9",
                ])
                .to_string(),
            'y' => self
                .rng
                .pick(&["0", "1", "2", "3", "-1", "TRUE", "FALSE", "1.9"])
                .to_string(),
            'z' => self
                .rng
                .pick(&["0", "1", "2", "3", "4", "5", "-1", "1.5"])
                .to_string(),
            'w' => self
                .rng
                .pick(&[
                    "1", "2", "11", "12", "13", "14", "15", "16", "17", "3", "0", "TRUE",
                ])
                .to_string(),
            'k' => self.rng.pick(CRITS).to_string(),
            'f' => self.rng.pick(FMTS).to_string(),
            'o' => self
                .rng
                .pick(&["0", "1", "-1", "2", "TRUE", "FALSE", "-2", "0.5"])
                .to_string(),
            'c' => self
                .rng
                .pick(&["1", "2", "3", "4", "0", "5", "-1", "1.9", "\"2\"", "TRUE"])
                .to_string(),
            'u' => self.rng.pick(UNITS).to_string(),
            'U' => self.rng.pick(CONV).to_string(),
            'g' => self.rng.pick(COMPLEXES).to_string(),
            'q' => self.rng.pick(BASES).to_string(),
            'M' => self
                .rng
                .pick(&[
                    "Data!M1:O3",
                    "Data!M5:N6",
                    "Data!M8:N9",
                    "Data!A1:B2",
                    "Data!M1:N2",
                    "Data!M1:O1",
                    "Data!M1:M3",
                    "Data!A1:C3",
                    "{1,2;3,4}",
                    "{2}",
                ])
                .to_string(),
            'S' => self
                .rng
                .pick(&[
                    "1", "2", "3", "4", "5", "6", "7", "8", "9", "101", "102", "109", "0", "12",
                    "10", "11",
                ])
                .to_string(),
            'H' => self
                .rng
                .pick(&[
                    "Data!AF1:AF2",
                    "Data!AF1",
                    "Data!E1:E2",
                    "{45300}",
                    "Data!Z1:Z2",
                ])
                .to_string(),
            'C' => self.rng.pick(INFO_TYPES).to_string(),
            other => panic!("모르는 인수 종류 {other}"),
        }
    }

    /// 수식 하나 (최상위는 아무 값)
    pub fn formula(&mut self) -> Case {
        // 배열 수식 (범위를 값 자리에 쓰거나 배열을 다루는 식) 10%
        if self.rng.chance(10) {
            let (h, w) = (4, 3);
            let f = match self.rng.below(7) {
                0 => {
                    let r = self.rng.pick(RANGES);
                    let n = self.expr('n', 1);
                    format!("{r}*{}", wrap(&n))
                }
                1 => {
                    let r = self.rng.pick(RANGES);
                    let c = self.rng.pick(CRITS);
                    format!("IF({r}>{},{r},\"-\")", c[..1].replace('"', "0"))
                }
                2 => format!("TRANSPOSE({})", self.rng.pick(RANGES)),
                3 => {
                    let r = self.rng.pick(RANGES);
                    let f = self.rng.pick(FMTS);
                    format!("TEXT({r},{f})")
                }
                4 => {
                    let n = self.call('n', 2);
                    let r = self.rng.pick(RANGES);
                    format!("{r}+{}", wrap(&n))
                }
                5 => format!("{}&{}", self.rng.pick(RANGES), self.rng.pick(TEXTS)),
                _ => self.call('x', 2),
            };
            return Case {
                f,
                array: Some((h, w)),
            };
        }
        let cls = ['x', 'x', 'n', 'n', 't', 'b'][self.rng.below(6)];
        // 대부분 함수 호출이나 연산식으로 (값 하나만 적은 식은 시험이 되지 않는다)
        let f = match self.rng.below(100) {
            0..=71 => self.call(cls, 3),
            72..=89 => {
                let k = if cls == 'x' {
                    ['n', 't', 'b'][self.rng.below(3)]
                } else {
                    cls
                };
                self.op_expr(k, 3)
            }
            _ => self.expr(cls, 3),
        };
        Case { f, array: None }
    }
}

/// 연산자 앞뒤에 놓을 때 식을 괄호로 감싼다 (연산자·범위가 섞인 식이 의도와 다르게 읽히지 않도록)
fn wrap(s: &str) -> String {
    let simple = s
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '!' || c == '_' || c == '$' || c == '"')
        && !s.contains('"');
    let call = s.ends_with(')')
        && s.find('(')
            .is_some_and(|i| s[..i].chars().all(|c| c.is_alphanumeric() || c == '.') && matched(s));
    let literal = s.starts_with('"') && s.ends_with('"') && !s[1..s.len() - 1].contains('"');
    if simple || call || literal {
        s.to_string()
    } else {
        format!("({s})")
    }
}

/// 첫 `(` 가 마지막 `)` 와 짝인가
fn matched(s: &str) -> bool {
    let mut d = 0;
    let mut in_str = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '(' if !in_str => d += 1,
            ')' if !in_str => {
                d -= 1;
                if d == 0 && i + 1 != s.len() {
                    return false;
                }
            }
            _ => {}
        }
    }
    d == 0
}

pub fn cases(n: usize, seed: u64) -> Vec<Case> {
    let mut g = Gen::new(seed);
    (0..n).map(|_| g.formula()).collect()
}
