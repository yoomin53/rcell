<!--
  Copyright 2026 yoomin53 (https://github.com/yoomin53)
  SPDX-License-Identifier: Apache-2.0
-->

# 한셀 레거시 차트 레코드 0x7EF0

> 작성: yoomin53 · 대상: 한셀 2024 레거시 `.cell` (`xlCell8`, -2005) 의 시트 차트
>
> 한셀이 쓴 차트 레코드 77개(만든 차트 15, 대조 실험 54, 한셀 서식 파일 차트 8)만으로 얻은 **관찰 결과**다.
> 근거 수준 표기는 [`cell-legacy.md`](cell-legacy.md) 와 같다. 구현: `src/legacy/vtchart.rs` (필드 배치 표 `CLASSES`),
> `src/legacy/vtchart_map.rs` (차트 모델로 옮기기, 9절), `src/legacy/vtbuild.rs`·`vtwrite.rs` (쓰기, 10절). 검증:
> `vtchart.rs` 의 시험 — `RCELL_CORPUS` 를 주면 말뭉치의 레거시 파일 117개를 모두 열어 0x7EF0 77개를 읽는다 — 과
> `tests/chart_xml.rs`·`tests/real_samples.rs` (9.10절, 10.6절).

| | |
|---|---|
| 상태 | 77 / 77 이 문법대로 끝까지 읽힌다: 아카이브는 모두 길이 필드가 가리키는 자리에서 끝나고, 레코드는 꼬리까지 남김없이 소비된다 (관측) |
| 클래스 | 39개, 클래스마다 스키마 판 하나 |
| 객체 | 차트마다 365–608개 (계열 하나짜리 막대 404개), 모두 31,668개, 앞 객체 참조 2,450개, 객체 중첩 최대 13단계 |
| 추측 | 없음: 찾기·건너뛰기·다시 맞추기 없이 배치 표를 순서대로 따른다 |
| 읽기 연결 | `src/legacy/reader.rs` 가 모든 시트를 읽은 뒤 레코드마다 차트 모델을 만들어 그 차트 도형(`Shape.chart`)에 붙인다. 77 / 77 이 한셀의 레거시 읽기(`X.legacy.as.cell`)와 차트 파트가 바이트까지 같다 (9절) |
| 쓰기 | 차트 모델 → 객체 나무(`vtbuild`) → 아카이브(`vtwrite`). XML 차트 70개 모두 한셀이 같은 XML 을 레거시로 저장한 나무와 같다 (VtString 의 pad 빼고), 읽은 아카이브 77개를 다시 쓰면 바이트까지 같다 (10절) |

모든 정수는 리틀 엔디언, "twip" = 1/20 pt.

## 1. 레코드 틀

CONTINUE 를 합친 논리 레코드 하나가 차트 하나다 (한셀은 긴 레코드도 나누지 않고 확장 길이를 쓴다, `cell-legacy.md` 3절).
시트 레코드 사이에서 다른 그리기 개체보다 먼저 오고, 끝 표시 0x7EF8 (`ff ff ff ff`) 이 뒤따른다.

| 위치 | 크기 | 필드 | 값 (관측) |
|---|---|---|---|
| 0x00 | u32 | 판 | 늘 1 |
| 0x04 | u16 | 종류 | 늘 0x67 |
| 0x06 | u16 | 차트 번호 | 문서에 그리기(MSODRAWINGGROUP)가 없으면 0, 있으면 그 시트의 OBJ 수 + 시트 안 차트 차례 (1부터) — 10.1절 |
| 0x08 | 22 | 앵커 | `flags u16` (늘 0x0606), `col1 u16, dx1 u16, row1 u32, dy1 u16, col2 u16, dx2 u16, row2 u32, dy2 u16` |
| 0x1E | 11 | 예약 | 늘 0 |
| 0x29 | u16 | | 늘 0x0011 |
| 0x2B | u16 + cce | 원본 범위 수식 | 2절. 하나의 범위로 말할 수 없으면 cce 0 |
| … | u32 | 아카이브 길이 | |
| … | 길이 | Vt 아카이브 | 3절. 늘 `00000000 00000000 0800 "VtChart\0" 0600` 으로 시작 |
| … | 나머지 | 꼬리 | 5절 |

앵커의 `col/row` 는 0부터의 셀 번호로, 한셀이 같은 파일을 XML 로 읽은 결과의 `xdr:from`/`xdr:to` 와 같다.
`dx/dy` 는 그리기 개체의 ClientAnchor 와 같은 단위다: dx = 픽셀 × 32768 / (열 픽셀 − 1), dy = 픽셀 × 8192 / (행 픽셀 − 1)
([`cell-legacy.md`](cell-legacy.md) 10.8절 — 서식 파일의 `(1,47)+(3390,4630)` 같은 앵커가 이 식으로 쓴 것과 같다). 앵커
편집 방식(`editAs`)은 저장되지 않는다 (`c2_e12_anchor_abs/one` 이 기준 파일과 같은 바이트).

## 2. 수식

`cce u16` + BIFF8 꼴 RPN 토큰. 행은 u32, 열은 u16 (한셀 레거시 폭, `cell-legacy.md` 7절).

| 토큰 | 이름 | 피연산자 | 표기 |
|---|---|---|---|
| 0x3A (0x5A, 0x7A) | Ref3d | `ixti u16, 행 u32, 열 u16` (8) | `시트!$C$1` |
| 0x3B (0x5B, 0x7B) | Area3d | `ixti u16, 첫 행 u32, 끝 행 u32, 첫 열 u16, 끝 열 u16` (14) | `시트!$C$2:$C$6` |
| 0x10 | Union | — | 피연산자를 `,` 로 잇고, 합집합이 있으면 수식 전체를 `( )` 로 감싼다 |
| 0x17 | Str | `cch u16, 플래그 u8 (비트 0 = UTF-16), 글자` | `"글자"` (글자로 쓴 계열 이름) |
| 0x15 | Paren | — | `( … )` (읽지만 표본에는 없음) |

- 표본에 나온 토큰: 0x3A, 0x3B, 0x5B (`c2_e10_str_cat` 의 참조 하나짜리 원본 범위), 0x10, 0x17. 모두 절대 참조라 열 필드의 상대 비트는 BIFF8 처럼
  하위 14비트 = 열, 0x4000 열 상대, 0x8000 행 상대로 본다 (추정).
- **원본 범위의 `ixti` 는 시트 번호, 항목·계열 수식의 `ixti` 는 EXTERNSHEET 의 XTI 번호다** (차트 수식 ixti 대조
  실험: 첫 시트 `Sheet1` 의 차트가 둘째 시트 `Data` 의 칸을 가리키는 XML 을 한셀이 레거시로 저장하면 원본 범위
  `3b 01 00 …` (ixti 1), 항목·계열 ixti 0, EXTERNSHEET 는 XTI 하나 (내부, 1, 1)). 말뭉치의 수식 참조 415개는 모두
  `XTI[ixti]` 의 첫·끝 시트가 `ixti` 와 같아 두 해석이 구별되지 않았다. rcell 은 풀 수 없는 번호를 `#REF!` 로 쓴다.

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
| VtColor (6) | `u16 종류` (늘 2), `u8 R, G, B`, `u8 플래그` — 플래그는 거의 0, 계열 색 사본(`VtSeries.markerPen/color2/color3`) 0x80, `VtSeries.pen5/markerColor` 0x20 (뜻은 모름 — 한셀의 레거시 읽기는 보지 않는다, 9.5절) |
| VtString 내용 | `u32 cb` + cb 바이트 = cp949 글자 + `00`, 바이트 `00` 하나, UTF-16LE 글자 + `0000`. 표본의 문자열 5,218개 모두 두 부호화가 같다 → rcell 은 UTF-16 부분을 읽고, 쓸 때는 둘 다 쓴다 (cp949 에 없는 글자는 `?`). 내용 뒤 VtString 본문에는 **초기화되지 않은 바이트** 하나(`pad`, 값은 아무것 — 같은 문서를 한셀이 두 번 저장해도 다르다)가 기반 부분 앞에 온다. rcell 은 0 을 쓰고, `rec-diff --ignore-volatile` 은 이 바이트를 보지 않는다 |
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
| VtSeriesPoint (1) | →label VtSeriesLabel, explode f32 (쪼개기 비율, 0.25 = 25%), u16s[4], →element VtSeriesPointElement, →ptr_a (늘 없음), **기반** VtObject |
| VtSeriesPointElement (1) | u16_a, brush Brush (요소 채우기), edgePen Pen, →ptr_a (늘 없음), u16_b, markerPen Pen, markerSize f32, showLine, markerColor 색, →picture VtPicture, markerStyle, **기반** VtObject |
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
- **채워진 적 없는 그릇.** `VtList` 항목, `VtChartPlot.ptr_a`, `VtSurfaceDesc.list`, `VtPicture.data`,
  `VtBytes`, `VtValueBlock.ptr_a`, `VtDataSourceSsLink.ptr_a/ptr_b`, `VtCLineItem.ptr_a` 는 표본에서 늘 비었다. 이것들을
  채우는 파일은 표에 없는 클래스나 배치를 가질 수 있다 — 그때 rcell 은 추측하지 않고 "배치를 모르는 클래스" 나 클래스
  불일치 오류를 낸다. `VtSeries.points` 는 한셀이 COM 으로 만든 원형·도넛 차트(요소마다 색)를 레거시로 저장하면
  채워진다 (차트 종류 실험 73종: VtSeriesPoint 요소 서식 — 한셀 화면은 원형 조각 k 를 계열 목록 k 번째의 요소 서식 0 번
  칠 색으로 그리고, 범례 표지는 그 계열의 칠, 쪼개기는 explode).
- **뜻을 모르는 것.** `u16_*`·`f32_*` 필드 대부분, Pen 의 `u16_a/u16_b/u16_c`, VtColor 플래그 (0x20 은 한셀의 읽기가 보지 않는다 — 표식 채우기는 pen5 의 색, 9.5절
  읽힌다, 9.5절), `VtSeries.typeTable` (27개, 늘 같음: 차트 종류별 계열 종류로 보인다), `VtFormatter` 의 숫자 필드
  (서식 코드의 부호는 9.6절), 광원 벡터, `VtChartSection.u16_a/u16_b` (160/150).
- **수식.** 상대 참조가 표본에 없어 상대 비트는 BIFF8 과 같다고 보았다. `ixti` 해석은 2절의 대조 실험이다.

## 9. 차트 모델로 옮기기 — 한셀의 레거시 읽기

`src/legacy/vtchart_map.rs`. 목표는 한셀이 같은 레거시 파일을 열어 XML 로 저장한 차트 파트(`X.legacy.as.cell`)다: 옮긴
모델을 `chart_xml` 로 쓰면 그 파트와 바이트까지 같다 (77 / 77). 그래서 아래 표는 "저장된 값이 OOXML 의 무엇이 되는가" 이자
"한셀이 무엇을 버리고 무엇을 채우는가" 다. 한셀이 늘 같게 채우는 꼴(선의 끝 모양, 글 상자의 `a:bodyPr`, 차트 공간의 상수)은
[`cell-2024.md`](cell-2024.md) 11.2절. 근거 = 표본 77개의 필드별 대조 (관측), 표본에 없는 값은 (추정).

### 9.1 종류와 묶음

| 저장 | OOXML | 근거 |
|---|---|---|
| `VtChart.chartType` | 0 `bar3DChart` col, 1 `barChart` col, 2 `line3DChart`, 3 `lineChart`, 4 `area3DChart`, 5 `areaChart`, 11 `barChart` bar, 13 `pie3DChart`, 14 `pieChart`, 15 `doughnutChart`, 18 `radarChart`. 분산형·거품형(3)은 `lineChart`, 주식형(1)은 `barChart` 로 읽힌다 | 관측 |
| 표본에 없는 번호 | MSChart VtChChartType 이름에 가장 가까운 관측 종류: 단계형 6/7 → 꺾은선, 조합 8/9 → 막대, 10 → 3차원 가로 막대, 12 → 3차원 막대 (`clustered`), 16·19 → 꺾은선, 17 → 방사형, 20 → 막대, 21/22 → 가로 막대, 23/24 → 영역, 25/26 → 3차원 꺾은선, 그 밖 → 막대 | 추정 |
| 누적 | 쓰는 계열(꼬리의 계열 수, 적어도 1) 중 하나라도 `VtSeries.u16s2[6]` ≠ 계열 순서 또는 `u16s2[7]` ≠ 0 | 관측 |
| 100% | 값 축(`axesY[0]`)의 `u16_r` = 2 | 관측 |
| `c:grouping` | 2차원 막대 `percentStacked`/`stacked`/`clustered`, 2차원 꺾은선·영역 `percentStacked`/`stacked`/`standard`, 3차원 막대·꺾은선·영역은 누적이 저장되어 있어도 `standard` (서식 파일 8개는 원본이 `stacked`) | 관측 |
| `c:varyColors` | 원형·도넛만 1 (저장된 표시 없음 — 서식 파일 원본의 1 도 0 으로 읽힌다) | 관측 |
| `c:gapWidth` | 막대: `round(VtChartPlot.gap × 100)` (1.5 → 150, 주식형 실험 0.5 → 50) | 관측 |
| `c:overlap` | 2차원 막대만: `round(−overlap × 100)` (누적 −1 → 100) | 관측 |
| `c:gapDepth` | 3차원 막대·꺾은선·영역: `round(gapDepth × 100)` (1.5 → 150) | 관측 |
| `c:shape` | 3차원 막대 묶음에 `box` | 관측 |
| 꺾은선 묶음 | 2차원만 `c:marker 1`, `c:smooth 0` | 관측 |
| 원형 | 2차원 원형·도넛 `c:firstSliceAng 0`, 도넛 `c:holeSize` = `round(VtChartPlot.f32_k × 100)` (0.67 → 67) | 관측 |
| 방사형 | `c:radarStyle marker` | 관측 |
| `c:axId` | 원형이 아니면 0, 1 (3차원이면 2 도) | 관측 |

### 9.2 계열 고르기와 순서

꼬리(5절)의 계열 중 **값 수식이 있고** 계열 목록 항목의 `u16s2[3]` 이 0 인 것만 `c:ser` 가 된다 — 거품형의 둘째 꼬리
항목(거품 크기 열)은 둘 다 아니어서 빠지고, `c:bubbleSize` 도 쓰지 않는다. `c:idx`·`c:order` 는 남은 계열의 순서다.
계열 목록의 같은 자리 항목(`VtSeries`)이 서식을 준다. 계열이 없는 차트(서식 파일 `Marketing_Budget_1/2`)도 묶음·축은 쓴다.

### 9.3 데이터: 참조와 캐시

| OOXML | 값 | 근거 |
|---|---|---|
| `c:f` | 꼬리 수식 (2절, 시트 이름은 ixti 를 EXTERNSHEET 로 푼 시트) | 대조 실험 |
| `c:tx` | 이름 수식이 참조면 `strRef` + 캐시 하나 = 데이터 표 0행의 그 계열 칸 (이름표 행이 없으면 계열 목록의 이름). 글자로 쓴 이름(ptgStr)은 **한셀이 읽지 않는다** — `c:tx` 없음 (`chart_litname`) | 관측 |
| `c:cat` | 항목 수식. 참조한 칸 가운데 빈 칸이 아닌 것이 모두 숫자면 `numRef` + `formatCode` (칸 형식, 섞이면 `General`) — 점에는 형식을 쓰지 않는다. 아니면 `strRef`. `ptCount` = `VtDataGrid.rows`, 점 = 데이터 표 0열의 이름표 글 | 관측 |
| `c:val` | `numRef`, 점 = 데이터 표의 계열 열 (`labelCols + 꼬리 번호`), 점마다 `formatCode` = 그 칸의 형식. 빈 칸은 `<c:v/>` + `General`, 오류 칸은 `<c:v/>` + 그 칸의 형식 (`c2_e10_blank_na`: 데이터 표에는 둘 다 0 이 있다). 묶음 `formatCode` = 점 형식이 하나면 그것, 아니면 `General` | 관측 |
| 숫자 글 | 데이터 표의 f64 를 유효 숫자 15자리로 맞춘 최단 표기 (표본은 정수만) | 추정 |

칸 형식은 한셀이 차트에 쓰는 꼴이다: 서식 번호 0 → `G/표준`, 그 밖은 서식 코드에서 글자 따옴표와 `\` 를 벗기고 `[Red]` →
`[빨강]` (서식 파일: 번호 6 `"₩"#,##0;[Red]\-"₩"#,##0` → `₩#,##0;[빨강]-₩#,##0`, 번호 10 `0.00%`). 다른 색 이름은 표본에
없다.

### 9.4 계열 서식 (`c:spPr`)

| 갈래 | 채우기 | 선 | 근거 |
|---|---|---|---|
| 막대·영역 | `brush.fill` (칠 style 0 이면 채우기 없음, 추정) | `edgePen`: style 0 → 9525 선 없음, 아니면 두께·색·점선 | 관측 (`c2_e5_ser_pen` 00ff00 20 twip → 12700) |
| 꺾은선·방사형 (분산형·거품형 포함) | `brush.fill` | `linePen` 두께·색·점선 — style 과 `showLine` 은 보지 않는다 (거품형 style 0, 3차원 꺾은선 showLine 0 도 실선) | 관측 (40 twip → 28575) |
| 원형·도넛 | 쓰지 않는다 | — | 관측 |

두께: twip/20 pt 를 기본 두께 {0.25, 0.5, 0.75, 1, 1.5, 2.25, 3, 4.5, 6} pt 가운데 가장 가까운 것으로 (같으면 굵은 쪽), × 12700 EMU
(관측 20 → 12700, 40 → 28575; 나머지는 추정). 점선: Pen.style 1 실선 (관측), 2 dash, 3 sysDot, 4 dashDot, 5 sysDashDotDot,
7 lgDashDot, 8 lgDashDotDot (MSChart VtPenStyle 순서, 추정). 기본 팔레트는 6절.

### 9.5 계열의 나머지

| OOXML | 값 | 근거 |
|---|---|---|
| `c:invertIfNegative` | 막대 계열 0 | 관측 |
| `c:marker` | 꺾은선·방사형: `VtSeries.u16_d` 1 (자동) → `none` 크기 7. 0 (지정) → 모양 `markerStyle` (1 plus, 2 x, 3 star, 4·9 circle, 5 square, 6 diamond, 7 triangle 관측 — 표식 모양 실험의 한셀 읽기; 나머지는 MSChart VtMarkerStyle 순서 0 dash, 8 triangle, 10 square, 11 diamond, 12·13 triangle, 14 circle, 15 none — 추정), 크기 `round(markerSize / 20)` (180 → 9, 600 → 30), **`spPr` 채우기 = `pen5` 의 색, 검정(000000)이면 `spPr` 없음** — 색 플래그(0x20)·pen5 의 선 모양·계열 칠·`markerColor` 는 보지 않는다 | 관측: 거품형 표식 실험 (한셀의 `c2_e6_bubble` 레거시에서 필드 하나씩 바꾼 사본 18개를 한셀이 읽은 것: pen5 색 ff0000 → 표식 ff0000, 000000 → 채우기 없음, 010101·ffffff 는 그 색; 꺾은선 `c2_e5_line_marker` 의 pen5 를 123456 으로 → 123456, 플래그 0x20 이어도. pen5·markerColor 플래그, pen5 선 모양, 계열 칠, `u16s1[0]`, `typeTable`, 선 모양, 표식 선 색을 바꿔도 그대로) |
| `c:dLbls` | 위치 `u16s3[8]` ≠ 0 이 켬. 막대는 켰을 때만, 꺾은선·영역·방사형은 늘 (끄면 표시 0). `labelComponents` 비트 1 값, 2 백분율, 8 항목 (4 계열 이름, 추정). `delete 0`, 범례 표지·거품 크기·지시선 0, `dLblPos` 없음. 원형은 **한셀이 읽지 않는다** (`c2_e8_dl_pie_pct`: 위치 2 = outEnd, 백분율이 저장되어 있다) | 관측 |
| `c:explosion` | 원형·도넛 0 | 관측 |
| `c:smooth` | 꺾은선 계열 0 — 부드러운 선(`u16s3[5]` = 2)은 **한셀이 읽지 않는다** (`c2_e6_scatter_smooth`) | 관측 |

### 9.6 축

원형·도넛은 축이 없다. 나머지: X(`axisX`) → `catAx` 번호 0 (`axPos b`, 교차 1) — 분산형·거품형의 X 값 축도 항목 축으로 읽힌다,
Y(`axesY[0]`) → `valAx` 번호 1 (`l`, 교차 0), 3차원이면 Z(`axisZ`) → `serAx` 번호 2 (`b`, 교차 1). 보조 축(`axesY2`)과 표면 축은
쓰지 않는다.

| OOXML | 값 | 근거 |
|---|---|---|
| `c:delete` | `VtAxis.hidden` | 관측 (`c2_e7_cat_del`, `c2_e7_val_del`) |
| `c:spPr` | 숨긴 축에만 9525 선 없음 (보이는 축의 `axisPen` 808080 은 쓰지 않는다) | 관측 |
| `c:majorGridlines` / `c:minorGridlines` | `majorGridPen` / `minorGridPen` 의 style ≠ 0 이면 빈 요소 (색은 쓰지 않는다) | 관측 |
| `c:scaling` | `orientation minMax` 만 — 최소·최대·단위·거꾸로·로그 눈금은 저장되지 않는다 (6절) | 관측 |
| `c:majorTickMark` / `c:minorTickMark` | 늘 `out` / `none` — 저장된 `tickStyle` (0 none, 1 cross, 2 in, 3 out — 원본과 같다) 은 **한셀이 읽지 않는다** | 관측 (`c2_e7_tick_none`, 서식 파일 cross) |
| `c:tickLblPos` | `VtAxis.u16_p` 0 → `low`, 그 밖 `nextTo` (서식 파일 원본의 `none` 도 `low`) | 관측 |
| `c:numFmt` | 이름표(`labels[0]` 의 `VtValueBlock.textFormat`)가 없으면 없음. `VtFormatGroup.u16_a` 0 = 셀에 연결: 첫 계열 값 범위 첫 칸의 형식 (9.3절의 꼴) + `sourceLinked 1`, 계열이 없으면 `General` + 0. 1 = 자기 형식: 쓰는 구역(`VtFormatter.u16_a` ≠ 10 이고 부호가 있는 것)을 `;` 로 이은 것 + 0 | 관측 |
| 형식 특이점 | 100% 차트의 값 축과 항목 축, 누적 막대(종류 0·1·11 이고 겹치기 −1)의 항목 축은 저장된 형식과 상관없이 `0%` + 0 | 관측 (`chart_stacked`, `c2_e3_*`) |
| `c:title` | `VtAxis.title` (VtTextBlock) 의 `u16_b` 1 이고 글이 있으면: 글꼴 (9.7절), 방향 `u16_e` 0 가로 (`rot="0"`), 2 `vert270` (1 wordArtVert, 3 vert 추정), 빈 `c:layout`, `overlay 0` | 관측 (`c2_e7_ax_titles`) |
| `c:txPr` | 이름표 글 상자의 글꼴 (표본 모두 10pt `돋움` 000000) | 관측 |
| 상수 | `crosses autoZero`; 항목 축 `auto 1`, `lblAlgn ctr`, `lblOffset 100`, `tickLblSkip 1`, `tickMarkSkip 1`, `noMultiLvlLbl 0`; 값 축 `crossBetween between`; 계열 축 `tickMarkSkip 1` | 관측 |

서식 코드 부호 (`VtFormatter.code`, 글자마다 부호 하나, `n` 이 끝): `A` General, `B<d>` 0 (뒤 글자는 정수·소수 자리), `E` 점, `J` %,
`O` m, `W` yy, `Z` g, `l` /, `g<c>` 글자 c. 표본의 부호는 이것이 전부다 (`Zlg표g준n` → `g/표준` — 값 축의 소문자 g 는 연호 부호에서
온다, `B0Jn` → `0%`, `B0EB1B1Jn` → `0.00%`, `OlWn` → `m/yy`, `An` → `General`). 모르는 부호는 U+FFFD 로 둔다.

### 9.7 제목·범례·글꼴

| OOXML | 값 | 근거 |
|---|---|---|
| 글꼴 (`a:defRPr`·`a:rPr`) | `VtFont`: 크기 pt × 100, 스타일 비트 0 굵게, 비트 1 기울임 (표본은 0 과 3), 색, 이름 (빈 이름은 `함초롬돋움`). 밑줄은 저장되지 않는다 | 관측 |
| `c:title` | `VtChartTitle` 구역이 보이면 (`visible` 1): 글 상자의 글을 줄마다 문단 하나로, 문단마다 같은 글꼴의 `defRPr`·조각; `c:layout/c:manualLayout` 에 `xMode edge`·`yMode edge` 만 (위치는 저장되지 않는다), `overlay 0`, `spPr noFill`. 한셀의 레거시 저장은 제목 글꼴을 늘 12pt 굵게·기울임으로 쓴다 (6절) | 관측 |
| `c:autoTitleDeleted` | 늘 0 | 관측 |
| `c:legend` | `VtChartLegend` 구역이 보이면: 위치 `location` 1 `t`, 3 `l`, 4 `r` (오른쪽 위도), 6 `b` (0·2 `tr`, 5·7 `b` 추정), 빈 `c:layout`, `overlay 0` (겹침은 저장되지 않는다), `spPr` = 9525 검은 선 (`prstClr black`, 채우기 없음 — 저장된 배경과 상관없이), `txPr` = 범례 글꼴 | 관측 (`c2_e1_*`, `c2_e11_legend_fill`) |

### 9.8 3차원·영역

| OOXML | 값 | 근거 |
|---|---|---|
| `c:view3D` | 3차원 차트: `rAngAx 1, rotX 15, rotY 20, perspective 30, depthPercent 100`, 3차원 원형 `rAngAx 0, rotX 30, rotY 0` — 저장된 `rotation`·`elevation` 은 **한셀이 읽지 않는다** (`c2_e4_bar3d_a/_b`). 바닥·벽은 쓰지 않는다 | 관측 |
| 차트 공간 `c:spPr` | 채우기 = 뿌리 `backdrop` 의 VtFill (style 1 + 칠 style 1 이면 단색, 아니면 채우기 없음 — 추정), 선 = 테두리 (`u16_c` 1 이면 `frame` 색 + 두께 twip × 635 EMU, 아니면 9525 선 없음). 표본은 모두 ffffff + 12700 8c8c8c (채우기 없음 원본 `c2_e11_cs_nofill` 도) | 관측 |
| 그림 영역 `c:spPr` | `noFill` + 그림 영역 구역 배경의 테두리 (위와 같은 규칙, 표본은 모두 9525 선 없음). 저장된 흰 채우기는 쓰지 않는다 (`c2_e11_plot_fill` 원본 ffff00) | 관측 |
| 그 밖 | 그림 영역 빈 `c:layout`, `plotVisOnly 1`, `dispBlanksAs gap`, `date1904 0`, `roundedCorners 0`, 스타일 102/2, `hncChartStyle -1 0 0` | 관측 |

### 9.9 그리기 앵커 (쓰기)

모델의 차트는 한셀이 레거시 차트를 읽어 쓰는 꼴로 쓴다 (`src/writer.rs`, `*.legacy.as.cell` 의 그리기 파트와 같다):
`xdr:twoCellAnchor` (편집 방식 없음 — 레거시에 저장되지 않는다), `xdr:graphicFrame fPublished="0"`, `cNvPr id` 는 그리기마다 2 부터
(다른 도형 번호와 겹치지 않게), `name=""` (차트 개체 이름은 저장되지 않는다), `xdr:xfrm` = 시트 안 절대 위치 (앞 열·행 픽셀 합 +
칸 안 오프셋, EMU — 한셀은 내부 좌표에서 계산해 1 EMU 안팎 다를 수 있다), `a:graphic` → `c:chart r:id`,
`xdr:clientData fLocksWithSheet="1" fPrintsWithSheet="1"`. 차트 파트는 `xl/charts/chartN.xml` (N 은 1 부터, 원본 그대로 옮기는 파트와
겹치지 않게), 콘텐츠 형식 `application/vnd.openxmlformats-officedocument.drawingml.chart+xml`.

### 9.10 검증

| 비교 | 결과 |
|---|---|
| `chart_xml(옮긴 모델)` 대 `X.legacy.as.cell` 의 차트 파트 (`craft-charts` 15, `craft-charts2` 54, `templates` 8) | 77 / 77 바이트까지 같다 (`tests/chart_xml.rs` 의 `legacy_records_map_to_hancell_chart_parts`) |
| rcell 의 레거시 읽기 대 한셀의 레거시 읽기, 차트 내용까지 (`rcell diff`, 레거시 무시 옵션, 차트 무시 없음) | 차트 문서 96개 (위 세 폴더의 모든 레거시 파일) 모두 차이 0 (`legacy_reading_matches_hancell`, 말뭉치 117쌍) |
| 한셀 판정: 한셀이 rcell 의 XML 을 다시 저장한 것 대 `X.legacy.as.cell` | 96개 모두 `rcell diff` 차이 0 (`writer_output_is_read_identically_by_hancell`, `RCELL_OUT`). 차트 파트로는 77개 중 69개가 바이트까지 같고, 8개(꺾은선 계열 10개)는 한셀이 XML 을 읽을 때 숨은 데이터 레이블에 넣는 `<c:dLblPos val="r"/>` 하나만 다르다 — 한셀이 자기 레거시 읽기 결과를 다시 저장할 때와 같은 변화다 ([`cell-2024.md`](cell-2024.md) 11.2절) |

차트 비교(`src/compare.rs`, `chart_key::aspects`)는 묶음·계열·축·제목·범례·3차원·영역·기타를 항목마다 견주고, 표현만 다른 것(없는
요소 = 기본값, 16진 대소문자, 캐시 숫자 표기, XML 원본의 축 번호, 위치 없는 `manualLayout`, 보이지 않는 데이터 레이블의 위치·형식)은
같게 본다.

## 10. 쓰기 — 한셀의 레거시 저장

`src/legacy/vtbuild.rs` (나무 짓기), `src/legacy/vtwrite.rs` (직렬화), `src/legacy/writer.rs::chart_record` (레코드). 목표는 한셀이
같은 XML 차트를 열어 레거시로 저장한 레코드(**hx**)다. 한셀은 차트마다 클래스·포인터 배치가 같은 나무(뿌리 VtChart, 객체 400개
남짓)를 쓰고 모델에 따라 값만 바꾼다 — 아래 규칙 밖의 값은 표본 77개에서 늘 같은 값이다. 근거 = 표본끼리·hx 와의 대조 (관측),
읽기의 반대로만 정한 것은 (추정).

### 10.1 레코드

- 차트 도형마다 0x7EF0 + 0x7EF8 (`ff ff ff ff`) 을 시트의 그리기(MSODRAWING) 바로 앞에 쓴다.
- **차트 번호**: 문서에 그리기(MSODRAWINGGROUP)가 없으면 0. 있으면 그 시트의 OBJ 수(자동 필터 단추, 도형과 자식, 메모) +
  시트 안 차트 차례(1부터). 차트 번호 대조 실험 (`c2_e12_two_charts` 를 고친 XML 을 한셀이 레거시로): 차트 둘만 → 0, 0; 같은
  시트에 사각형 하나 → 2, 3; 사각형 둘 → 3, 4; 다른 시트에만 사각형 넷 (그 시트가 앞이든 뒤든) → 1, 2, 3. 서식 파일
  `CM_and_BreakEven_Analysis` 는 사각형 셋이 있는 시트의 차트 다섯이 4–8, `Marketing_Budget` 은 1–3. 같은 문서를 새 한셀로
  다시 저장해도 같은 번호다. 도형 자식·메모·단추가 OBJ 수에 드는 것은 OBJ 번호 매기기(`cell-legacy.md` 10.8절)에서 본 추정.
- **앵커**: 그리기와 같은 ClientAnchor 단위 (1절), 플래그 0x0606.
- **수식**: 원본 범위는 시트 번호, 항목·계열(값, 이름)은 XTI 번호 — XTI 는 항목, 계열마다 값·이름 차례로 생긴다 (2절).
  토큰은 참조 클래스 (0x3B, 한 칸이면 0x3A) — 다만 **참조가 하나뿐인 원본 범위는 값 클래스 0x5B** (`c2_e10_str_cat`: 한셀
  레코드 77개의 원본 범위 중 참조 하나짜리는 이것뿐, 합집합의 피연산자는 모두 0x3B·0x3A; 한 칸이면 0x5A 로 본다 — 추정).
  글자로 쓴 계열 이름은 ptgStr (UTF-16 플래그 1).

### 10.2 원본 범위

한셀이 XML 차트를 열 때 다시 만드는 범위다. 글자 이름이 있는 차트, 거품형, 여러 영역을 참조하는 항목·값은 cce 0. 그 밖은
사각형 목록 (계열 이름들, 항목, 계열 값들, 머리 칸)에서 **합쳐 사각형이 되는 첫 쌍**(뒤 원소 차례로 찾는다)을 빼고 합친 것을 끝에
붙이기를 더 없을 때까지 한다. 머리 칸은 이름이 있을 때만: 항목이 세로면 (첫 이름 행, 항목 열), 가로면 (항목 행, 첫 이름 열).
합친 차례대로 Area3d(한 칸이면 Ref3d) + 둘째부터 Union. 관측: `chart_base` `(C1:C6,A1:A6)`, 계열 둘 `(A1:A6,C1:D6)`, 행 방향 계열
`(A1:A3,C1:E3)`, 주식형 `(F1:F6,A1:A6,C1:D6)`, 서식 파일 `(B33,E6:P6,E33:P33,B6)`·`(F6:Q6,F66:Q66)`, 이웃한 항목 `B1:C6`.

### 10.3 데이터 표와 계열 목록

| 값 | 규칙 | 근거 |
|---|---|---|
| `VtDataGrid` | 이름표 열 = 항목이 있으면 1, 이름표 행 = 이름 있는 계열이 하나라도 있으면 1 (서식 파일은 이름 행이 없다), 행 = 항목·값의 긴 쪽, 열 = 계열 수. 칸은 열 우선: 항목 이름표 `VtString`, 계열 이름 `VtString` (없는 이름은 `계열N`), 값 `VtDouble` (빈 값 0, 플래그 0xFFFF). `u16_a` 원형 0 | 관측 |
| 거품형 | 계열마다 (값, 거품 크기) 두 열, 거품 크기 열의 이름 칸은 비고 꼬리의 그 계열은 값·이름 cce 0 | `c2_e6_bubble` |
| 계열 목록 | 실제 계열 뒤에 기본 자리 계열 `행N` — 모두 다섯이 되게 (더 많으면 그대로). 항목·값이 하나도 없으면 `열N` 으로 넷. 원형은 항목마다 하나 (이름 = 항목) | 관측, 서식 파일 `Marketing_Budget` |
| 같이 쓰는 객체 | 기본 글꼴 이름 `함초롬돋움` (첫 VtFont 가 만들고 나머지 기본 글꼴이 가리킨다), 계열 이름 (계열 이름표·글 상자가 가리킨다). 축 이름표 글꼴 `돋움` 은 축마다 새 문자열 | 관측 |

### 10.4 모델에 따라 바뀌는 값

| 값 | 규칙 | 근거 |
|---|---|---|
| `chartType` | 9.1절의 반대: 막대 1 (3차원 0), 가로 막대 11 (3차원 10), 꺾은선·분산형·거품형 3 (3차원 꺾은선 2), 영역 5 (3차원 4), 원형 14 (3차원 13), 도넛 15, 방사형 18, 주식형 1 | 관측 |
| `mask` | 가로 막대 0xFFCF, 2차원 영역·누적·100% 0xFF3F, 그 밖 0xFFFF | 6절 |
| `VtChart.f32_a` | 3차원(원형 포함)이고 rotY 0 이면 90, 도넛 90, 그 밖 0 | `c2_e4_*`, `chart_col3d` |
| `gap` / `overlap` / `gapDepth` | 막대 gapWidth/100 (없으면 1.5), 주식형 0.5, 도넛 0, 그 밖 1 / 막대 −overlap/100 / 3차원 축 gapDepth/100 (1.5), 3차원 원형·도넛 0, 그 밖 1 | 관측 (`c2_e6_stock`) |
| `rotation`, `elevation` | 3차원: rotY 0 이면 (0, rotX), 아니면 (90 − rotY, rotX). 도넛 (0, 90). 2차원 (30, 25) | `c2_e4_bar3d_*` |
| 그림 영역 | 광원: 3차원·도넛 하나 (0.80, 0.27, 0.53; 주변광 0.6), 그 밖 둘. `f32_g` 3차원 원형 70, `f32_i` 도넛 0.01, `u16s[2]` 3차원 0, `u16s[3]` 도넛 0. 3차원 축이면 벽 선 없음·바닥 칠 | 관측 |
| 축 고르기 | X = 항목·날짜 축 (분산형·거품형은 아래쪽 값 축, `xy` 1 + XYScale (1, 1, 1, 1, 1)), Y = 다른 값 축, Z = 계열 축. Y2 는 늘 숨김 | 관측 |
| 축 선·눈금선 | 모델에 축이 있고 숨기지 않았으면 축 선 808080, 아니면 8C8C8C. 주 눈금선이 있으면 (1, 808080), 값 축이 없는 원형은 (1, 8C8C8C). 3차원 축 차트의 Y2 눈금선 = Y 의 것 | 관측 |
| `tickStyle` | 주 눈금 none 0, cross 1, in 2, out 3. 모델에 없는 X 0, Y 1; 3차원 Z 3 (숨김, 이름표 있음) | `c2_e7_tick_none`, 서식 파일 |
| `u16_p` | X 축 이름표 위치 `none` 이면 0 (값 축은 `none` 이어도 1). `low` 도 0 (추정: 9.6절 읽기의 반대) | 서식 파일 `CM_and_BreakEven_Analysis` |
| `u16_q` / `u16_r` | 영역형의 X 축 1 / 100% 차트의 값 축 2 | 관측 |
| 방사형 축 | X `f32_c` 90, Y `f32_b` 90, X·Y·Z `u16_s` 0 (그 밖 4) | `c2_e6_radar` |
| 이름표 형식 | 9.6절 부호의 반대: `General` → `An` (VtFormatter.u16_a 0), `G/표준`·`g/표준` → `Zlg표g준n` (3), `m/yy` → `OlWn` (3), `0%` → `B0Jn` (6, `u16s[2]` 65534), `0.00%` → `B0EB1B1Jn` (6, 둘째 자릿수 묶음 (2, 1)). 그 밖은 셀에 연결 (VtFormatGroup.u16_a 0 + 빈 구역 넷). 값 축은 형식이 없으면 `G/표준`, 분산형 X 는 `General`. 100% 차트와 겹치기 −1 인 누적 막대의 항목 축 `0%` 는 쓰지 않는다 (추정: 읽기의 특이점 반대) | 관측 (서식 파일) |
| 제목 | 글이 있으면 구역 보임, 글 상자 `u16_b` 1, 글꼴은 이름 빈 새 문자열 12pt 스타일 3 (없으면 기본 글꼴 12pt, 글 `차트 제목`). 축 제목도 같은 꼴 10pt, 방향 `u16_e` (vert270 2, wordArtVert 1, vert 3) | `chart_title`, `c2_e7_ax_titles` |
| 범례 | 보임 = 모델에 범례, 위치 t 1, l 3, b 6, 그 밖 4 | `c2_e1_*` |
| 계열 색 | 칠 = 계열 단색 채우기 (없으면 팔레트), 선·표식 선·색 사본 = 팔레트 (`chart_color`: 채우기 FF0000 이어도 선 73B1D1). 꺾은선 갈래는 계열 선(색, 두께 EMU/635 twip)이 `linePen`, 막대·영역은 `edgePen` (실선 20 twip) | `chart_color`, `c2_e5_ser_pen` |
| 누적 | 누적·100% 의 실제 계열: `u16s2[6]` 1, `u16s2[7]` = 계열 차례 (그 밖 `u16s2[4..6]` = 계열 차례) | `c2_e3_*`, 서식 파일 |
| 표식 | 2차원 꺾은선: 모델의 표식이면 `u16_d` 0, 모양 (plus 1, x 2, star 3, circle·auto 4, square 5, diamond 6, triangle 7; 담을 자리가 없는 dash·dot·picture 도 6 — 한셀은 다시 읽을 때 diamond), 크기 pt × 20, 표식·pen5 색 (000000, 플래그 0x20). 한셀의 XML→레거시 저장은 표식 채우기를 버린다 (pen5 는 늘 검정) — 레거시에서 읽은 문서만 표식의 단색 채우기를 pen5 색으로 쓴다 (플래그 0; 읽기가 그 색을 표식 채우기로 읽는다). `none` 은 크기와 상관없이 자동 (`u16_d` 1, 크기 200, 모양 2 — 표식 모양 실험의 `none` 크기 9; 다시 읽으면 `none` 크기 7) | 표식 모양 실험 (c2_e5_line_marker 의 모양 11가지), 표식 채우기 실험 (채우기·선·`noFill`·검정 m1–m6) |
| `showLine` / 부드러운 선 | 2차원 꺾은선·분산형·방사형의 실제 계열 1 / 분산형 `u16s3[5]` 2 | 관측 |
| 데이터 레이블 | 구성 비트 (값 1, 백분율 2, 계열 이름 4, 항목 8), `u16s3[8]` 위치 (outEnd 2, 그 밖 1), 레이블 글 상자 `u16_b` 1 + 배경, 새 글꼴 `돋움` 10pt. 원형은 첫 계열의 레이블이 항목마다 | `c2_e8_*` |
| 거품형 | 값 계열: 선·칠·표식 선·색 사본 8C8C8C (선 style 0), `u16s1[0]` 0, `u16_d` 0, pen5 (0, 8C8C8C), 표식 크기 600, 표식 색 9999FF, 모양 9, `typeTable[7, 8, 19]` = 5, 6, 6. 거품 크기 계열: 같은 표식 값에 크기 0, `u16s2[3]` 1 | `c2_e6_bubble` |

### 10.5 직렬화

`vtwrite` 는 3절의 반대다: 배치 표(`CLASSES`)의 차례대로 필드를 쓰고, 나무에 없는 필드는 0 (포인터는 없음, 묶음·배열은 그
꼴의 0, 기반 부분은 빈 객체). 새 객체는 다음 번호, 같이 쓰는 객체는 두 번째부터 앞 객체 참조, 클래스는 처음 쓸 때 이름·판을
붙인다. VtString 은 cp949 + UTF-16 두 부호화와 pad 0.

### 10.6 검증

| 비교 | 결과 |
|---|---|
| 읽은 아카이브를 그대로 다시 쓰기 (`vtwrite` 시험 `corpus_archives_round_trip_through_writer`) | 77 / 77 바이트까지 같다 |
| XML 차트 → 나무 대 hx 의 나무 (VtString pad 빼고, `vtdump` 줄 비교) | 문서 70 / 70, 차트 77개 (만든 차트 15, 대조 실험 54 — 거품형·방사형·주식형 포함, 서식 파일 8) 가 머리(차트 번호·앵커)·수식·나무까지 같다 |
| 레코드 (`rec-diff --ignore-volatile`, hx 기준) | 차트 문서의 0x7EF0 이 모두 같다 |
| 레거시 입력 → 레거시 (원본 레코드 기준) | 문서 70 개 중 51 개 같다. 나머지는 한셀의 레거시 읽기가 버리는 것 (9절: 3차원 회전, 눈금 모양, 3차원 누적, 분산형·거품형 → 꺾은선, 글자 이름, 원형 레이블, 선 두께의 기본 두께 맞춤 40 → 45 twip, 100% 값 축 형식) — 다시 읽으면 같은 차트다 |
| 왕복 (`legacy_writer_round_trip`) | 레거시 보기는 같은 레코드를 지어 읽기로 다시 읽은 차트 — 234 / 234 |
| 표식 실험 (표식 채우기 m1–m6, 모양 11가지 — c2_e5_line_marker 를 고친 XML) | 0x7EF0 이 한셀 레거시와 17 / 17 같고, 한셀이 rcell 레거시를 읽은 것이 한셀 레거시를 읽은 것과 17 / 17 차이 0 |
| 한셀 판정, 레거시 입력 (`legacy_writer_output_is_read_identically_by_hancell`) | 말뭉치 117 / 117 차이 0 — 거품형 `c2_e6_bubble` 의 표식 채우기(pen5 색)도 |

