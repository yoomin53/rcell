<!--
  Copyright 2026 yoomin53 (https://github.com/yoomin53)
  SPDX-License-Identifier: Apache-2.0
-->

# 한셀 바이너리 OOXML `.cell` 형식 (xlCellb)

> 작성: yoomin53 · 대상: 한셀 2024 저장 형식 `xlCellb`(-2007). 구현: `src/binary/`

## 1. 요약

ZIP 패키지 구조는 XML `.cell` 과 같고, 파트 내용이 BIFF12 레코드다. 레코드 배치는
공개 명세 [MS-XLSB] (Excel `.xlsb`) 를 그대로 따른다 (관측).

| 파트 | 콘텐츠 타입 |
|---|---|
| `xl/workbook.bin` | `application/vnd.ms-excel.sheet.binary.macroEnabled.main` |
| `xl/worksheets/sheetN.bin` | `application/vnd.ms-excel.worksheet` |
| `xl/styles.bin` | `application/vnd.ms-excel.styles` |
| `xl/sharedStrings.bin` | `application/vnd.ms-excel.sharedStrings` |
| `xl/theme/theme1.xml` | 테마 (XML 그대로) |
| `xl/vbsProject.bin` | 한컴 매크로 프로젝트 — XML 과 같은 복합 문서 파트 (`application/vnd.hancom-office.vbsProject`, [cell-2024.md](cell-2024.md) 7.10절) |
| `xl/tables/tableN.bin` | 표 — 레코드 파트 (2절 표) |
| `xl/pivotTables/pivotTableN.bin`, `xl/pivotCache/pivotCacheDefinitionN.bin`·`pivotCacheRecordsN.bin` | 피벗 — 레코드 파트 (`application/vnd.ms-excel.pivotTable`·`pivotCacheDefinition`·`pivotCacheRecords`, 5절) |

## 2. 한셀 고유 사항 (관측)

| 항목 | 내용 |
|---|---|
| 한컴 확장 | **저장하지 않는다.** 장평·자간·그림자 종류·문단 모양 등 `hs:` 정보는 사라지고, 표준 비트(외곽선 등)만 남는다 |
| 저장 경로 | `BrtAbsPath15`(2071) 레코드에 저장한 폴더의 절대 경로가 들어간다 (개인정보 주의) |
| 다시 열기 | 한셀 2024 의 COM 자동화는 확장자가 `.cell` 인 이 형식을 열지 못하지만(RPC_E_SERVERFAULT), 같은 파일을 `.xlsb` 로 이름만 바꾸면 연다 (`tools/probe/bin-as.ps1`). 이 길은 한셀의 Excel 가져오기를 거쳐 문단 모양을 기본값으로 채우고, 차등 서식 글꼴에 검정을 넣고, 쓰이지 않는 사용자 정의 스타일·일부 이름을 버린다 — 바이너리 읽기의 정답은 같은 문서의 XML 저장본이고, 이 길은 바이너리 저장 검증(같은 길로 연 두 결과 비교)에 쓴다 |
| 미상 레코드 | 시트 끝의 643, 645, 660, 662 |
| 스타일 | `BrtFont`(43) grbit 0x02 기울임, 0x08 취소선, 0x10 외곽선, 0x20 그림자, 0x40 condense, 0x80 extend. `BrtXF`(47) 플래그 0x8000 = `quotePrefix`. `BrtStyle`(48) 의 `iLevel`(0xFF 없음) — 같은 (내장 번호, 수준)이 두 번째로 나오면 사용자 정의 스타일이다 |
| 채우기 | `BrtFill`(45): `fls u32, 무늬 색, 배경색 (BrtColor), 그라데이션 종류 u32 (0 직선, 1 경로), 각도·왼쪽·오른쪽·위·아래 f64, 정지점 수 u32, (BrtColor + 위치 f64) × n`. `fls` 0x28 이면 그라데이션 (그라데이션 대조 실험) |
| 표 스타일 | `BrtBeginTableStyles`(508: `cts u32` (사용자 정의 수), 기본 표·피벗 스타일 이름 XLWideString), `BrtBeginTableStyle`(510: 플래그 u16 — 0x02 피벗, 0x04 표, `ctse u32`, 이름), `BrtTableStyleElement`(512: `tseType u32, size u32, dxfId u32`), 끝 511·509 |
| 함수 번호 | `PtgFunc`/`PtgFuncVar` 는 [MS-XLSB] Ftab 그대로 (368–484 Excel 2007 함수, 150 CALL). 한컴 함수는 1904 SUBTOTALA (가변), 1907 CONVERTK (인자 4) — 함수 대조 실험 |
| 공유 수식 | `BrtShrFmla`(**427**, `RfX` + 수식)가 기준 셀 수식 레코드 **뒤에** 온다. 셀의 `PtgExp` 는 **행 u32 만** 담고 열은 `rgcb` 의 `PtgExtraCol`(u32)에 있다 ([MS-XLSB]) |
| 상대 3차원 참조 | 상대 위치 수식(공유 수식·조건부 서식·유효성)에서는 `PtgRef3d`/`PtgArea3d` 도 상대 플래그가 선 축을 기준 셀에서의 **오프셋**으로 쓴다 (RgceLocRel). 3차원 참조 대조 실험 (공유 수식을 넣어 만든 XML → 한셀 → 바이너리): D1:D6 `Sheet2!C1+1` → `5A 0000 00000000 FFFF`(열 −1), E2:E6 `SUM(Sheet2!B1:C2)` → 행 `FFFFF`(−1)·0, 열 −3·−2, 조건부 서식 B3:B8 `Sheet2!B3` → 0·0, 유효성 G4:G8 `Sheet2!B4` → 열 −5. 이름 정의는 A1 기준이라 오프셋과 좌표가 같다 |
| 이름 | `BrtName` 의 `fBuiltin`(0x20) 이면 `Print_Area` 같은 내장 이름 → XML 의 `_xlnm.Print_Area`. `itab` 은 시트 범위(0xFFFFFFFF 통합 문서), 수식 뒤 `comment`(XLNullableWideString)가 XML `comment` |
| 배열 수식 | 왼쪽 위 셀의 `BrtFmla*` 뒤에 `BrtArrFmla`(426: RfX, 플래그 u8, 수식). 범위 안 셀은 모두 `PtgExp`(왼쪽 위의 행 + PtgExtraCol 열) 수식 셀이다 (수식 대조 실험) |
| 하이퍼링크 | `BrtHLink`(494: RfX, 관계 ID XLNullableWideString, 위치·화면 설명·표시 글 XLWideString). 한셀은 관계 ID·위치가 없는 링크에 **앞 레코드의 값을 그대로 남긴다** (하이퍼링크 대조 실험: 위치만 있는 링크에 앞 링크의 `rId1`, 주소만 있는 링크에 앞 링크의 위치) — rcell 은 앞 레코드와 한쪽만 같은 값을 남은 값으로 보고 버린다 (주소·위치가 함께 바뀌는 링크는 그대로) |
| 표 | 표 파트 (시트 관계 `table`, 시트 끝 `BrtBeginListParts`(660) 안 `BrtListPart`(661: 관계 ID) 순서): `BrtBeginList`(343: RfX, 종류 u32 (0 worksheet, 1 xml, 2 queryTable), 표 번호, 머리글 행 수, 요약 행 수, 플래그 u32 (0x01 요약 행을 보인 적 있음), 차등 서식 6개 (머리글·데이터·요약·표 테두리·머리글 테두리·요약 테두리, 없으면 0xFFFFFFFF), 연결 번호, 이름·표시 이름·설명·셀 스타일 3개 XLNullableWideString), 시트와 같은 자동 필터 레코드(161…), `BrtBeginListCols`(345: 열 수), 열마다 `BrtBeginListCol`(347: 열 번호, 요약 함수 (0 없음, 1 average, 2 count, 3 countNums, 4 max, 5 min, 6 sum, 7 stdDev, 8 var, 9 custom), 차등 서식 3개 (머리글·데이터·요약), 쿼리 표 필드, 고유 이름·이름·요약 글·셀 스타일 3개)와 요약 행 사용자 수식 `BrtListTrFmla`(352: fArray u8 + CellParsedFormula), `BrtTableStyleClient`(513: 플래그 u16 — 0x01 첫 열, 0x02 끝 열, 0x04 행 줄무늬, 0x08 열 줄무늬 — 와 이름), `BrtEndList`(344). 한셀은 `totalsRowBorderDxfId` 를 늘 0 으로 쓴다 (표 대조 실험) |
| 구조적 참조 | `PtgList`(0x18 0x19): `ixti u16, 플래그 u16, 표 번호 u32, 첫 열 u16, 끝 열 u16`. 플래그 비트 0–1 열 (0 없음, 1 하나, 2 범위), 비트 2–6 행 종류 (0 기본(데이터), 0x01 `#All`, 0x02 `#Headers`, 0x04 `#Data`, 0x08 `#Totals`, 0x10 `#This Row`, 겹치면 합), 비트 10–11 토큰 클래스. `ixti` 는 표가 있는 시트의 XTI. 한셀이 XML 에 쓰는 글: `Table1[수량]`, `Table1[]`(열 없음), `Table1[[수량]:[단가]]`, `Table1[#All]`, `Table1[[#This Row],[수량]]`, 겹친 행 종류는 `#Data`·`#Headers`·`#Totals` 순 (`[[#Headers],[#Data],[수량]]` → `[[#Data],[#Headers],[수량]]`) |
| 데이터 표 | `BrtTable`(428: RfX, 입력 1 행·열 u32, 입력 2 행·열 u32, 플래그 u8 — 0x01 행 입력, 0x02 2변수)이 왼쪽 위 셀 **앞에** 온다. 표의 셀은 수식 없이 값 셀이다. 1변수 표의 입력 2 는 0 |
| 외부 연결 | `workbook.bin` 의 `BrtSupBookSrc`(355) = 관계 ID → `xl/externalLinks/externalLinkN.bin`: `BrtBeginSupBook`(360, `sbt u16` 0 = 외부 + 관계 ID → `externalLinkPath`), `BrtSupTabs`(359, `cTab u32` + 시트 이름들) |
| 통합 문서 | `BrtWbProp`(153): `플래그 u32 (0x01 1904 날짜 체계), dwThemeVersion u32, 코드 이름 XLWideString` (`workbookPr/@codeName`). `BrtBookView`(158, 29바이트): `xWn i32, yWn i32, dxWn u32, dyWn u32 (1/20 pt), iTabRatio u32, itabFirst u32, itabCur u32, 플래그 u8` — 0x08 가로 스크롤, 0x10 세로 스크롤, 0x20 시트 탭 ([MS-XLSB] 그대로, 통합 문서 창 대조 실험) |
| 시트 속성 | `BrtWsProp`(147): 플래그 3바이트(0x000001 자동 페이지 나누기, 0x000040 요약 행 아래, 0x000080 요약 열 오른쪽, 0x000100 페이지에 맞추기, 0x010000 필터 적용 중), `BrtColor` 탭 색(자동 = 없음), `rwSync`, `colSync`, 코드 이름 (없으면 한셀이 채운다) |
| 행 높이 | `BrtRowHdr` 의 `miyRw` 는 지정 높이(`fUnsynced` 0x2000)가 아니어도 그 행의 실제 높이다 — XML 의 `ht` (customHeight 없이). 버리면 한셀이 높이를 다시 계산해 그리기 앵커가 어긋난다 |
| 개요 | `BrtRowHdr` 플래그 비트 8–10 수준, 0x0800 접힘. `BrtColInfo` 플래그 비트 8–10 수준, 접힘은 [MS-XLSB] 의 0x1000 이 아니라 **0x2000** 에 쓴다 (rcell 은 둘 다 받는다) |
| 보호 | `BrtBookProtection`(534: `protpwdBook u16, protpwdRev u16, 플래그 u16` — 1 구조, 2 창), `BrtSheetProtection`(535: `protpwd u16` + u32 × 16 — `fLocked` 다음 15개는 **허용** 여부라 OOXML 속성과 반대) |
| 자동 필터 | `BrtBeginAFilter`(161: RfX) 안에 `BrtBeginFilterColumn`(163: `dwCol u32, 플래그 u16`) 마다: 값 목록 `BrtBeginFilters`(165: `fBlank u32, 달력 u32`) + `BrtFilter`(167: 문자열); 사용자 지정 `BrtBeginCustomFilters`(172: `fAnd u32`) + `BrtCustomFilter`(174: `vts u8` (4 수, 6 문자열), `grbitSign u8`, 값 8바이트, 문자열이면 이어서 XLWideString); 상위/하위 `BrtTop10Filter`(170: 플래그 u8 (1 위, 2 백분율, 4 기준값), 개수 f64, 기준값 f64); 동적 `BrtDynamicFilter`(171: `cft u32` — 레거시 AUTOFILTER12 와 같은 번호, 플래그 u8, `val` f64, `maxVal` f64 — 한셀은 플래그 바이트를 채우지 않고 쓰므로(같은 종류에 0x03·0x63·0xCC) 값이 0 이 아니면 있는 것으로 본다; 필터 값 대조 실험) |
| 시트 보기 | `BrtBeginWsView`(137) 는 **30바이트**: `flags u16, xlView u32, rwTop u32, colLeft u32, icvHdr u8, 예약 u8, ? u16, 현재 확대율 u16 (18), u16 (20), u16 (22), u16 (24), iWbkView u32`. flags: 0x02 수식, 0x04 눈금선, 0x08 머리글, 0x10 0 표시, 0x40 선택 |
| 틀·선택 | `BrtPane`(151): `x f64, y f64, rwTop u32, colLeft u32, pnnAct u32, flags u8 (0x01 고정, 0x02 나누기 없음)`. `BrtSel`(152): `pnn u32, rwAct u32, colAct u32, iRfx u32, crfx u32 + RfX × crfx` |
| 인쇄 | `BrtPrintOptions`(477, 0x01 가로 가운데, 0x02 세로 가운데, 0x04 머리글, 0x08 눈금선), `BrtMargins`(476, f64 × 6), `BrtPageSetup`(478, `paper, scale, res, vres, copies, pageStart, fitW, fitH u32, flags u16` — 0x02 가로), `BrtBeginHeaderFooter`(479, `flags u16` + 머리글·바닥글 6개), `BrtBeginRwBrks`(392)/`BrtBeginColBrks`(394) 안의 `BrtBrk`(396: `id, min, max, fMan, fPivot`) |
| 메모 | 시트 관계 `comments` → `xl/commentsN.bin`: `BrtCommentAuthor`(632, 작성자), `BrtBeginComment`(635: `iauthor u32` + RfX + GUID), `BrtCommentText`(637: RichStr). 표시 여부는 XML 과 같은 VML 파트 (`BrtLegacyDrawing` 551 = 관계 ID) |
| 조건부 서식 | `BrtBeginConditionalFormatting`(461: ccf, fPivot, UncheckedSqRfX), `BrtBeginCFRule`(463: 종류·틀·dxfId·우선순위·매개 변수·플래그·수식 길이 3개·strParam·CFParsedFormula), `BrtBeginColorScale`(469)/`BrtBeginDatabar`(467)/`BrtBeginIconSet`(465), `BrtCFVO`(471), `BrtColor`(564). 차등 서식은 styles.bin 의 `BrtDXF`(507) — XFProp 목록 (2.2절) |
| 데이터 유효성 | `BrtBeginDVals`(573) 안의 `BrtDVal`(64): 플래그 u32 (종류 4비트, 오류 모양 3비트, 0x100 빈 칸 허용, 0x200 목록 숨김, 0x40000 입력 메시지, 0x80000 오류 메시지, 연산자 비트 20–23), UncheckedSqRfX, 문자열 4개(오류 제목, 오류, 입력 제목, 입력), 수식 2개 |

### 2.1 한셀의 바이너리 저장이 잃거나 바꾸는 것 (조작한 XML → 한셀 → 바이너리, 관측)

- 조건부 서식의 차등 서식에서 표시 형식과 맞춤이 사라진다. 글꼴 이름(XFProp 0x18)은 깨진 문자열로 쓴다. "시작 문자" 규칙의 글자(`strParam`)를 수식의 다른 부분으로 잘못 쓴다.
- 요약 열 왼쪽(`summaryRight="0"`)을 잃는다 (`BrtWsProp` 0x80 을 언제나 켠다). 구조 잠금이 없는 통합 문서 보호
  (창 잠금·암호만)는 `BrtBookProtection` 째로 저장하지 않는다.
- 통합 문서 구조 잠금이 걸린 문서는 표준 암호화(ECMA-376, AES-128, 기본 암호 `VelvetSweatshop`)로 감싸 저장한다
  (시트 보호·창 잠금·통합 문서 암호만이면 감싸지 않는다: filter2·filter3, 창 잠금만 건 COM 문서) — rcell 은 투명하게
  풀고, 쓸 때도 똑같이 감싼다 (4.10절).
- 그림을 다시 부호화한다 (pictures: PNG 149 → 178 바이트, JPEG 155,595 → 146,356 바이트; XML 저장은 원본 그대로).
  조작한 XML 의 그림에는 `macro=""`·`fPublished="0"`·`rotWithShape="1"`·`fLocksWithSheet`·`fPrintsWithSheet` 를 채우고
  `a:xfrm` 의 위치·크기를 앵커에서 다시 계산한다.
- 외부 연결의 상태를 잃는다: 값 캐시(`BrtExternTableStart` … 를 쓰지 않아 `refreshError` 도 사라짐)와
  경로를 잃은 연결의 표시(관계 종류 `xlPathMissing` → 보통의 `externalLinkPath`). 한셀은 이 둘에 따라 이름 참조의
  따옴표를 정하므로, 바이너리를 거친 문서는 이름의 표기가 달라질 수 있다.

- 글꼴 없는 글 조각만 있는 메모(`<x:text><x:t>…</x:t></x:text>`, `<x:r>` 에 `x:rPr` 가 없는 것)는 `BrtCommentText` 를
  쓰지 않아 내용을 잃는다 (메모 대조 실험). 메모 상자는 XML 과 같은 VML 파트라 그대로다.
- 캐시 값(`<v>`)이 없는 수식 셀이 있으면 바이너리 저장이 실패한다 (COM `RPC_E_SERVERFAULT`). 한셀이 저장한 문서는
  늘 캐시 값이 있으므로 조작한 문서에서만 생긴다.
- `draft`, `autoPageBreaks="0"` 이 저장되지 않는다.
- 머리글·바닥글에서 글꼴 코드가 없는 구역에 기본 글꼴 코드(`&"맑은 고딕,Regular"`)를 넣는다.
- 여백을 HWPUNIT(1/7200 인치)로 잘라 쓴다.
- 나눈 창(고정 아님)의 활성 창 `bottomLeft` 를 잃는다 (`pnnAct` 0, 선택은 창 3).
- **보조 확대율**: `BrtBeginWsView` 의 (20)·(22) 와 XML 의 `zoomScaleNormal`·`zoomScaleSheetLayoutView`
  사이에 일관된 대응이 없다. XML 이 같은 두 시트가 바이너리에서 다른 값을 갖기도 한다 — 문서에 드러나지 않는
  상태가 있다. 한셀이 바이너리를 읽을 때는 (22) 를 `zoomScaleNormal`, (20) 을 `zoomScaleSheetLayoutView` 로 둔다
  (X.bin.as.cell 6쌍, [MS-XLSB] 이름과 반대) — rcell 도 그렇게 읽는다. 비교(`rcell diff`)에서는 현재 확대율만 본다.

### 2.2 차등 서식 XFProp (BrtDXF 507, 레거시 DXF 0x088D)

`flags u16 (0x8000 fNewBorder), 예약 u16, cprops u16` 뒤에 `종류 u16, cb u16 (머리 4바이트 포함), 값` 이 cprops 개.
번호는 [MS-XLSB] XFProp 그대로다 (차등 서식 대조 실험: 조작한 XML → 한셀 → 바이너리·레거시):

| 종류 | 값 | XML |
|---|---|---|
| 0x00 | 무늬 u8 (0x28 = 그라데이션) | `patternFill/@patternType` |
| 0x01 / 0x02 | XFPropColor (8바이트) | `fgColor` / `bgColor` |
| 0x03 | 종류 u32, 각도·왼쪽·오른쪽·위·아래 f64 | `gradientFill` |
| 0x04 | 예약 u16, 위치 f64, XFPropColor | `gradientFill/stop` |
| 0x05 | XFPropColor | `font/color` |
| 0x06–0x0C | XFPropColor + 선 종류 u16 — 위, 아래, 왼쪽, 오른쪽, 대각선, 안쪽 세로, 안쪽 가로 | `border/*`. 선 종류 0 은 '선 없음'으로 지정한 선 (`<x:left><x:color auto="1"/></x:left>`), 없는 선은 '그대로 둠' |
| 0x0D / 0x0E | u8 | `diagonalUp` / `diagonalDown` |
| 0x0F / 0x10 / 0x11 / 0x12 / 0x13 / 0x14 / 0x16 | 가로·세로 맞춤 u8 (XF 와 같은 번호), 회전 u8, 들여쓰기 u16, 읽기 순서 u8, 줄 바꿈 u8, 축소 u8 | `alignment` |
| 0x18 | 글꼴 이름 (u16 cch + UTF-16) | 한셀은 XML 에 쓰지 않는다 |
| 0x19 | 굵기 u16 (700 굵게) | `b` |
| 0x1A | 밑줄 u16 (1 한 줄, 2 두 줄, 0x21 회계, 0x22 이중 회계) | `u` |
| 0x1B | 첨자 u16 (1 위, 2 아래) | `vertAlign` |
| 0x1C / 0x1D / 0x1E / 0x1F / 0x20 / 0x21 | u8 | `i` / `strike` / `outline` / `shadow` / `condense` / `extend` |
| 0x22 / 0x23 / 0x25 | u8 | `charset` / `family` / `scheme` |
| 0x24 | 크기 u32 — 바이너리는 1/20 pt, **레거시는 1/100 pt** | `sz` |
| 0x2B / 0x2C | u8 | `protection/@locked` / `@hidden` |

XFPropColor 는 `fValidRGB 1비트 + 종류 7비트 (0 자동, 1 색인, 2 RGB, 3 테마), icv u8, 밝기 i16, RGBA`. 종류 0, 또는 값
`FF FF FF DF`(한셀의 자동 색 표지, 레거시는 RGB 종류로 쓴다)이면 자동이다. 한셀은 테마·색인 색을 RGB 로 바꿔 쓴다.

### 2.3 조건부 서식의 x14 확장 (x14 조건부 서식 실험)

본 규칙(`BrtBeginCFRule` 463 …)의 데이터 막대 뒤에 `BrtFRTBegin`(35) + `BrtCFRuleExt`(1146: FRTHeader u32 + GUID 16) +
`BrtFRTEnd` 가 오고, 시트 끝(인쇄 설정 뒤)의 `BrtFRTBegin` 안에 x14 구역이 있다:

```
BrtBeginConditionalFormattings 1135
  BrtBeginConditionalFormatting14 1046   FRTHeader (0x02 범위: cSqref u32, 플래그 u32, UncheckedSqRfX), ccf u32, fPivot u32
    BrtBeginCFRule14 1048                FRTHeader, iType, iTemplate, dxfId, iPri (본 규칙과 이어지면 -1), iParam, 예약 ×2 (u32),
                                         플래그 u16, cbFmla ×3 (u32), strParam, GUID 16, u32
      BrtBeginDatabar14 1051             FRTHeader, 최소 u8, 최대 u8, 값 표시 u8, 방향 u8 (0 context, 1 leftToRight, 2 rightToLeft),
                                         축 u8 (0 automatic, 1 middle, 2 none), 플래그 u8 (0x01 테두리, 0x02 그라데이션,
                                         0x04 음수 색 다름, 0x08 음수 테두리 색 다름), 예약 u8
        BrtCFVO14 1050 × 2               FRTHeader (0x04 수식: cFormula u32, (플래그 u32, cce u32, cb u32, rgce, rgcb) × n),
                                         iType u32, numParam f64, fSaveGTE u32, fGTE u32, cce u32
        BrtColor14 1055 × n              FRTHeader + BrtColor
      BrtEndDatabar14 1156
      BrtBeginIconSet14 1052             FRTHeader, 집합 i32, 플래그 u16 (0x01 사용자 지정, 0x02 아이콘만, 0x04 거꾸로)
        BrtCFVO14 1050 × n, BrtCFIcon 1112 × n (FRTHeader, 집합 i32 — -1 아이콘 없음, 번호 u32)
      BrtEndIconSet14 1155
    BrtEndCFRule14 1049
  BrtEndConditionalFormatting14 1047
BrtEndConditionalFormattings 1136
```

- 기준 값 종류(`BrtCFVO` 471, `BrtCFVO14`): 1 num, 2 min, 3 max, 4 percent, 5 percentile, 7 formula, 8 autoMin, 9 autoMax.
  `xm:f` 로 받은 값은 수식(PtgInt 등)으로 쓴다 — 본 규칙의 `BrtCFVO` 도 `cbFmla` 뒤 CFParsedFormula 에 담는다.
- 본 규칙의 `BrtBeginDatabar`(467)는 x14 확장이 있으면 늘 `0a 5a 01`(10/90)이다. 길이는 1051 에 있다.
- **색의 종류가 없다**: 한셀은 모델에 있는 색만 테두리·음수 채우기·음수 테두리·축 순서로 쓰고, 색 조합이 달라도 1051 이 같다
  (색 조합 실험: 같은 속성에 색 0–3개를 바꾼 막대 9개). rcell 은 넷이면 모두, 아니면 플래그로 쓰임새가 있는 자리를 고른다
  (`assign_bar_colors`). 한셀의 `.xlsb` 읽기는 셋을 테두리·음수 채우기·음수 테두리로 읽는다 — 저장본과 다르다.
- BrtColor14 의 RGB 색은 색인 바이트에 빨강 값이 들어 있고, 색인 색은 RGB 종류에 색인 바이트만 채운다 (`indexed="10"` →
  `05 0a 00 00 00 00 00 ff`). 자동 색은 검정과 같은 바이트다.
- 수식 규칙의 x14 규칙(1048 iType 2)은 수식도 서식도 쓰지 않는다 — 한셀의 바이너리 저장이 잃는다.

## 3. 검증

같은 문서를 XML 형식과 이 형식으로 함께 저장해(`tools/probe/make-corpus.ps1`) 비교한다.
한컴 확장을 뺀 나머지는 샘플 8종과 서식 파일 28종 모두 일치한다:

```
rcell diff X.bin.cell X.cell --ignore-hancom
```

쓰기(4절)의 검증 (`tests/write_binary.rs`, 코퍼스 114쌍):

| 검증 | 방법 | 결과 |
|---|---|---|
| 왕복 | XML·한셀 바이너리·레거시 원본 → rcell 바이너리 → rcell 읽기, `rcell diff` | 342/342 차이 0 |
| 레코드 대조 | rcell 이 `X.cell` 에서 쓴 바이너리 대 한셀의 `X.bin.cell` (`rcell bindiff`) | 한셀 레코드 47,382 중 바이트까지 같음 46,905 (98.99%, `exact`) · 46,881 (98.94%, 기본) |
| 한셀 읽기 | rcell 바이너리를 한셀이 .xlsb 로 열어 XML 저장 (`tools/probe/binw-oracle.ps1`) 대 `X.bin.as.cell`, `rcell diff --ignore-hancom` | `exact` 114/114 차이 0. 기본은 한셀의 바이너리 저장이 잃는 값(4.8절)이 있는 조작 문서 5개만 다르다 |

## 4. 쓰기 — 한셀이 쓰는 꼴 (`src/binary/writer.rs`)

근거: 코퍼스 114개 문서의 `X.cell`(한셀 XML 저장)과 `X.bin.cell`(같은 문서를 한셀이 -2007 로 저장)을 레코드마다
맞댄 것 (`rcell bindiff`, `tests/write_binary.rs`). 한셀은 두 형식을 같은 메모리 상태에서 저장하므로, XML 에서
알 수 있는 값은 바이너리에서 같아야 한다. 아래 "숨은 상태" 는 XML 에 드러나지 않아 정할 수 없는 칸이다.

### 4.1 패키지

| 항목 | 한셀 |
|---|---|
| ZIP | 파트를 이름 순으로, Deflate, 날짜 1980-01-01 |
| `[Content_Types].xml` | Default: `rels`, `xml`, 메모가 있으면 `vml`, 그림 확장자(`png`·`jpeg` …), **외부 연결 대상의 확장자**(`xls` → `application/vnd.ms-excel`). Override: 나머지 파트를 이름 순으로 (`xl/workbook.bin` = `…sheet.binary.macroEnabled.main`, 시트 `…ms-excel.worksheet`, `…styles`, `…sharedStrings`, `…comments`, `…externalLink`) |
| 루트 관계 | rId1 officeDocument, rId2 core-properties, rId3 extended-properties |
| 통합 문서 관계 | rId1 테마, rId2 styles.bin, rId3 sharedStrings.bin, rId4… 시트, 그 뒤 외부 연결 (`externalLinks/externalLinkN.bin`), 피벗 캐시, 매크로 프로젝트. 문자열이 하나도 없으면 sharedStrings.bin 과 그 관계·콘텐츠 타입이 없고 시트가 rId3 부터다 (3차원 참조 대조 실험의 숫자·수식만 있는 문서) |
| 매크로 프로젝트 | `xl/vbsProject.bin` 은 XML 저장본과 같은 복합 문서 (스트림 바이트가 같고 복합 문서의 저장 시각만 저장할 때마다 다르다), 관계 `http://schemas.hancom.com/office/relationships/vbsProject`, 콘텐츠 타입 `application/vnd.hancom-office.vbsProject` (패키지 파트 실험의 vbs.bin.cell). rcell 은 XML·바이너리 원본의 파트를 그대로, 레거시의 `_VBS_PROJECT_W` 는 스트림 하나짜리 복합 문서로 싸서 쓴다 — 레코드 대조에서 콘텐츠 타입·관계·나머지 레코드가 한셀과 같다. 한셀은 매크로가 든 바이너리를 다시 열지 못한다 (한셀 자신의 것도 rcell 의 것도 `.xlsb` 로 열면 E_FAIL) |
| 그 밖의 원본 파트 | 사용자 지정 XML 은 통합 문서 관계로, 프린터 설정은 시트 관계 `rIdP<n>` + `BrtPageSetup` 끝의 관계 ID 로 옮긴다 (한셀은 둘 다 세 형식에서 버린다). 연결·쿼리 표·XML 대응은 바이너리에서 레코드 파트라 옮기지 않는다 — 한셀도 버린다 |
| 시트 관계 | 하이퍼링크(외부), 그리기, VML, 표, 피벗 표, 메모 순 (한 시트에 다 넣은 COM 문서). 대상은 상대 경로 (`../drawings/drawing1.xml`) |
| 표 파트 | `xl/tables/tableN.bin` (`application/vnd.ms-excel.table`), N 은 문서 안 시트·표 순서로 1부터 (같은 세션의 XML 저장본과 같은 번호 — 세션 번호가 아니다). 레코드는 2절 표, 시트 끝의 `BrtBeginListParts`(660: 표 수) `BrtListPart`(661: 관계 ID)×n `BrtEndListParts`(662) |
| 외부 연결 파트 | `BrtBeginSupBook`(360: `sbt 0`, 관계 ID `rId1`, 빈 문자열 0xFFFFFFFF), `BrtSupTabs`(359), `BrtEndSupBook`(588). 관계는 경로를 잃은 연결도 보통의 `externalLinkPath` |
| 그리기·차트·그림·VML | XML 파트 그대로 (차트는 모델에서 `chart_xml` 로). 한셀은 그리기·차트·그림 파트에 **세션 전체의 번호**를 붙인다 (같은 문서의 `X.cell` 은 `drawing69.xml`, `X.bin.cell` 은 `drawing70.xml`) — 문서의 정보가 아니다. 메모 VML 은 XML 저장본과 같은 꼴 ([cell-2024.md](cell-2024.md) 7.5절: 이름, 시트 안 `o:spid`, 그리기 개체 수에서 이어지는 z-index, `idmap 0`, 세션 GUID `x:ShapeUri`) — rcell 은 메모 상자 모델(`CommentShape`, 이름 포함)에서 같은 꼴로 쓴다 (코퍼스 6개 문서의 VML 이 `x:ShapeUri` 말고 같다) |
| docProps | app.xml: `Application` Cell, 시트 이름 목록(`TitlesOfParts`), `TotalTime` 원본 값(서식 파일 51 → 51, 없으면 0), `AppVersion` 12.0300. core.xml 은 늘 쓰고 끝에 `cp:version` 1300.0100.01 (바이너리 113개 모두). 수정 번호(+1)·수정 날짜(저장 시각)는 저장할 때 바뀐다 — rcell 은 모델 값을 쓰고, 대조(`binary::parity`)와 `rcell diff` 는 이 둘과 수정한 사람을 뺀다 |

### 4.2 `workbook.bin`

`BrtBeginBook`, `BrtFileVersion`(guid 0, `HCell` / `13.0` / `13.0` / `0.711`), `BrtWbProp`(플래그 = 1904 날짜 체계만, 테마
버전 0, 코드 이름 `ThisWorkbook`), [`BrtBookProtection`], `BrtACBegin`(`01 00 03 0F 00 80`) `BrtAbsPath15`(저장 폴더,
끝에 `\`) `BrtACEnd`, `BrtBeginBookViews` `BrtBookView`(창 위치·크기·탭 비율·첫 탭·활성 탭, 플래그 0x38) `BrtEndBookViews`,
`BrtBeginBundleShs` `BrtBundleSh`×n (`hsState`, 시트 번호 1…, 관계 ID, 이름) `BrtEndBundleShs`, `BrtBeginExternals`
[연결들 + `BrtExternSheet`] `BrtEndExternals`, `BrtName`×n, `BrtCalcProp`(늘 `00000000 01000000 64000000` + 0.001 +
`00000000 2A00`), [피벗 캐시 목록 (5.1절)], `BrtEndBook`.

- **연결(SupBook) 순서**: 외부 연결은 `[n]` 번호 순서, 이 통합 문서(`BrtSupSelf`)는 쓰일 때만 첫 외부 연결 바로 뒤 —
  관측한 다섯 꼴 `[외부1, 자신]`, `[외부1, 자신, 외부2]`, `[외부1, 외부2]`, `[자신]`, 없음이 모두 이 규칙이다.
- **XTI**: 수식에 처음 나오는 순서 — 이름 정의, 시트마다 셀·조건부 서식·유효성, 그리고 **차트 계열의 참조**
  (`Sheet1!$A$1:$A$5`). 차트만 있는 문서도 `BrtSupSelf` + `BrtExternSheet` 가 있는 까닭이다 (차트 문서 68개).
  **표가 있는 시트**는 쓰이지 않아도 그 시트의 수식보다 먼저 XTI 를 받는다 (XTI 순서 실험: 시트 셋에 수식·표·이름을
  나눠 넣은 COM 문서 셋 — `S1` 에 표와 `=S3!A1+1`, `S2` 에 표 → `[S1, S3, S2]`; 이름 `'S1'!$A$1` 이 있으면 이름이 먼저).
  구조적 참조(PtgList)의 `ixti` 가 이 XTI 다. **피벗 캐시의 원본 시트**는 모든 시트의 수식 다음에 XTI 를 받는다 (5.1절).
- **이름**: 모델 순서 그대로, `_xlnm.` 은 떼고 `fBuiltin`(0x20). 수식에 쓰였지만 정의되지 않은 이름은 수식 없는
  이름(cce 0)으로 덧붙인다 (`=XYZ` → `BrtName XYZ`). 주석은 0xFFFFFFFF.
- 구조 잠금이 걸린 통합 문서는 ECMA-376 표준 암호화로 감싼다 (4.10절).

### 4.3 시트 (`sheetN.bin`)

`BrtBeginSheet`, `BrtWsProp`, `BrtWsDim`, `BrtBeginWsViews` `BrtBeginWsView` [`BrtPane`] `BrtSel` `BrtEndWsView`
`BrtEndWsViews`, `BrtWsFmtInfo`, [`BrtBeginColInfos` `BrtColInfo`×n `BrtEndColInfos`], `BrtBeginSheetData` (행·셀)
`BrtEndSheetData`, [`BrtSheetProtection`], [자동 필터], [병합], [조건부 서식]×n, [유효성], [`BrtHLink`×n],
`BrtPrintOptions`, `BrtMargins`, `BrtPageSetup`, [머리글·바닥글], [행 나누기], [열 나누기], [`BrtDrawing`],
[`BrtLegacyDrawing`], 643·645 (빈 ActiveX 묶음), 660(표 수)·[661 × 표]·662 (표 목록 — 표가 없어도 쓴다), [x14 조건부 서식
확장], `BrtEndSheet`.

| 레코드 | 한셀 |
|---|---|
| `BrtWsProp` | 1바이트째 0x08 을 늘 켠다(+ 0x01 자동 나누기, 0x40 요약 행 아래, 0x80 요약 열 오른쪽). 2바이트째 0x04 늘(+ 0x01 페이지에 맞추기; 0x02 는 숨은 상태 — 4.9절), 3바이트째 0x02 늘(+ 0x01 필터 적용 중). 탭 색이 없으면 `01 00 00 00 FF FF FF FF`, `rwSync`·`colSync` 0xFFFFFFFF, 코드 이름이 없으면 `SheetN` |
| `BrtBeginWsView` | 플래그 0x0300 늘(+ 0x02 수식, 0x04 눈금선, 0x08 머리글, 0x10 0 값, 0x40 선택), `icvHdr` 0x40, (24) 100. 기본 보기는 (20) = 현재 확대율, (22) = `zoomScaleNormal`, 페이지 나누기 미리 보기는 (20) = `zoomScaleNormal`, (22) = 현재 |
| `BrtSel` | 활성 창의 선택 **하나만** (XML 의 창마다 `selection` 중). `iRfxAct` = 활성 셀을 담은 범위의 번호 |
| `BrtWsFmtInfo` | 기본 열 너비 ×256, 8, 기본 행 높이(twips), 0, 개요 수준 0·0 (XML 의 `outlineLevelRow` 를 쓰지 않는다) |
| `BrtColInfo` | 너비 ×256, 서식, 플래그 (0x01 숨김, 0x02 너비 지정, 비트 8–10 수준, 접힘 **0x2000**) |
| `BrtRowHdr` | 높이는 지정이 아니어도 그 행의 높이(없으면 기본), 열 구간은 그 행 셀의 (첫 열, 끝 열) — XML 의 `spans` 그대로 |
| 셀 | 수는 부동소수 상위 30비트로 정확히 나타낼 수 있으면 `BrtCellRk`(부동소수 꼴만; 정수·×100 꼴은 쓰지 않는다 — RK 8218개 모두), 아니면 `BrtCellReal`. 글은 모두 공유 문자열(`BrtCellIsst`). 수식 셀은 값 종류대로 `BrtFmla*`, 휘발성 함수가 있으면 `grbitFlags` 0x02 |
| 공유 수식 | 기준 셀과 종속 셀 모두 `PtgExp`(행) + `rgcb` 의 열, 기준 셀 바로 뒤에 `BrtShrFmla`(범위 + 기준 셀에 대한 상대 토큰) |
| `BrtSheetProtection` | 잠기지 않은 셀 선택을 막으면 잠긴 셀 선택도 막는다 |
| 병합 | `BrtBeginMergeCells`(개수) — 순서는 한셀의 메모리 차례로 XML 과 다르다 (4.9절; 같은 문서의 레거시 MERGECELLS 와는 같다) |
| 하이퍼링크 | RfX, 관계 ID(내부면 없음), 위치, 설명, 표시 글 |
| `BrtPrintOptions` | 0x10 은 숨은 상태: `printOptions` 가 없는 시트 104/108 에 켜고, 있는 시트 2/51 에 켠다 |
| `BrtPageSetup` | 해상도는 XML 의 dpi 그대로, 끝의 관계 ID 는 빈 문자열(cch 0) |
| 머리글·바닥글 | 글이 하나라도 있을 때만 쓴다 (빈 `headerFooter` 는 쓰지 않는다) |
| `BrtBrk` | 끝 칸: 행 나누기 1048575, 열 나누기 16383 (XML 의 `max` 와 반대) |
| 메모 | `commentsN.bin`: 작성자는 처음 나온 순서, 글은 RichStr (조각 글꼴은 styles.bin 의 글꼴 번호). VML 은 XML 저장본과 같다 (4.1절) |

### 4.4 `styles.bin`

순서: [`BrtBeginFmts`…], 글꼴, 채우기, 테두리, 스타일 XF, 셀 XF, 스타일, `BrtBeginDXFs`(늘), `BrtBeginTableStyles`(늘).

| 항목 | 한셀 |
|---|---|
| 글꼴 | 계열·문자 집합이 없으면 2·0x81, 색은 RGB `05 00 00 00 R G B FF` |
| 채우기 | 무늬 색이 없으면 `05 40 00 00 00 00 00 FF`, 배경색이 없으면 `01 41 00 00 FF FF FF FF`, RGB 는 icv 칸에 0x40(무늬)·0x41(배경), 색인 64 는 `03 40 00 00 FF FF FF DF`. 그라데이션은 두 색 칸에 `01 40 … DF`·`01 41 … FF` |
| 테두리 | 선 종류가 없는 선(dg 0)에도 색이 있다 (한셀은 검정 RGB) — rcell 은 남겨 XML 에 쓴다 ([cell-2024.md](cell-2024.md) 7.10절의 `dataDxfId`). 선의 자동 색은 `01 00 00 00 FF FF FF DF` (서식 파일 683·조작 문서 33 선; 새 문서 15 선은 `FF` — 숨은 상태), 선 없음은 RGB 색 그대로, 자동·없음이면 검정 RGB (문서에 따라 자동 표지 — 4.9절). 색인 65(피벗 서식의 테두리)는 `exact` 면 한셀처럼 RGB `05 00 00 00 02 00 80 FF` (레거시 COLORREF 0x80800002 의 아래 세 바이트 — 피벗 실험 pa 의 테두리 6개), 기본은 색인 그대로 `03 41 …` |
| XF | 맞춤이 없으면 세로 아래(2), 보호가 없으면 잠금. 적용 비트는 XML 의 `apply*` 그대로 |
| 스타일 | 모두 `fBuiltIn`, 사용자 정의 스타일은 앞 스타일의 내장 번호, 수준 없음 0xFF |
| 차등 서식 | XFProp 은 채우기 → 글꼴 → 테두리 순, 묶음 안은 번호 순. `fNewBorder`(0x8000)는 위·아래·왼쪽·오른쪽을 모두 지정한 테두리. 속성 없는 글꼴은 0x18(이름)으로 자리를 남긴다 |
| 표 스타일 | 요소는 종류 번호 순 |

### 4.5 공유 문자열

XML 의 공유 문자열 순서를 지킨다 (서식 파일 28개 모두; 조작 문서 몇 개는 한셀이 순서를 바꾼다 — 숨은 상태).
`cstTotal` 은 XML 의 `count` 를 그대로 옮긴다 (실제 참조 수와 다를 수 있다). 문자열이 하나도 없으면 파트째 쓰지
않는다 (4.1절).

### 4.6 수식 토큰 (`src/binary/formula.rs`)

| 규칙 | 근거 |
|---|---|
| 클래스: 연산자의 피연산자는 값, `:`·`,`·교집합은 참조, 함수 인자는 인자 형식대로. 참조를 돌려주는 함수(`IF`·`INDEX`·`CHOOSE`·`OFFSET`·`INDIRECT`)는 자리의 클래스, 값을 돌려주는 함수는 값 | `IF(…,D18,IF(C18,C18,""))` 의 안쪽 IF 는 0x22, `ROUND` 는 R 자리에서도 0x41 |
| 인자 하나인 `SUM` → `tAttrSum`; `IF` → `tAttrIf`(참 쪽 + goto 길이) … `tAttrGoto`(끝까지 길이 − 1) | `IF(A1>5,"크다","작다")` |
| `IFERROR` → 첫 인자, `tAttr 0x80`(둘째 인자 길이), 둘째 인자, `tAttrGoto`, `PtgFunc 480` | `IFERROR(1/0,0)` |
| 휘발성 함수(`NOW`·`TODAY`·`RAND`·`OFFSET`·`INDIRECT`·`CELL`·`INFO`·`RANDBETWEEN`)가 있으면 맨 앞에 `tAttrVolatile` | `TODAY()-TODAY()` |
| 0–65535 정수 `PtgInt`, 나머지 `PtgNum`; 수 앞의 `-` 는 음수 `PtgNum` 하나 | `ABS(-5)` → `1F … 14 C0` |
| 이름 정의는 참조 클래스, 합집합은 `PtgMemFunc`(0x29 + 길이)로 감싼다 | 인쇄 영역 `A,B` |
| 상대 토큰(공유 수식·조건부 서식·유효성)의 행 오프셋은 **20비트** (`-7` → `F9 FF 0F 00`), 열은 14비트 | 서식 파일의 공유 수식 |
| 상대 위치 수식의 3차원 참조는 `PtgRef3d`/`PtgArea3d` 그대로, 상대 축만 오프셋 (`PtgRefN` 같은 3차원 토큰은 없다) | 3차원 참조 대조 실험 (2절) — 읽기는 `legacy::formula::decode_relative` |
| 구조적 참조는 `PtgList`(`18 19`, 2절): 열 하나면 끝 열 0, 형식 비트(10–11)는 `#This Row` 에만 1 — 함수 인자·연산자 자리와 상관없이 나머지는 0. 이름만 쓴 표(`COUNTA(Table1)`)도 열 없는 PtgList (XML 에는 `Table1[]`) | 표 대조 실험 (tbl2 의 구조적 참조 수식 16개가 한셀 토큰과 바이트까지 같다) |
| 공백은 쓰지 않는다 (`tAttrSpace` 없음). 한셀은 수식을 받을 때 공백·줄 바꿈을 지운다 — 남는 것은 교집합 연산자뿐 | 공백 대조 실험: COM 으로 넣은 `=A1 + B1`, `=SUM( A1 )`, `= A1*2`, `=A1+⏎B1` 등 20개가 XML 에서 `A1+B1`, `SUM(A1)` …, 바이너리에 `19 40` 없음 |

### 4.7 조건부 서식·유효성·자동 필터

- 규칙은 **우선순위가 큰 것부터** 쓴다 (XML 순서와 상관없이). `BrtBeginConditionalFormatting` 의 개수 = 규칙 수.
- 틀: 셀 값 0, 식 1, 색 눈금 2, 데이터 막대 3, 아이콘 4, 상위/하위 5, 글 8, 빈 칸 9·0x0A, 오류 0x0B·0x0C, 날짜
  0x0F…, 평균 0x19 위·0x1A 아래·0x1D·0x1E, 중복 0x1B, **고유 값 7**. 평균 초과 규칙은 플래그 0x04.
- 색(`BrtColor` 564): `05 R 00 00 R G B FF` — icv 칸에 빨강 값, 흰색은 `01 FF 00 00 FF FF FF FF`.
- 데이터 막대: 기본 레코드(467)의 길이는 늘 10·90, 실제 길이는 시트 끝의 x14 확장(1051)에. 규칙 끝에
  `BrtFRTBegin(02 0E 00 00)` `BrtCFRuleExt(1146: 0 + GUID)` `BrtFRTEnd` 로 x14 규칙과 GUID 로 잇는다. 시트 끝 묶음:
  35, 1135, (1046 범위·규칙 수, 1048 종류 4·틀 3·GUID·1, 1051, 1050×n, 1156, 1049, 1047)×막대, 1136, 36. XML 에 x14
  확장이 없으면 한셀이 새 GUID 와 기본값(1051 뒷부분 `01 00 06 00`)으로 만든다.
- x14 데이터 막대 확장의 칸 (x14 대조 실험: 열마다 `x14:dataBar` 속성 하나씩 바꾼 조작 XML 13개 규칙 → 한셀 → 바이너리,
  한셀은 자기 바이너리에서 모두 되읽는다). rcell 은 모델(`DataBar::ext`, 없으면 한셀이 만드는 기본 확장)에서 쓴다:
  - `BrtBeginDatabar14`(1051): FRT 머리 4, `minLength` u8, `maxLength` u8 (x14 기본 10·90), `showValue` u8,
    `direction` u8 (0 context, 1 leftToRight, 2 rightToLeft), `axisPosition` u8 (0 automatic, 1 middle, 2 none),
    플래그 u16 (0x01 `border`, 0x02 `gradient`, 0x04 `negativeBarColorSameAsPositive="0"`, 0x08
    `negativeBarBorderColorSameAsPositive="0"`) — 기본값은 0x06.
  - `BrtCFVO14`(1050): FRT 머리 (0x04 = 수식 있음), 수식이 없으면 종류 u32 (2 min, 3 max, 8 autoMin, 9 autoMax) + 0 ×20.
    수식이 있으면 `01 00 00 00 02 00 00 00`, cce u32, 0 u32, 토큰열, 종류 u32 (1 num, 4 percent, 5 percentile, 7 formula),
    0 ×20, 끝에 cce u32 다시 (`num 2` → `1E 02 00`, `formula $A$1` → `44 00000000 0000`).
  - `BrtColor14`(1055): FRT 머리 4 + BrtColor 8 (564 와 같은 꼴). 쓰인 색만 테두리 → 음수 막대 → 음수 테두리 → 축 순서로
    쓴다 (`x14:fillColor` 는 쓰지 않는다 — 버린다). 테두리만이면 하나, cf_dv 의 음수·축이면 둘 — 한셀은 둘 다 바르게
    되읽지만 어느 색인지 가리는 칸은 아직 찾지 못했다 (레코드 자체에는 없다).
- 유효성: `BrtBeginDVals`(0, 0, 0, 0, 개수), 플래그 = 종류 | 오류 모양<<4 | 0x100 빈 칸 | 0x200 목록 숨김 |
  0x40000 입력 메시지 | 0x80000 오류 메시지 | 연산자<<20. 수식 기준은 첫 범위의 왼쪽 위 셀.
- 자동 필터: 사용자 지정 문자열 조건은 `vts 6`, 값 칸 `00 00 cch 00 00 00 00 00` + 문자열. 상위/하위는 플래그에 0x04 를
  늘 켜고, 기준값을 끝에 쓴다 — 원본에 기준값(`filterVal`)이 있으면 그 값 그대로(낡은 999 도), 없으면 한셀이 데이터로 셈한
  값이다 (상위/하위 필터 기준값 실험: 시트 22개의 조작한 XML → 한셀 → 바이너리): 필터 범위의 머리글 아래 그 열의 숫자
  값(글·빈 칸은 빼고 숨은 행은 넣는다, 같은 값도 하나씩)을 늘어놓아 위(아래)에서 k번째 값. k 는 개수, 백분율이면
  ⌊숫자 수 × 값 / 100⌋ 이되 적어도 1 (11개의 20% → 2, 50% → 5, 5% → 1, 25개의 10% → 2), 숫자 수보다 크면 끝 값.
  rcell 도 그렇게 셈해 쓴다 (`top10_value`) — 한셀의 `.xlsb` 읽기는 백분율 비트(0x02)를 버리고 기준값이 0 이면 다시 셈하므로,
  0 을 쓰면 하위 20% 가 '하위 20개'로 셈해져 기준값이 달라진다 (filter2: −3 대신 6). 동적 필터의 5번째 바이트와 값(평균
  등)도 한셀이 계산한다 — rcell 은 원본에 있는 값을 쓰고 없으면 0 을 쓴다.

### 4.8 한셀의 바이너리 저장이 잃는 것과 rcell

2.1절의 손실 가운데 한셀의 바이너리 **읽기**는 되살리는 것이 있다 (rcell 이 쓴 바이너리를 .xlsb 로 열어 본 결과,
`tools/probe/binw-oracle.ps1`): 차등 서식의 맞춤, `draft`, 구조 잠금 없는 통합 문서 보호(암호, 창 잠금 비트는 읽지
않는다), 나눈 창의 활성 창, 요약 열 왼쪽. rcell 은 기본으로 이것들을 쓴다 (잃지 않는다). `Options::exact`
(`rcell convert … --exact`)는 한셀처럼 잃는다 — 그때 머리글·바닥글의 글꼴 코드 없는 구역에 기본 글꼴 코드도 넣는다.

차등 서식의 표시 형식은 한셀의 바이너리 읽기가 제대로 읽는 꼴이 없다 (대조 실험: 형식 문자열 0x26 → 빈 형식,
내장 번호 0x29 → 다른 형식, `BrtFmt` 에 올린 사용자 번호 0x29 → 무시). rcell 은 마지막 꼴로 쓴다: 한셀에는 해가 없고
rcell 은 `BrtFmt` 에서 형식을 찾아 다시 읽는다.

### 4.9 숨은 상태 (레코드 대조에 남는 차이)

XML 이 같은데 바이너리가 다른 칸 — rcell 은 가장 흔한 값을 쓴다. 아래 셋은 다시 조사했다 (말뭉치 114쌍과 피벗 실험 56개):

- **병합 차례**: 한셀의 바이너리 저장과 레거시 저장은 같은 메모리 차례를 쓴다 (병합 둘 이상인 말뭉치 시트 12개 모두 `BrtMergeCell` 차례 =
  레거시 MERGECELLS 차례), XML 은 행 차례로 정렬해 쓴다. 해시 표 순회로 설명되지 않는다 (열쇠 8가지 × 표 크기 44가지 × 차례 넷 —
  12개 중 4개를 넘지 못한다). XML 입력에서는 정할 수 없고, 레거시 입력이면 모델 차례가 그 차례다 (Basic_Invoice 레거시 → rcell
  바이너리의 병합 레코드가 한셀 바이너리와 같다).
- **`BrtWsProp` 2바이트째 0x02**: 시트 160개 중 73개. 말뭉치 뿌리 문서·craft-charts2·craft 일부에는 켜지고 서식 파일·craft-charts 에는
  꺼진다 — 같은 꼴의 시트 XML(차트 하나짜리 chart_area 대 c2_e1_leg_l, 셀 자료 말고 바이트까지 같다)에서도 갈리므로 문서를 만든
  세션의 상태다.
- **선 없는 가장자리의 자동 색** (`<x:left><x:color auto="1"/></x:left>`): 검정 RGB 881, 자동 표지 `…FF DF` 138 — 자동 표지는 서식
  파일 넷(Cash_Flow_Sensitivity·College_Monthly_Budget·Portfolio_Allocation·Statement_of_Net_Worth)과 craft/odd_anchor, 피벗 실험
  문서(피벗 서식의 테두리)의 0번이 아닌 테두리 전부다. 테두리의 꼴(선 있는 가장자리 수·색 종류)·쓰임과 상관없이 문서 단위로
  갈리고 XML 은 같다 — 한셀 메모리의 '색 없음'과 '자동'이 XML 에서는 같은 `auto="1"` 로 나오는 것으로 보인다.

그 밖에: `BrtWsProp` 2바이트째 0x02, `BrtBeginWsView` 의
(22)·(24), `BrtPrintOptions` 0x10, 테두리 자동 색의 `DF`/`FF`, 머리글·바닥글 여백의 HWPUNIT 하나, 병합과 공유 문자열의
순서, 몇몇 서식 파일의 XTI 순서와 쓰이지 않는 XTI, XF 의 순서가 바뀐 문서 하나, 쓰이지 않는 글꼴 하나, 차등 서식의
이중 회계 밑줄(XML 은 단일 회계로 쓴다), 데이터 막대의 GUID, 동적 필터가 데이터로 계산한 값(원본에 없을 때),
`BrtAbsPath15` 의 폴더, 메모 VML 의 `x:ShapeUri` (세션 GUID — 레코드 대조는 뺀다). rcell 모델에 없어 다른 것: 다시
부호화한 그림과 조작한 XML 의 그림 앵커 정규화 (2.1절). 데이터 막대의 x14 확장(방향·축·플래그·x14 기준 값·색)은 모델(`DataBar::ext`)에서 쓰고 `rcell diff` 가
비교한다 (x14 조건부 서식 실험).

### 4.10 구조 잠금 문서의 암호화 컨테이너 (`src/binary/encrypt.rs`)

구조 잠금 문서를 한셀이 바이너리로 저장한 것 (코퍼스 outline, COM 으로 만든 패키지 5.6 KB·99 KB·706 KB 문서 셋):
OLE2 복합 문서 판 3 (0x3E, 512바이트 섹터) 안에 스트림 `EncryptionInfo` 와 `EncryptedPackage` 둘뿐이다
(`\x06DataSpaces` 없음).

- `EncryptionInfo` 224 바이트: 버전 4.2, 플래그 0x24, 머리 길이 140 (플래그 0x24, 0, AES-128 0x660E, SHA-1 0x8004,
  128, 공급자 0x18, 0, 0, `Microsoft Enhanced RSA and AES Cryptographic Provider`), 검증자 (salt 16, 암호화한 검증자 16,
  해시 길이 20, 암호화한 해시 32 — 해시 뒤 12바이트는 0).
- **salt 와 검증자가 같은 16바이트**이고, 한셀 한 번 실행하는 동안 같은 값이다 (같은 세션의 두 문서). rcell 은
  패키지 SHA-1 의 앞 16바이트를 쓴다.
- `EncryptedPackage`: 원래 길이 u64 + AES-128 ECB 암호문. 마지막 블록을 채운 바이트는 한셀은 쓰레기 값, rcell 은 0.
- 섹터: 0 FAT, 1 디렉터리 (루트 → 자식 `EncryptionInfo`(검정) → 오른쪽 `EncryptedPackage`(빨강), 넷째 항목은 빈
  항목), 2 미니 FAT, 3 미니 스트림 (`EncryptionInfo`), 4… 패키지. FAT 섹터는 0, 125, 254, 383, 512, 633, 762, 891,
  1020, 1149, 1278, 1407 (706 KB) — 패키지를 8바이트 머리 다음 4096바이트씩 쓰면서, 쓰기 뒤의 섹터 수가 FAT 이 덮는
  수를 넘으면 그 자리에 FAT 섹터를 잡는 꼴이다. rcell 은 이 규칙으로 같은 자리에 둔다.
- 한셀은 루트에 저장 시각을 적고, 파일을 패키지 스트림 끝에서 자른다 (마지막 섹터가 덜 찬 채). rcell 은 시각 0,
  마지막 섹터를 채운다. 패키지가 4096 바이트보다 작으면 [MS-CFB] 대로 미니 스트림에 넣는다 (한셀에서는 보지 못한
  크기 — 관측한 한셀 패키지는 모두 5.5 KB 넘는다. rcell 은 레거시에서 읽은 문서처럼 테마가 없으면 더 작게 쓴다).
- XML 저장: 같은 세션에서 COM 으로 구조를 잠근 문서는 XML 도 감싸지만 (`xmlfirst.cell`), 코퍼스의 outline.cell 은
  감싸지 않았다 — 조건 미확인. 한셀이 감싼 바이너리를 다시 열어 XML 로 저장하면 그것도 감싼다 (X.bin.as.cell).
- 한셀 대조: rcell 이 감싼 outline 바이너리를 한셀이 .xlsb 로 열어 XML 로 저장한 결과가 한셀 자기 바이너리를 같은
  길로 연 것과 차이 0 (exact·기본 모두), 그 XML 도 한셀이 감싼다.

### 4.11 표 (`table_part`)

표 모델(`Sheet::tables`)에서 표 파트를 쓴다 — XML 원본의 표 파트(XML)는 옮기지 않는다. 계산 열 수식은 `BrtListCCFmla`(351),
요약 행 사용자 수식은 `BrtListTrFmla`(352: `fArray u8` + CellParsedFormula), 셀 수식·이름의 구조적 참조는 PtgList (4.6절).
한셀 대조 (표 문서 14개 — 표 대조 실험 tbl·tbl2·s·v 와 COM 문서): rcell 이 XML 저장본에서 쓴 바이너리가 한셀의 바이너리와
레코드까지 같고 (`BrtAbsPath15` 말고), 한셀이 .xlsb 로 읽은 결과가 한셀 자기 바이너리를 읽은 것과 차이 0. 바이너리 원본 →
rcell 바이너리도 차이 0. 한셀의 `.xlsb` 읽기는 요약 행을 버린다 — 자기 바이너리도 `totalsRowCount="1"` 이 사라지고
`totalsRowShown="0"` 이 된다 (tbl·tbl2) — 그래서 요약 행은 레코드 대조로 확인했다. 조작한 XML 의 계산 열 수식(351)은 한셀이
읽지 않고 버린다 (XML 읽기와 같다).

## 5. 피벗 (`src/binary/pivot.rs`)

rcell 의 피벗 모델은 XML 파트다 (원본 그대로 옮기는 파트 + 관계 `PartLink`, 캐시 번호 `cache_id`). 바이너리 쓰기는 XML
파트를 레코드 파트로, 바이너리 읽기는 레코드 파트를 한셀의 `.xlsb` 읽기가 쓰는 꼴의 XML 파트로 바꾼다. 근거는 피벗
실험이다: COM 으로 만든 피벗 문서 11개(pa–pi: 행·열·페이지·데이터 필드, 데이터 함수 11가지, 날짜·빈 칸·참거짓 값, 다른
시트의 피벗, 캐시 둘·피벗 셋)와 그 XML 저장본의 속성을 하나씩 바꾼 조작 문서 33개(v_*)를 한셀이 XML·바이너리·레거시로
저장한 것을 레코드마다 맞댔다. 레코드·필드 이름은 [MS-XLSB] 2.4 를 따른다.

### 5.1 패키지·통합 문서

| 항목 | 한셀 |
|---|---|
| 파트 | `xl/pivotCache/pivotCacheDefinitionN.bin` (관계 rId1 → `pivotCacheRecordsN.bin`), `xl/pivotTables/pivotTableN.bin` (관계 rId1 → `../pivotCache/pivotCacheDefinitionN.bin`). N 은 문서 안에서 1부터 — 캐시는 통합 문서 관계 순서, 피벗 표는 시트 순서 |
| 관계 | 통합 문서: 시트·외부 연결 다음에 캐시 정의 (pf3: 시트 둘 뒤 rId6·rId7). 시트: 표 다음, 메모 앞에 피벗 표 — 시트 레코드에는 피벗이 없다 |
| `workbook.bin` | `BrtCalcProp` 뒤 `BrtBeginPivotCacheIDs`(384) [`BrtBeginPivotCacheID`(386: `idSx u32` = `cacheId`, 관계 ID) `BrtEndPivotCacheID`(387)]×n `BrtEndPivotCacheIDs`(385) |
| XTI | 캐시의 원본 시트는 쓰이지 않아도 XTI 를 받는다 — **모든 시트의 수식 다음** (XTI 실험 px1: 시트 다섯, 첫째 시트가 원본인 피벗이 둘째 시트에, 첫째·둘째·셋째 시트의 수식이 셋째·넷째·다섯째 시트를 가리킴 → `[셋째, 넷째, 다섯째, 첫째]`). 이름 정의 등이 먼저 쓰면 그 번호 그대로 (px2) |

### 5.2 피벗 표 (`pivotTableN.bin`)

```
BrtBeginSXView 280
  BrtBeginSXLocation 314  BrtEndSXLocation 313
  BrtBeginSXVDs 287 (필드 수)
    (BrtBeginSXVD 285 [BrtBeginSXVIs 283 (BrtBeginSXVI 282 BrtEndSXVI 281)×n BrtEndSXVIs 284] BrtEndSXVD 286)×n
  BrtEndSXVDs 288
  [BrtBeginISXVDRws 309 (수 u32, 필드 i32 ×n — −2 는 값) BrtEndISXVDRws 310]
  [BrtBeginSXLIRws 299 (BrtBeginSXLI 297 [BrtBeginISXVIs 388 BrtEndISXVIs 389] BrtEndSXLI 298)×n BrtEndSXLIRws 300]
  [BrtBeginISXVDCols 311 … 312]  [BrtBeginSXLICols 301 … 302]
  [BrtBeginSXPIs 291 (BrtBeginSXPI 289 BrtEndSXPI 290)×n BrtEndSXPIs 292]
  [BrtBeginSXDIs 295 (BrtBeginSXDI 293 BrtEndSXDI 294)×n BrtEndSXDIs 296]
  [BrtTableStyleClient 513]
BrtEndSXView 315
```

| 레코드 | 칸 |
|---|---|
| `BrtBeginSXView` | 판 u8 (늘 3 — XML 의 판과 상관없이, v_ver), 플래그 u16, `indent`+1 u8 (0x80 `showHeaders="0"`), 플래그 u32 ×2, 값 축 u8 (1 행 = `dataOnRows`, 2 열), `pageWrap` u8, 3, 3, `dataPosition` u32 (없으면 0x0000FFFF), `autoFormatId` u16, 0 u16, `chartFormat` u32, `cacheId` u32, 이름, `dataCaption`, 있는 글만 차례로: 총합계·오류·빈 칸·페이지 스타일·표 스타일(`pivotTableStyleInfo` 이름)·빈 자리 스타일·태그·열 머리글·행 머리글 |
| 플래그 u16 | 0x0001 `showItems`, 0x0002 `editData`, 0x0004 `disableFieldList`, 0x0010 `showCalcMbrs="0"`, 0x0020 `visualTotals="0"`, 0x0040 `showMultipleLabel`, 0x0100 `showDataDropDown="0"`, 0x1000 `showDrill="0"`, 0x2000 `printDrill`, 0x4000 `showMemberPropertyTips`, 0x8000 `showDataTips="0"` |
| 첫 플래그 u32 | 0x01 `showDropZones="0"`, 0x02 `asteriskTotals="0"`, 0x04 `showEmptyRow`, 0x08 `showEmptyCol`, 0x10 `enableWizard`, 0x20 `enableDrill`, 0x40 `enableFieldProperties`, 0x80 `preserveFormatting`, 0x100 `useAutoFormatting`, 0x200 `showError`, 0x400 `showMissing`, 0x800 `pageOverThenDown`, 0x1000 `subtotalHiddenItems`, 0x2000 `rowGrandTotals`, 0x4000 `colGrandTotals`, 0x8000 `fieldPrintTitles`, 0x20000 `itemPrintTitles`, 0x40000 `mergeItem`, 0x80000 늘, 0x100000 총합계 글, 0x200000 페이지 스타일, 0x400000 표 스타일, 0x800000 빈 자리 스타일, 0x1000000–0x10000000 `applyNumber`·`Font`·`Alignment`·`Border`·`PatternFormats`, 0x20000000 `applyWidthHeightFormats`, 0x40000000 태그 |
| 둘째 플래그 u32 | 0x01 `compact`, 0x02 `outline`, 0x04 `outlineData`, 0x08 `compactData`, 0x10 `gridDropZones="0"`, 0x20 `published`, 0x40 오류 글 없음, 0x80 빈 칸 글 없음, 0x100 `immersive="0"`, 0x200 `multipleFieldFilters="0"`, 0x400 행 머리글 글, 0x800 열 머리글 글, 0x1000 `fieldListSortAscending`, 0x4000 `customListSort="0"` |
| `BrtBeginSXLocation` | RfX, 머리글 행·데이터 행·데이터 열 u32 (**절대 좌표** — XML 은 범위 왼쪽 위에서의 거리), 페이지 필드 행 수·열 수 u32 |
| `BrtBeginSXVD` | 축 u8 (0x01 행, 0x02 열, 0x04 페이지, 0x08 데이터 필드), 플래그 24비트 (0x01 `defaultSubtotal`, 0x02–0x800 `sum`·`countA`·`avg`·`max`·`min`·`product`·`count`·`stdDev`·`stdDevP`·`var`·`varPSubtotal`, 0x10000 `allDrilled`, 0x20000 `showDropDowns="0"`, 0x40000 `hiddenLevel`, 0x80000 `uniqueMemberProperty` 있음, 0x100000 `compact`, 0x200000 이름 있음, 0x400000 `subtotalCaption` 있음, 0x800000 `dataSourceSort`), `numFmtId` u32, 플래그 u32 (0x01–0x10 `dragToRow`·`Col`·`Page`·`Off`·`Data`, 0x20 `showAll`, 0x40 `outline`, 0x80 `insertBlankRow`, 0x100 `subtotalTop`, 0x200 `serverField`, 0x800 `insertPageBreak`, 0x1000 자동 정렬, 0x2000 오름 — 수동(`sortType` 없음)도 켠다, 내림차순은 0x1000 만 (v_sorta·v_sortd — [MS-XLSB] 의 fAscendSort 는 1 이 내림차순이라 한셀과 반대), 0x4000 `autoShow`, 0x8000 `topAutoShow`, 0x10000 `hideNewItems`, 0x20000 `measureFilter`, 0x40000 늘, 0x80000 `multipleItemSelectionAllowed`, 0x100000 `nonAutoSortDefault`), `itemPageCount` i32 (10), `rankBy` i32 (−1), 이름, `subtotalCaption` |
| `BrtBeginSXVI` | 종류 u8 (0 data, 1 default, 2 sum, 3 countA, 4 avg, 5 max, 6 min, 7 product, 8 count, 9 stdDev, 10 stdDevP, 11 var, 12 varP, 13 grand, 14 blank), 플래그 u16 (0x01 `h`, 0x02 `sd="0"`, 0x04 `f`, 0x08 `m`, 0x10 이름 있음, 0x20 `d`, 0x40 `c`), 캐시 항목 i32 (data 가 아니면 −1), 이름 |
| `BrtBeginSXLI` | 앞 줄과 같은 항목 수 u16 (`r`), 종류 u8 (`t`), 0 u8, 항목 수 u32, 데이터 필드 u32 (`i`). 항목은 `BrtBeginISXVIs`(388) u32 ×n |
| `BrtBeginSXPI` | 필드 u32, 항목 u32 (고르지 않으면 0x001000FE), 계층 i32 (−1), 플래그 u8 (0x01 이름, 0x02 캡션), 글 |
| `BrtBeginSXDI` | 필드 u32, 함수 u32 (0 sum, 1 count, 2 average, 3 max, 4 min, 5 product, 6 countNums, 7 stdDev, 8 stdDevp, 9 var, 10 varp), 값 표시 u32 (`showDataAs`: 0 normal … 8 index), 기준 필드 i32, 기준 항목 u32, `numFmtId` u32, 이름 있음 u8 (한셀 0x07), 이름 |
| `BrtTableStyleClient` | 0x02 끝 열, 0x04 행 줄무늬, 0x08 열 줄무늬, 0x10 행 머리글, 0x20 열 머리글. 한셀은 나머지 비트(0x40 …)에 초기화하지 않은 값을 남긴다 — rcell 은 0 |

### 5.3 캐시 (`pivotCacheDefinitionN.bin`, `pivotCacheRecordsN.bin`)

```
BrtBeginPivotCacheDef 179
  BrtBeginPCDSource 185 (종류 u32 — 0 worksheet, 1 external, 2 consolidation, 3 scenario; 연결 u32)
    BrtBeginPCDSRange 187  BrtEndPCDSRange 188
  BrtEndPCDSource 186
  BrtBeginPCDFields 181 (필드 수)
    BrtBeginPCDField 183
      [BrtBeginPNames 253 (BrtBeginPName 255 BrtEndPName 256)×n BrtEndPNames 254]                       계산 필드
      [BrtBeginPCDFAtbl 189 (항목 20–25 | BrtBeginPCDIRun 191 … BrtEndPCDIRun 192)… BrtEndPCDFAtbl 190]
      [BrtBeginPCDFGroup 219 [BrtBeginPCDFGRange 223 224] [BrtBeginPCDFGItems 221 … 222] BrtEndPCDFGroup 220]
    BrtEndPCDField 184
  BrtEndPCDFields 182
BrtEndPivotCacheDef 180
```

| 레코드 | 칸 |
|---|---|
| `BrtBeginPivotCacheDef` | 판 u8 ×3 (늘 3), 플래그 u8 (0x01 `saveData`, 0x02 `invalid`, 0x04 `refreshOnLoad`, 0x08 `optimizeMemory`, 0x10 `enableRefresh`, 0x20 `backgroundQuery`, 0x40 `upgradeOnRefresh`, 0x80 `tupleCache`), `missingItemsLimit` i32 (−1), `refreshedDate` f64, 플래그 u8 (0x02 레코드 관계 ID 있음; 한셀은 `refreshedBy`(0x01)를 쓰지 않는다), 레코드 수 u32, 관계 ID (`rId1`) |
| `BrtBeginPCDSRange` | 이름 원본 u8, 내장 이름 u8, 플래그 u8 (0x01 관계 ID, 0x02 시트), [시트], [관계 ID], 이름 또는 RfX |
| `BrtBeginPCDField` | 플래그 u16 (0x04 원본 필드 `databaseField`, 0x08 캡션, 0x100 수식), `numFmtId` u32, `sqlType` u16, 계층 u32, 수준 u32, 덧붙은 u32 수 + u32 ×n, 이름, [캡션], [수식: cce u32, 토큰열, cb u32, 덧붙은 데이터]. 계산 필드 수식의 필드 이름은 `PtgSxName`(0x18 0x1D + 이름 목록 번호 u32), 목록은 `BrtBeginPName`(`ifdb` u32 = 캐시 필드 번호, 함수 u8 0xFF 없음, 플래그 u8) — v_calc: `금액*2` → `18 1D 00000000 1E 0200 05`, 목록 [3] |
| `BrtBeginPCDFAtbl` | 플래그 u16 (0x01 `containsSemiMixedTypes`, 0x02 `containsNonDate`, 0x04 `containsDate`, 0x08 `containsString`, 0x10 `containsBlank`, 0x20 `containsMixedTypes`, 0x40 `containsNumber`, 0x80 `containsInteger`, 0x100 최소·최대 있음, 0x200 `longText`), 항목 수 u32, [최소 f64, 최대 f64 — 날짜는 일련번호]. 한셀은 날짜만 있는 필드에도 0x02 를 켜고 (XML 은 `containsNonDate="0"`, pg), 묶음으로 쓴 필드의 최소·최대에 둘 다 **첫 값**을 쓴다 (한셀의 읽기는 항목에서 다시 셈한다) |
| 항목 | `BrtPCDIMissing`(20), `BrtPCDINumber`(21: f64), `BrtPCDIBoolean`(22: u8), `BrtPCDIError`(23: u8), `BrtPCDIString`(24), `BrtPCDIDatetime`(25: 해 u16, 달 u16, 날·시·분·초 u8). 한셀은 수만 있는 공유 항목을 `BrtBeginPCDIRun`(191: 종류 u16 — 0x01 수, 0x02 글, 0x10 오류, 0x20 날짜 — 개수 u32, 값들) 하나로, 날짜만이면 날짜 묶음으로, 묶음 항목(`groupItems`)의 글과 255개를 넘는 공유 항목의 글은 글 묶음으로 (w_big: 300개는 묶음, 3개는 하나씩), 나머지는 항목마다 레코드로 쓴다 |
| 묶음 | `BrtBeginPCDFGroup`(219: 부모 i32, 기준 i32), `BrtBeginPCDFGRange`(223: 단위 u8 — 0 range, 1 seconds … 7 years, 플래그 u8 — 0x01 `autoStart`, 0x02 `autoEnd`, 0x04 날짜, 시작·끝·간격 f64), `BrtBeginPCDFGItems`(221: 수 u32) + 항목 |
| 레코드 파트 | `BrtBeginPivotCacheRecords`(193: 수 u32), 행마다 `BrtPCRRecord`(33) — 원본 필드마다 공유 항목이 있으면 번호 u32, 없으면 수 f64·날짜·글 (계산 필드는 없다) |

### 5.4 한셀의 바이너리 저장이 잃는 것과 rcell

조작한 XML 33개 → 한셀 → 바이너리에서 잃는 것: 총합계·빈 자리 스타일·태그·행/열 머리글 글, `apply*Formats` 다섯
(`applyWidthHeightFormats` 말고), `autoFormatId`, 필드의 `subtotalCaption`·`autoShow`·`itemPageCount`·`rankBy`·끌기 금지
(`dragTo*="0"` 을 늘 1 로)·`showDropDowns="0"`·`hideNewItems`·`measureFilter`·`multipleItemSelectionAllowed`·
`nonAutoSortDefault`·`insertPageBreak`, 데이터 필드의 `showDataAs`·`baseField`·`baseItem`·`numFmtId` (모두 0), 캐시 필드의
`caption`, `refreshedBy`, 피벗 서식(`formats`)·조건부 서식·차트 서식·x14 확장. 대부분 한셀의 XML 저장도 잃는다
([cell-2024.md](cell-2024.md) 7.12절). rcell 은 기본으로 이것들을 쓰고 `exact` 면 한셀처럼 잃는다 — `refreshedBy` 와 서식은
쓰지 않는다. 한셀은 읽을 때 `outline="0"` 필드를 `compact="0"` 으로, `dataOnRows` 를 값 필드(−2)의 자리로 고친다 (조작한
XML 에서만 어긋난다 — v_blank·v_dor). 한셀은 항목이 없는 피벗 필드(계산 필드)의 `defaultSubtotal="0"` 을 XML 에 쓰지
않는다 — 그래서 한셀 XML 저장본에서 쓴 rcell 바이너리는 그 비트만 한셀 바이너리와 다르다 (v_calc).

### 5.5 읽기

레코드 → XML 은 한셀이 같은 바이너리를 `.xlsb` 로 열어 XML 로 저장한 것과 같은 꼴이다 (속성 순서까지): `recordCount` 를
쓰고, 최소·최대는 항목에서 셈하고, x14 확장(`hideValuesRow="1" altText=""`)을 늘 붙이고, 이름 원본은 `sheet=""`, 판은
표 `updatedVersion`·`createdVersion` 4, 캐시 `createdVersion` 3. 한셀과 다른 점: 한셀은 `sortType` 을 XML 에 쓰지 않고
(rcell 은 쓴다), 계산 필드의 피벗 필드에 `dragToRow`·`Col`·`Page="0"` 을 넣고 `defaultSubtotal="0"` 을 빼며 (rcell 은
레코드 그대로), 묶음 필드가 있는 캐시의 마지막 레코드 값을 4294967295 로 쓴다 (v_group — rcell 은 레코드의 6).

### 5.6 검증

- 레코드 대조 (`rcell bindiff`): 피벗 문서 44개(pa–pi, v_*)에서 rcell 이 한셀 XML 저장본으로 쓴 바이너리의 피벗 파트가 한셀
  바이너리와 바이트까지 같다 — `BrtTableStyleClient` 의 한셀 쓰레기 비트, 한셀이 XML 에서 잃은 값(`sortType` 등)과 v_calc 의
  위 비트 말고. 조작한 원본 XML 에서 `exact` 로 쓰면 한셀의 읽기 정규화 둘(v_blank·v_dor)과 공유 항목 없는 필드의 항목
  (v_group, 한셀이 버린다)만 다르다. 한셀 바이너리 → rcell → 바이너리는 44개 모두 쓰레기 비트 말고 같다.
- XML 대조: rcell 이 한셀 바이너리에서 쓴 피벗 XML 파트가 한셀의 `.xlsb` 읽기 결과와 같다 — 5.5절의 차이(v_calc·v_group·
  v_sorta·v_sortd)만.
- 한셀 대조 (`rcell diff`, 피벗 문서 56개 — pa–pi, v_*, XTI 실험 px1·px2, 하나만 바꾼 w_* 와 항목 300개 필드의 w_big):
  rcell 바이너리를 한셀이 읽은 것 대 한셀 자기 바이너리를 읽은 것, 바이너리 원본 → rcell 바이너리, 바이너리 원본 → rcell XML 대
  같은 문서의 XML 저장본 — 피벗 표 차이는 모두 0 (비교: 위치·캐시 원본과 필드·공유 항목 수·계산 필드 수식·필드 축·숨긴 항목·
  부분합·행/열/페이지/데이터 필드·스타일·총합계). 한셀은 바이너리 저장에서 테두리의 색인 65 를 RGB `020080` 으로
  바꾼다 (피벗 서식의 테두리) — rcell 은 기본으로 색인 65 를 쓰고, `exact` 는 한셀처럼 쓴다: 한셀이 rcell 의 `exact` 바이너리를 읽은 것이
  한셀이 자기 바이너리를 읽은 것과 56개 모두 문서 전체 차이 0 (`rcell diff --ignore-hancom`). 바이너리 원본 → rcell 바이너리와 XML → rcell XML 은 차이 0.
