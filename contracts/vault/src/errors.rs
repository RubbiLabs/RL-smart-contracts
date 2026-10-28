//! Vault error codes.
//!
//! Codes are stable and part of the public interface: clients map them to
//! human-readable messages. Never renumber an existing variant once the
//! contract is deployed.

use soroban_sdk::contracterror;

/// Every expected failure of the vault contract.
///
/// Authorization failures are not listed here: every role check goes
/// through `require_auth`, so a wrong signer fails at the host level before
/// the contract runs.
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
    /// A spend or refund reference was already used (idempotency guard).
    DuplicateReference = 4,
    /// `accept_admin` was called while no admin transfer is pending.
    NoPendingAdmin = 5,
    /// An arithmetic operation would overflow.
    Overflow = 6,
}
