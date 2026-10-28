#!/usr/bin/env bash
#
# Exercise the deployed Rubbi testnet contracts with dedicated demo accounts so
# that their explorer pages show real activity. Demo usage only: the three demo
# identities are not real users and hold only testnet USDC.
#
# Prerequisites (see README.md, "Testnet activity"):
#   - stellar CLI 28.1.0, jq, shasum or sha256sum
#   - deployments/testnet.json (written by scripts/deploy-testnet.sh)
#   - the role identities rubbi-admin, rubbi-operator and rubbi-treasury in the
#     stellar CLI keystore (the script never reads or prints a secret key)
#   - demo-user funded with at least 15 USDC from https://faucet.circle.com
#     (the script prints the address and stops when the balance is too low)
#
# What a normal (day-1) run does, in order:
#   1. creates the identities demo-user, demo-merchant and demo-staff when
#      missing (`stellar keys generate <name> --network testnet --fund`);
#   2. makes sure all three have a USDC trustline;
#   3. checks demo-user holds >= 15 USDC, otherwise prints the faucet
#      instructions and exits 0;
#   4. vault: demo-user deposits 5 USDC and withdraws 1; rubbi-operator settles
#      a 2 USDC card load for demo-user; rubbi-treasury refunds 1 USDC;
#   5. subscriptions: rubbi-admin allowlists demo-merchant (skipped when it is
#      already a merchant); demo-merchant creates "Demo plan" (1 USDC / day,
#      reused when recorded); demo-user approves 3 USDC for the subscriptions
#      contract and subscribes (skipped when already subscribed);
#   6. payroll: demo-user funds 3 USDC of escrow, creates one schedule paying
#      demo-staff 1 USDC / day ("Demo salary", first due in 30 s), waits for it
#      to fall due and disburses it once;
#   7. records ids, references and transaction hashes in
#      deployments/testnet-demo.json and prints a stellar.expert link per tx.
#
# Usage:
#   scripts/demo-testnet.sh [--dry-run] [--day2]
#
#   --dry-run   print every command that would run and submit nothing.
#               Read-only lookups still run where the accounts exist.
#   --day2      instead of steps 4-6, call `charge` on the recorded demo
#               subscription and `disburse` on the recorded demo schedule.
#               Both are time-gated; "not due yet" is reported cleanly.
#
# Portable to bash 3.2 (macOS default).

set -euo pipefail

# ---------------------------------------------------------------- settings --
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NETWORK="testnet"
DEPLOY_FILE="$REPO_ROOT/deployments/testnet.json"
DEMO_FILE="$REPO_ROOT/deployments/testnet-demo.json"
EXPLORER="https://stellar.expert/explorer/testnet"
FAUCET_URL="https://faucet.circle.com"

ADMIN_ID="rubbi-admin"
OPERATOR_ID="rubbi-operator"
TREASURY_ID="rubbi-treasury"
USER_ID="demo-user"
MERCHANT_ID="demo-merchant"
STAFF_ID="demo-staff"
# Identities this script must never create, fund or sign with.
PROTECTED_IDS="test-alice test-bob"

# Amounts in stroops (USDC has 7 decimals).
ONE_USDC=10000000
MIN_USER_BALANCE=$((15 * ONE_USDC))
DEPOSIT_AMOUNT=$((5 * ONE_USDC))
WITHDRAW_AMOUNT=$ONE_USDC
SPEND_AMOUNT=$((2 * ONE_USDC))
REFUND_AMOUNT=$ONE_USDC
PLAN_NAME="Demo plan"
PLAN_AMOUNT=$ONE_USDC
PLAN_PERIOD=86400
APPROVE_PERIODS=3
APPROVE_AMOUNT=$((APPROVE_PERIODS * PLAN_AMOUNT))
ESCROW_AMOUNT=$((3 * ONE_USDC))
SALARY_AMOUNT=$ONE_USDC
SALARY_PERIOD=86400
SALARY_LABEL="Demo salary"
FIRST_DUE_DELAY=30
# Ledgers close every ~5 s (measured; see contracts/vault/src/storage.rs).
DAY_IN_LEDGERS=17280
APPROVE_BUFFER_LEDGERS=$DAY_IN_LEDGERS

DRY_RUN=0
DAY2=0

# ----------------------------------------------------------------- helpers --
log() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

show_cmd() { printf '  $'; printf ' %q' "$@"; printf '\n'; }

# Variants for functions whose stdout is captured by the caller: everything
# that is not the returned value must go to stderr.
log_err() { log "$@" >&2; }
note_err() { note "$@" >&2; }
show_cmd_err() { show_cmd "$@" >&2; }

# A captured identity address must be a real G address, or (dry-run only) the
# placeholder for an identity that does not exist yet.
require_address() {
  local what="$1" value="$2"
  if is_g_address "$value"; then return 0; fi
  if [ "$DRY_RUN" -eq 1 ] && is_placeholder "$value"; then return 0; fi
  die "$what resolved to '$value', which is not a Stellar account address (G..., 56 chars)"
}

is_g_address() { [[ "$1" =~ ^G[A-Z2-7]{55}$ ]]; }
is_c_address() { [[ "$1" =~ ^C[A-Z2-7]{55}$ ]]; }
is_placeholder() { case "$1" in \<*\>) return 0 ;; *) return 1 ;; esac; }

public_key() { stellar keys public-key "$1" 2>/dev/null; }

sha256_hex() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
  else
    printf '%s' "$1" | sha256sum | awk '{print $1}'
  fi
}

deploy_field() {
  local value
  value="$(jq -er "$1" "$DEPLOY_FILE")" || die "missing field $1 in $DEPLOY_FILE"
  printf '%s\n' "$value"
}

# Value from the demo record (empty when the file or the field is missing).
demo_field() {
  [ -f "$DEMO_FILE" ] || return 0
  jq -r "$1 // empty" "$DEMO_FILE" 2>/dev/null || true
}

latest_ledger() {
  stellar ledger latest --network "$NETWORK" --output json | jq -er '.sequence'
}

# Transactions recorded during this run, as a JSON array.
TXS='[]'
record_tx() {
  local step="$1" hash="$2"
  TXS="$(printf '%s' "$TXS" | jq --arg s "$step" --arg h "$hash" --arg e "$EXPLORER" \
    '. + [{step: $s, hash: $h, link: ($e + "/tx/" + $h)}]')"
}

# Run a state-changing CLI command. stdout (the return value) ends up in
# LAST_RESULT, the transaction hash in LAST_HASH, the CLI's stderr in
# LAST_STDERR. Returns the command's exit status. In dry-run mode the command
# is only printed and LAST_RESULT is set to the given placeholder.
LAST_RESULT=""; LAST_HASH=""; LAST_STDERR=""
# Set to 0 around commands that do not log a transaction hash.
WANT_HASH=1
send_cmd() {
  local step="$1" placeholder="$2"
  shift 2
  show_cmd "$@"
  if [ "$DRY_RUN" -eq 1 ]; then
    LAST_RESULT="$placeholder"; LAST_HASH="<$step-tx-hash>"; LAST_STDERR=""
    return 0
  fi
  local err status=0
  err="$(mktemp)"
  LAST_RESULT="$("$@" 2>"$err")" || status=$?
  LAST_STDERR="$(cat "$err")"
  rm -f "$err"
  LAST_HASH="$(printf '%s\n' "$LAST_STDERR" | grep -o 'Transaction hash is [0-9a-f]*' | tail -n 1 | awk '{print $NF}')"
  if [ "$status" -ne 0 ]; then
    return "$status"
  fi
  if [ -n "$LAST_HASH" ]; then
    note "tx: $EXPLORER/tx/$LAST_HASH"
    record_tx "$step" "$LAST_HASH"
  elif [ "$WANT_HASH" -eq 1 ]; then
    warn "$step: no transaction hash found in the CLI output"
  fi
  return 0
}

# Like send_cmd, but a failure aborts the script with the CLI's stderr.
must_send() {
  local step="$1"
  if ! send_cmd "$@"; then
    printf '%s\n' "$LAST_STDERR" >&2
    die "$step failed"
  fi
}

# Contract error code (e.g. 14) found in LAST_STDERR, or empty.
last_contract_error() {
  printf '%s\n' "$LAST_STDERR" | grep -o 'Error(Contract, #[0-9]*)' | head -n 1 | grep -o '[0-9]*$' || true
}

# Signed contract invocation: step, placeholder, source identity, contract, fn, args...
invoke() {
  local step="$1" placeholder="$2" source="$3" contract="$4"
  shift 4
  send_cmd "$step" "$placeholder" stellar contract invoke \
    --id "$contract" --network "$NETWORK" --source-account "$source" --send=yes -- "$@"
}

must_invoke() {
  local step="$1"
  if ! invoke "$@"; then
    printf '%s\n' "$LAST_STDERR" >&2
    die "$step failed"
  fi
}

# Read-only simulation: contract, fn, args... Prints the decoded result.
read_call() {
  local contract="$1"
  shift
  stellar contract invoke \
    --id "$contract" --network "$NETWORK" --source-account "$ADMIN_ID" \
    --send=no -- "$@" 2>/dev/null
}

usdc_trustline_count() {
  stellar ledger entry fetch trustline \
    --account "$1" --asset "USDC:$USDC_ISSUER" \
    --network "$NETWORK" --output json | jq -r '.entries | length'
}

# USDC balance of an account in stroops ("0" when the trustline is missing).
usdc_balance() {
  local out
  if out="$(read_call "$USDC_SAC" balance --id "$1")"; then
    printf '%s\n' "$out" | tr -d '"'
  else
    printf '0\n'
  fi
}

refuse_protected() {
  local name
  for name in $PROTECTED_IDS; do
    [ "$1" = "$name" ] && die "refusing to touch the protected identity $name"
  done
  return 0
}

# Create a demo identity when it does not exist. The caller captures stdout,
# so the ONLY thing written to stdout is the public key (or, in dry-run mode,
# a placeholder for an identity that does not exist yet); logs go to stderr.
ensure_identity() {
  local identity="$1" pk
  refuse_protected "$identity"
  if pk="$(public_key "$identity")" && is_g_address "$pk"; then
    note_err "$identity exists: $pk"
    printf '%s\n' "$pk"
    return 0
  fi
  log_err "$identity is missing; creating and funding it with Friendbot"
  show_cmd_err stellar keys generate "$identity" --network "$NETWORK" --fund
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '<%s-public-key>\n' "$identity"
    return 0
  fi
  stellar keys generate "$identity" --network "$NETWORK" --fund >&2
  pk="$(public_key "$identity")"
  is_g_address "$pk" || die "could not read the public key of $identity after creating it"
  printf '%s\n' "$pk"
}

ensure_trustline() {
  local identity="$1" address="$2" count
  if is_placeholder "$address"; then
    note "$identity does not exist yet; would create its USDC trustline"
    show_cmd stellar tx new change-trust --source-account "$identity" --line "USDC:$USDC_ISSUER" --network "$NETWORK"
    return 0
  fi
  count="$(usdc_trustline_count "$address")" \
    || die "could not query the USDC trustline of $identity ($address)"
  case "$count" in
    0)
      log "$identity ($address) has no USDC trustline; creating it"
      # `stellar tx new` does not log a transaction hash like `contract invoke`
      # does, so the trustline is confirmed by reading the ledger afterwards.
      WANT_HASH=0
      must_send "trustline-$identity" "" stellar tx new change-trust \
        --source-account "$identity" --line "USDC:$USDC_ISSUER" --network "$NETWORK"
      WANT_HASH=1
      if [ "$DRY_RUN" -eq 0 ]; then
        count="$(usdc_trustline_count "$address")" \
          || die "could not re-query the USDC trustline of $identity ($address)"
        [ "$count" = "1" ] || die "trustline for $identity was submitted but is not on the ledger (lookup returned '$count')"
        note "trustline confirmed: $identity trusts USDC:$USDC_ISSUER"
      fi
      ;;
    1) note "$identity already trusts USDC:$USDC_ISSUER" ;;
    *) die "unexpected trustline lookup result for $identity: '$count'" ;;
  esac
}

# -------------------------------------------------------------------- args --
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --day2) DAY2=1 ;;
    -h|--help) usage 0 ;;
    *) printf 'error: unknown argument: %s\n\n' "$1" >&2; usage 1 ;;
  esac
  shift
done

# ------------------------------------------------------------- 0. checks --
log "Checking tools"
for tool in stellar jq; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' is not installed or not on PATH"
done
command -v shasum >/dev/null 2>&1 || command -v sha256sum >/dev/null 2>&1 \
  || die "neither shasum nor sha256sum is available"
stellar --version | head -n 1
[ -f "$DEPLOY_FILE" ] || die "$DEPLOY_FILE not found; run scripts/deploy-testnet.sh first"

USDC_ISSUER="$(deploy_field '.usdc_issuer')"
USDC_SAC="$(deploy_field '.usdc_sac')"
VAULT_ID="$(deploy_field '.contracts.vault.contract_id')"
SUBS_ID="$(deploy_field '.contracts.subscriptions.contract_id')"
PAYROLL_ID="$(deploy_field '.contracts.payroll.contract_id')"
is_g_address "$USDC_ISSUER" || die "usdc_issuer in $DEPLOY_FILE is not a G address"
for id in "$USDC_SAC" "$VAULT_ID" "$SUBS_ID" "$PAYROLL_ID"; do
  is_c_address "$id" || die "contract id '$id' in $DEPLOY_FILE is not a C address"
done
note "vault $VAULT_ID"
note "subscriptions $SUBS_ID"
note "payroll $PAYROLL_ID"
note "USDC SAC $USDC_SAC"

log "Checking role identities (public keys only)"
missing=""
for identity in "$ADMIN_ID" "$OPERATOR_ID" "$TREASURY_ID"; do
  if pk="$(public_key "$identity")" && is_g_address "$pk"; then
    printf '  %-16s %s\n' "$identity" "$pk"
  else
    missing="$missing $identity"
  fi
done
[ -z "$missing" ] || die "missing stellar CLI identities:$missing (see README.md, \"Deploy to testnet\")"

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
RUN_STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if [ "$DRY_RUN" -eq 1 ]; then
  log "Dry run: nothing will be signed or submitted"
fi

# --------------------------------------------------------- 1. identities --
log "Demo identities"
USER_PK="$(ensure_identity "$USER_ID")"
MERCHANT_PK="$(ensure_identity "$MERCHANT_ID")"
STAFF_PK="$(ensure_identity "$STAFF_ID")"
require_address "$USER_ID" "$USER_PK"
require_address "$MERCHANT_ID" "$MERCHANT_PK"
require_address "$STAFF_ID" "$STAFF_PK"

# --------------------------------------------------------- 2. trustlines --
log "USDC trustlines for the demo accounts"
ensure_trustline "$USER_ID" "$USER_PK"
ensure_trustline "$MERCHANT_ID" "$MERCHANT_PK"
ensure_trustline "$STAFF_ID" "$STAFF_PK"

# ------------------------------------------------------------ 3. funding --
log "Checking demo-user's USDC balance (needs at least $((MIN_USER_BALANCE / ONE_USDC)) USDC)"
if is_placeholder "$USER_PK"; then
  note "demo-user does not exist yet; the balance check runs on a real run"
elif [ "$DAY2" -eq 0 ]; then
  USER_BALANCE="$(usdc_balance "$USER_PK")"
  note "demo-user holds $USER_BALANCE stroops ($((USER_BALANCE / ONE_USDC)) USDC)"
  if [ "$USER_BALANCE" -lt "$MIN_USER_BALANCE" ]; then
    if [ "$DRY_RUN" -eq 1 ]; then
      warn "a real run would stop here until demo-user is funded"
    else
      printf '\n'
      printf 'demo-user needs at least %s USDC on testnet and holds %s stroops.\n' \
        "$((MIN_USER_BALANCE / ONE_USDC))" "$USER_BALANCE"
      printf 'Fund this address at %s (choose "Stellar Testnet"), then re-run this script:\n\n' "$FAUCET_URL"
      printf '    %s\n\n' "$USER_PK"
      exit 0
    fi
  fi
fi

# Previously recorded demo state (reused where it still applies).
PLAN_ID="$(demo_field '.plan_id')"
SUB_ID="$(demo_field '.sub_id')"
SCHEDULE_ID="$(demo_field '.schedule_id')"
SPEND_REFERENCE=""; REFUND_REFERENCE=""

if [ "$DAY2" -eq 0 ]; then
  # -------------------------------------------------------------- 4. vault --
  log "vault: demo-user deposits $((DEPOSIT_AMOUNT / ONE_USDC)) USDC"
  must_invoke vault-deposit "" "$USER_ID" "$VAULT_ID" \
    deposit --user "$USER_PK" --amount "$DEPOSIT_AMOUNT"

  log "vault: demo-user withdraws $((WITHDRAW_AMOUNT / ONE_USDC)) USDC"
  must_invoke vault-withdraw "" "$USER_ID" "$VAULT_ID" \
    withdraw --user "$USER_PK" --amount "$WITHDRAW_AMOUNT"

  SPEND_REFERENCE="$(sha256_hex "rubbi-demo-$RUN_ID-card-load")"
  log "vault: rubbi-operator settles a $((SPEND_AMOUNT / ONE_USDC)) USDC card load for demo-user (reference $SPEND_REFERENCE)"
  must_invoke vault-settle-card-spend "" "$OPERATOR_ID" "$VAULT_ID" \
    settle_card_spend --user "$USER_PK" --amount "$SPEND_AMOUNT" --reference "$SPEND_REFERENCE"

  REFUND_REFERENCE="$(sha256_hex "rubbi-demo-$RUN_ID-card-unload")"
  log "vault: rubbi-treasury refunds $((REFUND_AMOUNT / ONE_USDC)) USDC to demo-user (reference $REFUND_REFERENCE)"
  must_invoke vault-credit-refund "" "$TREASURY_ID" "$VAULT_ID" \
    credit_refund --user "$USER_PK" --amount "$REFUND_AMOUNT" --reference "$REFUND_REFERENCE"

  # ------------------------------------------------------ 5. subscriptions --
  log "subscriptions: merchant allowlist"
  merchant_json="null"
  if ! is_placeholder "$MERCHANT_PK"; then
    merchant_json="$(read_call "$SUBS_ID" get_merchant --merchant "$MERCHANT_PK" || printf 'null')"
  fi
  if [ "$merchant_json" != "null" ] && [ -n "$merchant_json" ]; then
    note "demo-merchant is already a merchant: $merchant_json (add_merchant skipped)"
  else
    must_invoke subscriptions-add-merchant "" "$ADMIN_ID" "$SUBS_ID" \
      add_merchant --merchant "$MERCHANT_PK" --payout "$MERCHANT_PK"
  fi

  log "subscriptions: plan \"$PLAN_NAME\" ($((PLAN_AMOUNT / ONE_USDC)) USDC every $((PLAN_PERIOD / 86400)) day)"
  reuse_plan=0
  if [ -n "$PLAN_ID" ] && ! is_placeholder "$MERCHANT_PK"; then
    if plan_json="$(read_call "$SUBS_ID" get_plan --plan_id "$PLAN_ID")" \
       && [ "$(printf '%s' "$plan_json" | jq -r --arg m "$MERCHANT_PK" '(.merchant == $m) and .active')" = "true" ]; then
      reuse_plan=1
      note "reusing recorded plan $PLAN_ID: $plan_json"
    else
      warn "recorded plan $PLAN_ID is not an active demo-merchant plan; creating a new one"
    fi
  fi
  if [ "$reuse_plan" -eq 0 ]; then
    must_invoke subscriptions-create-plan "<plan-id>" "$MERCHANT_ID" "$SUBS_ID" \
      create_plan --merchant "$MERCHANT_PK" --name "$PLAN_NAME" --amount "$PLAN_AMOUNT" --period "$PLAN_PERIOD"
    PLAN_ID="$(printf '%s' "$LAST_RESULT" | tr -d '" ')"
    note "plan id: $PLAN_ID"
  fi

  log "subscriptions: demo-user approves $((APPROVE_AMOUNT / ONE_USDC)) USDC for the subscriptions contract"
  CURRENT_LEDGER="$(latest_ledger)" || die "could not read the latest ledger"
  LIVE_UNTIL=$((CURRENT_LEDGER + APPROVE_PERIODS * PLAN_PERIOD / 5 + APPROVE_BUFFER_LEDGERS))
  note "current ledger $CURRENT_LEDGER, allowance valid until ledger $LIVE_UNTIL (~$APPROVE_PERIODS days + 1 day buffer)"
  must_invoke usdc-approve "" "$USER_ID" "$USDC_SAC" \
    approve --from "$USER_PK" --spender "$SUBS_ID" --amount "$APPROVE_AMOUNT" --live_until_ledger "$LIVE_UNTIL"

  log "subscriptions: demo-user subscribes to plan $PLAN_ID"
  existing_sub="null"
  if ! is_placeholder "$USER_PK" && ! is_placeholder "$PLAN_ID"; then
    existing_sub="$(read_call "$SUBS_ID" sub_id_of --subscriber "$USER_PK" --plan_id "$PLAN_ID" || printf 'null')"
  fi
  if [ "$existing_sub" != "null" ] && [ -n "$existing_sub" ]; then
    SUB_ID="$(printf '%s' "$existing_sub" | tr -d '" ')"
    note "demo-user is already subscribed (subscription $SUB_ID); subscribe skipped"
  else
    must_invoke subscriptions-subscribe "<sub-id>" "$USER_ID" "$SUBS_ID" \
      subscribe --subscriber "$USER_PK" --plan_id "$PLAN_ID"
    SUB_ID="$(printf '%s' "$LAST_RESULT" | tr -d '" ')"
    note "subscription id: $SUB_ID (first period charged now)"
  fi

  # ------------------------------------------------------------ 6. payroll --
  log "payroll: demo-user funds $((ESCROW_AMOUNT / ONE_USDC)) USDC of escrow"
  must_invoke payroll-fund "" "$USER_ID" "$PAYROLL_ID" \
    fund --employer "$USER_PK" --amount "$ESCROW_AMOUNT"

  FIRST_DUE=$(( $(date +%s) + FIRST_DUE_DELAY ))
  log "payroll: one schedule paying demo-staff $((SALARY_AMOUNT / ONE_USDC)) USDC every $((SALARY_PERIOD / 86400)) day, first due at $FIRST_DUE (in ${FIRST_DUE_DELAY}s)"
  INPUTS="$(jq -cn --arg r "$STAFF_PK" --arg a "$SALARY_AMOUNT" --argjson p "$SALARY_PERIOD" --arg l "$SALARY_LABEL" \
    '[{recipient: $r, amount: $a, period: $p, label: $l}]')"
  must_invoke payroll-create-schedules "[<schedule-id>]" "$USER_ID" "$PAYROLL_ID" \
    create_schedules --employer "$USER_PK" --inputs "$INPUTS" --first_due "$FIRST_DUE"
  if [ "$DRY_RUN" -eq 1 ]; then
    SCHEDULE_ID="<schedule-id>"
  else
    SCHEDULE_ID="$(printf '%s' "$LAST_RESULT" | jq -r '.[0]')"
  fi
  note "schedule id: $SCHEDULE_ID"

  log "payroll: waiting until the schedule is due, then disbursing it once"
  if [ "$DRY_RUN" -eq 1 ]; then
    show_cmd stellar contract invoke --id "$PAYROLL_ID" --network "$NETWORK" \
      --source-account "$USER_ID" --send=yes -- disburse --schedule_id "$SCHEDULE_ID"
  else
    while [ "$(date +%s)" -lt $((FIRST_DUE + 5)) ]; do sleep 5; done
    attempt=0; disbursed=0
    while [ "$attempt" -lt 12 ]; do
      attempt=$((attempt + 1))
      if invoke payroll-disburse "" "$USER_ID" "$PAYROLL_ID" disburse --schedule_id "$SCHEDULE_ID"; then
        note "disburse returned $LAST_RESULT"
        [ "$LAST_RESULT" = "true" ] || warn "disburse returned false (see the disburse_failed event: insufficient escrow or unpayable recipient)"
        disbursed=1
        break
      fi
      if [ "$(last_contract_error)" = "14" ]; then
        note "not due yet on-chain (ledger time lags a little); retrying in 10 s"
        sleep 10
      else
        printf '%s\n' "$LAST_STDERR" >&2
        die "payroll-disburse failed"
      fi
    done
    [ "$disbursed" -eq 1 ] || die "the schedule did not become due within the wait window"
  fi
else
  # ------------------------------------------------------------- 7. day 2 --
  [ -n "$SUB_ID" ] || die "no sub_id recorded in $DEMO_FILE; run the script without --day2 first"
  [ -n "$SCHEDULE_ID" ] || die "no schedule_id recorded in $DEMO_FILE; run the script without --day2 first"

  log "day 2: charge subscription $SUB_ID (time-gated)"
  if invoke subscriptions-charge "<bool>" "$USER_ID" "$SUBS_ID" charge --sub_id "$SUB_ID"; then
    note "charge returned $LAST_RESULT"
    [ "$LAST_RESULT" = "true" ] || [ "$DRY_RUN" -eq 1 ] \
      || warn "charge returned false: the pull failed (allowance or balance), the subscription is PastDue"
  else
    case "$(last_contract_error)" in
      15) note "not due yet: subscription $SUB_ID is not chargeable before its next_due" ;;
      16) note "not chargeable: subscription $SUB_ID is paused or cancelled" ;;
      *) printf '%s\n' "$LAST_STDERR" >&2; die "subscriptions-charge failed" ;;
    esac
  fi

  log "day 2: disburse schedule $SCHEDULE_ID (time-gated)"
  if invoke payroll-disburse "<bool>" "$USER_ID" "$PAYROLL_ID" disburse --schedule_id "$SCHEDULE_ID"; then
    note "disburse returned $LAST_RESULT"
    [ "$LAST_RESULT" = "true" ] || [ "$DRY_RUN" -eq 1 ] \
      || warn "disburse returned false (insufficient escrow or unpayable recipient; the period stays owed)"
  else
    case "$(last_contract_error)" in
      14) note "not due yet: schedule $SCHEDULE_ID is not payable before its next_due" ;;
      11) note "not active: schedule $SCHEDULE_ID is paused or cancelled" ;;
      *) printf '%s\n' "$LAST_STDERR" >&2; die "payroll-disburse failed" ;;
    esac
  fi
fi

# ---------------------------------------------------------------- 8. record --
log "Recording the demo state in $DEMO_FILE"
MODE="day1"; [ "$DAY2" -eq 1 ] && MODE="day2"
PREVIOUS='{}'
[ -f "$DEMO_FILE" ] && PREVIOUS="$(cat "$DEMO_FILE")"
RECORD="$(printf '%s' "$PREVIOUS" | jq \
  --arg network "$NETWORK" \
  --arg run_id "$RUN_ID" --arg mode "$MODE" \
  --arg started "$RUN_STARTED_AT" --arg finished "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg user "$USER_PK" --arg merchant "$MERCHANT_PK" --arg staff "$STAFF_PK" \
  --arg vault "$VAULT_ID" --arg subs "$SUBS_ID" --arg payroll "$PAYROLL_ID" \
  --arg plan "$PLAN_ID" --arg sub "$SUB_ID" --arg schedule "$SCHEDULE_ID" \
  --arg spend_ref "$SPEND_REFERENCE" --arg refund_ref "$REFUND_REFERENCE" \
  --argjson txs "$TXS" --arg explorer "$EXPLORER" \
  '. + {
    network: $network,
    purpose: "demo accounts exercising the Rubbi testnet contracts; not real users",
    contracts: {vault: $vault, subscriptions: $subs, payroll: $payroll},
    identities: {
      "demo-user": {address: $user, link: ($explorer + "/account/" + $user)},
      "demo-merchant": {address: $merchant, link: ($explorer + "/account/" + $merchant)},
      "demo-staff": {address: $staff, link: ($explorer + "/account/" + $staff)}
    },
    plan_id: (if $plan == "" then .plan_id else $plan end),
    sub_id: (if $sub == "" then .sub_id else $sub end),
    schedule_id: (if $schedule == "" then .schedule_id else $schedule end),
    last_run: {id: $run_id, mode: $mode, started_at: $started, finished_at: $finished,
               spend_reference: (if $spend_ref == "" then null else $spend_ref end),
               refund_reference: (if $refund_ref == "" then null else $refund_ref end),
               transactions: $txs},
    runs: ((.runs // []) + [{id: $run_id, mode: $mode, finished_at: $finished, transactions: ($txs | length)}])
  }')"

if [ "$DRY_RUN" -eq 1 ]; then
  log "Dry run: nothing was submitted. $DEMO_FILE would contain:"
  printf '%s\n' "$RECORD"
  exit 0
fi

tmp="$(mktemp "$DEMO_FILE.XXXXXX")"
printf '%s\n' "$RECORD" > "$tmp"
mv "$tmp" "$DEMO_FILE"

log "Transactions submitted in this run"
printf '%s' "$TXS" | jq -r '.[] | "  \(.step)\t\(.link)"'
log "Explorer pages"
printf '  vault          %s/contract/%s\n' "$EXPLORER" "$VAULT_ID"
printf '  subscriptions  %s/contract/%s\n' "$EXPLORER" "$SUBS_ID"
printf '  payroll        %s/contract/%s\n' "$EXPLORER" "$PAYROLL_ID"
if [ "$DAY2" -eq 0 ]; then
  log "Done. Tomorrow (after the plan period and the salary period have elapsed):"
  printf '  scripts/demo-testnet.sh --day2\n'
else
  log "Done."
fi
