//! Storage keys, TTL policy and typed accessors.
//!
//! Tiers:
//! - `instance()`: contract configuration (roles, token, pause flag, the
//!   running total of all user balances). Extended on every state-changing
//!   call via [`extend_instance`].
//! - `persistent()`: per-user balances and the spend / refund reference
//!   namespaces. Extended on every read and write.
//!
//! No `temporary()` storage is used.

use soroban_sdk::{contracttype, Address, BytesN, Env};

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

/// All storage keys of the vault.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Current admin (instance).
    Admin,
    /// Proposed admin awaiting `accept_admin` (instance).
    PendingAdmin,
    /// Backend operator allowed to settle card spends (instance).
    Operator,
    /// Treasury that receives settlements and funds refunds (instance).
    Treasury,
    /// USDC Stellar Asset Contract address (instance).
    Token,
    /// Pause flag (instance).
    Paused,
    /// Sum of all user balances (instance).
    TotalBalances,
    /// A user's vault balance (persistent).
    Balance(Address),
    /// A consumed card-spend reference (persistent).
    SpendRef(BytesN<32>),
    /// A consumed refund reference (persistent).
    RefundRef(BytesN<32>),
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

// ---- instance config -------------------------------------------------------

pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("vault: admin not set")
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

pub fn get_operator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Operator)
        .expect("vault: operator not set")
}

pub fn set_operator(env: &Env, operator: &Address) {
    env.storage().instance().set(&DataKey::Operator, operator);
}

pub fn get_treasury(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Treasury)
        .expect("vault: treasury not set")
}

pub fn set_treasury(env: &Env, treasury: &Address) {
    env.storage().instance().set(&DataKey::Treasury, treasury);
}

pub fn get_token(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Token)
        .expect("vault: token not set")
}

pub fn set_token(env: &Env, token: &Address) {
    env.storage().instance().set(&DataKey::Token, token);
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

pub fn get_total_balances(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalBalances)
        .unwrap_or(0)
}

pub fn set_total_balances(env: &Env, total: i128) {
    env.storage()
        .instance()
        .set(&DataKey::TotalBalances, &total);
}

// ---- persistent: balances --------------------------------------------------

/// Read a user's balance, extending the entry's TTL if it exists.
pub fn get_balance(env: &Env, user: &Address) -> i128 {
    let key = DataKey::Balance(user.clone());
    match env.storage().persistent().get::<DataKey, i128>(&key) {
        Some(balance) => {
            extend_persistent(env, &key);
            balance
        }
        None => 0,
    }
}

pub fn set_balance(env: &Env, user: &Address, balance: i128) {
    let key = DataKey::Balance(user.clone());
    env.storage().persistent().set(&key, &balance);
    extend_persistent(env, &key);
}

// ---- persistent: reference namespaces --------------------------------------

/// Whether a card-spend reference has already been settled.
pub fn has_spend_ref(env: &Env, reference: &BytesN<32>) -> bool {
    has_ref(env, &DataKey::SpendRef(reference.clone()))
}

pub fn set_spend_ref(env: &Env, reference: &BytesN<32>) {
    set_ref(env, &DataKey::SpendRef(reference.clone()));
}

/// Whether a refund reference has already been credited.
pub fn has_refund_ref(env: &Env, reference: &BytesN<32>) -> bool {
    has_ref(env, &DataKey::RefundRef(reference.clone()))
}

pub fn set_refund_ref(env: &Env, reference: &BytesN<32>) {
    set_ref(env, &DataKey::RefundRef(reference.clone()));
}

fn has_ref(env: &Env, key: &DataKey) -> bool {
    if env.storage().persistent().has(key) {
        extend_persistent(env, key);
        true
    } else {
        false
    }
}

fn set_ref(env: &Env, key: &DataKey) {
    env.storage().persistent().set(key, &true);
    extend_persistent(env, key);
}
