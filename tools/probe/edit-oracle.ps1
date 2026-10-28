# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 편집 대조 실험: 편집 대본(JSON, `rcell edit` 과 같은 형식 — docs/editing.md)을 한셀 COM 으로 실행해 저장한다.
# 같은 대본을 rcell 로 실행한 결과(`rcell edit`)를 한셀이 다시 저장한 것과 `rcell diff` 로 비교하는 데 쓴다.
#
#   -In     : 원본 문서 (그대로 둔다 — 한셀은 연 문서 옆에 잠금 파일을 만들므로 작업용 사본을 넘긴다)
#   -Script : 편집 대본 JSON ({"edits":[{"op":"set",...}, ...]})
#   -Out    : 결과 (-Format: cell(XML, -2006) | bin(-2007) | xlsx(51))
#   -List   : 여러 작업을 한셀 하나로: [{"in":..,"script":..,"out":..}, ...] JSON 파일
#   -Resave : 편집 없이 다시 저장만 (rcell 이 쓴 파일을 한셀이 읽은 결과 = 판정 기준)
#
# 사용: powershell -File edit-oracle.ps1 -In a.cell -Script edits.json -Out a.hcell.cell
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param(
    [string]$In,
    [string]$Script,
    [string]$Out,
    [string]$List,
    [string]$Format = "cell",
    [switch]$Resave
)
$ErrorActionPreference = "Stop"
$codes = @{ "cell" = -2006; "bin" = -2007; "legacy" = -2005; "xlsx" = 51 }

# 16진 RRGGBB → COM 색 (BGR 정수)
function Get-Bgr([string]$hex) {
    $h = $hex.TrimStart('#')
    $r = [Convert]::ToInt32($h.Substring(0, 2), 16)
    $g = [Convert]::ToInt32($h.Substring(2, 2), 16)
    $b = [Convert]::ToInt32($h.Substring(4, 2), 16)
    return [int]($r + $g * 256 + $b * 65536)
}

function Get-Sheet($wb, $name) {
    if ($null -eq $name) { return $wb.Worksheets.Item(1) }
    # 숫자는 0부터 시작하는 시트 번호
    if ($name -is [int] -or $name -is [long]) { return $wb.Worksheets.Item([int]$name + 1) }
    return $wb.Worksheets.Item([string]$name)
}

# 열 이름 → 범위 주소 (C → C1, 2개 → C1:D1)
function Get-ColRange([string]$col, [int]$count) {
    $n = 0
    foreach ($ch in $col.ToUpper().ToCharArray()) { $n = $n * 26 + ([int]$ch - 64) }
    $last = $n + $count - 1
    $name = ""
    $k = $last
    while ($k -gt 0) { $m = ($k - 1) % 26; $name = [char](65 + $m) + $name; $k = [math]::Floor(($k - 1) / 26) }
    return "$($col.ToUpper())1:$($name)1"
}

# 편집의 count (없으면 1)
function Get-Count($e) { if ($null -ne $e.count) { return [int]$e.count } else { return 1 } }

$hAlign = @{ "left" = -4131; "center" = -4108; "right" = -4152; "general" = 1; "justify" = -4130; "distributed" = -4117; "centerContinuous" = 7; "fill" = 5 }
$vAlign = @{ "top" = -4160; "center" = -4108; "bottom" = -4107; "justify" = -4130; "distributed" = -4117 }
$lineStyles = @{ "thin" = @(1, 2); "medium" = @(1, -4138); "thick" = @(1, 4); "dashed" = @(-4115, 2); "dotted" = @(-4118, 2); "double" = @(-4119, 4); "hair" = @(1, 1) }

function Set-Edge($range, [int]$which, $style, $color) {
    $b = $range.Borders.Item($which)
    if ($null -eq $style) { $b.LineStyle = -4142; return }
    $b.LineStyle = $style[0]
    $b.Weight = $style[1]
    if ($color) { $b.Color = (Get-Bgr $color) }
}

function Invoke-Edit($wb, $e) {
    $ws = Get-Sheet $wb $e.sheet
    switch ($e.op) {
        "set" {
            $r = $ws.Range([string]$e.cell)
            $v = [string]$e.input
            if ($v.StartsWith("=")) { $r.Formula = $v } else { $r.Value2 = $v }
        }
        "clear" { $ws.Range([string]$e.range).ClearContents() | Out-Null }
        "clearAll" { $ws.Range([string]$e.range).Clear() | Out-Null }
        "insertRows" { $ws.Range("A$($e.row):A$([int]$e.row + [int]$e.count - 1)").EntireRow.Insert() | Out-Null }
        "deleteRows" { $ws.Range("A$($e.row):A$([int]$e.row + [int]$e.count - 1)").EntireRow.Delete() | Out-Null }
        "insertCols" { $ws.Range((Get-ColRange $e.col ([int]$e.count))).EntireColumn.Insert() | Out-Null }
        "deleteCols" { $ws.Range((Get-ColRange $e.col ([int]$e.count))).EntireColumn.Delete() | Out-Null }
        "format" {
            $r = $ws.Range([string]$e.range)
            $p = $e.PSObject.Properties.Name
            if ($p -contains "bold") { $r.Font.Bold = [bool]$e.bold }
            if ($p -contains "italic") { $r.Font.Italic = [bool]$e.italic }
            if ($p -contains "underline") { if ($e.underline) { $r.Font.Underline = 2 } else { $r.Font.Underline = -4142 } }
            if ($p -contains "strike") { $r.Font.Strikethrough = [bool]$e.strike }
            if ($p -contains "fontName") { $r.Font.Name = [string]$e.fontName }
            if ($p -contains "fontSize") { $r.Font.Size = [double]$e.fontSize }
            if ($p -contains "fontColor") { $r.Font.Color = (Get-Bgr $e.fontColor) }
            if ($p -contains "fill") {
                if ($e.fill) { $r.Interior.Color = (Get-Bgr $e.fill) } else { $r.Interior.ColorIndex = -4142 }
            }
            if ($p -contains "hAlign") { $r.HorizontalAlignment = $hAlign[[string]$e.hAlign] }
            if ($p -contains "vAlign") { $r.VerticalAlignment = $vAlign[[string]$e.vAlign] }
            if ($p -contains "wrap") { $r.WrapText = [bool]$e.wrap }
            if ($p -contains "numFmt") { $r.NumberFormat = [string]$e.numFmt }
        }
        "border" {
            $r = $ws.Range([string]$e.range)
            $style = if ($e.style) { $lineStyles[[string]$e.style] } else { $lineStyles["thin"] }
            $color = if ($e.color) { [string]$e.color } else { $null }
            switch ([string]$e.preset) {
                "all" { foreach ($k in 7, 8, 9, 10, 11, 12) { try { Set-Edge $r $k $style $color } catch {} } }
                "outline" { foreach ($k in 7, 8, 9, 10) { Set-Edge $r $k $style $color } }
                "none" { foreach ($k in 5, 6, 7, 8, 9, 10, 11, 12) { try { Set-Edge $r $k $null $null } catch {} } }
                "left" { Set-Edge $r 7 $style $color }
                "top" { Set-Edge $r 8 $style $color }
                "bottom" { Set-Edge $r 9 $style $color }
                "right" { Set-Edge $r 10 $style $color }
                default { throw "테두리 종류 $($e.preset)" }
            }
        }
        "merge" {
            $r = $ws.Range([string]$e.range)
            $r.Merge() | Out-Null
            if ($e.center) { $r.HorizontalAlignment = -4108 }
        }
        "unmerge" { $ws.Range([string]$e.range).UnMerge() | Out-Null }
        "colWidth" { $ws.Range((Get-ColRange $e.col 1)).EntireColumn.ColumnWidth = [double]$e.width }
        "rowHeight" { $ws.Range("A$($e.row)").EntireRow.RowHeight = [double]$e.height }
        "addSheet" {
            $after = if ($null -ne $e.after) { Get-Sheet $wb $e.after } else { $wb.Worksheets.Item($wb.Worksheets.Count) }
            $n = $wb.Worksheets.Add([Type]::Missing, $after)
            if ($e.name) { $n.Name = [string]$e.name }
        }
        "renameSheet" { $ws.Name = [string]$e.name }
        "deleteSheet" { $ws.Delete() | Out-Null }
        "moveSheet" {
            $to = [int]$e.to
            $count = $wb.Worksheets.Count
            if ($to -ge $count - 1) { $ws.Move([Type]::Missing, $wb.Worksheets.Item($count)) | Out-Null }
            else {
                # 옮긴 뒤 자리 to (0부터): 자기 자리보다 뒤로 가면 그 자리의 시트 뒤에, 앞으로 가면 그 시트 앞에
                $from = $ws.Index - 1
                if ($to -gt $from) { $ws.Move([Type]::Missing, $wb.Worksheets.Item($to + 1)) | Out-Null }
                else { $ws.Move($wb.Worksheets.Item($to + 1)) | Out-Null }
            }
        }
        "copy" {
            $dst = Get-Sheet $wb $(if ($e.toSheet) { $e.toSheet } else { $e.sheet })
            $ws.Range([string]$e.from).Copy($dst.Range([string]$e.to)) | Out-Null
        }
        "move" {
            $dst = Get-Sheet $wb $(if ($e.toSheet) { $e.toSheet } else { $e.sheet })
            $ws.Range([string]$e.from).Cut($dst.Range([string]$e.to)) | Out-Null
        }
        "fillDown" { $ws.Range([string]$e.range).FillDown() | Out-Null }
        "fillRight" { $ws.Range([string]$e.range).FillRight() | Out-Null }
        "sort" {
            # 정렬 (Range.Sort — 열쇠 3개까지). Sort 개체(Worksheet.Sort)는 여러 열 범위에서 Header=xlNo 를 따르지
            # 않고 첫 행을 머리글로 둔다 (관측). 왼쪽에서 오른쪽은 Orientation 2
            $rg = $ws.Range([string]$e.range)
            $M = [Type]::Missing
            $keys = @($M, $M, $M); $orders = @(1, 1, 1)
            $i = 0
            foreach ($k in $e.keys) {
                $by = [string]$k.by
                # 왼쪽에서 오른쪽: 한셀 자동화는 열쇠 칸의 (열 − 범위 첫 열) 을 행 차례로 쓴다 (관측) — 그 자리의 칸을 준다
                if ($e.leftToRight) { $keys[$i] = $ws.Cells.Item($rg.Row, $rg.Column + [int]$by - $rg.Row) }
                else { $keys[$i] = $ws.Cells.Item($rg.Row, $ws.Range("$($by)1").Column) }
                if ($k.desc) { $orders[$i] = 2 }
                $i++
            }
            $hdr = if ($e.header) { 1 } else { 2 }
            $orient = if ($e.leftToRight) { 2 } else { 1 }
            $rg.Sort($keys[0], $orders[0], $keys[1], $M, $orders[1], $keys[2], $orders[2], $hdr, $M, [bool]$e.matchCase, $orient) | Out-Null
        }
        "autoFill" {
            # 자동 채우기 (Range.AutoFill, XlAutoFillType)
            $types = @{ "default" = 0; "copy" = 1; "series" = 2; "formats" = 3; "values" = 4; "days" = 5; "weekdays" = 6; "months" = 7; "years" = 8 }
            $t = if ($e.type) { $types[[string]$e.type] } else { 0 }
            $ws.Range([string]$e.range).AutoFill($ws.Range([string]$e.to), $t) | Out-Null
        }
        "series" {
            # 연속 데이터 (Range.DataSeries)
            $rg = $ws.Range([string]$e.range)
            $M = [Type]::Missing
            $rowcol = if ($null -ne $e.rows) { if ($e.rows) { 1 } else { 2 } } elseif ($rg.Columns.Count -gt $rg.Rows.Count) { 1 } else { 2 }
            $types = @{ "linear" = -4132; "growth" = 2; "date" = 3; "autoFill" = 4 }
            $dates = @{ "day" = 1; "weekday" = 2; "month" = 3; "year" = 4 }
            $t = if ($e.type) { $types[[string]$e.type] } else { -4132 }
            $d = if ($e.date) { $dates[[string]$e.date] } else { 1 }
            $step = if ($null -ne $e.step) { [double]$e.step } else { 1.0 }
            $stop = if ($null -ne $e.stop) { [double]$e.stop } else { $M }
            $rg.DataSeries($rowcol, $t, $d, $step, $stop, [bool]$e.trend) | Out-Null
        }
        "replace" {
            # 바꾸기 (Range.Replace — LookAt 1 = 셀 전체, 2 = 일부). all 이면 모든 시트
            $look = if ($e.wholeCell) { 1 } else { 2 }
            $n = if ($e.all) { $wb.Worksheets.Count } else { 1 }
            for ($k = 1; $k -le $n; $k++) {
                $s = if ($e.all) { $wb.Worksheets.Item($k) } else { $ws }
                $rg = if ($e.range) { $s.Range([string]$e.range) } else { $s.UsedRange }
                [void]$rg.Replace([string]$e.find, [string]$e.replace, $look, 1, [bool]$e.matchCase)
            }
        }
        "insertCells" {
            # 셀 삽입 (아래로 -4121, 오른쪽으로 -4161)
            $sh = if ([string]$e.shift -eq "right") { -4161 } else { -4121 }
            [void]$ws.Range([string]$e.range).Insert($sh)
        }
        "deleteCells" {
            # 셀 삭제 (위로 -4162, 왼쪽으로 -4159)
            $sh = if ([string]$e.shift -eq "left") { -4159 } else { -4162 }
            [void]$ws.Range([string]$e.range).Delete($sh)
        }
        "hideRows" { $ws.Range("A$($e.row):A$([int]$e.row + [int](Get-Count $e) - 1)").EntireRow.Hidden = $true }
        "unhideRows" { $ws.Range("A$($e.row):A$([int]$e.row + [int](Get-Count $e) - 1)").EntireRow.Hidden = $false }
        "hideCols" { $ws.Range((Get-ColRange $e.col ([int](Get-Count $e)))).EntireColumn.Hidden = $true }
        "unhideCols" { $ws.Range((Get-ColRange $e.col ([int](Get-Count $e)))).EntireColumn.Hidden = $false }
        "hideSheet" { $ws.Visible = 0 }
        "unhideSheet" { $ws.Visible = -1 }
        "groupRows" { [void]$ws.Range("A$($e.row):A$([int]$e.row + [int](Get-Count $e) - 1)").EntireRow.Group() }
        "ungroupRows" { [void]$ws.Range("A$($e.row):A$([int]$e.row + [int](Get-Count $e) - 1)").EntireRow.Ungroup() }
        "groupCols" { [void]$ws.Range((Get-ColRange $e.col ([int](Get-Count $e)))).EntireColumn.Group() }
        "ungroupCols" { [void]$ws.Range((Get-ColRange $e.col ([int](Get-Count $e)))).EntireColumn.Ungroup() }
        "freeze" {
            # 틀 고정은 창이 보여야 적용된다 (관측) — 이 인스턴스의 창을 잠깐 보인다
            $a = $wb.Application
            $a.Visible = $true
            $ws.Activate()
            $w = $a.ActiveWindow
            if ($w.FreezePanes) { $w.FreezePanes = $false }
            [void]$ws.Range([string]$e.cell).Select()
            $w.FreezePanes = $true
            $a.Visible = $false
        }
        "unfreeze" {
            $a = $wb.Application
            $a.Visible = $true
            $ws.Activate()
            $a.ActiveWindow.FreezePanes = $false
            $a.Visible = $false
        }
        "tabColor" { if ($e.color) { $ws.Tab.Color = (Get-Bgr $e.color) } else { $ws.Tab.ColorIndex = -4142 } }
        "sortFilter" {
            # 자동 필터의 정렬 (AutoFilter.Sort): 필터 범위의 그 열
            $af = $ws.AutoFilter
            $rg = $af.Range
            $c = $ws.Range("$([string]$e.col)1").Column
            $key = $ws.Range($ws.Cells.Item($rg.Row, $c), $ws.Cells.Item($rg.Row + $rg.Rows.Count - 1, $c))
            $s = $af.Sort
            $s.SortFields.Clear()
            $order = if ($e.desc) { 2 } else { 1 }
            $s.SortFields.Add($key, 0, $order) | Out-Null
            $s.Header = 1
            $s.MatchCase = [bool]$e.matchCase
            $s.Apply()
        }
        default { throw "모르는 편집 $($e.op)" }
    }
}

function Invoke-Job($app, [string]$src, [string]$script, [string]$dst) {
    $wb = $app.Workbooks.Open($src)
    try {
        $fails = 0
        if (-not $Resave -and $script) {
            $doc = Get-Content -Raw -Encoding UTF8 $script | ConvertFrom-Json
            $i = 0
            foreach ($e in $doc.edits) {
                try { Invoke-Edit $wb $e } catch { $fails++; Write-Host "  (실패) #$i $($e.op): $($_.Exception.Message)" }
                $i++
            }
        }
        # 바뀐 것이 없으면 한셀은 SaveAs 에서 파일을 그대로 복사한다 (관측) — 다시 쓰게 만든다
        try { $wb.Saved = $false } catch {}
        if (Test-Path $dst) { Remove-Item $dst -Force }
        $wb.SaveAs($dst, $codes[$Format])
        Write-Host "OK   $([IO.Path]::GetFileName($dst)) (실패 $fails)"
    } finally {
        try { $wb.Close($false) } catch {}
    }
}

$jobs = @()
if ($List) {
    $jobs = Get-Content -Raw -Encoding UTF8 $List | ConvertFrom-Json
} else {
    if (-not $In -or -not $Out) { throw "사용: edit-oracle.ps1 -In <문서> -Script <대본.json> -Out <결과>" }
    $jobs = @([pscustomobject]@{ in = $In; script = $Script; out = $Out })
}
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    foreach ($j in $jobs) {
        $src = [IO.Path]::GetFullPath([string]$j.in)
        $dst = [IO.Path]::GetFullPath([string]$j.out)
        for ($try = 1; $try -le 3; $try++) {
            try { Invoke-Job $app $src ([string]$j.script) $dst; break }
            catch {
                Write-Host "FAIL $([IO.Path]::GetFileName($dst)) (시도 $try): $($_.Exception.Message)"
                Start-Sleep -Milliseconds 500
            }
        }
    }
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
