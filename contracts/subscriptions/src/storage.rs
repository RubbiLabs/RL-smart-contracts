//! Storage keys, TTL policy and typed accessors.
//!
//! Tiers:
//! - `instance()`: contract configuration (admin, token, fee settings, pause
//!   flag, id counters). Extended on every state-changing call via
//!   [`extend_instance`].
//! - `persistent()`: merchants, plans, subscriptions and the
//!   subscriber+plan → subscription id index. Extended on every read and
//!   write.
//!
//! No `temporary()` storage is used.

use soroban_sdk::{contracttype, Address, Env};

use crate::{Merchant, Plan, Subscription};

/// Ledgers per day. Stellar closes a ledger roughly every 5 seconds
/// (measured on 2026-10-27 over 100 consecutive ledgers on both testnet and
/// mainnet via Horizon `/ledgers`: 500 s per 100 ledgers), so
/// 86_400 / 5 = 17_280.
pub const DAY_IN_LEDGERS: u32 = 17_280;

/// Instance entries are extended to 30 days ...
pub const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
/// ... whenever fewer than 29 days remain.
pub const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;

/// Persistent entries are extended to 90 days ...
pub const PERSISTENT_BUMP: u32 = 90 * DAY_IN_LEDGERS;
/// ... whenever fewer than 60 days remain.
pub const PERSISTENT_THRESHOLD: u32 = 60 * DAY_IN_LEDGERS;

/// All storage keys of the subscriptions contract.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Current admin (instance).
    Admin,
    /// Proposed admin awaiting `accept_admin` (instance).
    PendingAdmin,
    /// USDC Stellar Asset Contract address (instance).
    Token,
    /// Address that receives the fee share of every charge (instance).
    FeeRecipient,
    /// Fee in basis points, at most 1_000 (instance).
    FeeBps,
    /// Pause flag (instance).
    Paused,
    /// Next plan id to assign; ids start at 1 (instance).
    NextPlanId,
    /// Next subscription id to assign; ids start at 1 (instance).
    NextSubId,
    /// An allowlisted merchant (persistent).
    Merchant(Address),
    /// A plan by id (persistent).
    Plan(u32),
    /// A subscription by id (persistent).
    Subscription(u64),
    /// Index: the non-cancelled subscription of (subscriber, plan id), if
    /// any (persistent). Removed when the subscription is cancelled.
    SubOf(Address, u32),
}

/// Extend the TTL of the contract instance and code.
pub fn extend_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
}

fn extend_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_THRESHOLD, PERSISTENT_BUMP);
}

/// Read a persistent entry, extending its TTL if it exists.
fn get_persistent<T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Option<T> {
    let value = env.storage().persistent().get::<DataKey, T>(key)?;
    extend_persistent(env, key);
    Some(value)
}

fn set_persistent<T: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
    value: &T,
) {
    env.storage().persistent().set(key, value);
    extend_persistent(env, key);
}

// ---- instance config -------------------------------------------------------

pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("subscriptions: admin not set")
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn get_pending_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::PendingAdmin)
}

pub fn set_pending_admin(env: &Env, pending: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::PendingAdmin, pending);
}

pub fn clear_pending_admin(env: &Env) {
    env.storage().instance().remove(&DataKey::PendingAdmin);
}

pub fn get_token(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Token)
        .expect("subscriptions: token not set")
}

pub fn set_token(env: &Env, token: &Address) {
    env.storage().instance().set(&DataKey::Token, token);
}

pub fn get_fee_recipient(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::FeeRecipient)
        .expect("subscriptions: fee recipient not set")
}

pub fn set_fee_recipient(env: &Env, recipient: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::FeeRecipient, recipient);
}

pub fn get_fee_bps(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0)
}

pub fn set_fee_bps(env: &Env, fee_bps: u32) {
    env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}

// ---- instance counters -----------------------------------------------------

/// Allocate the next plan id (starting at 1).
pub fn next_plan_id(env: &Env) -> u32 {
    let id: u32 = env
        .storage()
        .instance()
        .get(&DataKey::NextPlanId)
        .unwrap_or(1);
    env.storage()
        .instance()
        .set(&DataKey::NextPlanId, &(id + 1));
    id
}

/// Allocate the next subscription id (starting at 1).
pub fn next_sub_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&DataKey::NextSubId)
        .unwrap_or(1);
    env.storage().instance().set(&DataKey::NextSubId, &(id + 1));
    id
}

// ---- persistent: merchants and plans ---------------------------------------

pub fn get_merchant(env: &Env, merchant: &Address) -> Option<Merchant> {
    get_persistent(env, &DataKey::Merchant(merchant.clone()))
}

pub fn set_merchant(env: &Env, merchant: &Address, value: &Merchant) {
    set_persistent(env, &DataKey::Merchant(merchant.clone()), value);
}

pub fn get_plan(env: &Env, plan_id: u32) -> Option<Plan> {
    get_persistent(env, &DataKey::Plan(plan_id))
}

pub fn set_plan(env: &Env, plan: &Plan) {
    set_persistent(env, &DataKey::Plan(plan.id), plan);
}

// ---- persistent: subscriptions ---------------------------------------------

pub fn get_subscription(env: &Env, sub_id: u64) -> Option<Subscription> {
    get_persistent(env, &DataKey::Subscription(sub_id))
}

pub fn set_subscription(env: &Env, sub: &Subscription) {
    set_persistent(env, &DataKey::Subscription(sub.id), sub);
}

/// The subscriber's non-cancelled subscription to `plan_id`, if any.
pub fn get_sub_id_of(env: &Env, subscriber: &Address, plan_id: u32) -> Option<u64> {
    get_persistent(env, &DataKey::SubOf(subscriber.clone(), plan_id))
}

pub fn set_sub_id_of(env: &Env, subscriber: &Address, plan_id: u32, sub_id: u64) {
    set_persistent(env, &DataKey::SubOf(subscriber.clone(), plan_id), &sub_id);
}

/// Free the (subscriber, plan) slot when a subscription is cancelled.
pub fn remove_sub_id_of(env: &Env, subscriber: &Address, plan_id: u32) {
    env.storage()
        .persistent()
        .remove(&DataKey::SubOf(subscriber.clone(), plan_id));
}
