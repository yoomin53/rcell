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
    /// 문서 속성 (docProps/core.xml, 레거시 SummaryInformation)
    pub properties: DocProperties,
    pub file_version: Option<FileVersion>,
    pub date1904: bool,
    pub sheets: Vec<Sheet>,
    pub shared_strings: Vec<RichText>,
    pub styles: Styles,
    pub defined_names: Vec<DefinedName>,
    /// 원본 그대로 옮겨 쓰는 파트 (차트·그림과 그 관계 파일)
    #[serde(skip)]
    pub raw_parts: Vec<RawPart>,
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
}

/// 조건부 서식의 기준 값 (`cfvo`): `min`/`max`/`num`/`percent`/`percentile`/`formula`
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Cfvo {
    pub kind: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    pub show_value: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct IconSet {
    /// `3Arrows`, `3TrafficLights1`, `5Rating` …
    pub set: String,
    pub cfvos: Vec<Cfvo>,
    pub show_value: bool,
    pub reverse: bool,
    pub percent: bool,
}

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

/// 차트 (`c:chartSpace`). 그리기·비교·저장에 필요한 만큼만 담는다.
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
    /// 동적 필터 종류 (`aboveAverage`, `today` …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct Formula {
    /// `=` 없는 수식 원문. 공유 수식의 종속 셀도 읽을 때 기준 셀 원문을 옮겨 채운다.
    pub text: String,
    pub kind: FormulaKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
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
    pub local_sheet_id: Option<u32>,
    pub hidden: bool,
    pub formula: String,
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
}

/// `x:dxf` — 차등 서식. 지정된 항목만 덮어쓴다.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Dxf {
    pub font: Option<Font>,
    pub fill: Option<Fill>,
    pub border: Option<Border>,
    pub num_fmt: Option<NumFmt>,
    pub alignment: Option<Alignment>,
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
    pub gradient: bool,
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
