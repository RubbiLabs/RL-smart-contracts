//! Typed events. Every state change emits exactly one of these.
//!
//! Each event's first topic is its name in snake case (e.g. `deposit`), and
//! events about a user carry that user's address as the second topic, so an
//! indexer can filter by contract + event name + address.

use soroban_sdk::{contractevent, Address};

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
