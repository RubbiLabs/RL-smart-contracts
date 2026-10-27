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
    contract, contractimpl, contracttype, token, Address, BytesN, ContractExecutable, Env, String,
    Symbol, Vec,
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

/// Lifecycle state of a subscription.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubStatus {
    /// Charged every period.
    Active,
    /// Paused by the subscriber; not charged until resumed.
    Paused,
    /// The last charge failed; retried by the next `charge` call.
    PastDue,
    /// Cancelled by the subscriber. Final.
    Cancelled,
}

/// A subscription of `subscriber` to `plan_id`. No personal data is stored.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subscription {
    pub id: u64,
    pub subscriber: Address,
    pub plan_id: u32,
    /// Ledger timestamp (seconds) from which the next charge is allowed.
    pub next_due: u64,
    pub status: SubStatus,
    pub created_at: u64,
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

    // ---- subscriptions (subscriber) ----------------------------------------

    /// Subscribe `subscriber` to `plan_id` and return the new subscription
    /// id. The first period is charged immediately by pulling `plan.amount`
    /// from the subscriber's wallet (this contract must hold a sufficient
    /// token allowance as spender); `next_due` becomes `now + period`.
    ///
    /// If the pull fails, the whole call fails with `PaymentFailed` and no
    /// subscription is created.
    ///
    /// Auth: `subscriber`.
    /// Errors: `Paused`, `PlanNotFound`, `PlanInactive`, `MerchantNotFound`,
    /// `MerchantInactive`, `AlreadySubscribed` (a non-cancelled subscription
    /// to this plan exists), `PaymentFailed`, `Overflow`.
    /// Events: `subscribed`, then `charged`.
    pub fn subscribe(env: Env, subscriber: Address, plan_id: u32) -> Result<u64, Error> {
        subscriber.require_auth();
        require_not_paused(&env)?;
        let plan = storage::get_plan(&env, plan_id).ok_or(Error::PlanNotFound)?;
        if !plan.active {
            return Err(Error::PlanInactive);
        }
        let merchant = require_active_merchant(&env, &plan.merchant)?;
        if storage::get_sub_id_of(&env, &subscriber, plan_id).is_some() {
            return Err(Error::AlreadySubscribed);
        }
        storage::extend_instance(&env);

        let fee = fee_for(&env, plan.amount)?;
        let now = env.ledger().timestamp();
        let next_due = now.checked_add(plan.period).ok_or(Error::Overflow)?;

        // Interaction first: the subscription only exists if the first
        // period was actually paid. A failed call rolls everything back.
        if pull(&env, &subscriber, plan.amount).is_err() {
            return Err(Error::PaymentFailed);
        }

        let id = storage::next_sub_id(&env);
        storage::set_subscription(
            &env,
            &Subscription {
                id,
                subscriber: subscriber.clone(),
                plan_id,
                next_due,
                status: SubStatus::Active,
                created_at: now,
            },
        );
        storage::set_sub_id_of(&env, &subscriber, plan_id, id);

        pay_out(&env, &merchant.payout, plan.amount, fee);

        events::Subscribed {
            subscriber: subscriber.clone(),
            sub_id: id,
            plan_id,
            next_due,
        }
        .publish(&env);
        events::Charged {
            subscriber,
            sub_id: id,
            plan_id,
            merchant: plan.merchant,
            amount: plan.amount,
            fee,
            next_due,
        }
        .publish(&env);
        Ok(id)
    }

    /// Pause subscription `sub_id`: it is not charged until resumed.
    /// Allowed while the contract is paused.
    ///
    /// Auth: `subscriber`, who must own the subscription.
    /// Errors: `SubscriptionNotFound`, `NotSubscriber`, `NotPausable` (status
    /// is not `Active` or `PastDue`).
    /// Events: `subscription_paused`.
    pub fn pause_subscription(env: Env, subscriber: Address, sub_id: u64) -> Result<(), Error> {
        subscriber.require_auth();
        let mut sub = owned_subscription(&env, &subscriber, sub_id)?;
        if !matches!(sub.status, SubStatus::Active | SubStatus::PastDue) {
            return Err(Error::NotPausable);
        }
        storage::extend_instance(&env);
        sub.status = SubStatus::Paused;
        storage::set_subscription(&env, &sub);
        events::SubscriptionPaused { subscriber, sub_id }.publish(&env);
        Ok(())
    }

    /// Resume a paused subscription. `next_due` becomes
    /// `max(next_due, now)`: periods that fell due while paused are not
    /// charged, and a subscription whose period has not elapsed yet keeps
    /// its original due date. Allowed while the contract is paused.
    ///
    /// Auth: `subscriber`, who must own the subscription.
    /// Errors: `SubscriptionNotFound`, `NotSubscriber`, `NotPaused`.
    /// Events: `subscription_resumed`.
    pub fn resume_subscription(env: Env, subscriber: Address, sub_id: u64) -> Result<(), Error> {
        subscriber.require_auth();
        let mut sub = owned_subscription(&env, &subscriber, sub_id)?;
        if sub.status != SubStatus::Paused {
            return Err(Error::NotPaused);
        }
        storage::extend_instance(&env);
        sub.status = SubStatus::Active;
        sub.next_due = sub.next_due.max(env.ledger().timestamp());
        storage::set_subscription(&env, &sub);
        events::SubscriptionResumed {
            subscriber,
            sub_id,
            next_due: sub.next_due,
        }
        .publish(&env);
        Ok(())
    }

    /// Cancel subscription `sub_id`. Final: the subscription can never be
    /// charged or resumed again, and the subscriber may subscribe to the
    /// same plan afresh. Allowed while the contract is paused.
    ///
    /// Auth: `subscriber`, who must own the subscription.
    /// Errors: `SubscriptionNotFound`, `NotSubscriber`, `AlreadyCancelled`.
    /// Events: `subscription_cancelled`.
    pub fn cancel_subscription(env: Env, subscriber: Address, sub_id: u64) -> Result<(), Error> {
        subscriber.require_auth();
        let mut sub = owned_subscription(&env, &subscriber, sub_id)?;
        if sub.status == SubStatus::Cancelled {
            return Err(Error::AlreadyCancelled);
        }
        storage::extend_instance(&env);
        sub.status = SubStatus::Cancelled;
        storage::set_subscription(&env, &sub);
        storage::remove_sub_id_of(&env, &subscriber, sub.plan_id);
        events::SubscriptionCancelled { subscriber, sub_id }.publish(&env);
        Ok(())
    }

    // ---- charging (permissionless, time-gated) -----------------------------

    /// Charge one period of subscription `sub_id`, if it is due.
    ///
    /// Requires status `Active` or `PastDue` and `now >= next_due`. The plan
    /// amount is pulled from the subscriber in one `transfer_from`; on
    /// success the merchant receives `amount - fee`, the fee recipient
    /// `fee`, the status becomes `Active` and `next_due` advances by one
    /// period (if that is still in the past, to `now + period`: a subscriber
    /// who was past due for three months is charged once, not three times).
    ///
    /// Returns `false` without changing `next_due` in two cases:
    /// - the pull failed (allowance or balance): status becomes `PastDue`
    ///   and `charge_failed` is emitted;
    /// - the plan's merchant is inactive: nothing is pulled, nothing is
    ///   written, `charge_skipped` is emitted. Charging resumes when the
    ///   admin reactivates the merchant.
    ///
    /// Auth: none (permissionless).
    /// Errors: `Paused`, `SubscriptionNotFound`, `NotChargeable` (paused or
    /// cancelled), `NotDue`, `PlanNotFound`, `MerchantNotFound`, `Overflow`.
    /// Events: `charged`, `charge_failed` or `charge_skipped`.
    pub fn charge(env: Env, sub_id: u64) -> Result<bool, Error> {
        require_not_paused(&env)?;
        storage::extend_instance(&env);
        charge_one(&env, sub_id)
    }

    /// Charge every subscription in `sub_ids` that is due. Returns one
    /// `bool` per input id, in order: `true` when a period was charged,
    /// `false` when the item was skipped for any per-item reason (not due,
    /// not chargeable, unknown id, inactive merchant) or its pull failed
    /// (status set to `PastDue`, as in `charge`).
    ///
    /// Auth: none (permissionless).
    /// Errors: `BatchTooLarge` (> 25 ids), `Paused`. Per-item errors never
    /// fail the batch.
    /// Events: as `charge`, per item.
    pub fn charge_batch(env: Env, sub_ids: Vec<u64>) -> Result<Vec<bool>, Error> {
        if sub_ids.len() > MAX_BATCH {
            return Err(Error::BatchTooLarge);
        }
        require_not_paused(&env)?;
        storage::extend_instance(&env);

        let mut results = Vec::new(&env);
        for sub_id in sub_ids.iter() {
            results.push_back(charge_one(&env, sub_id).unwrap_or(false));
        }
        Ok(results)
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

    /// Subscription by id. Auth: none. Errors: `SubscriptionNotFound`.
    pub fn get_subscription(env: Env, sub_id: u64) -> Result<Subscription, Error> {
        storage::get_subscription(&env, sub_id).ok_or(Error::SubscriptionNotFound)
    }

    /// Id of `subscriber`'s non-cancelled subscription to `plan_id`, if
    /// any. Auth: none.
    pub fn sub_id_of(env: Env, subscriber: Address, plan_id: u32) -> Option<u64> {
        storage::get_sub_id_of(&env, &subscriber, plan_id)
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

fn require_not_paused(env: &Env) -> Result<(), Error> {
    if storage::is_paused(env) {
        return Err(Error::Paused);
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

/// The subscription `sub_id`, which must belong to `subscriber`.
fn owned_subscription(env: &Env, subscriber: &Address, sub_id: u64) -> Result<Subscription, Error> {
    let sub = storage::get_subscription(env, sub_id).ok_or(Error::SubscriptionNotFound)?;
    if sub.subscriber != *subscriber {
        return Err(Error::NotSubscriber);
    }
    Ok(sub)
}

/// `amount * fee_bps / 10_000`, rounded down.
fn fee_for(env: &Env, amount: i128) -> Result<i128, Error> {
    let fee = amount
        .checked_mul(i128::from(storage::get_fee_bps(env)))
        .ok_or(Error::Overflow)?
        / BPS_DENOMINATOR;
    Ok(fee)
}

/// Pull `amount` from `from` into this contract using the allowance `from`
/// granted to this contract. Never panics: a failed `transfer_from` (no or
/// expired allowance, insufficient balance) is returned as `Err`.
fn pull(env: &Env, from: &Address, amount: i128) -> Result<(), ()> {
    let this = env.current_contract_address();
    token::TokenClient::new(env, &storage::get_token(env))
        .try_transfer_from(&this, from, &this, &amount)
        .map(|_| ())
        .map_err(|_| ())
}

/// Pay `amount - fee` to `payout` and `fee` to the fee recipient out of the
/// tokens this contract just pulled.
fn pay_out(env: &Env, payout: &Address, amount: i128, fee: i128) {
    let token = token::TokenClient::new(env, &storage::get_token(env));
    let this = env.current_contract_address();
    token.transfer(&this, payout, &(amount - fee));
    if fee > 0 {
        token.transfer(&this, storage::get_fee_recipient(env), &fee);
    }
}

/// No back-charging: advance by one period; if that is still not in the
/// future, restart from `now`.
fn advance_next_due(next_due: u64, period: u64, now: u64) -> Result<u64, Error> {
    let advanced = next_due.checked_add(period).ok_or(Error::Overflow)?;
    if advanced <= now {
        return now.checked_add(period).ok_or(Error::Overflow);
    }
    Ok(advanced)
}

/// The body of `charge` for one subscription, without the pause check.
fn charge_one(env: &Env, sub_id: u64) -> Result<bool, Error> {
    let mut sub = storage::get_subscription(env, sub_id).ok_or(Error::SubscriptionNotFound)?;
    if !matches!(sub.status, SubStatus::Active | SubStatus::PastDue) {
        return Err(Error::NotChargeable);
    }
    let now = env.ledger().timestamp();
    if now < sub.next_due {
        return Err(Error::NotDue);
    }
    let plan = storage::get_plan(env, sub.plan_id).ok_or(Error::PlanNotFound)?;
    let merchant = storage::get_merchant(env, &plan.merchant).ok_or(Error::MerchantNotFound)?;

    if !merchant.active {
        events::ChargeSkipped {
            subscriber: sub.subscriber,
            sub_id,
            reason: Symbol::new(env, "merchant_inactive"),
        }
        .publish(env);
        return Ok(false);
    }

    let fee = fee_for(env, plan.amount)?;
    let next_due = advance_next_due(sub.next_due, plan.period, now)?;

    // The outcome of the pull decides the new state, so state is written
    // after it is known. A failed pull is rolled back by the host and
    // leaves the tokens untouched.
    if pull(env, &sub.subscriber, plan.amount).is_err() {
        sub.status = SubStatus::PastDue;
        storage::set_subscription(env, &sub);
        events::ChargeFailed {
            subscriber: sub.subscriber,
            sub_id,
            plan_id: sub.plan_id,
            amount: plan.amount,
        }
        .publish(env);
        return Ok(false);
    }

    sub.status = SubStatus::Active;
    sub.next_due = next_due;
    storage::set_subscription(env, &sub);

    pay_out(env, &merchant.payout, plan.amount, fee);

    events::Charged {
        subscriber: sub.subscriber,
        sub_id,
        plan_id: sub.plan_id,
        merchant: plan.merchant,
        amount: plan.amount,
        fee,
        next_due,
    }
    .publish(env);
    Ok(true)
}
