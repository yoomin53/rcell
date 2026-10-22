// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 바이너리 OOXML 파트(`*.bin`)의 레코드 계층 (BIFF12).
//!
//! 레코드 틀은 공개 명세 [MS-XLSB] 2.1.4 를 따른다: 타입과 길이를 7비트 가변 길이
//! 정수로 쓴다 (타입 최대 2바이트, 길이 최대 4바이트).

#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    pub offset: usize,
    pub rtype: u32,
    pub data: &'a [u8],
}

impl Record<'_> {
    pub fn name(&self) -> &'static str {
        record_name(self.rtype)
    }
}

fn varint(b: &[u8], pos: &mut usize, max_bytes: usize) -> Option<u32> {
    let mut v: u32 = 0;
    for i in 0..max_bytes {
        let byte = *b.get(*pos)?;
        *pos += 1;
        v |= ((byte & 0x7F) as u32) << (7 * i);
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    Some(v)
}

/// 파트 전체를 레코드로 나눈다.
pub fn records(part: &[u8]) -> Vec<Record<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < part.len() {
        let offset = pos;
        // 타입은 바이트 그대로의 가변 길이 표기 (MS-XLSB 2.1.4: 최상위 비트가 연속 표시)
        let Some(t) = varint(part, &mut pos, 2) else {
            break;
        };
        let Some(len) = varint(part, &mut pos, 4) else {
            break;
        };
        let end = (pos + len as usize).min(part.len());
        out.push(Record {
            offset,
            rtype: t,
            data: &part[pos..end],
        });
        pos = end;
    }
    out
}

/// [MS-XLSB] 2.3 레코드 이름 (자주 쓰이는 것).
pub fn record_name(t: u32) -> &'static str {
    match t {
        0 => "BrtRowHdr",
        1 => "BrtCellBlank",
        2 => "BrtCellRk",
        3 => "BrtCellError",
        4 => "BrtCellBool",
        5 => "BrtCellReal",
        6 => "BrtCellSt",
        7 => "BrtCellIsst",
        8 => "BrtFmlaString",
        9 => "BrtFmlaNum",
        10 => "BrtFmlaBool",
        11 => "BrtFmlaError",
        19 => "BrtSSTItem",
        35 => "BrtFRTBegin",
        36 => "BrtFRTEnd",
        37 => "BrtACBegin",
        38 => "BrtACEnd",
        39 => "BrtName",
        40 => "BrtIndexRowBlock",
        42 => "BrtIndexBlock",
        43 => "BrtFont",
        44 => "BrtFmt",
        45 => "BrtFill",
        46 => "BrtBorder",
        47 => "BrtXF",
        48 => "BrtStyle",
        60 => "BrtColInfo",
        62 => "BrtCellRString",
        64 => "BrtDVal",
        128 => "BrtFileVersion",
        129 => "BrtBeginSheet",
        130 => "BrtEndSheet",
        131 => "BrtBeginBook",
        132 => "BrtEndBook",
        133 => "BrtBeginWsViews",
        134 => "BrtEndWsViews",
        135 => "BrtBeginBookViews",
        136 => "BrtEndBookViews",
        137 => "BrtBeginWsView",
        138 => "BrtEndWsView",
        143 => "BrtBeginBundleShs",
        144 => "BrtEndBundleShs",
        145 => "BrtBeginSheetData",
        146 => "BrtEndSheetData",
        147 => "BrtWsProp",
        148 => "BrtWsDim",
        151 => "BrtPane",
        152 => "BrtSel",
        153 => "BrtWbProp",
        154 => "BrtWbFactoid",
        155 => "BrtFileRecover",
        156 => "BrtBundleSh",
        157 => "BrtCalcProp",
        158 => "BrtBookView",
        159 => "BrtBeginSst",
        160 => "BrtEndSst",
        176 => "BrtMergeCell",
        177 => "BrtBeginMergeCells",
        178 => "BrtEndMergeCells",
        278 => "BrtBeginStyleSheet",
        279 => "BrtEndStyleSheet",
        353 => "BrtBeginExternals",
        354 => "BrtEndExternals",
        355 => "BrtSupBookSrc",
        357 => "BrtSupSelf",
        358 => "BrtSupSame",
        359 => "BrtSupTabs",
        360 => "BrtBeginSupBook",
        362 => "BrtExternSheet",
        588 => "BrtEndSupBook",
        390 => "BrtBeginColInfos",
        391 => "BrtEndColInfos",
        426 => "BrtArrFmla",
        427 => "BrtShrFmla",
        428 => "BrtTable",
        461 => "BrtBeginConditionalFormatting",
        462 => "BrtEndConditionalFormatting",
        463 => "BrtBeginCFRule",
        464 => "BrtEndCFRule",
        465 => "BrtBeginIconSet",
        466 => "BrtEndIconSet",
        467 => "BrtBeginDatabar",
        468 => "BrtEndDatabar",
        469 => "BrtBeginColorScale",
        470 => "BrtEndColorScale",
        471 => "BrtCFVO",
        564 => "BrtColor",
        1046 => "BrtBeginConditionalFormatting14",
        1047 => "BrtEndConditionalFormatting14",
        1048 => "BrtBeginCFRule14",
        1049 => "BrtEndCFRule14",
        1050 => "BrtCFVO14",
        1051 => "BrtBeginDatabar14",
        1052 => "BrtBeginIconSet14",
        1055 => "BrtColor14",
        1112 => "BrtCFIcon",
        1135 => "BrtBeginConditionalFormattings",
        1136 => "BrtEndConditionalFormattings",
        1146 => "BrtCFRuleExt",
        1155 => "BrtEndIconSet14",
        1156 => "BrtEndDatabar14",
        485 => "BrtWsFmtInfo",
        476 => "BrtMargins",
        477 => "BrtPrintOptions",
        478 => "BrtPageSetup",
        479 => "BrtBeginHeaderFooter",
        480 => "BrtEndHeaderFooter",
        494 => "BrtHLink",
        603 => "BrtBeginFills",
        604 => "BrtEndFills",
        611 => "BrtBeginFonts",
        612 => "BrtEndFonts",
        613 => "BrtBeginBorders",
        614 => "BrtEndBorders",
        615 => "BrtBeginFmts",
        616 => "BrtEndFmts",
        617 => "BrtBeginCellXFs",
        618 => "BrtEndCellXFs",
        619 => "BrtBeginStyles",
        620 => "BrtEndStyles",
        626 => "BrtBeginCellStyleXFs",
        627 => "BrtEndCellStyleXFs",
        2071 => "BrtAbsPath15",
        // 표 (표 대조 실험의 한셀 바이너리)
        343 => "BrtBeginList",
        344 => "BrtEndList",
        345 => "BrtBeginListCols",
        346 => "BrtEndListCols",
        347 => "BrtBeginListCol",
        348 => "BrtEndListCol",
        351 => "BrtListCCFmla",
        352 => "BrtListTrFmla",
        513 => "BrtTableStyleClient",
        550 => "BrtDrawing",
        551 => "BrtLegacyDrawing",
        660 => "BrtBeginListParts",
        661 => "BrtListPart",
        662 => "BrtEndListParts",
        161 => "BrtBeginAFilter",
        162 => "BrtEndAFilter",
        163 => "BrtBeginFilterColumn",
        164 => "BrtEndFilterColumn",
        165 => "BrtBeginFilters",
        166 => "BrtEndFilters",
        167 => "BrtFilter",
        170 => "BrtTop10Filter",
        171 => "BrtDynamicFilter",
        172 => "BrtBeginCustomFilters",
        173 => "BrtEndCustomFilters",
        174 => "BrtCustomFilter",
        // 피벗 표 (피벗 실험의 한셀 바이너리 — 묶음의 시작·끝 짝은 [MS-XLSB] 이름)
        280 => "BrtBeginSXView",
        281 => "BrtEndSXVI",
        282 => "BrtBeginSXVI",
        283 => "BrtBeginSXVIs",
        284 => "BrtEndSXVIs",
        285 => "BrtBeginSXVD",
        286 => "BrtEndSXVD",
        287 => "BrtBeginSXVDs",
        288 => "BrtEndSXVDs",
        289 => "BrtBeginSXPI",
        290 => "BrtEndSXPI",
        291 => "BrtBeginSXPIs",
        292 => "BrtEndSXPIs",
        293 => "BrtBeginSXDI",
        294 => "BrtEndSXDI",
        295 => "BrtBeginSXDIs",
        296 => "BrtEndSXDIs",
        297 => "BrtBeginSXLI",
        298 => "BrtEndSXLI",
        299 => "BrtBeginSXLIRws",
        300 => "BrtEndSXLIRws",
        301 => "BrtBeginSXLICols",
        302 => "BrtEndSXLICols",
        303 => "BrtBeginSXFormat",
        304 => "BrtEndSXFormat",
        305 => "BrtBeginSXFormats",
        306 => "BrtEndSxFormats",
        309 => "BrtBeginISXVDRws",
        310 => "BrtEndISXVDRws",
        311 => "BrtBeginISXVDCols",
        312 => "BrtEndISXVDCols",
        313 => "BrtEndSXLocation",
        314 => "BrtBeginSXLocation",
        315 => "BrtEndSXView",
        388 => "BrtBeginISXVIs",
        389 => "BrtEndISXVIs",
        // 피벗 캐시
        20 => "BrtPCDIMissing",
        21 => "BrtPCDINumber",
        22 => "BrtPCDIBoolean",
        23 => "BrtPCDIError",
        24 => "BrtPCDIString",
        25 => "BrtPCDIDatetime",
        26 => "BrtPCDIIndex",
        27 => "BrtPCDIAMissing",
        28 => "BrtPCDIANumber",
        29 => "BrtPCDIABoolean",
        30 => "BrtPCDIAError",
        31 => "BrtPCDIAString",
        32 => "BrtPCDIADatetime",
        33 => "BrtPCRRecord",
        34 => "BrtPCRRecordDt",
        179 => "BrtBeginPivotCacheDef",
        180 => "BrtEndPivotCacheDef",
        181 => "BrtBeginPCDFields",
        182 => "BrtEndPCDFields",
        183 => "BrtBeginPCDField",
        184 => "BrtEndPCDField",
        185 => "BrtBeginPCDSource",
        186 => "BrtEndPCDSource",
        187 => "BrtBeginPCDSRange",
        188 => "BrtEndPCDSRange",
        189 => "BrtBeginPCDFAtbl",
        190 => "BrtEndPCDFAtbl",
        191 => "BrtBeginPCDIRun",
        192 => "BrtEndPCDIRun",
        193 => "BrtBeginPivotCacheRecords",
        194 => "BrtEndPivotCacheRecords",
        384 => "BrtBeginPivotCacheIDs",
        385 => "BrtEndPivotCacheIDs",
        386 => "BrtBeginPivotCacheID",
        387 => "BrtEndPivotCacheID",
        _ => "?",
    }
}
