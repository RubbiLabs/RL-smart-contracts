#![cfg(test)]

extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::{
        Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, MockAuth,
        MockAuthInvoke,
    },
    token::{StellarAssetClient, TokenClient},
    Address, Env, Event as _, IntoVal,
};

use crate::{events, Error, VaultContract, VaultContractClient};

/// One USDC in its smallest unit (7 decimals on Stellar).
const USDC: i128 = 10_000_000;

/// Everything a test needs: the env, the roles, the token and the vault.
struct Setup {
    env: Env,
    admin: Address,
    operator: Address,
    treasury: Address,
    token: TokenClient<'static>,
    token_admin: StellarAssetClient<'static>,
    vault: Address,
    client: VaultContractClient<'static>,
}

fn setup() -> Setup {
    let env = Env::default();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let treasury = Address::generate(&env);
    let token_issuer = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(token_issuer);
    let token = TokenClient::new(&env, &sac.address());
    let token_admin = StellarAssetClient::new(&env, &sac.address());

    let vault = env.register(
        VaultContract,
        (&admin, &operator, &treasury, &sac.address()),
    );
    let client = VaultContractClient::new(&env, &vault);

    Setup {
        env,
        admin,
        operator,
        treasury,
        token,
        token_admin,
        vault,
        client,
    }
}

/// A fresh user holding `amount` tokens in their wallet (minted with all auths
/// mocked, then mocking is left enabled for the caller to override).
fn funded_user(s: &Setup, amount: i128) -> Address {
    let user = Address::generate(&s.env);
    s.env.mock_all_auths();
    s.token_admin.mint(&user, &amount);
    user
}

/// The authorization tree the vault requires for `deposit(user, amount)`:
/// the user authorizes the call and, nested in it, the SAC transfer.
fn deposit_auth<'a>(
    s: &'a Setup,
    user: &'a Address,
    amount: i128,
) -> (
    soroban_sdk::Vec<soroban_sdk::Val>,
    soroban_sdk::Vec<soroban_sdk::Val>,
) {
    (
        (user.clone(), amount).into_val(&s.env),
        (user.clone(), s.vault.clone(), amount).into_val(&s.env),
    )
}

// ---- constructor -------------------------------------------------------------

#[test]
fn constructor_stores_roles_and_token() {
    let s = setup();
    assert_eq!(s.client.admin(), s.admin);
    assert_eq!(s.client.operator(), s.operator);
    assert_eq!(s.client.treasury(), s.treasury);
    assert_eq!(s.client.token(), s.token.address);
    assert_eq!(s.client.pending_admin(), None);
    assert!(!s.client.is_paused());
    assert_eq!(s.client.total_balances(), 0);
}

// ---- 1. deposit / withdraw round trip (L2) -----------------------------------

#[test]
fn deposit_withdraw_round_trip() {
    let s = setup();
    let user = funded_user(&s, 1_000 * USDC);

    s.client.deposit(&user, &(400 * USDC));
    assert_eq!(s.client.balance(&user), 400 * USDC);
    assert_eq!(s.client.total_balances(), 400 * USDC);
    assert_eq!(s.token.balance(&user), 600 * USDC);
    assert_eq!(s.token.balance(&s.vault), 400 * USDC);

    // L2: withdraw sends tokens to the user, not to the contract.
    s.client.withdraw(&user, &(150 * USDC));
    assert_eq!(s.client.balance(&user), 250 * USDC);
    assert_eq!(s.client.total_balances(), 250 * USDC);
    assert_eq!(s.token.balance(&user), 750 * USDC);
    assert_eq!(s.token.balance(&s.vault), 250 * USDC);

    s.client.withdraw(&user, &(250 * USDC));
    assert_eq!(s.client.balance(&user), 0);
    assert_eq!(s.client.total_balances(), 0);
    assert_eq!(s.token.balance(&user), 1_000 * USDC);
    assert_eq!(s.token.balance(&s.vault), 0);
}

// ---- 2. withdraw requires the owner's auth (L1) ------------------------------

#[test]
fn withdraw_requires_exact_user_auth() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);

    // Exact auth tree: the user authorizes `withdraw(user, 40)` and nothing
    // else (the vault→user token transfer is authorized by the contract).
    s.client.withdraw(&user, &40);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            user.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    symbol_short!("withdraw"),
                    (user.clone(), 40_i128).into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
}

#[test]
fn withdraw_by_another_signer_fails() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);

    // An attacker signs a withdraw of the victim's balance: only the
    // attacker's own authorization is present, so `user.require_auth()`
    // fails before any state changes.
    let attacker = Address::generate(&s.env);
    s.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &s.vault,
            fn_name: "withdraw",
            args: (user.clone(), 60_i128).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    let res = s.client.try_withdraw(&user, &60);
    assert!(matches!(res, Err(Err(_))), "expected a host auth error");

    s.env.mock_all_auths();
    assert_eq!(s.client.balance(&user), 100);
    assert_eq!(s.token.balance(&s.vault), 100);
}

#[test]
fn deposit_requires_user_auth_with_token_transfer_sub_invocation() {
    let s = setup();
    let user = funded_user(&s, 100);
    let (args, transfer_args) = deposit_auth(&s, &user, 70);

    // Only the user's authorization for exactly this call is mocked.
    s.env.mock_auths(&[MockAuth {
        address: &user,
        invoke: &MockAuthInvoke {
            contract: &s.vault,
            fn_name: "deposit",
            args: args.clone(),
            sub_invokes: &[MockAuthInvoke {
                contract: &s.token.address,
                fn_name: "transfer",
                args: transfer_args.clone(),
                sub_invokes: &[],
            }],
        },
    }]);
    s.client.deposit(&user, &70);

    // `env.auths()` describes the last invocation, so assert it before any
    // other call.
    assert_eq!(
        s.env.auths(),
        std::vec![(
            user.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    symbol_short!("deposit"),
                    args,
                )),
                sub_invocations: std::vec![AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        s.token.address.clone(),
                        symbol_short!("transfer"),
                        transfer_args,
                    )),
                    sub_invocations: std::vec![],
                }],
            }
        )]
    );
    assert_eq!(s.client.balance(&user), 70);
}

// ---- 8. amount <= 0 rejected --------------------------------------------------

#[test]
fn deposit_and_withdraw_reject_non_positive_amounts() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &50);

    for amount in [0_i128, -1, i128::MIN] {
        assert_eq!(
            s.client.try_deposit(&user, &amount),
            Err(Ok(Error::InvalidAmount))
        );
        assert_eq!(
            s.client.try_withdraw(&user, &amount),
            Err(Ok(Error::InvalidAmount))
        );
    }
    assert_eq!(s.client.balance(&user), 50);
    assert_eq!(s.client.total_balances(), 50);
}

// ---- 11. withdraw more than balance -------------------------------------------

#[test]
fn withdraw_more_than_balance_fails_without_state_change() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &60);

    assert_eq!(
        s.client.try_withdraw(&user, &61),
        Err(Ok(Error::InsufficientBalance))
    );
    // A user with no balance at all gets the same error.
    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.client.try_withdraw(&stranger, &1),
        Err(Ok(Error::InsufficientBalance))
    );

    assert_eq!(s.client.balance(&user), 60);
    assert_eq!(s.client.total_balances(), 60);
    assert_eq!(s.token.balance(&user), 40);
    assert_eq!(s.token.balance(&s.vault), 60);
}

// ---- 14 (part). events --------------------------------------------------------

#[test]
fn deposit_and_withdraw_emit_events() {
    let s = setup();
    let user = funded_user(&s, 100);

    s.client.deposit(&user, &80);
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.vault),
        [events::Deposit {
            user: user.clone(),
            amount: 80,
        }
        .to_xdr(&s.env, &s.vault)]
    );

    s.client.withdraw(&user, &30);
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.vault),
        [events::Withdraw {
            user: user.clone(),
            amount: 30,
        }
        .to_xdr(&s.env, &s.vault)]
    );
}
