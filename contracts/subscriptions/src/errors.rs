//! Subscriptions error codes.
//!
//! Codes are stable and part of the public interface: clients map them to
//! human-readable messages. Never renumber an existing variant once the
//! contract is deployed.

use soroban_sdk::contracterror;

/// Every expected failure of the subscriptions contract.
///
/// Authorization failures are not listed here: every role and ownership
/// check goes through `require_auth`, so a wrong signer fails at the host
/// level before the contract runs. (Ownership of a subscription or plan is
/// additionally checked against storage: see `NotSubscriber` and
/// `NotPlanOwner`.)
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
    /// The plan name is longer than 64 bytes.
    NameTooLong = 5,
    /// `add_merchant` was called for an address that is already a merchant.
    MerchantExists = 6,
    /// The address is not on the merchant allowlist.
    MerchantNotFound = 7,
    /// The merchant is deactivated: it cannot create plans or take new
    /// subscribers.
    MerchantInactive = 8,
    /// No plan with this id exists.
    PlanNotFound = 9,
    /// The plan is deactivated and accepts no new subscribers.
    PlanInactive = 10,
    /// The caller is not the merchant that owns the plan.
    NotPlanOwner = 11,
    /// The subscriber already has a non-cancelled subscription to this plan.
    AlreadySubscribed = 12,
    /// No subscription with this id exists.
    SubscriptionNotFound = 13,
    /// The caller is not the owner of the subscription.
    NotSubscriber = 14,
    /// The subscription's `next_due` is in the future.
    NotDue = 15,
    /// The subscription is paused or cancelled and cannot be charged.
    NotChargeable = 16,
    /// Only `Active` or `PastDue` subscriptions can be paused.
    NotPausable = 17,
    /// Only `Paused` subscriptions can be resumed.
    NotPaused = 18,
    /// The subscription is already cancelled.
    AlreadyCancelled = 19,
    /// The first-period pull in `subscribe` failed (allowance or balance).
    PaymentFailed = 20,
    /// A batch has more than `MAX_BATCH` (25) items.
    BatchTooLarge = 21,
    /// `accept_admin` was called while no admin transfer is pending.
    NoPendingAdmin = 22,
    /// An arithmetic operation would overflow.
    Overflow = 23,
    /// The merchant's payout address cannot receive USDC: it has no USDC
    /// trustline, or the trustline is frozen.
    PayoutUnavailable = 24,
}
