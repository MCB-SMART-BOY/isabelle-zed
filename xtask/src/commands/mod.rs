pub(crate) mod build;
pub(crate) mod doctor;
pub(crate) mod mock;
pub(crate) mod zed;

use crate::cli::Commands;
use anyhow::Result;
use std::path::Path;

pub(crate) fn run(command: Commands, repo_root: &Path) -> Result<()> {
    match command {
        Commands::Doctor => doctor::run(repo_root),
        Commands::ReleaseBuild => build::release_build(repo_root),
        Commands::InstallLocal => build::install_local(repo_root),
        Commands::InstallZedNative => zed::install_native(repo_root),
        Commands::UninstallZedNative => zed::uninstall_native(repo_root),
        Commands::InstallZedShortcuts => zed::install_shortcuts(),
        Commands::UninstallZedShortcuts => zed::uninstall_shortcuts(),
        Commands::BuildIsabelleGrammar => build::build_isabelle_grammar(repo_root),
        Commands::ZedOfficialCheck => zed::official_check(repo_root),
        Commands::ReleasePackage { platform } => zed::release_package(repo_root, platform),
        Commands::BridgeMockUp { socket } => mock::bridge_mock_up(repo_root, &socket),
        Commands::BridgeMockDown { socket } => mock::bridge_mock_down(&socket),
        Commands::MockSend { socket } => mock::mock_send(&socket),
        Commands::MockLspE2e => mock::mock_lsp_e2e(repo_root),
        Commands::MockLspE2eTcp => mock::mock_lsp_e2e_tcp(repo_root),
        Commands::BridgeRealSmoke => mock::bridge_real_smoke(repo_root),
        Commands::NativeLspSmoke => mock::native_lsp_smoke(),
        Commands::SpawnE2eNdjson => mock::spawn_e2e_ndjson(repo_root),
    }
}
