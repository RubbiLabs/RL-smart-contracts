//! Payroll error codes.
//!
//! Codes are stable and part of the public interface: clients map them to
//! human-readable messages. Never renumber an existing variant once the
//! contract is deployed.

use soroban_sdk::contracterror;

/// Every expected failure of the payroll contract.
///
/// Authorization failures are not listed here: every role and ownership
/// check goes through `require_auth`, so a wrong signer fails at the host
/// level before the contract runs. (Ownership of a schedule is additionally
/// checked against storage: see `NotEmployer`.)
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The amount is zero or negative.
    InvalidAmount = 1,
    /// The contract is paused and this operation is blocked while paused.
    Paused = 2,
    /// `fee_bps` exceeds the hard cap of 1_000 (10%).
    FeeTooHigh = 3,
    /// The period is shorter than 1 day or longer than 366 days.
    InvalidPeriod = 4,
    /// The schedule label is longer than 64 bytes.
    LabelTooLong = 5,
    /// The employer's escrow is smaller than the requested withdrawal.
    InsufficientEscrow = 6,
    /// `create_schedules` was called with more than 50 inputs.
    TooManySchedules = 7,
    /// `first_due` is in the past.
    FirstDueInPast = 8,
    /// No schedule with this id exists.
    ScheduleNotFound = 9,
    /// The caller is not the employer that owns the schedule.
    NotEmployer = 10,
    /// The schedule is not `Active` (paused or cancelled).
    NotActive = 11,
    /// Only `Paused` schedules can be resumed.
    NotPaused = 12,
    /// The schedule is already cancelled.
    AlreadyCancelled = 13,
    /// The schedule's `next_due` is in the future.
    NotDue = 14,
    /// A batch has more than `MAX_BATCH` (25) items.
    BatchTooLarge = 15,
    /// `accept_admin` was called while no admin transfer is pending.
    NoPendingAdmin = 16,
    /// An arithmetic operation would overflow.
    Overflow = 17,
}
