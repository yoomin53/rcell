// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 `.cell` (한셀 바이너리) `/Workbook` 스트림의 레코드 계층.
//!
//! 레코드 틀은 Excel BIFF 와 같다: `[타입 u16][길이 u16][데이터]`, 리틀 엔디언.
//! 레코드 이름은 공개 명세 [MS-XLS] 의 BIFF8 이름을 쓰고, 한컴 전용 레코드는
//! 관찰로 붙인 이름(`HC_*`)을 쓴다.

/// 레코드 하나 (원본 스트림을 빌림).
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    /// 스트림 안에서 레코드 헤더의 위치
    pub offset: usize,
    pub rtype: u16,
    pub data: &'a [u8],
}

impl Record<'_> {
    pub fn name(&self) -> &'static str {
        record_name(self.rtype)
    }
}

/// 스트림 전체를 레코드로 나눈다. 잘린 마지막 레코드는 남은 바이트만 담는다.
///
/// 한셀 레거시는 CONTINUE 로 나누지 않고 긴 레코드를 그대로 쓴다. 길이가 0x8000 이상이면
/// u16 길이의 최상위 비트를 세우고 u16 을 하나 더 쓴다 (관측):
/// `길이 = (첫 u16 & 0x7FFF) | (둘째 u16 << 15)`.
pub fn records(stream: &[u8]) -> Vec<Record<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= stream.len() {
        let rtype = u16::from_le_bytes([stream[pos], stream[pos + 1]]);
        let len16 = u16::from_le_bytes([stream[pos + 2], stream[pos + 3]]) as usize;
        let mut start = pos + 4;
        let len = if len16 & 0x8000 != 0 && start + 2 <= stream.len() {
            let ext = u16::from_le_bytes([stream[start], stream[start + 1]]) as usize;
            start += 2;
            (len16 & 0x7FFF) | (ext << 15)
        } else {
            len16
        };
        let end = (start + len).min(stream.len());
        out.push(Record {
            offset: pos,
            rtype,
            data: &stream[start..end],
        });
        pos = start + len;
    }
    out
}

/// BIFF8 공개 명세 레코드 이름, 또는 한셀 전용 레코드의 관찰 이름.
pub fn record_name(rtype: u16) -> &'static str {
    match rtype {
        0x0006 => "FORMULA",
        0x000A => "EOF",
        0x000C => "CALCCOUNT",
        0x000D => "CALCMODE",
        0x000E => "PRECISION",
        0x000F => "REFMODE",
        0x0010 => "DELTA",
        0x0011 => "ITERATION",
        0x0012 => "PROTECT",
        0x0013 => "PASSWORD",
        0x0014 => "HEADER",
        0x0015 => "FOOTER",
        0x0017 => "EXTERNSHEET",
        0x0018 => "NAME",
        0x0019 => "WINDOWPROTECT",
        0x001A => "VERTICALPAGEBREAKS",
        0x001B => "HORIZONTALPAGEBREAKS",
        0x001C => "NOTE",
        0x001D => "SELECTION",
        0x0022 => "DATEMODE",
        0x0026 => "LEFTMARGIN",
        0x0027 => "RIGHTMARGIN",
        0x0028 => "TOPMARGIN",
        0x0029 => "BOTTOMMARGIN",
        0x002A => "PRINTHEADERS",
        0x002B => "PRINTGRIDLINES",
        0x002F => "FILEPASS",
        0x0031 => "FONT",
        0x003C => "CONTINUE",
        0x003D => "WINDOW1",
        0x0040 => "BACKUP",
        0x0041 => "PANE",
        0x0042 => "CODEPAGE",
        0x004D => "PLS",
        0x0055 => "DEFCOLWIDTH",
        0x005C => "WRITEACCESS",
        0x005D => "OBJ",
        0x005E => "UNCALCED",
        0x005F => "SAVERECALC",
        0x0063 => "OBJPROTECT",
        0x007D => "COLINFO",
        0x0080 => "GUTS",
        0x0081 => "WSBOOL",
        0x0082 => "GRIDSET",
        0x0083 => "HCENTER",
        0x0084 => "VCENTER",
        0x0085 => "BOUNDSHEET",
        0x0086 => "WRITEPROT",
        0x008C => "COUNTRY",
        0x008D => "HIDEOBJ",
        0x0092 => "PALETTE",
        0x0099 => "STANDARDWIDTH",
        0x00A0 => "SCL",
        0x00A1 => "SETUP",
        0x00BD => "MULRK",
        0x00BE => "MULBLANK",
        0x00C1 => "MMS",
        0x00D7 => "DBCELL",
        0x00DA => "BOOKBOOL",
        0x00DD => "SCENPROTECT",
        0x00E0 => "XF",
        0x00E1 => "INTERFACEHDR",
        0x00E2 => "INTERFACEEND",
        0x00E5 => "MERGECELLS",
        0x00EB => "MSODRAWINGGROUP",
        0x00EC => "MSODRAWING",
        0x00EF => "PHONETICPR",
        0x00FC => "SST",
        0x00FD => "LABELSST",
        0x00FF => "EXTSST",
        0x0160 => "USESELFS",
        0x0161 => "DSF",
        0x01AE => "SUPBOOK",
        0x01AF => "PROT4REV",
        0x01B0 => "CONDFMT",
        0x01B1 => "CF",
        0x01B2 => "DVAL",
        0x01B6 => "TXO",
        0x01B7 => "REFRESHALL",
        0x01B8 => "HLINK",
        0x01BA => "CODENAME",
        0x01BC => "PROT4REVPASS",
        0x01BE => "DV",
        0x01C0 => "EXCEL9FILE",
        0x01C1 => "RECALCID",
        0x0200 => "DIMENSIONS",
        0x0201 => "BLANK",
        0x0203 => "NUMBER",
        0x0204 => "LABEL",
        0x0205 => "BOOLERR",
        0x0207 => "STRING",
        0x0208 => "ROW",
        0x020B => "INDEX",
        0x0221 => "ARRAY",
        0x0225 => "DEFAULTROWHEIGHT",
        0x0236 => "TABLE",
        0x023E => "WINDOW2",
        0x027E => "RK",
        0x0293 => "STYLE",
        0x041E => "FORMAT",
        0x04BC => "SHRFMLA",
        0x0800 => "HLINKTOOLTIP",
        0x0809 => "BOF",
        0x0862 => "SHEETEXT",
        0x0863 => "BOOKEXT",
        0x0867 => "FEATHDR",
        0x0868 => "FEAT",
        0x087C => "XFCRC",
        0x087D => "XFEXT",
        0x088C => "COMPAT12",
        0x088E => "TABLESTYLES",
        0x0892 => "STYLEEXT",
        0x0896 => "THEME",
        0x089A => "MTRSETTINGS",
        0x089B => "COMPRESSPICTURES",
        0x08A3 => "FORCEFULLCALCULATION",
        _ => "?",
    }
}

/// 한셀 전용으로 보이는 레코드 타입인지 (BIFF8 명세에 없는 번호).
pub fn is_unknown(rtype: u16) -> bool {
    record_name(rtype) == "?"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_records_and_names_them() {
        let s = [0x09, 0x08, 0x02, 0x00, 0x01, 0x10, 0x0A, 0x00, 0x00, 0x00];
        let r = records(&s);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].name(), r[0].data), ("BOF", &[0x01, 0x10][..]));
        assert_eq!((r[1].name(), r[1].offset), ("EOF", 6));
    }

    #[test]
    fn extended_length_records() {
        // 길이 0x8001: 첫 u16 = 0x8001 (0x0001 | 상위 비트), 둘째 u16 = 1 → 1 | (1 << 15)
        let mut s = vec![0xFC, 0x00, 0x01, 0x80, 0x01, 0x00];
        s.resize(6 + 0x8001, 0xAA);
        s.extend_from_slice(&[0x0A, 0x00, 0x00, 0x00]);
        let r = records(&s);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].data.len(), 0x8001);
        assert_eq!(r[1].name(), "EOF");
    }
}
