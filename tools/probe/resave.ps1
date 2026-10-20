# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 폴더의 문서를 한셀 하나로 차례로 열어 다른 이름·형식으로 다시 저장한다.
# rcell 이 쓴 파일을 한셀이 어떻게 읽는지(= 정답) 확인할 때 쓴다:
#   X.rcell.cell → 한셀 → X.rcell.as.cell, 그다음 rcell diff 로 비교.
#
#   -Filter : 대상 파일 (예: *.rcell.cell)
#   -Suffix : 원래 확장자(.cell) 대신 붙일 꼬리 (예: .as.cell)
#   -Format : cell(XML, -2006) | bin(-2007) | legacy(-2005) | xlsx(51)
#
# 사용: powershell -File resave.ps1 -InDir <폴더> -Filter *.rcell.cell -Suffix .as.cell
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param(
    [Parameter(Mandatory = $true)][string]$InDir,
    [Parameter(Mandatory = $true)][string]$Filter,
    [string]$Suffix = ".as.cell",
    [string]$Format = "cell"
)
$ErrorActionPreference = "Stop"
$codes = @{ "cell" = -2006; "bin" = -2007; "legacy" = -2005; "xlsx" = 51 }
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    foreach ($f in Get-ChildItem $InDir -File -Filter $Filter) {
        $out = Join-Path $f.DirectoryName ([IO.Path]::GetFileNameWithoutExtension($f.Name) + $Suffix)
        try {
            $wb = $app.Workbooks.Open($f.FullName)
            # 바뀐 것이 없으면 한셀은 SaveAs 에서 파일을 그대로 복사한다 (관측) — 다시 쓰게 만든다
            try { $wb.Saved = $false } catch {}
            if (Test-Path $out) { Remove-Item $out -Force }
            $wb.SaveAs($out, $codes[$Format])
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
}
