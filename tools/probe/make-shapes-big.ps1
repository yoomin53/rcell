# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 도형 윤곽 따라 그리기 (0/3): 한셀(COM)로 msoAutoShapeType 목록의 도형을 큰 크기(기본 240×240pt)로 시트마다 8개 (4 × 2) 그려 저장한다.
# 갈무리(screen-capture.ps1) → shape-trace.ps1 → shape-trace-gen.js 순으로 윤곽 표를 만든다.
param([Parameter(Mandatory = $true)][string]$OutDir, [string]$Types = "", [double]$W = 240, [double]$H = 240, [string]$Name = "shapes2", [int]$Cols = 4, [int]$Rows = 2)
# shape tracing experiment: the given msoAutoShapeTypes big (default 240 x 240 pt), 8 per sheet, text-free
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $OutDir | Out-Null
if (-not $Types) { $Types = "126,127,128,133,134,135,136,87,85,101,102,97,98,99,100,172,173,45,46,47,48,89,90,140,168,42,174,39,40,41,23,178,176,60,177,103,19,108,179" }
$list = $Types -split "," | ForEach-Object { [int]$_ }
$app = New-Object -ComObject HCell.Application
$log = New-Object System.Collections.Generic.List[string]
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    $wb = $app.Workbooks.Add()
    $per = $Cols * $Rows
    $sheet = 0; $q = 0; $ws = $null
    foreach ($t in $list) {
        if ($q % $per -eq 0) {
            $sheet++
            while ($wb.Worksheets.Count -lt $sheet) { $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) | Out-Null }
            $ws = $wb.Worksheets.Item($sheet); $ws.Name = "s$sheet"
            $q = 0
        }
        $left = 10 + ($W + 10) * ($q % $Cols); $top = 10 + ($H + 10) * [math]::Floor($q / $Cols)
        try {
            $s = $ws.Shapes.AddShape($t, $left, $top, $W, $H)
            $adj = @()
            try { for ($i = 1; $i -le $s.Adjustments.Count; $i++) { $adj += [string]$s.Adjustments.Item($i) } } catch {}
            $log.Add("$sheet`t$q`t$t`t$($s.AutoShapeType)`t$($adj -join ',')")
        } catch {
            $log.Add("$sheet`t$q`t$t`tFAIL")
        }
        $q++
    }
    $wb.Worksheets.Item(1).Activate() | Out-Null
    $p = Join-Path $OutDir "$Name.cell"
    if (Test-Path $p) { Remove-Item $p -Force }
    $wb.SaveAs($p, -2006)
    $wb.Close($false)
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
    [IO.File]::WriteAllLines((Join-Path $OutDir "$Name.tsv"), $log)
}
