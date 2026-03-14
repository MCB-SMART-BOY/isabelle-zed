[CmdletBinding()]
param(
  [string]$KeymapPath = $env:ISABELLE_ZED_KEYMAP_PATH
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$startMarker = "// >>> isabelle shortcuts >>>"
$endMarker = "// <<< isabelle shortcuts <<<"

$taskCheck = "isabelle: check current theory (process_theories)"
$taskBuild = "isabelle: build worktree session (build -D)"

$checkCandidates = @("f8", "alt-shift-i", "f6")
$buildCandidates = @("f9", "alt-shift-b", "f10")
$rerunCandidates = @("f7", "alt-i", "f11")

$defaultReservedKeys = @(
  "ctrl-alt-i",
  "ctrl-alt-j",
  "ctrl-alt-k",
  "ctrl-alt-l"
)

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

function Looks-LikeBindingKey {
  param([string]$Name)

  $value = $Name.Trim().ToLowerInvariant()
  if (-not $value) {
    return $false
  }
  if ($value.StartsWith("f") -and $value.Substring(1) -match '^[0-9]+$') {
    return $true
  }
  return $value.Contains("-") -or $value.Contains("+") -or $value.Contains(" ")
}

function Extract-UsedBindingKeys {
  param([string]$Text)

  $used = New-Object System.Collections.Generic.HashSet[string]
  foreach ($line in $Text.Split("`n")) {
    $match = [regex]::Match($line, '^\s*"([^"]+)"\s*:\s*(?:\[|")')
    if (-not $match.Success) {
      continue
    }
    $key = $match.Groups[1].Value.Trim().ToLowerInvariant()
    if (Looks-LikeBindingKey $key) {
      $used.Add($key) | Out-Null
    }
  }
  return $used
}

function Parse-ReservedKeys {
  $reserved = New-Object System.Collections.Generic.HashSet[string]
  foreach ($key in $defaultReservedKeys) {
    $reserved.Add($key) | Out-Null
  }

  $extra = $env:ISABELLE_ZED_RESERVED_KEYS
  if ($extra -and $extra.Trim()) {
    foreach ($token in $extra.Split(",")) {
      $value = $token.Trim().ToLowerInvariant()
      if ($value) {
        $reserved.Add($value) | Out-Null
      }
    }
  }
  return $reserved
}

function Choose-Keys {
  param(
    [string[]]$Candidates,
    [System.Collections.Generic.HashSet[string]]$Used,
    [System.Collections.Generic.HashSet[string]]$Reserved,
    [int]$Limit
  )

  $chosen = @()
  foreach ($candidate in $Candidates) {
    $key = $candidate.ToLowerInvariant()
    if ($Used.Contains($key)) {
      continue
    }
    if ($Reserved.Contains($key)) {
      continue
    }
    if ($chosen -contains $key) {
      continue
    }
    $chosen += $key
    if ($chosen.Count -ge $Limit) {
      break
    }
  }
  return $chosen
}

function Spawn-Binding {
  param([string]$TaskName)
  return "[\n        \"task::Spawn\",\n        {\n          \"task_name\": \"$TaskName\",\n          \"reveal_target\": \"dock\"\n        }\n      ]"
}

function Rerun-Binding {
  return "[\"task::Rerun\", { \"reevaluate_context\": true }]"
}

function Build-Block {
  param([System.Collections.IDictionary]$Bindings)

  $lines = @(
    "  $startMarker",
    "  {",
    "    \"context\": \"Workspace\",",
    "    \"bindings\": {"
  )

  $items = @($Bindings.GetEnumerator())
  for ($i = 0; $i -lt $items.Count; $i++) {
    $item = $items[$i]
    $comma = if ($i -lt ($items.Count - 1)) { "," } else { "" }
    $lines += "      \"$($item.Key)\": $($item.Value)$comma"
  }

  $lines += @(
    "    }",
    "  }",
    "  $endMarker"
  )

  return ($lines -join "`n")
}

$keymapPath = Resolve-KeymapPath $KeymapPath
$keymapDir = Split-Path $keymapPath -Parent
if ($keymapDir) {
  New-Item -ItemType Directory -Path $keymapDir -Force | Out-Null
}

if (Test-Path $keymapPath) {
  $text = Get-Content -Path $keymapPath -Raw -Encoding UTF8
} else {
  $text = "[\n]\n"
}

$text = Strip-ExistingBlock $text
$usedKeys = Extract-UsedBindingKeys $text
$reservedKeys = Parse-ReservedKeys

$checkKeys = Choose-Keys -Candidates $checkCandidates -Used $usedKeys -Reserved $reservedKeys -Limit 2
foreach ($key in $checkKeys) {
  $usedKeys.Add($key) | Out-Null
}
$buildKeys = Choose-Keys -Candidates $buildCandidates -Used $usedKeys -Reserved $reservedKeys -Limit 2
foreach ($key in $buildKeys) {
  $usedKeys.Add($key) | Out-Null
}
$rerunKeys = Choose-Keys -Candidates $rerunCandidates -Used $usedKeys -Reserved $reservedKeys -Limit 2

$bindings = [ordered]@{}
foreach ($key in $checkKeys) {
  $bindings[$key] = Spawn-Binding $taskCheck
}
foreach ($key in $buildKeys) {
  $bindings[$key] = Spawn-Binding $taskBuild
}
foreach ($key in $rerunKeys) {
  $bindings[$key] = Rerun-Binding
}

if ($bindings.Count -eq 0) {
  throw "No non-conflicting shortcut candidates available. Set ISABELLE_ZED_RESERVED_KEYS to customize exclusions."
}

$block = Build-Block $bindings
$closingIndex = $text.LastIndexOf("]")
if ($closingIndex -lt 0) {
  throw "Keymap file is not an array (missing closing ']'): $keymapPath"
}

$before = $text.Substring(0, $closingIndex).TrimEnd()
$after = $text.Substring($closingIndex)

if ($before.EndsWith("[")) {
  $newText = $before + "`n" + $block + "`n" + $after.TrimStart()
} else {
  if (-not $before.EndsWith(",")) {
    $before += ","
  }
  $newText = $before + "`n" + $block + "`n" + $after.TrimStart()
}

if (-not $newText.EndsWith("`n")) {
  $newText += "`n"
}

$utf8NoBom = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($keymapPath, $newText, $utf8NoBom)

Write-Host "Installed Isabelle shortcuts into keymap: $keymapPath"
Write-Host "Selected Isabelle key bindings:"
foreach ($key in $bindings.Keys) {
  $action = if ($checkKeys -contains $key) { $taskCheck } elseif ($buildKeys -contains $key) { $taskBuild } else { "task::Rerun" }
  Write-Host "  $key -> $action"
}
