use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "isabelle-zed-xtask")]
#[command(about = "Rust task runner for Isabelle-Zed tooling")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    Doctor,
    ReleaseBuild,
    InstallLocal,
    InstallZedNative,
    UninstallZedNative,
    InstallZedShortcuts,
    UninstallZedShortcuts,
    BuildIsabelleGrammar,
    ZedOfficialCheck,
    ReleasePackage {
        #[arg(long)]
        platform: Option<String>,
    },
    BridgeMockUp {
        #[arg(default_value = "/tmp/isabelle.sock")]
        socket: PathBuf,
    },
    BridgeMockDown {
        #[arg(default_value = "/tmp/isabelle.sock")]
        socket: PathBuf,
    },
    MockSend {
        #[arg(default_value = "/tmp/isabelle.sock")]
        socket: PathBuf,
    },
    MockLspE2e,
    NativeLspSmoke,
    SpawnE2eNdjson,
}
