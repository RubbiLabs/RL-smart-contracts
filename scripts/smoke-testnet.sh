#!/usr/bin/env bash
#
# Read-only smoke test of a Rubbi testnet deployment.
#
# Reads deployments/testnet.json (written by scripts/deploy-testnet.sh) and
# checks, with simulated (never submitted) contract calls, that every
# contract is configured as recorded: admin, operator, treasury, token,
# fee recipient, fee_bps, not paused, sane totals, deployed wasm hash, and
# that the treasury and fee recipient hold a USDC trustline.
#
# Usage:
#   scripts/smoke-testnet.sh [path/to/testnet.json]
#
# Environment:
#   SMOKE_SOURCE   stellar CLI identity used as the simulation source account
#                  (default: rubbi-admin). Nothing is signed or sent.
#
# Exits 0 when every check passes, 1 on any mismatch.
# Portable to bash 3.2 (macOS default).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_FILE="${1:-$REPO_ROOT/deployments/testnet.json}"
SMOKE_SOURCE="${SMOKE_SOURCE:-rubbi-admin}"

PASSED=0
FAILED=0

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

pass() { PASSED=$((PASSED + 1)); printf 'PASS  %s\n' "$*"; }
fail() { FAILED=$((FAILED + 1)); printf 'FAIL  %s\n' "$*"; }

# check <label> <expected> <actual>
check() {
  if [ "$2" = "$3" ]; then
    pass "$1 = $3"
  else
    fail "$1: expected '$2', got '$3'"
  fi
}

# check_uint <label> <actual>: a non-negative integer (balances after use).
check_uint() {
  if [[ "$2" =~ ^[0-9]+$ ]]; then
    pass "$1 = $2 (non-negative)"
  else
    fail "$1: expected a non-negative integer, got '$2'"
  fi
}

# Read one field from the deployment record; fails if missing or null.
field() {
  local value
  value="$(jq -er "$1" "$DEPLOY_FILE")" || die "missing field $1 in $DEPLOY_FILE"
  printf '%s\n' "$value"
}

# Simulate a read-only contract call and print its decoded result.
# JSON strings (addresses, i128 values) are unquoted; bools/ints pass through.
call() {
  local contract="$1" fn="$2"
  shift 2
  stellar contract invoke \
    --id "$contract" --network "$NETWORK" --source-account "$SMOKE_SOURCE" \
    --send=no -- "$fn" "$@" | jq -r '.'
}

# Hash of the wasm actually installed behind a contract id.
deployed_wasm_hash() {
  stellar contract info hash --contract-id "$1" --network "$NETWORK"
}

# Number of USDC trustline entries of an account (0 or 1).
usdc_trustline_count() {
  stellar ledger entry fetch trustline \
    --account "$1" --asset "USDC:$USDC_ISSUER" \
    --network "$NETWORK" --output json | jq -r '.entries | length'
}

check_trustline() {
  local label="$1" account="$2" count
  count="$(usdc_trustline_count "$account")" || die "could not query the trustline of $account"
  if [ "$count" = "1" ]; then
    pass "$label ($account) trusts USDC:$USDC_ISSUER"
  else
    fail "$label ($account) has no USDC:$USDC_ISSUER trustline"
  fi
}

# ------------------------------------------------------------------ setup --
for tool in stellar jq; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' is not installed or not on PATH"
done
[ -f "$DEPLOY_FILE" ] || die "$DEPLOY_FILE not found; run scripts/deploy-testnet.sh first"
stellar keys public-key "$SMOKE_SOURCE" >/dev/null 2>&1 \
  || die "SMOKE_SOURCE identity '$SMOKE_SOURCE' is not in the stellar CLI keystore"

NETWORK="$(field .network)"
USDC_ISSUER="$(field .usdc_issuer)"
USDC_SAC="$(field .usdc_sac)"
VAULT_ID="$(field .contracts.vault.contract_id)"
SUBS_ID="$(field .contracts.subscriptions.contract_id)"
PAYROLL_ID="$(field .contracts.payroll.contract_id)"
ADMIN="$(field .roles.admin)"
OPERATOR="$(field .roles.operator)"
TREASURY="$(field .roles.treasury)"
FEES="$(field .roles.fee_recipient)"

printf 'Smoke test of %s (network %s, simulations signed by nobody, source %s)\n\n' \
  "$DEPLOY_FILE" "$NETWORK" "$SMOKE_SOURCE"

# ------------------------------------------------------------------- USDC --
printf -- '-- USDC\n'
check "usdc_sac matches issuer" \
  "$(stellar contract id asset --asset "USDC:$USDC_ISSUER" --network "$NETWORK")" "$USDC_SAC"
check_trustline "treasury" "$TREASURY"
check_trustline "fee_recipient" "$FEES"

# ------------------------------------------------------------------ vault --
printf -- '-- vault %s\n' "$VAULT_ID"
check "vault.admin" "$(field .contracts.vault.admin)" "$(call "$VAULT_ID" admin)"
check "vault.admin is roles.admin" "$ADMIN" "$(call "$VAULT_ID" admin)"
check "vault.operator" "$OPERATOR" "$(call "$VAULT_ID" operator)"
check "vault.treasury" "$TREASURY" "$(call "$VAULT_ID" treasury)"
check "vault.token" "$USDC_SAC" "$(call "$VAULT_ID" token)"
check "vault.is_paused" "false" "$(call "$VAULT_ID" is_paused)"
check_uint "vault.total_balances" "$(call "$VAULT_ID" total_balances)"
check "vault.wasm_hash" "$(field .contracts.vault.wasm_hash)" "$(deployed_wasm_hash "$VAULT_ID")"

# ---------------------------------------------------------- subscriptions --
printf -- '-- subscriptions %s\n' "$SUBS_ID"
check "subscriptions.admin" "$ADMIN" "$(call "$SUBS_ID" admin)"
check "subscriptions.token" "$USDC_SAC" "$(call "$SUBS_ID" token)"
check "subscriptions.fee_recipient" "$FEES" "$(call "$SUBS_ID" fee_recipient)"
check "subscriptions.fee_bps" "$(field .contracts.subscriptions.fee_bps)" "$(call "$SUBS_ID" fee_bps)"
check "subscriptions.is_paused" "false" "$(call "$SUBS_ID" is_paused)"
check "subscriptions.wasm_hash" "$(field .contracts.subscriptions.wasm_hash)" "$(deployed_wasm_hash "$SUBS_ID")"

# ---------------------------------------------------------------- payroll --
printf -- '-- payroll %s\n' "$PAYROLL_ID"
check "payroll.admin" "$ADMIN" "$(call "$PAYROLL_ID" admin)"
check "payroll.token" "$USDC_SAC" "$(call "$PAYROLL_ID" token)"
check "payroll.fee_recipient" "$FEES" "$(call "$PAYROLL_ID" fee_recipient)"
check "payroll.fee_bps" "$(field .contracts.payroll.fee_bps)" "$(call "$PAYROLL_ID" fee_bps)"
check "payroll.is_paused" "false" "$(call "$PAYROLL_ID" is_paused)"
check_uint "payroll.total_escrow" "$(call "$PAYROLL_ID" total_escrow)"
check "payroll.wasm_hash" "$(field .contracts.payroll.wasm_hash)" "$(deployed_wasm_hash "$PAYROLL_ID")"

# ----------------------------------------------------------------- result --
printf '\n%d passed, %d failed\n' "$PASSED" "$FAILED"
if [ "$FAILED" -ne 0 ]; then
  printf 'smoke test FAILED\n' >&2
  exit 1
fi
printf 'smoke test OK\n'
