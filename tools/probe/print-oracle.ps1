# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 인쇄 페이지 정답 생성기: 한셀(COM)로 문서를 열어 시트마다 페이지 수와 페이지 나누기(HPageBreaks·VPageBreaks)를
# TSV 로 적는다. -PdfDir 를 주면 시트마다 한셀 PDF(ExportAsFixedFormat)도 저장한다 (print-diff.ps1 이 쓴다).
# rcell 의 인쇄 배치(rcell::print)를 이 결과와 대조한다 (tests/print_layout.rs 가 RCELL_PRINT_ORACLE 로 받는다).
#
#   #dir<TAB>문서 폴더
#   파일<TAB>시트번호(1부터)<TAB>시트이름<TAB>페이지 수<TAB>가로 나누기(행, m = 수동)<TAB>세로 나누기(열)<TAB>배율(맞추기면 fit)<TAB>인쇄 영역<TAB>보임
#
# 사용: powershell -File print-oracle.ps1 -InDir <폴더> -Out print.tsv [-Filter *.cell] [-Only 정규식] [-PdfDir <폴더>]
#   -MaxRows / -MaxCols : 쓰는 범위가 이보다 큰 시트는 건너뛴다 (한셀이 수십만 페이지를 세다 멈춘다)
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다. 한셀 인스턴스를 따로 띄우고
#       끝나면 닫는다. 기본 프린터(한셀이 설치하는 Hancom PDF)에 맞춘 페이지다. 문서는 저장하지 않는다.
param(
    [Parameter(Mandatory = $true)][string]$InDir,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Filter = "*.cell",
    [string]$Only = "",
    [string]$PdfDir = "",
    [int]$MaxRows = 20000,
    [int]$MaxCols = 2000
)
$ErrorActionPreference = "Stop"

function Retry([scriptblock]$action) {
    for ($i = 0; ; $i++) {
        try { return & $action } catch {
            if ($i -ge 5) { throw }
            Start-Sleep -Milliseconds (300 * ($i + 1))
        }
    }
}

if ($PdfDir) { New-Item -ItemType Directory -Force $PdfDir | Out-Null }
# 한 줄씩 바로 쓴다 (한셀이 어떤 문서에서 멈춰도 앞 결과는 남는다)
$writer = New-Object IO.StreamWriter($Out, $false, (New-Object Text.UTF8Encoding $false))
$writer.AutoFlush = $true
$count = 0
function Emit([string]$line) { $script:writer.WriteLine($line); $script:count++ }
Emit "#dir`t$((Resolve-Path $InDir).Path)"
$files = Get-ChildItem $InDir -File -Filter $Filter |
    Where-Object { $_.Name -notmatch '\.(as|rcell)\.' -and -not $_.Name.StartsWith('~$') -and (-not $Only -or $_.Name -match $Only) } | Sort-Object Name
# 바이너리 판(.bin.)은 .xlsb 이름의 사본으로 연다 (한셀은 .bin.cell 이름으로는 열지 못하고 .xlsb 로는 연다 — bin-as.ps1 과
# 같은 길). ~$ 로 시작하는 것은 한셀 잠금 파일
function NewApp {
    $a = New-Object -ComObject HCell.Application
    try { $a.Visible = $false } catch {}
    try { $a.DisplayAlerts = $false } catch {}
    return $a
}
function QuitApp($a) {
    try { $a.Quit() } catch {}
    try { [System.Runtime.InteropServices.Marshal]::ReleaseComObject($a) | Out-Null } catch {}
}
$app = NewApp
try {
    foreach ($f in $files) {
        $wb = $null
        try {
            $path = $f.FullName
            if ($f.Name -match '\.bin\.cell$') {
                $tmpx = Join-Path ([IO.Path]::GetTempPath()) ("rcell-oracle-" + [Guid]::NewGuid().ToString("N") + ".xlsb")
                Copy-Item $f.FullName $tmpx -Force
                $path = $tmpx
            }
            $wb = Retry { $app.Workbooks.Open($path) }
            $k = 0
            $huge = $false
            foreach ($ws in @($wb.Worksheets)) {
                $k++
                # 쓰는 범위가 아주 크면 (마지막 칸에 값 — 수십만 페이지) 한셀이 페이지를 세다 멈추므로 건너뛴다
                $ur = $ws.UsedRange
                $lastRow = [int]$ur.Row + [int]$ur.Rows.Count - 1
                $lastCol = [int]$ur.Column + [int]$ur.Columns.Count - 1
                if ($lastRow -gt $MaxRows -or $lastCol -gt $MaxCols) {
                    Write-Host "SKIP $($f.Name) $k : $lastRow x $lastCol"
                    $huge = $true
                    continue
                }
                $ps = $ws.PageSetup
                $pages = Retry { [int]$ps.Pages.Count }
                $h = @(); $v = @()
                $hb = $ws.HPageBreaks
                for ($i = 1; $i -le $hb.Count; $i++) { $b = $hb.Item($i); $h += "$($b.Location.Row)$(if ($b.Type -eq -4135) {'m'} else {''})" }
                $vb = $ws.VPageBreaks
                for ($i = 1; $i -le $vb.Count; $i++) { $b = $vb.Item($i); $v += "$($b.Location.Column)$(if ($b.Type -eq -4135) {'m'} else {''})" }
                $zoom = $ps.Zoom
                if ($zoom -is [bool]) { $zoom = "fit" }
                $vis = [int]$ws.Visible
                Emit ("{0}`t{1}`t{2}`t{3}`t{4}`t{5}`t{6}`t{7}`t{8}" -f $f.Name, $k, $ws.Name, $pages, ($h -join ','), ($v -join ','), $zoom, $ps.PrintArea, $vis)
            }
            # 시트의 ExportAsFixedFormat 은 통합 문서의 보이는 시트를 모두 PDF 로 쓴다 (한 번만)
            if ($PdfDir -and -not $huge) {
                $pdf = Join-Path $PdfDir ("{0}.pdf" -f $f.Name)
                $first = @($wb.Worksheets)[0]
                try { Retry { $first.ExportAsFixedFormat(0, $pdf) } } catch { Write-Host "PDF FAIL $($f.Name): $($_.Exception.Message)" }
            }
            Write-Host "OK   $($f.Name)"
        } catch {
            Write-Host "FAIL $($f.Name): $($_.Exception.Message)"
            # 실패한 인스턴스는 닫고 새로 띄운다 (대화 상자가 남으면 뒤 문서도 모두 실패한다)
            if ($wb) { try { $wb.Close($false) } catch {} }
            $wb = $null
            QuitApp $app
            $app = NewApp
        } finally {
            if ($wb) { try { $wb.Close($false) } catch {} }
        }
    }
} finally {
    QuitApp $app
}
$writer.Close()
"wrote $count lines to $Out"
