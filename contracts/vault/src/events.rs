//! Typed events. Every state change emits exactly one of these.
//!
//! Each event's first topic is its name in snake case (e.g. `deposit`), and
//! events about a user carry that user's address as the second topic, so an
//! indexer can filter by contract + event name + address.

use soroban_sdk::{contractevent, Address, BytesN};

/// `deposit`: `user` moved `amount` from their wallet into the vault.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deposit {
    #[topic]
    pub user: Address,
    pub amount: i128,
}

/// `withdraw`: `user` moved `amount` from the vault back to their wallet.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withdraw {
    #[topic]
    pub user: Address,
    pub amount: i128,
}

/// `card_spend_settled`: the operator moved `amount` of `user`'s balance to
/// the treasury for the card spend identified by `reference`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardSpendSettled {
    #[topic]
    pub user: Address,
    pub amount: i128,
    pub reference: BytesN<32>,
}

/// `refund_credited`: the treasury credited `amount` back to `user` for the
/// refund identified by `reference`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundCredited {
    #[topic]
    pub user: Address,
    pub amount: i128,
    pub reference: BytesN<32>,
}

/// `operator_set`: the admin rotated the operator role.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorSet {
    pub operator: Address,
}

/// `treasury_set`: the admin rotated the treasury role.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasurySet {
    pub treasury: Address,
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

/// `paused`: deposits and settlements are blocked until `unpaused`.
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
