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
| 이름 | `BrtName` 의 `fBuiltin`(0x20) 이면 `Print_Area` 같은 내장 이름 → XML 의 `_xlnm.Print_Area` |
| 외부 연결 | `workbook.bin` 의 `BrtSupBookSrc`(355) = 관계 ID → `xl/externalLinks/externalLinkN.bin`: `BrtBeginSupBook`(360, `sbt u16` 0 = 외부 + 관계 ID → `externalLinkPath`), `BrtSupTabs`(359, `cTab u32` + 시트 이름들) |
| 시트 속성 | `BrtWsProp`(147): 플래그 3바이트(0x000001 자동 페이지 나누기, 0x000040 요약 행 아래, 0x000080 요약 열 오른쪽, 0x000100 페이지에 맞추기, 0x010000 필터 적용 중), `BrtColor` 탭 색(자동 = 없음), `rwSync`, `colSync`, 코드 이름 (없으면 한셀이 채운다) |
| 행 높이 | `BrtRowHdr` 의 `miyRw` 는 지정 높이(`fUnsynced` 0x2000)가 아니어도 그 행의 실제 높이다 — XML 의 `ht` (customHeight 없이). 버리면 한셀이 높이를 다시 계산해 그리기 앵커가 어긋난다 |
| 개요 | `BrtRowHdr` 플래그 비트 8–10 수준, 0x0800 접힘. `BrtColInfo` 플래그 비트 8–10 수준, 접힘은 [MS-XLSB] 의 0x1000 이 아니라 **0x2000** 에 쓴다 (rcell 은 둘 다 받는다) |
| 보호 | `BrtBookProtection`(534: `protpwdBook u16, protpwdRev u16, 플래그 u16` — 1 구조, 2 창), `BrtSheetProtection`(535: `protpwd u16` + u32 × 16 — `fLocked` 다음 15개는 **허용** 여부라 OOXML 속성과 반대) |
| 자동 필터 | `BrtBeginAFilter`(161: RfX) 안에 `BrtBeginFilterColumn`(163: `dwCol u32, 플래그 u16`) 마다: 값 목록 `BrtBeginFilters`(165: `fBlank u32, 달력 u32`) + `BrtFilter`(167: 문자열); 사용자 지정 `BrtBeginCustomFilters`(172: `fAnd u32`) + `BrtCustomFilter`(174: `vts u8` (4 수, 6 문자열), `grbitSign u8`, 값 8바이트, 문자열이면 이어서 XLWideString); 상위/하위 `BrtTop10Filter`(170: 플래그 u8 (1 위, 2 백분율, 4 기준값), 개수 f64, 기준값 f64); 동적 `BrtDynamicFilter`(171: `cft u32` — 레거시 AUTOFILTER12 와 같은 번호, 플래그 u8, 값 f64, 최댓값 f64) |
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
- 보호가 걸린 문서는 표준 암호화(ECMA-376, AES-128, 기본 암호 `VelvetSweatshop`)로 저장한다 — rcell 은 투명하게 푼다.
- 외부 연결의 상태를 잃는다: 값 캐시(`BrtExternTableStart` … 를 쓰지 않아 `refreshError` 도 사라짐)와
  경로를 잃은 연결의 표시(관계 종류 `xlPathMissing` → 보통의 `externalLinkPath`). 한셀은 이 둘에 따라 이름 참조의
  따옴표를 정하므로, 바이너리를 거친 문서는 이름의 표기가 달라질 수 있다.

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

## 3. 검증

같은 문서를 XML 형식과 이 형식으로 함께 저장해(`tools/probe/make-corpus.ps1`) 비교한다.
한컴 확장을 뺀 나머지는 샘플 8종과 서식 파일 28종 모두 일치한다:

```
rcell diff X.bin.cell X.cell --ignore-hancom
```

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
| 통합 문서 관계 | rId1 테마, rId2 styles.bin, rId3 sharedStrings.bin, rId4… 시트, 그 뒤 외부 연결 (`externalLinks/externalLinkN.bin`) |
| 시트 관계 | 하이퍼링크(외부), 그리기, VML, 메모 순. 대상은 상대 경로 (`../drawings/drawing1.xml`) |
| 외부 연결 파트 | `BrtBeginSupBook`(360: `sbt 0`, 관계 ID `rId1`, 빈 문자열 0xFFFFFFFF), `BrtSupTabs`(359), `BrtEndSupBook`(588). 관계는 경로를 잃은 연결도 보통의 `externalLinkPath` |
| 그리기·차트·그림·VML | XML 파트 그대로 (차트는 모델에서 `chart_xml` 로). 한셀은 그리기·차트·그림 파트에 **세션 전체의 번호**를 붙인다 (같은 문서의 `X.cell` 은 `drawing69.xml`, `X.bin.cell` 은 `drawing70.xml`) — 문서의 정보가 아니다 |
| docProps | app.xml: `Application` Cell, 시트 이름 목록(`TitlesOfParts`), `TotalTime` 0, `AppVersion` 12.0300. core.xml 은 늘 쓴다 |

### 4.2 `workbook.bin`

`BrtBeginBook`, `BrtFileVersion`(guid 0, `HCell` / `13.0` / `13.0` / `0.711`), `BrtWbProp`(플래그 = 1904 날짜 체계만, 테마
버전 0, 코드 이름 `ThisWorkbook`), [`BrtBookProtection`], `BrtACBegin`(`01 00 03 0F 00 80`) `BrtAbsPath15`(저장 폴더,
끝에 `\`) `BrtACEnd`, `BrtBeginBookViews` `BrtBookView`(창 위치·크기·탭 비율·첫 탭·활성 탭, 플래그 0x38) `BrtEndBookViews`,
`BrtBeginBundleShs` `BrtBundleSh`×n (`hsState`, 시트 번호 1…, 관계 ID, 이름) `BrtEndBundleShs`, `BrtBeginExternals`
[연결들 + `BrtExternSheet`] `BrtEndExternals`, `BrtName`×n, `BrtCalcProp`(늘 `00000000 01000000 64000000` + 0.001 +
`00000000 2A00`), `BrtEndBook`.

- **연결(SupBook) 순서**: 외부 연결은 `[n]` 번호 순서, 이 통합 문서(`BrtSupSelf`)는 쓰일 때만 첫 외부 연결 바로 뒤 —
  관측한 다섯 꼴 `[외부1, 자신]`, `[외부1, 자신, 외부2]`, `[외부1, 외부2]`, `[자신]`, 없음이 모두 이 규칙이다.
- **XTI**: 수식에 처음 나오는 순서 — 이름 정의, 시트마다 셀·조건부 서식·유효성, 그리고 **차트 계열의 참조**
  (`Sheet1!$A$1:$A$5`). 차트만 있는 문서도 `BrtSupSelf` + `BrtExternSheet` 가 있는 까닭이다 (차트 문서 68개).
- **이름**: 모델 순서 그대로, `_xlnm.` 은 떼고 `fBuiltin`(0x20). 수식에 쓰였지만 정의되지 않은 이름은 수식 없는
  이름(cce 0)으로 덧붙인다 (`=XYZ` → `BrtName XYZ`). 주석은 0xFFFFFFFF.
- 보호가 걸린 통합 문서(구조 잠금)는 한셀이 ECMA-376 표준 암호화로 감싸 저장한다 (2절). rcell 은 감싸지 않는다.

### 4.3 시트 (`sheetN.bin`)

`BrtBeginSheet`, `BrtWsProp`, `BrtWsDim`, `BrtBeginWsViews` `BrtBeginWsView` [`BrtPane`] `BrtSel` `BrtEndWsView`
`BrtEndWsViews`, `BrtWsFmtInfo`, [`BrtBeginColInfos` `BrtColInfo`×n `BrtEndColInfos`], `BrtBeginSheetData` (행·셀)
`BrtEndSheetData`, [`BrtSheetProtection`], [자동 필터], [병합], [조건부 서식]×n, [유효성], [`BrtHLink`×n],
`BrtPrintOptions`, `BrtMargins`, `BrtPageSetup`, [머리글·바닥글], [행 나누기], [열 나누기], [`BrtDrawing`],
[`BrtLegacyDrawing`], 643·645 (빈 ActiveX 묶음), 660(개수 0)·662 (빈 표 목록), [x14 조건부 서식 확장], `BrtEndSheet`.

| 레코드 | 한셀 |
|---|---|
| `BrtWsProp` | 1바이트째 0x08 을 늘 켠다(+ 0x01 자동 나누기, 0x40 요약 행 아래, 0x80 요약 열 오른쪽). 2바이트째 0x04 늘(+ 0x01 페이지에 맞추기; 0x02 는 숨은 상태), 3바이트째 0x02 늘(+ 0x01 필터 적용 중). 탭 색이 없으면 `01 00 00 00 FF FF FF FF`, `rwSync`·`colSync` 0xFFFFFFFF, 코드 이름이 없으면 `SheetN` |
| `BrtBeginWsView` | 플래그 0x0300 늘(+ 0x02 수식, 0x04 눈금선, 0x08 머리글, 0x10 0 값, 0x40 선택), `icvHdr` 0x40, (24) 100. 기본 보기는 (20) = 현재 확대율, (22) = `zoomScaleNormal`, 페이지 나누기 미리 보기는 (20) = `zoomScaleNormal`, (22) = 현재 |
| `BrtSel` | 활성 창의 선택 **하나만** (XML 의 창마다 `selection` 중). `iRfxAct` = 활성 셀을 담은 범위의 번호 |
| `BrtWsFmtInfo` | 기본 열 너비 ×256, 8, 기본 행 높이(twips), 0, 개요 수준 0·0 (XML 의 `outlineLevelRow` 를 쓰지 않는다) |
| `BrtColInfo` | 너비 ×256, 서식, 플래그 (0x01 숨김, 0x02 너비 지정, 비트 8–10 수준, 접힘 **0x2000**) |
| `BrtRowHdr` | 높이는 지정이 아니어도 그 행의 높이(없으면 기본), 열 구간은 그 행 셀의 (첫 열, 끝 열) — XML 의 `spans` 그대로 |
| 셀 | 수는 부동소수 상위 30비트로 정확히 나타낼 수 있으면 `BrtCellRk`(부동소수 꼴만; 정수·×100 꼴은 쓰지 않는다 — RK 8218개 모두), 아니면 `BrtCellReal`. 글은 모두 공유 문자열(`BrtCellIsst`). 수식 셀은 값 종류대로 `BrtFmla*`, 휘발성 함수가 있으면 `grbitFlags` 0x02 |
| 공유 수식 | 기준 셀과 종속 셀 모두 `PtgExp`(행) + `rgcb` 의 열, 기준 셀 바로 뒤에 `BrtShrFmla`(범위 + 기준 셀에 대한 상대 토큰) |
| `BrtSheetProtection` | 잠기지 않은 셀 선택을 막으면 잠긴 셀 선택도 막는다 |
| 병합 | `BrtBeginMergeCells`(개수) — 순서는 XML 과 다르다 (숨은 상태, 해시 순서로 보인다) |
| 하이퍼링크 | RfX, 관계 ID(내부면 없음), 위치, 설명, 표시 글 |
| `BrtPrintOptions` | 0x10 은 숨은 상태: `printOptions` 가 없는 시트 104/108 에 켜고, 있는 시트 2/51 에 켠다 |
| `BrtPageSetup` | 해상도는 XML 의 dpi 그대로, 끝의 관계 ID 는 빈 문자열(cch 0) |
| 머리글·바닥글 | 글이 하나라도 있을 때만 쓴다 (빈 `headerFooter` 는 쓰지 않는다) |
| `BrtBrk` | 끝 칸: 행 나누기 1048575, 열 나누기 16383 (XML 의 `max` 와 반대) |
| 메모 | `commentsN.bin`: 작성자는 처음 나온 순서, 글은 RichStr (조각 글꼴은 styles.bin 의 글꼴 번호). VML 은 XML 과 같다 |

### 4.4 `styles.bin`

순서: [`BrtBeginFmts`…], 글꼴, 채우기, 테두리, 스타일 XF, 셀 XF, 스타일, `BrtBeginDXFs`(늘), `BrtBeginTableStyles`(늘).

| 항목 | 한셀 |
|---|---|
| 글꼴 | 계열·문자 집합이 없으면 2·0x81, 색은 RGB `05 00 00 00 R G B FF` |
| 채우기 | 무늬 색이 없으면 `05 40 00 00 00 00 00 FF`, 배경색이 없으면 `01 41 00 00 FF FF FF FF`, RGB 는 icv 칸에 0x40(무늬)·0x41(배경), 색인 64 는 `03 40 00 00 FF FF FF DF`. 그라데이션은 두 색 칸에 `01 40 … DF`·`01 41 … FF` |
| 테두리 | 선의 자동 색은 `01 00 00 00 FF FF FF DF` (서식 파일 683·조작 문서 33 선; 새 문서 15 선은 `FF` — 숨은 상태), 선 없음은 RGB 색 그대로, 자동·없음이면 검정 RGB |
| XF | 맞춤이 없으면 세로 아래(2), 보호가 없으면 잠금. 적용 비트는 XML 의 `apply*` 그대로 |
| 스타일 | 모두 `fBuiltIn`, 사용자 정의 스타일은 앞 스타일의 내장 번호, 수준 없음 0xFF |
| 차등 서식 | XFProp 은 채우기 → 글꼴 → 테두리 순, 묶음 안은 번호 순. `fNewBorder`(0x8000)는 위·아래·왼쪽·오른쪽을 모두 지정한 테두리. 속성 없는 글꼴은 0x18(이름)으로 자리를 남긴다 |
| 표 스타일 | 요소는 종류 번호 순 |

### 4.5 공유 문자열

XML 의 공유 문자열 순서를 지킨다 (서식 파일 28개 모두; 조작 문서 몇 개는 한셀이 순서를 바꾼다 — 숨은 상태).
`cstTotal` 은 XML 의 `count` 를 그대로 옮긴다 (실제 참조 수와 다를 수 있다).

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

### 4.7 조건부 서식·유효성·자동 필터

- 규칙은 **우선순위가 큰 것부터** 쓴다 (XML 순서와 상관없이). `BrtBeginConditionalFormatting` 의 개수 = 규칙 수.
- 틀: 셀 값 0, 식 1, 색 눈금 2, 데이터 막대 3, 아이콘 4, 상위/하위 5, 글 8, 빈 칸 9·0x0A, 오류 0x0B·0x0C, 날짜
  0x0F…, 평균 0x19 위·0x1A 아래·0x1D·0x1E, 중복 0x1B, **고유 값 7**. 평균 초과 규칙은 플래그 0x04.
- 색(`BrtColor` 564): `05 R 00 00 R G B FF` — icv 칸에 빨강 값, 흰색은 `01 FF 00 00 FF FF FF FF`.
- 데이터 막대: 기본 레코드(467)의 길이는 늘 10·90, 실제 길이는 시트 끝의 x14 확장(1051)에. 규칙 끝에
  `BrtFRTBegin(02 0E 00 00)` `BrtCFRuleExt(1146: 0 + GUID)` `BrtFRTEnd` 로 x14 규칙과 GUID 로 잇는다. 시트 끝 묶음:
  35, 1135, (1046 범위·규칙 수, 1048 종류 4·틀 3·GUID·1, 1051, 1050×n, 1156, 1049, 1047)×막대, 1136, 36. XML 에 x14
  확장이 없으면 한셀이 새 GUID 와 기본값(1051 뒷부분 `01 00 06 00`)으로 만든다.
- 유효성: `BrtBeginDVals`(0, 0, 0, 0, 개수), 플래그 = 종류 | 오류 모양<<4 | 0x100 빈 칸 | 0x200 목록 숨김 |
  0x40000 입력 메시지 | 0x80000 오류 메시지 | 연산자<<20. 수식 기준은 첫 범위의 왼쪽 위 셀.
- 자동 필터: 사용자 지정 문자열 조건은 `vts 6`, 값 칸 `00 00 cch 00 00 00 00 00` + 문자열. 상위/하위는 플래그에 0x04 를
  늘 켜고, 기준값(한셀이 데이터로 계산한 값)을 끝에 쓴다. 동적 필터의 5번째 바이트와 값(평균 등)도 한셀이 계산한다.

### 4.8 한셀의 바이너리 저장이 잃는 것과 rcell

2.1절의 손실 가운데 한셀의 바이너리 **읽기**는 되살리는 것이 있다 (rcell 이 쓴 바이너리를 .xlsb 로 열어 본 결과,
`tools/probe/binw-oracle.ps1`): 차등 서식의 맞춤, `draft`, 구조 잠금 없는 통합 문서 보호(암호, 창 잠금 비트는 읽지
않는다), 나눈 창의 활성 창, 요약 열 왼쪽. rcell 은 기본으로 이것들을 쓴다 (잃지 않는다). `Options::exact`
(`rcell convert … --exact`)는 한셀처럼 잃는다 — 그때 머리글·바닥글의 글꼴 코드 없는 구역에 기본 글꼴 코드도 넣는다.

차등 서식의 표시 형식은 한셀의 바이너리 읽기가 제대로 읽는 꼴이 없다 (대조 실험: 형식 문자열 0x26 → 빈 형식,
내장 번호 0x29 → 다른 형식, `BrtFmt` 에 올린 사용자 번호 0x29 → 무시). rcell 은 마지막 꼴로 쓴다: 한셀에는 해가 없고
rcell 은 `BrtFmt` 에서 형식을 찾아 다시 읽는다.

### 4.9 숨은 상태 (레코드 대조에 남는 차이)

XML 이 같은데 바이너리가 다른 칸 — rcell 은 가장 흔한 값을 쓴다: `BrtWsProp` 2바이트째 0x02, `BrtBeginWsView` 의
(22)·(24), `BrtPrintOptions` 0x10, 테두리 자동 색의 `DF`/`FF`, 머리글·바닥글 여백의 HWPUNIT 하나, 병합과 공유 문자열의
순서, 몇몇 서식 파일의 XTI 순서와 쓰이지 않는 XTI, XF 의 순서가 바뀐 문서 하나, 쓰이지 않는 글꼴 하나, 차등 서식의
이중 회계 밑줄(XML 은 단일 회계로 쓴다), 데이터 막대의 GUID 와 XML x14 확장의 세부(rcell 모델에 없다), 상위/하위·동적
필터가 데이터로 계산한 값, `BrtAbsPath15` 의 폴더.
