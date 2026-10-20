# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 일괄 차분 분석 샘플 생성: 폴더의 문서(.xlsx/.cell 등)를 한셀로 열어
#   <이름>.cell / <이름>.bin.cell / <이름>.legacy.cell 로 저장하고,
#   레거시 파일을 한셀로 다시 열어 <이름>.legacy.as.cell (한셀의 해석 = 정답) 로 저장한다.
# 한셀 하나를 계속 써서 빠르다. 결과는 저장소 밖에 둔다.
#
# 사용: powershell -File batch-formats.ps1 -InDir <원본 폴더> -OutDir <출력 폴더>
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param(
    [Parameter(Mandatory = $true)][string]$InDir,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$Filter = "*.xlsx"
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $OutDir | Out-Null
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    foreach ($f in Get-ChildItem $InDir -Recurse -File -Filter $Filter) {
        $base = Join-Path $OutDir ([IO.Path]::GetFileNameWithoutExtension($f.Name) -replace '\s+', '_')
        try {
            $wb = $app.Workbooks.Open($f.FullName)
            # 바뀐 것이 없으면 한셀은 SaveAs 에서 같은 형식 파일을 그대로 복사한다 (관측) — 다시 쓰게 만든다
            try { $wb.Saved = $false } catch {}
            foreach ($t in @(@(".cell", -2006), @(".bin.cell", -2007), @(".legacy.cell", -2005))) {
                $p = $base + $t[0]
                if (Test-Path $p) { Remove-Item $p -Force }
                $wb.SaveAs($p, $t[1])
            }
            $wb.Close($false)
            $wb = $app.Workbooks.Open($base + ".legacy.cell")
            $p = $base + ".legacy.as.cell"
            if (Test-Path $p) { Remove-Item $p -Force }
            $wb.SaveAs($p, -2006)
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
