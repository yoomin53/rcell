<!--
  Copyright 2026 yoomin53 (https://github.com/yoomin53)
  SPDX-License-Identifier: Apache-2.0
-->

# 한셀 레거시 차트 레코드 0x7EF0

> 작성: yoomin53 · 대상: 한셀 2024 레거시 `.cell` (`xlCell8`, -2005) 의 시트 차트
>
> 한셀이 쓴 차트 레코드 77개(만든 차트 15, 대조 실험 54, 한셀 서식 파일 차트 8)만으로 얻은 **관찰 결과**다.
> 근거 수준 표기는 [`cell-legacy.md`](cell-legacy.md) 와 같다. 구현: `src/legacy/vtchart.rs` (필드 배치 표 `CLASSES`),
> 검증: 같은 파일의 시험 — `RCELL_CORPUS` 를 주면 말뭉치의 레거시 파일 117개를 모두 열어 0x7EF0 77개를 읽는다.

| | |
|---|---|
| 상태 | 77 / 77 이 문법대로 끝까지 읽힌다: 아카이브는 모두 길이 필드가 가리키는 자리에서 끝나고, 레코드는 꼬리까지 남김없이 소비된다 (관측) |
| 클래스 | 39개, 클래스마다 스키마 판 하나 |
| 객체 | 차트마다 365–608개 (계열 하나짜리 막대 404개), 모두 31,668개, 앞 객체 참조 2,450개, 객체 중첩 최대 13단계 |
| 추측 | 없음: 찾기·건너뛰기·다시 맞추기 없이 배치 표를 순서대로 따른다 |
| 읽기 연결 | 아직 하지 않았다 (`src/legacy/reader.rs` 는 앵커만 읽는다) |

모든 정수는 리틀 엔디언, "twip" = 1/20 pt.

## 1. 레코드 틀

CONTINUE 를 합친 논리 레코드 하나가 차트 하나다 (한셀은 긴 레코드도 나누지 않고 확장 길이를 쓴다, `cell-legacy.md` 3절).
시트 레코드 사이에서 다른 그리기 개체보다 먼저 오고, 끝 표시 0x7EF8 (`ff ff ff ff`) 이 뒤따른다.

| 위치 | 크기 | 필드 | 값 (관측) |
|---|---|---|---|
| 0x00 | u32 | 판 | 늘 1 |
| 0x04 | u16 | 종류 | 늘 0x67 |
| 0x06 | u16 | 차트 번호 | 만든 파일 0 (한 시트의 두 차트도 둘 다 0), 서식 파일 4–8·1–3 — 뜻은 모름 |
| 0x08 | 22 | 앵커 | `flags u16` (늘 0x0606), `col1 u16, dx1 u16, row1 u32, dy1 u16, col2 u16, dx2 u16, row2 u32, dy2 u16` |
| 0x1E | 11 | 예약 | 늘 0 |
| 0x29 | u16 | | 늘 0x0011 |
| 0x2B | u16 + cce | 원본 범위 수식 | 2절. 하나의 범위로 말할 수 없으면 cce 0 |
| … | u32 | 아카이브 길이 | |
| … | 길이 | Vt 아카이브 | 3절. 늘 `00000000 00000000 0800 "VtChart\0" 0600` 으로 시작 |
| … | 나머지 | 꼬리 | 5절 |

앵커의 `col/row` 는 0부터의 셀 번호로, 한셀이 같은 파일을 XML 로 읽은 결과의 `xdr:from`/`xdr:to` 와 같다.
`dx/dy` 는 만든 파일에서 0, 서식 파일에서 0x7FFF 까지 — 단위는 확인하지 않았다. 앵커 편집 방식(`editAs`)은 저장되지
않는다 (`c2_e12_anchor_abs/one` 이 기준 파일과 같은 바이트).

## 2. 수식

`cce u16` + BIFF8 꼴 RPN 토큰. 행은 u32, 열은 u16 (한셀 레거시 폭, `cell-legacy.md` 7절).

| 토큰 | 이름 | 피연산자 | 표기 |
|---|---|---|---|
| 0x3A (0x5A, 0x7A) | Ref3d | `ixti u16, 행 u32, 열 u16` (8) | `시트!$C$1` |
| 0x3B (0x5B, 0x7B) | Area3d | `ixti u16, 첫 행 u32, 끝 행 u32, 첫 열 u16, 끝 열 u16` (14) | `시트!$C$2:$C$6` |
| 0x10 | Union | — | 피연산자를 `,` 로 잇고, 합집합이 있으면 수식 전체를 `( )` 로 감싼다 |
| 0x17 | Str | `cch u16, 플래그 u8 (비트 0 = UTF-16), 글자` | `"글자"` (글자로 쓴 계열 이름) |
| 0x15 | Paren | — | `( … )` (읽지만 표본에는 없음) |

- 표본에 나온 토큰: 0x3A, 0x3B, 0x5B (`c2_e10_str_cat`), 0x10, 0x17. 모두 절대 참조라 열 필드의 상대 비트는 BIFF8 처럼
  하위 14비트 = 열, 0x4000 열 상대, 0x8000 행 상대로 본다 (추정).
- `ixti` 는 표본마다 통합 문서의 시트 순서 번호와 같다 (만든 파일 0 = `Sheet1`, 서식 파일 1 = 둘째 시트 `서식`). 같은
  파일의 EXTERNSHEET 로 풀어도 수식 참조 415개 모두 `XTI[ixti]` 의 첫·끝 시트가 `ixti` 와 같다 — 어느 쪽으로 풀어도
  결과가 같다 (관측). rcell 은 목록 밖 번호를 `#REF!` 로 쓴다.

## 3. Vt 아카이브

객체 그래프 하나를 깊이 우선·전위 순서로 쓴 것이다. 처음은 뿌리 `VtChart` 를 가리키는 포인터다.

### 3.1 포인터와 객체 번호

```
포인터 := u32 번호
  0xFFFFFFFF          없음
  번호 < 다음 번호     이미 다 읽은 객체 (앞 객체 참조)
  번호 = 다음 번호     새 객체: 다음 번호 += 1, 그다음 클래스 참조 + 본문
  그 밖               오류
```

번호는 읽는 순서대로 0(뿌리)부터 붙는다. 새 객체의 번호는 클래스 참조와 자식보다 먼저 정해지므로 자식은 언제나 부모보다
번호가 크다. 아직 다 읽지 않은 객체(자기 조상)를 가리키는 일은 없다. 앞 객체 참조는 같이 쓰는 문자열(글꼴 이름, 데이터 레이블
글이 다시 쓰는 계열 이름)에 나온다.

### 3.2 클래스 참조

```
클래스 참조 := u32 클래스 번호
  번호 < 정의된 수     정의된 클래스
  번호 = 정의된 수     정의가 붙는다: u16 길이 (NUL 포함), ASCII 이름 + NUL, u16 스키마 판; 정의된 수 += 1
```

클래스 번호는 처음 쓰일 때 붙으므로 문서마다 다를 수 있다 (`VtString` 은 대개 5, 데이터 표가 빈 `Marketing_Budget_1/2`
에서는 12).

### 3.3 번호 없이 박힌 객체 (기반 클래스 부분)

포인터 없이 클래스 참조 + 본문만 오면 그 자리에 값으로 박힌 객체다. **기반 클래스 부분**이 이렇게 온다: `VtString` 본문은
`VtValue` 부분으로 끝나고, 그것은 다시 `VtObject` 부분(빈 본문)으로 끝난다 — 16진으로 흔히 보이는 `06000000 04000000`
(VtValue, VtObject 의 클래스 번호) 이 그것이다. 기반 부분이 본문의 어디에 오는지는 클래스마다 다르다:

| 기반 부분이 맨 앞 | 기반 부분이 맨 뒤 |
|---|---|
| VtMatrix, VtDataGrid, VtTextFormat, VtFormatGroup, VtFormatter, VtDigitSet, VtWindow | 그 밖 모두 |

`VtCollection`·`VtArray`·`VtList` 는 머리와 항목 **사이**에 둔다 (`…개수, VtObject, 항목…`). 모든 클래스의 기반 사슬은
`VtObject` 에서 끝나므로 번호 있는 객체 수 = `VtObject` 부분 수 (표본 31,668).

조건에 따라 있는 필드는 하나뿐이다: `VtAxis.xy == 1` 이면 `XYScale` 24바이트가 붙는다. 그 밖의 길이 차이는 모두 포인터
(없음 / 앞 객체 참조 / 새 객체), 개수 필드로 정해지는 배열, 문자열 길이에서 온다.

### 3.4 기본 부호화

| 이름 | 부호화 |
|---|---|
| VtColor (6) | `u16 종류` (늘 2), `u8 R, G, B`, `u8 플래그` — 플래그는 거의 0, 계열 색 사본(`VtSeries.markerPen/color2/color3`) 0x80, `VtSeries.pen5/markerColor` 0x20 (뜻은 모름) |
| VtString 내용 | `u32 cb` + cb 바이트 = cp949 글자 + `00`, 바이트 `00` 하나, UTF-16LE 글자 + `0000`. 표본의 문자열 5,218개 모두 두 부호화가 같다 → rcell 은 UTF-16 부분을 쓴다. 내용 뒤 VtString 본문에는 **초기화되지 않은 바이트** 하나(`pad`, 값은 아무것)가 기반 부분 앞에 온다 |
| VtDouble | `f64 값, u16 플래그 (늘 0xFFFF)` + 기반 부분 |
| VtBytes | `u32 길이` + 바이트 (늘 0) + 기반 부분 |

### 3.5 끝과 자체 검사

뿌리 `VtChart` 가 끝나는 자리 = 아카이브 시작 + 길이 (77 / 77). 그 밖에 늘 성립하는 것: 포인터 대상의 클래스가 배치 표의
기대 클래스와 같다, `VtArray.nonNull` = 없음이 아닌 항목 수, `VtCollection.count` = 데이터 표의 빈 칸 아닌 칸 수,
`VtMatrix` 폭·높이 = 이름표 수 + 데이터 수, VtColor 종류 = 2, VtString 내용이 위 모양이다. rcell 은 클래스가 다르면 오류,
나머지는 경고로 남긴다 (표본에서 경고 0).

## 4. 객체 나무

계열 하나짜리 막대 차트 (객체 404개). `→` 포인터, `:` 기반 부분.

```
#0 VtChart : VtWindow
 ├ dataGrid   → VtDataGrid : VtMatrix : VtCollection        cells → VtString / VtDouble (열 우선)
 ├ backdrop   → VtBackdrop{fill → VtFill{picture → VtPicture}}          (차트 영역)
 ├ footnote   → VtChartFootnote : VtChartText{textBlock → VtTextBlock} : VtChartSection{backdrop}
 ├ legend     → VtChartLegend{font → VtFont} : VtChartSection{backdrop}
 ├ plot       → VtChartPlot
 │    ├ ptr_a   → VtArray (빔)
 │    ├ light   → VtLight3{sources → VtArray[VtInfLight3 ×2]}
 │    ├ axisX   → VtAxis{bytes → VtBytes, title → VtTextBlock, labels → VtArray[VtAxisScaleBlock]}
 │    ├ axesY   → VtArray[VtAxis]
 │    ├ axesY2  → VtArray[VtAxis]            (숨은 보조 축)
 │    ├ axisZ   → VtAxis
 │    ├ surface → VtSurfaceDesc{list → VtArray, axis → VtAxis, item1/2 → VtCLineItem}
 │    └ : VtChartSection{backdrop}            (그림 영역)
 ├ seriesList → VtArray[VtSeries ×n]
 │    VtSeries{points → VtArray, name → VtString, seriesLabel → VtSeriesLabel : VtTextBlock,
 │             textBlock → VtTextBlock, valueFormat/percentFormat → VtTextFormat,
 │             dataLabel → VtSeriesLabel, picture → VtPicture}
 ├ title      → VtChartTitle : VtChartText : VtChartSection
 ├ list       → VtList (빔)
 └ ssLink     → VtChSsLink{dataSource → VtDataSourceSsLink : VtDataSource}

VtAxisScaleBlock{slots → VtArray(없음 ×5)} : VtValueBlock{textFormat → VtTextFormat, formatCode → VtString} : VtTextBlock
VtTextFormat{group → VtFormatGroup{section1..4 → VtFormatter{code → VtString, digits1..3 → VtDigitSet}}}
VtTextBlock{backdrop → VtBackdrop | 없음, font → VtFont{name → VtString}, text → VtString | 없음}
```

계열 목록에는 실제 계열 뒤에 기본 자리 계열(이름 `행2`…`행5`, `열2`…)이 이어진다: 계열이 1–5개면 5개, n > 5 면 n개
(`Marketing_Budget_1/2` 는 4개, 꼬리의 계열 0). 원형 차트는 항목마다 하나다. 실제로 쓰는 계열 수는 꼬리(5절)와
`VtDataGrid.cols` 가 알려 준다.

## 5. 꼬리

```
꼬리 := 수식 항목              ; cce 0 = 없음
        u16 계열 수
        계열 수 × (수식 값, 수식 이름)   ; 이름: Ref3d, Str (글자) 또는 cce 0
```

예: 계열 하나짜리 막대는 항목 `Sheet1!$A$2:$A$6`, 값 `Sheet1!$C$2:$C$6`, 이름 `Sheet1!$C$1`. 글자 이름은 `"이름XY"`
(ptgStr). 거품형 실험은 계열 둘 중 둘째의 값·이름이 비어 있다. `Marketing_Budget_1/2` 는 항목 없음, 계열 0. 꼬리는 언제나
레코드 끝에서 끝난다.

## 6. 대조 실험으로 확정한 뜻

근거 = 기준 파일과 이름의 속성 하나만 다른 실험 파일 (읽은 나무의 차이).

| 필드 | 뜻 | 근거 |
|---|---|---|
| `VtChart.chartType` | 0 3차원 막대, 1 막대(세로), 2 3차원 꺾은선, 3 꺾은선, 4 3차원 영역, 5 영역, 11 가로 막대, 13 3차원 원형, 14 원형, 15 도넛형, 18 방사형. 분산형·거품형은 3, 주식형은 1 로 저장 | `chart_*`, `c2_e4_*`, `c2_e6_*` |
| `VtChart.mask` | 0xFFFF; 영역·누적·100% 와 서식 파일 0xFF3F; 가로 막대 0xFFCF | `chart_area/stacked/barh`, `c2_e3_*` |
| `VtChartSection.visible`, `.location` | 범례 보임; 범례 위치 1 위, 3 왼쪽, 4 오른쪽 (오른쪽 위도), 6 아래; 각주 5 | `chart_nolegend`, `c2_e1_leg_*`, `chart_legendb` |
| 제목 | `VtChartTitle` 의 visible 1, `textBlock.u16_b` 1, 글, 글꼴 12pt 스타일 3 | `chart_title` |
| 축 제목 | `VtAxis.u16_a` 1→0, `title.u16_b` 0→1, 글꼴 9→10pt 스타일 3, 글 | `c2_e7_ax_titles` |
| `VtAxis.hidden` | 축 삭제 | `c2_e7_cat_del`, `c2_e7_val_del` |
| `VtAxis.majorGridPen` | 주 눈금선이 있으면 style 1, 색 808080 | `<c:majorGridlines/>` 가 있는 `c2_e7_*` 원본 |
| `VtAxis.tickStyle` | 주 눈금: 보이는 X·Y 축 3 (바깥, 기본); Y2·Z, 서식 파일 X 축, 눈금 실험의 Y 축 1; 표면 축·원형 X 축 0 | `c2_e7_tick_none` (Y 3→1), 원형 |
| `VtAxis.u16_r` | 100% 누적 차트의 값 축 2 | `c2_e3_*_pct` |
| `VtAxis.xy` | 분산형·거품형의 X 축 1, `XYScale` 24바이트가 붙는다 | `chart_scatter`, `c2_e6_scatter_*`, `c2_e6_bubble` |
| `VtChartPlot.overlap` | −겹치기/100 (누적·100% 막대 −1) | `chart_stacked`, `c2_e3_bar*` |
| `VtChartPlot.gap` | 간격 너비/100: 막대 1.5 (종류 0, 1, 11; 주식형 실험 0.5), 그 밖 1, 도넛형 0 | 전부 |
| `VtChartPlot.rotation`, `.elevation` | 3차원 rotY, rotX (도): 45/30 ← rotY 45 rotX 30 | `c2_e4_bar3d_a/_b` |
| `VtSeries.brush.fill`, `.linePen.color` | 계열 색 (기본 팔레트 73b1d1, cd6565, ebc452, 90c968, 7581c8, e585b0, 8fc1da, d78484) | `chart_color` (ff0000) |
| `VtSeries.edgePen` | 계열 테두리: style 1, 색, 두께 20 twip | `c2_e5_ser_pen` (00ff00) |
| `VtSeries.showLine`, `.markerStyle`, `.markerSize`, `.u16_d` | 2차원 꺾은선·분산형·방사형 showLine 1; 표식 지정: markerStyle 5, 크기 180, u16_d 0 | `chart_line`, `c2_e6_*`, `c2_e5_line_marker` |
| `VtSeries.labelComponents`, `.dataLabel`, `.u16s3[8]` | 데이터 레이블 구성 1 값, 2 백분율, 8 항목; 레이블 글 상자 `u16_b` 1 + 자기 배경; `u16s3[8]` 1 (원형 백분율 2) | `c2_e8_dl_*` |
| `VtSeries.u16s2[4..7]` | 계열 순서·누적 묶음 | `chart_stacked`, `c2_e3_*` |
| `VtDataGrid` | 이름표·데이터 수; 행렬은 열 우선, 0열 = 항목 이름표, 0행 = 계열 이름, [0][0] 없음 | 전부 |
| `VtFont.size` | pt (범례·축 제목 9, 이름표 10, 제목 12, 데이터 레이블 8) | 전부 |

**한셀의 레거시 저장이 쓰지 않는 것** (기준 파일과 같은 바이트): 제목 글꼴 이름·크기·색·굵게·기울임·밑줄과 수동 배치
(`c2_e2_*`, `c2_e1_title_manual`), 차트 영역·범례·그림 영역 채우기 (`c2_e11_*`), 범례 겹침, 축 최소·최대·주 단위·거꾸로·로그
눈금 (`c2_e7_val_scale`, `c2_e7_val_log`), 보조 눈금선 (`c2_e7_grid_minor`), 앵커 편집 방식 (`c2_e12_anchor_*`).
이런 차트는 레거시에서 기본값으로만 다시 만들 수 있다.

## 7. 클래스 배치

묶음(클래스 표시 없이 그 자리에 오는 구조):

| 묶음 | 바이트 | 배치 |
|---|---|---|
| Pen | 22 | `u16_a` (1), `style` (0 없음, 1 실선, 9 등고선 항목), `u16_b`, `u16_c` (계열 선 2), `color`, `width` f32 (twip, 20 = 1pt), `limit` f32 (3.0) |
| Brush | 16 | `style` (1 단색, 0 없음), `fill` 색, `pattern` 색 (8c8c8c), `index` u16 (0) |
| Gradient | 14 | `style`, `from` 색 (ffffff), `to` 색 (80c0ff) |
| Shadow | 26 | `style`, Brush, `offsetX` f32, `offsetY` f32 (100) |
| Frame | 18 | `style`, `color`, `spaceColor`, `width` f32 (20) |
| XYScale | 24 | f64 ×2, u16 ×3, u8 ×2 |
| Wall | 44 | `u16_a`, Brush, Pen, `width` f32 |
| Floor | 44 | `u16_a`, Brush, Pen, `u16_b`, `u16_c` |

클래스 (흔한 차트의 클래스 번호 순서, 판은 표본의 스키마 판). 적지 않은 필드는 u16, `→` 는 포인터(뒤는 기대 클래스),
**기반** 은 기반 부분의 자리, `[n]` 은 u16 n개. 필드 이름은 `vtchart.rs` 의 표와 같다.

| 클래스 (판) | 본문 순서 |
|---|---|
| VtChart (6) | →dataGrid VtDataGrid, →backdrop VtBackdrop, →footnote VtChartFootnote, →legend VtChartLegend, →plot VtChartPlot, →seriesList VtArray, →title VtChartTitle, →list VtList, u16_a, u16_b, f32_a f32, →ssLink VtChSsLink, mask, u16_c–u16_f, chartType, u32_a u32, **기반** VtWindow |
| VtDataGrid (1) | **기반** VtMatrix, labelCols, labelRows, rows, cols, u16_a, reserved[8] |
| VtMatrix (1) | **기반** VtCollection, width, height, →cells[width × height] VtString·VtDouble |
| VtCollection (1) | count, **기반** VtObject |
| VtObject (1) | (빈 본문) |
| VtString (2) | text (내용, 3.4절), pad u8, **기반** VtValue |
| VtValue (1) | **기반** VtObject |
| VtDouble (1) | value f64, flags, **기반** VtValue |
| VtBackdrop (1) | u16_a, shadow Shadow, u16_b, frame Frame, u16_c (차트 영역 1), →fill VtFill, **기반** VtObject |
| VtFill (1) | style, brush Brush, gradient Gradient, u16_a, →picture VtPicture, u16_b, **기반** VtObject |
| VtPicture (1) | u16_a, u16_b, →data (늘 없음), **기반** VtObject |
| VtChartFootnote (1) | **기반** VtChartText |
| VtChartText (1) | →textBlock VtTextBlock, **기반** VtChartSection |
| VtTextBlock (2) | u16_a–u16_f, →backdrop VtBackdrop, →font VtFont, u16_g, u16_h (100), reserved1[10], →text VtString, reserved2[13], **기반** VtObject |
| VtFont (1) | →name VtString, size f32 (pt), style u32 (0, 3), color, **기반** VtObject |
| VtChartSection (1) | u16_a (160), u16_b (150), u16_c, visible, x1 y1 x2 y2 f32 (0), location, →backdrop VtBackdrop, **기반** VtObject |
| VtChartLegend (1) | →font VtFont, u16_a–u16_e, **기반** VtChartSection |
| VtChartPlot (5) | →ptr_a VtArray, f32_a, f32_b, overlap, gap, gapDepth, f32_c (f32), wall Wall, floor Floor, rotation, elevation, f32_d–f32_f (f32), u32_a u32, →light VtLight3, →axisX VtAxis, →axesY VtArray, →axesY2 VtArray, →axisZ VtAxis, u16_a, f32_g–f32_k (f32), u16_b, f32_l f32, u16_c, →surface VtSurfaceDesc, u16s[8], **기반** VtChartSection |
| VtArray (2) | size, u16_a, u32_a u32 (3), nonNull, **기반** VtObject, →items[size] |
| VtLight3 (1) | →sources VtArray, ambient f32, edge f32, edgeVisible, **기반** VtObject |
| VtInfLight3 (1) | x, y, z, intensity (f32), **기반** VtObject |
| VtAxis (6) | hidden, u16_a, →bytes VtBytes, u16_b, xy, axisPen, majorGridPen, minorGridPen (Pen), f32_a f32, u32_a u32, →title VtTextBlock, →labels VtArray, u16_c, f32_b f32, u16_d, f32_c f32, u16_e–u16_q (13개), f32_d f32, *xy = 1 이면* xyScale XYScale, u16_r, u16_s, tickStyle, reserved[7], **기반** VtObject |
| VtBytes (1) | data (u32 길이 + 바이트), **기반** VtValue |
| VtAxisScaleBlock (1) | →slots VtArray, u16_a, u16_b (Y 축 1), **기반** VtValueBlock |
| VtValueBlock (1) | →ptr_a (늘 없음), →textFormat VtTextFormat, u16_a, →formatCode VtString, u8_a–u8_c (u8), **기반** VtTextBlock |
| VtTextFormat (2) | **기반** VtObject, u16_a, →group VtFormatGroup |
| VtFormatGroup (2) | **기반** VtObject, u16_a, →section1–section4 VtFormatter |
| VtFormatter (3) | **기반** VtObject, u16_a–u16_d, u16s[9], color, →code VtString (내부 부호화한 서식 코드: G/표준 → `Zlg표g준n`), →digits1–digits3 VtDigitSet |
| VtDigitSet (2) | **기반** VtObject, u16_a, u16_b, u16_c |
| VtSurfaceDesc (1) | u16_a, brush Brush, pen Pen, reserved1[3], →list VtArray, →axis VtAxis, u16_b, →item1 VtCLineItem, →item2 VtCLineItem, u16s[7], **기반** VtObject |
| VtCLineItem (1) | reserved[4], u16_a, brush Brush, pen Pen, →ptr_a (늘 없음), **기반** VtObject |
| VtSeries (5) | linePen, edgePen (Pen), u16_a, brush Brush, →ptr_a (늘 없음), →points VtArray, u16s1[4], pen3 Pen, u16s2[18], markerPen Pen, u16_b, →name VtString, →seriesLabel VtSeriesLabel, →textBlock VtTextBlock, labelComponents, →valueFormat VtTextFormat, →percentFormat VtTextFormat, →dataLabel VtSeriesLabel, u16_c, u16_d, u16_e, pen5 Pen, markerSize f32, showLine, markerColor 색, →picture VtPicture, markerStyle, u16_f, u16_g, f32_a f32, color2 색, color3 색, index, typeCount u32 (27), typeTable[typeCount], u16s3[10], **기반** VtObject |
| VtSeriesLabel (1) | **기반** VtTextBlock |
| VtChartTitle (1) | **기반** VtChartText |
| VtList (2) | u32_a u32 (3), count (0), **기반** VtObject, →items[count] (추정, 표본에서 늘 빔) |
| VtChSsLink (2) | u16_a, →dataSource VtDataSourceSsLink, **기반** VtObject |
| VtDataSourceSsLink (1) | →ptr_a, →ptr_b (늘 없음), **기반** VtDataSource |
| VtDataSource (1) | **기반** VtObject |
| VtWindow (3) | **기반** VtObject, u16_a |

## 8. 모호한 점

- **늘 같은 값인 구간의 필드 경계.** 표본에서 값이 바뀌지 않는 구간(대부분 0)은 `u16[n]` 으로 적었다 (`reserved…`, `u16s…`).
  실제로 u16·u32·f32 중 무엇인지는 표본으로 알 수 없다. 값이 바뀌는 필드는 모두 꼴이 맞는지 보았다 (f32 비정규 수 없음,
  f32 의 반쪽처럼 보이는 u16 없음).
- **행렬 뒤 바이트의 주인.** `VtDataGrid` 는 기반 부분이 먼저라 `u16_a` (원형 0) 와 뒤의 16바이트 0 은 `VtChart` 의
  `dataGrid` 포인터 다음 필드로 볼 수도 있다. `VtList` 뒤의 `VtChart.u16_a/u16_b/f32_a` 도 마찬가지 (VtList 가 기반 부분
  뒤에서 끝난다고 보았다).
- **`XYScale` 의 자리.** 분산형 X 축의 24바이트는 `f32_d` 와 `u16_r` 사이에 둔다. 이웃이 0 이라 2바이트 뒤(`u16_r` 다음)에
  두어도 표본과 맞는다. 붙는 조건은 `xy == 1` 로 보았다 (축 385개에서 성립).
- **채워진 적 없는 그릇.** `VtList` 항목, `VtSeries.points`, `VtChartPlot.ptr_a`, `VtSurfaceDesc.list`, `VtPicture.data`,
  `VtBytes`, `VtValueBlock.ptr_a`, `VtDataSourceSsLink.ptr_a/ptr_b`, `VtCLineItem.ptr_a` 는 표본에서 늘 비었다. 이것들을
  채우는 파일은 표에 없는 클래스나 배치를 가질 수 있다 — 그때 rcell 은 추측하지 않고 "배치를 모르는 클래스" 나 클래스
  불일치 오류를 낸다.
- **뜻을 모르는 것.** `u16_*`·`f32_*` 필드 대부분, Pen 의 `u16_a/u16_b/u16_c`, VtColor 플래그, `VtSeries.typeTable`
  (27개, 늘 같음: 차트 종류별 계열 종류로 보인다), `VtFormatter` 내부와 서식 코드 부호화 (`Zlg표g준n` = G/표준,
  백분율 서식 `B0Jn`, 사용자 서식 `B0EB1B1Jn`), 광원 벡터, `VtChartSection.u16_a/u16_b` (160/150), 앵커 `dx/dy` 단위,
  차트 번호.
- **수식.** 상대 참조가 표본에 없어 상대 비트는 BIFF8 과 같다고 보았다. `ixti` 해석은 2절의 관측이다.
