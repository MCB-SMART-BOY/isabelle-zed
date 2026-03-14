[CmdletBinding()]
param(
  [string]$ExtensionsDir = $env:ISABELLE_ZED_EXTENSIONS_DIR,
  [switch]$SkipShortcuts
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

function Require-Command([string]$Name) {
  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    throw "$Name is required"
  }
}

Require-Command cargo
Require-Command rustup

$installedTargets = & rustup target list --installed
if ($installedTargets -notmatch 'wasm32-wasip2') {
  Write-Host "Installing Rust target wasm32-wasip2..."
  & rustup target add wasm32-wasip2
}

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

Write-Host "Building extension wasm (release)..."
& cargo build --manifest-path (Join-Path $RepoRoot "zed-extension/Cargo.toml") --target wasm32-wasip2 --release

$wasmSrc = Join-Path $RepoRoot "zed-extension/target/wasm32-wasip2/release/isabelle_zed_extension.wasm"
if (-not (Test-Path $wasmSrc)) {
  throw "extension wasm artifact not found: $wasmSrc"
}

$grammarSrc = Join-Path $RepoRoot "zed-extension/grammars/isabelle.wasm"
if (-not (Test-Path $grammarSrc)) {
  throw "missing grammar artifact: $grammarSrc"
}

if (Test-Path $destDir) {
  Remove-Item -Recurse -Force $destDir
}

New-Item -ItemType Directory -Path $destDir -Force | Out-Null
Copy-Item $extensionToml (Join-Path $destDir "extension.toml")
Copy-Item $wasmSrc (Join-Path $destDir "extension.wasm")
Copy-Item (Join-Path $RepoRoot "zed-extension/languages") -Destination (Join-Path $destDir "languages") -Recurse
Copy-Item (Join-Path $RepoRoot "zed-extension/grammars") -Destination (Join-Path $destDir "grammars") -Recurse

Write-Host "Zed extension installed to: $destDir"

if (Get-Command isabelle -ErrorAction SilentlyContinue) {
  Write-Host "isabelle command detected: native mode is ready."
} else {
  Write-Warning "'isabelle' not found in PATH. native mode will not start until PATH is fixed."
}

if (-not $SkipShortcuts) {
  Write-Warning "Shortcut install is not implemented for Windows. Use Zed command palette to open keymap and copy examples/zed-keymap-isabelle.json manually."
}

Write-Host "Restart Zed (or reload extensions) and open a .thy file."
