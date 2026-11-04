set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Run the complete CI validation gate in exact order
check: lint-scripts drift fmt-check clippy test build test-upgrade

# Check shell scripts with shellcheck
lint-scripts:
    shellcheck scripts/*.sh

# Verify that README error tables match errors.rs (pass --write to update)
drift *FLAGS:
    scripts/check-readme-errors.sh {{FLAGS}}

# Format Rust codebase
fmt:
    cargo fmt --all

# Check Rust code formatting without modification
fmt-check:
    cargo fmt --all --check

# Run clippy with warnings treated as errors
clippy:
    cargo clippy --all-targets -- -D warnings

# Run all workspace unit and integration tests
test:
    cargo test --workspace

# Build contracts for wasm32v1-none
build:
    stellar contract build

# Test contract upgrade paths against built wasm
test-upgrade:
    cargo test -p rubbi-vault --features wasm-upgrade-test
    cargo test -p rubbi-subscriptions --features wasm-upgrade-test
    cargo test -p rubbi-payroll --features wasm-upgrade-test

# Deploy contracts to testnet (passes optional flags such as --dry-run or --force)
deploy-testnet *FLAGS:
    scripts/deploy-testnet.sh {{FLAGS}}

# Run testnet smoke tests
smoke-testnet *FLAGS:
    scripts/smoke-testnet.sh {{FLAGS}}

# Clean build artifacts
clean:
    cargo clean
