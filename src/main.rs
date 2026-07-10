//! propel-tools — Rust CLI for the oci-dockworker-build build matrix.
//!
//! Replaces the former Python/uv scripts and the inline `python3` snippets in
//! `.gitea/workflows/ci.yaml`:
//!   * `sync-agent-builds`   — was `scripts/sync_agent_builds.py`
//!   * `validate-config`     — was the `validate-config` Gitea job
//!   * `validate-manifests`  — was the `validate-kustomize` Gitea job
//!   * `list-builds`         — was the inline `python3` inside
//!     `scripts/validate-nix-outputs.sh`
//!
//! Everything is driven off the same TOML/YAML the Python did, so behaviour is a
//! faithful port. Built and shipped via Cargo + the repo `flake.nix` (no Python).

mod dockworker;
mod sync;
mod validate;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "propel-tools",
    about = "Build-matrix + config tooling for oci-dockworker-build",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate `.propel/agent-builds.generated.toml` from the registry
    /// agent registry (one `[[builds]]` per `your-org/agents` agent).
    SyncAgentBuilds {
        /// Path to `agents.toml`.
        registry: PathBuf,
        /// Output file (default: `.propel/agent-builds.generated.toml`).
        #[arg(short, long, default_value = ".propel/agent-builds.generated.toml")]
        out: PathBuf,
    },
    /// Validate `.propel/config.toml`: parse it and check every `[[builds]]`
    /// entry carries the required fields.
    ValidateConfig {
        #[arg(short, long, default_value = ".propel/config.toml")]
        config: PathBuf,
    },
    /// Validate rendered Kustomize manifests: every document has
    /// kind/apiVersion/metadata.name, and the `propel-config` ConfigMap's
    /// embedded `propel-config.toml` is valid TOML.
    ValidateManifests {
        /// Path to the rendered multi-doc manifests YAML.
        manifests: PathBuf,
    },
    /// Emit `name|nix_attr|repo|context|subdir` per build in `.propel/config.toml`
    /// (consumed by `scripts/validate-nix-outputs.sh`).
    ListBuilds {
        #[arg(short, long, default_value = ".propel/config.toml")]
        config: PathBuf,
        /// Which builds to emit: `all`, `local` (no `repo`), or `satellite`
        /// (has a `repo`).
        #[arg(long, default_value = "all")]
        filter: String,
    },
    /// Expand a caller's `dockworker.toml` into a GitHub Actions build matrix,
    /// filtered by changed files on pull_request. Reads `CHANGED_FILES` and
    /// writes `has_changes` + `matrix` to `$GITHUB_OUTPUT`.
    DockworkerMatrix {
        /// Path to the caller's dockworker config (`inputs.dockworker-config`).
        #[arg(short, long)]
        config: String,
        /// Image registry prefix (`inputs.registry`).
        #[arg(short, long)]
        registry: String,
        /// GitHub event name (`github.event_name`).
        #[arg(short, long, default_value = "")]
        event: String,
    },
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::SyncAgentBuilds { registry, out } => sync::sync_agent_builds(&registry, &out),
        Cmd::ValidateConfig { config } => validate::validate_config(&config),
        Cmd::ValidateManifests { manifests } => validate::validate_manifests(&manifests),
        Cmd::ListBuilds { config, filter } => validate::list_builds(&config, &filter),
        Cmd::DockworkerMatrix {
            config,
            registry,
            event,
        } => dockworker::dockworker_matrix(&config, &registry, &event),
    }
}
