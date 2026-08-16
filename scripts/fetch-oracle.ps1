# Fetch and verify the pinned facebook/zstd v1.5.7 Windows CLI.
# Usage: pwsh scripts/fetch-oracle.ps1
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root "third_party\zstd"
$Zip = Join-Path $Dest "zstd-v1.5.7-win64.zip"
$Url = "https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-v1.5.7-win64.zip"
$ZipSha = "acb4e8111511749dc7a3ebedca9b04190e37a17afeb73f55d4425dbf0b90fad9"
$ExeSha = "8076aae03feac7c66b319579e82172eed168deed2a3f25e5e2d3c60f55e84111"

New-Item -ItemType Directory -Force -Path $Dest | Out-Null
if (-not (Test-Path $Zip)) {
    Write-Host "downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Zip -UseBasicParsing
}
$got = (Get-FileHash -Algorithm SHA256 $Zip).Hash.ToLower()
if ($got -ne $ZipSha) {
    throw "zip sha256 mismatch: $got != $ZipSha"
}
$Extract = Join-Path $Dest "extracted"
if (Test-Path $Extract) { Remove-Item -Recurse -Force $Extract }
Expand-Archive -Path $Zip -DestinationPath $Extract -Force
$exe = Get-ChildItem -Path $Extract -Recurse -Filter zstd.exe | Select-Object -First 1
if (-not $exe) { throw "zstd.exe not found after extract" }
$exeGot = (Get-FileHash -Algorithm SHA256 $exe.FullName).Hash.ToLower()
if ($exeGot -ne $ExeSha) {
    throw "zstd.exe sha256 mismatch: $exeGot != $ExeSha"
}
& $exe.FullName --version
Write-Host "oracle ok: $($exe.FullName)"
Write-Host "set RUSTY_ZSTD_ORACLE=$($exe.FullName)"
