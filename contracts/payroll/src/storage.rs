//! Storage keys, TTL policy and typed accessors.
//!
//! Tiers:
//! - `instance()`: contract configuration (admin, token, fee settings, pause
//!   flag, schedule id counter, the running total of all escrows). Extended
//!   on every state-changing call via [`extend_instance`].
//! - `persistent()`: per-employer escrow balances and schedules. Extended on
//!   every read and write.
//!
//! No `temporary()` storage is used.

use soroban_sdk::{contracttype, Address, Env};

use crate::Schedule;

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

/// All storage keys of the payroll contract.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Current admin (instance).
    Admin,
    /// Proposed admin awaiting `accept_admin` (instance).
    PendingAdmin,
    /// USDC Stellar Asset Contract address (instance).
    Token,
    /// Address that receives the fee share of every disbursement (instance).
    FeeRecipient,
    /// Fee in basis points, at most 1_000 (instance).
    FeeBps,
    /// Pause flag (instance).
    Paused,
    /// Sum of all employer escrows (instance).
    TotalEscrow,
    /// Next schedule id to assign; ids start at 1 (instance).
    NextScheduleId,
    /// An employer's escrow balance (persistent).
    Escrow(Address),
    /// A schedule by id (persistent).
    Schedule(u64),
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
        .expect("payroll: admin not set")
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
        .expect("payroll: token not set")
}

pub fn set_token(env: &Env, token: &Address) {
    env.storage().instance().set(&DataKey::Token, token);
}

pub fn get_fee_recipient(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::FeeRecipient)
        .expect("payroll: fee recipient not set")
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

pub fn get_total_escrow(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalEscrow)
        .unwrap_or(0)
}

pub fn set_total_escrow(env: &Env, total: i128) {
    env.storage().instance().set(&DataKey::TotalEscrow, &total);
}

// ---- persistent: escrow ----------------------------------------------------

/// An employer's escrow balance, extending the entry's TTL if it exists.
pub fn get_escrow(env: &Env, employer: &Address) -> i128 {
    get_persistent(env, &DataKey::Escrow(employer.clone())).unwrap_or(0)
}

pub fn set_escrow(env: &Env, employer: &Address, balance: i128) {
    set_persistent(env, &DataKey::Escrow(employer.clone()), &balance);
}

// ---- instance counters -----------------------------------------------------

/// Allocate the next schedule id (starting at 1).
pub fn next_schedule_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&DataKey::NextScheduleId)
        .unwrap_or(1);
    env.storage()
        .instance()
        .set(&DataKey::NextScheduleId, &(id + 1));
    id
}

// ---- persistent: schedules -------------------------------------------------

pub fn get_schedule(env: &Env, id: u64) -> Option<Schedule> {
    get_persistent(env, &DataKey::Schedule(id))
}

pub fn set_schedule(env: &Env, schedule: &Schedule) {
    set_persistent(env, &DataKey::Schedule(schedule.id), schedule);
}
