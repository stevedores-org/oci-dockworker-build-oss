//! `dockworker-matrix` — port of the inline `python3` in
//! `.github/workflows/oci-build.yaml`.
//!
//! Parses a caller's `dockworker.toml`, expands it into a GitHub Actions build
//! matrix (one entry per target), filters targets by changed files on
//! pull_request events, and writes `has_changes` + `matrix` to `$GITHUB_OUTPUT`.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

/// One matrix entry. Field order mirrors the Python dict so the emitted JSON is
/// identical (GitHub's `fromJson` is order-insensitive, but keep parity anyway).
#[derive(Debug, Serialize, Default)]
struct Target {
    name: String,
    image: String,
    context: String,
    nix_output: String,
    gar_image: String,
    install_command: String,
    build_command: String,
    // Satellite-build fields (empty for the legacy `[[targets]]` schema). When
    // `repo` is set, the build workflow checks out `your-org/<repo>` and builds
    // from `<subdir>` inside it; `bun_dir`/`dist_dir` drive the optional frontend
    // asset build the flake's OCI output consumes.
    repo: String,
    subdir: String,
    bun_dir: String,
    dist_dir: String,
}

fn str_field<'a>(v: &'a toml::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

/// `build_cfg.get(key, "")` where build_cfg is the target's `build` table if
/// present, else the top-level `build` table.
fn build_command(target: &toml::Value, cfg: &toml::Value, key: &str) -> String {
    let build_cfg = target.get("build").or_else(|| cfg.get("build"));
    build_cfg
        .and_then(|b| b.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn expand_targets(cfg: &toml::Value, registry: &str) -> Result<Vec<Target>> {
    let mut targets = Vec::new();

    if let Some(list) = cfg.get("targets").and_then(|v| v.as_array()) {
        for t in list {
            let name = str_field(t, "name")
                .context("target is missing required field `name`")?
                .to_string();
            let image = str_field(t, "image").unwrap_or(&name).to_string();
            let context = str_field(t, "context").unwrap_or("").to_string();
            let nix_output = str_field(t, "nix_output")
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("oci-image-{image}"));
            targets.push(Target {
                gar_image: format!("{registry}/{image}"),
                install_command: build_command(t, cfg, "install"),
                build_command: build_command(t, cfg, "build"),
                name,
                image,
                context,
                nix_output,
                ..Default::default()
            });
        }
    } else {
        // Single target.
        let name = cfg
            .get("package")
            .and_then(|p| str_field(p, "name"))
            .unwrap_or("app")
            .to_string();
        let nix_output = cfg
            .get("oci")
            .and_then(|o| str_field(o, "nix_output"))
            .unwrap_or("oci")
            .to_string();
        let image = cfg
            .get("registry")
            .and_then(|r| str_field(r, "name"))
            .unwrap_or(&name)
            .to_string();
        targets.push(Target {
            gar_image: format!("{registry}/{image}"),
            install_command: build_command(cfg, cfg, "install"),
            build_command: build_command(cfg, cfg, "build"),
            name,
            image,
            context: String::new(), // single-target has no context
            nix_output,
            ..Default::default()
        });
    }

    Ok(targets)
}

/// Expand the `[[builds]]` schema — the canonical `.propel/config.toml` format
/// (satellite repos + nix flake attrs) — into matrix targets. This is what the
/// fleet actually uses; `[[targets]]` (above) is the older generic
/// schema kept for back-compat.
///
/// Context patterns handled:
///   * `context = "satellite:<repo>"`  → check out `your-org/<repo>`, build in
///     the optional `subdir`.
///   * `repo = "<repo>"` + `context = "<path>"` → check out `<repo>`, build in
///     `<path>` (the context is the subdir).
///   * `context = "<path>"` with no `repo` → local build from `<path>` in this
///     repo (no satellite checkout).
fn expand_builds(cfg: &toml::Value, registry: &str) -> Result<Vec<Target>> {
    let list = cfg
        .get("builds")
        .and_then(|v| v.as_array())
        .context("config has no `[[builds]]` array")?;
    let mut targets = Vec::with_capacity(list.len());
    for b in list {
        let name = str_field(b, "name")
            .context("build is missing required field `name`")?
            .to_string();
        let nix_output = str_field(b, "nix_attr")
            .context("build is missing required field `nix_attr`")?
            .to_string();
        let context = str_field(b, "context").unwrap_or("").to_string();
        // Resolve the satellite repo + subdir from the context patterns above.
        let (repo, subdir) = if let Some(r) = context.strip_prefix("satellite:") {
            (
                r.to_string(),
                str_field(b, "subdir").unwrap_or("").to_string(),
            )
        } else if let Some(r) = str_field(b, "repo") {
            (
                r.to_string(),
                str_field(b, "subdir").unwrap_or(&context).to_string(),
            )
        } else {
            (String::new(), String::new())
        };
        // GAR image name mirrors the build name.
        let image = name.clone();
        targets.push(Target {
            gar_image: format!("{registry}/{image}"),
            name,
            image,
            context,
            nix_output,
            repo,
            subdir,
            bun_dir: str_field(b, "bun_dir").unwrap_or("").to_string(),
            dist_dir: str_field(b, "dist_dir").unwrap_or("").to_string(),
            install_command: String::new(),
            build_command: String::new(),
        });
    }
    Ok(targets)
}

fn filter_targets(
    targets: Vec<Target>,
    event: &str,
    changed: &[String],
    config_path: &str,
) -> Vec<Target> {
    if event != "pull_request" || changed.is_empty() {
        return targets;
    }
    let workspace_files = ["Cargo.toml", "Cargo.lock", "flake.nix", "flake.lock"];
    targets
        .into_iter()
        .filter(|t| {
            if t.context.is_empty() {
                return true;
            }
            let mut ctx = t.context.clone();
            if !ctx.ends_with('/') {
                ctx.push('/');
            }
            changed.iter().any(|f| {
                f.starts_with(&ctx) || workspace_files.contains(&f.as_str()) || f == config_path
            })
        })
        .collect()
}

pub fn dockworker_matrix(config_path: &str, registry: &str, event: &str) -> Result<()> {
    let registry = registry.trim_end_matches('/');

    if !Path::new(config_path).exists() {
        println!("Error: {config_path} not found");
        std::process::exit(1);
    }
    let text = fs::read_to_string(config_path).with_context(|| format!("reading {config_path}"))?;
    let cfg: toml::Value = toml::from_str(&text).with_context(|| "invalid TOML".to_string())?;

    // Prefer the canonical `[[builds]]` schema; fall back to the legacy
    // `[[targets]]` schema for older configs.
    let targets = if cfg.get("builds").and_then(|v| v.as_array()).is_some() {
        expand_builds(&cfg, registry)?
    } else {
        expand_targets(&cfg, registry)?
    };

    let changed: Vec<String> = std::env::var("CHANGED_FILES")
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let filtered = filter_targets(targets, event, &changed, config_path);

    let filter_image = std::env::var("FILTER_IMAGE").unwrap_or_default();
    let filtered = if filter_image.is_empty() {
        filtered
    } else {
        filtered
            .into_iter()
            .filter(|t| t.name == filter_image || t.image == filter_image)
            .collect()
    };

    let has_changes = if filtered.is_empty() { "false" } else { "true" };
    let matrix = serde_json::json!({ "include": filtered });
    let matrix_json = serde_json::to_string(&matrix)?;

    println!("has_changes: {has_changes}");
    println!("matrix: {matrix_json}");

    if let Ok(out_path) = std::env::var("GITHUB_OUTPUT") {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&out_path)
            .with_context(|| format!("opening GITHUB_OUTPUT {out_path}"))?;
        writeln!(f, "has_changes={has_changes}")?;
        writeln!(f, "matrix={matrix_json}")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_target_expansion_and_defaults() {
        let cfg: toml::Value = toml::from_str(
            r#"
            [build]
            install = "cargo fetch"
            build = "cargo build"

            [[targets]]
            name = "api"
            context = "services/api"

            [[targets]]
            name = "web"
            image = "web-frontend"
            nix_output = "custom-oci"
            context = "services/web"
            [targets.build]
            build = "bun run build"
            "#,
        )
        .unwrap();
        let ts = expand_targets(&cfg, "reg.io/org/").unwrap();
        // registry trailing slash is stripped by caller; here we passed it raw to
        // exercise gar_image formatting only via dockworker_matrix, so trim here:
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].image, "api");
        assert_eq!(ts[0].nix_output, "oci-image-api");
        assert_eq!(ts[0].install_command, "cargo fetch");
        assert_eq!(ts[0].build_command, "cargo build");
        assert_eq!(ts[1].image, "web-frontend");
        assert_eq!(ts[1].nix_output, "custom-oci");
        assert_eq!(ts[1].build_command, "bun run build"); // target-level override
                                                          // Fallback is per-target, not per-field (matches the Python original): a
                                                          // target with its own [build] table does NOT inherit missing keys from
                                                          // the top-level [build], so install stays empty here.
        assert_eq!(ts[1].install_command, "");
    }

    #[test]
    fn builds_schema_satellite_and_local_expansion() {
        let cfg: toml::Value = toml::from_str(
            r#"
            [[builds]]
            name = "web-chat-frontend"
            context = "satellite:web-chat"
            repo = "web-chat"
            dist_dir = "dist"
            isNixFlake = true
            nix_attr = "packages.x86_64-linux.frontend-oci"

            [[builds]]
            name = "ciso-agent"
            context = "satellite:agents"
            repo = "agents"
            subdir = "agents/security/ciso-agent"
            isNixFlake = true
            nix_attr = "packages.x86_64-linux.image"

            [[builds]]
            name = "zen"
            context = "agents/observability/zen"
            repo = "agents"
            isNixFlake = true
            nix_attr = "packages.x86_64-linux.oci-image-zen"

            [[builds]]
            name = "zero-copy-connector"
            context = "apps/zero-copy-connector"
            isNixFlake = true
            nix_attr = "packages.x86_64-linux.zero-copy-connector-oci"
            "#,
        )
        .unwrap();
        let ts = expand_builds(&cfg, "reg.io/org").unwrap();
        assert_eq!(ts.len(), 4);

        // satellite:<repo> → repo from context, nix_attr → nix_output, gar image.
        assert_eq!(ts[0].name, "web-chat-frontend");
        assert_eq!(ts[0].repo, "web-chat");
        assert_eq!(ts[0].subdir, "");
        assert_eq!(ts[0].dist_dir, "dist");
        assert_eq!(ts[0].nix_output, "packages.x86_64-linux.frontend-oci");
        assert_eq!(ts[0].gar_image, "reg.io/org/web-chat-frontend");

        // satellite:<repo> + explicit subdir.
        assert_eq!(ts[1].repo, "agents");
        assert_eq!(ts[1].subdir, "agents/security/ciso-agent");

        // repo set, no satellite prefix → context becomes the subdir.
        assert_eq!(ts[2].repo, "agents");
        assert_eq!(ts[2].subdir, "agents/observability/zen");

        // no repo → local build, no satellite checkout.
        assert_eq!(ts[3].repo, "");
        assert_eq!(ts[3].context, "apps/zero-copy-connector");
    }

    #[test]
    fn pr_filter_by_context_and_workspace_files() {
        let ts = vec![
            Target {
                name: "api".into(),
                image: "api".into(),
                context: "services/api".into(),
                nix_output: "oci-image-api".into(),
                gar_image: "r/api".into(),
                install_command: String::new(),
                build_command: String::new(),
                ..Default::default()
            },
            Target {
                name: "web".into(),
                image: "web".into(),
                context: "services/web".into(),
                nix_output: "oci-image-web".into(),
                gar_image: "r/web".into(),
                install_command: String::new(),
                build_command: String::new(),
                ..Default::default()
            },
        ];
        // Only services/api changed -> only api rebuilds.
        let out = filter_targets(
            ts,
            "pull_request",
            &["services/api/src/main.rs".to_string()],
            "dockworker.toml",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "api");
    }

    #[test]
    fn workspace_file_change_rebuilds_all() {
        let ts = vec![Target {
            name: "api".into(),
            image: "api".into(),
            context: "services/api".into(),
            nix_output: "oci-image-api".into(),
            gar_image: "r/api".into(),
            install_command: String::new(),
            build_command: String::new(),
            ..Default::default()
        }];
        let out = filter_targets(
            ts,
            "pull_request",
            &["flake.nix".to_string()],
            "dockworker.toml",
        );
        assert_eq!(out.len(), 1);
    }
}
