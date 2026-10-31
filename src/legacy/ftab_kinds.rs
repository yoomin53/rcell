// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 함수 인자 칸의 토큰 클래스 종류 표 (한셀 대조 실험으로 얻은 표).
//!
//! 함수마다 `PtgFunc` 가 받는 칸의 종류 — 맨 참조(`A1:B2`)와 연산자 안의 참조(`A1:B2+1`)가 어느 클래스로 쓰이는지 —
//! 와 반환 종류. 실험: 함수마다 `f(1,…,X,…,1)` (X = `A1:B2`·`A1:B2+1`·`{1,2}`·`nm`) 을 한셀에 넣어 XML 로 연 뒤
//! 바이너리·레거시로 저장하고 토큰 클래스를 읽었다 (`docs/format/cell-binary.md` 의 "토큰 클래스").
//!
//! 칸 종류 글자 (맨 참조, 연산자 안의 참조): `V` (값, 값), `R` (참조, 값), `B` (참조, 배열), `A` (배열, 배열),
//! `D` (참조, 참조), `T` TRANSPOSE 의 칸 (값, 값 — 안의 배열 반환 함수·배열 상수는 배열), `W` (값, 값 — 다만 배열 상수는
//! 배열: IF 의 조건·CHOOSE·IFERROR 의 첫 인자·TYPE), `X` (참조, 값 — 다만 배열 상수는 값: QUOTIENT).
//! 반환 종류: `V` 값, `R` 참조 (INDEX·IF·OFFSET·CHOOSE·INDIRECT 따위), `A` 배열 (TRANSPOSE·MMULT …).
//!
//! 표에 없는 함수는 [`super::ftab::signature`] 로 얻는다.

/// (함수 번호, 반환 종류, 칸 종류 글자들). 칸 목록보다 뒤의 칸은 `signature` 의 되풀이 규칙을 따른다
pub const KINDS: &[(u16, u8, &str)] = &[
    (0, b'V', "RRRRR"),    // COUNT
    (1, b'R', "WRR"),      // IF
    (2, b'V', "V"),        // ISNA
    (3, b'V', "V"),        // ISERROR
    (4, b'V', "R"),        // SUM
    (5, b'V', "RRRRR"),    // AVERAGE
    (6, b'V', "RRRRR"),    // MIN
    (7, b'V', "RRRRR"),    // MAX
    (8, b'V', "R"),        // ROW
    (9, b'V', "R"),        // COLUMN
    (10, b'V', "V"),       // NA
    (11, b'V', "VRRRR"),   // NPV
    (12, b'V', "RRRRR"),   // STDEV
    (13, b'V', "VV"),      // DOLLAR
    (14, b'V', "VVV"),     // FIXED
    (15, b'V', "V"),       // SIN
    (16, b'V', "V"),       // COS
    (17, b'V', "V"),       // TAN
    (18, b'V', "V"),       // ATAN
    (19, b'V', "V"),       // PI
    (20, b'V', "V"),       // SQRT
    (21, b'V', "V"),       // EXP
    (22, b'V', "V"),       // LN
    (23, b'V', "V"),       // LOG10
    (24, b'V', "V"),       // ABS
    (25, b'V', "V"),       // INT
    (26, b'V', "V"),       // SIGN
    (27, b'V', "VV"),      // ROUND
    (28, b'V', "VBB"),     // LOOKUP
    (29, b'R', "BVVV"),    // INDEX
    (30, b'V', "VV"),      // REPT
    (31, b'V', "VVV"),     // MID
    (32, b'V', "V"),       // LEN
    (33, b'V', "V"),       // VALUE
    (34, b'V', "V"),       // TRUE
    (35, b'V', "V"),       // FALSE
    (36, b'V', "RRRRR"),   // AND
    (37, b'V', "RRRRR"),   // OR
    (38, b'V', "V"),       // NOT
    (39, b'V', "VV"),      // MOD
    (40, b'V', "RRR"),     // DCOUNT
    (41, b'V', "RRR"),     // DSUM
    (42, b'V', "RRR"),     // DAVERAGE
    (43, b'V', "RRR"),     // DMIN
    (44, b'V', "RRR"),     // DMAX
    (45, b'V', "RRR"),     // DSTDEV
    (46, b'V', "RRRRR"),   // VAR
    (47, b'V', "RRR"),     // DVAR
    (48, b'V', "VV"),      // TEXT
    (49, b'V', "BBVV"),    // LINEST
    (50, b'V', "BBBV"),    // TREND
    (51, b'V', "BBVV"),    // LOGEST
    (52, b'V', "BBBV"),    // GROWTH
    (56, b'V', "VVVVV"),   // PV
    (57, b'V', "VVVVV"),   // FV
    (58, b'V', "VVVVV"),   // NPER
    (59, b'V', "VVVVV"),   // PMT
    (60, b'V', "VVVVV"),   // RATE
    (61, b'V', "RVV"),     // MIRR
    (62, b'V', "RV"),      // IRR
    (63, b'V', "V"),       // RAND
    (64, b'V', "VRR"),     // MATCH
    (65, b'V', "VVV"),     // DATE
    (66, b'V', "VVV"),     // TIME
    (67, b'V', "V"),       // DAY
    (68, b'V', "V"),       // MONTH
    (69, b'V', "V"),       // YEAR
    (70, b'V', "VV"),      // WEEKDAY
    (71, b'V', "V"),       // HOUR
    (72, b'V', "V"),       // MINUTE
    (73, b'V', "V"),       // SECOND
    (74, b'V', "V"),       // NOW
    (75, b'V', "R"),       // AREAS
    (76, b'V', "B"),       // ROWS
    (77, b'V', "B"),       // COLUMNS
    (78, b'R', "RVVVV"),   // OFFSET
    (82, b'V', "VVV"),     // SEARCH
    (83, b'A', "T"),       // TRANSPOSE
    (86, b'V', "W"),       // TYPE
    (97, b'V', "VV"),      // ATAN2
    (98, b'V', "V"),       // ASIN
    (99, b'V', "V"),       // ACOS
    (100, b'R', "WVRRR"),  // CHOOSE
    (101, b'V', "VBBV"),   // HLOOKUP
    (102, b'V', "VBBV"),   // VLOOKUP
    (105, b'V', "R"),      // ISREF
    (109, b'V', "VV"),     // LOG
    (111, b'V', "V"),      // CHAR
    (112, b'V', "V"),      // LOWER
    (113, b'V', "V"),      // UPPER
    (114, b'V', "V"),      // PROPER
    (115, b'V', "VV"),     // LEFT
    (116, b'V', "VV"),     // RIGHT
    (117, b'V', "VV"),     // EXACT
    (118, b'V', "V"),      // TRIM
    (119, b'V', "VVVV"),   // REPLACE
    (120, b'V', "VVVV"),   // SUBSTITUTE
    (121, b'V', "V"),      // CODE
    (124, b'V', "VVV"),    // FIND
    (125, b'V', "VR"),     // CELL
    (126, b'V', "V"),      // ISERR
    (127, b'V', "V"),      // ISTEXT
    (128, b'V', "V"),      // ISNUMBER
    (129, b'V', "V"),      // ISBLANK
    (130, b'V', "R"),      // T
    (131, b'V', "R"),      // N
    (140, b'V', "V"),      // DATEVALUE
    (141, b'V', "V"),      // TIMEVALUE
    (142, b'V', "VVV"),    // SLN
    (143, b'V', "VVVV"),   // SYD
    (144, b'V', "VVVVV"),  // DDB
    (148, b'R', "VV"),     // INDIRECT
    (150, b'V', "VVVVV"),  // CALL
    (162, b'V', "V"),      // CLEAN
    (163, b'V', "A"),      // MDETERM
    (164, b'A', "A"),      // MINVERSE
    (165, b'A', "AA"),     // MMULT
    (167, b'V', "VVVVV"),  // IPMT
    (168, b'V', "VVVVV"),  // PPMT
    (169, b'V', "RRRRR"),  // COUNTA
    (183, b'V', "RRRRR"),  // PRODUCT
    (184, b'V', "V"),      // FACT
    (189, b'V', "RRR"),    // DPRODUCT
    (190, b'V', "V"),      // ISNONTEXT
    (193, b'V', "RRRRR"),  // STDEVP
    (194, b'V', "RRRRR"),  // VARP
    (195, b'V', "RRR"),    // DSTDEVP
    (196, b'V', "RRR"),    // DVARP
    (197, b'V', "VV"),     // TRUNC
    (198, b'V', "V"),      // ISLOGICAL
    (199, b'V', "RRR"),    // DCOUNTA
    (204, b'V', "VV"),     // USDOLLAR
    (205, b'V', "VVV"),    // FINDB
    (206, b'V', "VVV"),    // SEARCHB
    (207, b'V', "VVVV"),   // REPLACEB
    (208, b'V', "VV"),     // LEFTB
    (209, b'V', "VV"),     // RIGHTB
    (210, b'V', "VVV"),    // MIDB
    (211, b'V', "V"),      // LENB
    (212, b'V', "VV"),     // ROUNDUP
    (213, b'V', "VV"),     // ROUNDDOWN
    (214, b'V', "V"),      // ASC
    (215, b'V', "V"),      // DBCS
    (216, b'V', "VR"),     // RANK
    (219, b'V', "VVVVV"),  // ADDRESS
    (220, b'V', "VVV"),    // DAYS360
    (221, b'V', "V"),      // TODAY
    (222, b'V', "VVVVV"),  // VDB
    (227, b'V', "RRRRR"),  // MEDIAN
    (228, b'V', "A"),      // SUMPRODUCT
    (229, b'V', "V"),      // SINH
    (230, b'V', "V"),      // COSH
    (231, b'V', "V"),      // TANH
    (232, b'V', "V"),      // ASINH
    (233, b'V', "V"),      // ACOSH
    (234, b'V', "V"),      // ATANH
    (235, b'V', "RRR"),    // DGET
    (244, b'V', "V"),      // INFO
    (247, b'V', "VVVVV"),  // DB
    (252, b'A', "RR"),     // FREQUENCY
    (261, b'V', "V"),      // ERROR.TYPE
    (269, b'V', "RRRRR"),  // AVEDEV
    (270, b'V', "VVVVV"),  // BETADIST
    (271, b'V', "V"),      // GAMMALN
    (272, b'V', "VVVVV"),  // BETAINV
    (273, b'V', "VVVV"),   // BINOMDIST
    (274, b'V', "VV"),     // CHIDIST
    (275, b'V', "VV"),     // CHIINV
    (276, b'V', "VV"),     // COMBIN
    (277, b'V', "VVV"),    // CONFIDENCE
    (278, b'V', "VVV"),    // CRITBINOM
    (279, b'V', "V"),      // EVEN
    (280, b'V', "VVV"),    // EXPONDIST
    (281, b'V', "VVV"),    // FDIST
    (282, b'V', "VVV"),    // FINV
    (283, b'V', "V"),      // FISHER
    (284, b'V', "V"),      // FISHERINV
    (285, b'V', "VV"),     // FLOOR
    (286, b'V', "VVVV"),   // GAMMADIST
    (288, b'V', "VV"),     // CEILING
    (289, b'V', "VVVV"),   // HYPGEOMDIST
    (290, b'V', "VVV"),    // LOGNORMDIST
    (291, b'V', "VVV"),    // LOGINV
    (292, b'V', "VVV"),    // NEGBINOMDIST
    (293, b'V', "VVVV"),   // NORMDIST
    (294, b'V', "V"),      // NORMSDIST
    (295, b'V', "VVV"),    // NORMINV
    (296, b'V', "V"),      // NORMSINV
    (297, b'V', "VVV"),    // STANDARDIZE
    (298, b'V', "V"),      // ODD
    (299, b'V', "VV"),     // PERMUT
    (300, b'V', "VVV"),    // POISSON
    (301, b'V', "VVV"),    // TDIST
    (302, b'V', "VVVV"),   // WEIBULL
    (303, b'V', "AA"),     // SUMXMY2
    (304, b'V', "AA"),     // SUMX2MY2
    (305, b'V', "AA"),     // SUMX2PY2
    (306, b'V', "AA"),     // CHITEST
    (307, b'V', "AA"),     // CORREL
    (308, b'V', "AA"),     // COVAR
    (309, b'V', "VAA"),    // FORECAST
    (310, b'V', "AA"),     // FTEST
    (311, b'V', "AA"),     // INTERCEPT
    (312, b'V', "AA"),     // PEARSON
    (313, b'V', "AA"),     // RSQ
    (314, b'V', "AA"),     // STEYX
    (315, b'V', "AA"),     // SLOPE
    (317, b'V', "AAVV"),   // PROB
    (318, b'V', "RRRRR"),  // DEVSQ
    (319, b'V', "RRRRR"),  // GEOMEAN
    (320, b'V', "RRRRR"),  // HARMEAN
    (321, b'V', "RRRRR"),  // SUMSQ
    (322, b'V', "RRRRR"),  // KURT
    (323, b'V', "RRRRR"),  // SKEW
    (324, b'V', "RVV"),    // ZTEST
    (325, b'V', "RV"),     // LARGE
    (326, b'V', "RV"),     // SMALL
    (327, b'V', "RV"),     // QUARTILE
    (328, b'V', "RV"),     // PERCENTILE
    (329, b'V', "RVV"),    // PERCENTRANK
    (330, b'V', "AAAAA"),  // MODE
    (331, b'V', "RV"),     // TRIMMEAN
    (332, b'V', "VV"),     // TINV
    (336, b'V', "VVVVV"),  // CONCATENATE
    (337, b'V', "VV"),     // POWER
    (342, b'V', "V"),      // RADIANS
    (343, b'V', "V"),      // DEGREES
    (344, b'V', "VRRRR"),  // SUBTOTAL
    (345, b'V', "RVR"),    // SUMIF
    (346, b'V', "RV"),     // COUNTIF
    (347, b'V', "R"),      // COUNTBLANK
    (350, b'V', "VVVV"),   // ISPMT
    (351, b'V', "VVV"),    // DATEDIF
    (352, b'R', "D"),      // DATESTRING
    (353, b'V', "VV"),     // NUMBERSTRING
    (354, b'V', "VV"),     // ROMAN
    (358, b'V', "RRRRR"),  // GETPIVOTDATA
    (359, b'V', "VV"),     // HYPERLINK
    (360, b'V', "R"),      // PHONETIC
    (361, b'V', "RRRRR"),  // AVERAGEA
    (362, b'V', "RRRRR"),  // MAXA
    (363, b'V', "RRRRR"),  // MINA
    (364, b'V', "RRRRR"),  // STDEVPA
    (365, b'V', "RRRRR"),  // VARPA
    (366, b'V', "RRRRR"),  // STDEVA
    (367, b'V', "RRRRR"),  // VARA
    (368, b'R', "D"),      // BAHTTEXT
    (369, b'R', "D"),      // THAIDAYOFWEEK
    (370, b'R', "D"),      // THAIDIGIT
    (371, b'R', "D"),      // THAIMONTHOFYEAR
    (372, b'R', "D"),      // THAINUMSOUND
    (373, b'R', "D"),      // THAINUMSTRING
    (374, b'R', "D"),      // THAISTRINGLENGTH
    (375, b'R', "D"),      // ISTHAIDIGIT
    (376, b'R', "D"),      // ROUNDBAHTDOWN
    (377, b'R', "D"),      // ROUNDBAHTUP
    (378, b'R', "D"),      // THAIYEAR
    (379, b'R', "DDDDD"),  // RTD
    (380, b'R', "DDDDD"),  // CUBEVALUE
    (381, b'R', "DDDDD"),  // CUBEMEMBER
    (382, b'R', "DDD"),    // CUBEMEMBERPROPERTY
    (383, b'R', "DDDDD"),  // CUBERANKEDMEMBER
    (384, b'V', "RR"),     // HEX2BIN
    (385, b'V', "R"),      // HEX2DEC
    (386, b'V', "RR"),     // HEX2OCT
    (387, b'V', "RR"),     // DEC2BIN
    (388, b'V', "RR"),     // DEC2HEX
    (389, b'V', "RR"),     // DEC2OCT
    (390, b'V', "RR"),     // OCT2BIN
    (391, b'V', "RR"),     // OCT2HEX
    (392, b'V', "R"),      // OCT2DEC
    (393, b'V', "R"),      // BIN2DEC
    (394, b'V', "RR"),     // BIN2OCT
    (395, b'V', "RR"),     // BIN2HEX
    (396, b'V', "RR"),     // IMSUB
    (397, b'V', "RR"),     // IMDIV
    (398, b'V', "RR"),     // IMPOWER
    (399, b'V', "R"),      // IMABS
    (400, b'V', "R"),      // IMSQRT
    (401, b'V', "R"),      // IMLN
    (402, b'V', "R"),      // IMLOG2
    (403, b'V', "R"),      // IMLOG10
    (404, b'V', "R"),      // IMSIN
    (405, b'V', "R"),      // IMCOS
    (406, b'V', "R"),      // IMEXP
    (407, b'V', "R"),      // IMARGUMENT
    (408, b'V', "R"),      // IMCONJUGATE
    (409, b'V', "R"),      // IMAGINARY
    (410, b'V', "R"),      // IMREAL
    (411, b'V', "RRR"),    // COMPLEX
    (412, b'V', "RRRRR"),  // IMSUM
    (413, b'V', "RRRRR"),  // IMPRODUCT
    (414, b'V', "RRRR"),   // SERIESSUM
    (415, b'V', "R"),      // FACTDOUBLE
    (416, b'V', "R"),      // SQRTPI
    (417, b'V', "XX"),     // QUOTIENT
    (418, b'V', "RR"),     // DELTA
    (419, b'V', "RR"),     // GESTEP
    (420, b'V', "R"),      // ISEVEN
    (421, b'V', "R"),      // ISODD
    (422, b'V', "RR"),     // MROUND
    (423, b'V', "RR"),     // ERF
    (424, b'V', "R"),      // ERFC
    (425, b'V', "RR"),     // BESSELJ
    (426, b'V', "RR"),     // BESSELK
    (427, b'V', "RR"),     // BESSELY
    (428, b'V', "RR"),     // BESSELI
    (429, b'V', "RRR"),    // XIRR
    (430, b'V', "RRR"),    // XNPV
    (431, b'V', "RRRRR"),  // PRICEMAT
    (432, b'V', "RRRRR"),  // YIELDMAT
    (433, b'V', "RRRRR"),  // INTRATE
    (434, b'V', "RRRRR"),  // RECEIVED
    (435, b'V', "RRRRR"),  // DISC
    (436, b'V', "RRRRR"),  // PRICEDISC
    (437, b'V', "RRRRR"),  // YIELDDISC
    (438, b'V', "RRR"),    // TBILLEQ
    (439, b'V', "RRR"),    // TBILLPRICE
    (440, b'V', "RRR"),    // TBILLYIELD
    (443, b'V', "RR"),     // DOLLARDE
    (444, b'V', "RR"),     // DOLLARFR
    (445, b'V', "RR"),     // NOMINAL
    (446, b'V', "RR"),     // EFFECT
    (447, b'V', "RRRRR"),  // CUMPRINC
    (448, b'V', "RRRRR"),  // CUMIPMT
    (449, b'V', "RR"),     // EDATE
    (450, b'V', "RR"),     // EOMONTH
    (451, b'V', "RRR"),    // YEARFRAC
    (452, b'V', "RRRR"),   // COUPDAYBS
    (453, b'V', "RRRR"),   // COUPDAYS
    (454, b'V', "RRRR"),   // COUPDAYSNC
    (455, b'V', "RRRR"),   // COUPNCD
    (456, b'V', "RRRR"),   // COUPNUM
    (457, b'V', "RRRR"),   // COUPPCD
    (458, b'V', "RRRRR"),  // DURATION
    (459, b'V', "RRRRR"),  // MDURATION
    (464, b'V', "RR"),     // RANDBETWEEN
    (465, b'V', "RR"),     // WEEKNUM
    (468, b'V', "RRR"),    // CONVERT
    (470, b'V', "RRRRR"),  // ACCRINTM
    (471, b'V', "RRR"),    // WORKDAY
    (472, b'V', "RRR"),    // NETWORKDAYS
    (473, b'V', "RRRRR"),  // GCD
    (474, b'V', "RRRRR"),  // MULTINOMIAL
    (475, b'V', "RRRRR"),  // LCM
    (476, b'V', "RR"),     // FVSCHEDULE
    (477, b'R', "DDDDD"),  // CUBEKPIMEMBER
    (478, b'R', "DDDDD"),  // CUBESET
    (479, b'R', "D"),      // CUBESETCOUNT
    (480, b'V', "WR"),     // IFERROR
    (481, b'V', "R"),      // COUNTIFS
    (482, b'V', "RRRRR"),  // SUMIFS
    (483, b'V', "RRR"),    // AVERAGEIF
    (484, b'V', "RRRRR"),  // AVERAGEIFS
    (1904, b'V', "VRRRR"), // SUBTOTALA
    (1907, b'V', "RRRR"),  // CONVERTK
];

/// 한셀이 `_xlfn.` 이름 함수(사용자 함수 꼴로 부르지만 반환은 값)로 저장하는 새 함수의 칸 종류 (함수 이름, 칸 종류 글자들).
/// 칸 목록보다 뒤의 칸은 마지막 글자를 따른다. 같은 실험 (`_xlfn.` 접두사를 붙여 함수마다 `f(1,…,X,…,1)` 을 저장)
pub const FUTURE_KINDS: &[(&str, &str)] = &[
    ("ACOT", "V"),
    ("ACOTH", "V"),
    ("ARABIC", "R"),
    ("BASE", "VVV"),
    ("BETA.INV", "VVVVV"),
    ("BINOM.DIST", "VVVV"),
    ("BINOM.DIST.RANGE", "VVVV"),
    ("BINOM.INV", "VVV"),
    ("BITAND", "RR"),
    ("BITLSHIFT", "RR"),
    ("BITOR", "RR"),
    ("BITRSHIFT", "RR"),
    ("BITXOR", "RR"),
    ("CEILING.MATH", "VVV"),
    ("CEILING.PRECISE", "VV"),
    ("CHISQ.DIST", "VVV"),
    ("CHISQ.DIST.RT", "VV"),
    ("CHISQ.INV", "VV"),
    ("CHISQ.INV.RT", "VV"),
    ("CHISQ.TEST", "RR"),
    ("COMBINA", "VV"),
    ("CONCAT", "RRR"),
    ("CONFIDENCE.NORM", "VVV"),
    ("CONFIDENCE.T", "VVV"),
    ("COT", "V"),
    ("COTH", "V"),
    ("COVARIANCE.P", "AA"),
    ("COVARIANCE.S", "RR"),
    ("CSC", "V"),
    ("CSCH", "V"),
    ("DAYS", "VV"),
    ("DECIMAL", "VV"),
    ("ERF.PRECISE", "R"),
    ("ERFC.PRECISE", "R"),
    ("EXPON.DIST", "VVV"),
    ("F.DIST", "VVVV"),
    ("F.DIST.RT", "VVV"),
    ("F.INV", "VVV"),
    ("F.INV.RT", "VVV"),
    ("F.TEST", "AA"),
    ("FLOOR.MATH", "VVV"),
    ("FLOOR.PRECISE", "VV"),
    ("FORMULATEXT", "R"),
    ("GAMMA", "V"),
    ("GAMMA.DIST", "VVVV"),
    ("GAMMALN.PRECISE", "V"),
    ("GAUSS", "V"),
    ("HYPGEOM.DIST", "VVVVV"),
    ("IFNA", "VR"),
    ("IFS", "RRRR"),
    ("IMCOSH", "R"),
    ("IMCOT", "R"),
    ("IMCSC", "R"),
    ("IMCSCH", "R"),
    ("IMSEC", "R"),
    ("IMSECH", "R"),
    ("IMSINH", "R"),
    ("IMTAN", "R"),
    ("ISFORMULA", "V"),
    ("ISO.CEILING", "VV"),
    ("LOGNORM.DIST", "VVVV"),
    ("LOGNORM.INV", "VVV"),
    ("MAXIFS", "RRRRR"),
    ("MINIFS", "RRRRR"),
    ("MODE.MULT", "AAA"),
    ("MODE.SNGL", "AAA"),
    ("MUNIT", "R"),
    ("NEGBINOM.DIST", "VVVV"),
    ("NETWORKDAYS.INTL", "RRRR"),
    ("NORM.DIST", "VVVV"),
    ("NORM.INV", "VVV"),
    ("NORM.S.DIST", "RR"),
    ("NORM.S.INV", "V"),
    ("NUMBERVALUE", "RRR"),
    ("PERCENTILE.EXC", "RV"),
    ("PERCENTILE.INC", "RV"),
    ("PERCENTRANK.EXC", "RVV"),
    ("PERCENTRANK.INC", "RVV"),
    ("PERMUTATIONA", "VV"),
    ("PHI", "V"),
    ("POISSON.DIST", "VVV"),
    ("QUARTILE.EXC", "RV"),
    ("QUARTILE.INC", "RV"),
    ("SEC", "V"),
    ("SECH", "V"),
    ("SEQUENCE", "RRR"),
    ("SHEET", "R"),
    ("SHEETS", "R"),
    ("SKEW.P", "RRR"),
    ("STDEV.P", "RRR"),
    ("STDEV.S", "RRR"),
    ("SWITCH", "RRRRR"),
    ("T.DIST", "VVV"),
    ("T.DIST.2T", "VV"),
    ("T.DIST.RT", "VV"),
    ("T.INV", "VV"),
    ("T.INV.2T", "VV"),
    ("TEXTJOIN", "RRRRR"),
    ("UNICHAR", "R"),
    ("UNICODE", "R"),
    ("UNIQUE", "RRR"),
    ("VAR.P", "RRR"),
    ("VAR.S", "RRR"),
    ("WEIBULL.DIST", "VVVV"),
    ("WORKDAY.INTL", "RRRR"),
    ("XLOOKUP", "RRRRR"),
    ("XOR", "AAA"),
    ("Z.TEST", "RVV"),
];
