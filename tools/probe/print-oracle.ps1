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
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다. 한셀 인스턴스를 따로 띄우고
#       끝나면 닫는다. 기본 프린터(한셀이 설치하는 Hancom PDF)에 맞춘 페이지다. 문서는 저장하지 않는다.
param(
    [Parameter(Mandatory = $true)][string]$InDir,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Filter = "*.cell",
    [string]$Only = "",
    [string]$PdfDir = ""
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
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("#dir`t$((Resolve-Path $InDir).Path)")
$files = Get-ChildItem $InDir -File -Filter $Filter |
    Where-Object { $_.Name -notmatch '\.(as|rcell|bin)\.' -and (-not $Only -or $_.Name -match $Only) } | Sort-Object Name
# 바이너리 판은 한셀 COM 이 다시 열지 못한다 (.bin. 은 뺀다)
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
            $wb = Retry { $app.Workbooks.Open($f.FullName) }
            $k = 0
            foreach ($ws in @($wb.Worksheets)) {
                $k++
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
                $lines.Add(("{0}`t{1}`t{2}`t{3}`t{4}`t{5}`t{6}`t{7}`t{8}" -f $f.Name, $k, $ws.Name, $pages, ($h -join ','), ($v -join ','), $zoom, $ps.PrintArea, $vis))
                if ($PdfDir -and $vis -eq -1) {
                    $pdf = Join-Path $PdfDir ("{0}.{1}.pdf" -f $f.Name, $k)
                    try { Retry { $ws.ExportAsFixedFormat(0, $pdf) } } catch { Write-Host "PDF FAIL $($f.Name) $k : $($_.Exception.Message)" }
                }
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
[IO.File]::WriteAllLines($Out, $lines, (New-Object Text.UTF8Encoding $false))
"wrote $($lines.Count) lines to $Out"
