//! Typed events. Every state change emits exactly one of these.
//!
//! Each event's first topic is its name in snake case (e.g. `charged`), and
//! events about a subscriber or merchant carry that address as the second
//! topic, so an indexer can filter by contract + event name + address.

use soroban_sdk::{contractevent, Address, BytesN, String, Symbol};

/// `merchant_added`: the admin allowlisted `merchant`, paying out to
/// `payout`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerchantAdded {
    #[topic]
    pub merchant: Address,
    pub payout: Address,
}

/// `merchant_active_set`: the admin (de)activated `merchant`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerchantActiveSet {
    #[topic]
    pub merchant: Address,
    pub active: bool,
}

/// `plan_created`: `merchant` created plan `plan_id`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanCreated {
    #[topic]
    pub merchant: Address,
    pub plan_id: u32,
    pub name: String,
    pub amount: i128,
    pub period: u64,
}

/// `plan_active_set`: `merchant` (de)activated plan `plan_id`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanActiveSet {
    #[topic]
    pub merchant: Address,
    pub plan_id: u32,
    pub active: bool,
}

/// `subscribed`: `subscriber` subscribed to `plan_id` as subscription
/// `sub_id`; the first period was charged (see the `charged` event emitted
/// in the same call) and the next charge is due at `next_due`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subscribed {
    #[topic]
    pub subscriber: Address,
    pub sub_id: u64,
    pub plan_id: u32,
    pub next_due: u64,
}

/// `charged`: `amount` was pulled from `subscriber` for subscription
/// `sub_id`; `merchant` received `amount - fee` and the fee recipient
/// `fee`. The next charge is due at `next_due`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Charged {
    #[topic]
    pub subscriber: Address,
    pub sub_id: u64,
    pub plan_id: u32,
    pub merchant: Address,
    pub amount: i128,
    pub fee: i128,
    pub next_due: u64,
}

/// `charge_failed`: pulling `amount` from `subscriber` for subscription
/// `sub_id` failed (allowance or balance); the subscription is now
/// `PastDue`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChargeFailed {
    #[topic]
    pub subscriber: Address,
    pub sub_id: u64,
    pub plan_id: u32,
    pub amount: i128,
}

/// `charge_skipped`: subscription `sub_id` was due but not charged, and its
/// state was left unchanged. `reason` is currently always
/// `merchant_inactive`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChargeSkipped {
    #[topic]
    pub subscriber: Address,
    pub sub_id: u64,
    pub reason: Symbol,
}

/// `subscription_paused`: `subscriber` paused subscription `sub_id`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionPaused {
    #[topic]
    pub subscriber: Address,
    pub sub_id: u64,
}

/// `subscription_resumed`: `subscriber` resumed subscription `sub_id`; the
/// next charge is due at `next_due` (never in the past).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionResumed {
    #[topic]
    pub subscriber: Address,
    pub sub_id: u64,
    pub next_due: u64,
}

/// `subscription_cancelled`: `subscriber` cancelled subscription `sub_id`.
/// Final; the subscriber may subscribe to the plan again.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionCancelled {
    #[topic]
    pub subscriber: Address,
    pub sub_id: u64,
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

/// `paused`: new subscriptions and charges are blocked until `unpaused`.
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
