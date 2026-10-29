#!/usr/bin/env bash
#
# Verify released Rubbi contract Wasm artifacts and their checksums.
#
# Usage:
#   scripts/verify-release.sh <tag> [path/to/testnet.json]
#
# Environment variables:
#   GITHUB_REPOSITORY   owner/repo (default: RubbiLabs/RL-smart-contracts, or from git origin)
#   CURL_OPTS           extra options passed to curl (e.g. -H "Authorization: token ...")
#
# What it does:
#   1. Downloads rubbi_vault.wasm, rubbi_subscriptions.wasm, rubbi_payroll.wasm,
#      and SHA256SUMS.txt from the specified GitHub Release tag.
#   2. Recomputes sha256 checksums locally and compares them against SHA256SUMS.txt.
#   3. Optionally compares the release hashes against the active wasm_hash in testnet.json.
#   4. Exits 0 on complete match, exits 1 on mismatch or download error.
#
# Portable to bash 3.2 (macOS default).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${1:-}"
DEPLOY_FILE="${2:-$REPO_ROOT/deployments/testnet.json}"

if [ -z "$TAG" ]; then
  printf 'usage: %s <tag> [path/to/testnet.json]\n' "$0" >&2
  printf 'example: %s v0.1.0\n' "$0" >&2
  exit 2
fi

# Resolve repository
if [ -n "${GITHUB_REPOSITORY:-}" ]; then
  REPO="$GITHUB_REPOSITORY"
elif git remote get-url origin 2>/dev/null | grep -q 'github.com'; then
  REPO="$(git remote get-url origin | sed -E 's/.*github\.com[:\/]([^\/]+\/[^\/\.]+)(\.git)?/\1/')"
else
  REPO="RubbiLabs/RL-smart-contracts"
fi

CURL_OPTS="${CURL_OPTS:-}"

log() { printf '==> %s\n' "$*"; }
pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# Portable sha256 function
compute_sha256() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    die "neither sha256sum nor shasum is available"
  fi
}

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
CONTRACTS="rubbi_vault.wasm rubbi_subscriptions.wasm rubbi_payroll.wasm"

log "Verifying release $TAG from $REPO"

# 1. Download SHA256SUMS.txt
log "Downloading SHA256SUMS.txt"
# shellcheck disable=SC2086
if ! curl -sSL $CURL_OPTS -f "${BASE_URL}/SHA256SUMS.txt" -o "$TMP_DIR/SHA256SUMS.txt"; then
  die "failed to download SHA256SUMS.txt for release tag $TAG from $REPO"
fi

# 2. Download and verify each contract Wasm
FAILED_COUNT=0

for wasm in $CONTRACTS; do
  log "Downloading $wasm..."
  # shellcheck disable=SC2086
  if ! curl -sSL $CURL_OPTS -f "${BASE_URL}/${wasm}" -o "$TMP_DIR/${wasm}"; then
    fail "failed to download $wasm for tag $TAG"
    FAILED_COUNT=$((FAILED_COUNT + 1))
    continue
  fi

  local_hash="$(compute_sha256 "$TMP_DIR/${wasm}")"
  expected_hash="$(awk -v f="$wasm" '$2 == f || $2 == "*"f || $2 ~ "/"f"$" {print $1}' "$TMP_DIR/SHA256SUMS.txt" | head -n 1)"

  if [ -z "$expected_hash" ]; then
    fail "$wasm is not listed in SHA256SUMS.txt"
    FAILED_COUNT=$((FAILED_COUNT + 1))
    continue
  fi

  if [ "$local_hash" = "$expected_hash" ]; then
    pass "$wasm sha256: $local_hash"
  else
    fail "$wasm checksum mismatch: expected $expected_hash, computed $local_hash"
    FAILED_COUNT=$((FAILED_COUNT + 1))
  fi
done

if [ "$FAILED_COUNT" -gt 0 ]; then
  die "release verification failed with $FAILED_COUNT error(s)"
fi

# 3. Optional comparison against deployment file
if [ -f "$DEPLOY_FILE" ]; then
  log "Comparing release artifacts with deployment file ($DEPLOY_FILE)"
  if command -v jq >/dev/null 2>&1; then
    vault_deployed="$(jq -r '.contracts.vault.wasm_hash // empty' "$DEPLOY_FILE")"
    sub_deployed="$(jq -r '.contracts.subscriptions.wasm_hash // empty' "$DEPLOY_FILE")"
    pay_deployed="$(jq -r '.contracts.payroll.wasm_hash // empty' "$DEPLOY_FILE")"

    vault_released="$(compute_sha256 "$TMP_DIR/rubbi_vault.wasm")"
    sub_released="$(compute_sha256 "$TMP_DIR/rubbi_subscriptions.wasm")"
    pay_released="$(compute_sha256 "$TMP_DIR/rubbi_payroll.wasm")"

    [ "$vault_deployed" = "$vault_released" ] && pass "vault hash matches testnet.json" || log "vault: release ($vault_released) differs from testnet ($vault_deployed) -> upgrade pending"
    [ "$sub_deployed" = "$sub_released" ] && pass "subscriptions hash matches testnet.json" || log "subscriptions: release ($sub_released) differs from testnet ($sub_deployed) -> upgrade pending"
    [ "$pay_deployed" = "$pay_released" ] && pass "payroll hash matches testnet.json" || log "payroll: release ($pay_released) differs from testnet ($pay_deployed) -> upgrade pending"
  else
    log "jq not installed; skipping testnet.json hash comparison"
  fi
fi

log "SUCCESS: All contracts in release $TAG match their build checksums"
exit 0
