use crate::common::{
    bridge_binary_path, command_exists, command_output, extension_wasm_path, lsp_binary_path,
};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) fn run(repo_root: &Path) -> Result<()> {
    println!("Running Isabelle-Zed doctor");

    if command_exists("cargo") {
        println!("[ok] command 'cargo' is available");
    } else {
        bail!("required command 'cargo' is missing");
    }

    if command_exists("rustup") {
        println!("[ok] command 'rustup' is available");
    } else {
        bail!("required command 'rustup' is missing");
    }

    if command_exists("isabelle") {
        println!("[ok] command 'isabelle' is available");
    } else {
        println!("[warn] optional command 'isabelle' is missing");
    }

    let installed = command_output(Command::new("rustup").args(["target", "list", "--installed"]))?;
    if installed.lines().any(|line| line.trim() == "wasm32-wasip2") {
        println!("[ok] Rust target wasm32-wasip2 is installed");
    } else {
        println!(
            "[warn] Rust target wasm32-wasip2 is not installed (run: rustup target add wasm32-wasip2)"
        );
    }

    let bridge_bin = bridge_binary_path(repo_root, "release");
    if bridge_bin.is_file() {
        println!("[ok] bridge release binary is present");
    } else {
        println!(
            "[warn] bridge release binary not found (run: cargo run -p isabelle-zed-xtask -- release-build)"
        );
    }

    let lsp_bin = lsp_binary_path(repo_root, "release");
    if lsp_bin.is_file() {
        println!("[ok] isabelle-zed-lsp release binary is present");
    } else {
        println!(
            "[warn] isabelle-zed-lsp release binary not found (run: cargo run -p isabelle-zed-xtask -- release-build)"
        );
    }

    let extension_wasm = extension_wasm_path(repo_root, "release");
    if extension_wasm.is_file() {
        println!("[ok] extension wasm artifact is present");
    } else {
        println!(
            "[warn] extension wasm artifact not found (run: cargo run -p isabelle-zed-xtask -- release-build)"
        );
    }

    let grammar_wasm = repo_root.join("zed-extension/grammars/isabelle.wasm");
    if grammar_wasm.is_file() {
        println!("[ok] isabelle grammar wasm artifact is present");
    } else {
        println!(
            "[warn] isabelle grammar wasm artifact not found (run: cargo run -p isabelle-zed-xtask -- build-isabelle-grammar)"
        );
    }

    for license in [
        repo_root.join("LICENSE"),
        repo_root.join("zed-extension/LICENSE"),
    ] {
        if license.is_file() {
            println!("[ok] license file is present: {}", license.display());
        } else {
            println!("[warn] license file is missing: {}", license.display());
        }
    }

    if command_exists("isabelle") {
        let status = Command::new("isabelle")
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to execute `isabelle version`")?;
        if status.success() {
            println!("[ok] isabelle command runs successfully");
        } else {
            println!("[warn] isabelle command exists but `isabelle version` failed");
        }
    }

    println!("Doctor check complete");
    Ok(())
}
