#![cfg(test)]

extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::{
        Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, Ledger as _, MockAuth,
        MockAuthInvoke,
    },
    token::{StellarAssetClient, TokenClient},
    xdr::ScErrorType,
    Address, ConversionError, Env, Event as _, IntoVal, InvokeError, Symbol,
};

use crate::{events, Error, PayrollContract, PayrollContractClient};

/// Default fee used by the tests: 50 bps = 0.5%.
const FEE_BPS: u32 = 50;
/// One USDC in its smallest unit (7 decimals on Stellar).
const USDC: i128 = 10_000_000;
/// Ledger timestamp at which every test starts (2023-11-14T22:13:20Z).
const START: u64 = 1_700_000_000;

/// Everything a test needs: the env, the roles, the token and the contract.
struct Setup {
    env: Env,
    admin: Address,
    fee_recipient: Address,
    token: TokenClient<'static>,
    token_admin: StellarAssetClient<'static>,
    contract: Address,
    client: PayrollContractClient<'static>,
}

fn setup() -> Setup {
    let env = Env::default();
    env.ledger().with_mut(|li| {
        li.timestamp = START;
        li.sequence_number = 1_000;
    });
    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let token_issuer = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(token_issuer);
    let token = TokenClient::new(&env, &sac.address());
    let token_admin = StellarAssetClient::new(&env, &sac.address());

    let contract = env.register(
        PayrollContract,
        (&admin, &sac.address(), &fee_recipient, FEE_BPS),
    );
    let client = PayrollContractClient::new(&env, &contract);

    Setup {
        env,
        admin,
        fee_recipient,
        token,
        token_admin,
        contract,
        client,
    }
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

/// Mock `signer` authorizing `fn_name` on the contract with `args` and no
/// sub-invocations.
fn mock_auth_for(
    s: &Setup,
    signer: &Address,
    fn_name: &str,
    args: soroban_sdk::Vec<soroban_sdk::Val>,
) {
    s.env.mock_auths(&[MockAuth {
        address: signer,
        invoke: &MockAuthInvoke {
            contract: &s.contract,
            fn_name,
            args,
            sub_invokes: &[],
        },
    }]);
}

/// The single top-level authorization `signer` must have given for
/// `fn_name(args)` on the contract, with no sub-invocations.
fn expected_auth(
    s: &Setup,
    signer: &Address,
    fn_name: &str,
    args: soroban_sdk::Vec<soroban_sdk::Val>,
) -> std::vec::Vec<(Address, AuthorizedInvocation)> {
    std::vec![(
        signer.clone(),
        AuthorizedInvocation {
            function: AuthorizedFunction::Contract((
                s.contract.clone(),
                Symbol::new(&s.env, fn_name),
                args,
            )),
            sub_invocations: std::vec![],
        }
    )]
}

/// Events emitted by the contract during the last invocation.
fn last_events(s: &Setup) -> soroban_sdk::testutils::ContractEvents {
    s.env.events().all().filter_by_contract(&s.contract)
}

/// A fresh employer wallet holding `balance` tokens (all auths mocked).
fn employer(s: &Setup, balance: i128) -> Address {
    let employer = Address::generate(&s.env);
    s.env.mock_all_auths();
    if balance > 0 {
        s.token_admin.mint(&employer, &balance);
    }
    employer
}

// ---- constructor -------------------------------------------------------------

#[test]
fn constructor_stores_config() {
    let s = setup();
    assert_eq!(s.client.admin(), s.admin);
    assert_eq!(s.client.token(), s.token.address);
    assert_eq!(s.client.fee_recipient(), s.fee_recipient);
    assert_eq!(s.client.fee_bps(), FEE_BPS);
    assert_eq!(s.client.pending_admin(), None);
    assert!(!s.client.is_paused());
    assert_eq!(s.client.total_escrow(), 0);
}

#[test]
#[should_panic]
fn constructor_rejects_fee_over_cap() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    env.register(
        PayrollContract,
        (&admin, &token, &fee_recipient, crate::MAX_FEE_BPS + 1),
    );
}

#[test]
fn set_fee_enforces_cap() {
    let s = setup();
    s.env.mock_all_auths();
    assert_eq!(
        s.client.try_set_fee(&(crate::MAX_FEE_BPS + 1)),
        Err(Ok(Error::FeeTooHigh))
    );
    s.client.set_fee(&crate::MAX_FEE_BPS);
    assert_eq!(s.client.fee_bps(), crate::MAX_FEE_BPS);
}

// ---- escrow ------------------------------------------------------------------

#[test]
fn fund_and_withdraw_escrow_round_trip() {
    let s = setup();
    let boss = employer(&s, 1_000 * USDC);

    s.client.fund(&boss, &(400 * USDC));
    // Exact auth: the employer authorizes `fund` and, nested, the token
    // transfer into the contract.
    assert_eq!(
        s.env.auths(),
        std::vec![(
            boss.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    s.contract.clone(),
                    symbol_short!("fund"),
                    (boss.clone(), 400 * USDC).into_val(&s.env),
                )),
                sub_invocations: std::vec![AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        s.token.address.clone(),
                        symbol_short!("transfer"),
                        (boss.clone(), s.contract.clone(), 400 * USDC).into_val(&s.env),
                    )),
                    sub_invocations: std::vec![],
                }],
            }
        )]
    );
    assert_eq!(
        last_events(&s),
        [events::Funded {
            employer: boss.clone(),
            amount: 400 * USDC,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(s.client.escrow_of(&boss), 400 * USDC);
    assert_eq!(s.client.total_escrow(), 400 * USDC);
    assert_eq!(s.token.balance(&boss), 600 * USDC);
    assert_eq!(s.token.balance(&s.contract), 400 * USDC);

    s.client.withdraw_escrow(&boss, &(150 * USDC));
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &boss,
            "withdraw_escrow",
            (boss.clone(), 150 * USDC).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::EscrowWithdrawn {
            employer: boss.clone(),
            amount: 150 * USDC,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(s.client.escrow_of(&boss), 250 * USDC);
    assert_eq!(s.client.total_escrow(), 250 * USDC);
    assert_eq!(s.token.balance(&boss), 750 * USDC);
    assert_eq!(s.token.balance(&s.contract), 250 * USDC);

    // More than the escrow, or a stranger's empty escrow: rejected, no change.
    assert_eq!(
        s.client.try_withdraw_escrow(&boss, &(250 * USDC + 1)),
        Err(Ok(Error::InsufficientEscrow))
    );
    assert_eq!(
        s.client.try_withdraw_escrow(&Address::generate(&s.env), &1),
        Err(Ok(Error::InsufficientEscrow))
    );
    for amount in [0_i128, -1, i128::MIN] {
        assert_eq!(
            s.client.try_fund(&boss, &amount),
            Err(Ok(Error::InvalidAmount))
        );
        assert_eq!(
            s.client.try_withdraw_escrow(&boss, &amount),
            Err(Ok(Error::InvalidAmount))
        );
    }
    assert_eq!(s.client.escrow_of(&boss), 250 * USDC);
    assert_eq!(s.token.balance(&s.contract), 250 * USDC);
}

#[test]
fn escrow_functions_by_another_signer_fail() {
    let s = setup();
    let boss = employer(&s, 100 * USDC);
    s.client.fund(&boss, &(100 * USDC));
    let attacker = Address::generate(&s.env);

    // The attacker signs a withdrawal of the employer's escrow to the
    // employer's own wallet: only the attacker's authorization is present.
    mock_auth_for(
        &s,
        &attacker,
        "withdraw_escrow",
        (boss.clone(), 60 * USDC).into_val(&s.env),
    );
    assert!(matches!(
        s.client.try_withdraw_escrow(&boss, &(60 * USDC)),
        Err(Err(_))
    ));
    mock_auth_for(
        &s,
        &attacker,
        "fund",
        (boss.clone(), 10 * USDC).into_val(&s.env),
    );
    assert!(matches!(
        s.client.try_fund(&boss, &(10 * USDC)),
        Err(Err(_))
    ));
    s.env.mock_all_auths();
    assert_eq!(s.client.escrow_of(&boss), 100 * USDC);
    assert_eq!(s.token.balance(&s.contract), 100 * USDC);
}

#[test]
fn pause_blocks_fund_but_allows_withdraw_escrow() {
    let s = setup();
    let boss = employer(&s, 100 * USDC);
    s.client.fund(&boss, &(80 * USDC));

    s.client.pause();
    assert!(s.client.is_paused());
    assert_eq!(
        s.client.try_fund(&boss, &(10 * USDC)),
        Err(Ok(Error::Paused))
    );
    // Employers can always get their money out.
    s.client.withdraw_escrow(&boss, &(30 * USDC));
    assert_eq!(s.client.escrow_of(&boss), 50 * USDC);
    assert_eq!(s.token.balance(&boss), 50 * USDC);

    s.client.unpause();
    s.client.fund(&boss, &(10 * USDC));
    assert_eq!(s.client.escrow_of(&boss), 60 * USDC);
}

// ---- admin model -----------------------------------------------------------------

#[test]
fn admin_only_functions_reject_other_signers() {
    let s = setup();
    let boss = employer(&s, 0);
    let stranger = Address::generate(&s.env);
    let fake_hash = soroban_sdk::BytesN::from_array(&s.env, &[0xAB; 32]);

    for signer in [&boss, &s.fee_recipient, &stranger] {
        mock_auth_for(&s, signer, "pause", ().into_val(&s.env));
        assert!(is_host_error(&s.client.try_pause()));
        mock_auth_for(&s, signer, "unpause", ().into_val(&s.env));
        assert!(is_host_error(&s.client.try_unpause()));
        mock_auth_for(&s, signer, "set_fee", (10_u32,).into_val(&s.env));
        assert!(matches!(s.client.try_set_fee(&10), Err(Err(_))));
        mock_auth_for(
            &s,
            signer,
            "set_fee_recipient",
            (stranger.clone(),).into_val(&s.env),
        );
        assert!(is_host_error(&s.client.try_set_fee_recipient(&stranger)));
        mock_auth_for(
            &s,
            signer,
            "propose_admin",
            (stranger.clone(),).into_val(&s.env),
        );
        assert!(is_host_error(&s.client.try_propose_admin(&stranger)));
        mock_auth_for(&s, signer, "upgrade", (fake_hash.clone(),).into_val(&s.env));
        assert!(is_host_error(&s.client.try_upgrade(&fake_hash)));
    }

    s.env.mock_all_auths();
    assert!(!s.client.is_paused());
    assert_eq!(s.client.fee_bps(), FEE_BPS);
    assert_eq!(s.client.fee_recipient(), s.fee_recipient);
    assert_eq!(s.client.pending_admin(), None);
    assert_eq!(s.client.admin(), s.admin);
}

#[test]
fn admin_two_step_transfer() {
    let s = setup();
    s.env.mock_all_auths();
    let new_admin = Address::generate(&s.env);

    assert_eq!(s.client.try_accept_admin(), Err(Ok(Error::NoPendingAdmin)));

    s.client.propose_admin(&new_admin);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &s.admin,
            "propose_admin",
            (new_admin.clone(),).into_val(&s.env)
        )
    );
    assert_eq!(s.client.pending_admin(), Some(new_admin.clone()));
    assert_eq!(s.client.admin(), s.admin);

    for signer in [&s.admin, &Address::generate(&s.env)] {
        mock_auth_for(&s, signer, "accept_admin", ().into_val(&s.env));
        assert!(matches!(s.client.try_accept_admin(), Err(Err(_))));
    }

    mock_auth_for(&s, &new_admin, "accept_admin", ().into_val(&s.env));
    s.client.accept_admin();
    assert_eq!(
        s.env.auths(),
        expected_auth(&s, &new_admin, "accept_admin", ().into_val(&s.env))
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
