# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 바이너리 .cell(xlCellb)을 한셀이 읽은 결과를 XML .cell 로 저장한다: <이름>.bin.cell → <이름>.bin.as.cell.
# 한셀의 COM 은 확장자가 .cell 인 바이너리 패키지를 열지 못하지만(RPC_E_SERVERFAULT), 같은 파일을 .xlsb 로
# 이름만 바꾸면 연다 (관측). rcell 이 쓴 바이너리를 한셀이 어떻게 읽는지 확인할 때 쓴다.
# 주의: .xlsb 로 연 결과는 한셀의 Excel 가져오기를 거치므로 셀 서식의 한컴 확장(문단 모양)·차등 서식 글꼴의
# 기본 색·쓰이지 않는 사용자 정의 스타일이 바뀐다 — 같은 길로 연 두 결과끼리 비교한다.
#
# 사용: powershell -File bin-as.ps1 -InDir <폴더> [-Filter *.bin.cell] [-Suffix .bin.as.cell]
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param(
    [Parameter(Mandatory = $true)][string]$InDir,
    [string]$Filter = "*.bin.cell",
    [string]$Suffix = ".bin.as.cell"
)
$ErrorActionPreference = "Stop"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("rcell-bin-as-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force $tmp | Out-Null
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    foreach ($f in Get-ChildItem $InDir -File -Filter $Filter) {
        $stem = $f.Name -replace '\.bin\.cell$', '' -replace '\.cell$', ''
        $out = Join-Path $f.DirectoryName ($stem + $Suffix)
        $x = Join-Path $tmp ($stem + ".xlsb")
        Copy-Item $f.FullName $x -Force
        try {
            $wb = $app.Workbooks.Open($x)
            # 바뀐 것이 없으면 한셀은 SaveAs 에서 파일을 그대로 복사한다 (관측) — 다시 쓰게 만든다
            try { $wb.Saved = $false } catch {}
            if (Test-Path $out) { Remove-Item $out -Force }
            $wb.SaveAs($out, -2006)
            $wb.Close($false)
            Write-Host "OK   $($f.Name)"
        } catch {
            Write-Host "FAIL $($f.Name): $($_.Exception.Message)"
            try { $app.Workbooks | ForEach-Object { $_.Close($false) } } catch {}
        }
    }
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
