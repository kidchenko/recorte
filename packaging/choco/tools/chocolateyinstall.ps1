$ErrorActionPreference = 'Stop'

$version = '0.0.1'
$url     = "https://github.com/kidchenko/recorte/releases/download/v$version/Recorte-Setup-$version.exe"

$packageArgs = @{
  packageName    = 'recorte'
  fileType       = 'EXE'
  url64bit       = $url
  # The release workflow rewrites the version and this checksum from the
  # artifact it just built.
  checksum64     = 'REPLACE_WITH_SHA256'
  checksumType64 = 'sha256'
  # Inno Setup silent switches. The autostart task stays at its default (on).
  silentArgs     = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-'
  validExitCodes = @(0)
  softwareName   = 'Recorte*'
}

Install-ChocolateyPackage @packageArgs
