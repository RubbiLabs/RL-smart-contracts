//! Typed events. Every state change emits exactly one of these.
//!
//! Each event's first topic is its name in snake case (e.g. `disbursed`),
//! and events about an employer carry the employer address as the second
//! topic, so an indexer can filter by contract + event name + address.

use soroban_sdk::{contractevent, Address, BytesN};

/// `funded`: `employer` moved `amount` from their wallet into escrow.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Funded {
    #[topic]
    pub employer: Address,
    pub amount: i128,
}

/// `escrow_withdrawn`: `employer` moved `amount` from escrow back to their
/// wallet.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowWithdrawn {
    #[topic]
    pub employer: Address,
    pub amount: i128,
}

/// `schedule_created`: `employer` created schedule `schedule_id` paying
/// `amount` to `recipient` every `period` seconds, first at `next_due`.
///
/// The label is deliberately not part of the event: a transaction may
/// emit at most 16 KiB of contract events, and `create_schedules` emits one
/// event per schedule for up to 50 schedules. Read it with `get_schedule`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleCreated {
    #[topic]
    pub employer: Address,
    pub schedule_id: u64,
    pub recipient: Address,
    pub amount: i128,
    pub period: u64,
    pub next_due: u64,
}

/// `disbursed`: one period of schedule `schedule_id` was paid: `employer`'s
/// escrow was debited `amount`, `recipient` received `amount - fee` and the
/// fee recipient `fee`. The next period is due at `next_due`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Disbursed {
    #[topic]
    pub employer: Address,
    pub schedule_id: u64,
    pub recipient: Address,
    pub amount: i128,
    pub fee: i128,
    pub next_due: u64,
}

/// `disburse_failed`: schedule `schedule_id` was due but `employer`'s
/// escrow (`escrow`) is smaller than `amount`. Nothing changed; the period
/// stays owed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisburseFailed {
    #[topic]
    pub employer: Address,
    pub schedule_id: u64,
    pub amount: i128,
    pub escrow: i128,
}

/// `schedule_paused`: `employer` paused schedule `schedule_id`. Periods
/// that fall due while paused stay owed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulePaused {
    #[topic]
    pub employer: Address,
    pub schedule_id: u64,
}

/// `schedule_resumed`: `employer` resumed schedule `schedule_id`;
/// `next_due` is unchanged from before the pause.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleResumed {
    #[topic]
    pub employer: Address,
    pub schedule_id: u64,
    pub next_due: u64,
}

/// `schedule_cancelled`: `employer` cancelled schedule `schedule_id`.
/// Final.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleCancelled {
    #[topic]
    pub employer: Address,
    pub schedule_id: u64,
}

/// `fee_set`: the admin changed the fee to `fee_bps` basis points.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeSet {
    pub fee_bps: u32,
}

/// `fee_recipient_set`: the admin changed where fees are paid.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeRecipientSet {
    pub fee_recipient: Address,
}

/// `admin_proposed`: the admin proposed `pending_admin` as the next admin.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminProposed {
    #[topic]
    pub pending_admin: Address,
}

/// `admin_transferred`: `new_admin` accepted the role from `old_admin`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferred {
    #[topic]
    pub new_admin: Address,
    pub old_admin: Address,
}

/// `paused`: funding, new schedules and disbursements are blocked until
/// `unpaused`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paused {}

/// `unpaused`: normal operation resumed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unpaused {}

/// `upgraded`: the contract executable will be replaced by `wasm_hash`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Upgraded {
    pub wasm_hash: BytesN<32>,
}
