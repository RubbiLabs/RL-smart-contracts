//! Vault error codes.
//!
//! Codes are stable and part of the public interface: clients map them to
//! human-readable messages. Never renumber an existing variant.

use soroban_sdk::contracterror;

/// Every expected failure of the vault contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The amount is zero or negative.
    InvalidAmount = 1,
    /// The user's vault balance is smaller than the requested amount.
    InsufficientBalance = 2,
    /// The contract is paused and this operation is blocked while paused.
    Paused = 3,
    /// The caller is not allowed to perform this operation.
    Unauthorized = 4,
    /// A spend or refund reference was already used (idempotency guard).
    DuplicateReference = 5,
    /// `accept_admin` was called while no admin transfer is pending.
    NoPendingAdmin = 6,
    /// The caller is not the proposed admin.
    NotPendingAdmin = 7,
    /// An arithmetic operation would overflow.
    Overflow = 8,
}
