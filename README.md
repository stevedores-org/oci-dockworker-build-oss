# oci-dockworker-build

Nix-based OCI image build tooling: a small Rust CLI (`propel-tools`) that turns a
declarative `dockworker.toml` / `.propel/config.toml` into a GitHub Actions build
matrix, plus a **reusable OCI build-and-publish workflow** and composite actions.

Public OSS distribution.

## Components

| Piece | What it does |
| --- | --- |
| **`propel-tools`** (Rust CLI) | Expands a build config into a CI matrix, validates it, and syncs generated build lists. |
| **`.github/workflows/oci-build.yaml`** | Reusable (`workflow_call`) workflow: expands the matrix, `nix build`s each image, and pushes to a registry (GHCR/GAR/ECR) with ref-derived tags. |
| **Composite actions** (`.github/actions/*`) | `setup-attic-cache`, `setup-buildx-bun`, `build-push-bun`, `asset-mime-smoke`. |

## `propel-tools` subcommands

```bash
# Expand a dockworker config into a GitHub Actions build matrix
propel-tools dockworker-matrix --config dockworker.toml --registry ghcr.io/OWNER --event push

# Validate a .propel/config.toml (every [[builds]] entry parses + resolves)
propel-tools validate-config --config .propel/config.toml

# Regenerate a build list from a registry manifest
propel-tools sync-agent-builds <registry.toml> --out .propel/config.toml
```

## Config schema

Two accepted shapes. The canonical `[[builds]]`:

```toml
[[builds]]
name = "my-app"          # also the image name
nix_attr = "oci"          # flake attr: nix build .#oci  (built from `context`)
context = ""              # subdir/flake dir (optional)
```

…and the legacy `[[targets]]` (`name` / `image` / `nix_output`).

## Build

```bash
# Nix (hermetic)
nix build            # -> ./result (the propel-tools binary)
nix run . -- --help

# cargo
cargo build --release
cargo run -- --help
```

## Using the reusable workflow

```yaml
# .github/workflows/build-on-main.yml (in your repo)
jobs:
  build:
    uses: stevedores-org/oci-dockworker-build-oss/.github/workflows/oci-build.yaml@main
    with:
      dockworker-config: .propel/config.toml
      registry: ghcr.io/${{ github.repository_owner }}
    secrets: inherit
```

The workflow computes tags from `github.ref` (`main`, `main-<UTCyyyymmddHHMMSS>`,
`sha-<short>`), builds each `nix build .#<attr>`, and pushes them.

## License

Apache-2.0 — see [LICENSE](./LICENSE).
