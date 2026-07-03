{
  description = "synthetics — high-performance self-hosted synthetic monitoring (worker + control plane)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # Pinned stable toolchain (matches rust-toolchain.toml intent).
        toolchain = fenix.packages.${system}.stable.toolchain;
        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
          # ring (via tokio-rustls) needs a C compiler + perl to build its asm.
          nativeBuildInputs = [ pkgs.pkg-config pkgs.perl ];
          buildInputs = [ ];
          # Tests need Docker/Postgres (testcontainers); run them via cargo
          # nextest in CI/dev, not inside the pure Nix build sandbox.
          doCheck = false;
        };

        # Build all workspace dependencies once, shared by both binaries.
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        worker = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          pname = "synthetics";
          cargoExtraArgs = "-p synthetics --bin synthetics";
          meta.mainProgram = "synthetics";
        });

        control-plane = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          pname = "synthetics-control-plane";
          cargoExtraArgs = "-p synthetics-control-plane --bin synthetics-cp";
          meta.mainProgram = "synthetics-cp";
        });

        # Nix-built OCI images. The layered image includes each binary's full
        # runtime closure automatically; we add CA certs for outbound TLS.
        mkImage = { name, pkg, entry }:
          pkgs.dockerTools.buildLayeredImage {
            inherit name;
            tag = "latest";
            contents = [ pkg pkgs.cacert ];
            config = {
              Entrypoint = [ "/bin/${entry}" ];
              Env = [ "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" ];
            };
          };

        worker-image = mkImage {
          name = "synthetics-worker";
          pkg = worker;
          entry = "synthetics";
        };
        control-plane-image = mkImage {
          name = "synthetics-control-plane";
          pkg = control-plane;
          entry = "synthetics-cp";
        };
      in
      {
        packages = {
          inherit worker control-plane worker-image control-plane-image;
          default = worker;
        };

        # `nix flake check` builds both binaries.
        checks = { inherit worker control-plane; };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ worker ];
          packages = [
            toolchain
            pkgs.cargo-nextest
            pkgs.cargo-deny
            pkgs.just
            pkgs.tilt
            pkgs.kind
            pkgs.kubectl
            pkgs.kubernetes-helm
            pkgs.docker-client
            pkgs.postgresql # psql client
          ];
        };

        formatter = pkgs.nixpkgs-fmt;
      });
}
