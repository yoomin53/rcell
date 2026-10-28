// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 표시 형식 엔진 — 셀 값과 서식 코드로 화면에 보일 문자열을 만든다.
//!
//! 서식 코드 문법은 ECMA-376 Part 1 18.8.31 을 따른다. 한셀의 표시 결과(`Range.Text`)를
//! 정답으로 삼아 맞췄다 (`tools/probe/numfmt-oracle.ps1`, `tests/numfmt_oracle.rs`).
//! 한셀에서 관측한 동작:
//! - `_x`(공백 맞춤)와 `*x`(채우기)는 표시 문자열에 아무것도 넣지 않는다
//! - 반올림 결과가 0 이면 음수 부호를 붙이지 않는다
//! - 조건부 서식에서 조건에 맞지 않아 마지막 구역을 쓰면 음수에 `-` 를 붙인다
//! - `aaa`/`aaaa` 와 `[$-412]` 의 `ddd`/`dddd` 는 한국어 요일, `[$-412]` 의 AM/PM·A/P 는 오전/오후
//! - 표시할 수 없는 날짜(음수, 9999년 이후)는 `#` 로 채운다
//! - 일반 서식은 유효숫자 10자리(1 미만은 9자리), `[DBNum]` 일반 서식은 15자리, `@` 의 숫자도 15자리
//! - `[DBNum1]`–`[DBNum4]`: 한자(六 은 호환 한자 U+F9D1)·갖은자·전각·한글 숫자. 일반 서식과 날짜의 월·일·시·분·초는
//!   자릿값(十二, 壹拾貳, 일십이), 자리 표시자 서식과 연도는 숫자마다
//! - `G/표준` 은 일반 서식이 아니다 (연호 G + 글자 `/표준`)
//! - 조건 구역은 `[<0]` 만 음수 부호를 뺀다, 고정 분모 분수(`?/4`)는 정수부를 버린다, 공학 표기는 가수의 소수를 자른다
//! - `[$-F800]`·`[$-F400]` 은 한국어 시스템 날짜·시간, `[$-411]` 은 일본 연호, `[$-804]` 의 `aaaa` 는 星期
//!
//! 자세한 관측은 docs/format/cell-2024.md 7.11절.

use crate::model::CellValue;
use std::cell::Cell;

thread_local! {
    /// TEXT 함수용: `?` 자리 공백을 이 표시 글자로 넣는다 (TEXT 가 글 앞으로 옮긴다)
    static PAD_MARK: Cell<bool> = const { Cell::new(false) };
    /// 화면 배치용: `_x`(x 폭만큼 빈 자리)와 `*x`(채움)를 표시 글자로 남긴다 ([`format_layout`])
    static LAYOUT_MARK: Cell<bool> = const { Cell::new(false) };
}

/// `_x` 표시 글자 (사용자 정의 영역, 다음 글자가 x)
const SPACE_MARK: char = '\u{E0FD}';
/// `*x` 표시 글자 (사용자 정의 영역, 다음 글자가 x)
const FILL_MARK: char = '\u{E0FE}';

/// [`format`] 과 같되 `_x` 빈 자리와 `*x` 채움의 자리를 [`Formatted::pads`]·[`Formatted::fill`] 에 담는다
/// (화면에 그릴 때 — 회계 서식의 오른쪽 여백, 채움 글자). 글은 [`format`] 과 같다
pub fn format_layout(value: &CellValue, code: &str, date1904: bool) -> Formatted {
    LAYOUT_MARK.with(|p| p.set(true));
    let mut out = format(value, code, date1904);
    LAYOUT_MARK.with(|p| p.set(false));
    if out.text.contains([SPACE_MARK, FILL_MARK]) {
        let mut text = String::with_capacity(out.text.len());
        let mut it = out.text.chars();
        while let Some(c) = it.next() {
            match c {
                SPACE_MARK | FILL_MARK => {
                    let Some(x) = it.next() else { break };
                    if c == SPACE_MARK {
                        out.pads.push((text.len(), x));
                    } else if out.fill.is_none() {
                        out.fill = Some((text.len(), x));
                    }
                }
                c => text.push(c),
            }
        }
        out.text = text;
    }
    out
}

/// TEXT 함수가 `?` 자리의 공백을 알아볼 수 있게 넣는 표시 글자 (사용자 정의 영역)
pub const PAD_CHAR: char = '\u{E0FF}';

fn pad_str() -> &'static str {
    if PAD_MARK.with(|p| p.get()) {
        "\u{E0FF}"
    } else {
        " "
    }
}

/// [`format`] 과 같되 `?` 자리 공백을 [`PAD_CHAR`] 로, 정수 값의 분수 자리도 그 글자로 채운다
/// (한셀의 TEXT 는 이 공백을 글 앞으로 옮긴다: TEXT(5,"# ?/?") = "   5 ", 대조 실험 misc4)
pub fn format_padded(value: &CellValue, code: &str, date1904: bool) -> Formatted {
    PAD_MARK.with(|p| p.set(true));
    let out = format(value, code, date1904);
    PAD_MARK.with(|p| p.set(false));
    out
}

/// 표시 결과.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Formatted {
    pub text: String,
    /// 서식 코드의 색 (`[Red]` → "Red")
    pub color: Option<String>,
    /// 날짜가 범위를 벗어나 `#` 로 채워야 하는 경우
    pub overflow: bool,
    /// `_x` 자리: (글의 바이트 위치, x) — x 의 폭만큼 빈 자리 ([`format_layout`] 만 채운다)
    pub pads: Vec<(usize, char)>,
    /// `*x` 채움: (글의 바이트 위치, x) — 칸의 남는 폭을 x 로 채운다 ([`format_layout`] 만 채운다)
    pub fill: Option<(usize, char)>,
    /// 숫자를 글 구역(`@`)으로 표시했다 — 한셀은 일반 맞춤에서 글처럼 왼쪽에 둔다 (표시 형식 실험 `[Magenta]@`)
    pub as_text: bool,
}

/// 한셀(한국어) 내장 표시 형식. 5–8, 23–26, 41–44 는 한셀 레거시 파일의 FORMAT 레코드,
/// 14/37/38 등은 한셀 표시 결과에서 확인했다.
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"₩\"#,##0;\\-\"₩\"#,##0",
        6 => "\"₩\"#,##0;[Red]\\-\"₩\"#,##0",
        7 => "\"₩\"#,##0.00;\\-\"₩\"#,##0.00",
        8 => "\"₩\"#,##0.00;[Red]\\-\"₩\"#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "yyyy-mm-dd",
        15 => "d-mmm-yy",
        // 한셀은 16 을 dd-mmm 으로 그린다 (5일 → 05-Jan, 입력 대조 실험)
        16 => "dd-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "yyyy-mm-dd h:mm",
        23 => "\\$#,##0_);\\(\\$#,##0\\)",
        24 => "\\$#,##0_);[Red]\\(\\$#,##0\\)",
        25 => "\\$#,##0.00_);\\(\\$#,##0.00\\)",
        26 => "\\$#,##0.00_);[Red]\\(\\$#,##0.00\\)",
        // 한셀이 `2024년 1월 15일` 입력에 붙이는 형식 (입력 대조 실험)
        31 => "yyyy\"년\" mm\"월\" dd\"일\"",
        37 => "#,##0;-#,##0",
        38 => "#,##0;[Red]-#,##0",
        39 => "#,##0.00;-#,##0.00",
        40 => "#,##0.00;[Red]-#,##0.00",
        41 => "_-* #,##0_-;\\-* #,##0_-;_-* \"-\"_-;_-@_-",
        42 => "_-\"₩\"* #,##0_-;\\-\"₩\"* #,##0_-;_-\"₩\"* \"-\"_-;_-@_-",
        43 => "_-* #,##0.00_-;\\-* #,##0.00_-;_-* \"-\"??_-;_-@_-",
        44 => "_-\"₩\"* #,##0.00_-;\\-\"₩\"* #,##0.00_-;_-\"₩\"* \"-\"??_-;_-@_-",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        // 한셀이 `1/15`·`1-2` (올해 날짜) 입력에 붙이는 형식
        58 => "mm-dd",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// 토큰
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(String),
    /// '0' '#' '?'
    Digit(char),
    Dot,
    Comma,
    Percent,
    /// E+ (true) / E- (false)
    Exp(bool),
    Slash,
    At,
    General,
    /// 날짜/시간: 문자와 개수 (y, m, d, h, s, a(한국어 요일), g(연호), e)
    Dt(char, usize),
    /// 분 (m 이 시간 문맥일 때 해석 후 바뀜)
    Minute(usize),
    /// 경과 시간 [h] [m] [s]
    Elapsed(char, usize),
    /// AM/PM 표기 (오전, 오후)
    AmPm(String, String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Default)]
struct Section {
    toks: Vec<Tok>,
    color: Option<String>,
    cond: Option<(Op, f64)>,
    korean: bool,
    /// `[$-xxx]` 의 언어 번호 (16진, 예 0x411 일본어, 0xF800 시스템 긴 날짜)
    locale: u32,
    /// 시스템 시간([$-F400])의 초를 버린다: 원래 구역에 초가 없을 때 (있으면 반올림)
    sys_trunc: bool,
    /// `[DBNum1]`–`[DBNum4]`: 한자·갖은자·전각·한글 숫자 (이 구역만)
    dbnum: u8,
}

impl Section {
    fn is_date(&self) -> bool {
        self.toks.iter().any(|t| {
            matches!(
                t,
                Tok::Dt(..) | Tok::Minute(_) | Tok::Elapsed(..) | Tok::AmPm(..)
            )
        })
    }
    fn has_digits(&self) -> bool {
        self.toks.iter().any(|t| matches!(t, Tok::Digit(_)))
    }
    fn has_at(&self) -> bool {
        self.toks.iter().any(|t| matches!(t, Tok::At))
    }
    fn is_general(&self) -> bool {
        self.toks.iter().any(|t| matches!(t, Tok::General))
    }
}

const COLORS: [&str; 8] = [
    "Black", "Blue", "Cyan", "Green", "Magenta", "Red", "White", "Yellow",
];

fn split_sections(code: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.last_mut().unwrap().push(c);
                for d in chars.by_ref() {
                    out.last_mut().unwrap().push(d);
                    if d == '"' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                out.last_mut().unwrap().push(c);
                if let Some(d) = chars.next() {
                    out.last_mut().unwrap().push(d);
                }
            }
            '[' => {
                out.last_mut().unwrap().push(c);
                for d in chars.by_ref() {
                    out.last_mut().unwrap().push(d);
                    if d == ']' {
                        break;
                    }
                }
            }
            ';' => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out
}

fn parse_cond(s: &str) -> Option<(Op, f64)> {
    let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
        (Op::Ge, r)
    } else if let Some(r) = s.strip_prefix("<=") {
        (Op::Le, r)
    } else if let Some(r) = s.strip_prefix("<>") {
        (Op::Ne, r)
    } else if let Some(r) = s.strip_prefix('>') {
        (Op::Gt, r)
    } else if let Some(r) = s.strip_prefix('<') {
        (Op::Lt, r)
    } else {
        (Op::Eq, s.strip_prefix('=')?)
    };
    Some((op, rest.trim().parse().ok()?))
}

fn parse_section(src: &str) -> Section {
    let mut sec = Section::default();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let lower = |c: char| c.to_ascii_lowercase();
    let push_lit = |toks: &mut Vec<Tok>, s: &str| {
        if let Some(Tok::Lit(prev)) = toks.last_mut() {
            prev.push_str(s);
        } else {
            toks.push(Tok::Lit(s.to_string()));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let rest: String = chars[i..].iter().collect();
        let rest_l = rest.to_ascii_lowercase();
        match c {
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                i += 1;
                push_lit(&mut sec.toks, &s);
                continue;
            }
            '\\' => {
                if let Some(&d) = chars.get(i + 1) {
                    push_lit(&mut sec.toks, &d.to_string());
                }
                i += 2;
                continue;
            }
            '_' | '*' => {
                if LAYOUT_MARK.with(|p| p.get())
                    && let Some(&d) = chars.get(i + 1)
                {
                    let m = if c == '_' { SPACE_MARK } else { FILL_MARK };
                    push_lit(&mut sec.toks, &format!("{m}{d}"));
                }
                i += 2;
                continue;
            }
            '[' => {
                let end = chars[i..].iter().position(|&d| d == ']').map(|p| i + p);
                let Some(end) = end else {
                    i += 1;
                    continue;
                };
                let inner: String = chars[i + 1..end].iter().collect();
                let il = inner.to_ascii_lowercase();
                if let Some(col) = COLORS.iter().find(|c| c.eq_ignore_ascii_case(&inner)) {
                    sec.color = Some(col.to_string());
                } else if il.starts_with("color") {
                    sec.color = Some(inner.clone());
                } else if let Some(cond) = parse_cond(&inner) {
                    sec.cond = Some(cond);
                } else if !il.is_empty() && il.chars().all(|c| c == 'h') {
                    sec.toks.push(Tok::Elapsed('h', il.len()));
                } else if !il.is_empty() && il.chars().all(|c| c == 'm') {
                    sec.toks.push(Tok::Elapsed('m', il.len()));
                } else if !il.is_empty() && il.chars().all(|c| c == 's') {
                    sec.toks.push(Tok::Elapsed('s', il.len()));
                } else if let Some(k) = il.strip_prefix("dbnum") {
                    sec.dbnum = k.parse().ok().filter(|k| (1..=4).contains(k)).unwrap_or(0);
                } else if let Some(cur) = inner.strip_prefix('$') {
                    let (sym, loc) = cur.split_once('-').unwrap_or((cur, ""));
                    if !sym.is_empty() {
                        push_lit(&mut sec.toks, sym);
                    }
                    sec.locale = u32::from_str_radix(loc.trim(), 16).unwrap_or(0);
                    if sec.locale == 0x412 {
                        sec.korean = true;
                    }
                }
                i = end + 1;
                continue;
            }
            _ => {}
        }
        if rest_l.starts_with("general") {
            sec.toks.push(Tok::General);
            i += 7;
            continue;
        }
        if rest_l.starts_with("am/pm") {
            sec.toks.push(Tok::AmPm("AM".into(), "PM".into()));
            i += 5;
            continue;
        }
        if rest_l.starts_with("a/p") {
            sec.toks.push(Tok::AmPm("A".into(), "P".into()));
            i += 3;
            continue;
        }
        if rest.starts_with("오전/오후") {
            sec.toks.push(Tok::AmPm("오전".into(), "오후".into()));
            i += 5;
            continue;
        }
        if (c == 'E' || c == 'e') && matches!(chars.get(i + 1), Some('+') | Some('-')) {
            sec.toks.push(Tok::Exp(chars[i + 1] == '+'));
            i += 2;
            continue;
        }
        match lower(c) {
            'y' | 'm' | 'd' | 'h' | 's' | 'g' | 'e' | 'a' | 'b' => {
                let lc = lower(c);
                let mut n = 0;
                while i + n < chars.len() && lower(chars[i + n]) == lc {
                    n += 1;
                }
                sec.toks.push(Tok::Dt(lc, n));
                i += n;
                continue;
            }
            _ => {}
        }
        match c {
            '0' | '#' | '?' => sec.toks.push(Tok::Digit(c)),
            '.' => sec.toks.push(Tok::Dot),
            ',' => sec.toks.push(Tok::Comma),
            '%' => sec.toks.push(Tok::Percent),
            '@' => sec.toks.push(Tok::At),
            '/' => sec.toks.push(Tok::Slash),
            'E' | 'e' if matches!(chars.get(i + 1), Some('+') | Some('-')) => {
                sec.toks.push(Tok::Exp(chars[i + 1] == '+'));
                i += 2;
                continue;
            }
            other => push_lit(&mut sec.toks, &other.to_string()),
        }
        i += 1;
    }
    resolve_minutes(&mut sec.toks);
    // 시스템 날짜·시간 ([$-F800] 긴 날짜, [$-F400] 시간): 한셀은 구역의 나머지와 상관없이 한국어 꼴로 쓴다
    // (`2026년 10월 19일 월요일`, `오후 1:37:38` — 표시 형식 대조 실험)
    let system = match sec.locale {
        0xF800 => Some("yyyy\"년\" m\"월\" d\"일\" dddd"),
        0xF400 => Some("AM/PM h:mm:ss"),
        _ => None,
    };
    if let Some(code) = system {
        sec.sys_trunc = !sec.toks.iter().any(|t| matches!(t, Tok::Dt('s', _)));
        let s = parse_section(code);
        sec.toks = s.toks;
        sec.korean = true;
    }
    // 날짜 서식에서 '/' 는 글자 그대로다
    if sec.is_date() {
        for t in sec.toks.iter_mut() {
            if *t == Tok::Slash {
                *t = Tok::Lit("/".into());
            }
        }
    }
    // [DBNum] 구역의 글자(따옴표 안 숫자 포함)는 숫자를 바꾸지 않는다: 표지로 감싸 두고 마지막에 뗀다
    if sec.dbnum != 0 {
        for t in sec.toks.iter_mut() {
            if let Tok::Lit(l) = t {
                *l = format!("{LIT_OPEN}{l}{LIT_CLOSE}");
            }
        }
    }
    sec
}

/// [DBNum] 구역에서 바꾸지 않을 글자의 표지 (사용자 영역 문자, 출력 전에 뗀다)
const LIT_OPEN: char = '\u{E000}';
const LIT_CLOSE: char = '\u{E001}';

/// `[DBNumN]` 의 숫자 0–9 (한셀 표시 결과, 표시 형식 대조 실험): 1 한자 (0 은 전각 ０), 2 갖은자 (四·六·七·八·九 는
/// 보통 한자), 3 전각 아라비아 숫자, 4 한글. 한셀은 六·零·參 을 한자 호환 영역 글자(KS X 1001 의 따로 읽는 한자)로
/// 쓴다: 六 U+F9D1, 零 U+F9B2, 參 U+F96B (DBNum2 의 拾 도 U+F973)
fn dbnum_digit(kind: u8, d: u32) -> char {
    const D: [[char; 10]; 4] = [
        ['０', '一', '二', '三', '四', '五', '六', '七', '八', '九'],
        ['零', '壹', '貳', '參', '四', '伍', '六', '七', '八', '九'],
        ['０', '１', '２', '３', '４', '５', '６', '７', '８', '９'],
        ['영', '일', '이', '삼', '사', '오', '육', '칠', '팔', '구'],
    ];
    D[(kind.clamp(1, 4) - 1) as usize][d.min(9) as usize]
}

/// 정수의 자릿값 읽기 — `[DBNum]` 일반 서식의 정수부와 날짜의 월·일·시·분·초: 네 자리마다 만·억·조, 그 안은
/// 천·백·십. 0 인 자리는 건너뛰고, DBNum1·3 은 십·백·천 앞의 1 을 쓰지 않는다 (`12` → 十二, `10000` → 一万,
/// `11000` → 一万千), DBNum2·4 는 쓴다 (壹拾貳, 일만일천)
fn dbnum_positional(kind: u8, n: u64) -> String {
    if n == 0 {
        return dbnum_digit(kind, 0).to_string();
    }
    let (units, groups): ([&str; 3], [&str; 4]) = match kind {
        2 => (["拾", "百", "阡"], ["萬", "億", "兆", "京"]),
        4 => (["십", "백", "천"], ["만", "억", "조", "경"]),
        _ => (["十", "百", "千"], ["万", "億", "兆", "京"]),
    };
    let omit_one = matches!(kind, 1 | 3);
    let mut gs = Vec::new();
    let mut m = n;
    while m > 0 {
        gs.push(m % 10_000);
        m /= 10_000;
    }
    let mut s = String::new();
    for (gi, &g) in gs.iter().enumerate().rev() {
        if g == 0 {
            continue;
        }
        for pos in (0..4u32).rev() {
            let d = (g / 10u64.pow(pos)) % 10;
            if d == 0 {
                continue;
            }
            if pos == 0 || !(omit_one && d == 1) {
                s.push(dbnum_digit(kind, d as u32));
            }
            if pos > 0 {
                s.push_str(units[pos as usize - 1]);
            }
        }
        if gi > 0 {
            s.push_str(groups.get(gi - 1).copied().unwrap_or(""));
        }
    }
    s
}

/// `[DBNum]` 일반 서식: 정수부는 자릿값 읽기, 소수점 뒤는 지수까지 글자마다 바꾼다. 소수점이 없으면 지수는 아라비아
/// 숫자 그대로다 (한셀: `1.234E-6` → 一.二三四E-六, `1.5E+20` → 一.五E+二０, `1E+15` → 一E+15, `9E-5` → 九E-5)
fn dbnum_general(kind: u8, s: &str) -> String {
    let n = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (int, rest) = s.split_at(n);
    let mut out = dbnum_positional(kind, int.parse().unwrap_or(0));
    if rest.starts_with('.') {
        out.push_str(&dbnum_each(kind, rest));
    } else {
        out.push_str(rest);
    }
    out
}

/// 숫자 글자마다 바꾸기 (연도, DBNum3 의 날짜)
fn dbnum_each(kind: u8, s: &str) -> String {
    s.chars()
        .map(|c| match c.to_digit(10) {
            Some(d) if c.is_ascii_digit() => dbnum_digit(kind, d),
            _ => c,
        })
        .collect()
}

/// 표시 문자열 마무리: 표지 밖의 아라비아 숫자를 `[DBNum]` 숫자로 바꾸고 표지를 뗀다 (`kind` 0 이면 표지만)
fn dbnum_finish(s: &str, kind: u8) -> String {
    let mut out = String::with_capacity(s.len());
    let mut lit = false;
    for c in s.chars() {
        match c {
            LIT_OPEN => lit = true,
            LIT_CLOSE => lit = false,
            c if kind != 0 && !lit && c.is_ascii_digit() => {
                out.push(dbnum_digit(kind, c as u32 - '0' as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// `m` 이 시(h) 뒤나 초(s) 앞에 오면 분이다.
fn resolve_minutes(toks: &mut [Tok]) {
    let n = toks.len();
    for i in 0..n {
        if let Tok::Dt('m', k) = toks[i]
            && k <= 2
        {
            let after_h = toks[..i]
                .iter()
                .rev()
                .find(|t| matches!(t, Tok::Dt(..) | Tok::Elapsed(..)))
                .is_some_and(|t| matches!(t, Tok::Dt('h', _) | Tok::Elapsed('h', _)));
            let before_s = toks[i + 1..]
                .iter()
                .find(|t| matches!(t, Tok::Dt(..) | Tok::Elapsed(..)))
                .is_some_and(|t| matches!(t, Tok::Dt('s', _) | Tok::Elapsed('s', _)));
            if after_h || before_s {
                toks[i] = Tok::Minute(k);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 숫자
// ---------------------------------------------------------------------------

/// 유효숫자 15자리로 맞춘 뒤 소수 `d` 자리에서 반올림(0.5 는 0 에서 멀어지게)한
/// 십진 문자열 (정수부, 소수부).
fn round_decimal(x: f64, d: usize) -> (String, String) {
    let x = x.abs();
    if x == 0.0 || !x.is_finite() {
        return ("0".into(), "0".repeat(d));
    }
    let sci = format!("{x:.14e}");
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: Vec<u8> = mant
        .bytes()
        .filter(|b| b.is_ascii_digit())
        .map(|b| b - b'0')
        .collect();
    // 숫자열 digits 는 10^exp 자리부터 시작한다
    let point = exp + 1; // 정수부 자릿수
    let keep = point + d as i32; // 남길 자릿수
    let mut kept: Vec<u8> = if keep <= 0 {
        Vec::new()
    } else {
        digits.iter().copied().take(keep as usize).collect()
    };
    while (kept.len() as i32) < keep {
        kept.push(0);
    }
    let round_up = if keep < 0 {
        false
    } else {
        digits.get(keep as usize).is_some_and(|&g| g >= 5)
    };
    let mut point = point;
    if round_up {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, 1);
                point += 1;
                break;
            }
            i -= 1;
            if kept[i] == 9 {
                kept[i] = 0;
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let to_s = |v: &[u8]| v.iter().map(|d| (b'0' + d) as char).collect::<String>();
    if point <= 0 {
        let lead = (-point) as usize;
        let mut frac = "0".repeat(lead.min(d));
        frac.push_str(&to_s(&kept));
        frac.truncate(d);
        while frac.len() < d {
            frac.push('0');
        }
        ("0".into(), frac)
    } else {
        let p = point as usize;
        let int = to_s(&kept[..p.min(kept.len())]);
        let frac = if kept.len() > p {
            to_s(&kept[p..])
        } else {
            String::new()
        };
        let int = int.trim_start_matches('0');
        (
            if int.is_empty() {
                "0".into()
            } else {
                int.to_string()
            },
            frac,
        )
    }
}

fn group_thousands(int: &str) -> String {
    let b = int.as_bytes();
    let mut out = String::new();
    for (i, &c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c as char);
    }
    out
}

/// 일반 서식 — 한셀 표시 결과 (표시 형식 대조 실험): 1 이상은 유효숫자 10자리, 1 보다 작으면 9자리
/// (`1234567.891`, `3.141592654`, `0.123456789`, `0.000123456789`), 정수부가 11자리를 넘으면 지수 (가수 유효숫자
/// 6자리, `1.23457E+11`). 0.0001 보다 작은 수는 소수 7자리 안에 들면 그대로 (`0.0000123`, `0.0000001`) 아니면 지수
/// (`1.234E-5` — 음의 지수는 0 을 채우지 않는다)
pub fn general(v: f64) -> String {
    general_with(v, 10, 9, 1e11, 5, 7)
}

/// `[DBNum]` 의 일반 서식: 유효숫자 15자리, 1E+15 부터 지수 (한셀: `12345678901234` 을 자릿값으로 다 읽고,
/// `45663.0347222222` 의 소수를 다 쓴다), 0.0001 보다 작으면 늘 지수 (`9E-5`, `1.23E-5`)
fn general_dbnum(v: f64) -> String {
    general_with(v, 15, 15, 1e15, 14, 0)
}

/// 텍스트 서식(`@`)의 숫자: 유효숫자 15자리, 1E-9 보다 작거나 1E+15 이상이면 지수 (한셀: `0.000000001`,
/// `123456789012`, `3.14159265358979`, `1E-20`)
fn general_text(v: f64) -> String {
    let a = v.abs();
    if a != 0.0 && !(1e-9..1e15).contains(&a) {
        let s = format!("{v:.14E}");
        let (m, e) = s.split_once('E').unwrap();
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = e.parse().unwrap_or(0);
        return format!("{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    crate::model::format_general(v)
}

/// 일반 서식의 틀: 유효숫자 `sig_big`(1 이상)·`sig_small`(1 미만), `sci_from` 부터 지수, 지수의 가수 소수 자릿수,
/// 0.0001 보다 작은 수를 그대로 쓸 소수 자릿수의 한도
fn general_with(
    v: f64,
    sig_big: i32,
    sig_small: i32,
    sci_from: f64,
    mant: usize,
    small_fixed: usize,
) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if !v.is_finite() {
        return v.to_string();
    }
    let a = v.abs();
    let sign = if v < 0.0 { "-" } else { "" };
    let trim = |s: String| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    let sci = |a: f64| -> String {
        let s = format!("{a:.mant$e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = trim(m.to_string());
        let e: i32 = e.parse().unwrap_or(0);
        if e < 0 {
            format!("{m}E-{}", -e)
        } else {
            format!("{m}E+{e:02}")
        }
    };
    if a.round() >= sci_from {
        return format!("{sign}{}", sci(a));
    }
    let lead = a.log10().floor() as i32;
    if a < 1e-4 {
        let decimals = (-lead - 1 + sig_small).max(0) as usize;
        let fixed = trim(format!("{a:.decimals$}"));
        let ndec = fixed.split_once('.').map_or(0, |(_, f)| f.len());
        return if ndec <= small_fixed {
            format!("{sign}{fixed}")
        } else {
            format!("{sign}{}", sci(a))
        };
    }
    let decimals = if a >= 1.0 {
        (sig_big - lead - 1).max(0)
    } else {
        sig_small - lead - 1
    } as usize;
    format!("{sign}{}", trim(format!("{a:.decimals$}")))
}

fn fill_int(toks: &[&Tok], digits: &str, thousands: bool) -> String {
    let placeholders = toks.iter().filter(|t| matches!(t, Tok::Digit(_))).count();
    let contiguous = toks.iter().all(|t| matches!(t, Tok::Digit(_) | Tok::Comma));
    let mut ds: Vec<char> = if digits == "0" {
        Vec::new()
    } else {
        digits.chars().collect()
    };
    if contiguous {
        // 가장 흔한 꼴 (#,##0): 최소 자릿수만큼 채운 뒤 천 단위 구분
        let zeros = toks
            .iter()
            .filter(|t| matches!(t, Tok::Digit(c) if *c == '0'))
            .count();
        let qs = toks
            .iter()
            .filter(|t| matches!(t, Tok::Digit(c) if *c == '?'))
            .count();
        while ds.len() < zeros {
            ds.insert(0, '0');
        }
        let core: String = ds.iter().collect();
        let mut s = if thousands && !core.is_empty() {
            group_thousands(&core)
        } else {
            core
        };
        let width = zeros + qs;
        while s.chars().count() < width {
            s.insert_str(0, pad_str());
        }
        return s;
    }
    // 자리 표시자 사이에 글자가 섞인 꼴 (000-0000): 오른쪽부터 채운다
    let mut out: Vec<String> = Vec::new();
    let mut used = 0;
    for (k, t) in toks.iter().enumerate().rev() {
        match t {
            Tok::Digit(c) => {
                used += 1;
                let mut s = match ds.pop() {
                    Some(d) => d.to_string(),
                    None => match c {
                        '0' => "0".into(),
                        '?' => pad_str().into(),
                        _ => String::new(),
                    },
                };
                if used == placeholders {
                    // 남은 앞자리 숫자는 첫 자리 표시자에 몰아 넣는다
                    let lead: String = ds.drain(..).collect();
                    s = lead + &s;
                }
                out.push(s);
                let _ = k;
            }
            Tok::Lit(l) => out.push(l.clone()),
            _ => {}
        }
    }
    out.reverse();
    out.concat()
}

fn fill_frac(toks: &[&Tok], frac: &str) -> String {
    let mut out = String::new();
    let digits: Vec<char> = frac.chars().collect();
    // 끝에서부터 '#' 은 0 을 지우고, '?' 는 공백으로
    let ph: Vec<char> = toks
        .iter()
        .filter_map(|t| match t {
            Tok::Digit(c) => Some(*c),
            _ => None,
        })
        .collect();
    let mut shown: Vec<String> = digits.iter().map(|d| d.to_string()).collect();
    for k in (0..shown.len()).rev() {
        if shown[k] != "0" {
            break;
        }
        match ph.get(k) {
            Some('#') => shown[k] = String::new(),
            Some('?') => shown[k] = pad_str().into(),
            _ => break,
        }
    }
    let mut di = 0;
    for t in toks {
        match t {
            Tok::Digit(_) => {
                if let Some(s) = shown.get(di) {
                    out.push_str(s);
                }
                di += 1;
            }
            Tok::Lit(l) => out.push_str(l),
            _ => {}
        }
    }
    out
}

fn format_number(sec: &Section, v: f64, signed: bool) -> String {
    let toks = &sec.toks;
    if toks.iter().any(|t| matches!(t, Tok::Exp(_))) {
        return format_exp(sec, v, signed);
    }
    // 분수: 분모가 자리 표시자이거나 고정된 수 (`# ?/8`)
    if let Some(si) = toks.iter().position(|t| *t == Tok::Slash)
        && toks[..si].iter().any(|t| matches!(t, Tok::Digit(_)))
        && (toks[si + 1..].iter().any(|t| matches!(t, Tok::Digit(_)))
            || matches!(toks.get(si + 1), Some(Tok::Lit(l))
                if l.trim_matches(|c| c == LIT_OPEN || c == LIT_CLOSE).trim().parse::<u64>().is_ok()))
    {
        return format_fraction(sec, v, si, signed);
    }

    let pct = toks.iter().filter(|t| **t == Tok::Percent).count() as i32;
    // 숫자부 끝 쉼표 = 1000 으로 나누기
    let last_digit = toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let dot = toks.iter().position(|t| *t == Tok::Dot);
    let int_end = dot.or(last_digit.map(|p| p + 1)).unwrap_or(toks.len());
    // 마지막 자리 표시자 바로 뒤의 쉼표 하나마다 1000 으로 나눈다 (#,##0.0,)
    let mut scale = 0;
    if let Some(ld) = last_digit {
        let mut k = ld + 1;
        while k < toks.len() && toks[k] == Tok::Comma {
            scale += 1;
            k += 1;
        }
    }
    let first_digit = toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let is_digit = |t: &Tok| matches!(t, Tok::Digit(_));
    let thousands = first_digit.filter(|&a| a < int_end).is_some_and(|a| {
        toks[a..int_end]
            .iter()
            .enumerate()
            .any(|(k, t)| *t == Tok::Comma && toks[a + k + 1..int_end].iter().any(is_digit))
    });
    let decimals = dot
        .map(|d| {
            toks[d + 1..]
                .iter()
                .filter(|t| matches!(t, Tok::Digit(_)))
                .count()
        })
        .unwrap_or(0);

    let x = v.abs() * 100f64.powi(pct) / 1000f64.powi(scale);
    let (int, frac) = round_decimal(x, decimals);
    let nonzero = int != "0" || frac.chars().any(|c| c != '0');

    let mut out = String::new();
    if signed && v < 0.0 && nonzero {
        out.push('-');
    }
    // 숫자 앞 글자
    let fd = first_digit.unwrap_or(toks.len());
    let int_toks: Vec<&Tok> = toks[fd.min(int_end)..int_end]
        .iter()
        .filter(|t| !matches!(t, Tok::Comma) || thousands)
        .collect();
    for t in &toks[..fd.min(int_end)] {
        push_simple(&mut out, t, v);
    }
    let int_toks: Vec<&Tok> = int_toks.into_iter().filter(|t| **t != Tok::Comma).collect();
    // 정수 자리 표시자가 없으면(.00) 한셀은 정수부를 표시하지 않는다
    if !int_toks.is_empty() {
        out.push_str(&fill_int(&int_toks, &int, thousands));
    }
    if let Some(d) = dot {
        out.push('.');
        let frac_toks: Vec<&Tok> = toks[d + 1..]
            .iter()
            .take_while(|t| matches!(t, Tok::Digit(_) | Tok::Lit(_)))
            .collect();
        let consumed = frac_toks.len();
        out.push_str(&fill_frac(&frac_toks, &frac));
        for t in &toks[d + 1 + consumed..] {
            push_simple(&mut out, t, v);
        }
    } else {
        let after = last_digit.map(|p| p + 1).unwrap_or(toks.len());
        for t in &toks[after..] {
            push_simple(&mut out, t, v);
        }
    }
    out
}

/// 숫자 자리 밖의 단순 토큰.
fn push_simple(out: &mut String, t: &Tok, v: f64) {
    match t {
        Tok::Lit(l) => out.push_str(l),
        Tok::Percent => out.push('%'),
        Tok::At => out.push_str(&general_text(v)),
        Tok::General => out.push_str(&general(v.abs())),
        Tok::Slash => out.push('/'),
        _ => {}
    }
}

fn format_exp(sec: &Section, v: f64, signed: bool) -> String {
    let toks = &sec.toks;
    let ei = toks.iter().position(|t| matches!(t, Tok::Exp(_))).unwrap();
    let plus = matches!(toks[ei], Tok::Exp(true));
    let mant = &toks[..ei];
    let dot = mant.iter().position(|t| *t == Tok::Dot);
    let int_ph = mant[..dot.unwrap_or(mant.len())]
        .iter()
        .filter(|t| matches!(t, Tok::Digit(_)))
        .count()
        .max(1);
    let decimals = dot
        .map(|d| {
            mant[d + 1..]
                .iter()
                .filter(|t| matches!(t, Tok::Digit(_)))
                .count()
        })
        .unwrap_or(0);
    let exp_digits = toks[ei + 1..]
        .iter()
        .filter(|t| matches!(t, Tok::Digit(_)))
        .count();
    let engineering = int_ph > 1
        && mant
            .iter()
            .any(|t| matches!(t, Tok::Digit('#')) || matches!(t, Tok::Digit('?')));
    // 한셀: 공학 표기 패턴(`##0.0E+0`)은 값을 (정수 자리 표시자 + 소수 자리) 유효숫자로 먼저 반올림하고, 가수의 소수는
    // 자른다 (1999 → 1.9E+3, 32874.25 → 32.8E+3, 987654 → 987.7E+3, 123.45 → 123.5E+0 — 표시 형식 대조 실험)
    let a = v.abs();
    let a = if engineering && a != 0.0 {
        let p = a.log10().floor() as i32 - (int_ph + decimals) as i32 + 1;
        (a / 10f64.powi(p)).round() * 10f64.powi(p)
    } else {
        a
    };
    let mut e = if a == 0.0 {
        0
    } else {
        a.log10().floor() as i32
    };
    // 한셀: 공학 표기(##0.0E+0)에서 지수가 음수면 3의 배수로 맞추지 않고
    // 정수부 한 자리로 쓴다 (0.5 → 5.0E-1, 0.001234 → 1.2E-3)
    let eng_pattern = engineering;
    let engineering = engineering && e >= 0;
    if engineering {
        e = e.div_euclid(int_ph as i32) * int_ph as i32;
    } else if a != 0.0 && !eng_pattern {
        e -= int_ph as i32 - 1;
    }
    let mut m = if a == 0.0 { 0.0 } else { a / 10f64.powi(e) };
    let cut = |m: f64| {
        if eng_pattern {
            let k = 10f64.powi(decimals as i32);
            round_decimal(((m * k) + 1e-7).floor() / k, decimals)
        } else {
            round_decimal(m, decimals)
        }
    };
    let (mut int, mut frac) = cut(m);
    // 반올림으로 자리가 넘친 경우
    // 정수부 자릿수: 공학 표기 패턴인데 음수 지수면 한 자리, 아니면 자리 표시자 수
    let int_digits = if eng_pattern && !engineering {
        1
    } else {
        int_ph as i32
    };
    if a != 0.0 && int.parse::<f64>().unwrap_or(0.0) >= 10f64.powi(int_digits) {
        e += if engineering { int_ph as i32 } else { 1 };
        m = a / 10f64.powi(e);
        (int, frac) = cut(m);
    }
    let mut out = String::new();
    if signed && v < 0.0 {
        out.push('-');
    }
    out.push_str(&int);
    if dot.is_some() {
        out.push('.');
        out.push_str(&frac);
    }
    out.push('E');
    if e < 0 {
        out.push('-');
    } else if plus {
        out.push('+');
    }
    out.push_str(&format!("{:0width$}", e.abs(), width = exp_digits.max(1)));
    for t in toks[ei + 1..]
        .iter()
        .skip_while(|t| matches!(t, Tok::Digit(_)))
    {
        push_simple(&mut out, t, v);
    }
    out
}

fn best_fraction(x: f64, max_den: u64) -> (u64, u64) {
    let mut best = (0, 1);
    let mut err = f64::MAX;
    for d in 1..=max_den {
        let n = (x * d as f64).round() as u64;
        let e = (x - n as f64 / d as f64).abs();
        if e < err - 1e-12 {
            err = e;
            best = (n, d);
        }
    }
    best
}

fn format_fraction(sec: &Section, v: f64, si: usize, signed: bool) -> String {
    let toks = &sec.toks;
    // 분자 자리 시작: 슬래시 앞쪽의 연속된 자리 표시자
    let mut ns = si;
    while ns > 0 && matches!(toks[ns - 1], Tok::Digit(_)) {
        ns -= 1;
    }
    let int_toks: Vec<&Tok> = toks[..ns].iter().collect();
    let has_int = int_toks.iter().any(|t| matches!(t, Tok::Digit(_)));
    let den_toks: Vec<&Tok> = toks[si + 1..]
        .iter()
        .take_while(|t| matches!(t, Tok::Digit(_)))
        .collect();
    let den_len = den_toks.len();
    let fixed_den: Option<u64> = toks[si + 1..]
        .iter()
        .take_while(|t| matches!(t, Tok::Lit(_)))
        .find_map(|t| match t {
            Tok::Lit(l) => l
                .trim_matches(|c| c == LIT_OPEN || c == LIT_CLOSE)
                .trim()
                .parse()
                .ok(),
            _ => None,
        });
    let a = v.abs();
    // 한셀: 분모가 고정된 분수(`?/4`)는 정수 자리가 없어도 정수부를 버리고 소수부만 쓴다 (4385.5 → 2/4),
    // 분자가 0 이면 아무것도 쓰지 않는다 (표시 형식 대조 실험). 고정되지 않은 분수(`?/?`)는 가분수다
    let (whole, x) = if has_int || fixed_den.is_some() {
        (a.trunc(), a.fract())
    } else {
        (0.0, a)
    };
    let (n, d) = match fixed_den {
        Some(d) => ((x * d as f64).round() as u64, d),
        None => best_fraction(x, 10u64.pow(den_len as u32) - 1),
    };
    if !has_int && fixed_den.is_some() && n == 0 {
        return if signed && v < 0.0 {
            "-".into()
        } else {
            String::new()
        };
    }
    // 한셀: 분수가 1/1 로 반올림돼도 정수부로 올리지 않는다 (99.999 → 99 1/1)
    let whole = whole as u64;
    let mut out = String::new();
    if signed && v < 0.0 {
        out.push('-');
    }
    if has_int {
        let int_ph: Vec<&Tok> = int_toks
            .iter()
            .copied()
            .filter(|t| matches!(t, Tok::Digit(_)))
            .collect();
        // 정수부 0 은 자리 표시자가 '0' 일 때만 쓴다 (`0 ?/2` → 0 1/2, `# ?/2` → " 1/2")
        let zero_ph = int_ph.iter().any(|t| matches!(t, Tok::Digit('0')));
        let int_str = if whole == 0 && n != 0 && !zero_ph {
            String::new()
        } else {
            whole.to_string()
        };
        let lits_before: Vec<&Tok> = int_toks
            .iter()
            .copied()
            .take_while(|t| !matches!(t, Tok::Digit(_)))
            .collect();
        for t in lits_before {
            push_simple(&mut out, t, v);
        }
        out.push_str(&int_str);
        let after_int = int_toks
            .iter()
            .rposition(|t| matches!(t, Tok::Digit(_)))
            .map(|p| p + 1)
            .unwrap_or(0);
        for t in &int_toks[after_int..] {
            push_simple(&mut out, t, v);
        }
        if n == 0 {
            // TEXT: 분수 자리(분자 자리 + / + 분모 자리)만큼 공백 표시
            if PAD_MARK.with(|p| p.get()) {
                let num_len = si - ns;
                for _ in 0..num_len + 1 + den_len {
                    out.push(PAD_CHAR);
                }
            }
            return out;
        }
    }
    out.push_str(&n.to_string());
    out.push('/');
    out.push_str(&d.to_string());
    out
}

// ---------------------------------------------------------------------------
// 날짜
// ---------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const DAYS_KO: [&str; 7] = ["일", "월", "화", "수", "목", "금", "토"];
const DAYS_ZH: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];

/// 연호 이름 (머리글자, 첫 한자, 이름)
type EraNames = (&'static str, &'static str, &'static str);

/// 일본 연호: 이름, 시작 (연, 월, 일)
const JAPANESE_ERAS: [(EraNames, (i64, u32, u32)); 5] = [
    (("M", "明", "明治"), (1868, 1, 1)),
    (("T", "大", "大正"), (1912, 7, 30)),
    (("S", "昭", "昭和"), (1926, 12, 25)),
    (("H", "平", "平成"), (1989, 1, 8)),
    (("R", "令", "令和"), (2019, 5, 1)),
];

/// 날짜의 일본 연호와 그 해 (한셀: 1912-01-02 → 明45, 1990-04-02 → 平2, 2019-05-01 → 令1)
fn japanese_era(p: &DateParts) -> Option<(EraNames, i64)> {
    JAPANESE_ERAS
        .iter()
        .rev()
        .find(|(_, start)| (p.y, p.mo, p.d) >= *start)
        .map(|(names, start)| (*names, p.y - start.0 + 1))
}

/// 1970-01-01 기준 일수 → (연, 월, 일) (Howard Hinnant 알고리즘).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

struct DateParts {
    y: i64,
    mo: u32,
    d: u32,
    weekday: usize,
    h: u32,
    mi: u32,
    s: u32,
    /// 초 미만 (소수 자릿수 단위로 반올림된 값, 0..10^k)
    sub: u32,
    total_seconds: f64,
}

/// 날짜 일련번호(1900 체계) → 구성 요소. Excel 과 같이 1900-02-29 가 있다고 본다.
fn date_parts(serial: f64, sub_digits: usize) -> Option<DateParts> {
    if !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let unit = 10f64.powi(sub_digits as i32);
    let total = (serial * 86_400.0 * unit).round() / unit;
    let days = (total / 86_400.0).floor();
    let secs = total - days * 86_400.0;
    let whole = secs.floor();
    let sub = ((secs - whole) * unit).round() as u32;
    let days = days as i64;
    let (y, mo, d) = if days == 60 {
        (1900, 2, 29)
    } else if days == 0 {
        (1900, 1, 0)
    } else {
        // 1900-01-01 = 일련번호 1 = 1970 기준 -25567 일. 60 이후는 가짜 2월 29일만큼 하루 당긴다.
        let off = if days < 60 { days - 1 } else { days - 2 };
        civil_from_days(off - 25_567)
    };
    let whole = whole as u32;
    Some(DateParts {
        y,
        mo,
        d,
        weekday: ((days + 6).rem_euclid(7)) as usize,
        h: whole / 3600,
        mi: (whole / 60) % 60,
        s: whole % 60,
        sub,
        total_seconds: total,
    })
}

/// 1904 체계: 일련번호 0 = 1904-01-01.
fn days_from_1904(serial_days: i64) -> i64 {
    // 1904-01-01 은 1970-01-01 기준 -24107 일
    serial_days - 24_107
}

fn format_date(sec: &Section, v: f64, date1904: bool) -> Option<String> {
    let sub_digits = sec
        .toks
        .windows(2)
        .find_map(|w| match (&w[0], &w[1]) {
            (Tok::Dt('s', _) | Tok::Elapsed('s', _), Tok::Dot) => Some(()),
            _ => None,
        })
        .map(|_| {
            let p = sec.toks.iter().position(|t| *t == Tok::Dot).unwrap();
            sec.toks[p + 1..]
                .iter()
                .take_while(|t| matches!(t, Tok::Digit('0')))
                .count()
        })
        .unwrap_or(0);
    // 시스템 시간([$-F400])은 원래 구역에 초가 없으면 초를 반올림하지 않고 버린다 (한셀: `[$-F400]h` 의 8.64초 → 8초,
    // `[$-F400]h:mm:ss AM/PM` 은 9초)
    let v = if sec.sys_trunc && v >= 0.0 {
        (v * 86_400.0 + 1e-4).floor() / 86_400.0
    } else {
        v
    };
    let parts = if date1904 {
        date_parts_1904(v, sub_digits)?
    } else {
        date_parts(v, sub_digits)?
    };
    let has_ampm = sec.toks.iter().any(|t| matches!(t, Tok::AmPm(..)));
    // [DBNum] 날짜의 숫자 (한셀 표시 결과): 연도는 글자마다, 월·일·시·분·초는 자릿값 읽기 — 두 자리 꼴에서 10 보다 작은
    // 값은 앞에 0 (`０九`). DBNum3 은 모두 글자마다 (`１０월`)
    let part = |n: u32, width: usize| -> String {
        let ascii = format!("{n:0width$}");
        match sec.dbnum {
            0 => ascii,
            3 => dbnum_each(3, &ascii),
            k => {
                let p = dbnum_positional(k, u64::from(n));
                if width >= 2 && n < 10 {
                    format!("{}{p}", dbnum_digit(k, 0))
                } else {
                    p
                }
            }
        }
    };
    let year = |s: String| -> String {
        if sec.dbnum == 0 {
            s
        } else {
            dbnum_each(sec.dbnum, &s)
        }
    };
    let mut out = String::new();
    let mut i = 0;
    while i < sec.toks.len() {
        let t = &sec.toks[i];
        match t {
            Tok::Dt('y', n) => {
                if *n <= 2 {
                    out.push_str(&year(format!("{:02}", parts.y.rem_euclid(100))));
                } else {
                    out.push_str(&year(format!("{:04}", parts.y)));
                }
            }
            // 일본어([$-411])의 e 는 연호의 해 (0 을 채우지 않는다)
            Tok::Dt('e', _) if sec.locale == 0x411 => out.push_str(&year(
                japanese_era(&parts).map_or(parts.y, |e| e.1).to_string(),
            )),
            Tok::Dt('e', _) => out.push_str(&year(parts.y.to_string())),
            // 연호 이름 (일본어): g 머리글자, gg 첫 한자, ggg 이름 — 해(e·y)가 없는 구역에는 쓰지 않는다
            Tok::Dt('g', n)
                if sec.locale == 0x411
                    && sec.toks.iter().any(|t| matches!(t, Tok::Dt('e' | 'y', _))) =>
            {
                if let Some((era, _)) = japanese_era(&parts) {
                    out.push_str(match n {
                        1 => era.0,
                        2 => era.1,
                        _ => era.2,
                    });
                }
            }
            Tok::Dt('m', n) => match n {
                1 => out.push_str(&part(parts.mo, 1)),
                2 => out.push_str(&part(parts.mo, 2)),
                3 => out.push_str(&MONTHS[parts.mo as usize - 1][..3]),
                5 => out.push_str(&MONTHS[parts.mo as usize - 1][..1]),
                _ => out.push_str(MONTHS[parts.mo as usize - 1]),
            },
            Tok::Dt('d', n) => match n {
                1 => out.push_str(&part(parts.d, 1)),
                2 => out.push_str(&part(parts.d, 2)),
                3 if sec.korean => out.push_str(DAYS_KO[parts.weekday]),
                3 => out.push_str(&DAYS[parts.weekday][..3]),
                _ if sec.korean => {
                    out.push_str(DAYS_KO[parts.weekday]);
                    out.push_str("요일");
                }
                _ => out.push_str(DAYS[parts.weekday]),
            },
            // 중국어([$-804])의 aaaa 는 星期六 꼴
            Tok::Dt('a', n) if sec.locale == 0x804 && *n >= 4 => {
                out.push_str("星期");
                out.push_str(DAYS_ZH[parts.weekday]);
            }
            Tok::Dt('a', n) => {
                out.push_str(DAYS_KO[parts.weekday]);
                if *n >= 4 {
                    out.push_str("요일");
                }
            }
            Tok::Dt('h', n) => {
                let h = if has_ampm {
                    let h = parts.h % 12;
                    if h == 0 { 12 } else { h }
                } else {
                    parts.h
                };
                out.push_str(&part(h, (*n).min(2)));
            }
            Tok::Minute(n) => out.push_str(&part(parts.mi, (*n).min(2))),
            Tok::Dt('s', n) => out.push_str(&part(parts.s, (*n).min(2))),
            Tok::Elapsed(c, n) => {
                let total = parts.total_seconds.floor() as i64;
                let v = match c {
                    'h' => total / 3600,
                    'm' => total / 60,
                    _ => total,
                };
                out.push_str(&format!("{v:0width$}", width = *n));
            }
            // 한국어([$-412])는 AM/PM·A/P 모두 오전/오후
            Tok::AmPm(..) if sec.korean => {
                out.push_str(if parts.h < 12 { "오전" } else { "오후" })
            }
            Tok::AmPm(am, pm) => out.push_str(if parts.h < 12 { am } else { pm }),
            Tok::Dot if sub_digits > 0 => {
                out.push('.');
                out.push_str(&format!("{:0width$}", parts.sub, width = sub_digits));
                i += 1 + sub_digits;
                continue;
            }
            Tok::Dot => out.push('.'),
            Tok::Lit(l) => out.push_str(l),
            Tok::Dt('g', _) | Tok::Dt('b', _) => {}
            Tok::Digit(c) => out.push(*c),
            Tok::Comma => out.push(','),
            Tok::Percent => out.push('%'),
            Tok::Slash => out.push('/'),
            _ => {}
        }
        i += 1;
    }
    Some(out)
}

fn date_parts_1904(v: f64, sub_digits: usize) -> Option<DateParts> {
    if !(0.0..2_957_004.0).contains(&v) {
        return None;
    }
    let unit = 10f64.powi(sub_digits as i32);
    let total = (v * 86_400.0 * unit).round() / unit;
    let days = (total / 86_400.0).floor();
    let secs = total - days * 86_400.0;
    let whole = secs.floor();
    let sub = ((secs - whole) * unit).round() as u32;
    let days = days as i64;
    let (y, mo, d) = civil_from_days(days_from_1904(days));
    let whole = whole as u32;
    Some(DateParts {
        y,
        mo,
        d,
        weekday: ((days + 5).rem_euclid(7)) as usize,
        h: whole / 3600,
        mi: (whole / 60) % 60,
        s: whole % 60,
        sub,
        total_seconds: total,
    })
}

// ---------------------------------------------------------------------------
// 공개 API
// ---------------------------------------------------------------------------

fn cond_true(c: (Op, f64), v: f64) -> bool {
    match c.0 {
        Op::Lt => v < c.1,
        Op::Le => v <= c.1,
        Op::Gt => v > c.1,
        Op::Ge => v >= c.1,
        Op::Eq => v == c.1,
        Op::Ne => v != c.1,
    }
}

/// 숫자 값에 쓸 구역과 부호 표시 여부를 고른다.
fn pick_number_section(secs: &[Section], v: f64) -> (usize, bool) {
    let numeric: Vec<usize> = (0..secs.len().min(3)).collect();
    if secs.iter().take(3).any(|s| s.cond.is_some()) {
        for &i in &numeric {
            if let Some(c) = secs[i].cond {
                if cond_true(c, v) {
                    // 한셀: 조건이 `[<0]` 인 구역만 음수 부호를 빼고, 다른 조건(`[<=0]`, `[<-5]`, `[<=9999999]` …)은
                    // 음수에 `-` 를 붙인다 (표시 형식 대조 실험)
                    return (i, !(c.0 == Op::Lt && c.1 == 0.0));
                }
            } else {
                // 조건 없는 구역 = 나머지. 음수면 부호를 붙인다.
                return (i, true);
            }
        }
        return (numeric.len().saturating_sub(1), true);
    }
    match numeric.len() {
        0 | 1 => (0, true),
        2 => {
            if v < 0.0 {
                (1, false)
            } else {
                (0, true)
            }
        }
        _ => {
            if v > 0.0 {
                (0, true)
            } else if v < 0.0 {
                (1, false)
            } else {
                (2, true)
            }
        }
    }
}

/// 값을 서식 코드로 표시한다.
pub fn format(value: &CellValue, code: &str, date1904: bool) -> Formatted {
    let raw = split_sections(code);
    let secs: Vec<Section> = raw.iter().map(|s| parse_section(s)).collect();
    match value {
        CellValue::Empty => Formatted::default(),
        CellValue::Text(t) => {
            let sec = if secs.len() >= 4 {
                Some(&secs[3])
            } else {
                secs.iter().find(|s| s.has_at())
            };
            match sec {
                Some(s) => {
                    let mut out = String::new();
                    for tk in &s.toks {
                        match tk {
                            Tok::At => out.push_str(t),
                            Tok::Lit(l) => out.push_str(l),
                            _ => {}
                        }
                    }
                    Formatted {
                        text: dbnum_finish(&out, 0),
                        color: s.color.clone(),
                        overflow: false,
                        ..Default::default()
                    }
                }
                None => Formatted {
                    text: t.clone(),
                    ..Default::default()
                },
            }
        }
        CellValue::Bool(b) => Formatted {
            text: if *b { "TRUE" } else { "FALSE" }.into(),
            ..Default::default()
        },
        CellValue::Error(e) => Formatted {
            text: e.clone(),
            ..Default::default()
        },
        CellValue::Date(d) => Formatted {
            text: d.clone(),
            ..Default::default()
        },
        CellValue::Number(v) => format_number_value(*v, &secs, date1904),
    }
}

fn format_number_value(v: f64, secs: &[Section], date1904: bool) -> Formatted {
    if secs.is_empty() {
        return Formatted {
            text: general(v),
            ..Default::default()
        };
    }
    let (i, signed) = pick_number_section(secs, v);
    let Some(sec) = secs.get(i) else {
        return Formatted::default();
    };
    let mut f = format_section(sec, v, signed, date1904);
    f.as_text = sec.has_at();
    if sec.dbnum != 0 {
        f.text = dbnum_finish(&f.text, sec.dbnum);
    }
    f
}

/// 고른 구역 하나로 숫자를 표시한다 ([DBNum] 숫자 바꾸기는 [`dbnum_finish`] 가 마무리한다)
fn format_section(sec: &Section, v: f64, signed: bool, date1904: bool) -> Formatted {
    let color = sec.color.clone();
    if sec.toks.is_empty() {
        return Formatted {
            text: String::new(),
            color,
            overflow: false,
            ..Default::default()
        };
    }
    if sec.is_date() {
        // 날짜는 부호 없이 일련번호 그대로 쓴다 (음수는 표시 불가)
        return match format_date(sec, v, date1904) {
            Some(text) => Formatted {
                text,
                color,
                overflow: false,
                ..Default::default()
            },
            None => Formatted {
                text: "#".repeat(8),
                color,
                overflow: true,
                ..Default::default()
            },
        };
    }
    if sec.is_general() && !sec.has_digits() {
        let mut out = String::new();
        if signed && v < 0.0 {
            out.push('-');
        }
        for t in &sec.toks {
            match t {
                // [DBNum] 일반 서식은 자릿값으로 읽는다 (표지로 감싸 마무리에서 다시 바꾸지 않게)
                Tok::General if sec.dbnum != 0 => {
                    out.push(LIT_OPEN);
                    out.push_str(&dbnum_general(sec.dbnum, &general_dbnum(v.abs())));
                    out.push(LIT_CLOSE);
                }
                t => push_simple(&mut out, t, v),
            }
        }
        return Formatted {
            text: out,
            color,
            overflow: false,
            ..Default::default()
        };
    }
    if sec.has_at() && !sec.has_digits() {
        // 숫자에 텍스트 서식(@): 일반 서식으로 넣는다
        let mut out = String::new();
        for t in &sec.toks {
            push_simple(&mut out, t, v);
        }
        return Formatted {
            text: out,
            color,
            overflow: false,
            ..Default::default()
        };
    }
    if !sec.has_digits() {
        // 글자만 있는 구역 ("제로")
        let mut out = String::new();
        if signed && v < 0.0 {
            out.push('-');
        }
        for t in &sec.toks {
            if let Tok::Lit(l) = t {
                out.push_str(l);
            }
        }
        return Formatted {
            text: out,
            color,
            overflow: false,
            ..Default::default()
        };
    }
    Formatted {
        text: format_number(sec, v, signed),
        color,
        overflow: false,
        ..Default::default()
    }
}

/// 셀 서식 번호의 서식 코드 (문서의 numFmts → 한셀 내장 → General).
pub fn code_for(styles: &crate::model::Styles, num_fmt_id: u32) -> String {
    styles
        .num_fmts
        .iter()
        .find(|n| n.id == num_fmt_id)
        .map(|n| n.code.clone())
        .or_else(|| builtin(num_fmt_id).map(str::to_string))
        .unwrap_or_else(|| "General".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(code: &str, v: f64) -> String {
        format(&CellValue::Number(v), code, false).text
    }

    // 기대값은 한셀 2024 의 셀 표시 결과(Range.Text)에서 가져온 사실이다.
    #[test]
    fn numbers() {
        assert_eq!(f("0", 0.5), "1");
        assert_eq!(f("0", -0.5), "-1");
        assert_eq!(f("#,##0", 1234567.891), "1,234,568");
        assert_eq!(f("#,##0.00", -1234.5678), "-1,234.57");
        assert_eq!(f("0.00%", 0.001234), "0.12%");
        assert_eq!(f("000000", 1234.5678), "001235");
        assert_eq!(f("#.##", 0.5), ".5");
        assert_eq!(f("#.##", 0.001234), ".");
        assert_eq!(f(".00", 1234.5678), ".57");
        assert_eq!(f("#,##0.0,", 1234567.891), "1,234.6");
        assert_eq!(f("#,##0.0,", -1.0), "0.0");
        assert_eq!(f("\"₩\"#,##0", -1234.5678), "-₩1,235");
        assert_eq!(f("0.0_);\\(0.0\\)", -1234.5678), "(1234.6)");
        assert_eq!(f("0.00;\\-0.00;\"제로\"", 0.0), "제로");
    }

    #[test]
    fn scientific_and_fractions() {
        assert_eq!(f("0.00E+00", 1234.5678), "1.23E+03");
        assert_eq!(f("0.00E+00", 0.5), "5.00E-01");
        assert_eq!(f("##0.0E+0", 46314.75), "46.3E+3");
        assert_eq!(f("##0.0E+0", 1e10), "10.0E+9");
        assert_eq!(f("# ?/?", 1234.5678), "1234 4/7");
        assert_eq!(f("# ??/??", 1234.5678), "1234 46/81");
        assert_eq!(f("# ?/?", 0.5), " 1/2");
        assert_eq!(f("?/?", 1234.5678), "8642/7");
        // 한셀 고유: 음수 지수는 공학 표기 안 함, 분수 1/1 을 정수로 올리지 않음
        assert_eq!(f("##0.0E+0", 0.5), "5.0E-1");
        assert_eq!(f("##0.0E+0", 0.001234), "1.2E-3");
        assert_eq!(f("# ?/?", 99.999), "99 1/1");
    }

    #[test]
    fn conditions() {
        assert_eq!(f("[>=100]\"큰\";\"작은\"", 1234.0), "큰");
        assert_eq!(f("[>=100]\"큰\";\"작은\"", -1234.0), "-작은");
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(f("yyyy-mm-dd", 46314.75), "2026-10-19");
        assert_eq!(f("yyyy\"년\"\\ m\"월\"\\ d\"일\"", 0.0), "1900년 1월 0일");
        assert_eq!(f("h:mm:ss", 1234.5678), "13:37:38");
        assert_eq!(f("h:mm AM/PM", 0.0), "12:00 AM");
        assert_eq!(f("AM/PM\\ h:mm", 46314.75), "PM 6:00");
        assert_eq!(f("[h]:mm:ss", 1234.5678), "29629:37:38");
        assert_eq!(f("mm:ss.0", 1234.5678), "37:37.9");
        assert_eq!(
            f("[$-412]yyyy\"년\"\\ m\"월\"\\ d\"일\"\\ dddd", 46314.75),
            "2026년 10월 19일 월요일"
        );
        assert_eq!(f("aaa", 0.0), "토");
        assert_eq!(f("yy\\.m\\.d", 1234567.891), "80.2.15");
        assert!(format(&CellValue::Number(-1.0), "yyyy-mm-dd", false).overflow);
    }
    /// [DBNum1]–[DBNum4] (한셀 Range.Text, 표시 형식 대조 실험)
    #[test]
    fn dbnum_digits() {
        // 일반 서식: 자릿값 읽기 — DBNum1·3 은 십·백·천 앞의 1 을 빼고, 2·4 는 쓴다
        assert_eq!(f("[DBNum1]General", 12.0), "十二");
        assert_eq!(f("[DBNum1]General", 105.0), "百五");
        assert_eq!(f("[DBNum1]General", 11000.0), "一万千");
        assert_eq!(
            f("[DBNum1]General", 123456789.0),
            "一億二千三百四十五万六千七百八十九"
        );
        assert_eq!(
            f("[DBNum1]General", 12345678901234.0),
            "十二兆三千四百五十六億七千八百九十万千二百三十四"
        );
        assert_eq!(f("[DBNum1]General", 0.5), "０.五");
        assert_eq!(f("[DBNum1]General", -1.0), "-一");
        assert_eq!(f("[DBNum1]General", 1e15), "一E+15");
        assert_eq!(f("[DBNum1]General", 1.234e-6), "一.二三四E-六");
        assert_eq!(f("[DBNum2]General", 12.0), "壹拾貳");
        assert_eq!(f("[DBNum2]General", 0.0), "零");
        assert_eq!(f("[DBNum2]General", 101000.0), "壹拾萬壹阡");
        assert_eq!(f("[DBNum3]General", 20.0), "２十");
        assert_eq!(f("[DBNum3]General", 46022.5), "４万６千２十２.５");
        assert_eq!(f("[DBNum4]General", 11000.0), "일만일천");
        assert_eq!(f("[DBNum4]General", 10.0), "일십");
        // 자리 표시자 서식: 숫자마다 (0 도), 쉼표·소수점·글자는 그대로
        assert_eq!(f("[DBNum1]0", 105.0), "一０五");
        assert_eq!(f("[DBNum1]#,##0\"원\"", 1234.5678), "一,二三五원");
        assert_eq!(f("[DBNum1]0\"1\"", 21.0), "二一1");
        assert_eq!(f("[DBNum2]0.00", 3.25), "參.貳伍");
        assert_eq!(f("[DBNum3]#,##0", 123456789.0), "１２３,４５６,７８９");
        assert_eq!(f("[DBNum4]0.00", 1001.0), "일영영일.영영");
        assert_eq!(f("[DBNum1]0.00E+00", 1234.5678), "一.二三E+０三");
        assert_eq!(f("[DBNum1]# ?/?", 3.25), "三 一/四");
        // 날짜: 연도는 숫자마다, 월·일·시·분·초는 자릿값 (두 자리 꼴의 한 자리 값은 앞에 0), DBNum3 은 숫자마다
        assert_eq!(
            f("[DBNum1]yyyy\"년\" m\"월\" d\"일\"", 46314.75),
            "二０二六년 十월 十九일"
        );
        assert_eq!(
            f("[DBNum1]yy\"년\" mm\"월\" dd\"일\"", 1010.0),
            "０二년 十월 ０六일"
        );
        assert_eq!(f("[DBNum2]m\"월\" d\"일\"", 46022.0), "壹拾貳월 參拾壹일");
        assert_eq!(
            f("[DBNum3]yyyy\"년\" m\"월\" d\"일\"", 46314.75),
            "２０２６년 １０월 １９일"
        );
        assert_eq!(f("[DBNum4]hh:mm:ss", 46022.9583449074), "이십삼:영영:영일");
        assert_eq!(f("[DBNum1]h:m:s", 45689.3802083333), "九:七:三十");
        // [DBNum] 은 그 구역에만
        assert_eq!(f("[DBNum1][>100]0;0.0", 10.0), "10.0");
        assert_eq!(f("[DBNum1][>100]0;0.0", 1001.0), "一００一");
    }

    #[test]
    fn general_precision() {
        // 1 이상 유효숫자 10자리, 1 미만 9자리, 정수부 12자리부터 지수, 작은 수는 소수 7자리까지
        assert_eq!(general(1234567.891), "1234567.891");
        assert_eq!(general(1234567.89012), "1234567.89");
        assert_eq!(general(1.23456789123), "1.234567891");
        assert_eq!(general(1.0 / 3.0), "0.333333333");
        assert_eq!(general(0.0001234567891), "0.000123456789");
        assert_eq!(general(12345678901.2), "12345678901");
        assert_eq!(general(123456789012.0), "1.23457E+11");
        assert_eq!(general(99999999999.5), "1E+11");
        assert_eq!(general(0.0000123), "0.0000123");
        assert_eq!(general(0.00001234), "1.234E-5");
        assert_eq!(general(1e-7), "0.0000001");
        assert_eq!(general(1.2e-7), "1.2E-7");
        assert_eq!(general(1.5e-10), "1.5E-10");
        assert_eq!(general(-0.5), "-0.5");
        // 텍스트 서식(@)의 숫자는 유효숫자 15자리
        assert_eq!(f("@", 0.00001234), "0.00001234");
        assert_eq!(f("@", 123456789012.0), "123456789012");
        assert_eq!(f("@", 1e-9), "0.000000001");
        // [DBNum] 일반 서식은 0.0001 보다 작으면 늘 지수, 소수점 뒤는 지수까지 바꾼다
        assert_eq!(f("[DBNum1]General", 0.00009), "九E-5");
        assert_eq!(f("[DBNum1]General", 1.5e20), "一.五E+二０");
        assert_eq!(f("[DBNum1]General", 0.0001), "０.０００一");
    }

    #[test]
    fn engineering_rounds_to_significant_digits_then_cuts() {
        // 한셀: (정수 자리 표시자 + 소수 자리) 유효숫자로 반올림한 뒤 가수의 소수를 자른다
        assert_eq!(f("##0.0E+0", 1999.0), "1.9E+3");
        assert_eq!(f("##0.0E+0", 32874.25), "32.8E+3");
        assert_eq!(f("##0.0E+0", 987654.0), "987.7E+3");
        assert_eq!(f("##0.0E+0", 123.45), "123.5E+0");
        assert_eq!(f("##0.0E+0", 0.00987), "9.8E-3");
        assert_eq!(f("##0.00E+00", 1949.9), "1.94E+03");
        assert_eq!(f("0.0E+0", 1999.0), "2.0E+3");
    }

    #[test]
    fn hancell_codes() {
        // G/표준 은 일반 서식이 아니다: G 는 연호(한국어에서 빈칸), 나머지는 글자 (파일에서 읽어도 같다)
        assert_eq!(f("G/표준", 12.0), "/표준");
        assert!(format(&CellValue::Number(-1.0), "G/표준", false).overflow);
        // 조건 구역: [<0] 만 음수 부호를 뺀다
        assert_eq!(f("[<0]0;0.0", -1.0), "1");
        assert_eq!(f("[<=0]0;0", -1.0), "-1");
        assert_eq!(f("[<=9999999]###-####;(0##) ###-####", -1.0), "--1");
        assert_eq!(f("[>100]0;[<-100]0.0;0.00", -101.0), "-101.0");
        // 고정 분모: 정수 자리가 없으면 정수부를 버린다, 분자가 0 이면 분수를 쓰지 않는다
        assert_eq!(f("# ?/8", 1234.5678), "1234 5/8");
        assert_eq!(f("# ?/8", 0.5), " 4/8");
        assert_eq!(f("# ?/8", 1.0), "1 ");
        assert_eq!(f("?/4", 4385.5), "2/4");
        assert_eq!(f("?/4", 32965.0), "");
        assert_eq!(f("0 ?/2", 1.0 / 3.0), "0 1/2");
        // 로캘: 한국어 오전/오후, 시스템 날짜·시간, 일본 연호, 중국어 요일
        assert_eq!(f("[$-412]A/P h:mm", 0.1), "오전 2:24");
        assert_eq!(f("[$-412]h:mm AM/PM", 2.0 / 3.0), "4:00 오후");
        assert_eq!(
            f("[$-F800]dddd, mmmm dd, yyyy", 46314.75),
            "2026년 10월 19일 월요일"
        );
        assert_eq!(f("[$-F400]h:mm:ss AM/PM", 1234.5678), "오후 1:37:38");
        assert_eq!(f("[$-F400]h", 0.0001), "오전 12:00:08");
        assert_eq!(f("[$-411]ge.m.d", 46314.75), "R8.10.19");
        assert_eq!(f("[$-411]gge", 4385.5), "明45");
        assert_eq!(
            f("[$-411]ggge\"年\"m\"月\"d\"日\"", 10000.0),
            "昭和2年5月18日"
        );
        assert_eq!(f("[$-411]ee", 32965.0), "2");
        assert_eq!(
            f("[$-804]yyyy\"年\"m\"月\"d\"日\" aaaa", 43586.0),
            "2019年5月1日 星期三"
        );
    }
}
