# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# rcell 바이너리 쓰기의 한셀 검증: 코퍼스의 X.cell 마다 rcell 로 바이너리(X.binw.cell)를 쓰고, 한셀이 그것을
# .xlsb 로 이름만 바꿔 열어(bin-as.ps1 과 같은 길) XML 로 저장한다 → X.binw.as.cell (-Exact 면 한셀이 잃는 값도
# 잃는 쓰기 → X.binx.cell, X.binx.as.cell).
# 정답은 한셀이 자기 바이너리를 같은 길로 연 X.bin.as.cell 이다 (tests/write_binary.rs 의
# hancell_reads_rcell_binary_like_its_own, RCELL_BINW_ORACLE=<OutDir>).
# 코퍼스는 고치지 않는다 — 결과는 OutDir 아래 같은 하위 폴더에 둔다.
#
# 사용: powershell -File binw-oracle.ps1 -Corpus <코퍼스> -OutDir <출력> -Rcell <rcell.exe> [-Exact] [-Only <이름 정규식>]
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param(
    [Parameter(Mandatory = $true)][string]$Corpus,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$Rcell,
    [string]$Only = "",
    [switch]$Exact,
    [string[]]$Dirs = @("", "templates", "craft", "craft-charts", "craft-charts2")
)
$ErrorActionPreference = "Stop"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("rcell-binw-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force $tmp | Out-Null

# 1) rcell 로 바이너리 쓰기 (한셀처럼 저장 폴더를 BrtAbsPath15 에). -Exact 면 한셀이 잃는 값도 잃는 쓰기 → X.binx.*
$kind = if ($Exact) { "binx" } else { "binw" }
$jobs = @()
foreach ($d in $Dirs) {
    $src = Join-Path $Corpus $d
    if (-not (Test-Path $src)) { continue }
    $dst = Join-Path $OutDir $d
    New-Item -ItemType Directory -Force $dst | Out-Null
    foreach ($b in Get-ChildItem $src -File -Filter "*.bin.cell") {
        $stem = $b.Name -replace '\.bin\.cell$', ''
        if ($Only -and ($stem -notmatch $Only)) { continue }
        $x = Join-Path $src ($stem + ".cell")
        if (-not (Test-Path $x)) { continue }
        $out = Join-Path $dst ($stem + "." + $kind + ".cell")
        $rargs = @("convert", $x, $out, "--format", "bin", "--abs-path", ($dst.TrimEnd('\') + '\'))
        if ($Exact) { $rargs += "--exact" }
        & $Rcell @rargs
        if ($LASTEXITCODE -ne 0) { Write-Host "RCELL FAIL $stem"; continue }
        $jobs += , @($out, (Join-Path $dst ($stem + "." + $kind + ".as.cell")))
    }
}

# 2) 한셀로 열어 XML 로 저장 (자기 인스턴스, 문서를 닫고 마지막에 Quit)
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    foreach ($j in $jobs) {
        $in = $j[0]; $out = $j[1]
        $x = Join-Path $tmp ([IO.Path]::GetFileNameWithoutExtension($in) + ".xlsb")
        Copy-Item $in $x -Force
        $done = $false
        for ($try = 1; $try -le 3 -and -not $done; $try++) {
            try {
                $wb = $app.Workbooks.Open($x)
                # 바뀐 것이 없으면 한셀은 SaveAs 에서 파일을 그대로 복사한다 (관측) — 다시 쓰게 만든다
                try { $wb.Saved = $false } catch {}
                if (Test-Path $out) { Remove-Item $out -Force }
                $wb.SaveAs($out, -2006)
                $wb.Close($false)
                Write-Host "OK   $([IO.Path]::GetFileName($in))"
                $done = $true
            } catch {
                Write-Host "FAIL($try) $([IO.Path]::GetFileName($in)): $($_.Exception.Message)"
                try { $app.Workbooks | ForEach-Object { $_.Close($false) } } catch {}
                Start-Sleep -Milliseconds 500
            }
        }
    }
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
