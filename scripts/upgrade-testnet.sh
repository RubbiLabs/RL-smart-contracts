#!/usr/bin/env bash
#
# Upgrade the deployed Rubbi testnet contracts (vault, subscriptions,
# payroll) in place to an attested release built by
# .github/workflows/release.yml. Testnet only.
#
# Prerequisites (see docs/TESTNET.md, "Release and upgrade"):
#   - stellar CLI 28.1.0, gh (logged in), jq
#   - the identity rubbi-admin in the stellar CLI keystore, funded on testnet
#   - the release workflow has published, for tag vX.Y.Z, the releases
#     vX.Y.Z_<package>_cli28.1.0 with the asset <package>_vX.Y.Z.wasm
#
# What it does, in order:
#   1. checks the tools, the network (testnet only; any other network is
#      refused) and that rubbi-admin is the on-chain admin of every
#      contract (never reads or prints a secret key);
#   2. downloads the three release assets, prints their SHA-256 and verifies
#      each build attestation: signed by the pinned Stellar Expert workflow,
#      for this repository, at refs/tags/<tag>;
#   3. for each contract: uploads the WASM (`stellar contract upload`),
#      checks that the returned hash is the asset's SHA-256, then calls
#      `upgrade --new_wasm_hash <hash>` signed by rubbi-admin. A contract
#      that already runs the release is left alone;
#   4. fetches each contract's WASM from the network and checks that its
#      SHA-256 equals the release asset's;
#   5. records the release tag, WASM file names, hashes, source commit and
#      transaction hashes in deployments/testnet.json (after each upgrade, so
#      a partial run is recorded too).
#
# Exits non-zero if anything fails, including an on-chain hash that does not
# match its release asset.
#
# Usage:
#   scripts/upgrade-testnet.sh <tag> [--dry-run]
#
#   <tag>       the release tag, vX.Y.Z (the three crates have that version)
#   --dry-run   run steps 1, 2 and the read-only part of 4, print the
#               commands of step 3, submit nothing and write nothing
#
# Portable to bash 3.2 (macOS default).

set -euo pipefail

# ---------------------------------------------------------------- settings --
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="RubbiLabs/RL-smart-contracts"
NETWORK="testnet"
# The only network this script talks to. Passed explicitly to every stellar
# command instead of a named network, so a redefined "testnet" alias or a
# STELLAR_* environment variable cannot redirect it.
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
RPC_URL="https://soroban-testnet.stellar.org"
ADMIN_ID="rubbi-admin"
CONTRACTS="vault subscriptions payroll"
# stellar-cli version used by the pinned release workflow; part of the
# release name.
RELEASE_CLI="28.1.0"
# The reusable workflow that builds and attests releases, and the commit
# release.yml pins it to.
SIGNER_WORKFLOW="stellar-expert/soroban-build-workflow/.github/workflows/release.yml"
SIGNER_DIGEST="6b16426dd3a61715ddddf83a2df322a6371fe071"
OUT_FILE="$REPO_ROOT/deployments/testnet.json"

DRY_RUN=0
TAG=""

unset STELLAR_NETWORK STELLAR_RPC_URL STELLAR_NETWORK_PASSPHRASE STELLAR_CONTRACT_ID

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

is_g_address() { [[ "$1" =~ ^G[A-Z2-7]{55}$ ]]; }
is_c_address() { [[ "$1" =~ ^C[A-Z2-7]{55}$ ]]; }
is_hash() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# Cargo package name of a contract directory name.
package_of() { printf 'rubbi-%s\n' "$1"; }

# Value of a jq path in the deployment record.
record() { jq -r "$1 // empty" "$OUT_FILE"; }

# Transaction hash from the CLI's stderr. stellar-cli 28.1.0 logs
# "Signing transaction: <64 hex>" for every transaction it signs; some builds
# say "Transaction hash is". Prints the last hash found, or nothing.
extract_tx_hash() {
  printf '%s\n' "$1" \
    | grep -E -o '(Signing transaction:|Transaction hash is) *[0-9a-f]{64}' \
    | grep -E -o '[0-9a-f]{64}' | tail -n 1 || true
}

# Read-only contract call (simulated, never submitted). Prints the result
# with JSON string quotes removed.
read_call() {
  local contract="$1"
  shift
  stellar contract invoke --id "$contract" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" \
    --source-account "$ADMIN_ID" --send=no -- "$@" 2>/dev/null | jq -r '.'
}

# SHA-256 of the WASM a contract runs, from `stellar contract fetch`.
onchain_hash() {
  local contract="$1" out="$2"
  stellar contract fetch --id "$contract" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" \
    --out-file "$out" >/dev/null 2>&1 || return 1
  sha256 "$out"
}

# -------------------------------------------------------------------- args --
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage 0 ;;
    -*) printf 'error: unknown argument: %s\n\n' "$1" >&2; usage 1 ;;
    *)
      [ -z "$TAG" ] || die "only one tag may be given"
      TAG="$1"
      ;;
  esac
  shift
done
[ -n "$TAG" ] || { printf 'error: missing release tag\n\n' >&2; usage 1; }
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "tag '$TAG' is not of the form vX.Y.Z"
VERSION="${TAG#v}"

# ------------------------------------------------------------- 1. checks --
log "Checking tools"
for tool in stellar gh jq; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' is not installed or not on PATH"
done
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 \
  || die "neither sha256sum nor shasum is installed"
stellar --version | head -n 1
gh auth status >/dev/null 2>&1 || die "gh is not logged in (run: gh auth login)"

log "Checking the network (testnet only)"
[ -f "$OUT_FILE" ] || die "$OUT_FILE not found; deploy first with scripts/deploy-testnet.sh"
[ "$(record '.network')" = "$NETWORK" ] \
  || die "$OUT_FILE is for network '$(record '.network')'; this script only upgrades testnet"
[ "$(record '.network_passphrase')" = "$NETWORK_PASSPHRASE" ] \
  || die "$OUT_FILE has an unexpected network passphrase; refusing"
rpc_passphrase="$(curl -fsS -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getNetwork"}' "$RPC_URL" | jq -r '.result.passphrase // empty')" \
  || die "could not reach $RPC_URL"
[ "$rpc_passphrase" = "$NETWORK_PASSPHRASE" ] \
  || die "$RPC_URL reports passphrase '$rpc_passphrase', not testnet; refusing"
printf '  %s (%s)\n' "$RPC_URL" "$NETWORK_PASSPHRASE"

log "Checking $ADMIN_ID (public key only) against each contract's admin"
if ! ADMIN_PK="$(stellar keys public-key "$ADMIN_ID" 2>/dev/null)" || ! is_g_address "$ADMIN_PK"; then
  die "identity $ADMIN_ID not found in the stellar CLI keystore"
fi
printf '  %-16s %s\n' "$ADMIN_ID" "$ADMIN_PK"
for name in $CONTRACTS; do
  id="$(record ".contracts.$name.contract_id")"
  is_c_address "$id" || die "no contract id for $name in $OUT_FILE"
  admin="$(read_call "$id" admin)" || die "could not read the admin of $name ($id)"
  [ "$admin" = "$ADMIN_PK" ] \
    || die "$name ($id) has admin $admin, not $ADMIN_ID ($ADMIN_PK)"
  printf '  %-14s %s  admin ok\n' "$name" "$id"
done

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# ---------------------------------------------- 2. download and verify --
log "Downloading and verifying the release assets of $TAG"
for name in $CONTRACTS; do
  package="$(package_of "$name")"
  release="${TAG}_${package}_cli${RELEASE_CLI}"
  wasm="${package}_v${VERSION}.wasm"
  mkdir -p "$WORK_DIR/$name"
  show_cmd gh release download "$release" --repo "$REPO" --pattern "$wasm" --dir "$WORK_DIR/$name"
  gh release download "$release" --repo "$REPO" --pattern "$wasm" --dir "$WORK_DIR/$name" \
    || die "could not download $wasm from release $release (has the release workflow finished for $TAG?)"
  file="$WORK_DIR/$name/$wasm"
  [ -f "$file" ] || die "release $release has no asset $wasm"
  hash="$(sha256 "$file")"
  printf '  %-14s %s  sha256 %s\n' "$name" "$wasm" "$hash"

  show_cmd gh attestation verify "$file" --repo "$REPO" \
    --signer-workflow "$SIGNER_WORKFLOW" --signer-digest "$SIGNER_DIGEST" \
    --source-ref "refs/tags/$TAG"
  # The attestation is signed by the reusable workflow, not by a workflow
  # of this repository, so --signer-workflow is required for it to verify.
  source_commit="$(gh attestation verify "$file" --repo "$REPO" \
    --signer-workflow "$SIGNER_WORKFLOW" --signer-digest "$SIGNER_DIGEST" \
    --source-ref "refs/tags/$TAG" --format json \
    --jq '.[0].verificationResult.signature.certificate.sourceRepositoryDigest')" \
    || die "attestation verification failed for $wasm"
  [[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] \
    || die "attestation of $wasm has no source commit (got '$source_commit')"
  printf '  %-14s attestation ok, built from %s at %s\n' "$name" "$REPO" "$source_commit"
  printf '%s\n' "$source_commit" > "$WORK_DIR/$name/source_commit"
done

# --------------------------------------------------------- 3. upgrade --
UPGRADED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Merge one contract's release into the deployment record.
write_record() {
  local name="$1" wasm="$2" hash="$3" source_commit="$4" upload_tx="$5" upgrade_tx="$6" old_hash="$7"
  local tmp
  tmp="$(mktemp "$OUT_FILE.XXXXXX")"
  jq --arg c "$name" --arg tag "$TAG" --arg release "${TAG}_$(package_of "$name")_cli${RELEASE_CLI}" \
    --arg wasm "$wasm" --arg hash "$hash" --arg commit "$source_commit" \
    --arg upload_tx "$upload_tx" --arg upgrade_tx "$upgrade_tx" --arg old "$old_hash" \
    --arg at "$UPGRADED_AT" --arg repo "$REPO" '
    .contracts[$c].previous_wasm_hashes =
      ((.contracts[$c].previous_wasm_hashes // [])
        + (if $old != $hash and $old != "" then [$old] else [] end))
    | .contracts[$c].wasm_hash = $hash
    | .contracts[$c].release = {
        tag: $tag,
        release: $release,
        release_url: ("https://github.com/" + $repo + "/releases/tag/" + $release),
        wasm_file: $wasm,
        sha256: $hash,
        source_commit: $commit,
        upload_tx: (if $upload_tx == "" then null else $upload_tx end),
        upgrade_tx: (if $upgrade_tx == "" then null else $upgrade_tx end),
        upgraded_at: $at
      }
    | .release_tag = $tag' "$OUT_FILE" > "$tmp"
  mv "$tmp" "$OUT_FILE"
}

log "Upgrading the contracts to $TAG"
for name in $CONTRACTS; do
  package="$(package_of "$name")"
  wasm="${package}_v${VERSION}.wasm"
  file="$WORK_DIR/$name/$wasm"
  hash="$(sha256 "$file")"
  id="$(record ".contracts.$name.contract_id")"
  old_hash="$(onchain_hash "$id" "$WORK_DIR/$name/before.wasm")" \
    || die "could not fetch the current WASM of $name ($id)"
  source_commit="$(cat "$WORK_DIR/$name/source_commit")"

  if [ "$old_hash" = "$hash" ]; then
    log "$name already runs $wasm ($hash); nothing to do"
    continue
  fi

  log "$name: upload $wasm"
  show_cmd stellar contract upload --wasm "$file" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" --source-account "$ADMIN_ID"
  log "$name: upgrade $old_hash -> $hash"
  show_cmd stellar contract invoke --id "$id" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" --source-account "$ADMIN_ID" \
    --send=yes -- upgrade --new_wasm_hash "$hash"
  if [ "$DRY_RUN" -eq 1 ]; then
    continue
  fi

  err="$WORK_DIR/$name/upload.err"
  uploaded="$(stellar contract upload --wasm "$file" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" \
    --source-account "$ADMIN_ID" 2>"$err")" || { cat "$err" >&2; die "upload of $wasm failed"; }
  [ "$uploaded" = "$hash" ] \
    || die "upload of $wasm returned hash '$uploaded', expected $hash"
  upload_tx="$(extract_tx_hash "$(cat "$err")")"
  if [ -z "$upload_tx" ]; then
    printf '  no upload transaction (the WASM may already have been on the network)\n'
  else
    printf '  upload tx %s\n' "$upload_tx"
  fi

  err="$WORK_DIR/$name/upgrade.err"
  stellar contract invoke --id "$id" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" --source-account "$ADMIN_ID" \
    --send=yes -- upgrade --new_wasm_hash "$hash" >/dev/null 2>"$err" \
    || { cat "$err" >&2; die "upgrade of $name ($id) failed"; }
  upgrade_tx="$(extract_tx_hash "$(cat "$err")")"
  is_hash "$upgrade_tx" || warn "$name: no upgrade transaction hash found in the CLI output"
  printf '  upgrade tx %s\n' "${upgrade_tx:-<unknown>}"

  write_record "$name" "$wasm" "$hash" "$source_commit" "$upload_tx" "$upgrade_tx" "$old_hash"
done

# ------------------------------------------------------------- 4. verify --
log "Comparing each contract's on-chain WASM with its release asset"
mismatch=0
for name in $CONTRACTS; do
  package="$(package_of "$name")"
  wasm="${package}_v${VERSION}.wasm"
  expected="$(sha256 "$WORK_DIR/$name/$wasm")"
  id="$(record ".contracts.$name.contract_id")"
  actual="$(onchain_hash "$id" "$WORK_DIR/$name/onchain.wasm")" \
    || die "could not fetch the WASM of $name ($id)"
  if [ "$actual" = "$expected" ]; then
    printf '  %-14s ok        %s\n' "$name" "$actual"
  else
    printf '  %-14s MISMATCH  on-chain %s, release %s\n' "$name" "$actual" "$expected" >&2
    mismatch=1
  fi
done

if [ "$DRY_RUN" -eq 1 ]; then
  log "Dry run: nothing was submitted and $OUT_FILE was not changed"
  [ "$mismatch" -eq 0 ] || printf '  (a mismatch is expected before the upgrade has run)\n'
  exit 0
fi
[ "$mismatch" -eq 0 ] || die "on-chain WASM does not match the release for at least one contract"

log "Done. $OUT_FILE now records $TAG. Next steps:"
printf '  scripts/smoke-testnet.sh          # read-only checks against the upgraded contracts\n'
printf '  check "source verified" on Stellar Expert and Stellar Lab, then update docs/TESTNET.md and README.md\n'
printf '  git add deployments/testnet.json  # commit the deployment record\n'
