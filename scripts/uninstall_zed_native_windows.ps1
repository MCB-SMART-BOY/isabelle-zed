[CmdletBinding()]
param(
  [string]$ExtensionsDir = $env:ISABELLE_ZED_EXTENSIONS_DIR
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

$extensionToml = Join-Path $RepoRoot "zed-extension/extension.toml"
if (-not (Test-Path $extensionToml)) {
  throw "extension.toml not found: $extensionToml"
}

$match = Select-String -Path $extensionToml -Pattern '^\s*id\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $match) {
  throw "failed to read extension id from $extensionToml"
}
$extensionId = $match.Matches[0].Groups[1].Value
if (-not $extensionId) {
  throw "empty extension id in $extensionToml"
}

if (-not $ExtensionsDir) {
  if (-not $env:LOCALAPPDATA) {
    throw "LOCALAPPDATA not set; set ISABELLE_ZED_EXTENSIONS_DIR manually"
  }
  $ExtensionsDir = Join-Path $env:LOCALAPPDATA "Zed\extensions\installed"
}

$destDir = Join-Path $ExtensionsDir $extensionId

if (Test-Path $destDir) {
  Remove-Item -Recurse -Force $destDir
  Write-Host "Removed: $destDir"
} else {
  Write-Host "Nothing to remove at: $destDir"
}
