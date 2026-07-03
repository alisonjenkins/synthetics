set shell := ["bash", "-uc"]

CLUSTER := "synthetics"
REGISTRY := "localhost:5001"
CNPG_VERSION := "1.24.1"
RELEASE := "synthetics"

# List recipes.
default:
    @just --list

# --- dev ---

# Run the test suite (workspace).
test:
    cargo nextest run --workspace

# Lint gates (clippy + fmt + cargo-deny).
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all -- --check
    cargo deny check

# --- local cluster ---

# Create the KIND cluster + local registry.
kind-up:
    bash deploy/kind-with-registry.sh

# Delete the cluster + registry.
kind-down:
    -kind delete cluster --name {{CLUSTER}}
    -docker rm -f kind-registry

# Install the CloudNativePG operator and wait for it.
cnpg-install:
    kubectl apply --server-side -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/v{{CNPG_VERSION}}/releases/cnpg-{{CNPG_VERSION}}.yaml
    kubectl -n cnpg-system rollout status deployment/cnpg-controller-manager --timeout=180s

# Build both OCI images with nix and push them to the local registry.
images:
    #!/usr/bin/env bash
    set -euo pipefail
    for pair in "worker-image:synthetics-worker" "control-plane-image:synthetics-control-plane" "mock-datadog-image:synthetics-mock-datadog"; do
      attr="${pair%%:*}"; name="${pair##*:}"
      out="$(nix build ".#${attr}" --no-link --print-out-paths)"
      docker load < "$out"
      docker tag "${name}:latest" "{{REGISTRY}}/${name}:latest"
      docker push "{{REGISTRY}}/${name}:latest"
    done

# Deploy (or upgrade) the Helm release.
deploy:
    helm upgrade --install {{RELEASE}} deploy/helm/synthetics --wait

# Render the chart without deploying (sanity check).
template:
    helm template {{RELEASE}} deploy/helm/synthetics

# Stand the whole local stack up from scratch.
up: kind-up cnpg-install images deploy
    @echo "synthetics stack up. Control plane: kubectl port-forward svc/{{RELEASE}}-synthetics-control-plane 8080:8080"

# Tear everything down.
down: kind-down
