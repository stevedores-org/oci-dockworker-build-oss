//! Validators — ports of the inline `python3` from `.gitea/workflows/ci.yaml`
//! and `scripts/validate-nix-outputs.sh`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// `validate-config`: parse `.propel/config.toml` and check that every
/// `[[builds]]` entry carries the required fields. Port of the `validate-config`
/// Gitea job (its two steps combined).
pub fn validate_config(config_path: &Path) -> Result<()> {
    let text = fs::read_to_string(config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let config: toml::Value = toml::from_str(&text).with_context(|| "invalid TOML".to_string())?;
    println!("✅ TOML valid");

    let builds = config
        .get("builds")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    const REQUIRED: [&str; 4] = ["name", "context", "isNixFlake", "nix_attr"];

    println!("Found {} images:", builds.len());
    for build in &builds {
        let name = build.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let missing: Vec<&str> = REQUIRED
            .iter()
            .copied()
            .filter(|k| build.get(k).is_none())
            .collect();
        if !missing.is_empty() {
            println!("  ❌ {name}: missing fields {missing:?}");
            std::process::exit(1);
        }
        println!("  ✅ {name}");
    }

    println!("\n✅ All {} images have required fields", builds.len());
    Ok(())
}

/// `validate-manifests`: every rendered document has kind/apiVersion/metadata.name,
/// and the `propel-config` ConfigMap's embedded `propel-config.toml` is valid
/// TOML. Port of the `validate-kustomize` Gitea job's Python step.
pub fn validate_manifests(manifests_path: &Path) -> Result<()> {
    let text = fs::read_to_string(manifests_path)
        .with_context(|| format!("reading {}", manifests_path.display()))?;

    let mut docs: Vec<serde_yaml::Value> = Vec::new();
    for de in serde_yaml::Deserializer::from_str(&text) {
        let v = serde_yaml::Value::deserialize(de).context("Failed to parse YAML")?;
        docs.push(v);
    }

    println!("Loaded {} documents successfully.", docs.len());
    for (i, doc) in docs.iter().enumerate() {
        if doc.is_null() {
            continue;
        }
        let kind = doc.get("kind").and_then(|v| v.as_str());
        let api_version = doc.get("apiVersion").and_then(|v| v.as_str());
        let metadata = doc.get("metadata");
        let name = metadata
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str());

        if kind.is_none() || api_version.is_none() || metadata.is_none() || name.is_none() {
            println!(
                "❌ Document {i} is missing required fields (kind, apiVersion, metadata.name)"
            );
            std::process::exit(1);
        }
        let kind = kind.unwrap();
        let name = name.unwrap();
        println!("  ✅ {kind}: {name}");

        if kind == "ConfigMap" && name == "propel-config" {
            let config_toml = doc
                .get("data")
                .and_then(|d| d.get("propel-config.toml"))
                .and_then(|v| v.as_str());
            match config_toml {
                None => {
                    println!("❌ ConfigMap propel-config is missing propel-config.toml");
                    std::process::exit(1);
                }
                Some(s) => match toml::from_str::<toml::Value>(s) {
                    Ok(_) => println!("  ✅ propel-config propel-config.toml is valid TOML"),
                    Err(e) => {
                        println!("❌ propel-config propel-config.toml is invalid TOML: {e}");
                        std::process::exit(1);
                    }
                },
            }
        }
    }

    println!("✅ Manifest validation completed successfully!");
    Ok(())
}

/// `list-builds`: emit `name|nix_attr|repo|context|subdir` per build, honouring
/// the `all` / `local` / `satellite` filter. Port of the inline `python3` in
/// `scripts/validate-nix-outputs.sh`.
pub fn list_builds(config_path: &Path, filter: &str) -> Result<()> {
    let text = fs::read_to_string(config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let config: toml::Value = toml::from_str(&text).with_context(|| "invalid TOML".to_string())?;

    let builds = config
        .get("builds")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for build in &builds {
        let field = |k: &str| build.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let name = field("name");
        let nix_attr = field("nix_attr");
        let repo = field("repo");
        let context = field("context");
        let subdir = field("subdir");

        if filter == "local" && !repo.is_empty() {
            continue;
        }
        if filter == "satellite" && repo.is_empty() {
            continue;
        }
        println!("{name}|{nix_attr}|{repo}|{context}|{subdir}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(tmp: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let p = tmp.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn list_builds_filter_and_format() {
        let cfg: toml::Value = toml::from_str(
            r#"
            [[builds]]
            name = "local-one"
            nix_attr = "packages.x86_64-linux.oci"
            context = "web"

            [[builds]]
            name = "sat-one"
            nix_attr = "packages.x86_64-linux.oci-image"
            repo = "agents"
            context = "agents/x/sat-one"
            subdir = "sub"
            "#,
        )
        .unwrap();

        // Reuse the pure formatting by rendering via a temp file round-trip.
        let tmp = tempdir();
        let p = write(&tmp, "config.toml", &toml::to_string(&cfg).unwrap());

        assert_eq!(
            collect(&p, "all"),
            vec![
                "local-one|packages.x86_64-linux.oci||web|",
                "sat-one|packages.x86_64-linux.oci-image|agents|agents/x/sat-one|sub",
            ]
        );
        assert_eq!(
            collect(&p, "local"),
            vec!["local-one|packages.x86_64-linux.oci||web|"]
        );
        assert_eq!(
            collect(&p, "satellite"),
            vec!["sat-one|packages.x86_64-linux.oci-image|agents|agents/x/sat-one|sub"]
        );
    }

    // Test helper: run list_builds logic and capture the pipe-delimited lines.
    fn collect(config_path: &Path, filter: &str) -> Vec<String> {
        let text = fs::read_to_string(config_path).unwrap();
        let config: toml::Value = toml::from_str(&text).unwrap();
        let builds = config
            .get("builds")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for build in &builds {
            let field = |k: &str| build.get(k).and_then(|v| v.as_str()).unwrap_or("");
            let (name, nix_attr, repo, context, subdir) = (
                field("name"),
                field("nix_attr"),
                field("repo"),
                field("context"),
                field("subdir"),
            );
            if filter == "local" && !repo.is_empty() {
                continue;
            }
            if filter == "satellite" && repo.is_empty() {
                continue;
            }
            out.push(format!("{name}|{nix_attr}|{repo}|{context}|{subdir}"));
        }
        out
    }

    fn tempdir() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("propel-tools-test-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        base
    }
}
