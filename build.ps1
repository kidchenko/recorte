<#
.SYNOPSIS
  Builds recorte.exe and, if Inno Setup is installed, the installer.

.EXAMPLE
  .\build.ps1                  # test + release binary + installer
  .\build.ps1 -Run             # ... then launch it
  .\build.ps1 -Icons           # ... then redraw assets\icon.ico, the PNGs and docs\img
  .\build.ps1 -SkipTests -SkipInstaller
#>
[CmdletBinding()]
param(
  [switch]$Run,
  [switch]$Icons,
  [switch]$SkipTests,
  [switch]$SkipInstaller
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

function Step($m) { Write-Host "`n==> $m" -ForegroundColor Cyan }
function Ok($m)   { Write-Host "    $m" -ForegroundColor Green }
function Warn($m) { Write-Host "    $m" -ForegroundColor Yellow }

function Find($name, $fallbacks) {
  $c = Get-Command $name -ErrorAction SilentlyContinue
  if ($c) { return $c.Source }
  foreach ($f in $fallbacks) { if (Test-Path $f) { return $f } }
  return $null
}

# [package].version specifically: a bare '^version =' would also match a
# dependency table that happens to come first one day.
function Get-PackageVersion {
  $inPackage = $false
  foreach ($line in Get-Content (Join-Path $PSScriptRoot 'Cargo.toml')) {
    if ($line -match '^\s*\[(.+)\]\s*$') { $inPackage = ($Matches[1] -eq 'package'); continue }
    if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') { return $Matches[1] }
  }
  throw 'could not read [package].version from Cargo.toml'
}

$cargo = Find 'cargo' @("$env:USERPROFILE\.cargo\bin\cargo.exe")
if (-not $cargo) {
  Write-Host 'cargo not found. Install Rust: winget install Rustlang.Rustup' -ForegroundColor Red
  exit 1
}

# A running instance locks the exe and the link fails with "Access is denied".
$running = Get-Process recorte -ErrorAction SilentlyContinue
if ($running) {
  Warn "stopping $($running.Count) running instance(s) first"
  $running | Stop-Process -Force
  Start-Sleep -Milliseconds 500
}

if (-not $SkipTests) {
  Step 'Running tests'
  & $cargo test --quiet
  if ($LASTEXITCODE -ne 0) { throw "cargo test failed ($LASTEXITCODE)" }
  Ok 'tests passed'
}

Step 'Building release binary'
& $cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
$exe = Join-Path $PSScriptRoot 'target\release\recorte.exe'
Ok ("recorte.exe   {0:N2} MB" -f ((Get-Item $exe).Length / 1MB))
$version = Get-PackageVersion

if ($Icons) {
  Step 'Regenerating icons'
  # The app draws its own icons and the site's pictures; no other tool needed.
  & $exe --dump-icons (Join-Path $PSScriptRoot 'assets')
  if ($LASTEXITCODE -ne 0) { throw "--dump-icons failed ($LASTEXITCODE)" }
  & $exe --render-docs (Join-Path $PSScriptRoot 'docs\img')
  if ($LASTEXITCODE -ne 0) { throw "--render-docs failed ($LASTEXITCODE)" }
  # The .ico is embedded at compile time, so the exe has to be rebuilt.
  & $cargo build --release
  Ok 'assets\icon.ico, icon-256.png, icon-1024.png, docs\img'
}

New-Item -ItemType Directory -Force (Join-Path $PSScriptRoot 'dist') | Out-Null
Copy-Item $exe (Join-Path $PSScriptRoot "dist\recorte-$version-x64.exe") -Force
Ok "dist\recorte-$version-x64.exe (portable)"

if (-not $SkipInstaller) {
  $iscc = Find 'iscc' @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
  )
  if (-not $iscc) {
    Warn 'Inno Setup not found; skipping the installer.'
    Warn '  winget install JRSoftware.InnoSetup'
  } else {
    Step 'Building installer'
    & $iscc /Q "/DAppVersion=$version" (Join-Path $PSScriptRoot 'packaging\inno\recorte.iss')
    if ($LASTEXITCODE -ne 0) { throw "iscc failed ($LASTEXITCODE)" }
    $setup = Join-Path $PSScriptRoot "dist\Recorte-Setup-$version.exe"
    Ok ("Recorte-Setup-$version.exe   {0:N2} MB" -f ((Get-Item $setup).Length / 1MB))
    Ok ("sha256  " + (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower())
    # Same name every release, so /releases/latest/download/Recorte-Setup.exe works.
    Copy-Item $setup (Join-Path $PSScriptRoot 'dist\Recorte-Setup.exe') -Force
  }
}

if ($Run) {
  Step 'Launching'
  Start-Process $exe
  Ok 'running in the tray'
}
