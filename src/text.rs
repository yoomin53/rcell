// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 바이너리 형식 공용 문자열 도우미.

/// UTF-16LE 바이트를 코드 단위로 (홀수 끝 바이트는 버린다).
pub fn utf16le_units(raw: &[u8]) -> impl Iterator<Item = u16> + '_ {
    raw.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
}

/// UTF-16LE 바이트 전체를 문자열로.
pub fn utf16le(raw: &[u8]) -> String {
    let units: Vec<u16> = utf16le_units(raw).collect();
    String::from_utf16_lossy(&units)
}

/// UTF-16LE 바이트를 첫 NUL 앞까지 문자열로.
pub fn utf16le_nul(raw: &[u8]) -> String {
    let units: Vec<u16> = utf16le_units(raw).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

/// `_xHHHH_` 꼴인가 (OOXML ST_Xstring 의 글자 이스케이프)
fn xescape_at(b: &[u8], i: usize) -> Option<u32> {
    let s = b.get(i..i + 7)?;
    if s[0] != b'_' || s[1] != b'x' || s[6] != b'_' {
        return None;
    }
    let hex = std::str::from_utf8(&s[2..6]).ok()?;
    u32::from_str_radix(hex, 16).ok()
}

/// 셀 글의 `_xHHHH_` 를 글자로 푼다. XML 1.0 이 못 담는 제어 문자(CHAR(1) …)를 한셀·Excel 은 이렇게
/// 쓴다 (함수 대조 실험). 짝 없는 서로게이트는 U+FFFD 로.
pub fn xstring_decode(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains("_x") {
        return std::borrow::Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if let Some(cp) = xescape_at(b, i) {
            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
            i += 7;
            continue;
        }
        let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        i += ch.len_utf8();
    }
    std::borrow::Cow::Owned(out)
}

/// [`xstring_decode`] 의 반대: XML 이 못 담는 제어 문자와, 그대로 두면 이스케이프로 읽힐 `_xHHHH_` 의 `_` 를
/// `_xHHHH_` 로 쓴다.
pub fn xstring_encode(s: &str) -> std::borrow::Cow<'_, str> {
    let needs = s
        .chars()
        .any(|c| (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r'))
        || (s.contains("_x") && (0..s.len()).any(|i| xescape_at(s.as_bytes(), i).is_some()));
    if !needs {
        return std::borrow::Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < b.len() {
        if xescape_at(b, i).is_some() {
            out.push_str("_x005F_");
            i += 1;
            continue;
        }
        let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
        if (ch as u32) < 0x20 && !matches!(ch, '\t' | '\n' | '\r') {
            out.push_str(&format!("_x{:04X}_", ch as u32));
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    std::borrow::Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16le() {
        assert_eq!(utf16le(&[0x48, 0xc5, 0x55, 0xb1]), "안녕");
        assert_eq!(utf16le_nul(&[0x41, 0, 0, 0, 0x42, 0]), "A");
    }

    #[test]
    fn xstring_escapes() {
        assert_eq!(xstring_decode("a_x0001_b"), "a\u{1}b");
        assert_eq!(xstring_decode("_x005F_x0001_"), "_x0001_");
        assert_eq!(xstring_decode("_xZZZZ_"), "_xZZZZ_");
        assert_eq!(xstring_encode("a\u{1}b"), "a_x0001_b");
        assert_eq!(xstring_encode("_x0001_"), "_x005F_x0001_");
        for s in ["plain", "a\u{1}\u{1f}_x0041_ end", "x_y"] {
            assert_eq!(xstring_decode(&xstring_encode(s)), s);
        }
    }
}
