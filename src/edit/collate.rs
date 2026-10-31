// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀 정렬의 글 비교 (정렬 대조 실험 — docs/editing.md 12절).
//!
//! 한셀은 글을 Windows 의 한국어(ko-KR) 단어 정렬로 비교한다: 무작위 글 300개를 한셀로 정렬한 차례가
//! `CompareStringEx("ko-KR", NORM_IGNORECASE)` (대소문자 구분이면 플래그 없이)의 안정 정렬과 모두 같았다.
//! 비교는 정렬 키를 바이트로 견주는 것과 같다 — 키는 글자마다 주 가중치를 잇고, 이어서 발음 구별·대소문자
//! 가중치(뒤쪽 기본값 2 는 뗀다), 끝에 단어 정렬에서 건너뛴 글자(`'` `-` 와 제어 문자)의 자리·가중치를 둔다.
//! 글자의 가중치는 `collate_data` 표(라틴·그리스·키릴·기호·가나·한글 자모·전각 문자)와 한글 음절 규칙으로 얻고,
//! 한자는 그 음의 한글 음절 자리(같은 음이면 발음 구별 가중치 차례 — `collate_hanja.bin`)에 두고, 표에 없는 글자는
//! 모두 그 뒤에 코드 차례로 둔다.

use std::cmp::Ordering;

use super::collate_data::{HANGUL, IGNORE_CASE, MULTI, TABLES};
use super::collate_kana::KANA;

/// 글자 하나의 가중치
enum Weight {
    /// 정렬 키에서 아무것도 남기지 않는다
    Ignore,
    /// 단어 정렬에서 건너뛰고 특수 가중치만 남긴다 (가중치, 대소문자)
    Special(u8, u8),
    /// (주 가중치, 발음 구별, 대소문자) 하나
    One(u32, u8, u8),
    /// 여러 주 가중치로 펼쳐지는 글자 ([`MULTI`] 번호)
    Multi(usize),
}

fn table_value(cp: u32) -> Option<u32> {
    TABLES.iter().find_map(|&(start, vals)| {
        let i = cp.checked_sub(start)? as usize;
        vals.get(i).copied()
    })
}

/// 한자의 정렬 가중치 (자동 생성 — 그 음의 한글 음절 자리): 범위마다 글자당 3바이트, 주 가중치(u16 LE, 0 은 표에
/// 없음) + 발음 구별
static HANJA: &[u8] = include_bytes!("collate_hanja.bin");
const HANJA_RANGES: [(u32, u32); 3] = [(0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF)];

fn hanja(cp: u32) -> Option<Weight> {
    let mut base = 0usize;
    for (a, b) in HANJA_RANGES {
        if (a..=b).contains(&cp) {
            let o = (base + (cp - a) as usize) * 3;
            let prim = u16::from_le_bytes([*HANJA.get(o)?, *HANJA.get(o + 1)?]);
            return (prim != 0).then(|| Weight::One(prim as u32, HANJA[o + 2], 2));
        }
        base += (b - a + 1) as usize;
    }
    None
}

fn weight(c: char, ignore_case: bool) -> Weight {
    let cp = c as u32;
    // 가나의 탁점·반탁점은 주 가중치 없이 발음 구별 가중치만 (정렬 키 `01 03`·`01 04`)
    match cp {
        0x3099 | 0x309B | 0xFF9E => return Weight::One(0, 3, 2),
        0x309A | 0x309C | 0xFF9F => return Weight::One(0, 4, 2),
        _ => {}
    }
    // 장음·반복 기호(ゝ ゞ ー ヽ ヾ ｰ)는 정렬 키가 모두 같다 (주 가중치 맨 끝)
    if matches!(cp, 0x309D | 0x309E | 0x30FC | 0x30FD | 0x30FE | 0xFF70) {
        return Weight::One(0xFFFF, 2, 2);
    }
    if let Some(w) = hanja(cp) {
        return w;
    }
    if (0xAC00..=0xD7A3).contains(&cp) {
        let i = HANGUL.partition_point(|&(s, _)| s <= cp) - 1;
        let (s, p) = HANGUL[i];
        return Weight::One(p as u32 + (cp - s), 2, 2);
    }
    match table_value(cp) {
        Some(0) => {
            // 표 범위 안의 빈 자리는 Windows 가 건너뛰는 글자 (NUL 등)
            if cp == 0 {
                Weight::Ignore
            } else {
                fallback(cp)
            }
        }
        Some(v) => {
            let prim = v >> 16;
            match prim {
                1 => Weight::Special((v >> 8) as u8, v as u8),
                2 => Weight::Multi((v & 0xFFFF) as usize),
                _ => {
                    let mut case = v as u8;
                    if ignore_case {
                        case = IGNORE_CASE
                            .iter()
                            .find(|&&(x, _)| x == cp)
                            .map_or(case & !0x10, |&(_, k)| k);
                    }
                    Weight::One(prim, (v >> 8) as u8, case)
                }
            }
        }
        None => fallback(cp),
    }
}

/// 표에 없는 글자: 표의 모든 글자 뒤, 코드 차례
fn fallback(cp: u32) -> Weight {
    Weight::One(0x1_0000 + cp, 2, 2)
}

/// 정렬 키의 단계별 값
#[derive(Default)]
struct Key {
    prim: Vec<u32>,
    diac: Vec<u8>,
    case: Vec<u8>,
    special: Vec<u8>,
}

/// 가나 글자의 갈래 (1 작은 글자, 2 가타카나, 4 반각) — 가나가 아니면 None
fn kana_class(c: char) -> Option<u8> {
    let cp = c as u32;
    let i = KANA.partition_point(|&(_, b, _)| b < cp);
    KANA.get(i)
        .and_then(|&(a, _, f)| (a..).contains(&cp).then_some(f))
}

/// 가나 글자마다 같은 갈래를 묶음으로: 갈래가 선 글자(c4)까지 그 앞 보통 글자(e4)를 적고 ff 로 닫는다 (갈래가 선 글자가 없으면 ff 만)
fn kana_group(flags: &[bool]) -> Vec<u8> {
    let mut v = Vec::new();
    if let Some(last) = flags.iter().rposition(|&f| f) {
        v.extend(flags[..=last].iter().map(|&f| if f { 0xC4 } else { 0xE4 }));
    }
    v.push(0xFF);
    v
}

/// 가나 글자의 모음 가중치: 행마다 8 칸씩 (か행 0x220A … わ행 0x224A) 첫 글자가 `아` 모음, ん·모음 글자는 그대로
fn kana_vowel(prim: u32) -> u32 {
    match prim {
        0x220A..=0x2251 => 0x2202 + (prim - 0x220A) % 8,
        p => p,
    }
}

fn key(s: &str, ignore_case: bool) -> Key {
    let mut k = Key::default();
    let mut kana: Vec<u8> = Vec::new();
    // 직전 글자가 가나면 (모음 가중치, 갈래) — 장음 기호(ー ｰ)는 앞 글자의 모음을 되풀이하고 갈래를 물려받는다
    let mut prev: Option<(u32, u8)> = None;
    let mut prolonged = 0usize;
    for c in s.chars() {
        if matches!(c as u32, 0x30FC | 0xFF70)
            && let Some((v, f)) = prev
        {
            k.prim.push(v);
            k.diac.push(2);
            k.case.push(2);
            kana.push(f);
            prolonged += 1;
            continue;
        }
        let class = kana_class(c);
        if let Some(f) = class {
            kana.push(f);
        }
        prev = None;
        match weight(c, ignore_case) {
            Weight::Ignore => {}
            Weight::Special(w, cs) => {
                let pos = 0xFFFFu32.saturating_sub(k.prim.len() as u32);
                k.special
                    .extend_from_slice(&[(pos >> 8) as u8, pos as u8, w, cs]);
            }
            // 가나의 탁점·반탁점(ﾞ ﾟ ゛ ゜)은 앞 글자의 발음 구별 가중치에 더한다 (ｶﾞ = ｶ 2 + ﾞ 3 = ガ 5)
            Weight::One(0, d, _)
                if matches!(c as u32, 0x3099..=0x309C | 0xFF9E | 0xFF9F) && !k.diac.is_empty() =>
            {
                if let Some(last) = k.diac.last_mut() {
                    *last += d;
                }
            }
            Weight::One(p, d, cs) => {
                k.prim.push(p);
                k.diac.push(d);
                k.case.push(cs);
                if let Some(f) = class {
                    prev = Some((kana_vowel(p), f));
                }
            }
            Weight::Multi(i) => {
                let (_, prims, diac, cs, ic) = MULTI[i];
                let case = if ignore_case { ic } else { cs };
                for (j, &p) in prims.iter().enumerate() {
                    k.prim.push(p as u32);
                    k.diac.push(diac.get(j).copied().unwrap_or(2));
                    k.case.push(case.get(j).copied().unwrap_or(2));
                }
            }
        }
    }
    for v in [&mut k.diac, &mut k.case] {
        while v.last() == Some(&2) {
            v.pop();
        }
    }
    // 가나는 특수 부분 맨 앞에 [작은 글자] 02 [가타카나] [반각] 묶음을 둔다 (한셀 정렬 키: ｧ < ァ < ぁ < ｱ < ア < あ,
    // ｱｲ < アイ < アい < あイ < あい)
    if !kana.is_empty() {
        let mut g = kana_group(&kana.iter().map(|f| f & 1 != 0).collect::<Vec<_>>());
        // 장음 기호마다 `05` 하나 (`03 05 …` 다음에 `02`)
        if prolonged > 0 {
            g.push(0x03);
            g.extend(std::iter::repeat_n(0x05, prolonged));
        }
        g.push(0x02);
        g.extend(kana_group(
            &kana.iter().map(|f| f & 2 != 0).collect::<Vec<_>>(),
        ));
        g.extend(kana_group(
            &kana.iter().map(|f| f & 4 != 0).collect::<Vec<_>>(),
        ));
        g.extend(std::mem::take(&mut k.special));
        k.special = g;
    }
    k
}

/// 한셀 정렬의 글 비교. `match_case` 면 대소문자를 구분한다 (소문자가 앞)
pub fn compare(a: &str, b: &str, match_case: bool) -> Ordering {
    let (x, y) = (key(a, !match_case), key(b, !match_case));
    x.prim
        .cmp(&y.prim)
        .then_with(|| x.diac.cmp(&y.diac))
        .then_with(|| x.case.cmp(&y.case))
        .then_with(|| x.special.cmp(&y.special))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(list: &[&str], match_case: bool) -> Vec<String> {
        let mut v: Vec<&str> = list.to_vec();
        v.sort_by(|a, b| compare(a, b, match_case));
        v.into_iter().map(String::from).collect()
    }

    #[test]
    fn hcell_order_of_sort_experiment_1() {
        // 정렬 대조 실험 1: 한셀이 오름차순으로 정렬한 차례 (같은 것은 원래 차례)
        let want = [
            "", " x", "!x", "#x", "_x", "~x", "10", "1a", "9", "a", "Ａ", "a1", "a10", "a2",
            "apple", "Apple", "APPLE", "B", "banana", "Banana", "e", "é", "f", "'x", "-x", "xy",
            "x-y", "z", "Z", "あ", "ㄱ", "가", "가가", "가나", "각", "나", "漢", "ㅏ",
        ];
        let mut input: Vec<&str> = want.to_vec();
        input.reverse();
        // 대소문자만 다른 것은 같으므로 원래 차례를 맞춰 둔다
        let fix = |v: &mut Vec<&str>| {
            for grp in [
                ["apple", "Apple", "APPLE"].as_slice(),
                &["banana", "Banana"],
                &["z", "Z"],
            ] {
                let pos: Vec<usize> = v
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| grp.contains(s))
                    .map(|(i, _)| i)
                    .collect();
                for (k, &p) in pos.iter().enumerate() {
                    v[p] = grp[k];
                }
            }
        };
        fix(&mut input);
        let got = sorted(&input, false);
        assert_eq!(got, want.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    }

    /// 가나 (한셀 정렬 대조 실험): 작은 글자 < 보통, 반각 < 전각 가타카나 < 히라가나, 장음 기호는 앞 글자의 모음, 탁점은 발음 구별
    #[test]
    fn kana_like_hcell() {
        let want = [
            "ｧ", "ァ", "ぁ", "ｱ", "ア", "あ", "ｱｲ", "アイ", "アい", "あイ", "あい", "ｲ", "イ",
            "い", "ヴ", "う゛", "ｶ", "カ", "か", "ｶﾞ", "ガ", "が", "ワ", "わ", "ﾝ", "ン", "ん",
            "ー",
        ];
        for w in want.windows(2) {
            assert_eq!(
                compare(w[0], w[1], false),
                Ordering::Less,
                "{} < {}",
                w[0],
                w[1]
            );
        }
        // 장음 기호끼리는 같다 (안정 정렬이 입력 차례를 지킨다)
        assert_eq!(compare("ー", "ｰ", false), Ordering::Equal);
        // 장음 기호 = 앞 글자의 모음 (おー = おお), 앞 글자가 없으면 맨 끝
        assert_eq!(compare("おー", "おお", false), Ordering::Greater);
        assert_eq!(compare("おー", "おッｷ", false), Ordering::Less);
        assert_eq!(compare("キｰ", "ぎぃ", false), Ordering::Less);
        assert_eq!(compare("ぎぃ", "ギか", false), Ordering::Less);
        assert_eq!(compare("ーあ", "ん", false), Ordering::Greater);
    }

    #[test]
    fn case_and_width() {
        use Ordering::*;
        assert_eq!(compare("a", "A", false), Equal);
        assert_eq!(compare("a", "A", true), Less);
        assert_eq!(compare("a", "Ａ", false), Less);
        assert_eq!(compare("Ａ", "a1", false), Less);
        // 단어 정렬: 하이픈·작은따옴표는 끝에서만 따진다
        assert_eq!(compare("xy", "x-y", false), Less);
        assert_eq!(compare("'x", "-x", false), Less);
        assert_eq!(compare("-x", "xa", false), Less);
        // 동그라미 숫자는 숫자 자리 (대소문자 단계에서만 다르다)
        assert_eq!(compare("1", "①", false), Less);
        assert_eq!(compare("①", "2", false), Less);
        // 한글 자모는 그 자음의 음절 앞, 모음은 모든 음절 뒤
        assert_eq!(compare("ㄲ", "까", false), Less);
        assert_eq!(compare("깋", "ㄲ", false), Less);
        assert_eq!(compare("힣", "ㅏ", false), Less);
        // 한자는 그 음의 음절 자리 (漢 = 한): 나 < 漢 < 핟, 한 < 漢 < 韓
        assert_eq!(compare("나", "漢", false), Less);
        assert_eq!(compare("漢", "핟", false), Less);
        assert_eq!(compare("한", "漢", false), Less);
        assert_eq!(compare("漢", "韓", false), Less);
    }
}
