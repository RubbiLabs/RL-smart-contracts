//! Rubbi vault: USDC custody for card funding and settlement.
//!
//! Users deposit USDC to fund their Rubbi card. Card spends happen off-chain
//! through the card issuer; the backend operator then settles each spend
//! on-chain by moving the user's vault balance to the treasury. The treasury
//! can credit refunds back to a user.
//!
//! Roles (set in the constructor, rotatable by the admin):
//! - `admin`: configuration, pause, upgrade, role rotation (two-step transfer)
//! - `operator`: settles card spends (`settle_card_spend`)
//! - `treasury`: receives settlements and funds refunds (`credit_refund`)
//!
//! Users can always withdraw their own balance, even while the vault is
//! paused. Pause only blocks deposits and settlements.

#![no_std]

mod errors;
mod events;
mod storage;

#[cfg(test)]
mod test;

use soroban_sdk::{contract, contractimpl, token, Address, Env};

pub use errors::Error;

/// The vault contract.
#[contract]
pub struct VaultContract;

#[contractimpl]
impl VaultContract {
    /// Initialize the vault. Runs once, at deployment.
    ///
    /// - `admin`: can rotate roles, pause/unpause, upgrade, and hand over
    ///   admin rights with a two-step transfer.
    /// - `operator`: backend key allowed to settle card spends.
    /// - `treasury`: receives settled spends and funds refunds.
    /// - `token`: the USDC Stellar Asset Contract address.
    ///
    /// Auth: none (constructor). Errors: none. Events: none.
    pub fn __constructor(
        env: Env,
        admin: Address,
        operator: Address,
        treasury: Address,
        token: Address,
    ) {
        storage::set_admin(&env, &admin);
        storage::set_operator(&env, &operator);
        storage::set_treasury(&env, &treasury);
        storage::set_token(&env, &token);
        storage::set_paused(&env, false);
        storage::set_total_balances(&env, 0);
        storage::extend_instance(&env);
    }

    // ---- user funds --------------------------------------------------------

    /// Deposit `amount` of the vault token from `user` into their vault
    /// balance.
    ///
    /// Auth: `user`.
    /// Errors: `InvalidAmount` (amount <= 0), `Paused`, `Overflow`.
    /// Events: `deposit`.
    pub fn deposit(env: Env, user: Address, amount: i128) -> Result<(), Error> {
        user.require_auth();
        require_positive(amount)?;
        require_not_paused(&env)?;
        storage::extend_instance(&env);

        credit(&env, &user, amount)?;

        token::TokenClient::new(&env, &storage::get_token(&env)).transfer(
            &user,
            env.current_contract_address(),
            &amount,
        );

        events::Deposit { user, amount }.publish(&env);
        Ok(())
    }

    /// Withdraw `amount` from `user`'s vault balance back to their wallet.
    /// Allowed while the vault is paused.
    ///
    /// Auth: `user`.
    /// Errors: `InvalidAmount` (amount <= 0), `InsufficientBalance`.
    /// Events: `withdraw`.
    pub fn withdraw(env: Env, user: Address, amount: i128) -> Result<(), Error> {
        user.require_auth();
        require_positive(amount)?;
        storage::extend_instance(&env);

        debit(&env, &user, amount)?;

        token::TokenClient::new(&env, &storage::get_token(&env)).transfer(
            &env.current_contract_address(),
            user.clone(),
            &amount,
        );

        events::Withdraw { user, amount }.publish(&env);
        Ok(())
    }

    // ---- reads -------------------------------------------------------------

    /// `user`'s vault balance. Auth: none.
    pub fn balance(env: Env, user: Address) -> i128 {
        storage::get_balance(&env, &user)
    }

    /// Current admin. Auth: none.
    pub fn admin(env: Env) -> Address {
        storage::get_admin(&env)
    }

    /// Proposed admin awaiting `accept_admin`, if any. Auth: none.
    pub fn pending_admin(env: Env) -> Option<Address> {
        storage::get_pending_admin(&env)
    }

    /// Current operator. Auth: none.
    pub fn operator(env: Env) -> Address {
        storage::get_operator(&env)
    }

    /// Current treasury. Auth: none.
    pub fn treasury(env: Env) -> Address {
        storage::get_treasury(&env)
    }

    /// The USDC Stellar Asset Contract address. Auth: none.
    pub fn token(env: Env) -> Address {
        storage::get_token(&env)
    }

    /// Whether the vault is paused. Auth: none.
    pub fn is_paused(env: Env) -> bool {
        storage::is_paused(&env)
    }

    /// Sum of all user balances. Equals the vault's token balance as long as
    /// nobody transfers tokens to the vault directly. Auth: none.
    pub fn total_balances(env: Env) -> i128 {
        storage::get_total_balances(&env)
    }
}

// ---- internal helpers --------------------------------------------------------

fn require_positive(amount: i128) -> Result<(), Error> {
    if amount <= 0 {
        return Err(Error::InvalidAmount);
    }
    Ok(())
}

fn require_not_paused(env: &Env) -> Result<(), Error> {
    if storage::is_paused(env) {
        return Err(Error::Paused);
    }
    Ok(())
}

/// Add `amount` to `user`'s balance and to the running total.
fn credit(env: &Env, user: &Address, amount: i128) -> Result<(), Error> {
    let balance = storage::get_balance(env, user)
        .checked_add(amount)
        .ok_or(Error::Overflow)?;
    let total = storage::get_total_balances(env)
        .checked_add(amount)
        .ok_or(Error::Overflow)?;
    storage::set_balance(env, user, balance);
    storage::set_total_balances(env, total);
    Ok(())
}

/// Remove `amount` from `user`'s balance and from the running total.
fn debit(env: &Env, user: &Address, amount: i128) -> Result<(), Error> {
    let balance = storage::get_balance(env, user);
    if balance < amount {
        return Err(Error::InsufficientBalance);
    }
    let total = storage::get_total_balances(env)
        .checked_sub(amount)
        .ok_or(Error::Overflow)?;
    storage::set_balance(env, user, balance - amount);
    storage::set_total_balances(env, total);
    Ok(())
}
