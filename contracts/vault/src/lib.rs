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

use soroban_sdk::{contract, contractimpl, Address, Env};

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

    // ---- reads -------------------------------------------------------------

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
