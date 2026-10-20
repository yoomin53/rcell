# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 차분 분석용 샘플 묶음 생성기.
# 설치된 한셀(COM 자동화 HCell.Application)로 기능별 문서를 만들고, 같은 내용을
# 세 가지 .cell 형식으로 저장한다. rcell 은 XML 형식 결과를 기준으로 나머지 형식의
# 파서를 검증한다. 생성된 파일은 개인 환경에서만 쓰고 저장소에는 넣지 않는다.
#
#   <이름>.cell         xlCell9  (-2006)  OOXML(XML) — 한셀 2014 이후 기본
#   <이름>.bin.cell     xlCellb  (-2007)  OOXML 바이너리 레코드
#   <이름>.legacy.cell  xlCell8  (-2005)  OLE2 + BIFF 계열 레코드 (레거시)
#
# 사용: powershell -File make-corpus.ps1 -OutDir C:\rcell-corpus
# 주의: 이 파일은 UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param([Parameter(Mandatory = $true)][string]$OutDir)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $OutDir | Out-Null
$Formats = [ordered]@{ ".cell" = -2006; ".bin.cell" = -2007; ".legacy.cell" = -2005 }

function Try-Do([string]$what, [scriptblock]$action) {
    try { & $action } catch { Write-Host "  (실패) $what : $($_.Exception.Message)" }
}

function Save-All($wb, [string]$name) {
    foreach ($ext in $Formats.Keys) {
        $path = Join-Path $OutDir ($name + $ext)
        if (Test-Path $path) { Remove-Item $path -Force }
        Try-Do "SaveAs $name$ext" { $wb.SaveAs($path, $Formats[$ext]); Write-Host "  저장 $name$ext" }
    }
}

$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}

    # ------------------------------------------------------------------ values
    Write-Host "values"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    $vals = @("가나다", "Hello, World", "줄`n바꿈", "따옴표 `"인용`"", "", 0, 1, -1, 3.14159265358979,
        -0.000123, 1e15, 123456789012, 0.1, 99.99, 1e-10)
    $r = 1
    # PowerShell COM 호출 지점은 처음 넣은 값의 타입을 기억하므로 타입별로 나눈다.
    foreach ($v in $vals) {
        $cell = $ws.Cells.Item($r, 1)
        if ($v -is [string]) { $cell.Value2 = [string]$v } else { $cell.Value2 = [double]$v }
        $r++
    }
    $ws.Cells.Item(1, 2).Value2 = $true
    $ws.Cells.Item(2, 2).Value2 = $false
    $ws.Cells.Item(3, 2).Formula = "=1/0"
    $ws.Cells.Item(4, 2).Formula = "=NA()"
    $ws.Cells.Item(5, 2).Formula = "=`"a`"+1"
    $ws.Cells.Item(6, 2).Formula = "=SQRT(-1)"
    $ws.Cells.Item(7, 2).Formula = "=XYZ"
    Try-Do "date" { $ws.Cells.Item(8, 2).Formula = "=DATE(2026,10,19)" }
    $ws.Cells.Item(9, 2).Value2 = "'0123"
    Save-All $wb "values"; $wb.Close($false)

    # ---------------------------------------------------------------- formulas
    Write-Host "formulas"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    for ($i = 1; $i -le 5; $i++) { $ws.Cells.Item($i, 1).Value2 = [double]($i * 10); $ws.Cells.Item($i, 2).Value2 = "항목$i" }
    $f = @(
        "=A1+A2*A3-A4/A5", "=SUM(A1:A5)", "=AVERAGE(A1:A5)", "=MAX(A1:A5)-MIN(A1:A5)", "=COUNT(A1:A5)",
        "=IF(A1>5,`"크다`",`"작다`")", "=ROUND(A3/7,2)", "=`$A`$1+A`$2+`$A3", "=VLOOKUP(30,A1:B5,2,FALSE)",
        "=CONCATENATE(B1,`"-`",B2)", "=B1&B2", "=A1^2", "=-A1", "=A1%", "=(A1+A2)*A3", "=SUMIF(A1:A5,`">20`")",
        "=COUNTIF(A1:A5,`">=30`")", "=AND(A1>0,A2>0)", "=OR(A1<0,A2>0)", "=NOT(TRUE)", "=LEN(B1)", "=LEFT(B1,1)",
        "=TODAY()-TODAY()", "=INDEX(A1:A5,3)", "=MATCH(40,A1:A5,0)", "=ABS(-5)", "=MOD(17,5)", "=INT(7.8)",
        "=PI()", "=A1<>A2", "=A1>=A2", "=A1<=A2", "=A1=A2", "=SUM(A1,A2,A3)", "=IFERROR(1/0,0)", "=TEXT(A1,`"0.00`")",
        "=Sheet2!A1*2", "=SUM(A:A)", "=SUM(1:1)", "=`"문자`"", "=TRUE", "=1.5E+20", "=SUMPRODUCT(A1:A2,A3:A4)"
    )
    $ws2 = $wb.Worksheets.Add([Type]::Missing, $ws); $ws2.Name = "Sheet2"; $ws2.Cells.Item(1, 1).Value2 = [double]7
    $ws.Activate() | Out-Null
    $r = 1
    foreach ($x in $f) {
        Try-Do "formula $x" { $ws.Cells.Item($r, 4).Formula = $x }
        Try-Do "label $x" { $ws.Cells.Item($r, 5).Value2 = "'" + $x }
        $r++
    }
    Save-All $wb "formulas"; $wb.Close($false)

    # ------------------------------------------------------------------- fonts
    Write-Host "fonts"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    $cases = @(
        @{ l = "bold"; s = { param($f) $f.Bold = $true } },
        @{ l = "italic"; s = { param($f) $f.Italic = $true } },
        @{ l = "u-single"; s = { param($f) $f.Underline = 2 } },
        @{ l = "u-double"; s = { param($f) $f.Underline = -4119 } },
        @{ l = "strike"; s = { param($f) $f.Strikethrough = $true } },
        @{ l = "super"; s = { param($f) $f.Superscript = $true } },
        @{ l = "sub"; s = { param($f) $f.Subscript = $true } },
        @{ l = "size20"; s = { param($f) $f.Size = 20 } },
        @{ l = "size7.5"; s = { param($f) $f.Size = 7.5 } },
        @{ l = "red"; s = { param($f) $f.Color = 255 } },
        @{ l = "rgb-123456"; s = { param($f) $f.Color = 0x563412 } },
        @{ l = "batang"; s = { param($f) $f.Name = "바탕" } },
        @{ l = "arial"; s = { param($f) $f.Name = "Arial" } },
        @{ l = "shadow"; s = { param($f) $f.Shadow = $true } },
        @{ l = "outline"; s = { param($f) $f.OutlineFont = $true } },
        @{ l = "bold-italic-red"; s = { param($f) $f.Bold = $true; $f.Italic = $true; $f.Color = 255 } }
    )
    $r = 1
    foreach ($c in $cases) { $cell = $ws.Cells.Item($r, 1); $cell.Value2 = $c.l; Try-Do $c.l { & $c.s $cell.Font }; $r++ }
    Save-All $wb "fonts"; $wb.Close($false)

    # ------------------------------------------------------------ fills/borders
    Write-Host "fills_borders"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    $ws.Cells.Item(1, 1).Value2 = "yellow"; Try-Do "fill" { $ws.Cells.Item(1, 1).Interior.Color = 0x00FFFF }
    $ws.Cells.Item(2, 1).Value2 = "pattern"; Try-Do "pattern" { $ws.Cells.Item(2, 1).Interior.Pattern = 9; $ws.Cells.Item(2, 1).Interior.PatternColor = 255 }
    $ws.Cells.Item(3, 1).Value2 = "theme"; Try-Do "theme" { $ws.Cells.Item(3, 1).Interior.ThemeColor = 5 }
    $styles = @(1, -4115, 4, 5, -4119, -4118, 13)  # 실선, 파선, 일점쇄선, 이점쇄선, 이중, 점선, 사선 일점쇄선
    $r = 5
    foreach ($s in $styles) {
        $cell = $ws.Cells.Item($r, 2); $cell.Value2 = "border$s"
        Try-Do "border $s" { $cell.Borders.Item(9).LineStyle = $s }       # xlEdgeBottom
        Try-Do "border top thick" { $cell.Borders.Item(8).Weight = -4138 } # xlEdgeTop medium
        $r += 2
    }
    $c = $ws.Cells.Item(20, 2); $c.Value2 = "box-red"
    Try-Do "box" { $c.BorderAround(1, -4138, 3) | Out-Null }
    $c = $ws.Cells.Item(22, 2); $c.Value2 = "diag"
    Try-Do "diag" { $c.Borders.Item(5).LineStyle = 1; $c.Borders.Item(6).LineStyle = 1 }
    Save-All $wb "fills_borders"; $wb.Close($false)

    # ----------------------------------------------------------- align/numfmt
    Write-Host "align_numfmt"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    $fmts = @("0", "0.00", "#,##0", "#,##0.00", "0%", "0.00%", "0.00E+00", "yyyy-mm-dd", "yyyy`"년`" m`"월`" d`"일`"",
        "h:mm:ss", "[h]:mm", "@", "₩#,##0", "#,##0;[Red]-#,##0", "0.0_);(0.0)", "# ?/?", "[DBNum1]G/표준", "mm/dd/yy",
        "AM/PM h:mm", "ddd", "General")
    $r = 1
    foreach ($fm in $fmts) { $cell = $ws.Cells.Item($r, 1); $cell.Value2 = 1234.5678; Try-Do "fmt $fm" { $cell.NumberFormat = $fm }; $ws.Cells.Item($r, 2).Value2 = $fm; $r++ }
    $al = @(
        @{ l = "left"; s = { param($c) $c.HorizontalAlignment = -4131 } },
        @{ l = "center"; s = { param($c) $c.HorizontalAlignment = -4108 } },
        @{ l = "right"; s = { param($c) $c.HorizontalAlignment = -4152 } },
        @{ l = "top"; s = { param($c) $c.VerticalAlignment = -4160 } },
        @{ l = "bottom"; s = { param($c) $c.VerticalAlignment = -4107 } },
        @{ l = "wrap"; s = { param($c) $c.WrapText = $true } },
        @{ l = "rot90"; s = { param($c) $c.Orientation = 90 } },
        @{ l = "indent3"; s = { param($c) $c.IndentLevel = 3 } },
        @{ l = "shrink"; s = { param($c) $c.ShrinkToFit = $true } },
        @{ l = "locked-off"; s = { param($c) $c.Locked = $false } },
        @{ l = "hidden-formula"; s = { param($c) $c.FormulaHidden = $true } }
    )
    $r = 1
    foreach ($a in $al) { $cell = $ws.Cells.Item($r, 4); $cell.Value2 = $a.l; Try-Do $a.l { & $a.s $cell }; $r++ }
    Save-All $wb "align_numfmt"; $wb.Close($false)

    # --------------------------------------------------------------- structure
    Write-Host "structure"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1); $ws.Name = "첫 시트"
    $ws.Cells.Item(1, 1).Value2 = "병합"; Try-Do "merge" { $ws.Range("A1:C2").Merge() | Out-Null }
    Try-Do "colwidth" { $ws.Columns.Item(2).ColumnWidth = 20; $ws.Columns.Item(4).ColumnWidth = 3.5 }
    Try-Do "rowheight" { $ws.Rows.Item(5).RowHeight = 40 }
    Try-Do "hide col" { $ws.Columns.Item(6).Hidden = $true }
    Try-Do "hide row" { $ws.Rows.Item(7).Hidden = $true }
    $ws.Cells.Item(5, 1).Value2 = "row5"; $ws.Cells.Item(7, 1).Value2 = "row7(hidden)"; $ws.Cells.Item(1, 6).Value2 = "colF(hidden)"
    Try-Do "name" { $wb.Names.Add("가격", "='첫 시트'!`$A`$5") | Out-Null }
    Try-Do "freeze" { $ws.Activate() | Out-Null; $app.ActiveWindow.FreezePanes = $false; $ws.Cells.Item(2, 2).Select() | Out-Null; $app.ActiveWindow.FreezePanes = $true }
    Try-Do "hyperlink" { $ws.Hyperlinks.Add($ws.Cells.Item(9, 1), "https://github.com/yoomin53/rcell", "", "rcell", "rcell 저장소") | Out-Null }
    Try-Do "comment" { $ws.Cells.Item(10, 1).AddComment("메모 내용") | Out-Null }
    $s2 = $wb.Worksheets.Add([Type]::Missing, $ws); $s2.Name = "둘째"; $s2.Cells.Item(1, 1).Value2 = [double]2
    $s3 = $wb.Worksheets.Add([Type]::Missing, $s2); $s3.Name = "숨김"; $s3.Cells.Item(1, 1).Value2 = [double]3
    Try-Do "hide sheet" { $s3.Visible = 0 }
    Try-Do "tab color" { $s2.Tab.Color = 255 }
    $ws.Activate() | Out-Null
    Save-All $wb "structure"; $wb.Close($false)

    # ------------------------------------------------------------------ bigref
    Write-Host "bigref"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    $ws.Cells.Item(1, 1).Value2 = "start"
    Try-Do "row 70000" { $ws.Cells.Item(70000, 3).Value2 = [double]70000 }
    Try-Do "col 300" { $ws.Cells.Item(2, 300).Value2 = "col300" }
    Try-Do "row 1048576" { $ws.Cells.Item(1048576, 1).Value2 = "last" }
    Try-Do "ref formula" { $ws.Cells.Item(3, 1).Formula = "=C70000+1" }
    Save-All $wb "bigref"; $wb.Close($false)

    # ----------------------------------------------------------------- strings
    Write-Host "strings"
    $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
    $long = ("가나다라마바사아자차카타파하" * 40)
    $ws.Cells.Item(1, 1).Value2 = $long
    $ws.Cells.Item(2, 1).Value2 = "같은 문자열"; $ws.Cells.Item(3, 1).Value2 = "같은 문자열"
    $ws.Cells.Item(4, 1).Value2 = "emoji 😀 漢字 ひらがな"
    Try-Do "rich" { $c = $ws.Cells.Item(5, 1); $c.Value2 = "굵게보통"; $c.Characters(1, 2).Font.Bold = $true }
    for ($i = 6; $i -le 400; $i++) { $ws.Cells.Item($i, 1).Value2 = "문자열 $i" }
    Save-All $wb "strings"; $wb.Close($false)
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
