# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 표시 형식 정답 생성기: 설치된 한셀에 값과 서식 코드를 넣고, 셀에 보이는 문자열
# (Range.Text)을 TSV 로 저장한다. rcell 의 표시 형식 엔진을 이 결과와 대조한다.
# 결과 파일은 로컬에서만 쓴다 (RCELL_NUMFMT_ORACLE 환경 변수로 테스트에 전달).
#
# 사용: powershell -File numfmt-oracle.ps1 -Out C:\rcell-corpus\numfmt.tsv
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param([Parameter(Mandatory = $true)][string]$Out)
$ErrorActionPreference = "Stop"

$formats = @(
    "General", "0", "0.0", "0.00", "#,##0", "#,##0.00", "0%", "0.00%", "0.00E+00", "##0.0E+0",
    "# ?/?", "# ??/??", "?/?", "@", "`"₩`"#,##0", "\₩#,##0", "#,##0;[Red]-#,##0", "#,##0_);(#,##0)",
    "0.0_);(0.0)", "[>=100]`"큰`";`"작은`"", "yyyy-mm-dd", "yyyy`"년`" m`"월`" d`"일`"", "mm/dd/yy",
    "d-mmm-yy", "mmm-yy", "h:mm", "h:mm:ss", "h:mm AM/PM", "AM/PM h:mm", "[h]:mm:ss", "mm:ss.0",
    "yyyy-mm-dd hh:mm:ss", "ddd", "dddd", "mmmm", "[DBNum1]G/표준", "[DBNum2]G/표준", "[DBNum3]G/표준",
    "[$-412]yyyy`"년`" m`"월`" d`"일`" dddd", "0`"원`"", "#,##0`"개`"", ";;;", "0.00;-0.00;`"제로`"",
    "000000", "#.##", ".00", "`"합계: `"@", "@`" 님`"", "0.000", "#,##0.0,", "yy.m.d", "aaa", "aaaa"
)
$values = @(0.0, 1.0, -1.0, 1234.5678, -1234.5678, 0.5, 0.001234, 1234567.891, 46314.75, 1e10, -0.5, 100.0, 99.999)

$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    $wb = $app.Workbooks.Add()
    $ws = $wb.Worksheets.Item(1)
    for ($c = 1; $c -le $values.Count + 1; $c++) { $ws.Columns.Item($c).ColumnWidth = 60 }
    $lines = New-Object System.Collections.Generic.List[string]
    $r = 1
    foreach ($fm in $formats) {
        $c = 1
        foreach ($v in $values) {
            $cell = $ws.Cells.Item($r, $c)
            $cell.Value2 = [double]$v
            try { $cell.NumberFormat = $fm } catch {}
            $c++
        }
        $tc = $ws.Cells.Item($r, $c)
        $tc.Value2 = [string]"abc"
        try { $tc.NumberFormat = $fm } catch {}
        $r++
    }
    for ($r = 1; $r -le $formats.Count; $r++) {
        for ($c = 1; $c -le $values.Count + 1; $c++) {
            $cell = $ws.Cells.Item($r, $c)
            $lines.Add(("{0}`t{1}" -f $cell.Address($false, $false), $cell.Text))
        }
    }
    [IO.File]::WriteAllLines("$Out.tsv", $lines, (New-Object Text.UTF8Encoding $false))
    $wb.SaveAs([IO.Path]::GetFullPath("$Out.cell"), -2006)
    "wrote $($lines.Count) rows to $Out.tsv and $Out.cell"
    $wb.Close($false)
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
