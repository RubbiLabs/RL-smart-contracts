#!/usr/bin/env bash
#
# Deploy the Rubbi contracts (vault, subscriptions, payroll) to Stellar testnet.
#
# Prerequisites (see README.md, "Deploy to testnet"):
#   - stellar CLI 28.1.0, jq, git
#   - the five identities rubbi-admin, rubbi-operator, rubbi-treasury,
#     rubbi-keeper and rubbi-fees in the stellar CLI keystore, funded on
#     testnet (`stellar keys fund <name> --network testnet`)
#
# What it does, in order:
#   1. checks the tools and identities (never reads or prints a secret key);
#   2. resolves the USDC Stellar Asset Contract id from the issuer;
#   3. makes sure rubbi-treasury and rubbi-fees have a USDC trustline,
#      creating one (change_trust signed by that identity) when missing;
#   4. builds the contracts with `stellar contract build`;
#   5. deploys the three contracts with their constructor arguments, signed
#      by rubbi-admin;
#   6. writes deployments/testnet.json (contract ids, wasm hashes, roles'
#      public addresses, git commit, timestamp).
#
# Usage:
#   scripts/deploy-testnet.sh [--dry-run] [--force]
#
#   --dry-run   print every command that would run (public addresses only)
#               and deploy nothing. Read-only lookups still run.
#   --force     allow overwriting an existing deployments/testnet.json; the
#               old file is first copied to deployments/testnet.<timestamp>.json
#
# Environment:
#   USDC_ISSUER   USDC issuer account (default: Circle's testnet issuer)
#
# Portable to bash 3.2 (macOS default).

set -euo pipefail

# ---------------------------------------------------------------- settings --
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NETWORK="testnet"
# The stellar CLI's built-in testnet entry (`stellar network ls --long`).
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
RPC_URL="https://soroban-testnet.stellar.org"
# Circle's testnet USDC issuer, from
# https://developers.circle.com/stablecoins/usdc-contract-addresses
USDC_ISSUER="${USDC_ISSUER:-GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5}"
FEE_BPS=50
DEPLOYER="rubbi-admin"
ADMIN_ID="rubbi-admin"
OPERATOR_ID="rubbi-operator"
TREASURY_ID="rubbi-treasury"
KEEPER_ID="rubbi-keeper"
FEES_ID="rubbi-fees"
WASM_DIR="$REPO_ROOT/target/wasm32v1-none/release"
OUT_FILE="$REPO_ROOT/deployments/testnet.json"

DRY_RUN=0
FORCE=0

# ----------------------------------------------------------------- helpers --
log() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

# Print a command the way it would be typed, one argument per word.
show_cmd() { printf '  $'; printf ' %q' "$@"; printf '\n'; }

# Run a state-changing command, or only print it in dry-run mode.
run() {
  show_cmd "$@"
  if [ "$DRY_RUN" -eq 0 ]; then
    "$@"
  fi
}

is_g_address() { [[ "$1" =~ ^G[A-Z2-7]{55}$ ]]; }
is_c_address() { [[ "$1" =~ ^C[A-Z2-7]{55}$ ]]; }

# Public address of a stellar CLI identity. Fails if the identity is missing.
public_key() { stellar keys public-key "$1" 2>/dev/null; }

# Number of USDC:$USDC_ISSUER trustline entries of an account (0 or 1).
usdc_trustline_count() {
  stellar ledger entry fetch trustline \
    --account "$1" --asset "USDC:$USDC_ISSUER" \
    --network "$NETWORK" --output json | jq -r '.entries | length'
}

# Create the USDC trustline for an identity unless it already has one.
ensure_trustline() {
  local identity="$1" address="$2" count
  count="$(usdc_trustline_count "$address")" \
    || die "could not query the USDC trustline of $identity ($address)"
  case "$count" in
    0)
      log "$identity ($address) has no USDC trustline; creating it"
      run stellar tx new change-trust \
        --source-account "$identity" \
        --line "USDC:$USDC_ISSUER" \
        --network "$NETWORK"
      ;;
    1)
      log "$identity ($address) already trusts USDC:$USDC_ISSUER"
      ;;
    *)
      die "unexpected trustline lookup result for $identity: '$count'"
      ;;
  esac
}

# Deploy one contract. Prints the contract id (or a placeholder in dry-run).
# Arguments: name, wasm file, constructor arguments...
deploy_contract() {
  local name="$1" wasm="$2"
  shift 2
  # stdout is captured by the caller, so the echoed command goes to stderr.
  show_cmd stellar contract deploy \
    --wasm "$wasm" --network "$NETWORK" --source-account "$DEPLOYER" -- "$@" >&2
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '<%s-contract-id>\n' "$name"
    return 0
  fi
  local id
  id="$(stellar contract deploy \
    --wasm "$wasm" --network "$NETWORK" --source-account "$DEPLOYER" -- "$@")"
  is_c_address "$id" || die "unexpected deploy output for $name: '$id'"
  printf '%s\n' "$id"
}

wasm_hash() {
  if [ "$DRY_RUN" -eq 1 ] && [ ! -f "$1" ]; then
    printf '<%s-wasm-hash>\n' "$(basename "$1" .wasm)"
  else
    stellar contract info hash --wasm "$1"
  fi
}

# -------------------------------------------------------------------- args --
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --force) FORCE=1 ;;
    -h|--help) usage 0 ;;
    *) printf 'error: unknown argument: %s\n\n' "$1" >&2; usage 1 ;;
  esac
  shift
done

# ------------------------------------------------------------- 1. checks --
log "Checking tools"
for tool in stellar jq git; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' is not installed or not on PATH"
done
stellar --version | head -n 1

if [ -f "$OUT_FILE" ] && [ "$DRY_RUN" -eq 0 ]; then
  if [ "$FORCE" -eq 0 ]; then
    die "$OUT_FILE already exists. Re-running would deploy a second set of contracts.
       Pass --force to redeploy; the current file is then kept as deployments/testnet.<timestamp>.json."
  fi
  backup="$REPO_ROOT/deployments/testnet.$(date -u +%Y%m%dT%H%M%SZ).json"
  log "Keeping the existing deployment record as $backup"
  cp "$OUT_FILE" "$backup"
elif [ -f "$OUT_FILE" ]; then
  if [ "$FORCE" -eq 0 ]; then
    warn "$OUT_FILE exists; a real run needs --force (it would back the file up first)"
  else
    log "Would copy $OUT_FILE to deployments/testnet.<timestamp>.json"
  fi
fi

is_g_address "$USDC_ISSUER" || die "USDC_ISSUER '$USDC_ISSUER' is not a Stellar account address (G..., 56 chars)"

log "Checking identities (public keys only)"
missing=""
ADMIN_PK=""; OPERATOR_PK=""; TREASURY_PK=""; KEEPER_PK=""; FEES_PK=""
for identity in "$ADMIN_ID" "$OPERATOR_ID" "$TREASURY_ID" "$KEEPER_ID" "$FEES_ID"; do
  if pk="$(public_key "$identity")" && is_g_address "$pk"; then
    printf '  %-16s %s\n' "$identity" "$pk"
    case "$identity" in
      "$ADMIN_ID") ADMIN_PK="$pk" ;;
      "$OPERATOR_ID") OPERATOR_PK="$pk" ;;
      "$TREASURY_ID") TREASURY_PK="$pk" ;;
      "$KEEPER_ID") KEEPER_PK="$pk" ;;
      "$FEES_ID") FEES_PK="$pk" ;;
    esac
  else
    missing="$missing $identity"
  fi
done
if [ -n "$missing" ]; then
  die "missing stellar CLI identities:$missing
       Create them with: stellar keys generate <name> --network $NETWORK --fund
       (see README.md, \"Deploy to testnet\")."
fi

# ------------------------------------------------------------ 2. USDC SAC --
log "Resolving the USDC Stellar Asset Contract for USDC:$USDC_ISSUER"
show_cmd stellar contract id asset --asset "USDC:$USDC_ISSUER" --network "$NETWORK"
USDC_SAC="$(stellar contract id asset --asset "USDC:$USDC_ISSUER" --network "$NETWORK")"
is_c_address "$USDC_SAC" || die "could not resolve the USDC SAC id (got '$USDC_SAC')"
printf '  USDC SAC: %s\n' "$USDC_SAC"

# ----------------------------------------------------------- 3. trustlines --
log "USDC trustlines for the accounts that receive USDC"
ensure_trustline "$TREASURY_ID" "$TREASURY_PK"
ensure_trustline "$FEES_ID" "$FEES_PK"

# ---------------------------------------------------------------- 4. build --
log "Building contracts"
cd "$REPO_ROOT"
run stellar contract build
VAULT_WASM="$WASM_DIR/rubbi_vault.wasm"
SUBS_WASM="$WASM_DIR/rubbi_subscriptions.wasm"
PAYROLL_WASM="$WASM_DIR/rubbi_payroll.wasm"
if [ "$DRY_RUN" -eq 0 ]; then
  for wasm in "$VAULT_WASM" "$SUBS_WASM" "$PAYROLL_WASM"; do
    [ -f "$wasm" ] || die "build did not produce $wasm"
  done
fi

# --------------------------------------------------------------- 5. deploy --
log "Deploying vault (admin, operator, treasury, token)"
VAULT_ID="$(deploy_contract vault "$VAULT_WASM" \
  --admin "$ADMIN_PK" --operator "$OPERATOR_PK" --treasury "$TREASURY_PK" --token "$USDC_SAC")"
printf '  vault: %s\n' "$VAULT_ID"

log "Deploying subscriptions (admin, token, fee_recipient, fee_bps=$FEE_BPS)"
SUBS_ID="$(deploy_contract subscriptions "$SUBS_WASM" \
  --admin "$ADMIN_PK" --token "$USDC_SAC" --fee_recipient "$FEES_PK" --fee_bps "$FEE_BPS")"
printf '  subscriptions: %s\n' "$SUBS_ID"

log "Deploying payroll (admin, token, fee_recipient, fee_bps=$FEE_BPS)"
PAYROLL_ID="$(deploy_contract payroll "$PAYROLL_WASM" \
  --admin "$ADMIN_PK" --token "$USDC_SAC" --fee_recipient "$FEES_PK" --fee_bps "$FEE_BPS")"
printf '  payroll: %s\n' "$PAYROLL_ID"

# ---------------------------------------------------------------- 6. record --
log "Writing $OUT_FILE"
VAULT_HASH="$(wasm_hash "$VAULT_WASM")"
SUBS_HASH="$(wasm_hash "$SUBS_WASM")"
PAYROLL_HASH="$(wasm_hash "$PAYROLL_WASM")"
GIT_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
GIT_DIRTY=false
if [ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=no)" ]; then
  GIT_DIRTY=true
  warn "the working tree has uncommitted changes; git_commit may not describe the deployed code"
fi
DEPLOYED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

RECORD="$(jq -n \
  --arg network "$NETWORK" \
  --arg network_passphrase "$NETWORK_PASSPHRASE" \
  --arg rpc_url "$RPC_URL" \
  --arg usdc_issuer "$USDC_ISSUER" \
  --arg usdc_sac "$USDC_SAC" \
  --arg vault_id "$VAULT_ID" --arg vault_hash "$VAULT_HASH" \
  --arg subs_id "$SUBS_ID" --arg subs_hash "$SUBS_HASH" \
  --arg payroll_id "$PAYROLL_ID" --arg payroll_hash "$PAYROLL_HASH" \
  --argjson fee_bps "$FEE_BPS" \
  --arg admin "$ADMIN_PK" --arg operator "$OPERATOR_PK" --arg treasury "$TREASURY_PK" \
  --arg keeper "$KEEPER_PK" --arg fees "$FEES_PK" \
  --arg git_commit "$GIT_COMMIT" --argjson git_dirty "$GIT_DIRTY" \
  --arg deployed_at "$DEPLOYED_AT" \
  '{
    network: $network,
    network_passphrase: $network_passphrase,
    rpc_url: $rpc_url,
    usdc_issuer: $usdc_issuer,
    usdc_sac: $usdc_sac,
    contracts: {
      vault: {
        contract_id: $vault_id, wasm_hash: $vault_hash,
        admin: $admin, operator: $operator, treasury: $treasury, token: $usdc_sac
      },
      subscriptions: {
        contract_id: $subs_id, wasm_hash: $subs_hash,
        admin: $admin, token: $usdc_sac, fee_recipient: $fees, fee_bps: $fee_bps
      },
      payroll: {
        contract_id: $payroll_id, wasm_hash: $payroll_hash,
        admin: $admin, token: $usdc_sac, fee_recipient: $fees, fee_bps: $fee_bps
      }
    },
    roles: {
      admin: $admin, operator: $operator, treasury: $treasury,
      keeper: $keeper, fee_recipient: $fees
    },
    git_commit: $git_commit,
    git_dirty: $git_dirty,
    deployed_at: $deployed_at
  }')"

if [ "$DRY_RUN" -eq 1 ]; then
  log "Dry run: nothing was deployed. $OUT_FILE would contain:"
  printf '%s\n' "$RECORD"
  exit 0
fi

mkdir -p "$(dirname "$OUT_FILE")"
tmp="$(mktemp "$OUT_FILE.XXXXXX")"
printf '%s\n' "$RECORD" > "$tmp"
mv "$tmp" "$OUT_FILE"
printf '%s\n' "$RECORD"

log "Done. Next steps:"
printf '  scripts/smoke-testnet.sh          # read-only checks against the new deployment\n'
printf '  git add deployments/testnet.json  # commit the deployment record\n'
