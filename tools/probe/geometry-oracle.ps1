# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 화면 배치 정답 생성기: 한셀(COM)로 문서를 열어 시트마다 열의 왼쪽·너비, 행의 위·높이(pt)를
# TSV 로 저장한다. rcell 렌더러의 배치(render::sheet_geometry)를 이 결과와 대조한다
# (tests/render_geometry.rs 가 RCELL_GEOMETRY_ORACLE 환경 변수로 받는다).
#
#   #dir<TAB>문서 폴더
#   파일<TAB>시트이름<TAB>s<TAB>StandardWidth<TAB>StandardHeight
#   파일<TAB>시트이름<TAB>c<TAB>열(1부터)<TAB>Left<TAB>Width
#   파일<TAB>시트이름<TAB>r<TAB>행(1부터)<TAB>Top<TAB>Height
#
# 사용: powershell -File geometry-oracle.ps1 -InDir <폴더> -Out geometry.tsv
#   -Filter  : 대상 파일 (기본 *.cell — 이름에 .bin./.legacy./.as./.rcell. 이 든 파일은 뺀다)
#   -Legacy  : 레거시 판(X.legacy.cell)도 잰다 (한셀이 레거시를 읽은 화면이 XML 판과 같은지 대조)
#   -MaxCols / -MaxRows : 시트마다 잴 최대 열·행 수 (사용 범위 + 여유분까지)
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
#       한셀 인스턴스를 따로 띄우고 끝나면 닫는다 (다른 사람이 쓰는 한셀은 건드리지 않는다).
param(
    [Parameter(Mandatory = $true)][string]$InDir,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Filter = "*.cell",
    [int]$MaxCols = 60,
    [int]$MaxRows = 300,
    [switch]$Legacy
)
$ErrorActionPreference = "Stop"

# 한셀이 바쁠 때(RPC_E_CALL_REJECTED 등) 잠시 뒤 다시 부른다
function Retry([scriptblock]$action) {
    for ($i = 0; ; $i++) {
        try { return & $action } catch {
            if ($i -ge 5) { throw }
            Start-Sleep -Milliseconds (300 * ($i + 1))
        }
    }
}

$lines = New-Object System.Collections.Generic.List[string]
$files = Get-ChildItem $InDir -File -Filter $Filter |
    Where-Object { $_.Name -notmatch '\.(bin|legacy|as|rcell)\.' -or ($Legacy -and $_.Name -match '\.legacy\.cell$') } |
    Sort-Object Name
$lines.Add("#dir`t$((Resolve-Path $InDir).Path)")
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    foreach ($f in $files) {
        $wb = $null
        try {
            $wb = Retry { $app.Workbooks.Open($f.FullName) }
            foreach ($ws in @($wb.Worksheets)) {
                $name = $ws.Name
                $pre = "$($f.Name)`t$name"
                $lines.Add(("{0}`ts`t{1:R}`t{2:R}" -f $pre, [double]$ws.StandardWidth, [double]$ws.StandardHeight))
                $ur = $ws.UsedRange
                $lastCol = [int]$ur.Column + [int]$ur.Columns.Count - 1
                $lastRow = [int]$ur.Row + [int]$ur.Rows.Count - 1
                $nc = [Math]::Min($MaxCols, [Math]::Max(20, $lastCol + 3))
                $nr = [Math]::Min($MaxRows, [Math]::Max(40, $lastRow + 3))
                for ($c = 1; $c -le $nc; $c++) {
                    $col = $ws.Columns.Item($c)
                    $lines.Add(("{0}`tc`t{1}`t{2:R}`t{3:R}" -f $pre, $c, [double]$col.Left, [double]$col.Width))
                }
                for ($r = 1; $r -le $nr; $r++) {
                    $row = $ws.Rows.Item($r)
                    $lines.Add(("{0}`tr`t{1}`t{2:R}`t{3:R}" -f $pre, $r, [double]$row.Top, [double]$row.Height))
                }
            }
            Write-Host "OK   $($f.Name)"
        } catch {
            Write-Host "FAIL $($f.Name): $($_.Exception.Message)"
        } finally {
            if ($wb) { try { $wb.Close($false) } catch {} }
        }
    }
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
[IO.File]::WriteAllLines($Out, $lines, (New-Object Text.UTF8Encoding $false))
"wrote $($lines.Count) lines to $Out"
