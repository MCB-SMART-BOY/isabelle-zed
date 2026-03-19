use crate::common::{
    bridge_binary_path, command_exists, command_output, copy_executable,
    ensure_wasm_target_installed, extension_wasm_path, grammar_repo_and_rev, home_dir,
    lsp_binary_path, run_command,
};
use anyhow::{Context, Result, anyhow, bail};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) fn release_build(repo_root: &Path) -> Result<()> {
    if !command_exists("cargo") {
        bail!("cargo is required");
    }
    if !command_exists("rustup") {
        bail!("rustup is required to install/check wasm32-wasip2 target");
    }

    ensure_wasm_target_installed()?;

    println!("Building bridge (release)...");
    run_command(
        Command::new("cargo")
            .arg("build")
            .arg("-p")
            .arg("isabelle-bridge")
            .arg("--release"),
    )?;

    println!("Building isabelle-zed-lsp (release)...");
    run_command(
        Command::new("cargo")
            .arg("build")
            .arg("-p")
            .arg("isabelle-zed-lsp")
            .arg("--release"),
    )?;

    println!("Building Zed extension wasm (release)...");
    run_command(
        Command::new("cargo")
            .arg("build")
            .arg("-p")
            .arg("isabelle-zed-extension")
            .arg("--target")
            .arg("wasm32-wasip2")
            .arg("--release"),
    )?;

    let grammar = repo_root.join("zed-extension/grammars/isabelle.wasm");
    if !grammar.is_file() {
        println!("Building Isabelle grammar artifact...");
        build_isabelle_grammar(repo_root)?;
    }

    println!();
    println!("Build complete:");
    println!(
        "  bridge:            {}",
        bridge_binary_path(repo_root, "release").display()
    );
    println!(
        "  isabelle-zed-lsp:  {}",
        lsp_binary_path(repo_root, "release").display()
    );
    println!(
        "  extension wasm:    {}",
        extension_wasm_path(repo_root, "release").display()
    );
    println!("  grammar wasm:      {}", grammar.display());
    Ok(())
}

pub(crate) fn install_local(repo_root: &Path) -> Result<()> {
    release_build(repo_root)?;
    let install_dir = env::var("ISABELLE_ZED_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".local/bin"));
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("failed to create install dir: {}", install_dir.display()))?;

    copy_executable(
        &bridge_binary_path(repo_root, "release"),
        &install_dir.join("bridge"),
    )?;
    copy_executable(
        &lsp_binary_path(repo_root, "release"),
        &install_dir.join("isabelle-zed-lsp"),
    )?;

    println!("Installed binaries to: {}", install_dir.display());
    println!("  - bridge");
    println!("  - isabelle-zed-lsp");
    Ok(())
}

pub(crate) fn build_isabelle_grammar(repo_root: &Path) -> Result<()> {
    for cmd in ["git", "clang", "rustc"] {
        if !command_exists(cmd) {
            bail!("missing required command: {cmd}");
        }
    }

    let (repository, rev) = grammar_repo_and_rev(repo_root)?;
    let out_dir = repo_root.join("zed-extension/grammars");
    let out_file = out_dir.join("isabelle.wasm");

    let sysroot = command_output(Command::new("rustc").args(["--print", "sysroot"]))?;
    let rustc_verbose = command_output(Command::new("rustc").arg("-vV"))?;
    let host = rustc_verbose
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or_else(|| anyhow!("failed to parse rustc host triple"))?;

    let mut rust_lld = PathBuf::from(sysroot);
    rust_lld.push("lib/rustlib");
    rust_lld.push(host);
    rust_lld.push("bin");
    rust_lld.push(if cfg!(windows) {
        "rust-lld.exe"
    } else {
        "rust-lld"
    });
    if !rust_lld.is_file() {
        bail!(
            "rust-lld not found at expected path: {}",
            rust_lld.display()
        );
    }

    let temp_root = env::temp_dir().join(format!(
        "isabelle-zed-grammar-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    if temp_root.exists() {
        fs::remove_dir_all(&temp_root)
            .with_context(|| format!("failed to clean temp dir: {}", temp_root.display()))?;
    }
    fs::create_dir_all(&temp_root)
        .with_context(|| format!("failed to create temp dir: {}", temp_root.display()))?;

    let result = (|| -> Result<()> {
        let grammar_repo = temp_root.join("tree-sitter-isabelle");
        println!("Cloning tree-sitter-isabelle ({rev})...");

        let clone_status = Command::new("git")
            .arg("clone")
            .arg("--depth")
            .arg("1")
            .arg("--branch")
            .arg(&rev)
            .arg(&repository)
            .arg(&grammar_repo)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to start git clone for grammar repository")?;

        if !clone_status.success() {
            run_command(
                Command::new("git")
                    .arg("clone")
                    .arg("--depth")
                    .arg("1")
                    .arg(&repository)
                    .arg(&grammar_repo)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )?;
            run_command(
                Command::new("git")
                    .arg("-C")
                    .arg(&grammar_repo)
                    .arg("fetch")
                    .arg("--depth")
                    .arg("1")
                    .arg("origin")
                    .arg(&rev)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )?;
            run_command(
                Command::new("git")
                    .arg("-C")
                    .arg(&grammar_repo)
                    .arg("checkout")
                    .arg(&rev)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )?;
        }

        let include_dir = temp_root.join("include");
        fs::create_dir_all(&include_dir)?;
        fs::write(
            include_dir.join("stdlib.h"),
            "#ifndef _STDLIB_H\n#define _STDLIB_H\n#define NULL ((void*)0)\n#endif\n",
        )?;
        fs::write(
            include_dir.join("wctype.h"),
            "#ifndef _WCTYPE_H\n#define _WCTYPE_H\nstatic inline int iswspace(int c) {\n  return c == ' ' || c == '\\t' || c == '\\n' || c == '\\r' || c == '\\f' || c == '\\v';\n}\n#endif\n",
        )?;

        let parser_o = temp_root.join("parser.o");
        run_command(
            Command::new("clang")
                .arg("--target=wasm32-unknown-unknown")
                .arg("-O2")
                .arg("-fPIC")
                .arg(format!("-I{}", include_dir.display()))
                .arg(format!("-I{}", grammar_repo.join("src").display()))
                .arg("-c")
                .arg(grammar_repo.join("src/parser.c"))
                .arg("-o")
                .arg(&parser_o),
        )?;

        let mut objects = vec![parser_o];
        let mut exports = vec!["--export=tree_sitter_isabelle".to_string()];
        let scanner_c = grammar_repo.join("src/scanner.c");
        if scanner_c.is_file() {
            let scanner_o = temp_root.join("scanner.o");
            run_command(
                Command::new("clang")
                    .arg("--target=wasm32-unknown-unknown")
                    .arg("-O2")
                    .arg("-fPIC")
                    .arg(format!("-I{}", include_dir.display()))
                    .arg(format!("-I{}", grammar_repo.join("src").display()))
                    .arg("-c")
                    .arg(scanner_c)
                    .arg("-o")
                    .arg(&scanner_o),
            )?;
            objects.push(scanner_o);
            exports.extend([
                "--export=tree_sitter_isabelle_external_scanner_create".to_string(),
                "--export=tree_sitter_isabelle_external_scanner_destroy".to_string(),
                "--export=tree_sitter_isabelle_external_scanner_scan".to_string(),
                "--export=tree_sitter_isabelle_external_scanner_serialize".to_string(),
                "--export=tree_sitter_isabelle_external_scanner_deserialize".to_string(),
            ]);
        }

        let linked = temp_root.join("isabelle.wasm");
        let mut link_cmd = Command::new(&rust_lld);
        link_cmd.arg("-flavor").arg("wasm").arg("--shared");
        for export in &exports {
            link_cmd.arg(export);
        }
        for object in &objects {
            link_cmd.arg(object);
        }
        link_cmd.arg("-o").arg(&linked);
        run_command(&mut link_cmd)?;

        fs::create_dir_all(&out_dir)
            .with_context(|| format!("failed to create output dir: {}", out_dir.display()))?;
        fs::copy(&linked, &out_file).with_context(|| {
            format!("failed to copy grammar artifact to {}", out_file.display())
        })?;
        println!("Wrote grammar artifact: {}", out_file.display());
        Ok(())
    })();

    let _ = fs::remove_dir_all(&temp_root);
    result
}
