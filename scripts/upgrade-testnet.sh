#!/usr/bin/env bash
#
# Upgrade the Rubbi Soroban contracts on Stellar testnet to a released Wasm build.
#
# Usage:
#   scripts/upgrade-testnet.sh [options] <release-tag-or-wasm-dir>
#
# Options:
#   --dry-run             Print the commands that would execute without submitting
#                         transactions. Safe to run without admin credentials.
#   --admin-identity <id> Stellar CLI identity with admin authority (default: rubbi-admin).
#   --deploy-file <path>  Path to testnet.json (default: deployments/testnet.json).
#   --network <network>   Stellar network name (default: testnet).
#   --help, -h            Show this help text.
#
# Requirements:
#   - Refuses to run if the git working tree has uncommitted changes.
#   - Uses identity names in commands, never exposes or prints secret keys.
#   - Contributor runs only with --dry-run. Maintainer runs without --dry-run.
#
# Portable to bash 3.2 (macOS default).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NETWORK="testnet"
ADMIN_IDENTITY="${ADMIN_IDENTITY:-rubbi-admin}"
DEPLOY_FILE="$REPO_ROOT/deployments/testnet.json"
DRY_RUN=0
TARGET_REF=""

# Helper functions
log() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# Print command formatted with proper escaping
show_cmd() { printf '  $'; printf ' %q' "$@"; printf '\n'; }

run_or_show() {
  show_cmd "$@"
  if [ "$DRY_RUN" -eq 0 ]; then
    "$@"
  fi
}

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

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

# Parse options
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --admin-identity)
      [ $# -ge 2 ] || die "--admin-identity requires an argument"
      ADMIN_IDENTITY="$2"
      shift 2
      ;;
    --deploy-file)
      [ $# -ge 2 ] || die "--deploy-file requires an argument"
      DEPLOY_FILE="$2"
      shift 2
      ;;
    --network)
      [ $# -ge 2 ] || die "--network requires an argument"
      NETWORK="$2"
      shift 2
      ;;
    --help|-h)
      usage 0
      ;;
    -*)
      die "unknown option: $1 (see --help)"
      ;;
    *)
      if [ -z "$TARGET_REF" ]; then
        TARGET_REF="$1"
        shift
      else
        die "unexpected additional argument: $1"
      fi
      ;;
  esac
done

[ -n "$TARGET_REF" ] || { usage 1 >&2; }

# 1. Enforce clean git working tree
log "Checking git working tree..."
if ! git diff --quiet || ! git diff --staged --quiet || [ -n "$(git status --porcelain)" ]; then
  die "git working tree is dirty; refuse to run on uncommitted changes"
fi

# 2. Check deployment file
[ -f "$DEPLOY_FILE" ] || die "deployment file not found: $DEPLOY_FILE"
command -v jq >/dev/null 2>&1 || die "jq is required to parse $DEPLOY_FILE"

VAULT_ID="$(jq -r '.contracts.vault.contract_id // empty' "$DEPLOY_FILE")"
SUB_ID="$(jq -r '.contracts.subscriptions.contract_id // empty' "$DEPLOY_FILE")"
PAY_ID="$(jq -r '.contracts.payroll.contract_id // empty' "$DEPLOY_FILE")"

[ -n "$VAULT_ID" ] || die "missing contracts.vault.contract_id in $DEPLOY_FILE"
[ -n "$SUB_ID" ] || die "missing contracts.subscriptions.contract_id in $DEPLOY_FILE"
[ -n "$PAY_ID" ] || die "missing contracts.payroll.contract_id in $DEPLOY_FILE"

# 3. Resolve Wasm files and hashes
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

if [ -d "$TARGET_REF" ]; then
  log "Using local Wasm artifacts from directory $TARGET_REF"
  VAULT_WASM="$TARGET_REF/rubbi_vault.wasm"
  SUB_WASM="$TARGET_REF/rubbi_subscriptions.wasm"
  PAY_WASM="$TARGET_REF/rubbi_payroll.wasm"
elif [ -f "$TARGET_REF/rubbi_vault.wasm" ]; then
  VAULT_WASM="$TARGET_REF/rubbi_vault.wasm"
  SUB_WASM="$TARGET_REF/rubbi_subscriptions.wasm"
  PAY_WASM="$TARGET_REF/rubbi_payroll.wasm"
else
  # Treat as release tag
  log "Resolving release artifacts for tag $TARGET_REF..."
  if [ -n "${GITHUB_REPOSITORY:-}" ]; then
    REPO="$GITHUB_REPOSITORY"
  elif git remote get-url origin 2>/dev/null | grep -q 'github.com'; then
    REPO="$(git remote get-url origin | sed -E 's/.*github\.com[:\/]([^\/]+\/[^\/\.]+)(\.git)?/\1/')"
  else
    REPO="RubbiLabs/RL-smart-contracts"
  fi

  BASE_URL="https://github.com/${REPO}/releases/download/${TARGET_REF}"
  for wasm in rubbi_vault.wasm rubbi_subscriptions.wasm rubbi_payroll.wasm; do
    log "Downloading $wasm from $REPO @ $TARGET_REF"
    if ! curl -sSL -f "${BASE_URL}/${wasm}" -o "$TMP_DIR/${wasm}"; then
      die "failed to download ${wasm} from release ${TARGET_REF}"
    fi
  done
  VAULT_WASM="$TMP_DIR/rubbi_vault.wasm"
  SUB_WASM="$TMP_DIR/rubbi_subscriptions.wasm"
  PAY_WASM="$TMP_DIR/rubbi_payroll.wasm"
fi

[ -f "$VAULT_WASM" ] || die "missing $VAULT_WASM"
[ -f "$SUB_WASM" ] || die "missing $SUB_WASM"
[ -f "$PAY_WASM" ] || die "missing $PAY_WASM"

VAULT_HASH="$(compute_sha256 "$VAULT_WASM")"
SUB_HASH="$(compute_sha256 "$SUB_WASM")"
PAY_HASH="$(compute_sha256 "$PAY_WASM")"

log "Target Wasm SHA-256 Hashes:"
printf '  vault:         %s\n' "$VAULT_HASH"
printf '  subscriptions: %s\n' "$SUB_HASH"
printf '  payroll:       %s\n' "$PAY_HASH"

if [ "$DRY_RUN" -eq 1 ]; then
  log "DRY RUN MODE: displaying installation and upgrade transactions (no submissions)"
else
  log "LIVE MODE: executing contract upgrades signed by identity: $ADMIN_IDENTITY"
fi

# 4. Plan and run installation & upgrade calls
upgrade_one() {
  local name="$1" contract_id="$2" wasm_file="$3" new_hash="$4"

  log "Preparing upgrade for $name ($contract_id)..."

  # Install Wasm on-chain if needed
  log "Step 1: Install Wasm on $NETWORK"
  run_or_show stellar contract install \
    --wasm "$wasm_file" \
    --network "$NETWORK" \
    --source "$ADMIN_IDENTITY"

  # Invoke upgrade(new_wasm_hash)
  log "Step 2: Invoke upgrade on $name"
  run_or_show stellar contract invoke \
    --id "$contract_id" \
    --network "$NETWORK" \
    --source "$ADMIN_IDENTITY" \
    -- \
    upgrade \
    --new_wasm_hash "$new_hash"
}

upgrade_one "vault" "$VAULT_ID" "$VAULT_WASM" "$VAULT_HASH"
upgrade_one "subscriptions" "$SUB_ID" "$SUB_WASM" "$SUB_HASH"
upgrade_one "payroll" "$PAY_ID" "$PAY_WASM" "$PAY_HASH"

log "Upgrade sequence completed successfully"
