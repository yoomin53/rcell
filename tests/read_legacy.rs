// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 `.cell` 의 Workbook 스트림을 레코드 단위로 직접 만들어 읽기를 검사한다.
//! (한셀 샘플 파일은 저장소에 넣지 않는다 — docs/format/cell-legacy.md 의 배치를 따른다.)

use rcell::legacy::reader::read_workbook_stream;
use rcell::model::{CellRef, CellValue};

#[derive(Default)]
struct Biff(Vec<u8>);

impl Biff {
    fn rec(&mut self, rtype: u16, data: &[u8]) -> usize {
        let at = self.0.len();
        self.0.extend_from_slice(&rtype.to_le_bytes());
        self.0.extend_from_slice(&(data.len() as u16).to_le_bytes());
        self.0.extend_from_slice(data);
        at
    }
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn bof(dt: u16) -> Vec<u8> {
    let mut d = vec![0x01, 0x10];
    d.extend_from_slice(&dt.to_le_bytes());
    d.extend_from_slice(&[0x02, 0x01, 0xd8, 0x07, 0, 0, 0, 0, 0x06, 0, 0, 0]);
    d
}

/// 외부 SUPBOOK: ctab, cch, flag, 가상 경로, (cch, flag, 시트 이름) × ctab
fn supbook(path: &str, sheets: &[&str]) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&(sheets.len() as u16).to_le_bytes());
    d.extend_from_slice(&(path.encode_utf16().count() as u16).to_le_bytes());
    d.push(0);
    d.extend(utf16(path));
    for s in sheets {
        d.extend_from_slice(&(s.encode_utf16().count() as u16).to_le_bytes());
        d.push(0);
        d.extend(utf16(s));
    }
    d
}

fn cell_head(row: u32, col: u16) -> Vec<u8> {
    let mut d = row.to_le_bytes().to_vec();
    d.extend_from_slice(&col.to_le_bytes());
    d.extend_from_slice(&15u16.to_le_bytes());
    d
}

fn workbook_stream() -> Vec<u8> {
    let mut b = Biff::default();
    b.rec(0x0809, &bof(0x0005));
    // 통합 문서 코드 이름과 WINDOW1 (한셀이 쓴 그대로: 240,120 16000x9500, 탭 비율 723, 첫 탭 1, grbit 0x38)
    let mut code = 6u16.to_le_bytes().to_vec();
    code.push(0);
    code.extend(utf16("MyBook"));
    b.rec(0x01BA, &code);
    b.rec(
        0x003D,
        &[
            0xf0, 0, 0x78, 0, 0x80, 0x3e, 0x1c, 0x25, 0x38, 0, 0, 0, 1, 0, 1, 0, 0xd3, 0x02,
        ],
    );
    // BOUNDSHEET: 위치는 나중에 채운다
    let mut bs = vec![0, 0, 0, 0, 0, 0, 4, 0];
    bs.extend(utf16("Sheet"));
    let bs_at = b.rec(0x0085, &bs);
    // SUPBOOK: 이 통합 문서, 쓰이지 않는 외부 문서, 쓰이는 외부 문서
    b.rec(0x01AE, &[1, 0, 0x01, 0x04]);
    b.rec(0x01AE, &supbook("Unused.xls", &["X"]));
    b.rec(
        0x01AE,
        &supbook("\u{1}\u{2}Dir\u{3}Book One.xls", &["Data"]),
    );
    // EXTERNSHEET: XTI 0 = 이 문서 시트 0, XTI 1 = 외부 #1, XTI 2 = 외부 #2
    let mut xti = 3u16.to_le_bytes().to_vec();
    for (book, tab) in [(0u16, 0u16), (1, 0), (2, 0)] {
        xti.extend_from_slice(&book.to_le_bytes());
        xti.extend_from_slice(&tab.to_le_bytes());
        xti.extend_from_slice(&tab.to_le_bytes());
    }
    b.rec(0x0017, &xti);
    // NAME "Link" = PtgRef3d(XTI 2, $B$4)
    let rgce = [0x3A, 2, 0, 3, 0, 0, 0, 1, 0];
    let mut name = vec![0, 0, 0, 4];
    name.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    name.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    name.extend(utf16("Link"));
    name.extend_from_slice(&rgce);
    b.rec(0x0018, &name);
    b.rec(0x000A, &[]);

    let sheet_at = b.rec(0x0809, &bof(0x0010)) as u32;
    // COLINFO A:B 너비 9, 0xB00E 257–16384열
    b.rec(0x007D, &[0, 0, 1, 0, 0, 9, 15, 0, 2, 0, 0, 0]);
    b.rec(0xB00E, &[0, 1, 0xff, 0x3f, 0x24, 0x09, 15, 0, 0, 0, 0, 0]);
    // ROW 3 (0부터 2): 높이 15pt 지정
    let mut row = 2u32.to_le_bytes().to_vec();
    row.extend_from_slice(&[0, 0, 2, 0, 0x2c, 0x01, 0, 0, 0, 0, 0x40, 0, 0x0f, 0]);
    b.rec(0x0208, &row);
    // A3 = 1, B3 = A3 (행·열 상대 PtgRef)
    let mut num = cell_head(2, 0);
    num.extend_from_slice(&1.0f64.to_le_bytes());
    b.rec(0x0203, &num);
    let mut fm = cell_head(2, 1);
    fm.extend_from_slice(&1.0f64.to_le_bytes());
    fm.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let tokens = [0x44, 2, 0, 0, 0, 0x00, 0xC0];
    fm.extend_from_slice(&(tokens.len() as u16).to_le_bytes());
    fm.extend_from_slice(&tokens);
    b.rec(0x0006, &fm);
    // 행 반복: 3행을 4–5행에
    b.rec(0xB002, &[2, 0, 0, 0, 4, 0, 0, 0]);
    view_and_print_records(&mut b);
    // 실제 파일에서는 메모도 OfficeArt 도형을 가진다. 여기서는 메모 도형이 없으므로 그리기를 먼저 둔다
    drawing_records(&mut b);
    note_records(&mut b);
    b.rec(0x000A, &[]);

    b.0[bs_at + 4..bs_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    b.0
}

/// 보기·인쇄 레코드 (한셀이 쓴 배치 그대로, docs/format/cell-legacy.md 6.1절)
fn view_and_print_records(b: &mut Biff) {
    // WINDOW2: 고정(0x0008|0x0100) + 눈금선·머리글·0 표시·선택, rwTop 0, colLeft 0, …, 확대율
    b.rec(
        0x023E,
        &[
            0xbe, 0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x4b, 0, 0x55, 0, 0, 0, 0, 0x80,
        ],
    );
    b.rec(0x00A0, &[0x55, 0, 0x64, 0]); // SCL 85%
    // PANE: x 1, y 2, rwTop 2, colLeft 1, 활성 창 0 (오른쪽 아래)
    b.rec(0x0041, &[1, 0, 2, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0]);
    // SELECTION: 창 3 (쓰지 않음), 창 0 = C4, C4:D5
    let mut sel = |pnn: u8| {
        let mut d = vec![pnn, 3, 0, 0, 0, 2, 0, 0, 0, 1, 0];
        d.extend_from_slice(&[3, 0, 0, 0, 4, 0, 0, 0, 2, 0, 3, 0]);
        b.rec(0x001D, &d);
    };
    sel(3);
    sel(0);
    b.rec(0x0081, &[0xc1, 0x05]); // WSBOOL: 페이지에 맞추기
    b.rec(0x002B, &[1, 0]); // PRINTGRIDLINES
    b.rec(0x0083, &[1, 0]); // HCENTER
    b.rec(0x0026, &0.5f64.to_le_bytes()); // LEFTMARGIN
    // SETUP: A4, 85%, 시작 3, 맞춤 2×3, 가로 + 페이지 번호 지정, 600dpi, 머리글/바닥글 여백 0.3, 2부
    let mut setup = vec![
        9, 0, 0x55, 0, 3, 0, 2, 0, 3, 0, 0x80, 0, 0x58, 0x02, 0x58, 0x02,
    ];
    setup.extend_from_slice(&0.3f64.to_le_bytes());
    setup.extend_from_slice(&0.3f64.to_le_bytes());
    setup.extend_from_slice(&[2, 0]);
    b.rec(0x00A1, &setup);
    // HEADER: 레거시의 글꼴 코드는 "이름, REGULAR " 꼴
    let text = "&C&\"굴림, REGULAR \"&P";
    let mut hdr = (text.encode_utf16().count() as u16).to_le_bytes().to_vec();
    hdr.push(0);
    hdr.extend(utf16(text));
    b.rec(0x0014, &hdr);
    // 페이지 나누기: 행 10, 열 3
    b.rec(0x001B, &[1, 0, 10, 0, 0, 0, 0, 0, 0xff, 0x3f]);
    b.rec(0x001A, &[1, 0, 3, 0, 0, 0, 0xff, 0xff, 0x0f, 0]);
    // CODENAME, SHEETEXT(탭 색 COLORREF 00B050)
    let mut code = 6u16.to_le_bytes().to_vec();
    code.push(0);
    code.extend(utf16("Sheet1"));
    b.rec(0x01BA, &code);
    let mut ext = vec![0x62, 0x08];
    ext.extend_from_slice(&[0; 10]);
    ext.extend_from_slice(&[0x14, 0, 0, 0, 0x7f, 0, 0, 0, 0x00, 0xb0, 0x50, 0]);
    b.rec(0x0862, &ext);
}

/// 메모: OBJ(ftCmo, 종류 0x19 = 메모, 번호 7) → TXO(내용) → … → NOTE(셀·작성자·번호)
fn note_records(b: &mut Biff) {
    let mut obj = vec![0x15, 0, 0x12, 0, 0x19, 0, 7, 0, 0x11, 0x40];
    obj.extend_from_slice(&[0; 16]);
    b.rec(0x005D, &obj);
    let text = "메모 내용";
    let mut txo = vec![0, 0, 1, 0, 1, 0, 1, 0, 1, 0];
    txo.extend_from_slice(&(text.encode_utf16().count() as u16).to_le_bytes());
    txo.push(0);
    txo.extend(utf16(text));
    txo.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    b.rec(0x01B6, &txo);
    // NOTE: C2 (행 1, 열 2), 표시(0x0002), 번호 7, 작성자
    let mut note = 1u32.to_le_bytes().to_vec();
    note.extend_from_slice(&[2, 0, 2, 0, 7, 0]);
    note.extend_from_slice(&8u16.to_le_bytes());
    note.push(0);
    note.extend(utf16("yoomin53"));
    b.rec(0x001C, &note);
}

/// OfficeArt 레코드: ver/inst u16, type u16, len u32 + 내용
fn art(ver: u16, inst: u16, rtype: u16, body: &[u8]) -> Vec<u8> {
    let mut d = (ver | (inst << 4)).to_le_bytes().to_vec();
    d.extend_from_slice(&rtype.to_le_bytes());
    d.extend_from_slice(&(body.len() as u32).to_le_bytes());
    d.extend_from_slice(body);
    d
}

/// 글상자 하나: 사각형(1), 이름 "글상자 1", 채우기 FFFFE1, 선 없음, 앵커 B2(+½열)–D5, 글 "안녕"
fn drawing_records(b: &mut Biff) {
    let patriarch = [
        art(0x1, 0, 0xF009, &[0; 16]),
        art(0x2, 0, 0xF00A, &[0x00, 0x04, 0, 0, 0x05, 0, 0, 0]),
    ]
    .concat();
    let name: Vec<u8> = "글상자 1\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut opt = Vec::new();
    for (pid, v) in [
        (0x0181u16, 0x00E1FFFFu32),
        (0x01FF, 0x0008_0000),
        (0x0087, 1),
    ] {
        opt.extend_from_slice(&pid.to_le_bytes());
        opt.extend_from_slice(&v.to_le_bytes());
    }
    opt.extend_from_slice(&(0x0380u16 | 0x8000).to_le_bytes());
    opt.extend_from_slice(&(name.len() as u32).to_le_bytes());
    opt.extend_from_slice(&name);
    // 앵커 (한컴 22바이트: 행 u32): flag, col1 1, dx1 ½, row1 1, dy1 0, col2 3, dx2 0, row2 4, dy2 0
    let mut anchor = vec![0, 0, 1, 0, 0x00, 0x40];
    anchor.extend_from_slice(&1u32.to_le_bytes());
    anchor.extend_from_slice(&[0, 0, 3, 0, 0, 0]);
    anchor.extend_from_slice(&4u32.to_le_bytes());
    anchor.extend_from_slice(&[0, 0]);
    let shape_head = [
        art(0x2, 1, 0xF00A, &[0x01, 0x04, 0, 0, 0x00, 0x0A, 0, 0]),
        art(0x3, 4, 0xF00B, &opt),
        art(0x0, 0, 0xF010, &anchor),
        art(0x0, 0, 0xF011, &[]),
    ]
    .concat();
    let textbox = art(0x0, 0, 0xF00D, &[]);
    let sp_patriarch = art(0xF, 0, 0xF004, &patriarch);
    let sp_len = shape_head.len() + textbox.len();
    let mut sp = (0x000Fu16).to_le_bytes().to_vec();
    sp.extend_from_slice(&0xF004u16.to_le_bytes());
    sp.extend_from_slice(&(sp_len as u32).to_le_bytes());
    let spgr_len = sp_patriarch.len() + sp.len() + sp_len;
    let mut spgr = (0x000Fu16).to_le_bytes().to_vec();
    spgr.extend_from_slice(&0xF003u16.to_le_bytes());
    spgr.extend_from_slice(&(spgr_len as u32).to_le_bytes());
    let dg = art(0x0, 1, 0xF008, &[1, 0, 0, 0, 0x01, 0x04, 0, 0]);
    let dg_len = dg.len() + spgr.len() + spgr_len;
    let mut first = (0x000Fu16).to_le_bytes().to_vec();
    first.extend_from_slice(&0xF002u16.to_le_bytes());
    first.extend_from_slice(&(dg_len as u32).to_le_bytes());
    first.extend(dg);
    first.extend(spgr);
    first.extend(sp_patriarch);
    first.extend(sp);
    first.extend(shape_head);
    // MSODRAWING(도형) → OBJ(ot 0x1E) → MSODRAWING(ClientTextbox) → TXO
    b.rec(0x00EC, &first);
    let mut obj = vec![0x15, 0, 0x12, 0, 0x1E, 0, 1, 0, 0x11, 0x60];
    obj.extend_from_slice(&[0; 16]);
    b.rec(0x005D, &obj);
    b.rec(0x00EC, &textbox);
    let text = "안녕";
    let mut txo = vec![0, 0, 1, 0, 1, 0, 1, 0, 1, 0];
    txo.extend_from_slice(&(text.encode_utf16().count() as u16).to_le_bytes());
    txo.push(0);
    txo.extend(utf16(text));
    txo.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    b.rec(0x01B6, &txo);
}

#[test]
fn office_art_text_box_becomes_a_shape() {
    use rcell::model::{Paint, ShapeKind};
    let wb = read_workbook_stream(&workbook_stream()).unwrap();
    let s = &wb.sheets[0].shapes;
    assert_eq!(s.len(), 1);
    let t = &s[0];
    assert_eq!((t.kind, t.name.as_str()), (ShapeKind::Shape, "글상자 1"));
    assert_eq!(t.geometry.as_deref(), Some("rect"));
    assert_eq!(t.plain_text(), "안녕");
    assert_eq!(t.text.as_ref().unwrap().anchor.as_deref(), Some("ctr"));
    match &t.fill {
        Some(Paint::Solid(c)) => assert_eq!(c.value, "srgb:FFFFE1"),
        other => panic!("{other:?}"),
    }
    assert_eq!(t.line.as_ref().unwrap().fill, Some(Paint::None));
    // 글꼴이 없으면 11pt → MDW 8. B 열 9 글자 → 72px, 실효 71px 의 ½ = 35.5 → 36px
    let a = &t.anchor;
    assert_eq!((a.from.col, a.from.row), (1, 1));
    assert_eq!(a.from.col_off, 36 * 9525);
    assert_eq!((a.to.unwrap().col, a.to.unwrap().row), (3, 4));
}

#[test]
fn notes_join_obj_txo_and_note_records() {
    let wb = read_workbook_stream(&workbook_stream()).unwrap();
    let c = &wb.sheets[0].comments;
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].cell.to_string(), "C2");
    assert_eq!(c[0].author, "yoomin53");
    assert_eq!(c[0].text.text, "메모 내용");
    assert!(c[0].visible);
}

#[test]
fn workbook_window_and_code_name() {
    let wb = read_workbook_stream(&workbook_stream()).unwrap();
    assert_eq!(wb.code_name.as_deref(), Some("MyBook"));
    let v = wb.view.unwrap();
    assert_eq!(
        (v.x_window, v.y_window, v.window_width, v.window_height),
        (240, 120, 16000, 9500)
    );
    assert_eq!((v.tab_ratio, v.first_sheet, v.active_tab), (723, 1, 0));
    assert!(v.show_horizontal_scroll && v.show_vertical_scroll && v.show_sheet_tabs);
    // 시트의 CODENAME 은 시트 것으로 남는다
    assert_eq!(wb.sheets[0].code_name.as_deref(), Some("Sheet1"));
}

#[test]
fn views_panes_selection_and_print_settings() {
    let wb = read_workbook_stream(&workbook_stream()).unwrap();
    let s = &wb.sheets[0];
    let v = &s.view;
    assert!(v.tab_selected && v.show_row_col_headers && s.show_grid_lines);
    assert_eq!(v.zoom_scale, Some(85));
    let p = v.pane.as_ref().unwrap();
    assert_eq!((p.x_split, p.y_split), (1.0, 2.0));
    assert_eq!(p.top_left_cell.as_deref(), Some("B3"));
    assert_eq!(p.active_pane.as_deref(), Some("bottomRight"));
    assert_eq!(p.state.as_deref(), Some("frozen"));
    // 활성 창(0)의 SELECTION 을 쓴다
    assert_eq!(v.active_cell.as_deref(), Some("C4"));
    assert_eq!(v.sqref.as_deref(), Some("C4:D5"));

    let pg = &s.page;
    assert_eq!(pg.fit_to_page, Some(true));
    let o = pg.print_options.as_ref().unwrap();
    assert!(o.grid_lines && o.horizontal_centered && !o.headings);
    let m = pg.margins.as_ref().unwrap();
    assert_eq!((m.left, m.header), (0.5, 0.3f32 as f64)); // HWPUNIT 로 자른 뒤 f32
    let su = pg.setup.as_ref().unwrap();
    assert_eq!(su.paper_size, Some(9));
    assert_eq!(su.scale, Some(85));
    assert_eq!((su.fit_to_width, su.fit_to_height), (Some(2), Some(3)));
    assert_eq!(su.orientation.as_deref(), Some("landscape"));
    assert!(su.use_first_page_number);
    assert_eq!(su.copies, Some(2));
    let h = pg.header_footer.as_ref().unwrap();
    assert_eq!(h.odd_header.as_deref(), Some("&C&\"굴림,Regular\"&P"));
    assert_eq!(
        (pg.row_breaks.clone(), pg.col_breaks.clone()),
        (vec![10], vec![3])
    );
    assert_eq!(s.code_name.as_deref(), Some("Sheet1"));
    assert_eq!(
        s.tab_color.as_ref().and_then(|c| c.rgb.as_deref()),
        Some("ff00b050")
    );
}

#[test]
fn row_repeat_copies_cells_and_shifts_relative_references() {
    let wb = read_workbook_stream(&workbook_stream()).unwrap();
    let s = &wb.sheets[0];
    let at = |a1: &str| s.cell(CellRef::parse(a1).unwrap());
    for r in 3..=5 {
        let a = at(&format!("A{r}")).unwrap();
        assert!(matches!(a.value, CellValue::Number(v) if v == 1.0));
        let b = at(&format!("B{r}")).unwrap();
        assert_eq!(b.formula.as_ref().unwrap().text, format!("A{r}"));
    }
    assert!(at("A6").is_none());
    let rows: Vec<_> = s.rows.iter().map(|r| (r.index, r.height)).collect();
    assert_eq!(rows, [(3, Some(15.0)), (4, Some(15.0)), (5, Some(15.0))]);
}

#[test]
fn wide_colinfo_and_column_styles() {
    let wb = read_workbook_stream(&workbook_stream()).unwrap();
    let cols: Vec<_> = wb.sheets[0]
        .cols
        .iter()
        .map(|c| (c.min, c.max, c.width, c.style.is_some()))
        .collect();
    assert_eq!(
        cols,
        [(1, 2, Some(9.0), true), (257, 16384, Some(9.140625), true)]
    );
}

#[test]
fn external_links_are_numbered_quoted_and_pruned() {
    let wb = read_workbook_stream(&workbook_stream()).unwrap();
    // 쓰이지 않는 Unused.xls 는 버리고 Book One.xls 가 [1] 이 된다 (한셀과 같다)
    assert_eq!(wb.external_links.len(), 1);
    assert_eq!(wb.external_links[0].target, "/Dir/Book%20One.xls");
    assert_eq!(wb.external_links[0].sheet_names, ["Data"]);
    assert_eq!(wb.external_links[0].file_name(), "Book One.xls");
    let link = wb.defined_names.iter().find(|d| d.name == "Link").unwrap();
    // 파일 이름에 공백이 있으므로 시트 이름에 없어도 따옴표로 감싼다
    assert_eq!(link.formula, "'[1]Data'!$B$4");
}

/// 개요·자동 필터·보호만 담은 작은 스트림 (한셀이 쓴 배치 그대로, docs/format/cell-legacy.md 6.4절)
fn filter_stream() -> Vec<u8> {
    let mut b = Biff::default();
    b.rec(0x0809, &bof(0x0005));
    let mut bs = vec![0, 0, 0, 0, 0, 0, 5, 0];
    bs.extend(utf16("Sheet1"));
    let bs_at = b.rec(0x0085, &bs);
    b.rec(0x01AE, &[1, 0, 0x01, 0x04]);
    b.rec(0x0017, &[1, 0, 0, 0, 0, 0, 0, 0]);
    // 통합 문서 보호: 구조 잠금 + 암호 해시
    b.rec(0x0012, &[1, 0]);
    b.rec(0x0019, &[0, 0]);
    b.rec(0x0013, &[0x1a, 0xcc]);
    // NAME _xlnm._FilterDatabase (내장 0x0D, 숨김, 시트 1 범위) = PtgArea3d(XTI 0, $A$1:$E$12)
    let mut rgce = vec![0x3B, 0, 0];
    rgce.extend_from_slice(&0u32.to_le_bytes());
    rgce.extend_from_slice(&11u32.to_le_bytes());
    rgce.extend_from_slice(&[0, 0, 4, 0]);
    let mut name = vec![0x21, 0, 0, 1];
    name.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    name.extend_from_slice(&[0, 0, 1, 0, 0, 0, 0, 0, 0]);
    name.extend(utf16("\u{0D}"));
    name.extend_from_slice(&rgce);
    b.rec(0x0018, &name);
    b.rec(0x000A, &[]);

    let sheet_at = b.rec(0x0809, &bof(0x0010)) as u32;
    // WSBOOL: 요약 행 위(0x40 꺼짐), 요약 열 오른쪽(0x80)
    b.rec(0x0081, &[0x81, 0x04]);
    // 시트 보호: PROTECT, OBJPROTECT, PASSWORD, FEATHDR(isf 2, 서식·행 삽입 허용)
    b.rec(0x0012, &[1, 0]);
    b.rec(0x0063, &[1, 0]);
    b.rec(0x0013, &[0x1a, 0xcc]);
    let mut feat = vec![
        0x67, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0xff, 0xff, 0xff, 0xff,
    ];
    feat.extend_from_slice(&0x4044u32.to_le_bytes());
    b.rec(0x0867, &feat);
    // COLINFO B: 수준 1 숨김, C: 접힘
    b.rec(0x007D, &[1, 0, 1, 0, 0x24, 0x09, 15, 0, 0x01, 0x01, 0, 0]);
    b.rec(0x007D, &[2, 0, 2, 0, 0x24, 0x09, 15, 0, 0x00, 0x10, 0, 0]);
    // ROW 3: 수준 2, ROW 5: 접힘
    for (rw, flags) in [(2u32, 0x02u8), (4, 0x10)] {
        let mut row = rw.to_le_bytes().to_vec();
        row.extend_from_slice(&[0, 0, 5, 0, 0x48, 0x01, 0, 0, 0, 0, flags, 0, 0x0f, 0]);
        b.rec(0x0208, &row);
    }
    b.rec(0x009B, &[]);
    b.rec(0x009D, &[5, 0]);
    // A열: 문자열 "7" 또는 "9" (값 두 개는 DOPER 에도 남는다) + 한컴 값 목록 + 빈 칸
    let mut af = vec![0, 0, 0x0d, 0];
    for _ in 0..2 {
        af.extend_from_slice(&[0x06, 0x02, 0, 0, 0, 0, 1, 0, 0, 0]);
    }
    af.extend_from_slice(&[0, 0x37, 0, 0, 0x39, 0]);
    b.rec(0x009E, &af);
    b.rec(0x009F, &[0, 0, 0, 1, 0, 0, 0x37, 0]);
    b.rec(0x009F, &[0, 0, 0, 1, 0, 0, 0x39, 0]);
    b.rec(0x009F, &[0, 0, 0x0c]);
    // C열: 하위 20% (fTopN, fPercent, wTopN 20), 기준값 RK -3
    let mut af = vec![2, 0, 0x50, 0x0a, 0x02, 0x03, 0, 0, 0x08, 0xc0, 0, 0, 0, 0];
    af.extend_from_slice(&[0; 10]);
    b.rec(0x009E, &af);
    // D열: 동적 필터 today (AUTOFILTER12 cft 9)
    let mut af = vec![3, 0, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0];
    af.extend_from_slice(&[0; 10]);
    b.rec(0x009E, &af);
    let mut af12 = vec![0x7e, 0x08, 1, 0, 0, 0, 11, 0, 0, 0, 4, 0, 3, 0];
    for v in [0u32, 0, 9, 1, 0] {
        af12.extend_from_slice(&v.to_le_bytes());
    }
    af12.extend_from_slice(&[0, 0, 8, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
    af12.extend_from_slice(&[0; 16]);
    b.rec(0x087E, &af12);
    // ContinueFrt12: 기간 시작(>=) 46314, 끝(<) 46315 — 숫자 DOPER 둘
    for (sign, v) in [(6u8, 46314.0f64), (1, 46315.0)] {
        let mut c = vec![0x7f, 0x08, 1, 0, 0, 0, 11, 0, 0, 0, 4, 0, 0x04, sign];
        c.extend_from_slice(&v.to_le_bytes());
        b.rec(0x087F, &c);
    }
    // E열: > 20 그리고 < 100 (RK)
    b.rec(
        0x009E,
        &[
            4, 0, 0, 0, 0x02, 0x04, 0, 0, 0x34, 0x40, 0, 0, 0, 0, 0x02, 0x01, 0, 0, 0x59, 0x40, 0,
            0, 0, 0,
        ],
    );
    b.rec(0x000A, &[]);

    b.0[bs_at + 4..bs_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    b.0
}

#[test]
fn outline_auto_filter_and_protection() {
    let wb = read_workbook_stream(&filter_stream()).unwrap();
    let p = wb.protection.as_ref().unwrap();
    assert!(p.lock_structure && !p.lock_windows);
    assert_eq!(p.password.as_deref(), Some("CC1A"));

    let s = &wb.sheets[0];
    assert!(s.summary_above && !s.summary_left && s.filter_mode);
    let cols: Vec<_> = s
        .cols
        .iter()
        .map(|c| (c.min, c.outline_level, c.hidden, c.collapsed))
        .collect();
    assert_eq!(cols, [(2, 1, true, false), (3, 0, false, true)]);
    let rows: Vec<_> = s
        .rows
        .iter()
        .map(|r| (r.index, r.outline_level, r.collapsed))
        .collect();
    assert_eq!(rows, [(3, 2, false), (5, 0, true)]);

    let sp = s.protection.as_ref().unwrap();
    assert_eq!(sp.password.as_deref(), Some("CC1A"));
    for (k, on) in [
        ("sheet", true),
        ("objects", true),
        ("scenarios", true),
        ("formatCells", false),
        ("insertRows", false),
        ("formatRows", true),
        ("selectLockedCells", true),
        ("selectUnlockedCells", false),
    ] {
        assert_eq!(sp.flag(k), on, "{k}");
    }

    let af = s.auto_filter.as_ref().unwrap();
    assert_eq!(af.range, "A1:E12");
    let c = &af.columns;
    assert_eq!(c.len(), 4);
    assert_eq!(
        (c[0].values.clone(), c[0].blank),
        (vec!["7".to_string(), "9".into()], true)
    );
    assert!(c[0].custom.is_empty());
    assert_eq!(c[1].top10, Some((false, true, 20.0)));
    assert_eq!(c[1].filter_val, Some(-3.0));
    assert_eq!(c[2].dynamic.as_deref(), Some("today"));
    assert_eq!(
        (c[2].dynamic_val, c[2].dynamic_max_val),
        (Some(46314.0), Some(46315.0))
    );
    assert!(c[2].custom.is_empty());
    assert_eq!(
        c[3].custom,
        [
            ("greaterThan".to_string(), "20".to_string()),
            ("lessThan".into(), "100".into())
        ]
    );
    assert!(c[3].custom_and);
}
