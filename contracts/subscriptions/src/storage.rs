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

/// Ledgers per day. Stellar closes a ledger roughly every 5 seconds
/// (measured on 2026-10-27 over 100 consecutive ledgers on both testnet and
/// mainnet via Horizon `/ledgers`: 500 s per 100 ledgers), so
/// 86_400 / 5 = 17_280.
pub const DAY_IN_LEDGERS: u32 = 17_280;

/// Instance entries are extended to 30 days ...
pub const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
/// ... whenever fewer than 29 days remain.
pub const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;

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
}

/// Extend the TTL of the contract instance and code.
pub fn extend_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
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
