// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! .cell 파일에 나타나는 XML 네임스페이스.
//!
//! 한셀은 `x:` 접두사를 쓰고 Excel은 기본 네임스페이스를 쓰므로,
//! 요소는 항상 접두사가 아니라 네임스페이스 URI로 비교한다.

/// SpreadsheetML (ECMA-376 Transitional)
pub const SML: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// SpreadsheetML (ISO/IEC 29500 Strict)
pub const SML_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
/// 관계(r:id) 속성
pub const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const REL_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
/// OPC 패키지 관계 파트(.rels)
pub const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// OPC `[Content_Types].xml`
pub const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
/// Markup Compatibility (mc:AlternateContent, mc:Ignorable)
pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// Excel 2010 확장 (스파크라인 등)
pub const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
pub const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
pub const EXTENDED_PROPS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";

/// 한컴 스프레드시트 확장 (`hs:`). 글꼴 장평·자간, 문단 모양 등.
pub const HS: &str = "http://schemas.haansoft.com/office/spreadsheet/8.0";
/// VML (메모 도형)
pub const VML: &str = "urn:schemas-microsoft-com:vml";
pub const VML_EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
/// 한컴 스프레드시트 정렬 확장. HncOOXML.dll 문자열 테이블에서 확인, 샘플에서는 아직 미관측.
pub const HS_ALIGNMENT_9: &str = "http://schemas.haansoft.com/office/spreadsheet/9.0/Alignment";
/// 한컴 공통 확장. HncOOXML.dll 문자열 테이블에서 확인.
pub const HANCOM_OFFICE_8: &str = "http://schemas.haansoft.com/office/8.0";

/// 한컴 매크로(VBScript) 프로젝트 관계 타입. HncOOXML.dll 문자열 테이블에서 확인.
pub const REL_HANCOM_VBS_PROJECT: &str =
    "http://schemas.hancom.com/office/relationships/vbsProject";
/// 한컴 매크로 프로젝트 콘텐츠 타입 (`xl/vbsProject.bin`)
pub const CT_HANCOM_VBS_PROJECT: &str = "application/vnd.hancom-office.vbsProject";
/// 한컴 매크로 프로젝트 스트림. XML·바이너리는 `xl/vbsProject.bin` 복합 문서 안, 레거시는 OLE 루트에 있다
/// (패키지 파트 실험)
pub const VBS_PROJECT_STREAM: &str = "/_VBS_PROJECT_W";

/// 한컴이 정의한 네임스페이스인지 판정한다.
pub fn is_hancom(uri: &str) -> bool {
    uri.starts_with("http://schemas.haansoft.com/")
        || uri.starts_with("http://schemas.hancom.com/")
        || uri.starts_with("urn:haansoft:")
}

/// SpreadsheetML 본 네임스페이스(Transitional/Strict)인지 판정한다.
pub fn is_sml(uri: &str) -> bool {
    uri == SML || uri == SML_STRICT
}

pub fn is_rel(uri: &str) -> bool {
    uri == REL || uri == REL_STRICT
}

/// mc:AlternateContent 에서 rcell 이 이해한다고 선언하는 네임스페이스.
/// 여기 포함된 네임스페이스를 요구하는 mc:Choice 는 Fallback 보다 우선 선택된다.
pub const UNDERSTOOD: &[&str] = &[SML, SML_STRICT, REL, HS, HS_ALIGNMENT_9, X14, XM];
