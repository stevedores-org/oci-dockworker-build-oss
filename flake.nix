{
  description = "propel-tools — Rust build-matrix + config tooling for oci-dockworker-build (replaces the former Python/uv scripts)";

  nixConfig = {
    extra-substituters = "https://cache.example.org";
    extra-trusted-public-keys = "cache.example.org:RZvvJ2Hx0EKj4V+J9dHKkfJ5L5YmP0C+WJ8K8J5J8pY=";
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        propel-tools = pkgs.rustPlatform.buildRustPackage {
          pname = "propel-tools";
          version = "0.1.0";

          # Only the crate sources — so a docs/deploy edit does not rebuild the tool.
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: _type:
              let rel = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
              in rel == "Cargo.toml"
              || rel == "Cargo.lock"
              || rel == "src"
              || pkgs.lib.hasPrefix "src/" rel;
          };

          cargoLock.lockFile = ./Cargo.lock;

          meta = with pkgs.lib; {
            description = "Generate the agent build matrix and validate Propel config + Kustomize manifests";
            license = licenses.mit;
            mainProgram = "propel-tools";
          };
        };
      in
      {
        packages.default = propel-tools;
        packages.propel-tools = propel-tools;

        apps.default = flake-utils.lib.mkApp { drv = propel-tools; };
        apps.propel-tools = flake-utils.lib.mkApp { drv = propel-tools; };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc clippy rustfmt ];
        };
      });
}
