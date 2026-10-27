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

use soroban_sdk::{contract, contractimpl, token, Address, BytesN, ContractExecutable, Env};

pub use errors::Error;

/// Hard cap on `fee_bps`: 1_000 = 10%.
pub const MAX_FEE_BPS: u32 = 1_000;
/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;
/// Maximum number of items accepted by `disburse_batch`.
pub const MAX_BATCH: u32 = 25;
/// Maximum number of schedules created in one `create_schedules` call.
pub const MAX_SCHEDULES_PER_CALL: u32 = 50;
/// Shortest allowed schedule period: one day, in seconds.
pub const MIN_PERIOD: u64 = 86_400;
/// Longest allowed schedule period: 366 days, in seconds.
pub const MAX_PERIOD: u64 = 366 * 86_400;
/// Longest allowed schedule label, in bytes.
pub const MAX_LABEL_LEN: u32 = 64;

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
