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
| 다시 열기 | 한셀 2024 는 자기가 저장한 이 형식을 COM 자동화로 다시 열지 못한다 (서버 오류) |
| 미상 레코드 | 시트 끝의 643, 645, 660, 662 와 스타일의 505–509 (표 스타일) |
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
| 조건부 서식 | `BrtBeginConditionalFormatting`(461: ccf, fPivot, UncheckedSqRfX), `BrtBeginCFRule`(463: 종류·틀·dxfId·우선순위·매개 변수·플래그·수식 길이 3개·strParam·CFParsedFormula), `BrtBeginColorScale`(469)/`BrtBeginDatabar`(467)/`BrtBeginIconSet`(465), `BrtCFVO`(471), `BrtColor`(564). 차등 서식은 styles.bin 의 `BrtDXF`(507) — XFProp 목록 (한셀은 밑줄을 0x1C 로 쓴다) |
| 데이터 유효성 | `BrtBeginDVals`(573) 안의 `BrtDVal`(64): 플래그 u32 (종류 4비트, 오류 모양 3비트, 0x100 빈 칸 허용, 0x200 목록 숨김, 0x40000 입력 메시지, 0x80000 오류 메시지, 연산자 비트 20–23), UncheckedSqRfX, 문자열 4개(오류 제목, 오류, 입력 제목, 입력), 수식 2개 |

### 2.1 한셀의 바이너리 저장이 잃거나 바꾸는 것 (조작한 XML → 한셀 → 바이너리, 관측)

- 조건부 서식의 차등 서식에서 표시 형식이 사라진다. "시작 문자" 규칙의 글자(`strParam`)를 수식의 다른 부분으로 잘못 쓴다.
- 요약 열 왼쪽(`summaryRight="0"`)을 잃는다 (`BrtWsProp` 0x80 을 언제나 켠다). 구조 잠금이 없는 통합 문서 보호
  (창 잠금·암호만)는 `BrtBookProtection` 째로 저장하지 않는다.
- 보호가 걸린 문서는 표준 암호화(ECMA-376, AES-128, 기본 암호 `VelvetSweatshop`)로 저장한다 — rcell 은 투명하게 푼다.
- 외부 연결의 상태를 잃는다: 값 캐시(`BrtExternTableStart` … 를 쓰지 않아 `refreshError` 도 사라짐)와
  경로를 잃은 연결의 표시(관계 종류 `xlPathMissing` → 보통의 `externalLinkPath`). 한셀은 이 둘에 따라 이름 참조의
  따옴표를 정하므로, 바이너리를 거친 문서는 이름의 표기가 달라질 수 있다.

- `draft`, `autoPageBreaks="0"` 이 저장되지 않는다.
- 머리글·바닥글에서 글꼴 코드가 없는 구역에 기본 글꼴 코드(`&"맑은 고딕,Regular"`)를 넣는다.
- 여백을 HWPUNIT(1/7200 인치)로 잘라 쓴다.
- 나눈 창(고정 아님)의 활성 창 `bottomLeft` 를 잃는다 (`pnnAct` 0, 선택은 창 3).
- **보조 확대율**: `BrtBeginWsView` 의 (20)·(22) 와 XML 의 `zoomScaleNormal`·`zoomScaleSheetLayoutView`
  사이에 일관된 대응이 없다. XML 이 같은 두 시트가 바이너리에서 다른 값을 갖기도 한다 — 문서에 드러나지 않는
  상태가 있다. rcell 은 [MS-XLSB] 순서(20 = 기본, 22 = 페이지 나누기 미리 보기)로 읽고, 비교(`rcell diff`)에서는
  현재 확대율만 본다.

## 3. 검증

같은 문서를 XML 형식과 이 형식으로 함께 저장해(`tools/probe/make-corpus.ps1`) 비교한다.
한컴 확장을 뺀 나머지는 샘플 8종과 서식 파일 28종 모두 일치한다:

```
rcell diff X.bin.cell X.cell --ignore-hancom
```
