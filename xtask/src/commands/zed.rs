use crate::commands::build::release_build;
use crate::common::{
    bridge_binary_path, command_exists, copy_dir_recursive, copy_executable,
    ensure_wasm_target_installed, extension_id, extension_version, extension_wasm_path, home_dir,
    lsp_binary_path, run_command, validate_extension_id,
};
use anyhow::{Context, Result, anyhow, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tar::Builder;

const SHORTCUT_START_MARKER: &str = "// >>> isabelle shortcuts >>>";
const SHORTCUT_END_MARKER: &str = "// <<< isabelle shortcuts <<<";
const TASK_CHECK: &str = "isabelle: check current theory (process_theories)";
const TASK_BUILD: &str = "isabelle: build worktree session (build -D)";
const CHECK_CANDIDATES: &[&str] = &["f8", "alt-shift-i", "f6"];
const BUILD_CANDIDATES: &[&str] = &["f9", "alt-shift-b", "f10"];
const RERUN_CANDIDATES: &[&str] = &["f7", "alt-i", "f11"];
const DEFAULT_RESERVED_KEYS: &[&str] = &["ctrl-alt-i", "ctrl-alt-j", "ctrl-alt-k", "ctrl-alt-l"];

fn extensions_dir() -> Result<PathBuf> {
    if let Ok(explicit) = env::var("ISABELLE_ZED_EXTENSIONS_DIR") {
        return Ok(PathBuf::from(explicit));
    }

    let home = home_dir()?;
    match env::consts::OS {
        "linux" => Ok(home.join(".local/share/zed/extensions/installed")),
        "macos" => Ok(home.join("Library/Application Support/Zed/extensions/installed")),
        "windows" => {
            if let Ok(local) = env::var("LOCALAPPDATA") {
                Ok(PathBuf::from(local).join("Zed/extensions/installed"))
            } else {
                bail!("LOCALAPPDATA is not set; set ISABELLE_ZED_EXTENSIONS_DIR manually")
            }
        }
        other => bail!("unsupported platform for automatic Zed extension install: {other}"),
    }
}

pub(crate) fn install_native(repo_root: &Path) -> Result<()> {
    if !command_exists("cargo") {
        bail!("cargo is required");
    }
    if !command_exists("rustup") {
        bail!("rustup is required");
    }
    ensure_wasm_target_installed()?;

    let extension_id = extension_id(repo_root)?;
    validate_extension_id(&extension_id)?;
    let extensions_dir = extensions_dir()?;

    println!("Building extension wasm (release)...");
    run_command(
        Command::new("cargo")
            .arg("build")
            .arg("-p")
            .arg("isabelle-zed-extension")
            .arg("--target")
            .arg("wasm32-wasip2")
            .arg("--release"),
    )?;

    let wasm_src = extension_wasm_path(repo_root, "release");
    if !wasm_src.is_file() {
        bail!("extension wasm artifact not found: {}", wasm_src.display());
    }

    let grammar_src = repo_root.join("zed-extension/grammars/isabelle.wasm");
    if !grammar_src.is_file() {
        bail!(
            "missing grammar artifact: {} (run: make build-isabelle-grammar)",
            grammar_src.display()
        );
    }

    fs::create_dir_all(&extensions_dir).with_context(|| {
        format!(
            "failed to create extensions dir: {}",
            extensions_dir.display()
        )
    })?;
    let dest_dir = extensions_dir.join(&extension_id);
    if dest_dir.exists() {
        fs::remove_dir_all(&dest_dir).with_context(|| {
            format!("failed to remove old extension dir: {}", dest_dir.display())
        })?;
    }

    let legacy_dir = extensions_dir.join("isabelle-zed");
    if legacy_dir != dest_dir && legacy_dir.exists() {
        fs::remove_dir_all(&legacy_dir).with_context(|| {
            format!(
                "failed to remove legacy extension dir: {}",
                legacy_dir.display()
            )
        })?;
    }

    fs::create_dir_all(&dest_dir)
        .with_context(|| format!("failed to create extension dir: {}", dest_dir.display()))?;
    fs::copy(
        repo_root.join("zed-extension/extension.toml"),
        dest_dir.join("extension.toml"),
    )?;
    fs::copy(&wasm_src, dest_dir.join("extension.wasm"))?;
    copy_dir_recursive(
        &repo_root.join("zed-extension/languages"),
        &dest_dir.join("languages"),
    )?;
    copy_dir_recursive(
        &repo_root.join("zed-extension/grammars"),
        &dest_dir.join("grammars"),
    )?;

    println!("Zed extension installed to: {}", dest_dir.display());
    if command_exists("isabelle") {
        println!("isabelle command detected: native mode is ready.");
    } else {
        println!(
            "warning: 'isabelle' not found in PATH. native mode will not start until PATH is fixed."
        );
    }

    if env::var("ISABELLE_ZED_SKIP_SHORTCUTS").ok().as_deref() != Some("1") {
        install_shortcuts()?;
    }

    println!("Restart Zed (or reload extensions) and open a .thy file.");
    Ok(())
}

pub(crate) fn uninstall_native(repo_root: &Path) -> Result<()> {
    let extension_id = extension_id(repo_root)?;
    let extensions_dir = extensions_dir()?;

    let mut removed_any = false;
    for candidate in [&extension_id, "isabelle-zed"] {
        let dir = extensions_dir.join(candidate);
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("failed to remove extension dir: {}", dir.display()))?;
            println!("Removed Zed extension: {}", dir.display());
            removed_any = true;
        }
    }

    if !removed_any {
        println!(
            "extension is not installed in: {}",
            extensions_dir.display()
        );
    }

    if env::var("ISABELLE_ZED_SKIP_SHORTCUTS").ok().as_deref() != Some("1") {
        uninstall_shortcuts()?;
    }

    println!("Restart Zed (or reload extensions) to apply changes.");
    Ok(())
}

fn keymap_path() -> Result<PathBuf> {
    if let Ok(explicit) = env::var("ISABELLE_ZED_KEYMAP_PATH") {
        return Ok(PathBuf::from(explicit));
    }

    let home = home_dir()?;
    match env::consts::OS {
        "linux" => Ok(home.join(".config/zed/keymap.json")),
        "macos" => Ok(home.join("Library/Application Support/Zed/keymap.json")),
        "windows" => {
            if let Ok(local) = env::var("LOCALAPPDATA") {
                return Ok(PathBuf::from(local).join("Zed/keymap.json"));
            }
            if let Ok(appdata) = env::var("APPDATA") {
                return Ok(PathBuf::from(appdata).join("Zed/keymap.json"));
            }
            bail!(
                "unsupported platform for automatic keymap install: set ISABELLE_ZED_KEYMAP_PATH manually"
            )
        }
        other => bail!("unsupported platform for automatic keymap install: {other}"),
    }
}

fn strip_shortcut_block(text: &str) -> Result<String> {
    let pattern = Regex::new(&format!(
        r"(?s)\n?\s*{}.*?{}\s*,?\n?",
        regex::escape(SHORTCUT_START_MARKER),
        regex::escape(SHORTCUT_END_MARKER)
    ))?;
    Ok(pattern.replace_all(text, "\n").to_string())
}

fn extract_used_binding_keys(text: &str) -> Result<HashSet<String>> {
    let mut used = HashSet::new();
    let key_pattern = Regex::new(r#"^\s*"([^"]+)"\s*:\s*(?:\[|")"#)?;
    for line in text.lines() {
        let Some(captures) = key_pattern.captures(line) else {
            continue;
        };
        let key = captures
            .get(1)
            .map(|value| value.as_str().trim().to_lowercase())
            .unwrap_or_default();
        if looks_like_binding_key(&key) {
            used.insert(key);
        }
    }
    Ok(used)
}

fn looks_like_binding_key(value: &str) -> bool {
    let trimmed = value.trim().to_lowercase();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix('f')
        && !rest.is_empty()
        && rest.chars().all(|ch| ch.is_ascii_digit())
    {
        return true;
    }
    trimmed.contains('-') || trimmed.contains('+') || trimmed.contains(' ')
}

fn parse_reserved_keys() -> HashSet<String> {
    let mut reserved = DEFAULT_RESERVED_KEYS
        .iter()
        .map(|key| key.to_string())
        .collect::<HashSet<_>>();
    if let Ok(extra) = env::var("ISABELLE_ZED_RESERVED_KEYS") {
        for token in extra.split(',') {
            let key = token.trim().to_lowercase();
            if !key.is_empty() {
                reserved.insert(key);
            }
        }
    }
    reserved
}

fn choose_keys(
    candidates: &[&str],
    used: &mut HashSet<String>,
    reserved: &HashSet<String>,
    limit: usize,
) -> Vec<String> {
    let mut chosen = Vec::new();
    for candidate in candidates {
        let key = candidate.to_lowercase();
        if used.contains(&key) || reserved.contains(&key) || chosen.contains(&key) {
            continue;
        }
        used.insert(key.clone());
        chosen.push(key);
        if chosen.len() >= limit {
            break;
        }
    }
    chosen
}

fn spawn_binding(task_name: &str) -> String {
    format!(
        "[\n        \"task::Spawn\",\n        {{\n          \"task_name\": \"{task_name}\",\n          \"reveal_target\": \"dock\"\n        }}\n      ]"
    )
}

fn rerun_binding() -> String {
    "[\"task::Rerun\", { \"reevaluate_context\": true }]".to_string()
}

fn build_shortcut_block(bindings: &BTreeMap<String, String>) -> String {
    let mut lines = vec![
        "  // >>> isabelle shortcuts >>>".to_string(),
        "  {".to_string(),
        "    \"context\": \"Workspace\",".to_string(),
        "    \"bindings\": {".to_string(),
    ];

    let len = bindings.len();
    for (index, (key, value)) in bindings.iter().enumerate() {
        let comma = if index + 1 < len { "," } else { "" };
        lines.push(format!("      \"{key}\": {value}{comma}"));
    }

    lines.push("    }".to_string());
    lines.push("  }".to_string());
    lines.push("  // <<< isabelle shortcuts <<<".to_string());
    lines.join("\n")
}

pub(crate) fn install_shortcuts() -> Result<()> {
    let path = keymap_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create keymap parent dir: {}", parent.display()))?;
    }

    let existing = if path.is_file() {
        fs::read_to_string(&path)
            .with_context(|| format!("failed to read keymap file: {}", path.display()))?
    } else {
        "[\n]\n".to_string()
    };

    let text = strip_shortcut_block(&existing)?;
    let mut used = extract_used_binding_keys(&text)?;
    let reserved = parse_reserved_keys();

    let check_keys = choose_keys(CHECK_CANDIDATES, &mut used, &reserved, 2);
    let build_keys = choose_keys(BUILD_CANDIDATES, &mut used, &reserved, 2);
    let rerun_keys = choose_keys(RERUN_CANDIDATES, &mut used, &reserved, 2);

    let mut bindings = BTreeMap::new();
    for key in &check_keys {
        bindings.insert(key.clone(), spawn_binding(TASK_CHECK));
    }
    for key in &build_keys {
        bindings.insert(key.clone(), spawn_binding(TASK_BUILD));
    }
    for key in &rerun_keys {
        bindings.insert(key.clone(), rerun_binding());
    }

    if bindings.is_empty() {
        bail!(
            "No non-conflicting shortcut candidates available. Set ISABELLE_ZED_RESERVED_KEYS to customize exclusions."
        );
    }

    let block = build_shortcut_block(&bindings);
    let closing_index = text.rfind(']').ok_or_else(|| {
        anyhow!(
            "keymap file is not an array (missing closing ']'): {}",
            path.display()
        )
    })?;

    let mut before = text[..closing_index].trim_end().to_string();
    let after = text[closing_index..].trim_start().to_string();
    let new_text = if before.ends_with('[') {
        format!("{before}\n{block}\n{after}")
    } else {
        if !before.ends_with(',') {
            before.push(',');
        }
        format!("{before}\n{block}\n{after}")
    };

    let mut final_text = new_text;
    if !final_text.ends_with('\n') {
        final_text.push('\n');
    }
    fs::write(&path, final_text)
        .with_context(|| format!("failed to write keymap file: {}", path.display()))?;

    println!(
        "Installed Isabelle shortcuts into keymap: {}",
        path.display()
    );
    println!("Selected Isabelle key bindings:");
    for key in bindings.keys() {
        let action = if check_keys.contains(key) {
            TASK_CHECK
        } else if build_keys.contains(key) {
            TASK_BUILD
        } else {
            "task::Rerun"
        };
        println!("  {key} -> {action}");
    }

    Ok(())
}

pub(crate) fn uninstall_shortcuts() -> Result<()> {
    let path = keymap_path()?;
    if !path.is_file() {
        println!("keymap file does not exist: {}", path.display());
        return Ok(());
    }

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read keymap file: {}", path.display()))?;
    let pattern = Regex::new(&format!(
        r"(?s)\n?\s*{}.*?{}\s*,?\n?",
        regex::escape(SHORTCUT_START_MARKER),
        regex::escape(SHORTCUT_END_MARKER)
    ))?;
    let count = pattern.find_iter(&text).count();
    if count == 0 {
        println!(
            "Isabelle shortcuts were not found in keymap: {}",
            path.display()
        );
        return Ok(());
    }

    let mut new_text = pattern.replace_all(&text, "\n").to_string();
    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    fs::write(&path, new_text)
        .with_context(|| format!("failed to write keymap file: {}", path.display()))?;
    println!("Removed Isabelle shortcuts from keymap: {}", path.display());
    Ok(())
}

pub(crate) fn official_check(repo_root: &Path) -> Result<()> {
    let manifest = repo_root.join("zed-extension/extension.toml");
    if !manifest.is_file() {
        bail!("missing manifest: {}", manifest.display());
    }

    let extension_id = extension_id(repo_root)?;
    let version = extension_version(repo_root)?;
    validate_extension_id(&extension_id)?;

    for license_file in [
        repo_root.join("LICENSE"),
        repo_root.join("zed-extension/LICENSE"),
    ] {
        if !license_file.is_file() {
            bail!("missing license file: {}", license_file.display());
        }
        let content = fs::read_to_string(&license_file)
            .with_context(|| format!("failed to read license file: {}", license_file.display()))?;
        if !license_looks_accepted(&content) {
            bail!(
                "license file does not appear to match an accepted license: {}",
                license_file.display()
            );
        }
    }

    let remote_url =
        "https://raw.githubusercontent.com/zed-industries/extensions/main/extensions.toml";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    match client.get(remote_url).send() {
        Ok(response) if response.status().is_success() => {
            let body = response
                .text()
                .context("failed to read official extensions.toml")?;
            if body.contains(&format!("[{extension_id}]")) {
                bail!("extension id '{extension_id}' already exists in zed-industries/extensions");
            }
            println!(
                "[ok] extension id '{extension_id}' is not currently listed in official registry"
            );
        }
        Ok(response) => {
            println!(
                "[warn] could not fetch official extensions.toml for duplicate ID check (status={})",
                response.status()
            );
        }
        Err(err) => {
            println!(
                "[warn] could not fetch official extensions.toml for duplicate ID check: {err}"
            );
        }
    }

    println!("[ok] manifest id/version format passes");
    println!("[ok] required license files present");
    println!();
    println!("Suggested entry for zed-industries/extensions/extensions.toml:");
    println!("[{extension_id}]");
    println!("submodule = \"extensions/{extension_id}\"");
    println!("path = \"zed-extension\"");
    println!("version = \"{version}\"");
    Ok(())
}

fn license_looks_accepted(content: &str) -> bool {
    let lower = content.to_lowercase();
    [
        "mit license",
        "apache license",
        "bsd",
        "gnu general public license",
        "gnu lesser general public license",
        "zlib",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn default_platform_label() -> String {
    let os = match env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = env::consts::ARCH;
    format!("{os}-{arch}")
}

pub(crate) fn release_package(repo_root: &Path, platform: Option<String>) -> Result<()> {
    let platform = platform.unwrap_or_else(default_platform_label);
    let dist_dir = repo_root.join("dist");
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create dist dir: {}", dist_dir.display()))?;

    for file in [
        repo_root.join("README.md"),
        repo_root.join("CHANGELOG.md"),
        repo_root.join("LICENSE"),
        repo_root.join("zed-extension/extension.toml"),
        repo_root.join("zed-extension/README.md"),
        repo_root.join("zed-extension/Cargo.toml"),
    ] {
        if !file.is_file() {
            bail!("missing required file: {}", file.display());
        }
    }
    for dir in [
        repo_root.join("zed-extension/languages"),
        repo_root.join("examples"),
    ] {
        if !dir.is_dir() {
            bail!("missing required directory: {}", dir.display());
        }
    }

    let version = extension_version(repo_root)?;
    release_build(repo_root)?;

    let package_root = format!("isabelle-zed-v{version}-{platform}");
    let package_dir = dist_dir.join(&package_root);
    if package_dir.exists() {
        fs::remove_dir_all(&package_dir).with_context(|| {
            format!(
                "failed to remove old package directory: {}",
                package_dir.display()
            )
        })?;
    }
    fs::create_dir_all(package_dir.join("bin"))?;
    fs::create_dir_all(package_dir.join("zed-extension"))?;
    fs::create_dir_all(package_dir.join("examples"))?;
    fs::create_dir_all(package_dir.join("docs"))?;

    copy_executable(
        &bridge_binary_path(repo_root, "release"),
        &package_dir.join("bin/bridge"),
    )?;
    copy_executable(
        &lsp_binary_path(repo_root, "release"),
        &package_dir.join("bin/isabelle-zed-lsp"),
    )?;

    fs::copy(
        repo_root.join("zed-extension/extension.toml"),
        package_dir.join("zed-extension/extension.toml"),
    )?;
    fs::copy(
        repo_root.join("zed-extension/Cargo.toml"),
        package_dir.join("zed-extension/Cargo.toml"),
    )?;
    fs::copy(
        repo_root.join("zed-extension/README.md"),
        package_dir.join("zed-extension/README.md"),
    )?;
    copy_dir_recursive(
        &repo_root.join("zed-extension/src"),
        &package_dir.join("zed-extension/src"),
    )?;
    copy_dir_recursive(
        &repo_root.join("zed-extension/languages"),
        &package_dir.join("zed-extension/languages"),
    )?;

    let grammar_src = repo_root.join("zed-extension/grammars/isabelle.wasm");
    if !grammar_src.is_file() {
        bail!(
            "missing grammar artifact: {} (run: make build-isabelle-grammar)",
            grammar_src.display()
        );
    }
    copy_dir_recursive(
        &repo_root.join("zed-extension/grammars"),
        &package_dir.join("zed-extension/grammars"),
    )?;
    fs::copy(
        extension_wasm_path(repo_root, "release"),
        package_dir.join("zed-extension/extension.wasm"),
    )?;

    for file in [
        "zed-settings-native.json",
        "zed-settings-bridge-mock.json",
        "zed-keymap-isabelle.json",
    ] {
        fs::copy(
            repo_root.join("examples").join(file),
            package_dir.join("examples").join(file),
        )?;
    }

    fs::copy(
        repo_root.join("README.md"),
        package_dir.join("docs/README.md"),
    )?;
    fs::copy(
        repo_root.join("CHANGELOG.md"),
        package_dir.join("docs/CHANGELOG.md"),
    )?;
    fs::copy(repo_root.join("LICENSE"), package_dir.join("LICENSE"))?;

    let archive_path = dist_dir.join(format!("{package_root}.tar.gz"));
    if archive_path.exists() {
        fs::remove_file(&archive_path)
            .with_context(|| format!("failed to remove old archive: {}", archive_path.display()))?;
    }

    let archive_file = fs::File::create(&archive_path)
        .with_context(|| format!("failed to create archive: {}", archive_path.display()))?;
    let encoder = GzEncoder::new(archive_file, Compression::default());
    let mut tar = Builder::new(encoder);
    tar.append_dir_all(&package_root, &package_dir)
        .with_context(|| {
            format!(
                "failed to append package dir to tar: {}",
                package_dir.display()
            )
        })?;
    let encoder = tar.into_inner().context("failed to finalize tar stream")?;
    encoder.finish().context("failed to finish gzip stream")?;

    let archive_bytes = fs::read(&archive_path).with_context(|| {
        format!(
            "failed to read archive for checksum: {}",
            archive_path.display()
        )
    })?;
    let checksum = Sha256::digest(&archive_bytes);
    let checksum_hex = format!("{checksum:x}");
    let archive_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid archive name: {}", archive_path.display()))?;
    let checksum_path = dist_dir.join(format!("{archive_name}.sha256"));
    fs::write(&checksum_path, format!("{checksum_hex}  {archive_name}\n"))
        .with_context(|| format!("failed to write checksum file: {}", checksum_path.display()))?;

    println!("Release package created:");
    println!("  {}", archive_path.display());
    println!("  {}", checksum_path.display());
    Ok(())
}
