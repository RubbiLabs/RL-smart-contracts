#![cfg(test)]

extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::{
        Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, MockAuth,
        MockAuthInvoke,
    },
    token::{StellarAssetClient, TokenClient},
    xdr::ScErrorType,
    Address, BytesN, ConversionError, Env, Event as _, IntoVal, InvokeError, Symbol,
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

/// Whether a `try_*` call to a function *without* a contract error type
/// (returning `()`) failed with a host error such as a failed
/// `require_auth`. Functions returning `Result<_, Error>` surface the same
/// failure as `Err(Err(_))` instead, and tests match on that directly.
fn is_host_error<T: core::fmt::Debug>(
    res: &Result<Result<T, ConversionError>, Result<soroban_sdk::Error, InvokeError>>,
) -> bool {
    matches!(res, Err(Ok(e)) if !e.is_type(ScErrorType::Contract))
}

/// A deterministic 32-byte reference.
fn reference(s: &Setup, seed: u8) -> BytesN<32> {
    BytesN::from_array(&s.env, &[seed; 32])
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

// ---- 3. only the operator can settle (L1) -------------------------------------

#[test]
fn settle_requires_exact_operator_auth() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);
    let r = reference(&s, 1);

    s.client.settle_card_spend(&user, &40, &r);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.operator.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    Symbol::new(&s.env, "settle_card_spend"),
                    (user.clone(), 40_i128, r.clone()).into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
    assert_eq!(s.client.balance(&user), 60);
    assert_eq!(s.client.total_balances(), 60);
    assert_eq!(s.token.balance(&s.vault), 60);
    assert_eq!(s.token.balance(&s.treasury), 40);
}

#[test]
fn settle_by_non_operator_fails() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);
    let r = reference(&s, 1);

    // Neither the user themself, the admin, nor a stranger can settle.
    for signer in [&user, &s.admin, &Address::generate(&s.env)] {
        s.env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &s.vault,
                fn_name: "settle_card_spend",
                args: (user.clone(), 40_i128, r.clone()).into_val(&s.env),
                sub_invokes: &[],
            },
        }]);
        let res = s.client.try_settle_card_spend(&user, &40, &r);
        assert!(matches!(res, Err(Err(_))), "expected a host auth error");
    }

    s.env.mock_all_auths();
    assert_eq!(s.client.balance(&user), 100);
    assert_eq!(s.token.balance(&s.treasury), 0);
}

// ---- 4. duplicate spend reference rejected ------------------------------------

#[test]
fn duplicate_spend_reference_is_rejected() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);
    let r = reference(&s, 7);

    s.client.settle_card_spend(&user, &10, &r);
    assert_eq!(
        s.client.try_settle_card_spend(&user, &10, &r),
        Err(Ok(Error::DuplicateReference))
    );
    // Same reference, different user or amount: still a duplicate.
    let other = funded_user(&s, 100);
    s.client.deposit(&other, &100);
    assert_eq!(
        s.client.try_settle_card_spend(&other, &5, &r),
        Err(Ok(Error::DuplicateReference))
    );
    // A fresh reference works.
    s.client.settle_card_spend(&user, &10, &reference(&s, 8));

    assert_eq!(s.client.balance(&user), 80);
    assert_eq!(s.client.balance(&other), 100);
    assert_eq!(s.token.balance(&s.treasury), 20);
}

// ---- 5. refund requires treasury auth; separate reference namespace ----------

#[test]
fn refund_requires_exact_treasury_auth() {
    let s = setup();
    let user = funded_user(&s, 0);
    s.token_admin.mint(&s.treasury, &500);
    let r = reference(&s, 3);

    s.client.credit_refund(&user, &120, &r);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.treasury.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    Symbol::new(&s.env, "credit_refund"),
                    (user.clone(), 120_i128, r.clone()).into_val(&s.env),
                )),
                sub_invocations: std::vec![AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        s.token.address.clone(),
                        symbol_short!("transfer"),
                        (s.treasury.clone(), s.vault.clone(), 120_i128).into_val(&s.env),
                    )),
                    sub_invocations: std::vec![],
                }],
            }
        )]
    );
    assert_eq!(s.client.balance(&user), 120);
    assert_eq!(s.client.total_balances(), 120);
    assert_eq!(s.token.balance(&s.vault), 120);
    assert_eq!(s.token.balance(&s.treasury), 380);
}

#[test]
fn refund_by_non_treasury_fails() {
    let s = setup();
    let user = funded_user(&s, 0);
    s.token_admin.mint(&s.treasury, &500);
    let r = reference(&s, 3);

    for signer in [&user, &s.admin, &s.operator] {
        s.env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &s.vault,
                fn_name: "credit_refund",
                args: (user.clone(), 120_i128, r.clone()).into_val(&s.env),
                sub_invokes: &[MockAuthInvoke {
                    contract: &s.token.address,
                    fn_name: "transfer",
                    args: (s.treasury.clone(), s.vault.clone(), 120_i128).into_val(&s.env),
                    sub_invokes: &[],
                }],
            },
        }]);
        let res = s.client.try_credit_refund(&user, &120, &r);
        assert!(matches!(res, Err(Err(_))), "expected a host auth error");
    }

    s.env.mock_all_auths();
    assert_eq!(s.client.balance(&user), 0);
    assert_eq!(s.token.balance(&s.treasury), 500);
}

#[test]
fn spend_and_refund_references_are_separate_namespaces() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);
    s.token_admin.mint(&s.treasury, &500);
    let r = reference(&s, 9);

    // The same 32 bytes can be used once as a spend and once as a refund...
    s.client.settle_card_spend(&user, &30, &r);
    s.client.credit_refund(&user, &30, &r);
    // ...but not twice in either namespace.
    assert_eq!(
        s.client.try_settle_card_spend(&user, &1, &r),
        Err(Ok(Error::DuplicateReference))
    );
    assert_eq!(
        s.client.try_credit_refund(&user, &1, &r),
        Err(Ok(Error::DuplicateReference))
    );
    assert_eq!(s.client.balance(&user), 100);
}

// ---- 8 (cont). amount <= 0 rejected on settle and refund ----------------------

#[test]
fn settle_and_refund_reject_non_positive_amounts() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &50);
    s.token_admin.mint(&s.treasury, &500);

    for amount in [0_i128, -1, i128::MIN] {
        assert_eq!(
            s.client
                .try_settle_card_spend(&user, &amount, &reference(&s, 1)),
            Err(Ok(Error::InvalidAmount))
        );
        assert_eq!(
            s.client
                .try_credit_refund(&user, &amount, &reference(&s, 2)),
            Err(Ok(Error::InvalidAmount))
        );
    }
    // The references were not consumed by the rejected calls.
    s.client.settle_card_spend(&user, &1, &reference(&s, 1));
    s.client.credit_refund(&user, &1, &reference(&s, 2));
    assert_eq!(s.client.balance(&user), 50);
}

// ---- 12. settle more than balance ---------------------------------------------

#[test]
fn settle_more_than_balance_fails_and_keeps_reference_unused() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &60);
    let r = reference(&s, 5);

    assert_eq!(
        s.client.try_settle_card_spend(&user, &61, &r),
        Err(Ok(Error::InsufficientBalance))
    );
    assert_eq!(s.client.balance(&user), 60);
    assert_eq!(s.client.total_balances(), 60);
    assert_eq!(s.token.balance(&s.vault), 60);
    assert_eq!(s.token.balance(&s.treasury), 0);

    // The failed call did not consume the reference.
    s.client.settle_card_spend(&user, &60, &r);
    assert_eq!(s.client.balance(&user), 0);
    assert_eq!(s.token.balance(&s.treasury), 60);
}

// ---- 14 (cont). settle and refund events --------------------------------------

#[test]
fn settle_and_refund_emit_events() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);
    s.token_admin.mint(&s.treasury, &500);

    let r1 = reference(&s, 11);
    s.client.settle_card_spend(&user, &25, &r1);
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.vault),
        [events::CardSpendSettled {
            user: user.clone(),
            amount: 25,
            reference: r1,
        }
        .to_xdr(&s.env, &s.vault)]
    );

    let r2 = reference(&s, 12);
    s.client.credit_refund(&user, &5, &r2);
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.vault),
        [events::RefundCredited {
            user: user.clone(),
            amount: 5,
            reference: r2,
        }
        .to_xdr(&s.env, &s.vault)]
    );
}

// ---- auth helper for argument-less admin calls --------------------------------

/// Mock `signer` authorizing `fn_name` on the vault with `args`.
fn mock_auth_for(
    s: &Setup,
    signer: &Address,
    fn_name: &str,
    args: soroban_sdk::Vec<soroban_sdk::Val>,
) {
    s.env.mock_auths(&[MockAuth {
        address: signer,
        invoke: &MockAuthInvoke {
            contract: &s.vault,
            fn_name,
            args,
            sub_invokes: &[],
        },
    }]);
}

// ---- 6. pause blocks deposit and settle, allows withdraw ----------------------

#[test]
fn pause_blocks_deposit_and_settle_but_allows_withdraw_and_refund() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &80);
    s.token_admin.mint(&s.treasury, &500);

    s.client.pause();
    assert!(s.client.is_paused());

    assert_eq!(s.client.try_deposit(&user, &10), Err(Ok(Error::Paused)));
    assert_eq!(
        s.client
            .try_settle_card_spend(&user, &10, &reference(&s, 1)),
        Err(Ok(Error::Paused))
    );
    // Users can always get their money out.
    s.client.withdraw(&user, &30);
    assert_eq!(s.client.balance(&user), 50);
    assert_eq!(s.token.balance(&user), 50);
    // Refunds in favour of users are not blocked either.
    s.client.credit_refund(&user, &5, &reference(&s, 2));
    assert_eq!(s.client.balance(&user), 55);

    s.client.unpause();
    assert!(!s.client.is_paused());
    s.client.deposit(&user, &10);
    s.client.settle_card_spend(&user, &10, &reference(&s, 1));
    assert_eq!(s.client.balance(&user), 55);
}

// ---- 15. pause / unpause are admin-only ---------------------------------------

#[test]
fn pause_and_unpause_require_exact_admin_auth() {
    let s = setup();
    s.env.mock_all_auths();

    s.client.pause();
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    symbol_short!("pause"),
                    ().into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
    s.client.unpause();
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    symbol_short!("unpause"),
                    ().into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
}

#[test]
fn pause_and_unpause_by_non_admin_fail() {
    let s = setup();
    for signer in [&s.operator, &s.treasury, &Address::generate(&s.env)] {
        mock_auth_for(&s, signer, "pause", ().into_val(&s.env));
        assert!(is_host_error(&s.client.try_pause()));
    }
    s.env.mock_all_auths();
    assert!(!s.client.is_paused());
    s.client.pause();

    for signer in [&s.operator, &s.treasury, &Address::generate(&s.env)] {
        mock_auth_for(&s, signer, "unpause", ().into_val(&s.env));
        assert!(is_host_error(&s.client.try_unpause()));
    }
    s.env.mock_all_auths();
    assert!(s.client.is_paused());
}

// ---- 9. admin two-step transfer -----------------------------------------------

#[test]
fn admin_two_step_transfer() {
    let s = setup();
    s.env.mock_all_auths();
    let new_admin = Address::generate(&s.env);

    // Nothing pending: accept fails with a contract error.
    assert_eq!(s.client.try_accept_admin(), Err(Ok(Error::NoPendingAdmin)));

    // Only the admin can propose.
    mock_auth_for(
        &s,
        &new_admin,
        "propose_admin",
        (new_admin.clone(),).into_val(&s.env),
    );
    assert!(is_host_error(&s.client.try_propose_admin(&new_admin)));
    s.env.mock_all_auths();
    assert_eq!(s.client.pending_admin(), None);

    s.client.propose_admin(&new_admin);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    Symbol::new(&s.env, "propose_admin"),
                    (new_admin.clone(),).into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
    assert_eq!(s.client.pending_admin(), Some(new_admin.clone()));
    // The proposal changes nothing yet: the old admin still has rights.
    assert_eq!(s.client.admin(), s.admin);

    // Neither the old admin nor a stranger can accept on the proposed
    // admin's behalf.
    for signer in [&s.admin, &Address::generate(&s.env)] {
        mock_auth_for(&s, signer, "accept_admin", ().into_val(&s.env));
        assert!(matches!(s.client.try_accept_admin(), Err(Err(_))));
    }

    // The proposed admin accepts.
    mock_auth_for(&s, &new_admin, "accept_admin", ().into_val(&s.env));
    s.client.accept_admin();
    assert_eq!(
        s.env.auths(),
        std::vec![(
            new_admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    Symbol::new(&s.env, "accept_admin"),
                    ().into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
    assert_eq!(s.client.admin(), new_admin);
    assert_eq!(s.client.pending_admin(), None);

    // The old admin lost its rights; the new admin has them.
    mock_auth_for(&s, &s.admin, "pause", ().into_val(&s.env));
    assert!(is_host_error(&s.client.try_pause()));
    mock_auth_for(&s, &new_admin, "pause", ().into_val(&s.env));
    s.client.pause();
    s.env.mock_all_auths();
    assert!(s.client.is_paused());
}

// ---- 13. set_operator / set_treasury ------------------------------------------

#[test]
fn set_operator_and_set_treasury_are_admin_only_and_take_effect() {
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);
    let new_operator = Address::generate(&s.env);
    let new_treasury = Address::generate(&s.env);

    for signer in [&s.operator, &s.treasury, &new_operator] {
        mock_auth_for(
            &s,
            signer,
            "set_operator",
            (new_operator.clone(),).into_val(&s.env),
        );
        assert!(is_host_error(&s.client.try_set_operator(&new_operator)));
        mock_auth_for(
            &s,
            signer,
            "set_treasury",
            (new_treasury.clone(),).into_val(&s.env),
        );
        assert!(is_host_error(&s.client.try_set_treasury(&new_treasury)));
    }
    s.env.mock_all_auths();
    assert_eq!(s.client.operator(), s.operator);
    assert_eq!(s.client.treasury(), s.treasury);

    s.client.set_operator(&new_operator);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    Symbol::new(&s.env, "set_operator"),
                    (new_operator.clone(),).into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
    s.client.set_treasury(&new_treasury);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    Symbol::new(&s.env, "set_treasury"),
                    (new_treasury.clone(),).into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
    assert_eq!(s.client.operator(), new_operator);
    assert_eq!(s.client.treasury(), new_treasury);

    // The old operator can no longer settle; the new one can, and the
    // settlement goes to the new treasury.
    let r = reference(&s, 1);
    mock_auth_for(
        &s,
        &s.operator,
        "settle_card_spend",
        (user.clone(), 40_i128, r.clone()).into_val(&s.env),
    );
    assert!(matches!(
        s.client.try_settle_card_spend(&user, &40, &r),
        Err(Err(_))
    ));
    mock_auth_for(
        &s,
        &new_operator,
        "settle_card_spend",
        (user.clone(), 40_i128, r.clone()).into_val(&s.env),
    );
    s.client.settle_card_spend(&user, &40, &r);
    s.env.mock_all_auths();
    assert_eq!(s.token.balance(&new_treasury), 40);
    assert_eq!(s.token.balance(&s.treasury), 0);
}

// ---- 10. upgrade is admin-only ------------------------------------------------

#[test]
fn upgrade_by_non_admin_fails() {
    let s = setup();
    let fake_hash = BytesN::from_array(&s.env, &[0xAB; 32]);
    for signer in [&s.operator, &s.treasury, &Address::generate(&s.env)] {
        mock_auth_for(&s, signer, "upgrade", (fake_hash.clone(),).into_val(&s.env));
        assert!(is_host_error(&s.client.try_upgrade(&fake_hash)));
    }
    // The contract still runs its original code.
    s.env.mock_all_auths();
    assert_eq!(s.client.admin(), s.admin);
}

/// Positive upgrade path. Uploads this contract's own optimized wasm
/// (built by `stellar contract build`) and upgrades to it, which exercises
/// the full admin-authorized `update_current_contract` path.
#[cfg(feature = "wasm-upgrade-test")]
#[test]
fn upgrade_by_admin_replaces_the_executable() {
    const WASM: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/wasm32v1-none/release/rubbi_vault.wasm"
    ));
    let s = setup();
    let user = funded_user(&s, 100);
    s.client.deposit(&user, &100);

    let wasm_hash = s
        .env
        .deployer()
        .upload_contract_wasm(soroban_sdk::Bytes::from_slice(&s.env, WASM));

    mock_auth_for(
        &s,
        &s.admin,
        "upgrade",
        (wasm_hash.clone(),).into_val(&s.env),
    );
    s.client.upgrade(&wasm_hash);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.vault.clone(),
                    symbol_short!("upgrade"),
                    (wasm_hash.clone(),).into_val(&s.env),
                )),
                sub_invocations: std::vec![],
            }
        )]
    );
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.vault),
        [events::Upgraded { wasm_hash }.to_xdr(&s.env, &s.vault)]
    );

    // Storage survived and the (now wasm) contract still works.
    s.env.mock_all_auths();
    assert_eq!(s.client.admin(), s.admin);
    assert_eq!(s.client.balance(&user), 100);
    s.client.withdraw(&user, &100);
    assert_eq!(s.token.balance(&user), 100);
}
