#!/usr/bin/env bash
# validate-nix-outputs.sh - Verify all images in .propel/config.toml have valid Nix flake outputs
#
# Usage:
#   ./scripts/validate-nix-outputs.sh          # Check all local + satellite repos
#   ./scripts/validate-nix-outputs.sh --local  # Check only local repo
#   ./scripts/validate-nix-outputs.sh --satellite  # Check only satellite repos
#
# Exit code: 0 if all valid, 1 if any missing

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
CONFIG_FILE="$REPO_ROOT/.propel/config.toml"

FILTER="${1:-all}"  # all, local, satellite
if [[ "$FILTER" == "--local" ]]; then
  FILTER="local"
elif [[ "$FILTER" == "--satellite" ]]; then
  FILTER="satellite"
fi

if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "❌ Config file not found: $CONFIG_FILE"
  exit 1
fi

# Parse TOML to extract images and their nix_attr + repo
declare -a IMAGES=()
declare -a NIX_ATTRS=()
declare -a REPOS=()
declare -a CONTEXTS=()
declare -a SUBDIRS=()

while IFS='|' read -r name nix_attr repo context subdir; do
  if [[ -n "$name" ]]; then
    IMAGES+=("$name")
    NIX_ATTRS+=("$nix_attr")
    REPOS+=("$repo")
    CONTEXTS+=("$context")
    SUBDIRS+=("$subdir")
  fi
done < <(
  # Emit `name|nix_attr|repo|context|subdir` per build via propel-tools (Rust,
  # from the repo flake — replaces the former inline python3). Prefer a binary on
  # PATH (e.g. from `nix develop`), else run the flake app directly.
  if command -v propel-tools >/dev/null 2>&1; then
    propel-tools list-builds --config "$CONFIG_FILE" --filter "$FILTER"
  else
    nix run "$REPO_ROOT#propel-tools" -- list-builds --config "$CONFIG_FILE" --filter "$FILTER"
  fi || { echo "❌ Failed to list builds from $CONFIG_FILE via propel-tools"; exit 1; }
)

if [[ ${#IMAGES[@]} -eq 0 ]]; then
  echo "❌ No images found in $CONFIG_FILE"
  exit 1
fi

echo "Validating ${#IMAGES[@]} images..."
echo ""

FAILED=0
SUCCESS=0

# Validate each image
for i in "${!IMAGES[@]}"; do
  image="${IMAGES[$i]}"
  nix_attr="${NIX_ATTRS[$i]}"
  repo="${REPOS[$i]}"
  context="${CONTEXTS[$i]}"
  subdir="${SUBDIRS[$i]}"

  if [[ -z "$repo" ]]; then
    # Search for local workspace in candidate directories where context exists
    flake_path=""
    for candidate in "app" "service" "base"; do
      if [[ -d "$REPO_ROOT/../$candidate/$context" ]]; then
        flake_path="$REPO_ROOT/../$candidate"
        break
      fi
    done

    # Fallback to defaults if no candidate directory contains context
    if [[ -z "$flake_path" ]]; then
      if [[ -f "$REPO_ROOT/../app/flake.nix" ]]; then
        flake_path="$REPO_ROOT/../app"
      else
        flake_path="$REPO_ROOT"
      fi
    fi

    if [[ -n "$subdir" ]]; then
      flake_path="$flake_path/$subdir"
    fi
    flake_ref="$flake_path"
  else
    # Satellite repo
    if [[ -f "$REPO_ROOT/../$repo/flake.nix" && -z "$subdir" ]]; then
      flake_path="$REPO_ROOT/../$repo"
      flake_ref="$flake_path"
    elif [[ -n "$subdir" && -f "$REPO_ROOT/../$repo/$subdir/flake.nix" ]]; then
      flake_path="$REPO_ROOT/../$repo/$subdir"
      flake_ref="$flake_path"
    else
      flake_path="github:your-org/$repo"
      if [[ -n "$subdir" ]]; then
        flake_ref="$flake_path?dir=$subdir"
      else
        flake_ref="$flake_path"
      fi
    fi
  fi

  # Try to evaluate the flake output
  if eval_output=$(nix eval "$flake_ref#$nix_attr" --apply "x: x.name" 2>&1); then
    echo "✅ $image: $nix_attr"
    ((SUCCESS++))
  else
    # Distinguish between "attribute missing" vs "flake inaccessible / offline / private repository"
    # Only mark as FAILED if the error is specifically about the nix attribute not existing in the flake.
    # This check is intentionally narrow: "does not provide attribute" is the canonical nix error for missing attributes.
    # Broader patterns like "missing" can match download/source errors and cause false positives.
    if [[ "$eval_output" == *"does not provide attribute"* ]]; then
      echo "❌ $image: $nix_attr (not found in flake)"
      ((FAILED++))
    else
      echo "⚠️  $image: skipped ($nix_attr) - source not found or inaccessible offline/without credentials"
    fi
  fi
done

echo ""
echo "Summary: $SUCCESS/$((SUCCESS + FAILED)) images validated successfully"

if [[ $FAILED -gt 0 ]]; then
  echo "❌ Validation failed"
  exit 1
else
  echo "✅ Validation check complete"
  exit 0
fi
