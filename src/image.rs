// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그림 파일의 형식과 픽셀 크기 (머리만 읽는다).
//!
//! 한셀은 그림을 저장할 때마다 PNG 를 다시 압축하는 등 바이트가 바뀌므로,
//! 비교(`rcell diff`)는 그림을 형식과 크기로 본다.

/// 그림 형식과 픽셀 크기
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    /// `png`, `jpeg`, `gif`, `bmp`, `tiff`, `emf`, `wmf`
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
}

/// 파일 머리에서 형식과 크기를 읽는다. 모르는 형식이면 `None`.
pub fn info(d: &[u8]) -> Option<ImageInfo> {
    let be32 = |i: usize| {
        d.get(i..i + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let le32 = |i: usize| {
        d.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let be16 = |i: usize| {
        d.get(i..i + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
    };
    let le16 = |i: usize| {
        d.get(i..i + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as u32)
    };
    let mk = |format, width, height| {
        Some(ImageInfo {
            format,
            width,
            height,
        })
    };
    if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        return mk("png", be32(16)?, be32(20)?);
    }
    if d.starts_with(&[0xFF, 0xD8]) {
        // 표식을 따라가 SOFn (C0–CF, C4·C8·CC 제외) 에서 높이·너비
        let mut i = 2;
        while i + 4 <= d.len() {
            if d[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = d[i + 1];
            if matches!(m, 0xD8 | 0x01 | 0xD0..=0xD7 | 0xFF) {
                i += if m == 0xFF { 1 } else { 2 };
                continue;
            }
            let len = be16(i + 2)? as usize;
            if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                return mk("jpeg", be16(i + 7)?, be16(i + 5)?);
            }
            i += 2 + len;
        }
        return mk("jpeg", 0, 0);
    }
    if d.starts_with(b"GIF8") {
        return mk("gif", le16(6)?, le16(8)?);
    }
    if d.starts_with(b"BM") {
        let h = le32(22)? as i32;
        return mk("bmp", le32(18)?, h.unsigned_abs());
    }
    if d.starts_with(b"II*\0") || d.starts_with(b"MM\0*") {
        return mk("tiff", 0, 0);
    }
    if le32(0) == Some(1) && d.get(40..44) == Some(b" EMF") {
        // rclBounds 가 아니라 rclFrame(0.01mm)과 장치 크기로 픽셀을 셈하면 환경마다 달라진다 → 경계 상자
        let w = le32(16)? as i32 - le32(8)? as i32;
        let h = le32(20)? as i32 - le32(12)? as i32;
        return mk("emf", w.unsigned_abs() + 1, h.unsigned_abs() + 1);
    }
    if le32(0) == Some(0x9AC6_CDD7) {
        // 배치형 WMF: 경계 (left, top, right, bottom) i16
        let w = le16(10)? as i16 - le16(6)? as i16;
        let h = le16(12)? as i16 - le16(8)? as i16;
        return mk("wmf", w.unsigned_abs() as u32, h.unsigned_abs() as u32);
    }
    None
}

/// 형식 → (확장자, 미디어 형식)
pub fn ext_and_type(format: &str) -> (&'static str, &'static str) {
    match format {
        "png" => ("png", "image/png"),
        "jpeg" => ("jpeg", "image/jpeg"),
        "gif" => ("gif", "image/gif"),
        "bmp" => ("bmp", "image/bmp"),
        "tiff" => ("tiff", "image/tiff"),
        "emf" => ("emf", "image/x-emf"),
        "wmf" => ("wmf", "image/x-wmf"),
        "pict" => ("pict", "image/pict"),
        _ => ("bin", "application/octet-stream"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_png_jpeg_gif_bmp_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&8u32.to_be_bytes());
        png.extend_from_slice(&6u32.to_be_bytes());
        assert_eq!(
            info(&png).map(|i| (i.format, i.width, i.height)),
            Some(("png", 8, 6))
        );

        // SOI, APP0(길이 4), SOF0: 길이, 정밀도, 높이 3, 너비 5
        let jpg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0, 3, 0, 5, 1, 1, 0x11, 0,
        ];
        assert_eq!(info(&jpg).map(|i| (i.width, i.height)), Some((5, 3)));

        let gif = b"GIF89a\x0a\x00\x04\x00";
        assert_eq!(
            info(gif).map(|i| (i.format, i.width, i.height)),
            Some(("gif", 10, 4))
        );

        let mut bmp = vec![b'B', b'M'];
        bmp.resize(18, 0);
        bmp.extend_from_slice(&7u32.to_le_bytes());
        bmp.extend_from_slice(&(-2i32).to_le_bytes());
        assert_eq!(info(&bmp).map(|i| (i.width, i.height)), Some((7, 2)));
        assert_eq!(info(b"nothing"), None);
    }
}
