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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16le() {
        assert_eq!(utf16le(&[0x48, 0xc5, 0x55, 0xb1]), "안녕");
        assert_eq!(utf16le_nul(&[0x41, 0, 0, 0, 0x42, 0]), "A");
    }
}
