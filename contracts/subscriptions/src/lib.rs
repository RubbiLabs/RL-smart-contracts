//! Rubbi subscriptions: non-custodial, allowance-based recurring payments.
//!
//! Funds stay in the subscriber's wallet. The subscriber grants this
//! contract a bounded, expiring allowance on the USDC Stellar Asset Contract
//! (`approve(from, spender = this contract, amount, live_until_ledger)`) and
//! the contract pulls each period's payment with `transfer_from`.
//!
//! Roles:
//! - `admin`: curates the merchant allowlist, sets the fee, pauses, upgrades
//!   and hands over admin rights with a two-step transfer.
//! - merchants (allowlisted by the admin): create and deactivate plans.
//! - subscribers: subscribe, pause, resume and cancel their own
//!   subscriptions.
//! - anyone: `charge` / `charge_batch` are permissionless and time-gated;
//!   Rubbi's keeper calls them, but any account could.
//!
//! Fees: the payer always pays exactly the plan amount; the merchant
//! receives `amount - fee` and the fee recipient receives
//! `fee = amount * fee_bps / 10_000` (rounded down).
//!
//! Pause blocks new subscriptions and charges. It never blocks subscribers
//! from pausing, resuming or cancelling their own subscriptions.

#![no_std]

mod errors;
mod events;
mod storage;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractimpl, contracttype, Address, BytesN, ContractExecutable, Env, String,
};

pub use errors::Error;

/// Hard cap on `fee_bps`: 1_000 = 10%.
pub const MAX_FEE_BPS: u32 = 1_000;
/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;
/// Maximum number of items accepted by any batch function.
pub const MAX_BATCH: u32 = 25;
/// Shortest allowed plan period: one day, in seconds.
pub const MIN_PERIOD: u64 = 86_400;
/// Longest allowed plan period: 366 days, in seconds.
pub const MAX_PERIOD: u64 = 366 * 86_400;
/// Longest allowed plan name, in bytes.
pub const MAX_NAME_LEN: u32 = 64;

// ---- public types --------------------------------------------------------------

/// An allowlisted merchant (biller). Charges are paid to `payout`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Merchant {
    pub payout: Address,
    pub active: bool,
}

/// A subscription plan offered by a merchant. `amount` is in the token's
/// smallest unit and is charged once every `period` seconds. No personal
/// data is stored.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    pub id: u32,
    pub merchant: Address,
    pub name: String,
    pub amount: i128,
    pub period: u64,
    pub active: bool,
}

/// The subscriptions contract.
#[contract]
pub struct SubscriptionsContract;

#[contractimpl]
impl SubscriptionsContract {
    /// Initialize the contract. Runs once, at deployment.
    ///
    /// - `admin`: merchant allowlist, fee settings, pause, upgrade and
    ///   two-step admin transfer.
    /// - `token`: the USDC Stellar Asset Contract address.
    /// - `fee_recipient`: receives the fee share of every charge.
    /// - `fee_bps`: fee in basis points, at most [`MAX_FEE_BPS`].
    ///
    /// Auth: none (constructor). Errors: `FeeTooHigh` (deployment fails).
    /// Events: none.
    pub fn __constructor(
        env: Env,
        admin: Address,
        token: Address,
        fee_recipient: Address,
        fee_bps: u32,
    ) -> Result<(), Error> {
        require_fee_in_range(fee_bps)?;
        storage::set_admin(&env, &admin);
        storage::set_token(&env, &token);
        storage::set_fee_recipient(&env, &fee_recipient);
        storage::set_fee_bps(&env, fee_bps);
        storage::set_paused(&env, false);
        storage::extend_instance(&env);
        Ok(())
    }

    // ---- merchants (admin allowlist) ---------------------------------------

    /// Allowlist `merchant` with payout address `payout`. New merchants are
    /// active.
    ///
    /// Auth: admin. Errors: `MerchantExists`. Events: `merchant_added`.
    pub fn add_merchant(env: Env, merchant: Address, payout: Address) -> Result<(), Error> {
        storage::get_admin(&env).require_auth();
        if storage::get_merchant(&env, &merchant).is_some() {
            return Err(Error::MerchantExists);
        }
        storage::extend_instance(&env);
        storage::set_merchant(
            &env,
            &merchant,
            &Merchant {
                payout: payout.clone(),
                active: true,
            },
        );
        events::MerchantAdded { merchant, payout }.publish(&env);
        Ok(())
    }

    /// Activate or deactivate `merchant`. An inactive merchant cannot create
    /// plans or take new subscribers, and its existing subscriptions are
    /// skipped by `charge` until it is reactivated.
    ///
    /// Auth: admin. Errors: `MerchantNotFound`. Events: `merchant_active_set`.
    pub fn set_merchant_active(env: Env, merchant: Address, active: bool) -> Result<(), Error> {
        storage::get_admin(&env).require_auth();
        let mut record = storage::get_merchant(&env, &merchant).ok_or(Error::MerchantNotFound)?;
        storage::extend_instance(&env);
        record.active = active;
        storage::set_merchant(&env, &merchant, &record);
        events::MerchantActiveSet { merchant, active }.publish(&env);
        Ok(())
    }

    // ---- plans (merchant) --------------------------------------------------

    /// Create a new, active plan and return its id.
    ///
    /// Auth: `merchant`, which must be an active allowlisted merchant.
    /// Errors: `MerchantNotFound`, `MerchantInactive`, `NameTooLong`
    /// (> 64 bytes), `InvalidAmount` (<= 0), `InvalidPeriod` (outside
    /// 1..=366 days). Events: `plan_created`.
    pub fn create_plan(
        env: Env,
        merchant: Address,
        name: String,
        amount: i128,
        period: u64,
    ) -> Result<u32, Error> {
        merchant.require_auth();
        require_active_merchant(&env, &merchant)?;
        if name.len() > MAX_NAME_LEN {
            return Err(Error::NameTooLong);
        }
        require_positive(amount)?;
        require_valid_period(period)?;
        storage::extend_instance(&env);

        let id = storage::next_plan_id(&env);
        storage::set_plan(
            &env,
            &Plan {
                id,
                merchant: merchant.clone(),
                name: name.clone(),
                amount,
                period,
                active: true,
            },
        );
        events::PlanCreated {
            merchant,
            plan_id: id,
            name,
            amount,
            period,
        }
        .publish(&env);
        Ok(id)
    }

    /// Activate or deactivate a plan. A deactivated plan accepts no new
    /// subscribers; existing subscriptions keep being charged until their
    /// subscribers cancel.
    ///
    /// Auth: `merchant`, which must own the plan (an inactive merchant can
    /// still deactivate its plans). Errors: `PlanNotFound`, `NotPlanOwner`.
    /// Events: `plan_active_set`.
    pub fn set_plan_active(
        env: Env,
        merchant: Address,
        plan_id: u32,
        active: bool,
    ) -> Result<(), Error> {
        merchant.require_auth();
        let mut plan = storage::get_plan(&env, plan_id).ok_or(Error::PlanNotFound)?;
        if plan.merchant != merchant {
            return Err(Error::NotPlanOwner);
        }
        storage::extend_instance(&env);
        plan.active = active;
        storage::set_plan(&env, &plan);
        events::PlanActiveSet {
            merchant,
            plan_id,
            active,
        }
        .publish(&env);
        Ok(())
    }

    // ---- admin: fees -------------------------------------------------------

    /// Set the fee taken from every charge, in basis points.
    ///
    /// Auth: admin. Errors: `FeeTooHigh` (> 1_000). Events: `fee_set`.
    pub fn set_fee(env: Env, fee_bps: u32) -> Result<(), Error> {
        storage::get_admin(&env).require_auth();
        require_fee_in_range(fee_bps)?;
        storage::extend_instance(&env);
        storage::set_fee_bps(&env, fee_bps);
        events::FeeSet { fee_bps }.publish(&env);
        Ok(())
    }

    /// Set the address that receives fees.
    ///
    /// Auth: admin. Errors: none. Events: `fee_recipient_set`.
    pub fn set_fee_recipient(env: Env, fee_recipient: Address) {
        storage::get_admin(&env).require_auth();
        storage::extend_instance(&env);
        storage::set_fee_recipient(&env, &fee_recipient);
        events::FeeRecipientSet { fee_recipient }.publish(&env);
    }

    // ---- admin: two-step transfer ------------------------------------------

    /// Propose `new_admin` as the next admin. Nothing changes until the
    /// proposed admin calls `accept_admin`. Proposing again replaces the
    /// pending proposal.
    ///
    /// Auth: admin. Errors: none. Events: `admin_proposed`.
    pub fn propose_admin(env: Env, new_admin: Address) {
        storage::get_admin(&env).require_auth();
        storage::extend_instance(&env);
        storage::set_pending_admin(&env, &new_admin);
        events::AdminProposed {
            pending_admin: new_admin,
        }
        .publish(&env);
    }

    /// Accept a pending admin proposal and become the admin.
    ///
    /// Auth: the pending admin.
    /// Errors: `NoPendingAdmin` when nothing is pending.
    /// Events: `admin_transferred`.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let pending = storage::get_pending_admin(&env).ok_or(Error::NoPendingAdmin)?;
        pending.require_auth();
        storage::extend_instance(&env);

        let old_admin = storage::get_admin(&env);
        storage::set_admin(&env, &pending);
        storage::clear_pending_admin(&env);

        events::AdminTransferred {
            new_admin: pending,
            old_admin,
        }
        .publish(&env);
        Ok(())
    }

    // ---- admin: pause and upgrade ------------------------------------------

    /// Pause the contract: `subscribe`, `charge` and `charge_batch` are
    /// blocked. Subscribers can still pause, resume and cancel their own
    /// subscriptions.
    ///
    /// Auth: admin. Errors: none. Events: `paused`.
    pub fn pause(env: Env) {
        storage::get_admin(&env).require_auth();
        storage::extend_instance(&env);
        storage::set_paused(&env, true);
        events::Paused {}.publish(&env);
    }

    /// Resume normal operation after `pause`.
    ///
    /// Auth: admin. Errors: none. Events: `unpaused`.
    pub fn unpause(env: Env) {
        storage::get_admin(&env).require_auth();
        storage::extend_instance(&env);
        storage::set_paused(&env, false);
        events::Unpaused {}.publish(&env);
    }

    /// Replace the contract code with the already-uploaded wasm identified
    /// by `new_wasm_hash`. Storage is kept; the new code takes effect after
    /// this invocation completes.
    ///
    /// Auth: admin. Errors: none (the host fails if the wasm is not
    /// uploaded). Events: `upgraded`.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::get_admin(&env).require_auth();
        storage::extend_instance(&env);
        env.deployer()
            .update_current_contract(ContractExecutable::Wasm(new_wasm_hash.clone()));
        events::Upgraded {
            wasm_hash: new_wasm_hash,
        }
        .publish(&env);
    }

    // ---- reads -------------------------------------------------------------

    /// Merchant record, if `merchant` is allowlisted. Auth: none.
    pub fn get_merchant(env: Env, merchant: Address) -> Option<Merchant> {
        storage::get_merchant(&env, &merchant)
    }

    /// Plan by id. Auth: none. Errors: `PlanNotFound`.
    pub fn get_plan(env: Env, plan_id: u32) -> Result<Plan, Error> {
        storage::get_plan(&env, plan_id).ok_or(Error::PlanNotFound)
    }

    /// Current admin. Auth: none.
    pub fn admin(env: Env) -> Address {
        storage::get_admin(&env)
    }

    /// Proposed admin awaiting `accept_admin`, if any. Auth: none.
    pub fn pending_admin(env: Env) -> Option<Address> {
        storage::get_pending_admin(&env)
    }

    /// The USDC Stellar Asset Contract address. Auth: none.
    pub fn token(env: Env) -> Address {
        storage::get_token(&env)
    }

    /// Address that receives fees. Auth: none.
    pub fn fee_recipient(env: Env) -> Address {
        storage::get_fee_recipient(&env)
    }

    /// Fee in basis points. Auth: none.
    pub fn fee_bps(env: Env) -> u32 {
        storage::get_fee_bps(&env)
    }

    /// Whether the contract is paused. Auth: none.
    pub fn is_paused(env: Env) -> bool {
        storage::is_paused(&env)
    }
}

// ---- internal helpers --------------------------------------------------------

fn require_fee_in_range(fee_bps: u32) -> Result<(), Error> {
    if fee_bps > MAX_FEE_BPS {
        return Err(Error::FeeTooHigh);
    }
    Ok(())
}

fn require_positive(amount: i128) -> Result<(), Error> {
    if amount <= 0 {
        return Err(Error::InvalidAmount);
    }
    Ok(())
}

fn require_valid_period(period: u64) -> Result<(), Error> {
    if !(MIN_PERIOD..=MAX_PERIOD).contains(&period) {
        return Err(Error::InvalidPeriod);
    }
    Ok(())
}

/// The merchant record, or an error if it is missing or inactive.
fn require_active_merchant(env: &Env, merchant: &Address) -> Result<Merchant, Error> {
    let record = storage::get_merchant(env, merchant).ok_or(Error::MerchantNotFound)?;
    if !record.active {
        return Err(Error::MerchantInactive);
    }
    Ok(record)
}
