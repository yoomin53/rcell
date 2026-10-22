// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 글 비교 순서 (비교 연산자, 근사 찾기, 정렬).
//!
//! 한셀은 대소문자를 무시하고 Windows 의 언어별 정렬과 같은 순서로 비교한다 (함수 대조 실험 cmp 묶음):
//! - 1단계: 기호 < 숫자 < 라틴 문자 < 그리스 < 키릴 < 그 밖의 문자 < 가나 < 한글 < 한자.
//!   악센트는 1단계에서 무시한다 (`ä` 는 `a` 와 `b` 사이). `ß` = `ss`, `æ` = `ae`, `œ` = `oe`.
//!   하이픈·작은따옴표는 1단계에서 건너뛴다 (`coop` < `co-op`, `co-op` < `cooq`).
//! - 2단계: 악센트 (`a` < `ä`).
//! - 3단계: 전각·동그라미 숫자 같은 모양 차이 (`1` < `１` < `①`), 건너뛴 하이픈.
//!
//! 한글 음절은 자모(초성·중성·종성)로 풀어 비교한다: 호환 자모 `ㄱ` < `가` < `ㅏ`.
//! 표의 세부(기호끼리의 순서 …)는 관측한 경우에 맞춘 근사다.

use std::cmp::Ordering;

#[derive(Default)]
struct Key {
    primary: Vec<u32>,
    secondary: Vec<u32>,
    tertiary: Vec<u32>,
}

/// ASCII 기호의 순서 (Windows 문자열 정렬 표의 기호 부분)
const SYMBOLS: &str = " !\"#$%&()*,./:;?@[\\]^_`{|}~+<=>";

const CLASS_SYMBOL: u32 = 0x0100_0000;
const CLASS_DIGIT: u32 = 0x0200_0000;
const CLASS_LATIN: u32 = 0x0300_0000;
const CLASS_GREEK: u32 = 0x0400_0000;
const CLASS_CYRILLIC: u32 = 0x0500_0000;
const CLASS_OTHER: u32 = 0x0600_0000;
const CLASS_KANA: u32 = 0x0700_0000;
const CLASS_HANGUL: u32 = 0x0800_0000;
const CLASS_HAN: u32 = 0x0900_0000;

/// 라틴 문자의 기본 글자와 악센트 (악센트 번호 0 은 없음). 두 글자로 펼치는 것은 둘째 글자도.
fn latin(c: char) -> Option<(char, Option<char>, u32)> {
    let lc = c.to_lowercase().next().unwrap_or(c);
    if lc.is_ascii_lowercase() {
        return Some((lc, None, 0));
    }
    let (base, acc) = match lc {
        'à' => ('a', 1),
        'á' => ('a', 2),
        'â' => ('a', 3),
        'ã' => ('a', 4),
        'ä' => ('a', 5),
        'å' => ('a', 6),
        'ā' | 'ă' | 'ą' => ('a', 7),
        'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' => ('c', 8),
        'ď' | 'đ' => ('d', 9),
        'è' => ('e', 1),
        'é' => ('e', 2),
        'ê' => ('e', 3),
        'ë' => ('e', 5),
        'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => ('e', 7),
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => ('g', 8),
        'ĥ' | 'ħ' => ('h', 8),
        'ì' => ('i', 1),
        'í' => ('i', 2),
        'î' => ('i', 3),
        'ï' => ('i', 5),
        'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => ('i', 7),
        'ĵ' => ('j', 8),
        'ķ' => ('k', 8),
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => ('l', 8),
        'ñ' | 'ń' | 'ņ' | 'ň' => ('n', 4),
        'ò' => ('o', 1),
        'ó' => ('o', 2),
        'ô' => ('o', 3),
        'õ' => ('o', 4),
        'ö' => ('o', 5),
        'ø' => ('o', 9),
        'ō' | 'ŏ' | 'ő' => ('o', 7),
        'ŕ' | 'ŗ' | 'ř' => ('r', 8),
        'ś' | 'ŝ' | 'ş' | 'š' => ('s', 8),
        'ţ' | 'ť' | 'ŧ' => ('t', 8),
        'ù' => ('u', 1),
        'ú' => ('u', 2),
        'û' => ('u', 3),
        'ü' => ('u', 5),
        'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => ('u', 7),
        'ŵ' => ('w', 8),
        'ý' | 'ÿ' | 'ŷ' => ('y', 5),
        'ź' | 'ż' | 'ž' => ('z', 8),
        'ß' => return Some(('s', Some('s'), 0)),
        'æ' => return Some(('a', Some('e'), 0)),
        'œ' => return Some(('o', Some('e'), 0)),
        'þ' => return Some(('t', Some('h'), 0)),
        _ => return None,
    };
    Some((base, None, acc))
}

/// 한글 음절 → (초성, 중성, 종성) 번호 (종성 없으면 0)
fn hangul_jamo(c: char) -> Option<(u32, u32, u32)> {
    let s = c as u32;
    if !(0xAC00..=0xD7A3).contains(&s) {
        return None;
    }
    let i = s - 0xAC00;
    Some((i / 588, (i % 588) / 28, i % 28))
}

/// 호환 자모(ㄱ … ㅣ)의 무게: 자음은 초성 자리, 모음은 중성 자리
fn compat_jamo(c: char) -> Option<u32> {
    const CONS: &str = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";
    const VOWELS: &str = "ㅏㅐㅑㅒㅓㅔㅕㅖㅗㅘㅙㅚㅛㅜㅝㅞㅟㅠㅡㅢㅣ";
    if let Some(i) = CONS.chars().position(|x| x == c) {
        return Some(1 + i as u32);
    }
    VOWELS.chars().position(|x| x == c).map(|i| 100 + i as u32)
}

fn key(s: &str) -> Key {
    let mut k = Key::default();
    let push = |p: u32, sec: u32, ter: u32, k: &mut Key| {
        k.primary.push(p);
        k.secondary.push(sec);
        k.tertiary.push(ter);
    };
    for (pos, c) in s.chars().enumerate() {
        let cp = c as u32;
        // 하이픈·작은따옴표: 1단계에서 건너뛰고 3단계 끝에 위치를 남긴다
        if matches!(c, '-' | '\'' | '\u{2010}' | '\u{2011}' | '\u{00AD}') {
            k.tertiary.push(0x1000_0000 + pos as u32);
            continue;
        }
        // 전각 ASCII → 반각 + 3단계 차이
        let (c, width) = match cp {
            0xFF01..=0xFF5E => (char::from_u32(cp - 0xFEE0).unwrap_or(c), 1),
            0x3000 => (' ', 1),
            _ => (c, 0),
        };
        if let Some(i) = SYMBOLS.find(c) {
            push(CLASS_SYMBOL + i as u32, 0, width, &mut k);
            continue;
        }
        if c.is_ascii_digit() {
            push(CLASS_DIGIT + (c as u32 - '0' as u32), 0, width, &mut k);
            continue;
        }
        // 동그라미·로마 숫자: 숫자 값 + 3단계 차이
        match cp {
            0x2460..=0x2468 => {
                push(CLASS_DIGIT + 1 + (cp - 0x2460), 0, 2, &mut k);
                continue;
            }
            0x2160..=0x2168 | 0x2170..=0x2178 => {
                push(CLASS_DIGIT + 1 + (cp - 0x2160) % 16, 0, 3, &mut k);
                continue;
            }
            _ => {}
        }
        if let Some((base, second, acc)) = latin(c) {
            push(CLASS_LATIN + (base as u32 - 'a' as u32), acc, width, &mut k);
            if let Some(b2) = second {
                push(CLASS_LATIN + (b2 as u32 - 'a' as u32), 0, width, &mut k);
            }
            continue;
        }
        if let Some((l, v, t)) = hangul_jamo(c) {
            push(CLASS_HANGUL + 1 + l, 0, 0, &mut k);
            push(CLASS_HANGUL + 100 + v, 0, 0, &mut k);
            if t > 0 {
                push(CLASS_HANGUL + 200 + t, 0, 0, &mut k);
            }
            continue;
        }
        if let Some(w) = compat_jamo(c) {
            push(CLASS_HANGUL + w, 0, 1, &mut k);
            continue;
        }
        let lower = c.to_lowercase().next().unwrap_or(c) as u32;
        let (p, ter) = match cp {
            0x0370..=0x03FF => (CLASS_GREEK + lower, width),
            0x0400..=0x04FF => (CLASS_CYRILLIC + lower, width),
            // 가타카나는 같은 소리의 히라가나와 1단계가 같다
            0x3041..=0x3096 => (CLASS_KANA + cp, 0),
            0x30A1..=0x30F6 => (CLASS_KANA + cp - 0x60, 1),
            0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF => (CLASS_HAN + cp, 0),
            _ if c.is_alphanumeric() => (CLASS_OTHER + lower, width),
            // 그 밖의 기호·구두점은 ASCII 기호 다음, 숫자 앞
            _ => (CLASS_SYMBOL + 0x100 + cp, width),
        };
        push(p, 0, ter, &mut k);
    }
    k
}

/// 대소문자를 무시한 비교
pub fn compare(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (ka, kb) = (key(a), key(b));
    ka.primary
        .cmp(&kb.primary)
        .then_with(|| ka.secondary.cmp(&kb.secondary))
        .then_with(|| ka.tertiary.cmp(&kb.tertiary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ordering::*;

    #[test]
    fn observed_order() {
        // 함수 대조 실험 cmp 묶음에서 한셀이 낸 순서
        for (a, b, o) in [
            ("a", "A", Equal),
            ("ä", "a", Greater),
            ("ä", "b", Less),
            ("ß", "ss", Equal),
            ("æ", "b", Less),
            ("œ", "ss", Less),
            ("coop", "co-op", Less),
            ("co op", "co-op", Less),
            ("~", "1", Less),
            ("€", "1", Less),
            ("１", "1", Greater),
            ("①", "a", Less),
            ("Ω", "z", Greater),
            ("あ", "가", Less),
            ("一", "가", Greater),
            ("ㄱ", "가", Less),
            ("ㅏ", "가", Greater),
            ("가", "나", Less),
            ("a1", "a10", Less),
            ("_", "a", Less),
        ] {
            assert_eq!(compare(a, b), o, "{a} vs {b}");
        }
    }
}
