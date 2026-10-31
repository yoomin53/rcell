// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 문법 시험용 무작위 수식 (한셀이 입력을 어떻게 다시 쓰고 계산하는지).
//!
//! `--syntax N --seed S` 는 값이 아니라 글꼴을 흔든다: 숫자 표기(`1e3`, `.5`, `5.`, `007`), 이름 대소문자, 연산자 주변
//! 공백, 참조 꼴(`$a$1`, `A:A`, `1:1`, `b2:a1`, 3차원, 따옴표 시트, 합집합·교집합), 단항 연산자와 `^`·`%`·`&`·비교
//! 연결, 배열 상수. 결과는 한 줄에 수식 하나인 글이다 (`--emit`). `tools/probe/formula-probe.ps1` 로 한셀의 `Formula`
//! 를, `tests/formula_text_oracle.rs` 로 rcell 이 고른 글과 견준다.

use crate::rand::Rng;

const NUMS: &[&str] = &[
    "1",
    "0",
    "0.5",
    ".5",
    "5.",
    "1e3",
    "1E+3",
    "1E-3",
    "001",
    "1.50",
    "123456789012345678",
    "1E-10",
    "1E+15",
    "1E+20",
    "0.1e1",
    "1.5E-7",
    "100000000000000000000",
    "0.00001",
    "0.000001",
    "-1",
    "+1",
    "12345678.9012345678",
    "9.99999999999999999",
    "3.14159265358979323846",
    "1,5",
    "1E400",
    "0x10",
    "1_000",
    "1.2.3",
    "١٢",
];
const STRS: &[&str] = &[
    "\"abc\"",
    "\"a\"\"b\"",
    "\"\"",
    "\"  x  \"",
    "\"한글\"",
    "\"=1\"",
    "\"#N/A\"",
    "\"a b\"",
    "\"\"\"\"",
];
const REFS: &[&str] = &[
    "A1",
    "a1",
    "$A$1",
    "$a1",
    "A$1",
    "A1:B2",
    "b2:a1",
    "a1:b2",
    "$A$1:$B$2",
    "A:A",
    "a:b",
    "$A:$A",
    "1:1",
    "2:1",
    "$1:$2",
    "A1:A1",
    "Data!A1",
    "data!a1:b2",
    "'Data'!A1",
    "'Data'!A1:B2",
    "Data!A:A",
    "Data!1:1",
    "S1:S2!A1",
    "'S1:S2'!A1",
    "'S1:S2'!A1:B2",
    "S2:S1!A1",
    "Data!$A$1",
    "'Data'!$A:$B",
    "'No Sheet'!A1",
    "NoSheet!A1",
    "nm",
    "NM",
    "Nm",
    "nm2",
    "Data!nm",
    "A1:nm",
    "XFD1048576",
    "XFE1",
    "A1048577",
    "A0",
    "R1C1",
    "RC",
    "R[1]C[1]",
    "[1]Data!A1",
    "A1:B2:C3",
    "A1 B1",
    "(A1:B2 B1:C3)",
    "(A1,B2)",
    "(A1:B2,C3:D4)",
    "A1:B2 B1:C3",
    "Data!A1:S1!B2",
    "Table1[Col]",
    "Table1[[#All],[Col]]",
    "Table1[#Headers]",
];
const FUNCS: &[(&str, usize)] = &[
    ("SUM", 3),
    ("sum", 2),
    ("Sum", 2),
    ("IF", 3),
    ("if", 3),
    ("AVERAGE", 2),
    ("max", 3),
    ("MIN", 2),
    ("ABS", 1),
    ("Round", 2),
    ("COUNT", 2),
    ("CONCATENATE", 2),
    ("LEN", 1),
    ("INDEX", 3),
    ("ROW", 1),
    ("COLUMN", 1),
    ("SUMPRODUCT", 2),
    ("AND", 2),
    ("OR", 2),
    ("NOT", 1),
    ("TEXT", 2),
    ("VLOOKUP", 3),
    ("STDEV.S", 2),
    ("_xlfn.STDEV.S", 2),
    ("stdev.s", 2),
    ("COUNTIF", 2),
    ("XLOOKUP", 3),
    ("_xlfn.XLOOKUP", 3),
    ("BETA.DIST", 4),
    ("NOSUCHFN", 2),
    ("합계", 2),
    ("SUM.", 1),
    ("N", 1),
    ("TRUE", 0),
    ("true", 0),
    ("PI", 0),
    ("pi", 0),
    ("NOW", 0),
    ("INDIRECT", 1),
    ("OFFSET", 3),
];
const OPS: &[&str] = &[
    "+", "-", "*", "/", "^", "&", "=", "<>", "<", ">", "<=", ">=", "!=", "==", "=>", "=<", "><",
];

pub struct Syn {
    rng: Rng,
}

impl Syn {
    pub fn new(seed: u64) -> Syn {
        Syn {
            rng: Rng::new(seed ^ 0x5157_4e41),
        }
    }

    fn sp(&mut self) -> &'static str {
        match self.rng.below(10) {
            0 => " ",
            1 => "  ",
            2 => "\t",
            _ => "",
        }
    }

    fn atom(&mut self) -> String {
        match self.rng.below(100) {
            0..=29 => self.rng.pick(NUMS).to_string(),
            30..=39 => self.rng.pick(STRS).to_string(),
            40..=69 => self.rng.pick(REFS).to_string(),
            70..=74 => self
                .rng
                .pick(&[
                    "TRUE",
                    "FALSE",
                    "true",
                    "False",
                    "#N/A",
                    "#n/a",
                    "#REF!",
                    "#DIV/0!",
                    "#NAME?",
                    "#NULL!",
                    "#VALUE!",
                    "#NUM!",
                    "#GETTING_DATA",
                    "#SPILL!",
                    "#CALC!",
                    "#foo!",
                ])
                .to_string(),
            75..=84 => {
                let a = self.rng.pick(&[
                    "{1,2;3,4}",
                    "{1,2,3}",
                    "{\"a\",\"b\"}",
                    "{TRUE,FALSE}",
                    "{1;2}",
                    "{ 1 , 2 }",
                    "{1,2;3}",
                    "{}",
                    "{1,{2}}",
                    "{A1}",
                    "{-1,+2}",
                    "{1,\"a\";#N/A,TRUE}",
                    "{1e3,.5}",
                ]);
                a.to_string()
            }
            _ => self.rng.pick(NUMS).to_string(),
        }
    }

    fn expr(&mut self, depth: u32) -> String {
        if depth == 0 || self.rng.chance(25) {
            let mut s = self.atom();
            if self.rng.chance(12) {
                s.push('%');
            }
            if self.rng.chance(4) {
                s.push('%');
            }
            return s;
        }
        match self.rng.below(100) {
            0..=34 => {
                // 이항 연산
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                let op = self.rng.pick(OPS);
                format!("{a}{}{op}{}{b}", self.sp(), self.sp())
            }
            35..=44 => {
                // 단항
                let u = self
                    .rng
                    .pick(&["-", "+", "--", "-+", "+-", "- ", "+ ", "---"]);
                let e = self.expr(depth - 1);
                format!("{u}{e}")
            }
            45..=52 => {
                // 거듭제곱과 단항
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                let u = self.rng.pick(&["-", "", "+"]);
                format!("{u}{a}^{u}{b}")
            }
            53..=58 => format!("({})", self.expr(depth - 1)),
            59..=62 => {
                // 비교 연결
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                let c = self.expr(depth - 1);
                format!("{a}<{b}<{c}")
            }
            63..=66 => {
                // 연결
                let a = self.expr(depth - 1);
                let b = self.expr(depth - 1);
                let c = self.expr(depth - 1);
                format!("{a}&{b}&{c}")
            }
            _ => {
                let (name, n) = FUNCS[self.rng.below(FUNCS.len())];
                let mut args = Vec::new();
                let k = if n == 0 { 0 } else { 1 + self.rng.below(n) };
                for _ in 0..k {
                    args.push(self.expr(depth - 1));
                }
                if self.rng.chance(8) && !args.is_empty() {
                    args.insert(self.rng.below(args.len()), String::new());
                }
                let sep = if self.rng.chance(15) { " , " } else { "," };
                let open = if self.rng.chance(6) { " (" } else { "(" };
                format!("{name}{open}{}{}{})", self.sp(), args.join(sep), self.sp())
            }
        }
    }

    pub fn formula(&mut self) -> String {
        let e = self.expr(3);
        match self.rng.below(10) {
            0 => format!(" {e}"),
            1 => format!("{e} "),
            2 => format!("+{e}"),
            _ => e,
        }
    }
}

pub fn lines(n: usize, seed: u64) -> Vec<String> {
    let mut g = Syn::new(seed);
    (0..n).map(|_| g.formula()).collect()
}
