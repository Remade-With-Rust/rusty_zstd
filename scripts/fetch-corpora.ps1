# Fetch Silesia into corpora/data/silesia/ and record SHA-256.
# Generated corpora are created by rzstd-bench; this script is optional.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Data = Join-Path $Root "corpora\data"
$Url = if ($env:SILESIA_URL) { $env:SILESIA_URL } else { "http://sun.aei.polsl.pl/~sdeor/corpus/silesia.zip" }
New-Item -ItemType Directory -Force -Path $Data | Out-Null
$Zip = Join-Path $Data "silesia.zip"
if (-not (Test-Path $Zip)) {
    Write-Host "downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Zip -UseBasicParsing
}
$hash = (Get-FileHash -Algorithm SHA256 $Zip).Hash.ToLower()
Set-Content -Path (Join-Path $Data "SHA256SUMS") -Value "$hash  silesia.zip"
$Dest = Join-Path $Data "silesia"
if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
Expand-Archive -Path $Zip -DestinationPath $Dest -Force
Write-Host "silesia sha256=$hash"
Write-Host "extracted $Dest"
