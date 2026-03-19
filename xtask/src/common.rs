use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn repo_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("failed to compute repository root from xtask manifest path"))
}

pub(crate) fn workspace_target_dir(repo_root: &Path) -> PathBuf {
    repo_root.join("target")
}

pub(crate) fn bridge_binary_path(repo_root: &Path, profile: &str) -> PathBuf {
    let mut name = "bridge".to_string();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    workspace_target_dir(repo_root).join(profile).join(name)
}

pub(crate) fn lsp_binary_path(repo_root: &Path, profile: &str) -> PathBuf {
    let mut name = "isabelle-zed-lsp".to_string();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    workspace_target_dir(repo_root).join(profile).join(name)
}

pub(crate) fn extension_wasm_path(repo_root: &Path, profile: &str) -> PathBuf {
    workspace_target_dir(repo_root)
        .join("wasm32-wasip2")
        .join(profile)
        .join("isabelle_zed_extension.wasm")
}

pub(crate) fn run_command(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to start command: {command:?}"))?;
    if !status.success() {
        bail!("command failed with status {status}: {command:?}");
    }
    Ok(())
}

pub(crate) fn command_output(command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("failed to start command: {command:?}"))?;
    if !output.status.success() {
        bail!(
            "command failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn command_exists(name: &str) -> bool {
    let path_var = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path_var).any(|entry| {
        let full = entry.join(name);
        if full.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            let full_exe = entry.join(format!("{name}.exe"));
            if full_exe.is_file() {
                return true;
            }
        }
        false
    })
}

pub(crate) fn home_dir() -> Result<PathBuf> {
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home));
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        return Ok(PathBuf::from(profile));
    }
    bail!("failed to resolve home directory from HOME/USERPROFILE")
}

pub(crate) fn read_extension_toml(repo_root: &Path) -> Result<toml::Value> {
    let path = repo_root.join("zed-extension/extension.toml");
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read extension manifest: {}", path.display()))?;
    raw.parse::<toml::Value>().with_context(|| {
        format!(
            "failed to parse extension manifest TOML: {}",
            path.display()
        )
    })
}

pub(crate) fn extension_id(repo_root: &Path) -> Result<String> {
    read_extension_toml(repo_root)?
        .get("id")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("failed to read `id` from zed-extension/extension.toml"))
}

pub(crate) fn extension_version(repo_root: &Path) -> Result<String> {
    read_extension_toml(repo_root)?
        .get("version")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("failed to read `version` from zed-extension/extension.toml"))
}

pub(crate) fn grammar_repo_and_rev(repo_root: &Path) -> Result<(String, String)> {
    let manifest = read_extension_toml(repo_root)?;
    let grammar = manifest
        .get("grammars")
        .and_then(toml::Value::as_table)
        .and_then(|grammars| grammars.get("isabelle"))
        .and_then(toml::Value::as_table)
        .ok_or_else(|| anyhow!("failed to read [grammars.isabelle] from extension manifest"))?;

    let repository = grammar
        .get("repository")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("failed to read [grammars.isabelle].repository"))?;
    let rev = grammar
        .get("rev")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("failed to read [grammars.isabelle].rev"))?;
    Ok((repository, rev))
}

pub(crate) fn ensure_wasm_target_installed() -> Result<()> {
    let installed = command_output(Command::new("rustup").args(["target", "list", "--installed"]))?;
    if installed.lines().any(|line| line.trim() == "wasm32-wasip2") {
        return Ok(());
    }

    println!("Installing Rust target wasm32-wasip2...");
    run_command(Command::new("rustup").args(["target", "add", "wasm32-wasip2"]))
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("failed to create directory: {}", dst.display()))?;
    for entry in
        fs::read_dir(src).with_context(|| format!("failed to read directory: {}", src.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target).with_context(|| {
                format!("failed to copy {} -> {}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

pub(crate) fn copy_executable(src: &Path, dst: &Path) -> Result<()> {
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} -> {}", src.display(), dst.display()))?;
    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(dst, perms)
            .with_context(|| format!("failed to set executable bit on {}", dst.display()))?;
    }
    Ok(())
}

pub(crate) fn validate_extension_id(id: &str) -> Result<()> {
    let id_re = Regex::new(r"^[a-z0-9-]+$").expect("valid regex");
    if !id_re.is_match(id) {
        bail!("invalid extension id '{id}' (must match ^[a-z0-9-]+$)");
    }
    if id.starts_with("zed-") || id.ends_with("-zed") {
        bail!("invalid extension id '{id}' (must not start with 'zed-' or end with '-zed')");
    }
    Ok(())
}
