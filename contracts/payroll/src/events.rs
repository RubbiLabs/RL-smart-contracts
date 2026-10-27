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
