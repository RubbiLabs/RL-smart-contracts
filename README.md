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
│   └── vault/              USDC custody for card funding and settlement
├── legacy/evm/             previous Solidity/Foundry project (reference only)
└── .github/workflows/      CI: fmt, clippy, test, wasm build
```

Contracts planned for this workspace: `vault` (this phase), `subscriptions`
and `payroll` (next phase).

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
# positive upgrade-path test; needs the wasm built by the previous command
cargo test -p rubbi-vault --features wasm-upgrade-test
```

The optimized contract is written to
`target/wasm32v1-none/release/rubbi_vault.wasm`.

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
