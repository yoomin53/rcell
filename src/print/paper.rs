// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 용지 크기 (`pageSetup/@paperSize` 번호 → 세로 방향 너비·높이).
//!
//! 번호는 OOXML·Windows 인쇄 용지 번호와 같다. 한셀 페이지 나누기 대조 실험(Letter·Legal·A3·A4·A5·B4·B5)에서
//! 확인한 것은 B4·B5 가 JIS 판(257×364, 182×257 mm)이라는 것 — 나머지는 표준 크기를 그대로 둔다.

/// 용지 하나: 세로 방향 너비·높이 (pt)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    pub width: f64,
    pub height: f64,
}

const fn mm(w: f64, h: f64) -> (f64, f64) {
    (w, h)
}

const fn inch(w: f64, h: f64) -> (f64, f64) {
    (w * 25.4, h * 25.4)
}

/// (번호, 너비 mm, 높이 mm)
const TABLE: &[(u32, (f64, f64))] = &[
    (1, inch(8.5, 11.0)),     // Letter
    (2, inch(8.5, 11.0)),     // Letter Small
    (3, inch(11.0, 17.0)),    // Tabloid
    (4, inch(17.0, 11.0)),    // Ledger
    (5, inch(8.5, 14.0)),     // Legal
    (6, inch(5.5, 8.5)),      // Statement
    (7, inch(7.25, 10.5)),    // Executive
    (8, mm(297.0, 420.0)),    // A3
    (9, mm(210.0, 297.0)),    // A4
    (10, mm(210.0, 297.0)),   // A4 Small
    (11, mm(148.0, 210.0)),   // A5
    (12, mm(257.0, 364.0)),   // B4 (JIS)
    (13, mm(182.0, 257.0)),   // B5 (JIS)
    (14, inch(8.5, 13.0)),    // Folio
    (15, mm(215.0, 275.0)),   // Quarto
    (16, inch(10.0, 14.0)),   // 10×14
    (17, inch(11.0, 17.0)),   // 11×17
    (18, inch(8.5, 11.0)),    // Note
    (19, inch(3.875, 8.875)), // 봉투 #9
    (20, inch(4.125, 9.5)),   // 봉투 #10
    (21, inch(4.5, 10.375)),  // 봉투 #11
    (22, inch(4.75, 11.0)),   // 봉투 #12
    (23, inch(5.0, 11.5)),    // 봉투 #14
    (24, inch(17.0, 22.0)),   // C
    (25, inch(22.0, 34.0)),   // D
    (26, inch(34.0, 44.0)),   // E
    (27, mm(110.0, 220.0)),   // 봉투 DL
    (28, mm(162.0, 229.0)),   // 봉투 C5
    (29, mm(324.0, 458.0)),   // 봉투 C3
    (30, mm(229.0, 324.0)),   // 봉투 C4
    (31, mm(114.0, 162.0)),   // 봉투 C6
    (32, mm(114.0, 229.0)),   // 봉투 C65
    (33, mm(250.0, 353.0)),   // 봉투 B4 (ISO)
    (34, mm(176.0, 250.0)),   // 봉투 B5 (ISO)
    (35, mm(176.0, 125.0)),   // 봉투 B6
    (36, mm(110.0, 230.0)),   // 봉투 Italy
    (37, inch(3.875, 7.5)),   // 봉투 Monarch
    (38, inch(3.625, 6.5)),   // 봉투 6 3/4
    (39, inch(14.875, 11.0)), // US Std Fanfold
    (40, inch(8.5, 12.0)),    // German Std Fanfold
    (41, inch(8.5, 13.0)),    // German Legal Fanfold
    (42, mm(250.0, 353.0)),   // B4 (ISO)
    (43, mm(100.0, 148.0)),   // 일본 엽서
    (44, inch(9.0, 11.0)),
    (45, inch(10.0, 11.0)),
    (46, inch(15.0, 11.0)),
    (47, mm(220.0, 220.0)), // 봉투 Invite
    (50, inch(9.5, 12.0)),  // Letter Extra
    (51, inch(9.5, 15.0)),  // Legal Extra
    (52, inch(11.69, 18.0)),
    (53, mm(236.0, 322.0)), // A4 Extra
    (54, inch(8.5, 11.0)),  // Letter Transverse
    (55, mm(210.0, 297.0)), // A4 Transverse
    (56, inch(9.5, 12.0)),
    (57, mm(227.0, 356.0)),  // SuperA
    (58, mm(305.0, 487.0)),  // SuperB
    (59, inch(8.5, 12.69)),  // Letter Plus
    (60, mm(210.0, 330.0)),  // A4 Plus
    (61, mm(148.0, 210.0)),  // A5 Transverse
    (62, mm(182.0, 257.0)),  // B5 (JIS) Transverse
    (63, mm(322.0, 445.0)),  // A3 Extra
    (64, mm(174.0, 235.0)),  // A5 Extra
    (65, mm(201.0, 276.0)),  // B5 (ISO) Extra
    (66, mm(420.0, 594.0)),  // A2
    (67, mm(297.0, 420.0)),  // A3 Transverse
    (68, mm(322.0, 445.0)),  // A3 Extra Transverse
    (69, mm(200.0, 148.0)),  // 일본 왕복 엽서
    (70, mm(105.0, 148.0)),  // A6
    (88, mm(128.0, 182.0)),  // B6 (JIS)
    (118, mm(197.0, 273.0)), // PRC 16K
];

/// 용지 번호의 크기. 모르는 번호·0 이면 A4 (한셀 기본 용지)
pub fn paper(code: Option<u32>) -> Paper {
    let (w, h) = code
        .and_then(|c| TABLE.iter().find(|(k, _)| *k == c))
        .map_or((210.0, 297.0), |(_, s)| *s);
    Paper {
        width: w / 25.4 * 72.0,
        height: h / 25.4 * 72.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_sizes() {
        let a4 = paper(Some(9));
        assert!((a4.width - 595.2756).abs() < 1e-3 && (a4.height - 841.8898).abs() < 1e-3);
        let letter = paper(Some(1));
        assert_eq!((letter.width, letter.height), (612.0, 792.0));
        assert_eq!(paper(None), a4);
    }
}
