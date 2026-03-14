[CmdletBinding()]
param(
  [string]$KeymapPath = $env:ISABELLE_ZED_KEYMAP_PATH
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$startMarker = "// >>> isabelle shortcuts >>>"
$endMarker = "// <<< isabelle shortcuts <<<"

function Resolve-KeymapPath {
  param([string]$ExplicitPath)

  if ($ExplicitPath) {
    return $ExplicitPath
  }

  $candidates = @()
  if ($env:LOCALAPPDATA) {
    $candidates += (Join-Path $env:LOCALAPPDATA "Zed\keymap.json")
  }
  if ($env:APPDATA) {
    $candidates += (Join-Path $env:APPDATA "Zed\keymap.json")
  }

  foreach ($candidate in $candidates) {
    if (Test-Path $candidate) {
      return $candidate
    }
  }

  if ($candidates.Count -gt 0) {
    return $candidates[0]
  }

  throw "Unsupported platform: set ISABELLE_ZED_KEYMAP_PATH manually."
}

function Strip-ExistingBlock {
  param([string]$Text)

  $pattern = "(?s)\n?\s*" + [regex]::Escape($startMarker) + ".*?" + [regex]::Escape($endMarker) + "\s*,?\n?"
  return [regex]::Replace($Text, $pattern, "`n")
}

$keymapPath = Resolve-KeymapPath $KeymapPath
if (-not (Test-Path $keymapPath)) {
  Write-Host "No keymap found at: $keymapPath"
  return
}

$text = Get-Content -Path $keymapPath -Raw -Encoding UTF8
$text = Strip-ExistingBlock $text

$utf8NoBom = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($keymapPath, $text, $utf8NoBom)

Write-Host "Removed Isabelle shortcuts from keymap: $keymapPath"
