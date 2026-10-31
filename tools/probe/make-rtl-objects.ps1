# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 오른쪽에서 왼쪽 시트의 개체 실험: 같은 내용(도형·메모·데이터 막대·아이콘 집합·차트)을 보통 시트와 오른쪽에서 왼쪽 시트에 넣어 저장한다.
# 갈무리(screen-capture.ps1, 13번째 열이 오른쪽에서 왼쪽 여부)와 visual-diff.ps1 로 비교한다. 스파크라인·조건부 서식은 COM 으로 만들 수 없어
# 기존 문서의 시트 XML 에 rightToLeft="1" 을 넣은 사본을 `rcell patch` 로 만들어 쓴다.
param([Parameter(Mandatory = $true)][string]$OutDir)
# RTL sheet objects: the same content (shapes, visible note, sparkline, data bar, icon set, chart, picture-less) in a left-to-right and a right-to-left sheet
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $OutDir | Out-Null
function T([string]$what, [scriptblock]$blk) { try { & $blk | Out-Null } catch { Write-Host "  (fail) $what : $($_.Exception.Message)" } }
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    $wb = $app.Workbooks.Add()
    while ($wb.Worksheets.Count -lt 2) { $last = $wb.Worksheets.Item($wb.Worksheets.Count); $wb.Worksheets.Add([Type]::Missing, $last) | Out-Null }
    for ($k = 1; $k -le 2; $k++) {
        $ws = $wb.Worksheets.Item($k)
        $ws.Name = @("ltr", "rtl")[$k - 1]
        for ($c = 1; $c -le 10; $c++) { $ws.Columns.Item($c).ColumnWidth = 12 }
        for ($c = 1; $c -le 4; $c++) { $ws.Cells.Item(5, $c).Value2 = $c * 3 + 2 }
        for ($r = 8; $r -le 12; $r++) { $ws.Cells.Item($r, 1).Value2 = ($r - 7) * 10 }
        for ($r = 8; $r -le 12; $r++) { $ws.Cells.Item($r, 3).Value2 = ($r - 7) * 10 }
        $ws.Cells.Item(2, 1).Value2 = "텍스트 A2"
        $ws.Cells.Item(2, 3).Value2 = "메모 있는 칸"
        # shapes: rectangle with text, ellipse, an arrow line
        $s1 = $ws.Shapes.AddShape(1, 330, 20, 130, 50); T "t" { $s1.TextFrame.Characters().Text = "사각형 글" }
        $s2 = $ws.Shapes.AddShape(9, 60, 130, 90, 60); T "t" { $s2.TextFrame.Characters().Text = "타원" }
        $s3 = $ws.Shapes.AddShape(33, 200, 150, 120, 40)
        # visible comment
        T "comment" { $cm = $ws.Cells.Item(2, 3).AddComment("메모 글"); $cm.Visible = $true }
        # sparkline
        T "spark" { $ws.Range("E5").SparklineGroups.Add(1, "A5:D5") | Out-Null }
        # data bar and icon set
        T "databar" { $ws.Range("A8:A12").FormatConditions.AddDatabar() | Out-Null }
        T "iconset" { $ws.Range("C8:C12").FormatConditions.AddIconSetCondition() | Out-Null }
        # chart
        T "chart" {
            $co = $ws.ChartObjects().Add(420, 90, 260, 160)
            $co.Chart.ChartType = 51
            $co.Chart.SetSourceData($ws.Range("A8:A12"))
        }
    }
    T "rtl" { $wb.Worksheets.Item(2).DisplayRightToLeft = $true }
    $wb.Worksheets.Item(1).Activate() | Out-Null
    $p = Join-Path $OutDir "rtlobj.cell"
    if (Test-Path $p) { Remove-Item $p -Force }
    $wb.SaveAs($p, -2006) | Out-Null
    Write-Host "saved rtlobj.cell"
    $wb.Close($false)
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
