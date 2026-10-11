# Release build on Windows x86-64: NSIS (.exe) and MSI installers with pdfium.dll next to Blankey.exe, copied to dist\.
# Needs: rustup (x86_64-pc-windows-msvc), Visual Studio Build Tools, WebView2 (preinstalled on Windows 10/11),
# cargo install tauri-cli --version "^2", cargo install cargo-about, and Git for Windows (bash for licenses.sh and stage-ui.sh).
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Release = if ($env:PDFIUM_RELEASE) { $env:PDFIUM_RELEASE } else { "chromium/8086" }
Set-Location $Root

$PdfiumDir = Join-Path $Root "src-tauri\pdfium"
if (-not (Test-Path (Join-Path $PdfiumDir "pdfium.dll"))) {
    New-Item -ItemType Directory -Force -Path $PdfiumDir | Out-Null
    $Tmp = Join-Path $env:TEMP ("pdfium-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $Tmp | Out-Null
    $Archive = Join-Path $Tmp "pdfium.tgz"
    Invoke-WebRequest "https://github.com/bblanchon/pdfium-binaries/releases/download/$Release/pdfium-win-x64.tgz" -OutFile $Archive
    tar -xzf $Archive -C $Tmp
    Copy-Item (Join-Path $Tmp "bin\pdfium.dll") $PdfiumDir
    Copy-Item (Join-Path $Tmp "LICENSE") $PdfiumDir -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $Tmp
}

bash scripts/licenses.sh
if ($LASTEXITCODE -ne 0) { throw "licenses.sh failed" }
bash scripts/stage-ui.sh
if ($LASTEXITCODE -ne 0) { throw "stage-ui.sh failed" }

Set-Location (Join-Path $Root "src-tauri")
cargo tauri build --target x86_64-pc-windows-msvc --config tauri.release.conf.json @args
if ($LASTEXITCODE -ne 0) { throw "cargo tauri build failed" }

$Dist = Join-Path $Root "dist"
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
$Bundle = "target\x86_64-pc-windows-msvc\release\bundle"
Copy-Item "$Bundle\nsis\*.exe" $Dist
Copy-Item "$Bundle\msi\*.msi" $Dist
Copy-Item "THIRD_PARTY_LICENSES.html" $Dist
Get-ChildItem $Dist
