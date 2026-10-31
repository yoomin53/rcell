// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 미리 정의된 모양의 수식·경로 정의 (ECMA-376 의 가이드 수식 꼴 — 표준이 모양을 만드는 방식을 가설로 쓰고, 한셀 화면을
//! 여러 가로세로 비율로 따라 그린 윤곽에 맞춰 확인한 것만 둔다).
//!
//! 글 꼴 (한 줄에 하나):
//! - `av 이름 기본값` — 조정값 (문서의 조정값이 있으면 그 값, 1/100000)
//! - `gd 이름 연산 인수…` — 가이드 수식 (`*/ +- +/ ?: abs at2 cat2 sat2 cos sin tan max min mod pin sqrt val`)
//! - `path [채움] [nostroke]` — 경로 시작 (채움 = norm darken darkenLess lighten lightenLess none)
//! - `M x y`, `L x y`, `A wR hR 시작각 휘는각`, `C x1 y1 x2 y2 x y`, `Z`
//!
//! 같은 꼴을 뒤집거나 가로세로를 바꿔 쓰는 모양은 변환 행렬 `[m00 m01 m10 m11]` (점 = M·p, 바탕 모양은 가로세로를 바꾼
//! 크기로 그린다)을 함께 돌려준다.

/// 항등 변환
const ID: [f64; 4] = [1.0, 0.0, 0.0, 1.0];

/// 모양의 정의: 글, 변환 행렬, 경로를 거꾸로 그리는지 (겹치는 곳에서 어느 쪽이 위에 오는지)
pub struct Def {
    pub text: &'static str,
    pub xf: [f64; 4],
    pub reverse: bool,
}

const fn plain(text: &'static str) -> Def {
    Def {
        text,
        xf: ID,
        reverse: false,
    }
}

/// 모양 이름 → 정의
pub fn def(prst: &str) -> Option<Def> {
    Some(match prst {
        "bentArrow" => plain(BENT_ARROW),
        "uturnArrow" => plain(UTURN_ARROW),
        "curvedRightArrow" => plain(CURVED_RIGHT_ARROW),
        // 왼쪽으로 휘는 화살표 = 좌우를 뒤집은 것 (어두운 띠가 앞쪽 띠 위에 겹친다)
        "curvedLeftArrow" => Def {
            text: CURVED_RIGHT_ARROW,
            xf: [-1.0, 0.0, 0.0, 1.0],
            reverse: true,
        },
        // 위로 휘는 화살표 = 오른쪽으로 휘는 화살표를 반시계로 90도 돌린 것 (어두운 띠가 위), 아래로 휘는 것은 그 위아래를 뒤집은 것
        "curvedUpArrow" => Def {
            text: CURVED_RIGHT_ARROW,
            xf: [0.0, 1.0, -1.0, 0.0],
            reverse: true,
        },
        "curvedDownArrow" => Def {
            text: CURVED_RIGHT_ARROW,
            xf: [0.0, 1.0, 1.0, 0.0],
            reverse: false,
        },
        "quadArrow" => plain(QUAD_ARROW),
        "leftRightUpArrow" => plain(LEFT_RIGHT_UP_ARROW),
        "leftUpArrow" => plain(LEFT_UP_ARROW),
        "leftRightRibbon" => plain(LEFT_RIGHT_RIBBON),
        "verticalScroll" => plain(VERTICAL_SCROLL),
        // 가로로 말린 두루마리 = 세로로 말린 것의 가로세로를 바꾼 것
        "horizontalScroll" => Def {
            text: VERTICAL_SCROLL,
            xf: [0.0, 1.0, 1.0, 0.0],
            reverse: false,
        },
        _ => return None,
    })
}

/// 네 방향 화살표: 화살촉 폭의 반 dx2 = ss·조정값2, 띠 두께의 반 dx3 = ss·조정값1/2, 촉 길이 x1 = ss·조정값3
const QUAD_ARROW: &str = "
av adj1 22500
av adj2 22500
av adj3 22500
gd a2 pin 0 adj2 50000
gd maxAdj1 */ a2 2 1
gd a1 pin 0 adj1 maxAdj1
gd q1 +- 100000 0 maxAdj1
gd maxAdj3 */ q1 1 2
gd a3 pin 0 adj3 maxAdj3
gd x1 */ ss a3 100000
gd dx2 */ ss a2 100000
gd x2 +- hc 0 dx2
gd x5 +- hc dx2 0
gd dx3 */ ss a1 200000
gd x3 +- hc 0 dx3
gd x4 +- hc dx3 0
gd x6 +- r 0 x1
gd y2 +- vc 0 dx2
gd y5 +- vc dx2 0
gd y3 +- vc 0 dx3
gd y4 +- vc dx3 0
gd y6 +- b 0 x1
path
M l vc
L x1 y2
L x1 y3
L x3 y3
L x3 x1
L x2 x1
L hc t
L x5 x1
L x4 x1
L x4 y3
L x6 y3
L x6 y2
L r vc
L x6 y5
L x6 y4
L x4 y4
L x4 y6
L x5 y6
L hc b
L x2 y6
L x3 y6
L x3 y4
L x1 y4
L x1 y5
Z
";

/// 왼쪽·오른쪽·위쪽 화살표 (T 꼴): 가로 띠는 아래에 붙고 (가운데선 b − dx2), 위 화살표는 가운데에서 올라간다
const LEFT_RIGHT_UP_ARROW: &str = "
av adj1 25000
av adj2 25000
av adj3 25000
gd a2 pin 0 adj2 50000
gd maxAdj1 */ a2 2 1
gd a1 pin 0 adj1 maxAdj1
gd q1 +- 100000 0 maxAdj1
gd maxAdj3 */ q1 1 2
gd a3 pin 0 adj3 maxAdj3
gd x1 */ ss a3 100000
gd dx2 */ ss a2 100000
gd x2 +- hc 0 dx2
gd x5 +- hc dx2 0
gd dx3 */ ss a1 200000
gd x3 +- hc 0 dx3
gd x4 +- hc dx3 0
gd x6 +- r 0 x1
gd yc +- b 0 dx2
gd y2 +- yc 0 dx2
gd y3 +- yc 0 dx3
gd y4 +- yc dx3 0
path
M l yc
L x1 y2
L x1 y3
L x3 y3
L x3 x1
L x2 x1
L hc t
L x5 x1
L x4 x1
L x4 y3
L x6 y3
L x6 y2
L r yc
L x6 b
L x6 y4
L x1 y4
L x1 b
Z
";

/// 왼쪽·위쪽 화살표 (┘ 꼴): 가로 띠는 아래, 세로 띠는 오른쪽
const LEFT_UP_ARROW: &str = "
av adj1 25000
av adj2 25000
av adj3 25000
gd a2 pin 0 adj2 50000
gd maxAdj1 */ a2 2 1
gd a1 pin 0 adj1 maxAdj1
gd maxAdj3 +- 100000 0 a2
gd a3 pin 0 adj3 maxAdj3
gd x1 */ ss a3 100000
gd dx2 */ ss a2 100000
gd dx3 */ ss a1 200000
gd xc +- r 0 dx2
gd yc +- b 0 dx2
gd x2 +- xc 0 dx2
gd y2 +- yc 0 dx2
gd xs1 +- xc 0 dx3
gd xs2 +- xc dx3 0
gd ys1 +- yc 0 dx3
gd ys2 +- yc dx3 0
path
M xc t
L r x1
L xs2 x1
L xs2 ys2
L x1 ys2
L x1 b
L l yc
L x1 y2
L x1 ys1
L xs1 ys1
L xs1 x1
L x2 x1
Z
";

/// 양쪽 화살표 리본: 왼쪽 화살촉에 이어지는 띠(위쪽)와 오른쪽 화살촉으로 가는 띠(아래쪽, 두 띠 가운데선이 2·dy2 만큼 어긋난다)가
/// 가운데에서 접혀 겹친다. 접힌 곳은 가운데 ± 폭/32, 접힌 모서리 반지름 hR = 조정값3 × ss / 4
const LEFT_RIGHT_RIBBON: &str = "
av adj1 50000
av adj2 50000
av adj3 16667
gd a3 pin 0 adj3 33333
gd maxAdj1 +- 100000 0 a3
gd a1 pin 0 adj1 maxAdj1
gd w1 +- wd2 0 wd32
gd maxAdj2 */ 100000 w1 ss
gd a2 pin 0 adj2 maxAdj2
gd x1 */ ss a2 100000
gd x4 +- r 0 x1
gd dy1 */ h a1 200000
gd dy2 */ h a3 200000
gd hR */ a3 ss 400000
gd x2 +- hc 0 wd32
gd x3 +- hc wd32 0
gd x5 +- x2 hR 0
gd ltipy +- vc 0 dy2
gd rtipy +- vc dy2 0
gd ltop +- ltipy 0 dy1
gd lbot +- ltipy dy1 0
gd rtop +- rtipy 0 dy1
gd rbot +- rtipy dy1 0
gd lhb +- ltipy ltipy 0
gd rhb +- rtipy rtipy 0
gd rht +- rhb 0 h
gd ya +- ltop hR 0
gd y70 +- rtop 0 hR
gd y190 +- rbot 0 hR
path
M l ltipy
L x1 t
L x1 ltop
L hc ltop
A wd32 hR 3cd4 cd4
L x3 lbot
L x1 lbot
L x1 lhb
Z
path
M x2 y70
L x2 y190
A hR hR cd2 -5400000
L x4 rbot
L x4 b
L r rtipy
L x4 rht
L x4 rtop
L x5 rtop
A hR hR cd4 cd4
Z
path darkenLess
M x3 rtop
L x3 ya
A wd32 hR 0 cd4
A wd32 hR 3cd4 -5400000
A hR hR cd2 -5400000
Z
";

/// 세로로 말린 두루마리: 위쪽 가로 막대(오른쪽 끝은 반원, 왼쪽 끝은 말린 원)와 그 아래로 내려오는 종이, 왼쪽 아래의 말린 원.
/// 말린 원의 지름 = 말림 크기 ch = ss·조정값 (가로세로 비율과 무관)
const VERTICAL_SCROLL: &str = "
av adj 12500
gd a pin 0 adj 25000
gd ch */ ss a 100000
gd ch2 */ ch 1 2
gd ch4 */ ch 1 4
gd x5 +- ch ch2 0
gd x6 +- r 0 ch
gd x7 +- r 0 ch2
gd x9 +- x5 ch2 0
gd y3 +- b 0 ch
gd y4 +- b 0 ch2
path
M ch ch2
A ch2 ch2 cd2 cd4
L x7 t
A ch2 ch2 3cd4 cd2
L x6 ch
L x6 y4
A ch2 ch2 0 cd4
L ch b
Z
path darkenLess
M x5 ch
A ch4 ch4 cd4 cd2
L x9 ch2
A ch2 ch2 0 cd4
Z
path none
M x5 t
A ch2 ch2 3cd4 cd2
L x6 ch
path darkenLess
M ch2 b
A ch2 ch2 cd4 21600000
Z
path
M ch2 y3
L ch y3
L ch y4
L ch2 y4
A ch4 ch4 cd4 -10800000
Z
";

/// 오른쪽으로 휘는 화살표: 오른쪽 위에서 시작하는 뒤쪽 띠(어두운 면)가 타원 둘레로 왼쪽을 돌아, 앞쪽 띠가 오른쪽 아래의
/// 화살촉으로 간다. 띠의 가장자리는 모두 같은 타원(가운데 (w, hR), 반지름 w × hR)을 띠 두께 th 만큼 아래로 옮긴 것이고,
/// hR 은 높이의 반에서 (두께 + 화살촉 폭)/4 를 뺀 값이다
const CURVED_RIGHT_ARROW: &str = "
av adj1 25000
av adj2 50000
av adj3 25000
gd maxAdj2 */ 50000 h ss
gd a2 pin 0 adj2 maxAdj2
gd a1 pin 0 adj1 a2
gd th */ ss a1 100000
gd aw */ ss a2 100000
gd q1 +/ th aw 4
gd hR +- hd2 0 q1
gd maxAdj3 */ 100000 w ss
gd a3 pin 0 adj3 maxAdj3
gd ah */ ss a3 100000
gd xb +- r 0 ah
gd mah +- 0 0 ah
gd q2 */ w w 1
gd q3 */ ah ah 1
gd q4 +- q2 0 q3
gd q5 sqrt q4
gd dy */ hR q5 w
gd tAng at2 mah dy
gd swA +- tAng 0 cd2
gd swB +- cd2 0 tAng
gd th2 */ th 1 2
gd yc0 +- hR th2 0
gd yc +- yc0 dy 0
gd aw2 */ aw 1 2
gd yh1 +- yc 0 aw2
gd yh2 +- yc aw2 0
gd yt +- yc0 hR 0
gd ya +- hR dy 0
gd yb +- ya th 0
path darkenLess
M l hR
A w hR cd2 cd4
L r th
A w hR 3cd4 -5400000
Z
path
M l hR
A w hR cd2 swA
L xb yh1
L r yt
L xb yh2
L xb yb
A w hR tAng swB
Z
";

/// 꺾인 화살표: 왼쪽 아래에서 올라와 둥글게 꺾여 오른쪽으로 가는 띠와 화살촉
const BENT_ARROW: &str = "
av adj1 25000
av adj2 25000
av adj3 25000
av adj4 43750
gd a2 pin 0 adj2 50000
gd maxAdj1 */ a2 2 1
gd a1 pin 0 adj1 maxAdj1
gd a3 pin 0 adj3 50000
gd th */ ss a1 100000
gd aw2 */ ss a2 100000
gd th2 */ th 1 2
gd dh2 +- aw2 0 th2
gd ah */ ss a3 100000
gd bw +- r 0 ah
gd bh +- b 0 dh2
gd bs min bw bh
gd maxAdj4 */ bs 100000 ss
gd a4 pin 0 adj4 maxAdj4
gd bd */ ss a4 100000
gd bd3 +- bd 0 th
gd bd2 max bd3 0
gd x3 +- th bd2 0
gd x4 +- r 0 ah
gd y3 +- dh2 th 0
gd y4 +- y3 dh2 0
gd y5 +- dh2 bd 0
gd y6 +- y3 bd2 0
path
M l b
L l y5
A bd bd cd2 cd4
L x4 dh2
L x4 t
L r aw2
L x4 y4
L x4 y3
L x3 y3
A bd2 bd2 3cd4 -5400000
L th b
Z
";

/// U 자 화살표: 왼쪽 아래에서 올라가 위에서 돌아 오른쪽으로 내려오는 띠와 아래 화살촉
const UTURN_ARROW: &str = "
av adj1 25000
av adj2 25000
av adj3 25000
av adj4 43750
av adj5 75000
gd a2 pin 0 adj2 25000
gd maxAdj1 */ a2 2 1
gd a1 pin 0 adj1 maxAdj1
gd q2 */ a1 ss h
gd q3 +- 100000 0 q2
gd maxAdj3 */ q3 h ss
gd a3 pin 0 adj3 maxAdj3
gd q1 +- a3 a1 0
gd minAdj5 */ q1 ss h
gd a5 pin minAdj5 adj5 100000
gd th */ ss a1 100000
gd aw2 */ ss a2 100000
gd th2 */ th 1 2
gd dh2 +- aw2 0 th2
gd y5 */ h a5 100000
gd ah */ ss a3 100000
gd y4 +- y5 0 ah
gd x9 +- r 0 dh2
gd bw */ x9 1 2
gd bs min bw y4
gd maxAdj4 */ bs 100000 ss
gd a4 pin 0 adj4 maxAdj4
gd bd */ ss a4 100000
gd bd3 +- bd 0 th
gd bd2 max bd3 0
gd x3 +- th bd2 0
gd x8 +- r 0 aw2
gd x6 +- x8 0 aw2
gd x7 +- x6 dh2 0
gd x4 +- x9 0 bd
gd x5 +- x7 0 bd2
path
M l b
L l bd
A bd bd cd2 cd4
L x4 t
A bd bd 3cd4 cd4
L x9 y4
L r y4
L x8 y5
L x6 y4
L x7 y4
L x7 x3
A bd2 bd2 0 -5400000
L x3 th
A bd2 bd2 3cd4 -5400000
L th b
Z
";
