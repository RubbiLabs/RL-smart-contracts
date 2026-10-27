# Rubbi smart contracts (Stellar / Soroban)

Soroban contracts for [Rubbi](https://github.com/RubbiLabs), a Nigeria-focused
payment automation platform: users fund a USDC balance, pay recurring
subscriptions and bills, run recurring payroll, and spend through a virtual card.

This repository is being migrated from Solidity on Arbitrum to Soroban on
Stellar. The previous Foundry project is kept unchanged under `legacy/evm/`
for reference; the tag `v0-evm-arbitrum` holds the last buildable EVM state.

> This README is an interim version. The full version (architecture, error
> code tables, testnet deployment table, contributing guide) lands with the
> deployment tooling.

## Layout

```text
.
├── Cargo.toml              workspace (soroban-sdk 29.0.1, release profile)
├── rust-toolchain.toml     Rust 1.99.1 + wasm32v1-none
├── contracts/
│   ├── vault/              USDC custody for card funding and settlement
│   ├── subscriptions/      allowance-based recurring payments (non-custodial)
│   └── payroll/            employer-funded recurring salary payments
├── legacy/evm/             previous Solidity/Foundry project (reference only)
└── .github/workflows/      CI: fmt, clippy, test, wasm build
```

## Contracts

| Contract | Custody | Who calls what |
|---|---|---|
| `vault` | custodial (card funding) | users `deposit` / `withdraw`; the operator `settle_card_spend`; the treasury `credit_refund` |
| `subscriptions` | non-custodial: funds stay in the subscriber's wallet behind a bounded, expiring SAC allowance | admin curates merchants; merchants create plans; subscribers `subscribe` / `pause_subscription` / `resume_subscription` / `cancel_subscription`; anyone may call the time-gated `charge` / `charge_batch` |
| `payroll` | employer escrow | employers `fund` / `withdraw_escrow` / `create_schedules` (at most 25 per call) / `pause_schedule` / `resume_schedule` / `cancel_schedule`; anyone may call the time-gated `disburse` / `disburse_batch` |

Fees (subscriptions and payroll): the payer always pays exactly the plan or
schedule amount; the payee receives `amount - fee` and the fee recipient
`fee = amount * fee_bps / 10_000` rounded down, with `fee_bps` capped at
1_000 (10%).

Batch functions (`charge_batch`, `disburse_batch`) and `create_schedules`
accept at most 25 items per call. The cap on `create_schedules` keeps the
per-schedule `schedule_created` events well inside the 16 KiB of contract
events a transaction may emit; `schedule_created` carries no label (read it
with `get_schedule`).

Subscriptions never back-charge: a subscriber who was past due for three
months is charged once. Payroll deliberately does back-pay: each `disburse`
pays exactly one period, so missed periods are caught up one call at a time.

Before `subscribe`, the subscriber must `approve` this contract on the USDC
SAC for `plan.amount × N` periods (the frontend defaults to N = 3) with a
`live_until_ledger` covering those periods. When the allowance runs out or
expires, `charge` marks the subscription `PastDue` instead of failing and
retries on the next call once the subscriber re-approves.

## Requirements

- Rust 1.99.1 (installed automatically from `rust-toolchain.toml`) with the
  `wasm32v1-none` target
- [stellar-cli](https://github.com/stellar/stellar-cli) 28.1.0

## Build and test

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
stellar contract build
# positive upgrade-path tests; they need the wasm built by the previous command
cargo test -p rubbi-vault --features wasm-upgrade-test
cargo test -p rubbi-subscriptions --features wasm-upgrade-test
cargo test -p rubbi-payroll --features wasm-upgrade-test
```

The optimized contracts are written to `target/wasm32v1-none/release/`
(`rubbi_vault.wasm`, `rubbi_subscriptions.wasm`, `rubbi_payroll.wasm`).

## Conventions

- `#![no_std]`, amounts are `i128` in the asset's smallest unit (USDC has 7
  decimals on Stellar), time is `env.ledger().timestamp()` in seconds.
- Every expected failure is a typed error from the contract's `Error` enum;
  contracts never panic on user input.
- Every state change emits a typed event (`#[contractevent]`).
- Checks → effects (storage writes) → interactions (token transfers).

## Security

Please report vulnerabilities privately to the maintainers rather than opening
a public issue. A `SECURITY.md` with the disclosure contact will be added.
