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
| `xl/tables/tableN.bin`, `xl/pivotTables/pivotTableN.bin`, `xl/pivotCache/pivotCacheDefinitionN.bin`·`pivotCacheRecordsN.bin` | 표(`BrtBeginList` 343 …)·피벗 — 레코드 파트. rcell 은 아직 읽지 않는다 |

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
| 이름 | `BrtName` 의 `fBuiltin`(0x20) 이면 `Print_Area` 같은 내장 이름 → XML 의 `_xlnm.Print_Area`. `itab` 은 시트 범위(0xFFFFFFFF 통합 문서), 수식 뒤 `comment`(XLNullableWideString)가 XML `comment` |
| 배열 수식 | 왼쪽 위 셀의 `BrtFmla*` 뒤에 `BrtArrFmla`(426: RfX, 플래그 u8, 수식). 범위 안 셀은 모두 `PtgExp`(왼쪽 위의 행 + PtgExtraCol 열) 수식 셀이다 (수식 대조 실험) |
| 하이퍼링크 | `BrtHLink`(494: RfX, 관계 ID XLNullableWideString, 위치·화면 설명·표시 글 XLWideString). 한셀은 관계 ID·위치가 없는 링크에 **앞 레코드의 값을 그대로 남긴다** (하이퍼링크 대조 실험: 위치만 있는 링크에 앞 링크의 `rId1`, 주소만 있는 링크에 앞 링크의 위치) — rcell 은 앞 레코드와 한쪽만 같은 값을 남은 값으로 보고 버린다 (주소·위치가 함께 바뀌는 링크는 그대로) |
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
- 보호가 걸린 문서는 표준 암호화(ECMA-376, AES-128, 기본 암호 `VelvetSweatshop`)로 저장한다 — rcell 은 투명하게 푼다.
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
  상태가 있다. rcell 은 [MS-XLSB] 순서(20 = 기본, 22 = 페이지 나누기 미리 보기)로 읽고, 비교(`rcell diff`)에서는
  현재 확대율만 본다.

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
