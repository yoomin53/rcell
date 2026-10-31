// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀이 .ods 에 쓰는 도형 종류별 `draw:enhanced-geometry` (한셀 저장 관찰). (OOXML 이름, ODF 종류, 요소 글)

pub(crate) const GEOMS: &[(&str, &str, &str)] = &[
    (
        "ellipse",
        "ellipse",
        r#"<draw:enhanced-geometry draw:type="ellipse" svg:viewBox="0 0 21600 21600" draw:enhanced-path="U 10800 10800 10800 10800 0 360 Z N" draw:text-areas="3163 3163 18437 18437" draw:glue-points="10800 0 3163 3163 0 10800 3163 18437 10800 21600 18437 18437 21600 10800 18437 3163"/>"#,
    ),
    (
        "roundRect",
        "round-rectangle",
        r#"<draw:enhanced-geometry draw:type="round-rectangle" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M ?f7 0 X 0 ?f8 L 0 ?f9 Y ?f7 21600 L ?f10 21600 X 21600 ?f9 L 21600 ?f8 Y ?f10 0 Z N" draw:text-areas="?f3 ?f4 ?f5 ?f6" draw:modifiers="3600"><draw:equation draw:name="f0" draw:formula="45"/><draw:equation draw:name="f1" draw:formula="$0 *sin(?f0 *(pi/180))"/><draw:equation draw:name="f2" draw:formula="?f1 *3163/7636"/><draw:equation draw:name="f3" draw:formula="left+?f2 "/><draw:equation draw:name="f4" draw:formula="top+?f2 "/><draw:equation draw:name="f5" draw:formula="right-?f2 "/><draw:equation draw:name="f6" draw:formula="bottom-?f2"/><draw:equation draw:name="f7" draw:formula="left+$0"/><draw:equation draw:name="f8" draw:formula="top+$0"/><draw:equation draw:name="f9" draw:formula="bottom-$0"/><draw:equation draw:name="f10" draw:formula="right-$0"/><draw:handle draw:handle-switched="true" draw:handle-position="$0 top" draw:handle-range-x-minimum="0" draw:handle-range-x-maximum="10800"/></draw:enhanced-geometry>"#,
    ),
    (
        "triangle",
        "isosceles-triangle",
        r#"<draw:enhanced-geometry draw:type="isosceles-triangle" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M ?f0 0 L 21600 21600 0 21600 Z N" draw:text-areas="?f1 10800 ?f2 18000 ?f3 7200 ?f4 21600" draw:glue-points="?f0 0 ?f1 10800 0 21600 10800 21600 21600 21600 ?f7 10800" draw:modifiers="10800"><draw:equation draw:name="f0" draw:formula="$0 "/><draw:equation draw:name="f1" draw:formula="$0 /2 "/><draw:equation draw:name="f2" draw:formula="?f1 +10800 "/><draw:equation draw:name="f3" draw:formula="$0 *2/3"/><draw:equation draw:name="f4" draw:formula="?f3 +7200"/><draw:equation draw:name="f5" draw:formula="21600-?f0 "/><draw:equation draw:name="f6" draw:formula="?f5 /2"/><draw:equation draw:name="f7" draw:formula="21600-?f6 "/><draw:handle draw:handle-position="$0 top" draw:handle-range-x-minimum="0" draw:handle-range-x-maximum="21600"/></draw:enhanced-geometry>"#,
    ),
    (
        "diamond",
        "diamond",
        r#"<draw:enhanced-geometry draw:type="diamond" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M 10800 0 L 21600 10800 10800 21600 0 10800 10800 0 Z N" draw:text-areas="5400 5400 16200 16200" draw:glue-points="10800 0 0 10800 10800 21600 21600 10800"/>"#,
    ),
    (
        "rightArrow",
        "right-arrow",
        r#"<draw:enhanced-geometry draw:type="right-arrow" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M 0 ?f0 L ?f1 ?f0 ?f1 0 21600 10800 ?f1 21600 ?f1 ?f2 0 ?f2 Z N" draw:text-areas="0 ?f0 ?f5 ?f2" draw:modifiers="16200 5400"><draw:equation draw:name="f0" draw:formula="$1 "/><draw:equation draw:name="f1" draw:formula="$0 "/><draw:equation draw:name="f2" draw:formula="21600-$1 "/><draw:equation draw:name="f3" draw:formula="21600-?f1 "/><draw:equation draw:name="f4" draw:formula="?f3 *?f0 /10800"/><draw:equation draw:name="f5" draw:formula="?f1 +?f4 "/><draw:equation draw:name="f6" draw:formula="?f1 *?f0 /10800"/><draw:equation draw:name="f7" draw:formula="f1 -?f6 "/><draw:handle draw:handle-position="$0 $1" draw:handle-range-x-minimum="0" draw:handle-range-x-maximum="21600" draw:handle-range-y-minimum="0" draw:handle-range-y-maximum="10800"/></draw:enhanced-geometry>"#,
    ),
    (
        "star5",
        "star5",
        r#"<draw:enhanced-geometry draw:type="star5" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M 10797 0 L 8278 8256 0 8256 6722 13405 4198 21600 10797 16580 17401 21600 14878 13405 21600 8256 13321 8256 10797 0 Z N" draw:text-areas="6722 8256 14878 15460"/>"#,
    ),
    (
        "hexagon",
        "hexagon",
        r#"<draw:enhanced-geometry draw:type="hexagon" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M ?f0 0 L ?f1 0 21600 10800 ?f1 21600 ?f0 21600 0 10800 Z N" draw:text-areas="?f3 ?f3 ?f4 ?f4" draw:glue-points="10800 0 0 10800 10800 21600 21600 10800" draw:modifiers="5400"><draw:equation draw:name="f0" draw:formula="$0 "/><draw:equation draw:name="f1" draw:formula="21600-$0 "/><draw:equation draw:name="f2" draw:formula="$0 *100/234"/><draw:equation draw:name="f3" draw:formula="?f2 +1700"/><draw:equation draw:name="f4" draw:formula="21600-?f3 "/><draw:handle draw:handle-position="$0 top" draw:handle-range-x-minimum="0" draw:handle-range-x-maximum="10800"/></draw:enhanced-geometry>"#,
    ),
    (
        "parallelogram",
        "parallelogram",
        r#"<draw:enhanced-geometry draw:type="parallelogram" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M ?f0 0 L 21600 0 ?f1 21600 0 21600 Z N" draw:text-areas="?f3 ?f3 ?f4 ?f4" draw:glue-points="?f6 0 10800 ?f8 ?f11 10800 ?f9 21600 10800 ?f10 ?f5 10800" draw:modifiers="5400"><draw:equation draw:name="f0" draw:formula="$0 "/><draw:equation draw:name="f1" draw:formula="21600-$0 "/><draw:equation draw:name="f2" draw:formula="$0 *10/24"/><draw:equation draw:name="f3" draw:formula="?f2 +1750"/><draw:equation draw:name="f4" draw:formula="21600-?f3 "/><draw:equation draw:name="f5" draw:formula="?f0 /2"/><draw:equation draw:name="f6" draw:formula="10800+?f5 "/><draw:equation draw:name="f7" draw:formula="?f0 -10800"/><draw:equation draw:name="f8" draw:formula="if(?f7 ,?f13 ,0)"/><draw:equation draw:name="f9" draw:formula="10800-?f5 "/><draw:equation draw:name="f10" draw:formula="if(?f7 ,?f12 ,21600)"/><draw:equation draw:name="f11" draw:formula="21600-?f5 "/><draw:equation draw:name="f12" draw:formula="21600*10800/?f0 "/><draw:equation draw:name="f13" draw:formula="21600-?f12 "/><draw:handle draw:handle-position="$0 top" draw:handle-range-x-minimum="0" draw:handle-range-x-maximum="10800"/></draw:enhanced-geometry>"#,
    ),
    (
        "trapezoid",
        "trapezoid",
        r#"<draw:enhanced-geometry draw:type="trapezoid" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M ?f0 ?f3 L ?f14 ?f2 ?f15 ?f2 ?f1 ?f3 Z N" draw:path-stretchpoint-x="21600" draw:path-stretchpoint-y="21600" draw:text-areas="?f17 ?f18 ?f19 ?f3" draw:glue-points="?f13 ?f7 ?f16 ?f7" draw:glue-point-leaving-directions="-270, -90" draw:modifiers="25000"><draw:equation draw:name="f0" draw:formula="left"/><draw:equation draw:name="f1" draw:formula="right"/><draw:equation draw:name="f2" draw:formula="top"/><draw:equation draw:name="f3" draw:formula="bottom"/><draw:equation draw:name="f4" draw:formula="?f3 - ?f2"/><draw:equation draw:name="f5" draw:formula="?f4 / 2"/><draw:equation draw:name="f6" draw:formula="?f4 / 3"/><draw:equation draw:name="f7" draw:formula="?f2 + ?f5"/><draw:equation draw:name="f8" draw:formula="?f1 - ?f0"/><draw:equation draw:name="f9" draw:formula="?f8 / 4"/><draw:equation draw:name="f10" draw:formula="min(?f8, ?f4)"/><draw:equation draw:name="f11" draw:formula="50000 * ?f8 / ?f10"/><draw:equation draw:name="f12" draw:formula="$0"/><draw:equation draw:name="f13" draw:formula="?f10 * ?f12 / 200000"/><draw:equation draw:name="f14" draw:formula="?f10 * ?f12 / 100000"/><draw:equation draw:name="f15" draw:formula="?f1 - ?f14"/><draw:equation draw:name="f16" draw:formula="?f1 - ?f13"/><draw:equation draw:name="f17" draw:formula="?f9 * ?f12 / ?f11"/><draw:equation draw:name="f18" draw:formula="?f6 * ?f12 / ?f11"/><draw:equation draw:name="f19" draw:formula="?f1 - ?f17"/></draw:enhanced-geometry>"#,
    ),
    (
        "flowChartProcess",
        "flowchart-process",
        r#"<draw:enhanced-geometry draw:type="flowchart-process" svg:viewBox="0 0 21600 21600" draw:enhanced-path="M 0 0 L 21600 0 21600 21600 0 21600 0 0 Z N" draw:glue-points="10800 0 0 10800 10800 21600 21600 10800"/>"#,
    ),
];
