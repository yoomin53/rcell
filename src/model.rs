// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! .cell 문서 모델.
//!
//! SpreadsheetML 공통 부분과 한컴 확장(`hs:`)을 함께 담는다. 아직 의미가 확인되지
//! 않은 한컴 확장 값은 `other` 맵에 원문 그대로 보존한다.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Default, Serialize)]
pub struct Workbook {
    /// docProps/app.xml `Application` (한셀은 "Cell")
    pub application: Option<String>,
    /// docProps/app.xml `AppVersion`
    pub app_version: Option<String>,
    /// docProps/app.xml `TotalTime` — 편집 시간 (분). 레거시는 SummaryInformation PID 0x0A
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_time: Option<u32>,
    /// 문서 속성 (docProps/core.xml, 레거시 SummaryInformation)
    pub properties: DocProperties,
    pub file_version: Option<FileVersion>,
    pub date1904: bool,
    /// 통합 문서 창 (`bookViews/workbookView`, 레거시 WINDOW1, XLSB BrtBookView). 없으면 기본값
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<WorkbookView>,
    /// `workbookPr/@codeName` (레거시 전역 CODENAME, XLSB BrtWbProp 의 이름)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_name: Option<String>,
    pub sheets: Vec<Sheet>,
    pub shared_strings: Vec<RichText>,
    /// 공유 문자열 참조 수 (`x:sst/@count`, 바이너리 BrtBeginSst 의 cstTotal). 한셀이 바이너리에 그대로 옮겨 쓴다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_strings_count: Option<u32>,
    pub styles: Styles,
    pub defined_names: Vec<DefinedName>,
    /// 원본 그대로 옮겨 쓰는 파트 (차트·그림·표·피벗·매크로 프로젝트 …와 그 관계 파일)
    #[serde(skip)]
    pub raw_parts: Vec<RawPart>,
    /// 통합 문서 관계로만 이어지는 원본 파트 (피벗 캐시·연결·XML 대응·사용자 지정 XML·매크로 프로젝트)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub part_links: Vec<PartLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<WorkbookProtection>,
    /// 외부 통합 문서 연결. 수식의 `[n]` 은 이 목록의 n번째(1부터)다.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub external_links: Vec<ExternalLink>,
    pub hancom: HancomWorkbookExt,
    /// 테마 (xl/theme/theme1.xml). 없으면 렌더링 때 Office 기본 테마를 쓴다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<crate::theme::Theme>,
}

/// 외부 통합 문서 연결 (`xl/externalLinks/externalLinkN.xml`, 레거시 SUPBOOK).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExternalLink {
    /// 연결 대상 경로 (관계 Target, 예: `file:///C:/a/b.xls`, `/a/b.xls`)
    pub target: String,
    pub sheet_names: Vec<String>,
    /// 외부 시트 값의 캐시 (`sheetDataSet`). 레거시·바이너리에 없으면 비어 있다
    /// (한셀은 그때 시트마다 `refreshError="0"` 인 빈 캐시를 쓴다).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sheet_data: Vec<ExternalSheetData>,
    /// 경로를 잃은 연결 (관계 종류 `…/xlExternalLinkPath/xlPathMissing`, 레거시는 부호화되지 않은 가상 경로).
    /// 한셀은 이런 연결의 이름 참조를 따옴표 없이 쓴다.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub path_missing: bool,
}

/// 외부 시트 하나의 캐시 (`sheetData`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ExternalSheetData {
    /// `sheetNames` 안의 번호 (0부터)
    pub sheet_id: u32,
    /// 마지막으로 갱신할 때 오류가 났음
    pub refresh_error: bool,
    pub cells: Vec<(CellRef, CellValue)>,
}

impl ExternalLink {
    /// 대상 경로의 파일 이름 (퍼센트 부호화를 푼다). 수식 참조의 따옴표 판단에 쓴다.
    pub fn file_name(&self) -> String {
        let last = self.target.rsplit(['/', '\\']).next().unwrap_or("");
        let bytes = last.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%'
                && let Some(v) = last
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

impl Workbook {
    pub fn sheet_by_name(&self, name: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|s| s.name == name)
    }

    /// 한셀이 저장한 문서인지 (`fileVersion/@appName="HCell"` 또는 `Application="Cell"`).
    pub fn is_written_by_hancom(&self) -> bool {
        self.file_version
            .as_ref()
            .and_then(|f| f.app_name.as_deref())
            .is_some_and(|a| a.eq_ignore_ascii_case("HCell"))
            || self.application.as_deref() == Some("Cell")
    }
}

/// 통합 문서 창 하나 (`workbookView`). 위치·크기는 1/20 pt
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkbookView {
    pub x_window: i32,
    pub y_window: i32,
    pub window_width: u32,
    pub window_height: u32,
    /// 시트 탭 영역 대 가로 스크롤 막대 (1/1000, 기본 600)
    pub tab_ratio: u32,
    /// 탭 줄에 처음 보이는 시트 (0부터)
    pub first_sheet: u32,
    /// 활성 시트 (0부터). 저장할 때는 선택된 시트(`tabSelected`)를 쓴다
    pub active_tab: u32,
    pub show_horizontal_scroll: bool,
    pub show_vertical_scroll: bool,
    pub show_sheet_tabs: bool,
}

impl Default for WorkbookView {
    fn default() -> Self {
        WorkbookView {
            x_window: 0,
            y_window: 0,
            window_width: 0,
            window_height: 0,
            tab_ratio: 600,
            first_sheet: 0,
            active_tab: 0,
            show_horizontal_scroll: true,
            show_vertical_scroll: true,
            show_sheet_tabs: true,
        }
    }
}

/// `x:workbook/x:fileVersion`
#[derive(Debug, Clone, Default, Serialize)]
pub struct FileVersion {
    pub app_name: Option<String>,
    pub last_edited: Option<String>,
    pub lowest_edited: Option<String>,
    pub rup_build: Option<String>,
}

/// 통합 문서 수준의 한컴 확장.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HancomWorkbookExt {
    /// `x:calcPr/@hs:hclCalcId` — 한셀 계산 엔진 버전으로 추정
    pub calc_id: Option<u32>,
    /// 한컴 매크로(VBScript) 프로젝트 파트 이름
    pub vbs_project_part: Option<String>,
    /// 한컴 매크로 프로젝트의 `_VBS_PROJECT_W` 스트림 (XML·바이너리는 `xl/vbsProject.bin` 복합 문서 안,
    /// 레거시는 OLE 루트의 스트림 — 세 형식에서 바이트가 같다, 패키지 파트 실험)
    #[serde(skip)]
    pub vbs_project: Option<Vec<u8>>,
    /// 아직 해석하지 않은 통합 문서 수준 `hs:` 속성
    pub other: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SheetState {
    #[default]
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SheetKind {
    #[default]
    Worksheet,
    Chartsheet,
    Other(String),
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Sheet {
    pub name: String,
    pub sheet_id: u32,
    pub state: SheetState,
    pub kind: SheetKind,
    /// 패키지 안의 파트 이름
    pub part: String,
    pub dimension: Option<String>,
    pub show_grid_lines: bool,
    pub default_col_width: Option<f64>,
    pub default_row_height: Option<f64>,
    /// 기본 행 높이를 직접 정했다 (`sheetFormatPr/@customHeight`, 바이너리 BrtWsFmtInfo·레거시 DEFAULTROWHEIGHT 의
    /// 플래그 비트 0). 켜져 있으면 높이를 직접 정하지 않은 행도 다시 맞추지 않고 기본 높이를 쓴다 (한셀 화면)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default_row_custom: bool,
    pub cols: Vec<ColInfo>,
    /// 높이·숨김 등 속성이 있는 행만
    pub rows: Vec<RowInfo>,
    /// 행 우선 순서
    pub cells: Vec<Cell>,
    pub merges: Vec<CellRange>,
    pub hyperlinks: Vec<Hyperlink>,
    /// x14 스파크라인 그룹 수
    pub sparkline_groups: usize,
    pub drawing_parts: Vec<String>,
    /// `sheetPr/@codeName` (레거시 CODENAME)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_name: Option<String>,
    /// 시트 탭 색 (`sheetPr/tabColor`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_color: Option<Color>,
    /// 시트 보기 (확대율, 선택 영역, 틀 고정 …)
    pub view: SheetView,
    /// 인쇄 설정
    pub page: PageLayout,
    /// 메모
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    /// 그리기 개체 (도형·글상자·그림·차트·그룹)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    /// 조건부 서식
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conditional_formats: Vec<ConditionalFormat>,
    /// 데이터 유효성
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub data_validations: Vec<DataValidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_filter: Option<AutoFilter>,
    /// 필터가 걸려 있음 (`sheetPr/@filterMode`)
    pub filter_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<SheetProtection>,
    /// 개요 요약 행이 위 (`outlinePr summaryBelow="0"`), 요약 열이 왼쪽 (`summaryRight="0"`)
    pub summary_above: bool,
    pub summary_left: bool,
    /// 표 (ListObject)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<Table>,
    /// 시트 관계로만 이어지는 원본 파트 (피벗 표·쿼리 표·프린터 설정)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub part_links: Vec<PartLink>,
}

/// 원본 그대로 옮겨 쓰는 파트로 가는 관계 (XML 원본, 바이너리는 형식이 같은 파트만). 파트와 그 관계가
/// 가리키는 파트는 [`Workbook::raw_parts`] 에 있다
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PartLink {
    /// 관계 종류 URI
    pub rel_type: String,
    /// 파트 이름 (앞 `/` 없이)
    pub part: String,
    /// 피벗 캐시 번호 (`pivotCaches/pivotCache/@cacheId`, 피벗 캐시 관계만)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_id: Option<u32>,
}

/// 표 (ListObject): XML `x:table` 파트, 바이너리 `BrtBeginList`… 파트, 레거시 0xB005… 레코드
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Table {
    pub id: u32,
    pub name: String,
    pub display_name: String,
    /// 범위 (`ref`, 머리글·요약 행 포함)
    pub range: String,
    /// 머리글 행 수 (`headerRowCount`, 기본 1)
    pub header_rows: u32,
    /// 요약 행 수 (`totalsRowCount`, 기본 0)
    pub totals_rows: u32,
    /// 요약 행을 보인 적 있음 (`totalsRowShown`, 기본 참)
    pub totals_shown: bool,
    /// 종류 (`tableType`: 없으면 worksheet, `xml`, `queryTable`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<u32>,
    /// 차등 서식 번호: 머리글, 데이터, 요약, 표 테두리, 머리글 테두리, 요약 테두리
    /// (`headerRowDxfId`, `dataDxfId`, `totalsRowDxfId`, `tableBorderDxfId`, `headerRowBorderDxfId`,
    /// `totalsRowBorderDxfId`)
    pub dxfs: [Option<u32>; 6],
    /// 셀 스타일 이름: 머리글, 데이터, 요약 (`headerRowCellStyle`, `dataCellStyle`, `totalsRowCellStyle`)
    pub cell_styles: [Option<String>; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_filter: Option<AutoFilter>,
    pub columns: Vec<TableColumn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<TableStyleInfo>,
    /// XML 원본의 표 파트 이름 (그대로 옮겨 쓴다). 없으면 모델로 쓴다
    #[serde(skip)]
    pub part: Option<String>,
}

impl Default for Table {
    fn default() -> Self {
        Table {
            id: 0,
            name: String::new(),
            display_name: String::new(),
            range: String::new(),
            header_rows: 1,
            totals_rows: 0,
            totals_shown: true,
            table_type: None,
            comment: None,
            connection_id: None,
            dxfs: [None; 6],
            cell_styles: [None, None, None],
            auto_filter: None,
            columns: Vec::new(),
            style: None,
            part: None,
        }
    }
}

/// 표의 열 (`tableColumn`, 바이너리 `BrtBeginListCol`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TableColumn {
    pub id: u32,
    pub name: String,
    /// XML 대응의 고유 이름 (`uniqueName`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_name: Option<String>,
    /// 요약 행 함수 (`totalsRowFunction`: sum, average, count …, custom)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals_function: Option<String>,
    /// 요약 행 글 (`totalsRowLabel`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals_label: Option<String>,
    /// 요약 행 사용자 수식 (`totalsRowFormula`, `=` 없이)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals_formula: Option<String>,
    /// 계산 열 수식 (`calculatedColumnFormula`, `=` 없이)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculated_formula: Option<String>,
    /// 차등 서식 번호: 머리글, 데이터, 요약 (`headerRowDxfId`, `dataDxfId`, `totalsRowDxfId`)
    pub dxfs: [Option<u32>; 3],
    /// 쿼리 표 필드 (`queryTableFieldId`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_table_field_id: Option<u32>,
}

/// 표 스타일 적용 (`tableStyleInfo`, 바이너리 `BrtTableStyleClient`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TableStyleInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub first_column: bool,
    pub last_column: bool,
    pub row_stripes: bool,
    pub column_stripes: bool,
}

/// 조건부 서식 하나 (`conditionalFormatting`): 범위 + 규칙들
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ConditionalFormat {
    /// 공백으로 구분한 범위 목록
    pub sqref: String,
    pub rules: Vec<CfRule>,
}

/// 조건부 서식 규칙 (`cfRule`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CfRule {
    /// `cellIs`, `expression`, `colorScale`, `dataBar`, `iconSet`, `top10`, `aboveAverage`,
    /// `duplicateValues`, `uniqueValues`, `containsText`, `beginsWith` …
    pub kind: String,
    pub priority: u32,
    /// 적용할 서식 (`Styles::dxfs` 번호)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dxf_id: Option<u32>,
    pub stop_if_true: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    /// 수식 (`=` 없이), 비교 값 1–2개 또는 조건식
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub formulas: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_period: Option<String>,
    /// top10: 순위, 백분율 여부, 아래쪽 여부
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    pub percent: bool,
    pub bottom: bool,
    /// aboveAverage: 위(기본)/아래, 같음 포함, 표준 편차 배수
    #[serde(skip_serializing_if = "Option::is_none")]
    pub above_average: Option<bool>,
    pub equal_average: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub std_dev: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_scale: Option<ColorScale>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_bar: Option<DataBar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_set: Option<IconSet>,
    /// x14 확장 구역(`x:extLst` 의 `x14:conditionalFormatting`)에만 있는 규칙. 한셀은 Excel 2010 아이콘 집합
    /// (3Stars·3Triangles·5Boxes)과 x14 구역에서 읽은 아이콘 집합을 이 구역에 쓴다 (x14 조건부 서식 실험)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub x14: bool,
}

/// 조건부 서식의 기준 값 (`cfvo`): `min`/`max`/`num`/`percent`/`percentile`/`formula`,
/// x14 데이터 막대는 `autoMin`/`autoMax` 도
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Cfvo {
    pub kind: String,
    /// 값 또는 수식 (`=` 없이). x14 구역은 `xm:f` 의 글
    #[serde(skip_serializing_if = "Option::is_none")]
    pub val: Option<String>,
    /// 아이콘 기준: 이상(기본)/초과
    pub gte: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ColorScale {
    pub cfvos: Vec<Cfvo>,
    pub colors: Vec<Color>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DataBar {
    pub cfvos: Vec<Cfvo>,
    /// 막대 색 (x14 의 `fillColor` 는 한셀이 쓰지 않고 이 색을 쓴다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    pub show_value: bool,
    /// 가장 짧은·긴 막대 (%, 없으면 10·90). x14 확장이 있으면 그 값이다 (한셀은 본 규칙에도 같은 값을 쓴다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// x14 확장 (`x14:dataBar`). 없으면 한셀이 저장할 때 기본값으로 만든다 ([`DataBarExt::implied`])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<DataBarExt>,
}

/// 데이터 막대의 x14 확장 (`x14:dataBar`, XLSB BrtBeginDatabar14 1051). 기본값은 x14 스키마 그대로다
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataBarExt {
    /// x14 기준 값 (`autoMin`/`autoMax` 를 쓸 수 있다). 값은 `xm:f`
    pub cfvos: Vec<Cfvo>,
    /// 막대 테두리 (기본 거짓)
    pub border: bool,
    /// 그라데이션 채우기 (기본 참)
    pub gradient: bool,
    /// `leftToRight` / `rightToLeft` (없으면 `context`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// 음수 막대 색이 양수와 같음 (기본 거짓), 음수 테두리 색이 양수와 같음 (기본 참)
    pub negative_same_as_positive: bool,
    pub negative_border_same_as_positive: bool,
    /// `middle` / `none` (없으면 `automatic`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_fill_color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_border_color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis_color: Option<Color>,
}

impl Default for DataBarExt {
    fn default() -> Self {
        DataBarExt {
            cfvos: Vec::new(),
            border: false,
            gradient: true,
            direction: None,
            negative_same_as_positive: false,
            negative_border_same_as_positive: true,
            axis_position: None,
            border_color: None,
            negative_fill_color: None,
            negative_border_color: None,
            axis_color: None,
        }
    }
}

impl DataBarExt {
    /// x14 확장이 없는 데이터 막대를 한셀이 저장할 때 만드는 확장: 기준 값은 본 규칙 것, 방향 `leftToRight`
    /// (x14 조건부 서식 실험 — rcell 이 확장 없이 쓴 막대, 본 규칙만 있는 막대)
    pub fn implied(bar: &DataBar) -> DataBarExt {
        DataBarExt {
            cfvos: bar.cfvos.clone(),
            direction: Some("leftToRight".into()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct IconSet {
    /// `3Arrows`, `3TrafficLights1`, `5Rating` …
    pub set: String,
    pub cfvos: Vec<Cfvo>,
    pub show_value: bool,
    pub reverse: bool,
    pub percent: bool,
    /// 사용자 지정 아이콘 (`x14:cfIcon`): 단계마다 (아이콘 집합, 번호). 집합 `NoIcons` 는 아이콘 없음
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_icons: Vec<(String, u32)>,
}

/// x14 아이콘 집합만 있는 이름 (Excel 2010) — 한셀은 본 규칙에 있어도 x14 구역으로 옮겨 쓴다
pub const X14_ONLY_ICON_SETS: [&str; 3] = ["3Stars", "3Triangles", "5Boxes"];

/// 데이터 유효성 (`dataValidation`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DataValidation {
    pub sqref: String,
    /// `none`/`whole`/`decimal`/`list`/`date`/`time`/`textLength`/`custom`
    pub kind: String,
    /// `between`(기본)/`notBetween`/`equal`/`notEqual`/`lessThan`/`lessThanOrEqual`/`greaterThan`/`greaterThanOrEqual`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula2: Option<String>,
    pub allow_blank: bool,
    /// OOXML `showDropDown` 그대로: 참이면 목록 단추를 **숨긴다**
    pub hide_drop_down: bool,
    pub show_input: bool,
    pub show_error: bool,
    /// `stop`(기본)/`warning`/`information`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// 그리기 개체 하나. DrawingML 앵커(`xdr:twoCellAnchor` 등) 하나, 레거시 OfficeArt 도형 하나에 해당한다.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Shape {
    pub anchor: Anchor,
    pub kind: ShapeKind,
    /// `cNvPr/@id`, `@name`, `@descr`, `@hidden`
    pub id: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descr: Option<String>,
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xfrm: Option<Xfrm>,
    /// 미리 정의된 모양 (`rect`, `roundRect` …), 사용자 정의 경로면 `custom`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<String>,
    /// 채우기 (None = 지정 없음 → 스타일)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<Outline>,
    /// 바깥 그림자 (`a:effectLst/a:outerShdw`, 레거시 OPT 0x023F·0x0201·0x0205·0x0206)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShapeShadow>,
    /// 켜진 잠금 (`a:spLocks`·`a:cxnSpLocks`·`a:picLocks`·`a:grpSpLocks` 의 속성 이름: `noRot`, `noChangeAspect` …,
    /// 레거시 OPT 0x007F)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locks: Vec<String>,
    /// `xdr:clientData` 의 시트 보호와 함께 잠금·인쇄 (없으면 참, 레거시 OBJ ftCmo 의 fLocked 0x0001·fPrint 0x0010)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locks_with_sheet: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prints_with_sheet: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextBody>,
    /// 그룹의 자식
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Shape>,
    /// 차트·그림 파트 (패키지 경로)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// 차트 내용 (차트 파트를 읽은 것)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<Chart>,
    /// XML 원본의 앵커 요소 그대로 (저장 때 다시 쓴다)
    #[serde(skip)]
    pub raw: Option<String>,
    /// `raw` 안의 관계 (`r:id`/`r:embed`/`r:link`): (ID, 종류, 대상, 외부 여부)
    #[serde(skip)]
    pub raw_rels: Vec<(String, String, String, bool)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum ShapeKind {
    #[default]
    Shape,
    Connector,
    Picture,
    Chart,
    Group,
    /// 그 밖의 graphicFrame 등
    Other,
}

/// 위치 (`twoCellAnchor` / `oneCellAnchor` / `absoluteAnchor`)
#[derive(Debug, Clone, Default, Serialize)]
pub struct Anchor {
    pub kind: AnchorKind,
    /// twoCellAnchor 의 `editAs` (`twoCell`/`oneCell`/`absolute`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_as: Option<String>,
    pub from: Marker,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Marker>,
    /// oneCell/absolute 의 크기 (EMU)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<(i64, i64)>,
    /// absolute 의 위치 (EMU)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos: Option<(i64, i64)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum AnchorKind {
    #[default]
    TwoCell,
    OneCell,
    Absolute,
}

/// 칸 기준 위치: 0부터 시작하는 열·행 + 그 칸 안의 오프셋 (EMU, 1 pt = 12700)
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Marker {
    pub col: u32,
    pub col_off: i64,
    pub row: u32,
    pub row_off: i64,
}

/// `a:xfrm` (EMU, 회전은 1/60000 도)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Xfrm {
    pub x: i64,
    pub y: i64,
    pub cx: i64,
    pub cy: i64,
    pub rot: i64,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// 채우기
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Paint {
    None,
    Solid(DrawColor),
    /// 그라데이션·무늬·그림 채우기 (요소 이름)
    Other(String),
}

/// DrawingML 색: `srgb:RRGGBB`, `scheme:accent1`, `sys:windowText`, `prst:red` + 변형(`shade` 등, 1/1000 %)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DrawColor {
    pub value: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mods: Vec<(String, i64)>,
}

/// 선 (`a:ln`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Outline {
    /// 두께 (EMU)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dash: Option<String>,
    /// 선 끝 `cap` (`rnd`/`sq`/`flat`), 겹선 `cmpd` (`sng`/`dbl`/`thickThin`/`thinThick`/`tri`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compound: Option<String>,
    /// 이음 (`round`/`bevel`/`miter`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
    /// 화살표 `a:headEnd`·`a:tailEnd`: (종류, 너비, 길이)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<LineEnd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail: Option<LineEnd>,
}

/// 선 끝 화살표 (`a:headEnd`/`a:tailEnd`): 종류 `none`/`triangle`/`stealth`/`diamond`/`oval`/`arrow`, 너비·길이
/// `sm`/`med`/`lg`
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LineEnd {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub len: Option<String>,
}

/// 바깥 그림자 (`a:outerShdw`): 거리·흐림 EMU, 방향 1/60000 도
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ShapeShadow {
    pub dist: i64,
    pub dir: i64,
    pub blur: i64,
    /// `algn` (`ctr`, `t`, `b` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    pub rot_with_shape: bool,
    pub color: DrawColor,
}

/// 도형 안 글 (`xdr:txBody`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TextBody {
    /// 세로 맞춤 `t`/`ctr`/`b`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert: Option<String>,
    /// 안쪽 여백 왼·위·오른·아래 (EMU)
    pub insets: [Option<i64>; 4],
    /// 글 넘침 `vertOverflow`·`horzOverflow` (`overflow`/`ellipsis`/`clip`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert_overflow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horz_overflow: Option<String>,
    pub paragraphs: Vec<DrawingPara>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DrawingPara {
    /// `l`/`ctr`/`r`/`just`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    pub runs: Vec<DrawingRun>,
}

/// 글 조각. 줄 바꿈(`a:br`)은 텍스트 "\n".
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DrawingRun {
    pub text: String,
    /// 1/100 pt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<DrawColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ea: Option<String>,
}

impl Shape {
    /// 글상자 글자 전체 (문단은 줄 바꿈으로)
    pub fn plain_text(&self) -> String {
        self.text
            .as_ref()
            .map(|t| {
                t.paragraphs
                    .iter()
                    .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }
}

/// 차트 (`c:chartSpace`).
///
/// `kind`·`three_d`·`bar_dir`·`grouping`·`vary_colors`·`series` 는 그림 영역의 첫 차트, `legend` 는 범례
/// 위치다. `title`·`cat_axis_title`·`val_axis_title` 과 계열의 `name` … `color` 는 뷰어·비교용 요약으로
/// 자세한 필드에서 뽑은 것이다 ([`Chart::refresh_summary`]). 나머지는 한셀이 쓰는 차트 파트를 그대로 다시
/// 쓰기 위한 값이다 ([`crate::chart_xml`]): 한셀은 읽은 차트 요소의 있고 없음을 그대로 다시 쓰므로
/// (`hs/*.hs.cell` 대조) 값이 `None` 이면 그 요소를 쓰지 않는다. 한셀이 레거시 차트를 읽어 채우는 값은
/// `docs/format/cell-2024.md` 11절.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Chart {
    /// 첫 그림 영역 차트의 종류: `bar`, `line`, `pie`, `doughnut`, `area`, `scatter`, `radar`, `bubble` …
    pub kind: String,
    /// 3차원 (`bar3DChart` 등)
    pub three_d: bool,
    /// 막대 방향 `col`(세로)/`bar`(가로)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_dir: Option<String>,
    /// `clustered`/`stacked`/`percentStacked`/`standard`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 범례 위치 `r`/`l`/`t`/`b`/`tr` (없으면 범례 없음)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legend: Option<String>,
    pub vary_colors: bool,
    pub series: Vec<ChartSeries>,
    /// 축 제목 (항목 축, 값 축)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cat_axis_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub val_axis_title: Option<String>,

    /// `c:date1904`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date1904: Option<bool>,
    /// `c:roundedCorners`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounded_corners: Option<bool>,
    /// `c:style` (차트 스타일 번호). `c14_style` 이 있으면 `mc:Fallback` 안에 쓴다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<u32>,
    /// `mc:AlternateContent` → `mc:Choice Requires="c14"` 의 `c14:style`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c14_style: Option<u32>,
    /// `c:title` 요소 전체. `title` 은 이 글을 이어 붙인 것이다 (`autoTitleDeleted` 면 `None`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_full: Option<ChartTitle>,
    /// `c:autoTitleDeleted`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_title_deleted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view3d: Option<ChartView3D>,
    /// `c:floor` / `c:sideWall` / `c:backWall`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<ChartSurface>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side_wall: Option<ChartSurface>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub back_wall: Option<ChartSurface>,
    /// 그림 영역의 `c:layout`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plot_layout: Option<ChartLayout>,
    /// 첫 그림 영역 차트의 묶음 수준 값 (`kind`·`series` 와 같은 차트)
    #[serde(skip_serializing_if = "ChartGroupProps::is_empty")]
    pub group: ChartGroupProps,
    /// 그림 영역의 둘째 이후 차트 (혼합 차트)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub more_groups: Vec<ChartGroup>,
    /// 축 (그림 영역 순서)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<ChartAxis>,
    /// 그림 영역의 `c:spPr`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plot_sp_pr: Option<ChartShapeProps>,
    /// 범례의 위치 밖 값. `legend` 가 있을 때만 쓴다
    #[serde(skip_serializing_if = "ChartLegendProps::is_empty")]
    pub legend_props: ChartLegendProps,
    /// `c:plotVisOnly`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plot_vis_only: Option<bool>,
    /// `c:dispBlanksAs` (`gap`/`zero`/`span`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disp_blanks_as: Option<String>,
    /// 차트 공간의 `c:txPr` (기본 글꼴). 한셀은 `c:spPr` 앞에 쓴다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_pr: Option<ChartRich>,
    /// 차트 공간의 `c:spPr` (차트 영역의 채우기·테두리)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
    /// `c:extLst` 의 한컴 확장 `ho:hncChartStyle`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hnc_chart_style: Option<HncChartStyle>,
    /// 레거시(VtChart) 레코드에서 읽은 차트의 화면 정보 (렌더러용 — 쓰기·비교에는 쓰지 않는다)
    #[serde(skip)]
    pub legacy: Option<LegacyChartView>,
}

/// 레거시 차트의 화면 정보. 한셀은 레거시 문서에서 읽은 차트를 XML 차트와 다른 규칙(VtChart 화면)으로 그린다 —
/// 눈금이 촘촘하고, 범례 표지가 크고 틀이 없고, 원형은 계열 목록의 색을 쓴다 (docs/format/screen.md 6절).
/// 모델(XML)에 옮기지 않는 저장값을 함께 둔다. 문서 내용이 아니므로 비교(`==`)에서는 늘 같다
#[derive(Debug, Clone, Default)]
pub struct LegacyChartView {
    /// `VtChart.chartType`
    pub chart_type: u16,
    /// 계열 목록(VtSeries)마다 채우기 색 `RRGGBB` (원형·도넛은 조각마다)
    pub fills: Vec<String>,
    /// 계열 목록마다 표식이 자동인가 (`VtSeries.u16_d` 1 — 모델에는 `none` 으로 옮기지만 한셀 화면은 자동 표식을 그린다)
    pub auto_markers: Vec<bool>,
    /// 3차원 회전·높이 (도, `VtChartPlot.rotation`·`elevation`)
    pub rotation: f64,
    pub elevation: f64,
}

impl PartialEq for LegacyChartView {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartSeries {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 이름·항목·값의 셀 참조 (`시트!$A$1:$A$5`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cat_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub val_ref: Option<String>,
    /// 저장된 값 (캐시)
    pub categories: Vec<String>,
    pub values: Vec<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<DrawColor>,

    /// `c:idx` / `c:order`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    /// `c:tx` — 이름 참조(`strRef` + 캐시) 또는 글(`c:v`). `name`·`name_ref` 는 여기서 뽑은 것
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx: Option<ChartTx>,
    /// `c:spPr` (채우기·선)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
    /// `c:invertIfNegative` (막대·거품)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invert_if_negative: Option<bool>,
    /// `c:marker` (꺾은선·분산형·방사형)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<ChartMarker>,
    /// `c:explosion` (원형, %)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explosion: Option<u32>,
    /// `c:dPt` — 요소 하나만 다르게 꾸민 것
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub d_pts: Vec<ChartDataPoint>,
    /// `c:dLbls`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d_lbls: Option<ChartDataLabels>,
    /// 항목 `c:cat` (분산형·거품형은 `c:xVal`). `cat_ref`·`categories` 는 여기서 뽑은 것
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cat: Option<ChartData>,
    /// 값 `c:val` (분산형·거품형은 `c:yVal`). `val_ref`·`values` 는 여기서 뽑은 것
    #[serde(skip_serializing_if = "Option::is_none")]
    pub val: Option<ChartData>,
    /// `c:bubbleSize` (거품형)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bubble_size: Option<ChartData>,
    /// `c:shape` (3차원 막대 계열)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// `c:bubble3D` (거품형 계열)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bubble3d: Option<bool>,
    /// `c:smooth` (꺾은선·분산형 계열)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smooth: Option<bool>,
}

/// 그림 영역의 둘째 이후 차트 하나 (`c:barChart` …). 첫 차트는 [`Chart`] 의 앞 필드와 `group` 이다.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartGroup {
    /// [`Chart::kind`] 와 같은 이름 (`ofPie` 는 `pie` + `props.of_pie_type`)
    pub kind: String,
    pub three_d: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<String>,
    pub vary_colors: bool,
    #[serde(skip_serializing_if = "ChartGroupProps::is_empty")]
    pub props: ChartGroupProps,
    pub series: Vec<ChartSeries>,
}

/// 차트 묶음(`c:barChart` 등) 수준의 값. 차트 종류에 맞는 것만 채운다.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartGroupProps {
    /// `c:radarStyle` (`standard`/`marker`/`filled`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radar_style: Option<String>,
    /// `c:scatterStyle` (`lineMarker`/`line`/`marker`/`smooth`/`smoothMarker`/`none`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scatter_style: Option<String>,
    /// `c:ofPieType` (`pie`/`bar`) — 있으면 `c:ofPieChart`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub of_pie_type: Option<String>,
    /// `c:wireframe` (표면형)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wireframe: Option<bool>,
    /// 묶음 수준 `c:dLbls`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d_lbls: Option<ChartDataLabels>,
    /// `c:gapWidth` (%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_width: Option<u32>,
    /// `c:overlap` (%, −100..100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlap: Option<i32>,
    /// `c:splitType` / `c:splitPos` / `c:secondPieSize` (원형 대 원형)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_pos: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub second_pie_size: Option<u32>,
    /// `c:serLines` (계열 연결선)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ser_lines: Option<ChartLines>,
    /// `c:dropLines` / `c:hiLowLines`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_lines: Option<ChartLines>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hi_low_lines: Option<ChartLines>,
    /// `c:gapDepth` (3차원, %)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_depth: Option<u32>,
    /// `c:shape` (3차원 막대: `box`/`cylinder`/`cone`/`pyramid` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// 꺾은선 묶음의 `c:marker val` / `c:smooth val`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smooth: Option<bool>,
    /// `c:firstSliceAng` (원형·도넛, 도)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_slice_ang: Option<u32>,
    /// `c:holeSize` (도넛, %)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hole_size: Option<u32>,
    /// 거품형: `c:bubble3D`, `c:bubbleScale` (%), `c:showNegBubbles`, `c:sizeRepresents` (`area`/`w`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bubble3d: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bubble_scale: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_neg_bubbles: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_represents: Option<String>,
    /// `c:axId` (이 묶음이 쓰는 축 번호, 순서대로)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ax_ids: Vec<u32>,
}

impl ChartGroupProps {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl Chart {
    /// 요약 필드(`title`, 축 제목, 계열의 `name` … `color`)를 자세한 필드에서 다시 만든다.
    /// 파서와 같은 규칙이다. 모델을 직접 만들 때(레거시 차트 변환) 쓴다.
    pub fn refresh_summary(&mut self) {
        let deleted = self.auto_title_deleted == Some(true);
        self.title = self
            .title_full
            .as_ref()
            .and_then(ChartTitle::plain_text)
            .filter(|_| !deleted);
        let axis_title = |kind: &str| {
            self.axes
                .iter()
                .find(|a| a.kind == kind)
                .and_then(|a| a.title.as_ref())
                .and_then(ChartTitle::plain_text)
        };
        let cat = axis_title("catAx").or_else(|| axis_title("dateAx"));
        let val = axis_title("valAx");
        self.cat_axis_title = cat;
        self.val_axis_title = val;
        for s in self.series.iter_mut().chain(
            self.more_groups
                .iter_mut()
                .flat_map(|g| g.series.iter_mut()),
        ) {
            s.refresh_summary();
        }
    }
}

impl ChartSeries {
    /// `name`·`name_ref`·`cat_ref`·`val_ref`·`categories`·`values`·`color` 를 `tx`·`cat`·`val`·`sp_pr` 에서
    /// 다시 만든다 (파서와 같은 규칙).
    pub fn refresh_summary(&mut self) {
        (self.name, self.name_ref) = match &self.tx {
            Some(ChartTx::Ref(d)) => (d.dense().into_iter().flatten().next(), d.formula.clone()),
            Some(ChartTx::Value(v)) => (Some(v.clone()), None),
            _ => (None, None),
        };
        self.cat_ref = self.cat.as_ref().and_then(|d| d.formula.clone());
        self.categories = self.cat.as_ref().map_or_else(Vec::new, |d| {
            d.dense()
                .into_iter()
                .map(Option::unwrap_or_default)
                .collect()
        });
        self.val_ref = self.val.as_ref().and_then(|d| d.formula.clone());
        self.values = self.val.as_ref().map_or_else(Vec::new, |d| {
            d.dense()
                .into_iter()
                .map(|x| x.and_then(|t| t.trim().parse().ok()))
                .collect()
        });
        self.color = match self.sp_pr.as_ref().and_then(|p| p.fill.as_ref()) {
            Some(ChartFill::Solid(c)) if !c.value.is_empty() => Some(c.to_draw()),
            _ => None,
        };
    }
}

/// 축 (`c:catAx`/`c:valAx`/`c:serAx`/`c:dateAx`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartAxis {
    /// 요소 이름 `catAx`/`valAx`/`serAx`/`dateAx`
    pub kind: String,
    /// `c:axId`
    pub id: u32,
    /// `c:scaling` (한셀은 늘 쓴다)
    pub scaling: ChartScaling,
    /// `c:delete` (축 숨김)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete: Option<bool>,
    /// `c:axPos` (`b`/`l`/`r`/`t`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ax_pos: Option<String>,
    /// `c:majorGridlines` / `c:minorGridlines` (주·보조 눈금선)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_gridlines: Option<ChartLines>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_gridlines: Option<ChartLines>,
    /// 축 제목 `c:title`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<ChartTitle>,
    /// `c:numFmt`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_fmt: Option<ChartNumFmt>,
    /// `c:majorTickMark` / `c:minorTickMark` (`out`/`in`/`cross`/`none`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_tick_mark: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_tick_mark: Option<String>,
    /// `c:tickLblPos` (`nextTo`/`low`/`high`/`none`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_lbl_pos: Option<String>,
    /// 축 선 `c:spPr`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
    /// 눈금 레이블 글꼴 `c:txPr`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_pr: Option<ChartRich>,
    /// `c:crossAx` (교차하는 축의 번호)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_ax: Option<u32>,
    /// `c:crosses` (`autoZero`/`min`/`max`) 또는 `c:crossesAt`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crosses: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crosses_at: Option<f64>,
    /// 값 축 `c:crossBetween` (`between`/`midCat`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_between: Option<String>,
    /// 항목 축: `c:auto`, `c:lblAlgn`, `c:lblOffset`, `c:tickLblSkip`, `c:tickMarkSkip`, `c:noMultiLvlLbl`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lbl_algn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lbl_offset: Option<u32>,
    /// 날짜 축 `c:baseTimeUnit` / `c:majorTimeUnit` / `c:minorTimeUnit` (`days`/`months`/`years`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_time_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_lbl_skip: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_mark_skip: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_multi_lvl_lbl: Option<bool>,
    /// `c:majorUnit` / `c:minorUnit` (값·날짜 축 눈금 간격)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_unit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_time_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_unit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_time_unit: Option<String>,
}

/// `c:scaling`
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartScaling {
    /// `c:logBase` (로그 눈금의 밑)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_base: Option<f64>,
    /// `c:orientation` (`minMax`/`maxMin` = 거꾸로)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    /// `c:max` / `c:min`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
}

/// `c:numFmt` (축·데이터 레이블의 표시 형식)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartNumFmt {
    pub format_code: String,
    /// `sourceLinked` — 원본 셀의 표시 형식을 따른다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_linked: Option<bool>,
}

/// 선 요소 (`c:majorGridlines`, `c:dropLines`, `c:hiLowLines`, `c:serLines`). 비어 있으면 `<c:majorGridlines/>`
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartLines {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
}

/// 차트·축 제목 (`c:title`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartTitle {
    /// `c:tx` — 서식 있는 글(`c:rich`) 또는 셀 참조(`c:strRef`). 없으면 자동 제목
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx: Option<ChartTx>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<ChartLayout>,
    /// `c:overlay` (그림 영역과 겹침)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_pr: Option<ChartRich>,
}

impl ChartTitle {
    /// 제목 글 (문단은 줄 바꿈으로, 비었으면 없음). 참조면 저장된 첫 값.
    pub fn plain_text(&self) -> Option<String> {
        match self.tx.as_ref()? {
            ChartTx::Rich(r) => Some(r.plain_text()).filter(|s| !s.is_empty()),
            ChartTx::Ref(d) => d.dense().into_iter().flatten().next(),
            ChartTx::Value(_) => None,
        }
    }
}

/// 범례의 위치 밖 값 (`c:legend`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartLegendProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<ChartLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_pr: Option<ChartRich>,
}

impl ChartLegendProps {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `c:layout`. `manual` 이 없으면 `<c:layout/>` (자동 배치)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartLayout {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<ChartManualLayout>,
}

/// `c:manualLayout`. 위치·크기는 차트 크기에 대한 비율
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartManualLayout {
    /// `c:layoutTarget` (`inner`/`outer`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_target: Option<String>,
    /// `c:xMode` … `c:hMode` (`edge`/`factor`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
}

/// `c:view3D` (각도는 도)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartView3D {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r_ang_ax: Option<bool>,
    /// `c:rotX` (−90..90)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rot_x: Option<i32>,
    /// `c:hPercent` (%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_percent: Option<u32>,
    /// `c:rotY` (0..360)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rot_y: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perspective: Option<u32>,
    /// `c:depthPercent` (%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_percent: Option<u32>,
}

/// `c:floor` / `c:sideWall` / `c:backWall`
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartSurface {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
}

/// 한컴 차트 스타일 (`c:extLst/c:ext/ho:hncChartStyle`, 번호 −1 = 지정 안 함)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HncChartStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_index: Option<i32>,
}

/// 계열 표식 (`c:marker`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartMarker {
    /// `c:symbol` (`none`/`square`/`circle`/`diamond`/`triangle`/`x`/`star`/`dash`/`dot`/`plus`/`auto`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// `c:size` (2..72 pt)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
}

/// 데이터 요소 하나의 서식 (`c:dPt`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartDataPoint {
    pub idx: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invert_if_negative: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<ChartMarker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bubble3d: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explosion: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
}

/// 데이터 레이블 (`c:dLbls`). 한셀은 `c:delete` 와 표시 여부들을 함께 쓴다
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartDataLabels {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_fmt: Option<ChartNumFmt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_pr: Option<ChartShapeProps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_pr: Option<ChartRich>,
    /// `c:dLblPos` (`outEnd`/`inEnd`/`ctr`/`inBase`/`t`/`b`/`l`/`r`/`bestFit`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d_lbl_pos: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_legend_key: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_val: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_cat_name: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_ser_name: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_percent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_bubble_size: Option<bool>,
    /// `c:separator`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_leader_lines: Option<bool>,
}

/// 계열 이름·제목의 글 (`c:tx`)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ChartTx {
    /// `c:rich` (제목)
    Rich(ChartRich),
    /// `c:strRef` (셀 참조 + 캐시)
    Ref(ChartData),
    /// `c:v` (계열 이름 글자)
    Value(String),
}

/// 계열의 데이터 원본 (`c:cat`, `c:val`, `c:xVal`, `c:yVal`, `c:bubbleSize` 안, `c:tx` 의 `c:strRef`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartData {
    /// 요소 이름 `numRef`/`strRef`/`numLit`/`strLit`
    pub kind: String,
    /// `c:f` (참조일 때)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// 저장된 값 (`c:numCache`/`c:strCache`, 리터럴이면 그 내용)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<ChartCache>,
}

impl ChartData {
    /// 저장된 값을 번호 자리에 (`ptCount` 만큼, 없는 점은 None). 참조·리터럴 네 종류만
    pub fn dense(&self) -> Vec<Option<String>> {
        let Some(c) = self.cache.as_ref().filter(|_| {
            matches!(
                self.kind.as_str(),
                "numRef" | "strRef" | "numLit" | "strLit"
            )
        }) else {
            return Vec::new();
        };
        let mut v = vec![None; c.pt_count.unwrap_or(0) as usize];
        for p in &c.points {
            let i = p.idx as usize;
            if i >= v.len() {
                v.resize(i + 1, None);
            }
            v[i] = Some(p.value.clone());
        }
        v
    }
}

/// `c:numCache` / `c:strCache` (또는 `c:numLit`/`c:strLit` 의 내용)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartCache {
    /// `c:formatCode` (숫자만)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pt_count: Option<u32>,
    pub points: Vec<ChartPoint>,
}

/// 저장된 값 하나 (`c:pt`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartPoint {
    pub idx: u32,
    /// 점마다의 `formatCode` 속성 (한셀은 값 캐시의 점마다 쓴다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_code: Option<String>,
    /// `c:v` 원문
    pub value: String,
}

/// 도형 속성 (`c:spPr`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartShapeProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<ChartFill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<ChartLine>,
}

/// 채우기 (`a:noFill`/`a:solidFill`, 그 밖은 원본 XML)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ChartFill {
    None,
    Solid(ChartColor),
    /// `a:gradFill`/`a:pattFill`/`a:blipFill`/`a:grpFill` 원본 XML
    Other(String),
}

/// DrawingML 색. `value` 는 [`DrawColor::value`] 와 같은 꼴 (`srgb:RRGGBB` 대문자, `scheme:accent1`,
/// `sys:windowText`, `prst:black`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartColor {
    pub value: String,
    /// `a:sysClr/@lastClr`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_clr: Option<String>,
    /// 변형 (`lumMod` 등, 1/1000 %)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mods: Vec<(String, i64)>,
}

impl ChartColor {
    /// `srgb:RRGGBB` (16진 6자리, 대소문자 무관)
    pub fn srgb(hex: &str) -> Self {
        ChartColor {
            value: format!("srgb:{}", hex.to_ascii_uppercase()),
            ..Default::default()
        }
    }

    pub fn to_draw(&self) -> DrawColor {
        DrawColor {
            value: self.value.clone(),
            mods: self.mods.clone(),
        }
    }
}

/// 선 (`a:ln`). 한셀이 레거시 차트를 읽어 쓰는 선은 끝 모양까지 모두 채운다 ([`ChartLine::hcell`])
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartLine {
    /// `w` (EMU)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    /// `cap` (`flat`/`rnd`/`sq`), `cmpd` (`sng` …), `algn` (`ctr`/`in`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmpd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<ChartFill>,
    /// `a:prstDash/@val` (`solid`/`dash`/`sysDot` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dash: Option<String>,
    /// 꺾임 모양 `round`/`bevel`/`miter` (요소 이름)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_end: Option<ChartLineEnd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_end: Option<ChartLineEnd>,
}

impl ChartLine {
    /// 한셀이 레거시 차트를 읽어 쓰는 꼴의 선: `cap="flat" cmpd="sng" algn="ctr"` + 실선, 둥근 꺾임,
    /// 양 끝 `w="med" len="med"` (`*.legacy.as.cell` 77개의 a:ln 347개 모두, docs/format/cell-2024.md 11절)
    pub fn hcell(width: i64, fill: ChartFill) -> Self {
        let end = ChartLineEnd {
            kind: None,
            w: Some("med".into()),
            len: Some("med".into()),
        };
        ChartLine {
            width: Some(width),
            cap: Some("flat".into()),
            cmpd: Some("sng".into()),
            algn: Some("ctr".into()),
            fill: Some(fill),
            dash: Some("solid".into()),
            join: Some("round".into()),
            head_end: Some(end.clone()),
            tail_end: Some(end),
        }
    }
}

/// 선 끝 (`a:headEnd`/`a:tailEnd`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartLineEnd {
    /// `type` (`none`/`triangle`/`arrow` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// `w` / `len` (`sm`/`med`/`lg`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub len: Option<String>,
}

/// 차트의 글 (`c:rich`, `c:txPr`). `a:bodyPr` 의 나머지 속성(`wrap="none"`, 안쪽 여백 0, `anchor="ctr"`,
/// `anchorCtr="1"`)은 한셀이 늘 같은 값으로 쓰므로 담지 않는다
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartRich {
    /// `a:bodyPr/@rot` (1/60000 도)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rot: Option<i64>,
    /// `a:bodyPr/@vert` (`horz`/`vert`/`vert270` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert: Option<String>,
    pub paras: Vec<ChartPara>,
}

impl ChartRich {
    /// 글 전체 (문단은 줄 바꿈으로)
    pub fn plain_text(&self) -> String {
        self.paras
            .iter()
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 한셀이 레거시 차트를 읽어 쓰는 꼴의 글 상자: `rot="0" vert="horz"` (세로 글은 `vert="vert270"` 만),
    /// 왼쪽 맞춤(`algn="l"`) 문단 하나에 `defRPr` = 글꼴. 글이 있으면 같은 글꼴의 조각 하나
    /// (`c:rich`), 없으면 글꼴만 (`c:txPr`). 근거: `*.legacy.as.cell` 의 제목·축·범례 글 전부
    pub fn hcell(font: ChartFont, text: Option<&str>, vertical: bool) -> Self {
        ChartRich {
            rot: (!vertical).then_some(0),
            vert: Some(if vertical { "vert270" } else { "horz" }.into()),
            paras: vec![ChartPara {
                align: Some("l".into()),
                runs: text
                    .map(|t| ChartRun {
                        font: Some(font.clone()),
                        text: t.to_string(),
                    })
                    .into_iter()
                    .collect(),
                def_font: Some(font),
            }],
        }
    }
}

impl ChartFont {
    /// 한셀이 레거시 차트를 읽어 쓰는 꼴의 글꼴: `sz b i u="none"`, 단색 RGB, `latin`·`ea`·`cs`·`sym` 이 같은 이름
    pub fn hcell(size_pt100: u32, bold: bool, italic: bool, rgb: &str, face: &str) -> Self {
        ChartFont {
            size: Some(size_pt100),
            bold: Some(bold),
            italic: Some(italic),
            underline: Some("none".into()),
            fill: Some(ChartFill::Solid(ChartColor::srgb(rgb))),
            latin: Some(face.into()),
            ea: Some(face.into()),
            cs: Some(face.into()),
            sym: Some(face.into()),
            ..Default::default()
        }
    }
}

/// 문단 (`a:p`). 한셀은 끝에 빈 `a:endParaRPr` 를 쓴다
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartPara {
    /// `a:pPr/@algn`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    /// `a:pPr/a:defRPr`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub def_font: Option<ChartFont>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<ChartRun>,
}

/// 글 조각 (`a:r`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartRun {
    /// `a:rPr`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<ChartFont>,
    pub text: String,
}

/// 글꼴 (`a:defRPr`/`a:rPr`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChartFont {
    /// `sz` (1/100 pt)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// `u` (`none`/`sng`/`dbl` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    /// `strike` (`noStrike`/`sngStrike`/`dblStrike`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<String>,
    /// `baseline` (1/1000 %, 위·아래 첨자)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<ChartFill>,
    /// `a:latin`/`a:ea`/`a:cs`/`a:sym` 의 `typeface`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ea: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sym: Option<String>,
}

/// 원본 그대로 옮겨 쓰는 파트 (차트·그림 등, XML 원본에서만)
#[derive(Debug, Clone, Default)]
pub struct RawPart {
    pub name: String,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

/// 메모 (`xl/commentsN.xml` + VML, 레거시 NOTE/OBJ/TXO, XLSB BrtBeginComment)
#[derive(Debug, Clone, Default, Serialize)]
pub struct Comment {
    pub cell: CellRef,
    pub author: String,
    pub text: RichText,
    /// 항상 표시 (VML `x:Visible`, 레거시 NOTE 0x0002)
    pub visible: bool,
    /// 메모 상자의 자리와 모양. 없으면 기본 자리(셀 오른쪽 위)에 쓴다 ([`CommentShape::implied`])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<CommentShape>,
    /// 메모 도형 이름 (VML `v:shape/@id`, 레거시 OfficeArt 0x0380 — 한셀은 "메모 N")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_name: Option<String>,
    /// 메모 도형의 그리기 차례 (VML 도형 차례, 레거시 OBJ 차례)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_order: Option<u32>,
}

/// 메모 상자: VML `v:shape` + `x:ClientData` (XML·바이너리), 레거시는 OfficeArt 도형(앵커·채우기·선·그림자)과
/// TXO 의 맞춤. 한셀이 다시 쓰는 값만 담는다 (메모 대조 실험 — `margin-left` 같은 style 위치, `z-index`,
/// `x:AutoFill` 은 한셀이 버리거나 새로 쓴다)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommentShape {
    /// `x:Anchor`: 왼쪽 열, 오프셋(px), 위 행, 오프셋, 오른쪽 열, 오프셋, 아래 행, 오프셋
    pub anchor: [u32; 8],
    /// 채우기 색 `#rrggbb` (`fillcolor`)
    pub fill_color: String,
    /// 선 색 `#rrggbb` (`strokecolor`)
    pub stroke_color: String,
    /// 선 두께 (pt). VML 기본은 1 pt, 한셀의 메모는 0.75 pt
    pub stroke_weight: f64,
    /// 그림자 (`v:shadow on="t"`)
    pub shadow: bool,
    /// 셀과 함께 옮김·크기 바꿈. VML 은 반대로 `x:MoveWithCells`·`x:SizeWithCells` 가 있으면 고정이다
    /// (레거시 앵커 플래그 0 = 둘 다, 2 = 옮김만, 3 = 고정)
    pub move_with_cells: bool,
    pub size_with_cells: bool,
    /// `x:TextHAlign` (없으면 `Left`): `Center`, `Right`, `Justify`, `Distributed`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_halign: Option<String>,
    /// `x:TextVAlign` (없으면 `Top`): `Center`, `Bottom`, `Justify`, `Distributed`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_valign: Option<String>,
    /// 도형 이름 (VML `v:shape/@id`, 레거시 OfficeArt 이름 0x0380). 한셀은 세 형식에서 지킨다 (메모 이름 실험).
    /// 없으면 쓸 때 시트 안 순서로 `메모 N` ([`CommentShape::name_or`])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl CommentShape {
    /// 한셀이 새 메모에 쓰는 모양 (COM 으로 만든 메모: 노란 채우기, 0.75 pt 검정 선, 셀에 고정)
    pub fn hcell_default(anchor: [u32; 8]) -> CommentShape {
        CommentShape {
            anchor,
            fill_color: "#ffffe1".into(),
            stroke_color: "#000000".into(),
            stroke_weight: 0.75,
            shadow: false,
            move_with_cells: false,
            size_with_cells: false,
            text_halign: None,
            text_valign: None,
            name: None,
        }
    }

    /// 쓸 이름: 모델의 이름, 없으면 시트 안 `k`번째(0부터) 메모의 `메모 k+1` — rcell 이 예전에 쓰던 자동 번호
    /// (`_x0000_s{1024n+k+1}`)를 한셀이 읽어 붙이는 이름과 같다
    pub fn name_or(&self, k: usize) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("메모 {}", k + 1))
    }

    /// 한셀이 VML `v:shape/@id` 를 읽어 붙이는 메모 이름 (메모 이름 실험): 자동 번호 꼴 `_x0000_sN` 은 `메모 N%1024`
    /// (`_x0000_s1025` → `메모 1`, `_x0000_s2050` → `메모 2`), id 가 없으면 `메모 0`, 나머지(`메모 7`, `foo`,
    /// `Comment 3`)는 그대로 — 같은 이름이 둘이어도 그대로
    pub fn hcell_name_of_vml_id(id: Option<&str>) -> String {
        match id {
            None => "메모 0".into(),
            Some(v) => match v
                .strip_prefix("_x0000_s")
                .and_then(|n| n.parse::<u64>().ok())
            {
                Some(n) => format!("메모 {}", n % 1024),
                None => v.to_string(),
            },
        }
    }

    /// 모양이 없는 메모를 rcell 이 쓰는 자리: 셀 오른쪽 위 (Excel 기본 자리)
    pub fn implied(cell: CellRef) -> CommentShape {
        let (row, col) = (cell.row, cell.col);
        let top = row.saturating_sub(1);
        CommentShape::hcell_default([col + 1, 15, top, 10, col + 3, 15, top + 4, 4])
    }
}

/// 시트 보기 (`sheetView`, 레거시 WINDOW2/SCL/SELECTION/PANE, XLSB BrtBeginWsView/BrtSel/BrtPane).
/// 눈금선 표시는 [`Sheet::show_grid_lines`] 에 있다.
#[derive(Debug, Clone, Serialize)]
pub struct SheetView {
    pub show_row_col_headers: bool,
    pub show_zeros: bool,
    pub show_formulas: bool,
    pub tab_selected: bool,
    /// `normal` / `pageBreakPreview` / `pageLayout`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_left_cell: Option<String>,
    /// 현재 확대율 (%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom_scale: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom_scale_normal: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom_scale_sheet_layout_view: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<Pane>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_cell: Option<String>,
    /// 선택 영역 (공백으로 구분한 범위 목록)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqref: Option<String>,
}

impl Default for SheetView {
    fn default() -> Self {
        SheetView {
            show_row_col_headers: true,
            show_zeros: true,
            show_formulas: false,
            tab_selected: false,
            view: None,
            top_left_cell: None,
            zoom_scale: None,
            zoom_scale_normal: None,
            zoom_scale_sheet_layout_view: None,
            pane: None,
            active_cell: None,
            sqref: None,
        }
    }
}

/// 틀 고정·나누기 (`pane`)
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pane {
    /// 고정이면 열 수, 나누기면 1/20 pt
    pub x_split: f64,
    pub y_split: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_left_cell: Option<String>,
    /// `bottomRight` / `topRight` / `bottomLeft` / `topLeft`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_pane: Option<String>,
    /// `frozen` / `frozenSplit` / `split`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// 인쇄 설정 (`sheetPr/pageSetUpPr`, `printOptions`, `pageMargins`, `pageSetup`, `headerFooter`)
#[derive(Debug, Clone, Default, Serialize)]
pub struct PageLayout {
    /// 자동 페이지 나누기 표시 (기본 참)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_page_breaks: Option<bool>,
    /// 페이지에 맞추기 (기본 거짓)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_to_page: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print_options: Option<PrintOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margins: Option<PageMargins>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<PageSetup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_footer: Option<HeaderFooter>,
    /// 수동 페이지 나누기: 이 행(0부터) 위 / 이 열 왼쪽
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub row_breaks: Vec<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub col_breaks: Vec<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PrintOptions {
    pub horizontal_centered: bool,
    pub vertical_centered: bool,
    pub headings: bool,
    pub grid_lines: bool,
}

/// 여백 (인치)
#[derive(Debug, Clone, Default, Serialize)]
pub struct PageMargins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub header: f64,
    pub footer: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PageSetup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_page_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_to_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_to_height: Option<u32>,
    /// `default` / `portrait` / `landscape`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    /// `downThenOver` / `overThenDown`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_order: Option<String>,
    pub use_printer_defaults: bool,
    pub black_and_white: bool,
    pub draft: bool,
    pub use_first_page_number: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal_dpi: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_dpi: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copies: Option<u32>,
}

/// 머리글·바닥글. 내용은 `&C`, `&P` 같은 코드가 섞인 원문 (한셀은 `&[SHEET]`, `&[PAGE NUMBER]` 도 쓴다).
#[derive(Debug, Clone, Default, Serialize)]
pub struct HeaderFooter {
    pub different_odd_even: bool,
    pub different_first: bool,
    pub scale_with_doc: bool,
    pub align_with_margins: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub odd_header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub odd_footer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub even_header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub even_footer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_footer: Option<String>,
}

impl Sheet {
    pub fn cell(&self, r: CellRef) -> Option<&Cell> {
        self.cells
            .binary_search_by(|c| (c.r.row, c.r.col).cmp(&(r.row, r.col)))
            .ok()
            .map(|i| &self.cells[i])
    }

    /// 셀 레코드가 없는 칸의 서식: 행 서식 → 열 서식 → 0 (0부터 시작하는 좌표).
    pub fn effective_style(&self, row: u32, col: u32) -> u32 {
        self.rows
            .iter()
            .find(|r| r.index == row + 1)
            .and_then(|r| r.style)
            .or_else(|| {
                self.cols
                    .iter()
                    .find(|c| (c.min..=c.max).contains(&(col + 1)))
                    .and_then(|c| c.style)
            })
            .unwrap_or(0)
    }

    /// 값이 있는 셀이 차지하는 범위 (빈 서식 셀 제외).
    pub fn used_range(&self) -> Option<CellRange> {
        let mut it = self
            .cells
            .iter()
            .filter(|c| !c.value.is_empty() || c.formula.is_some());
        let first = it.next()?;
        let (mut r0, mut c0, mut r1, mut c1) = (first.r.row, first.r.col, first.r.row, first.r.col);
        for c in it {
            r0 = r0.min(c.r.row);
            c0 = c0.min(c.r.col);
            r1 = r1.max(c.r.row);
            c1 = c1.max(c.r.col);
        }
        Some(CellRange {
            start: CellRef { row: r0, col: c0 },
            end: CellRef { row: r1, col: c1 },
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ColInfo {
    /// 1부터 시작하는 열 번호 범위
    pub min: u32,
    pub max: u32,
    pub width: Option<f64>,
    pub style: Option<u32>,
    pub hidden: bool,
    pub custom_width: bool,
    /// 개요(그룹) 수준 0–7, 접힘 표시
    #[serde(skip_serializing_if = "is_zero")]
    pub outline_level: u8,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RowInfo {
    /// 1부터 시작하는 행 번호
    pub index: u32,
    pub height: Option<f64>,
    pub custom_height: bool,
    pub hidden: bool,
    pub style: Option<u32>,
    #[serde(skip_serializing_if = "is_zero")]
    pub outline_level: u8,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

/// 자동 필터 (`autoFilter`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AutoFilter {
    pub range: String,
    pub columns: Vec<FilterColumn>,
}

/// 자동 필터의 열 하나 (`filterColumn`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FilterColumn {
    /// 필터 범위 안의 열 번호 (0부터)
    pub col_id: u32,
    /// 값 목록 필터
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// 값 목록에 빈 칸 포함
    pub blank: bool,
    /// 사용자 지정 조건 (연산자, 값) 최대 2개, `and` 결합 여부
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<(String, String)>,
    pub custom_and: bool,
    /// 상위/하위: (위, 백분율, 값)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top10: Option<(bool, bool, f64)>,
    /// 상위/하위의 기준값 (`top10/@filterVal` — 필터를 적용할 때 계산한 경계값)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_val: Option<f64>,
    /// 동적 필터 종류 (`aboveAverage`, `today` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<String>,
    /// 동적 필터의 `val`(평균 또는 기간 시작 일련번호)·`maxVal`(기간 끝)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_val: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_max_val: Option<f64>,
}

/// 시트 보호 (`sheetProtection`). 속성은 원문 그대로 (기본값 해석은 비교 쪽에서).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SheetProtection {
    /// 레거시 해시(16비트 16진) 암호
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// `sheet`, `objects`, `formatCells` … → 참/거짓
    pub flags: Vec<(String, bool)>,
}

/// `CT_SheetProtection` 의 16개 항목과 기본값. 참 = 그 동작이 막혀 있음.
/// 순서는 XLSB `BrtSheetProtection` 의 필드 순서이자, `sheet` 를 뺀 나머지는
/// BIFF8 `EnhancedProtection`(FEATHDR) 비트 순서다.
pub const SHEET_PROTECTION_FLAGS: [(&str, bool); 16] = [
    ("sheet", false),
    ("objects", false),
    ("scenarios", false),
    ("formatCells", true),
    ("formatColumns", true),
    ("formatRows", true),
    ("insertColumns", true),
    ("insertRows", true),
    ("insertHyperlinks", true),
    ("deleteColumns", true),
    ("deleteRows", true),
    ("selectLockedCells", false),
    ("sort", true),
    ("autoFilter", true),
    ("pivotTables", true),
    ("selectUnlockedCells", false),
];

impl SheetProtection {
    /// 16개 항목의 상태(참 = 막힘, [`SHEET_PROTECTION_FLAGS`] 순서)에서 만든다. 기본값과 같은 항목은 생략.
    pub fn from_states(password: Option<String>, states: [bool; 16]) -> SheetProtection {
        SheetProtection {
            password,
            flags: SHEET_PROTECTION_FLAGS
                .iter()
                .zip(states)
                .filter(|((_, d), v)| v != d)
                .map(|((k, _), v)| (k.to_string(), v))
                .collect(),
        }
    }

    /// 항목 하나의 실제 상태 (없으면 기본값)
    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().find(|(n, _)| n == name).map_or_else(
            || {
                SHEET_PROTECTION_FLAGS
                    .iter()
                    .find(|(k, _)| *k == name)
                    .is_some_and(|(_, d)| *d)
            },
            |(_, v)| *v,
        )
    }
}

/// 동적 필터 종류. 색인 = XLSB `BrtDynamicFilter` / BIFF8 `AUTOFILTER12` 의 cft 값 (빈 자리 = 쓰지 않음)
pub const DYNAMIC_FILTER_TYPES: [&str; 40] = [
    "null",
    "aboveAverage",
    "belowAverage",
    "",
    "",
    "",
    "",
    "",
    "tomorrow",
    "today",
    "yesterday",
    "nextWeek",
    "thisWeek",
    "lastWeek",
    "nextMonth",
    "thisMonth",
    "lastMonth",
    "nextQuarter",
    "thisQuarter",
    "lastQuarter",
    "nextYear",
    "thisYear",
    "lastYear",
    "yearToDate",
    "Q1",
    "Q2",
    "Q3",
    "Q4",
    "M1",
    "M2",
    "M3",
    "M4",
    "M5",
    "M6",
    "M7",
    "M8",
    "M9",
    "M10",
    "M11",
    "M12",
];

/// 자동 필터 사용자 지정 조건의 연산자. 색인 = BIFF8/XLSB grbitSign (1–6)
pub const FILTER_OPERATORS: [&str; 7] = [
    "",
    "lessThan",
    "equal",
    "lessThanOrEqual",
    "greaterThan",
    "notEqual",
    "greaterThanOrEqual",
];

/// 문서 속성: docProps/core.xml (OPC Core Properties), 레거시는 `SummaryInformation` 속성 집합.
/// 날짜는 W3CDTF 원문 (`1997-03-01T10:49:08Z`, 한셀은 `…08.000` 처럼 시간대 없이 쓴다 — UTC).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DocProperties {
    pub title: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
    pub keywords: Option<String>,
    pub description: Option<String>,
    pub last_modified_by: Option<String>,
    pub revision: Option<String>,
    pub category: Option<String>,
    pub content_status: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub last_printed: Option<String>,
}

impl DocProperties {
    pub fn is_empty(&self) -> bool {
        *self == DocProperties::default()
    }
}

/// 통합 문서 보호 (`workbookProtection`)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WorkbookProtection {
    pub lock_structure: bool,
    pub lock_windows: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// 0부터 시작하는 셀 좌표.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    /// `B12`, `$B$12` 형식을 해석한다.
    pub fn parse(a1: &str) -> Option<CellRef> {
        let s = a1.trim().replace('$', "");
        let split = s.find(|c: char| c.is_ascii_digit())?;
        let (letters, digits) = s.split_at(split);
        if letters.is_empty() || letters.len() > 3 {
            return None;
        }
        let mut col: u32 = 0;
        for ch in letters.chars() {
            if !ch.is_ascii_alphabetic() {
                return None;
            }
            col = col * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
        }
        let row: u32 = digits.parse().ok()?;
        if row == 0 {
            return None;
        }
        Some(CellRef {
            row: row - 1,
            col: col - 1,
        })
    }

    pub fn col_name(col: u32) -> String {
        let mut n = col + 1;
        let mut out = Vec::new();
        while n > 0 {
            let rem = (n - 1) % 26;
            out.push((b'A' + rem as u8) as char);
            n = (n - 1) / 26;
        }
        out.iter().rev().collect()
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", CellRef::col_name(self.col), self.row + 1)
    }
}

impl Serialize for CellRef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRange {
    pub start: CellRef,
    pub end: CellRef,
}

impl CellRange {
    /// `A1:C3` 또는 단일 셀 `A1` 을 해석한다.
    pub fn parse(s: &str) -> Option<CellRange> {
        match s.split_once(':') {
            Some((a, b)) => Some(CellRange {
                start: CellRef::parse(a)?,
                end: CellRef::parse(b)?,
            }),
            None => {
                let r = CellRef::parse(s)?;
                Some(CellRange { start: r, end: r })
            }
        }
    }

    pub fn contains(&self, r: CellRef) -> bool {
        (self.start.row..=self.end.row).contains(&r.row)
            && (self.start.col..=self.end.col).contains(&r.col)
    }
}

impl fmt::Display for CellRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}:{}", self.start, self.end)
        }
    }
}

impl Serialize for CellRange {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Cell {
    pub r: CellRef,
    /// `cellXfs` 인덱스
    pub style: u32,
    pub value: CellValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<Formula>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum CellValue {
    #[default]
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    /// `#DIV/0!` 등
    Error(String),
    /// `t="d"` ISO 8601 원문
    Date(String),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        matches!(self, CellValue::Empty)
    }

    /// 서식 없이 사람이 읽을 수 있는 값. 숫자는 유효숫자 15자리 일반 서식.
    pub fn display(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Number(n) => format_general(*n),
            CellValue::Text(s) | CellValue::Error(s) | CellValue::Date(s) => s.clone(),
            CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        }
    }
}

/// 스프레드시트 '일반' 서식과 같은 유효숫자 15자리 표기.
pub fn format_general(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let rounded: f64 = format!("{v:.14e}").parse().unwrap_or(v);
    format!("{rounded}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FormulaKind {
    #[default]
    Normal,
    Shared,
    Array,
    DataTable,
}

/// 수식. 배열 수식(`t="array"`)과 데이터 표(`t="dataTable"`)는 범위의 왼쪽 위 셀에만 있고, 범위 안 다른 셀은 값만 갖는다
/// (레거시 ARRAY 0x0221·TABLE 0x0236, 바이너리 BrtArrFmla·BrtTable 도 같은 모델로 읽는다)
#[derive(Debug, Clone, Default, Serialize)]
pub struct Formula {
    /// `=` 없는 수식 원문. 공유 수식의 종속 셀도 읽을 때 기준 셀 원문을 옮겨 채운다.
    pub text: String,
    pub kind: FormulaKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_index: Option<u32>,
    /// 공유·배열 수식, 데이터 표의 범위 (`ref`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    /// 데이터 표의 입력 (`t="dataTable"`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_table: Option<DataTableInput>,
}

/// 데이터 표 (What-If 분석)의 입력: `dt2D`, `dtr`, `r1`, `r2`, `del1`, `del2`
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DataTableInput {
    /// 2변수 표 (`dt2D`) — 이때 `r1` 은 행 입력, `r2` 는 열 입력이다
    pub two_d: bool,
    /// 1변수 표의 입력이 행 입력 셀 (`dtr`). 한셀은 2변수 표에는 쓰지 않는다
    pub row_input: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r2: Option<String>,
    /// 입력 셀이 지워짐 (`del1`, `del2`) — 한셀은 버린다 (수식 대조 실험)
    pub del1: bool,
    pub del2: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RichText {
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<TextRun>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TextRun {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<Font>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DefinedName {
    pub name: String,
    /// 시트 범위 이름이면 그 시트 번호 (0부터)
    pub local_sheet_id: Option<u32>,
    pub hidden: bool,
    pub formula: String,
    /// 설명 (`comment`, 레거시 NAME 의 설명 글, 바이너리 BrtName 의 comment)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Hyperlink {
    pub range: String,
    /// 문서 내부 위치 (`'시트'!A1`)
    pub location: Option<String>,
    /// 외부 대상 (관계로 연결된 URL)
    pub target: Option<String>,
    pub tooltip: Option<String>,
    pub display: Option<String>,
}

// ---------------------------------------------------------------------------
// 스타일
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct Styles {
    pub num_fmts: Vec<NumFmt>,
    pub fonts: Vec<Font>,
    pub fills: Vec<Fill>,
    pub borders: Vec<Border>,
    pub cell_style_xfs: Vec<Xf>,
    pub cell_xfs: Vec<Xf>,
    pub cell_styles: Vec<NamedStyle>,
    /// 조건부 서식 등에 쓰이는 차등 서식
    pub dxfs: Vec<Dxf>,
    /// 표·피벗 표 스타일 (`x:tableStyles`). 한셀은 사용자 정의 스타일이 없어도 기본 스타일 이름을 쓴다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_styles: Option<TableStyles>,
    /// 문단 번호 정의 (`x:abstractNums`, 한컴 확장 — 문단 모양의 `abstractNumId` 가 가리킨다)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub abstract_nums: Vec<AbstractNum>,
    /// 레거시 XF 표를 그대로 읽은 것 (레거시 읽기가 켠다): 스타일 XF 가 레거시의 상용구(0–20)를 담고 셀 서식의
    /// 적용 비트·테두리 색이 저장된 그대로다. 꺼져 있으면(XML 등) 레거시 쓰기가 한셀이 XML 을 열 때처럼 서식 표를
    /// 고쳐 쓴다 (`docs/format/cell-legacy.md` 10절)
    #[serde(skip)]
    pub from_legacy: bool,
}

/// `x:tableStyles`
#[derive(Debug, Clone, Default, Serialize)]
pub struct TableStyles {
    pub default_table_style: Option<String>,
    pub default_pivot_style: Option<String>,
    pub styles: Vec<TableStyle>,
}

/// `x:tableStyle` — 표의 부분(머리글 행, 줄무늬 …)마다 차등 서식 하나
#[derive(Debug, Clone, Default, Serialize)]
pub struct TableStyle {
    pub name: String,
    /// 피벗 표에 쓸 수 있다 (기본 참)
    pub pivot: bool,
    /// 표에 쓸 수 있다 (기본 참)
    pub table: bool,
    pub elements: Vec<TableStyleElement>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TableStyleElement {
    /// ST_TableStyleType (`wholeTable`, `headerRow`, `firstRowStripe` …)
    pub kind: String,
    /// 줄무늬 폭 (행·열 수, 기본 1)
    pub size: u32,
    pub dxf_id: Option<u32>,
}

/// 표 스타일 요소 종류 — 바이너리·레거시 레코드의 번호 순서 ([MS-XLSB] BrtTableStyleElement,
/// [MS-XLS] TableStyleElement 의 tseType. 한셀 확인: 표 스타일 대조 실험)
pub const TABLE_STYLE_TYPES: [&str; 28] = [
    "wholeTable",
    "headerRow",
    "totalRow",
    "firstColumn",
    "lastColumn",
    "firstRowStripe",
    "secondRowStripe",
    "firstColumnStripe",
    "secondColumnStripe",
    "firstHeaderCell",
    "lastHeaderCell",
    "firstTotalCell",
    "lastTotalCell",
    "firstSubtotalColumn",
    "secondSubtotalColumn",
    "thirdSubtotalColumn",
    "firstSubtotalRow",
    "secondSubtotalRow",
    "thirdSubtotalRow",
    "blankRow",
    "firstColumnSubheading",
    "secondColumnSubheading",
    "thirdColumnSubheading",
    "firstRowSubheading",
    "secondRowSubheading",
    "thirdRowSubheading",
    "pageFieldLabels",
    "pageFieldValues",
];

/// `x:abstractNum` (한컴 확장). 한셀은 문단 모양에 번호가 있으면 저장할 때 이 목록을 새로 만든다
#[derive(Debug, Clone, Default, Serialize)]
pub struct AbstractNum {
    pub num_type: u32,
    pub id: u32,
    pub start_num: u32,
    /// 번호 모양, XML 에 쓰는 그대로 (`^` 를 `%5e` 로 쓴다: `%5e1.` = 1수준 번호 뒤 마침표)
    pub lvl_text: String,
}

/// `x:dxf` — 차등 서식. 지정된 항목만 덮어쓴다.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Dxf {
    pub font: Option<Font>,
    pub fill: Option<Fill>,
    pub border: Option<Border>,
    pub num_fmt: Option<NumFmt>,
    pub alignment: Option<Alignment>,
    /// `x:protection` (한셀 XML 은 값 없이 요소만 쓴다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<Protection>,
    /// `x:dxf hs:applyExtension="1"` 등
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hancom: Option<HancomXfExt>,
}

impl Styles {
    pub fn xf(&self, index: u32) -> Option<&Xf> {
        self.cell_xfs.get(index as usize)
    }

    pub fn font_of(&self, xf_index: u32) -> Option<&Font> {
        self.xf(xf_index)
            .and_then(|x| self.fonts.get(x.font_id as usize))
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NumFmt {
    pub id: u32,
    pub code: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Color {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rgb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Font {
    pub name: Option<String>,
    pub size: Option<f64>,
    pub bold: bool,
    pub italic: bool,
    /// `single`, `double`, ... (`x:u` 의 val, 생략 시 single)
    pub underline: Option<String>,
    pub strike: bool,
    /// `x:dstrike` — 이중 취소선. ECMA SpreadsheetML 에는 없고 한셀이 SML 네임스페이스에 쓰는 요소
    /// (HncOOXML.dll `CFont::InitMap` 에서 확인)
    pub double_strike: bool,
    /// `x:outline` · `x:shadow` · `x:condense` · `x:extend` (ECMA 글꼴 효과 — Excel 등 다른 프로그램이 읽는 쪽.
    /// 한셀은 한컴 확장의 외곽선·그림자 종류와 함께 쓴다)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub outline: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shadow: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub condense: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub extend: bool,
    pub color: Option<Color>,
    pub vert_align: Option<String>,
    pub family: Option<u32>,
    pub charset: Option<u32>,
    pub scheme: Option<String>,
    /// 한컴 글꼴 확장 (`x:font hs:extension="1"` 의 `hs:*` 자식)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hancom: Option<HancomFontExt>,
}

/// 한컴 글꼴 확장. 한글 워드프로세서의 '글자 모양'에 해당하는 속성들.
///
/// 퍼센트 값은 샘플에서 기본값 100(크기·장평), 0(자간·위치)으로 관측되었다.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HancomFontExt {
    /// `hs:size` — 상대 크기 (%)
    pub size: Option<i32>,
    /// `hs:ratio` — 장평 (%)
    pub ratio: Option<i32>,
    /// `hs:spacing` — 자간 (%)
    pub spacing: Option<i32>,
    /// `hs:offset` — 글자 위치 (%)
    pub offset: Option<i32>,
    pub underline_type: Option<String>,
    pub underline_shape: Option<String>,
    pub underline_color: Option<Color>,
    pub strikeout_type: Option<String>,
    pub strikeout_shape: Option<String>,
    pub strikeout_color: Option<Color>,
    pub outline_type: Option<String>,
    pub shadow_type: Option<String>,
    pub shadow_color: Option<Color>,
    pub shadow_offset_x: Option<i32>,
    pub shadow_offset_y: Option<i32>,
    pub shade_color: Option<Color>,
    pub emboss: Option<bool>,
    pub engrave: Option<bool>,
    pub superscript: Option<bool>,
    pub subscript: Option<bool>,
    pub use_font_space: Option<bool>,
    pub use_kerning: Option<bool>,
    /// `hs:diacsymMark` — 강조점
    pub diacsym_mark: Option<String>,
    /// 아직 모델링하지 않은 `hs:` 자식 (이름 → val)
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Fill {
    pub pattern_type: Option<String>,
    pub fg_color: Option<Color>,
    pub bg_color: Option<Color>,
    /// `x:gradientFill` (있으면 무늬 채우기 대신 쓴다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradient: Option<GradientFill>,
}

/// `x:gradientFill`: 직선(각도) 또는 경로(사각형 안쪽에서 바깥으로) 그라데이션
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GradientFill {
    /// `type="path"`
    pub path: bool,
    pub degree: f64,
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub stops: Vec<GradientStop>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GradientStop {
    pub position: f64,
    pub color: Color,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BorderEdge {
    pub style: Option<String>,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Border {
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
    pub diagonal: Option<BorderEdge>,
    /// 범위 안쪽 세로·가로 선 (`x:vertical`, `x:horizontal` — 차등 서식·표 스타일에서 쓴다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<BorderEdge>,
    pub diagonal_up: bool,
    pub diagonal_down: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Alignment {
    pub horizontal: Option<String>,
    pub vertical: Option<String>,
    pub wrap_text: bool,
    pub shrink_to_fit: bool,
    pub text_rotation: Option<i32>,
    pub indent: Option<u32>,
    pub reading_order: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Protection {
    pub locked: Option<bool>,
    pub hidden: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Xf {
    pub num_fmt_id: u32,
    pub font_id: u32,
    pub fill_id: u32,
    pub border_id: u32,
    pub xf_id: Option<u32>,
    pub apply_number_format: Option<bool>,
    pub apply_font: Option<bool>,
    pub apply_fill: Option<bool>,
    pub apply_border: Option<bool>,
    pub apply_alignment: Option<bool>,
    pub apply_protection: Option<bool>,
    pub alignment: Option<Alignment>,
    pub protection: Option<Protection>,
    /// 글자 접두(작은따옴표) — `quotePrefix`. 숫자처럼 보이는 글을 글로 둔다
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub quote_prefix: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hancom: Option<HancomXfExt>,
}

/// `x:xf` 의 한컴 확장.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HancomXfExt {
    /// `hs:applyExtension`
    pub apply_extension: bool,
    pub parashape: Option<ParaShape>,
    /// `hs:accountUnderline` 의 밑줄 모양 (export 심볼 `CAccountUnderline::Getshape`, 미관측)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_underline: Option<String>,
    /// `hs:accountUnderline` 등 아직 해석하지 않은 속성/자식
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, String>,
}

/// `hs:parashape` — 셀 안 텍스트의 문단 모양 (한글 워드프로세서의 '문단 모양').
#[derive(Debug, Clone, Default, Serialize)]
pub struct ParaShape {
    /// 값: [`crate::vocab::LINE_SPACING_TYPE`] (관측: `betweenLines`)
    pub line_spacing_type: Option<String>,
    pub line_spacing: Option<i32>,
    pub prev_spacing: Option<i32>,
    pub next_spacing: Option<i32>,
    /// 값: [`crate::vocab::LINE_BREAK`] (관측: `keepWord`)
    pub break_latin_word: Option<String>,
    /// 값: [`crate::vocab::LINE_BREAK`] (관측: `breakWord`)
    pub break_non_latin_word: Option<String>,
    /// 최소 공백 (%)
    pub condense: Option<i32>,
    /// 값: [`crate::vocab::TEXT_ALIGNMENT`] (관측: `baseLine`)
    pub text_alignment: Option<String>,
    pub font_line_height: Option<bool>,
    /// 왼쪽 여백. 한셀 들여쓰기 1단계 = 1134 (관측, HWPUNIT 1/7200 inch 로 4mm 추정)
    pub left_margin: Option<i32>,
    /// 오른쪽 여백. 오른쪽 정렬 + 들여쓰기에서 관측
    pub right_margin: Option<i32>,
    /// 첫 줄 들여쓰기/내어쓰기 (부호 있는 정수 — export 심볼 `Getindentation(int&)`)
    pub indentation: Option<i32>,
    /// 문단 번호 (export 심볼에서 확인, 미관측)
    pub abstract_num_id: Option<i32>,
    pub abstract_num_type: Option<i32>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NamedStyle {
    pub name: String,
    pub xf_id: u32,
    pub builtin_id: Option<u32>,
    /// 개요 수준 스타일(RowLevel_n / ColLevel_n, 내장 번호 1·2)의 수준. XML `iLevel`
    pub i_level: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_ref_round_trip() {
        for (s, row, col) in [
            ("A1", 0, 0),
            ("Z9", 8, 25),
            ("AA10", 9, 26),
            ("XFD1048576", 1048575, 16383),
        ] {
            let r = CellRef::parse(s).unwrap();
            assert_eq!((r.row, r.col), (row, col));
            assert_eq!(r.to_string(), s);
        }
        assert_eq!(CellRef::parse("$C$3").unwrap().to_string(), "C3");
        assert!(CellRef::parse("A0").is_none());
        assert!(CellRef::parse("12").is_none());
    }

    #[test]
    fn general_number_format() {
        assert_eq!(format_general(1.0), "1");
        assert_eq!(format_general(0.1 + 0.2), "0.3");
        assert_eq!(format_general(-2.5), "-2.5");
        assert_eq!(format_general(1e20), "100000000000000000000");
    }
}
