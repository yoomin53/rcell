// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 시트·통합 문서 보호 걸기와 풀기 (한셀 COM `Worksheet.Protect`·`Unprotect`·`Workbook.Protect`·`Unprotect` 대조 실험 —
//! docs/editing.md 21절).
//!
//! - 암호는 16비트 레거시 해시(소문자 16진 4자리)로 둔다. 글자는 코드 페이지 949 바이트로 본다 (`abc` → `cc1a`, `한글` → `c4bd`).
//! - 시트 보호: 내용(`sheet`)·개체(`objects`)·시나리오(`scenarios`)를 막고(기본 모두 막음) 서식·삽입·삭제·정렬·필터·피벗은 막되
//!   `allow…` 로 허용한 것만 푼다. 잠긴 칸·잠기지 않은 칸 선택은 막지 않는다.
//! - 풀 때 암호가 틀리면 오류이고 보호는 그대로. 암호가 없는 보호는 암호 없이도 풀린다. 암호가 있는 보호가 걸린 시트에 다시 걸면 오류
//!   (암호 없는 보호는 바뀐다). 통합 문서는 구조가 이미 보호돼 있으면 오류이고, 걸려 있던 창 보호는 그대로 둔다.
//! - 통합 문서 보호는 구조(시트 추가·지우기·옮기기)만 저장한다 — 창 보호(`windows`)는 한셀이 저장하지 않는다.

use super::err;
use crate::model::{SheetProtection, Workbook, WorkbookProtection};
use serde::Deserialize;

/// 16비트 레거시 암호 해시 (소문자 16진 4자리)
pub fn legacy_hash(password: &str) -> String {
    let bytes = crate::calc::cp949::encode_lossy(password);
    let mut hash: u32 = 0;
    for &b in bytes.iter().rev() {
        hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7fff);
        hash ^= b as u32;
    }
    hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7fff);
    hash ^= bytes.len() as u32;
    hash ^= 0xCE4B;
    format!("{hash:04x}")
}

/// 시트 보호 설정 (`Worksheet.Protect` 인자)
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProtectOptions {
    #[serde(default)]
    pub password: Option<String>,
    /// 개체·내용·시나리오를 막는가 (기본 참)
    #[serde(default)]
    pub drawing_objects: Option<bool>,
    #[serde(default)]
    pub contents: Option<bool>,
    #[serde(default)]
    pub scenarios: Option<bool>,
    /// 허용하는 동작 (기본 모두 막음)
    #[serde(default)]
    pub formatting_cells: bool,
    #[serde(default)]
    pub formatting_columns: bool,
    #[serde(default)]
    pub formatting_rows: bool,
    #[serde(default)]
    pub inserting_columns: bool,
    #[serde(default)]
    pub inserting_rows: bool,
    #[serde(default)]
    pub inserting_hyperlinks: bool,
    #[serde(default)]
    pub deleting_columns: bool,
    #[serde(default)]
    pub deleting_rows: bool,
    #[serde(default)]
    pub sorting: bool,
    #[serde(default)]
    pub filtering: bool,
    #[serde(default)]
    pub using_pivot_tables: bool,
}

fn hashed(password: &Option<String>) -> Option<String> {
    password
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(legacy_hash)
}

/// 시트 보호 걸기 (암호가 있는 보호가 이미 걸린 시트는 오류 — 암호 없는 보호는 새 보호로 바뀐다)
pub fn protect_sheet(wb: &mut Workbook, si: usize, o: &ProtectOptions) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    if sheet
        .protection
        .as_ref()
        .is_some_and(|p| p.flag("sheet") && p.password.is_some())
    {
        return Err(err("이미 보호된 시트입니다"));
    }
    // SHEET_PROTECTION_FLAGS 차례: 참 = 막힘
    let states = [
        o.contents.unwrap_or(true),
        o.drawing_objects.unwrap_or(true),
        o.scenarios.unwrap_or(true),
        !o.formatting_cells,
        !o.formatting_columns,
        !o.formatting_rows,
        !o.inserting_columns,
        !o.inserting_rows,
        !o.inserting_hyperlinks,
        !o.deleting_columns,
        !o.deleting_rows,
        false,
        !o.sorting,
        !o.filtering,
        !o.using_pivot_tables,
        false,
    ];
    sheet.protection = Some(SheetProtection::from_states(hashed(&o.password), states));
    Ok(())
}

/// 시트 보호 풀기: 암호가 있으면 맞아야 한다
pub fn unprotect_sheet(
    wb: &mut Workbook,
    si: usize,
    password: &Option<String>,
) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    if let Some(p) = &sheet.protection
        && let Some(h) = &p.password
        && !h.eq_ignore_ascii_case(&hashed(password).unwrap_or_default())
    {
        return Err(err("암호가 잘못되었습니다"));
    }
    sheet.protection = None;
    Ok(())
}

/// 통합 문서 구조 보호 걸기
pub fn protect_workbook(
    wb: &mut Workbook,
    password: &Option<String>,
    structure: bool,
) -> crate::Result<()> {
    if wb.protection.as_ref().is_some_and(|p| p.lock_structure) {
        return Err(err("이미 보호된 통합 문서입니다"));
    }
    // 창 보호는 한셀이 걸지 않지만 문서에 이미 걸려 있으면 그대로 둔다
    let windows = wb.protection.as_ref().is_some_and(|p| p.lock_windows);
    wb.protection = (structure || windows).then(|| WorkbookProtection {
        lock_structure: structure,
        lock_windows: windows,
        password: hashed(password),
    });
    Ok(())
}

/// 통합 문서 보호 풀기: 암호가 있으면 맞아야 한다
pub fn unprotect_workbook(wb: &mut Workbook, password: &Option<String>) -> crate::Result<()> {
    if let Some(p) = &wb.protection
        && let Some(h) = &p.password
        && !h.eq_ignore_ascii_case(&hashed(password).unwrap_or_default())
    {
        return Err(err("암호가 잘못되었습니다"));
    }
    wb.protection = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_like_hcell() {
        assert_eq!(legacy_hash("pw"), "cf75");
        assert_eq!(legacy_hash("abc"), "cc1a");
        assert_eq!(legacy_hash("a"), "ce88");
        assert_eq!(legacy_hash("1234567890123456"), "9ec2");
        assert_eq!(legacy_hash("한글"), "c4bd");
    }
}
