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
