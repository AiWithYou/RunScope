$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

cargo build --release --locked
if ($LASTEXITCODE -ne 0) {
    throw "cargo build --release failed with exit code $LASTEXITCODE"
}

$distDir = "dist"
$exePath = Join-Path $distDir "RunScope.exe"
$zipPath = Join-Path $distDir "RunScope-windows-x64.zip"
$shaPath = Join-Path $distDir "SHA256SUMS.txt"
$packageDir = Join-Path $distDir "RunScope-windows-x64"

New-Item -ItemType Directory -Force -Path $distDir | Out-Null
Copy-Item -LiteralPath "target\release\runscope.exe" -Destination $exePath -Force

if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}
if (Test-Path -LiteralPath $shaPath) {
    Remove-Item -LiteralPath $shaPath -Force
}
if (Test-Path -LiteralPath $packageDir) {
    $distRoot = (Resolve-Path -LiteralPath $distDir).Path
    $packageRoot = (Resolve-Path -LiteralPath $packageDir).Path
    if (-not $packageRoot.StartsWith($distRoot)) {
        throw "Refusing to remove package directory outside dist: $packageRoot"
    }
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}

New-Item -ItemType Directory -Force -Path $packageDir | Out-Null
Copy-Item -LiteralPath $exePath -Destination (Join-Path $packageDir "RunScope.exe") -Force
Copy-Item -LiteralPath "README.md" -Destination (Join-Path $packageDir "README.md") -Force
Copy-Item -LiteralPath "README.ja.md" -Destination (Join-Path $packageDir "README.ja.md") -Force
Copy-Item -LiteralPath "LICENSE" -Destination (Join-Path $packageDir "LICENSE") -Force
Copy-Item -LiteralPath "settings.example.json" -Destination (Join-Path $packageDir "settings.example.json") -Force
New-Item -ItemType Directory -Force -Path (Join-Path $packageDir "docs") | Out-Null
Copy-Item -LiteralPath "docs\RECORDING.md" -Destination (Join-Path $packageDir "docs\RECORDING.md") -Force
Copy-Item -LiteralPath "CHANGELOG.md" -Destination (Join-Path $packageDir "CHANGELOG.md") -Force

New-Item -ItemType Directory -Force -Path (Join-Path $packageDir "scripts") | Out-Null
Copy-Item -LiteralPath "scripts\diagnostic_workload.py" -Destination (Join-Path $packageDir "scripts\diagnostic_workload.py") -Force
Copy-Item -LiteralPath "scripts\smoke_recording.py" -Destination (Join-Path $packageDir "scripts\smoke_recording.py") -Force

Compress-Archive -Path (Join-Path $packageDir "*") -DestinationPath $zipPath -Force
Remove-Item -LiteralPath $packageDir -Recurse -Force

$hashLines = foreach ($file in @($exePath, $zipPath)) {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant()
    $name = Split-Path -Leaf $file
    "$hash  $name"
}
Set-Content -LiteralPath $shaPath -Value $hashLines -Encoding ascii

Write-Host "Created $exePath"
Write-Host "Created $zipPath"
Write-Host "Created $shaPath"
