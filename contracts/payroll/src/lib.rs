//! Rubbi payroll: employer-funded recurring salary payments.
//!
//! Employers pre-fund an escrow held by this contract and create schedules
//! that pay a recipient a fixed amount every period. Anyone may call
//! `disburse` / `disburse_batch` once a schedule is due (Rubbi's keeper
//! does, but the functions are permissionless and time-gated).
//!
//! Roles:
//! - `admin`: fee settings, pause, upgrade and two-step admin transfer.
//! - employers: fund and withdraw their own escrow; create, pause, resume
//!   and cancel their own schedules.
//!
//! Fees: the employer's escrow is debited exactly the schedule amount; the
//! recipient receives `amount - fee` and the fee recipient receives
//! `fee = amount * fee_bps / 10_000` (rounded down).
//!
//! Back pay: salary that fell due while nobody called `disburse` is still
//! owed. Each call pays exactly one period and advances `next_due` by
//! exactly one period, so repeated calls catch up one period at a time.
//!
//! Pause blocks funding, new schedules and disbursements. It never blocks
//! employers from withdrawing their escrow or managing their schedules.

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
/// Maximum number of items accepted by `disburse_batch`.
pub const MAX_BATCH: u32 = 25;
/// Maximum number of schedules created in one `create_schedules` call.
///
/// Each schedule emits a `schedule_created` event and a transaction may
/// emit at most 16 KiB of contract events; 25 schedules use about half of
/// that, leaving headroom for future network-config changes.
pub const MAX_SCHEDULES_PER_CALL: u32 = 25;
/// Shortest allowed schedule period: one day, in seconds.
pub const MIN_PERIOD: u64 = 86_400;
/// Longest allowed schedule period: 366 days, in seconds.
pub const MAX_PERIOD: u64 = 366 * 86_400;
/// Longest allowed schedule label, in bytes.
pub const MAX_LABEL_LEN: u32 = 64;

// ---- public types --------------------------------------------------------------

/// Lifecycle state of a schedule.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleStatus {
    /// Disbursed every period.
    Active,
    /// Paused by the employer; not disbursed until resumed. Periods that
    /// fall due while paused stay owed.
    Paused,
    /// Cancelled by the employer. Final.
    Cancelled,
}

/// One schedule to create. `label` is a short description such as
/// "Design lead" (at most 64 bytes); no personal data beyond the recipient
/// address is stored.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleInput {
    pub recipient: Address,
    pub amount: i128,
    pub period: u64,
    pub label: String,
}

/// A recurring salary payment from `employer` to `recipient`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Schedule {
    pub id: u64,
    pub employer: Address,
    pub recipient: Address,
    pub amount: i128,
    pub period: u64,
    /// Ledger timestamp (seconds) from which the next period may be paid.
    pub next_due: u64,
    pub status: ScheduleStatus,
    pub label: String,
    pub created_at: u64,
}

/// The payroll contract.
#[contract]
pub struct PayrollContract;

#[contractimpl]
impl PayrollContract {
    /// Initialize the contract. Runs once, at deployment.
    ///
    /// - `admin`: fee settings, pause, upgrade and two-step admin transfer.
    /// - `token`: the USDC Stellar Asset Contract address.
    /// - `fee_recipient`: receives the fee share of every disbursement.
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
        storage::set_total_escrow(&env, 0);
        storage::extend_instance(&env);
        Ok(())
    }

    // ---- escrow (employer) -------------------------------------------------

    /// Move `amount` from `employer`'s wallet into their escrow.
    ///
    /// Auth: `employer`.
    /// Errors: `InvalidAmount` (amount <= 0), `Paused`, `Overflow`.
    /// Events: `funded`.
    pub fn fund(env: Env, employer: Address, amount: i128) -> Result<(), Error> {
        employer.require_auth();
        require_positive(amount)?;
        require_not_paused(&env)?;
        storage::extend_instance(&env);

        credit_escrow(&env, &employer, amount)?;

        token::TokenClient::new(&env, &storage::get_token(&env)).transfer(
            &employer,
            env.current_contract_address(),
            &amount,
        );

        events::Funded { employer, amount }.publish(&env);
        Ok(())
    }

    /// Move `amount` from `employer`'s escrow back to their wallet. Allowed
    /// while the contract is paused.
    ///
    /// Auth: `employer`.
    /// Errors: `InvalidAmount` (amount <= 0), `InsufficientEscrow`.
    /// Events: `escrow_withdrawn`.
    pub fn withdraw_escrow(env: Env, employer: Address, amount: i128) -> Result<(), Error> {
        employer.require_auth();
        require_positive(amount)?;
        storage::extend_instance(&env);

        debit_escrow(&env, &employer, amount)?;

        token::TokenClient::new(&env, &storage::get_token(&env)).transfer(
            &env.current_contract_address(),
            employer.clone(),
            &amount,
        );

        events::EscrowWithdrawn { employer, amount }.publish(&env);
        Ok(())
    }

    // ---- schedules (employer) ----------------------------------------------

    /// Create up to 25 schedules for `employer`, all first due at
    /// `first_due`, and return their ids in input order. Every input is
    /// validated before anything is written, so the call is atomic: one bad
    /// input creates no schedules at all. Escrow is not checked here; it is
    /// checked at each disbursement.
    ///
    /// Auth: `employer`.
    /// Errors: `Paused`, `TooManySchedules` (> 25 inputs), `FirstDueInPast`
    /// (`first_due < now`), `InvalidAmount` (<= 0), `InvalidPeriod` (outside
    /// 1..=366 days), `LabelTooLong` (> 64 bytes).
    /// Events: one `schedule_created` per schedule.
    pub fn create_schedules(
        env: Env,
        employer: Address,
        inputs: Vec<ScheduleInput>,
        first_due: u64,
    ) -> Result<Vec<u64>, Error> {
        employer.require_auth();
        require_not_paused(&env)?;
        if inputs.len() > MAX_SCHEDULES_PER_CALL {
            return Err(Error::TooManySchedules);
        }
        let now = env.ledger().timestamp();
        if first_due < now {
            return Err(Error::FirstDueInPast);
        }
        // Validate everything first...
        for input in inputs.iter() {
            require_positive(input.amount)?;
            require_valid_period(input.period)?;
            if input.label.len() > MAX_LABEL_LEN {
                return Err(Error::LabelTooLong);
            }
        }
        storage::extend_instance(&env);

        // ...then write.
        let mut ids = Vec::new(&env);
        for input in inputs.iter() {
            let id = storage::next_schedule_id(&env);
            storage::set_schedule(
                &env,
                &Schedule {
                    id,
                    employer: employer.clone(),
                    recipient: input.recipient.clone(),
                    amount: input.amount,
                    period: input.period,
                    next_due: first_due,
                    status: ScheduleStatus::Active,
                    label: input.label.clone(),
                    created_at: now,
                },
            );
            ids.push_back(id);
            events::ScheduleCreated {
                employer: employer.clone(),
                schedule_id: id,
                recipient: input.recipient,
                amount: input.amount,
                period: input.period,
                next_due: first_due,
            }
            .publish(&env);
        }
        Ok(ids)
    }

    /// Pause schedule `schedule_id`: it is not disbursed until resumed.
    /// Periods that fall due meanwhile stay owed (`next_due` is not moved).
    /// Allowed while the contract is paused.
    ///
    /// Auth: `employer`, who must own the schedule.
    /// Errors: `ScheduleNotFound`, `NotEmployer`, `NotActive`.
    /// Events: `schedule_paused`.
    pub fn pause_schedule(env: Env, employer: Address, schedule_id: u64) -> Result<(), Error> {
        employer.require_auth();
        let mut schedule = owned_schedule(&env, &employer, schedule_id)?;
        if schedule.status != ScheduleStatus::Active {
            return Err(Error::NotActive);
        }
        storage::extend_instance(&env);
        schedule.status = ScheduleStatus::Paused;
        storage::set_schedule(&env, &schedule);
        events::SchedulePaused {
            employer,
            schedule_id,
        }
        .publish(&env);
        Ok(())
    }

    /// Resume a paused schedule. `next_due` is kept as it was: owed salary
    /// stays owed and is caught up one period per `disburse` call. Allowed
    /// while the contract is paused.
    ///
    /// Auth: `employer`, who must own the schedule.
    /// Errors: `ScheduleNotFound`, `NotEmployer`, `NotPaused`.
    /// Events: `schedule_resumed`.
    pub fn resume_schedule(env: Env, employer: Address, schedule_id: u64) -> Result<(), Error> {
        employer.require_auth();
        let mut schedule = owned_schedule(&env, &employer, schedule_id)?;
        if schedule.status != ScheduleStatus::Paused {
            return Err(Error::NotPaused);
        }
        storage::extend_instance(&env);
        schedule.status = ScheduleStatus::Active;
        storage::set_schedule(&env, &schedule);
        events::ScheduleResumed {
            employer,
            schedule_id,
            next_due: schedule.next_due,
        }
        .publish(&env);
        Ok(())
    }

    /// Cancel an `Active` or `Paused` schedule. Final: it can never be
    /// disbursed or resumed again. The employer's escrow is untouched (it is
    /// shared by all their schedules) and can be withdrawn with
    /// `withdraw_escrow`. Allowed while the contract is paused.
    ///
    /// Auth: `employer`, who must own the schedule.
    /// Errors: `ScheduleNotFound`, `NotEmployer`, `AlreadyCancelled`.
    /// Events: `schedule_cancelled`.
    pub fn cancel_schedule(env: Env, employer: Address, schedule_id: u64) -> Result<(), Error> {
        employer.require_auth();
        let mut schedule = owned_schedule(&env, &employer, schedule_id)?;
        if schedule.status == ScheduleStatus::Cancelled {
            return Err(Error::AlreadyCancelled);
        }
        storage::extend_instance(&env);
        schedule.status = ScheduleStatus::Cancelled;
        storage::set_schedule(&env, &schedule);
        events::ScheduleCancelled {
            employer,
            schedule_id,
        }
        .publish(&env);
        Ok(())
    }

    // ---- disbursing (permissionless, time-gated) ---------------------------

    /// Pay one period of schedule `schedule_id`, if it is due.
    ///
    /// Requires status `Active` and `now >= next_due`. Returns `false` with
    /// nothing changed (escrow, totals and `next_due` untouched, so the
    /// period stays owed) and emits `disburse_failed` in two cases:
    /// - `reason = insufficient_escrow`: the employer's escrow is smaller
    ///   than the schedule amount;
    /// - `reason = recipient_unpayable`: the recipient cannot receive USDC
    ///   (no trustline, frozen trustline, or trustline limit exceeded).
    ///
    /// Otherwise the escrow is debited `amount`, `next_due` advances by
    /// exactly one period, the recipient receives `amount - fee` and the fee
    /// recipient `fee`, `disbursed` is emitted and `true` is returned. The
    /// fee recipient is admin-controlled and its USDC trustline is created
    /// by the deploy script; a fee transfer that fails anyway aborts the
    /// whole call.
    ///
    /// Back pay: `next_due` always advances by exactly one period, so N
    /// missed periods are paid by N calls, each of them time-gated.
    ///
    /// Auth: none (permissionless).
    /// Errors: `Paused`, `ScheduleNotFound`, `NotActive` (paused or
    /// cancelled), `NotDue`, `Overflow`.
    /// Events: `disbursed` or `disburse_failed`.
    pub fn disburse(env: Env, schedule_id: u64) -> Result<bool, Error> {
        require_not_paused(&env)?;
        storage::extend_instance(&env);
        disburse_one(&env, schedule_id)
    }

    /// Pay one period of every schedule in `ids` that is due. Returns one
    /// `bool` per input id, in order: `true` when a period was paid, `false`
    /// when the item was skipped for any per-item reason (not due, not
    /// active, unknown id) or failed as in `disburse` (insufficient escrow,
    /// unpayable recipient). One recipient who cannot receive USDC never
    /// blocks the other items.
    ///
    /// Auth: none (permissionless).
    /// Errors: `BatchTooLarge` (> 25 ids), `Paused`. Per-item errors never
    /// fail the batch.
    /// Events: as `disburse`, per item.
    pub fn disburse_batch(env: Env, ids: Vec<u64>) -> Result<Vec<bool>, Error> {
        if ids.len() > MAX_BATCH {
            return Err(Error::BatchTooLarge);
        }
        require_not_paused(&env)?;
        storage::extend_instance(&env);

        let mut results = Vec::new(&env);
        for id in ids.iter() {
            results.push_back(disburse_one(&env, id).unwrap_or(false));
        }
        Ok(results)
    }

    // ---- admin: fees -------------------------------------------------------

    /// Set the fee taken from every disbursement, in basis points.
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

    /// Pause the contract: `fund`, `create_schedules`, `disburse` and
    /// `disburse_batch` are blocked. Employers can still withdraw escrow
    /// and pause, resume or cancel their schedules.
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

    /// Schedule by id. Auth: none. Errors: `ScheduleNotFound`.
    pub fn get_schedule(env: Env, schedule_id: u64) -> Result<Schedule, Error> {
        storage::get_schedule(&env, schedule_id).ok_or(Error::ScheduleNotFound)
    }

    /// `employer`'s escrow balance. Auth: none.
    pub fn escrow_of(env: Env, employer: Address) -> i128 {
        storage::get_escrow(&env, &employer)
    }

    /// Sum of all employer escrows. Equals the contract's token balance as
    /// long as nobody transfers tokens to the contract directly. Auth: none.
    pub fn total_escrow(env: Env) -> i128 {
        storage::get_total_escrow(&env)
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

/// The schedule `schedule_id`, which must belong to `employer`.
fn owned_schedule(env: &Env, employer: &Address, schedule_id: u64) -> Result<Schedule, Error> {
    let schedule = storage::get_schedule(env, schedule_id).ok_or(Error::ScheduleNotFound)?;
    if schedule.employer != *employer {
        return Err(Error::NotEmployer);
    }
    Ok(schedule)
}

/// `amount * fee_bps / 10_000`, rounded down.
fn fee_for(env: &Env, amount: i128) -> Result<i128, Error> {
    let fee = amount
        .checked_mul(i128::from(storage::get_fee_bps(env)))
        .ok_or(Error::Overflow)?
        / BPS_DENOMINATOR;
    Ok(fee)
}

/// The body of `disburse` for one schedule, without the pause check.
///
/// Ordering, deliberately: the recipient payout (`try_transfer`) runs
/// *before* the storage writes, so a recipient who cannot receive USDC
/// leaves no trace (no escrow debit, no `next_due` change) and is reported
/// per item instead of aborting the call. This is the one place in Rubbi
/// where an interaction precedes effects. It is safe because the Soroban
/// host rejects contract re-entry ("Contract re-entry is not allowed"), so
/// the token contract cannot call back into this contract while the payout
/// is in flight, and because the host rolls back a failed `try_transfer`
/// completely. The fee transfer stays a plain `transfer`: the fee recipient
/// is admin-controlled and the deploy script guarantees its trustline.
fn disburse_one(env: &Env, schedule_id: u64) -> Result<bool, Error> {
    let mut schedule = storage::get_schedule(env, schedule_id).ok_or(Error::ScheduleNotFound)?;
    if schedule.status != ScheduleStatus::Active {
        return Err(Error::NotActive);
    }
    let now = env.ledger().timestamp();
    if now < schedule.next_due {
        return Err(Error::NotDue);
    }

    let escrow = storage::get_escrow(env, &schedule.employer);
    if escrow < schedule.amount {
        events::DisburseFailed {
            employer: schedule.employer,
            schedule_id,
            amount: schedule.amount,
            escrow,
            reason: Symbol::new(env, "insufficient_escrow"),
        }
        .publish(env);
        return Ok(false);
    }

    let fee = fee_for(env, schedule.amount)?;
    let next_due = schedule
        .next_due
        .checked_add(schedule.period)
        .ok_or(Error::Overflow)?;

    // Interaction first (see above): pay the recipient out of the escrow
    // this contract holds. Any failure (missing or frozen trustline, limit
    // exceeded) is rolled back by the host and reported per item.
    let token = token::TokenClient::new(env, &storage::get_token(env));
    let this = env.current_contract_address();
    if token
        .try_transfer(&this, &schedule.recipient, &(schedule.amount - fee))
        .is_err()
    {
        events::DisburseFailed {
            employer: schedule.employer,
            schedule_id,
            amount: schedule.amount,
            escrow,
            reason: Symbol::new(env, "recipient_unpayable"),
        }
        .publish(env);
        return Ok(false);
    }

    // Effects: debit the escrow and advance by exactly one period (L3).
    debit_escrow(env, &schedule.employer, schedule.amount)?;
    schedule.next_due = next_due;
    storage::set_schedule(env, &schedule);

    // The fee recipient is admin-controlled: a plain transfer.
    if fee > 0 {
        token.transfer(&this, storage::get_fee_recipient(env), &fee);
    }

    events::Disbursed {
        employer: schedule.employer,
        schedule_id,
        recipient: schedule.recipient,
        amount: schedule.amount,
        fee,
        next_due: schedule.next_due,
    }
    .publish(env);
    Ok(true)
}

/// Add `amount` to `employer`'s escrow and to the running total.
fn credit_escrow(env: &Env, employer: &Address, amount: i128) -> Result<(), Error> {
    let balance = storage::get_escrow(env, employer)
        .checked_add(amount)
        .ok_or(Error::Overflow)?;
    let total = storage::get_total_escrow(env)
        .checked_add(amount)
        .ok_or(Error::Overflow)?;
    storage::set_escrow(env, employer, balance);
    storage::set_total_escrow(env, total);
    Ok(())
}

/// Remove `amount` from `employer`'s escrow and from the running total.
fn debit_escrow(env: &Env, employer: &Address, amount: i128) -> Result<(), Error> {
    let balance = storage::get_escrow(env, employer);
    if balance < amount {
        return Err(Error::InsufficientEscrow);
    }
    let total = storage::get_total_escrow(env)
        .checked_sub(amount)
        .ok_or(Error::Overflow)?;
    storage::set_escrow(env, employer, balance - amount);
    storage::set_total_escrow(env, total);
    Ok(())
}
