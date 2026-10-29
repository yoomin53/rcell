// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 머리글·바닥글 코드 (`&L`·`&C`·`&R` 구역, `&P` `&N` `&D` `&T` `&F` `&A` `&Z`, `&"글꼴,모양"`, `&크기`,
//! `&B` `&I` `&U` `&E` `&S` `&X` `&Y` `&K색`, 한셀의 `&[PAGE NUMBER]`·`&[SHEET]` 꼴).
//!
//! 한셀 PDF (머리글·바닥글 실험): 글꼴 크기는 인쇄 배율과 상관없이 pt × 25/3 장치 단위, 기본 글꼴은 글꼴 0.
//! 날짜는 `2026-10-28`, 시간은 `06:13 오후`, `&Z` 는 문서 폴더 경로(끝에 `\`).

/// 글자 조각 하나
#[derive(Debug, Clone, PartialEq)]
pub struct HfRun {
    pub text: String,
    /// 글꼴 이름 (None = 기본 글꼴)
    pub font: Option<String>,
    pub bold: bool,
    pub italic: bool,
    /// 크기 (pt, None = 기본 크기)
    pub size: Option<f64>,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    /// RRGGBB
    pub color: Option<String>,
}

/// 왼쪽·가운데·오른쪽 구역
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HfSections {
    pub left: Vec<HfRun>,
    pub center: Vec<HfRun>,
    pub right: Vec<HfRun>,
}

impl HfSections {
    pub fn is_empty(&self) -> bool {
        [&self.left, &self.center, &self.right]
            .iter()
            .all(|s| s.iter().all(|r| r.text.is_empty()))
    }
}

/// 코드에 넣을 값
#[derive(Debug, Clone, Default)]
pub struct HfFields {
    pub page: u32,
    pub pages: u32,
    pub sheet: String,
    pub file: String,
    pub path: String,
    pub date: String,
    pub time: String,
}

/// 머리글·바닥글 원문을 구역별 조각으로
pub fn parse(src: &str, f: &HfFields) -> HfSections {
    let mut out = HfSections::default();
    // 구역 지정이 없으면 가운데 (Excel·한셀 공통)
    let mut sec = 1usize;
    let mut cur = HfRun {
        text: String::new(),
        font: None,
        bold: false,
        italic: false,
        size: None,
        underline: false,
        strike: false,
        superscript: false,
        subscript: false,
        color: None,
    };
    let push = |out: &mut HfSections, sec: usize, run: &mut HfRun| {
        if run.text.is_empty() {
            return;
        }
        let r = HfRun {
            text: std::mem::take(&mut run.text),
            ..run.clone()
        };
        match sec {
            0 => out.left.push(r),
            2 => out.right.push(r),
            _ => out.center.push(r),
        }
    };
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch != '&' || i + 1 >= chars.len() {
            cur.text.push(ch);
            i += 1;
            continue;
        }
        let c = chars[i + 1];
        i += 2;
        match c {
            '&' => cur.text.push('&'),
            'L' | 'C' | 'R' => {
                push(&mut out, sec, &mut cur);
                sec = match c {
                    'L' => 0,
                    'C' => 1,
                    _ => 2,
                };
                // 구역마다 서식은 처음부터 (한셀: 왼쪽 구역의 &"Arial,Bold"&14 가 가운데 구역에 이어지지 않는다)
                cur = HfRun {
                    text: String::new(),
                    font: None,
                    bold: false,
                    italic: false,
                    size: None,
                    underline: false,
                    strike: false,
                    superscript: false,
                    subscript: false,
                    color: None,
                };
            }
            'P' => {
                // &P+n / &P-n
                let mut n = f.page as i64;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    let sign = if chars[i] == '+' { 1 } else { -1 };
                    let mut j = i + 1;
                    let mut v = 0i64;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        v = v * 10 + chars[j].to_digit(10).unwrap_or(0) as i64;
                        j += 1;
                    }
                    if j > i + 1 {
                        n += sign * v;
                        i = j;
                    }
                }
                cur.text.push_str(&n.to_string());
            }
            'N' => cur.text.push_str(&f.pages.to_string()),
            'D' => cur.text.push_str(&f.date),
            'T' => cur.text.push_str(&f.time),
            'F' => cur.text.push_str(&f.file),
            'A' => cur.text.push_str(&f.sheet),
            'Z' => cur.text.push_str(&f.path),
            'G' => {}
            '"' => {
                // &"글꼴,모양"
                push(&mut out, sec, &mut cur);
                let mut j = i;
                while j < chars.len() && chars[j] != '"' {
                    j += 1;
                }
                let spec: String = chars[i..j].iter().collect();
                i = (j + 1).min(chars.len());
                let (name, style) = spec.split_once(',').unwrap_or((&spec, ""));
                let name = name.trim();
                cur.font = (!name.is_empty() && name != "-").then(|| name.to_string());
                let st = style.to_ascii_lowercase();
                cur.bold = st.contains("bold") || style.contains("굵게");
                cur.italic =
                    st.contains("italic") || st.contains("oblique") || style.contains("기울임");
            }
            'B' => {
                push(&mut out, sec, &mut cur);
                cur.bold = !cur.bold;
            }
            'I' => {
                push(&mut out, sec, &mut cur);
                cur.italic = !cur.italic;
            }
            'U' | 'E' => {
                push(&mut out, sec, &mut cur);
                cur.underline = !cur.underline;
            }
            'S' => {
                push(&mut out, sec, &mut cur);
                cur.strike = !cur.strike;
            }
            'X' => {
                push(&mut out, sec, &mut cur);
                cur.superscript = !cur.superscript;
            }
            'Y' => {
                push(&mut out, sec, &mut cur);
                cur.subscript = !cur.subscript;
            }
            'K' => {
                push(&mut out, sec, &mut cur);
                // &KRRGGBB 또는 &KTTSNNN (테마 색: 무시하고 검정)
                let hex: String = chars[i..chars.len().min(i + 6)].iter().collect();
                if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    cur.color = Some(hex.to_ascii_uppercase());
                    i += 6;
                }
            }
            '[' => {
                // 한셀 꼴 필드: &[PAGE NUMBER] &[SHEET] …
                let mut j = i;
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                let name: String = chars[i..j].iter().collect::<String>().to_ascii_uppercase();
                i = (j + 1).min(chars.len());
                let v = match name.trim() {
                    "PAGE" | "PAGE NUMBER" | "쪽 번호" => f.page.to_string(),
                    "PAGES" | "TOTAL PAGES" | "NUMBER OF PAGES" | "전체 쪽수" => {
                        f.pages.to_string()
                    }
                    "SHEET" | "SHEET NAME" | "TAB" | "시트 이름" => f.sheet.clone(),
                    "FILE" | "FILE NAME" | "파일 이름" => f.file.clone(),
                    "PATH" | "FILE PATH" | "파일 경로" => f.path.clone(),
                    "DATE" | "날짜" => f.date.clone(),
                    "TIME" | "시간" => f.time.clone(),
                    _ => String::new(),
                };
                cur.text.push_str(&v);
            }
            d if d.is_ascii_digit() => {
                push(&mut out, sec, &mut cur);
                let mut j = i - 1;
                let mut v = 0u32;
                while j < chars.len() && chars[j].is_ascii_digit() && j < i + 2 {
                    v = v * 10 + chars[j].to_digit(10).unwrap_or(0);
                    j += 1;
                }
                i = j;
                if v > 0 {
                    cur.size = Some(v as f64);
                }
            }
            _ => {}
        }
    }
    push(&mut out, sec, &mut cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> HfFields {
        HfFields {
            page: 2,
            pages: 4,
            sheet: "시트1".into(),
            file: "책.cell".into(),
            path: "C:\\문서\\".into(),
            date: "2026-10-28".into(),
            time: "06:13 오후".into(),
        }
    }

    #[test]
    fn sections_and_fields() {
        let s = parse("&L왼쪽 &P/&N&C&A &F&R&D &T", &fields());
        assert_eq!(s.left[0].text, "왼쪽 2/4");
        assert_eq!(s.center[0].text, "시트1 책.cell");
        assert_eq!(s.right[0].text, "2026-10-28 06:13 오후");
        // 구역이 없으면 가운데, 한셀 꼴 필드
        let s = parse("&C&\"DOTUMCHE,Regular\"&11Page &[PAGE NUMBER]", &fields());
        assert_eq!(s.center[0].text, "Page 2");
        assert_eq!(s.center[0].font.as_deref(), Some("DOTUMCHE"));
        assert_eq!(s.center[0].size, Some(11.0));
        let s = parse("&\"Arial,Bold\"&14Big &Z", &fields());
        assert!(s.center[0].bold);
        assert_eq!(s.center[0].text, "Big C:\\문서\\");
        // 구역이 바뀌면 서식은 처음부터
        let s = parse("&L&\"Arial,Bold\"&14Big&C쪽 &P", &fields());
        assert!(s.left[0].bold && s.left[0].size == Some(14.0));
        assert!(!s.center[0].bold && s.center[0].size.is_none() && s.center[0].font.is_none());
        let s = parse("&8작게 &B굵게&B a&&b", &fields());
        assert_eq!(s.center.len(), 3);
        assert!(s.center[1].bold && !s.center[2].bold);
        assert_eq!(s.center[2].text, " a&b");
    }
}
