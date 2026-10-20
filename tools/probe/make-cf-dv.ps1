# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 조건부 서식·데이터 유효성 차분 샘플: 한셀 COM 으로 규칙을 만들고 세 형식으로 저장한다.
#   cf_dv.cell / cf_dv.bin.cell / cf_dv.legacy.cell (+ 한셀이 레거시를 다시 읽은 cf_dv.legacy.as.cell)
#
# 사용: powershell -File make-cf-dv.ps1 -OutDir C:\rcell-corpus
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param([Parameter(Mandatory = $true)][string]$OutDir)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $OutDir | Out-Null

function Try-Do([string]$what, [scriptblock]$action) {
    try { & $action; Write-Host "  ok   $what" } catch { Write-Host "  (실패) $what : $($_.Exception.Message)" }
}

$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    for ($r = 1; $r -le 12; $r++) {
        $ws.Cells.Item($r, 1).Value2 = [double]($r * 7 % 13)
        $ws.Cells.Item($r, 2).Value2 = "항목 $r"
        $ws.Cells.Item($r, 3).Value2 = [double]($r * 10)
        $ws.Cells.Item($r, 4).Value2 = [double](100 - $r * 5)
        $ws.Cells.Item($r, 5).Value2 = [double]($r - 6)
    }
    # --- 조건부 서식 (xlCellValue=1, xlExpression=2; xlGreater=5, xlBetween=1, xlLess=6, xlEqual=3)
    Try-Do "cellIs >" { $fc = $ws.Range("A1:A12").FormatConditions.Add(1, 5, "=8"); $fc.Interior.Color = 255; $fc.Font.Bold = $true }
    Try-Do "cellIs between" { $fc = $ws.Range("A1:A12").FormatConditions.Add(1, 1, "=2", "=4"); $fc.Interior.Color = 65280 }
    Try-Do "expression" { $fc = $ws.Range("C1:C12").FormatConditions.Add(2, 0, "=MOD(C1,20)=0"); $fc.Font.Color = 16711680 }
    Try-Do "text contains" { $fc = $ws.Range("B1:B12").FormatConditions.Add(9, 0, "1"); $fc.Interior.Color = 49407 }
    Try-Do "top10" { $t = $ws.Range("D1:D12").FormatConditions.AddTop10(); $t.TopBottom = 1; $t.Rank = 3; $t.Interior.Color = 15773696 }
    Try-Do "color scale" { $ws.Range("C1:C12").FormatConditions.AddColorScale(3) | Out-Null }
    Try-Do "data bar" { $ws.Range("D1:D12").FormatConditions.AddDatabar() | Out-Null }
    Try-Do "icon set" { $is = $ws.Range("E1:E12").FormatConditions.AddIconSetCondition(); $is.IconSet = $wb.IconSets.Item(4) }
    Try-Do "above average" { $aa = $ws.Range("C1:C12").FormatConditions.AddAboveAverage(); $aa.AboveBelow = 0; $aa.Font.Italic = $true }
    Try-Do "duplicate" { $u = $ws.Range("A1:A12").FormatConditions.AddUniqueValues(); $u.DupeUnique = 1; $u.Font.Underline = 2 }
    # --- 데이터 유효성 (xlValidateList=3, WholeNumber=1, Decimal=2, Date=4, TextLength=6)
    Try-Do "dv list" { $v = $ws.Range("G1:G5").Validation; $v.Add(3, 1, 1, "사과,배,감"); $v.InputTitle = "과일"; $v.InputMessage = "목록에서 고르세요"; $v.ErrorTitle = "오류"; $v.ErrorMessage = "목록에 없는 값" }
    Try-Do "dv whole between" { $ws.Range("H1:H5").Validation.Add(1, 1, 1, "1", "10") }
    Try-Do "dv decimal >" { $ws.Range("I1:I5").Validation.Add(2, 2, 5, "0.5") }
    Try-Do "dv date" { $ws.Range("J1:J5").Validation.Add(4, 1, 1, "2026-01-01", "2026-12-31") }
    Try-Do "dv textlen <=" { $v = $ws.Range("K1:K5").Validation; $v.Add(6, 3, 8, "5"); $v.IgnoreBlank = $false }
    Try-Do "dv list ref" { $ws.Range("L1:L5").Validation.Add(3, 1, 1, "=`$B`$1:`$B`$5") }
    foreach ($t in @(@(".cell", -2006), @(".bin.cell", -2007), @(".legacy.cell", -2005))) {
        $p = Join-Path $OutDir ("cf_dv" + $t[0])
        if (Test-Path $p) { Remove-Item $p -Force }
        $wb.SaveAs($p, $t[1])
    }
    $wb.Close($false)
    $wb = $app.Workbooks.Open((Join-Path $OutDir "cf_dv.legacy.cell"))
    $p = Join-Path $OutDir "cf_dv.legacy.as.cell"
    if (Test-Path $p) { Remove-Item $p -Force }
    $wb.SaveAs($p, -2006)
    $wb.Close($false)
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
