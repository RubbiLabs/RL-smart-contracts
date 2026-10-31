# Contract interface

What a client, indexer or wallet integration needs to call the Rubbi
contracts and read their events: who must sign each entry point, the event
topics and payloads, and where the error codes are listed.

The source of truth is the code: `contracts/<name>/src/lib.rs` (entry points,
with rustdoc on each), `events.rs` and `errors.rs`. Arguments, return
values, per-function errors and behaviour are in the README's
[Contract reference](../README.md#contract-reference). Deployed ids are in
[`docs/TESTNET.md`](TESTNET.md).

## Conventions

- Amounts are `i128` in USDC's smallest unit (7 decimals: `1_0000000` = 1
  USDC). Times are ledger timestamps in seconds (`u64`).
- "Must sign" means `require_auth` on that address. A wallet signs the
  invocation; the host rejects the call before the contract runs if the
  signature is missing. An auth failure is a host error, not a contract
  error code.
- Functions marked *permissionless* need no signature beyond the
  transaction's source account; they are time-gated in the contract.
- Read-only functions can be simulated for free
  (`stellar contract invoke ... --send=no`).

## Entry points and signers

### vault

| Entry point | Must sign |
|---|---|
| `deposit(user, amount)` | `user` |
| `withdraw(user, amount)` | `user` |
| `settle_card_spend(user, amount, reference)` | the stored operator |
| `credit_refund(user, amount, reference)` | the stored treasury |
| `set_operator(new_operator)`, `set_treasury(new_treasury)` | admin |
| `propose_admin(new_admin)` | admin |
| `accept_admin()` | the pending admin |
| `pause()`, `unpause()` | admin |
| `upgrade(new_wasm_hash)` | admin |
| `balance(user)`, `total_balances()`, `admin()`, `pending_admin()`, `operator()`, `treasury()`, `token()`, `is_paused()` | nobody (read-only) |

The user does not sign `settle_card_spend` or `credit_refund`: the backend
signs them with the operator key (card load) or the treasury key (card
unload) after the user asks the app to load or unload the card.
`reference` is a `BytesN<32>` (64 hex characters on the CLI) and can be used
once per function.

### subscriptions

| Entry point | Must sign |
|---|---|
| `add_merchant(merchant, payout)`, `set_merchant_active(merchant, active)` | admin |
| `create_plan(merchant, name, amount, period)` | `merchant` (active, allowlisted) |
| `set_plan_active(merchant, plan_id, active)` | `merchant` (plan owner) |
| `subscribe(subscriber, plan_id)` | `subscriber`, who must first `approve` this contract as spender on the USDC SAC |
| `pause_subscription`, `resume_subscription`, `cancel_subscription` `(subscriber, sub_id)` | `subscriber` (owner) |
| `charge(sub_id)`, `charge_batch(sub_ids)` | nobody (permissionless, time-gated) |
| `set_fee(fee_bps)`, `set_fee_recipient(fee_recipient)` | admin |
| `propose_admin(new_admin)` | admin |
| `accept_admin()` | the pending admin |
| `pause()`, `unpause()` | admin |
| `upgrade(new_wasm_hash)` | admin |
| `get_merchant(merchant)`, `get_plan(plan_id)`, `get_subscription(sub_id)`, `sub_id_of(subscriber, plan_id)`, `admin()`, `pending_admin()`, `token()`, `fee_recipient()`, `fee_bps()`, `is_paused()` | nobody (read-only) |

The allowance is granted on the USDC Stellar Asset Contract, not on this
contract: `approve(from = subscriber, spender = <subscriptions id>, amount,
live_until_ledger)`, signed by the subscriber.

### payroll

| Entry point | Must sign |
|---|---|
| `fund(employer, amount)`, `withdraw_escrow(employer, amount)` | `employer` |
| `create_schedules(employer, inputs, first_due)` | `employer` |
| `pause_schedule`, `resume_schedule`, `cancel_schedule` `(employer, schedule_id)` | `employer` (owner) |
| `disburse(schedule_id)`, `disburse_batch(ids)` | nobody (permissionless, time-gated) |
| `set_fee(fee_bps)`, `set_fee_recipient(fee_recipient)` | admin |
| `propose_admin(new_admin)` | admin |
| `accept_admin()` | the pending admin |
| `pause()`, `unpause()` | admin |
| `upgrade(new_wasm_hash)` | admin |
| `get_schedule(schedule_id)`, `escrow_of(employer)`, `total_escrow()`, `admin()`, `pending_admin()`, `token()`, `fee_recipient()`, `fee_bps()`, `is_paused()` | nobody (read-only) |

## Events

Every state change emits one event defined with `#[contractevent]`
(soroban-sdk 29.0.1, default `data_format = "map"`):

- **topic 0** is the event name as a `Symbol`, in snake case;
- the next topics are the fields marked `#[topic]` (listed below as
  *Topics*), at most one address;
- **data** is an `ScMap` of the remaining fields keyed by field name.
  `paused` and `unpaused` have an empty map.

Filter with RPC `getEvents` on the contract id and, if needed, on topic 0
(and topic 1 for a user, subscriber, merchant or employer). The token
movements themselves also appear as the USDC SAC's `transfer` events.

### vault events

| Event | Topics after the name | Data fields |
|---|---|---|
| `deposit` | `user: Address` | `amount: i128` |
| `withdraw` | `user: Address` | `amount: i128` |
| `card_spend_settled` | `user: Address` | `amount: i128`, `reference: BytesN<32>` |
| `refund_credited` | `user: Address` | `amount: i128`, `reference: BytesN<32>` |
| `operator_set` | — | `operator: Address` |
| `treasury_set` | — | `treasury: Address` |
| `admin_proposed` | `pending_admin: Address` | — |
| `admin_transferred` | `new_admin: Address` | `old_admin: Address` |
| `paused` / `unpaused` | — | — |
| `upgraded` | — | `wasm_hash: BytesN<32>` |

### subscriptions events

| Event | Topics after the name | Data fields |
|---|---|---|
| `merchant_added` | `merchant: Address` | `payout: Address` |
| `merchant_active_set` | `merchant: Address` | `active: bool` |
| `plan_created` | `merchant: Address` | `plan_id: u32`, `name: String`, `amount: i128`, `period: u64` |
| `plan_active_set` | `merchant: Address` | `plan_id: u32`, `active: bool` |
| `subscribed` | `subscriber: Address` | `sub_id: u64`, `plan_id: u32`, `next_due: u64` |
| `charged` | `subscriber: Address` | `sub_id: u64`, `plan_id: u32`, `merchant: Address`, `amount: i128`, `fee: i128`, `next_due: u64` |
| `charge_failed` | `subscriber: Address` | `sub_id: u64`, `plan_id: u32`, `amount: i128` |
| `charge_skipped` | `subscriber: Address` | `sub_id: u64`, `reason: Symbol` (`merchant_inactive` or `payout_unavailable`) |
| `subscription_paused` | `subscriber: Address` | `sub_id: u64` |
| `subscription_resumed` | `subscriber: Address` | `sub_id: u64`, `next_due: u64` |
| `subscription_cancelled` | `subscriber: Address` | `sub_id: u64` |
| `fee_set` | — | `fee_bps: u32` |
| `fee_recipient_set` | — | `fee_recipient: Address` |
| `admin_proposed` | `pending_admin: Address` | — |
| `admin_transferred` | `new_admin: Address` | `old_admin: Address` |
| `paused` / `unpaused` | — | — |
| `upgraded` | — | `wasm_hash: BytesN<32>` |

`subscribe` emits `subscribed` and then `charged` for the first period.
In `charged`, the merchant's payout address received `amount - fee` and the
fee recipient `fee`.

### payroll events

| Event | Topics after the name | Data fields |
|---|---|---|
| `funded` | `employer: Address` | `amount: i128` |
| `escrow_withdrawn` | `employer: Address` | `amount: i128` |
| `schedule_created` | `employer: Address` | `schedule_id: u64`, `recipient: Address`, `amount: i128`, `period: u64`, `next_due: u64` |
| `disbursed` | `employer: Address` | `schedule_id: u64`, `recipient: Address`, `amount: i128`, `fee: i128`, `next_due: u64` |
| `disburse_failed` | `employer: Address` | `schedule_id: u64`, `amount: i128`, `escrow: i128`, `reason: Symbol` (`insufficient_escrow` or `recipient_unpayable`) |
| `schedule_paused` | `employer: Address` | `schedule_id: u64` |
| `schedule_resumed` | `employer: Address` | `schedule_id: u64`, `next_due: u64` |
| `schedule_cancelled` | `employer: Address` | `schedule_id: u64` |
| `fee_set` | — | `fee_bps: u32` |
| `fee_recipient_set` | — | `fee_recipient: Address` |
| `admin_proposed` | `pending_admin: Address` | — |
| `admin_transferred` | `new_admin: Address` | `old_admin: Address` |
| `paused` / `unpaused` | — | — |
| `upgraded` | — | `wasm_hash: BytesN<32>` |

`schedule_created` carries no label (a transaction may emit at most 16 KiB
of contract events, and `create_schedules` emits up to 25 of them); read it
with `get_schedule`. `create_schedules` emits one `schedule_created` per
schedule, in input order.

## Errors

Each contract has one `#[contracterror]` enum with stable codes. A failed
call shows up as `Error(Contract, #<code>)`. The tables are in the README
and are checked against `errors.rs` in CI by
`scripts/check-readme-errors.sh`, so they are not repeated here:

- [vault errors](../README.md#vault-1)
- [subscriptions errors](../README.md#subscriptions-1)
- [payroll errors](../README.md#payroll-1)

Map codes to plain-language messages in the UI rather than showing the raw
`Error(Contract, #n)`. Batch functions (`charge_batch`, `disburse_batch`)
never fail because of one item: per-item problems come back as `false` in
the result vector, and only `BatchTooLarge` and `Paused` fail the batch.
