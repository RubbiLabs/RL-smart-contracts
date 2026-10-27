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
    Address, ConversionError, Env, Event as _, IntoVal, InvokeError, String, Symbol,
};

use crate::{
    events, Error, PayrollContract, PayrollContractClient, Schedule, ScheduleInput, ScheduleStatus,
    MAX_BATCH, MAX_LABEL_LEN, MAX_PERIOD, MAX_SCHEDULES_PER_CALL, MIN_PERIOD,
};

/// Default fee used by the tests: 50 bps = 0.5%.
const FEE_BPS: u32 = 50;
/// One USDC in its smallest unit (7 decimals on Stellar).
const USDC: i128 = 10_000_000;
/// Ledger timestamp at which every test starts (2023-11-14T22:13:20Z).
const START: u64 = 1_700_000_000;
/// A 30-day schedule period, in seconds.
const MONTH: u64 = 30 * 86_400;

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

/// Move the ledger clock forward by `secs` seconds (and a proportional
/// number of ledgers at 5 s each).
fn advance(s: &Setup, secs: u64) {
    s.env.ledger().with_mut(|li| {
        li.timestamp += secs;
        li.sequence_number += (secs / 5) as u32;
    });
}

fn now(s: &Setup) -> u64 {
    s.env.ledger().timestamp()
}

fn label(s: &Setup, text: &str) -> String {
    String::from_str(&s.env, text)
}

fn input(s: &Setup, recipient: &Address, amount: i128, period: u64, text: &str) -> ScheduleInput {
    ScheduleInput {
        recipient: recipient.clone(),
        amount,
        period,
        label: label(s, text),
    }
}

/// An employer with `escrow` already funded and one monthly schedule of
/// `amount` to a fresh recipient, first due now. Returns
/// (employer, recipient, schedule id).
fn funded_schedule(s: &Setup, escrow: i128, amount: i128) -> (Address, Address, u64) {
    let boss = employer(s, escrow);
    if escrow > 0 {
        s.client.fund(&boss, &escrow);
    }
    let staff = Address::generate(&s.env);
    let ids = s.client.create_schedules(
        &boss,
        &soroban_sdk::vec![&s.env, input(s, &staff, amount, MONTH, "Design lead")],
        &now(s),
    );
    (boss, staff, ids.get(0).unwrap())
}

fn status_of(s: &Setup, id: u64) -> ScheduleStatus {
    s.client.get_schedule(&id).status
}

fn next_due_of(s: &Setup, id: u64) -> u64 {
    s.client.get_schedule(&id).next_due
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

// ---- create_schedules --------------------------------------------------------------

#[test]
fn create_schedules_requires_exact_employer_auth_and_stores_every_schedule() {
    let s = setup();
    let boss = employer(&s, 0);
    let a = Address::generate(&s.env);
    let b = Address::generate(&s.env);
    let first_due = now(&s) + 7 * 86_400;
    let inputs = soroban_sdk::vec![
        &s.env,
        input(&s, &a, 100 * USDC, MONTH, "Design lead"),
        input(&s, &b, 50 * USDC, 2 * 86_400, ""),
    ];

    let ids = s.client.create_schedules(&boss, &inputs, &first_due);
    assert_eq!(ids, soroban_sdk::vec![&s.env, 1, 2]);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &boss,
            "create_schedules",
            (boss.clone(), inputs.clone(), first_due).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [
            events::ScheduleCreated {
                employer: boss.clone(),
                schedule_id: 1,
                recipient: a.clone(),
                amount: 100 * USDC,
                period: MONTH,
                next_due: first_due,
            }
            .to_xdr(&s.env, &s.contract),
            events::ScheduleCreated {
                employer: boss.clone(),
                schedule_id: 2,
                recipient: b.clone(),
                amount: 50 * USDC,
                period: 2 * 86_400,
                next_due: first_due,
            }
            .to_xdr(&s.env, &s.contract),
        ]
    );
    // L10: every schedule is stored and retrievable by its id.
    assert_eq!(
        s.client.get_schedule(&1),
        Schedule {
            id: 1,
            employer: boss.clone(),
            recipient: a.clone(),
            amount: 100 * USDC,
            period: MONTH,
            next_due: first_due,
            status: ScheduleStatus::Active,
            label: label(&s, "Design lead"),
            created_at: START,
        }
    );
    assert_eq!(
        s.client.get_schedule(&2),
        Schedule {
            id: 2,
            employer: boss.clone(),
            recipient: b.clone(),
            amount: 50 * USDC,
            period: 2 * 86_400,
            next_due: first_due,
            status: ScheduleStatus::Active,
            label: label(&s, ""),
            created_at: START,
        }
    );
    assert_eq!(
        s.client.try_get_schedule(&3),
        Err(Ok(Error::ScheduleNotFound))
    );

    // Ids are global across employers; an empty input list creates nothing.
    let other = employer(&s, 0);
    assert_eq!(
        s.client.create_schedules(
            &other,
            &soroban_sdk::vec![&s.env, input(&s, &a, USDC, MONTH, "x")],
            &first_due
        ),
        soroban_sdk::vec![&s.env, 3]
    );
    assert_eq!(
        s.client
            .create_schedules(&other, &soroban_sdk::Vec::new(&s.env), &first_due),
        soroban_sdk::Vec::new(&s.env)
    );
    assert_eq!(s.client.get_schedule(&3).employer, other);
}

#[test]
fn create_schedules_by_another_signer_fails() {
    let s = setup();
    let boss = employer(&s, 0);
    let staff = Address::generate(&s.env);
    let inputs = soroban_sdk::vec![&s.env, input(&s, &staff, USDC, MONTH, "x")];
    let first_due = now(&s);

    for signer in [&s.admin, &staff, &Address::generate(&s.env)] {
        mock_auth_for(
            &s,
            signer,
            "create_schedules",
            (boss.clone(), inputs.clone(), first_due).into_val(&s.env),
        );
        assert!(matches!(
            s.client.try_create_schedules(&boss, &inputs, &first_due),
            Err(Err(_))
        ));
    }
    s.env.mock_all_auths();
    assert_eq!(
        s.client.try_get_schedule(&1),
        Err(Ok(Error::ScheduleNotFound))
    );
}

#[test]
fn create_schedules_is_atomic_on_invalid_input() {
    let s = setup();
    let boss = employer(&s, 0);
    let staff = Address::generate(&s.env);
    let ok = input(&s, &staff, USDC, MONTH, &"l".repeat(MAX_LABEL_LEN as usize));
    let first_due = now(&s);

    let bad_inputs = [
        (input(&s, &staff, 0, MONTH, "zero"), Error::InvalidAmount),
        (
            input(&s, &staff, -1, MONTH, "negative"),
            Error::InvalidAmount,
        ),
        (
            input(&s, &staff, USDC, MIN_PERIOD - 1, "short"),
            Error::InvalidPeriod,
        ),
        (
            input(&s, &staff, USDC, MAX_PERIOD + 1, "long"),
            Error::InvalidPeriod,
        ),
        (
            input(&s, &staff, USDC, 0, "zero period"),
            Error::InvalidPeriod,
        ),
        (
            input(
                &s,
                &staff,
                USDC,
                MONTH,
                &"l".repeat(MAX_LABEL_LEN as usize + 1),
            ),
            Error::LabelTooLong,
        ),
    ];
    for (bad, err) in bad_inputs {
        // Valid inputs before and after the bad one: none is created.
        let inputs = soroban_sdk::vec![&s.env, ok.clone(), ok.clone(), bad, ok.clone()];
        assert_eq!(
            s.client.try_create_schedules(&boss, &inputs, &first_due),
            Err(Ok(err))
        );
        assert_eq!(
            s.client.try_get_schedule(&1),
            Err(Ok(Error::ScheduleNotFound))
        );
    }

    // first_due in the past, too many inputs, paused.
    assert_eq!(
        s.client.try_create_schedules(
            &boss,
            &soroban_sdk::vec![&s.env, ok.clone()],
            &(first_due - 1)
        ),
        Err(Ok(Error::FirstDueInPast))
    );
    let mut too_many = soroban_sdk::Vec::new(&s.env);
    for _ in 0..=MAX_SCHEDULES_PER_CALL {
        too_many.push_back(ok.clone());
    }
    assert_eq!(
        s.client.try_create_schedules(&boss, &too_many, &first_due),
        Err(Ok(Error::TooManySchedules))
    );
    s.client.pause();
    assert_eq!(
        s.client
            .try_create_schedules(&boss, &soroban_sdk::vec![&s.env, ok.clone()], &first_due),
        Err(Ok(Error::Paused))
    );
    s.client.unpause();
    assert_eq!(
        s.client.try_get_schedule(&1),
        Err(Ok(Error::ScheduleNotFound))
    );

    // Exactly the maximum is fine, and the rejected calls consumed no ids.
    too_many.pop_back();
    let ids = s.client.create_schedules(&boss, &too_many, &first_due);
    // The host enforces the network's per-transaction limits in tests, so
    // this call also proves that 50 schedules (with maximum-length labels)
    // fit in one transaction: 50 `schedule_created` events use ~15.6 KiB of
    // the 16 KiB contract-events budget.
    assert_eq!(ids.len(), MAX_SCHEDULES_PER_CALL);
    assert_eq!(ids.get(0), Some(1));
    assert_eq!(ids.last(), Some(u64::from(MAX_SCHEDULES_PER_CALL)));
}

// ---- disburse: time gate (L3) and back pay -------------------------------------------

#[test]
fn disburse_before_due_fails_and_after_due_pays_exactly_one_period() {
    let s = setup();
    let boss = employer(&s, 1_000 * USDC);
    s.client.fund(&boss, &(1_000 * USDC));
    let staff = Address::generate(&s.env);
    let first_due = now(&s) + 86_400;
    let id = s
        .client
        .create_schedules(
            &boss,
            &soroban_sdk::vec![&s.env, input(&s, &staff, 100 * USDC, MONTH, "Dev")],
            &first_due,
        )
        .get(0)
        .unwrap();
    let fee = 100 * USDC * FEE_BPS as i128 / 10_000;

    // L3: not due yet, even one second before.
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotDue)));
    advance(&s, 86_400 - 1);
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotDue)));
    assert_eq!(s.token.balance(&staff), 0);

    // Due: exactly one period is paid and next_due moves by one period.
    advance(&s, 1);
    assert!(s.client.disburse(&id));
    assert_eq!(status_of(&s, id), ScheduleStatus::Active);
    assert_eq!(s.token.balance(&staff), 100 * USDC - fee);
    assert_eq!(s.token.balance(&s.fee_recipient), fee);
    assert_eq!(s.client.escrow_of(&boss), 900 * USDC);
    assert_eq!(s.client.total_escrow(), 900 * USDC);
    assert_eq!(s.token.balance(&s.contract), 900 * USDC);
    assert_eq!(next_due_of(&s, id), first_due + MONTH);
    // L3: the state was updated, so calling again is rejected.
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotDue)));
    assert_eq!(s.token.balance(&staff), 100 * USDC - fee);

    // `disburse` is permissionless: no authorization is required at all.
    advance(&s, MONTH);
    s.env.mock_auths(&[]);
    assert!(s.client.disburse(&id));
    assert_eq!(s.env.auths(), std::vec![]);
    assert_eq!(s.token.balance(&staff), 2 * (100 * USDC - fee));

    assert_eq!(s.client.try_disburse(&99), Err(Ok(Error::ScheduleNotFound)));
}

#[test]
fn back_pay_rule_n_missed_periods_produce_exactly_n_payments() {
    let s = setup();
    let (boss, staff, id) = funded_schedule(&s, 1_000 * USDC, 100 * USDC);
    let fee = 100 * USDC * FEE_BPS as i128 / 10_000;

    // The keeper was down for 4 periods (first due was at START).
    advance(&s, 4 * MONTH - 1);
    assert_eq!(next_due_of(&s, id), START);
    for n in 1..=4_u64 {
        assert!(s.client.disburse(&id), "payment {n}");
        assert_eq!(s.token.balance(&staff), n as i128 * (100 * USDC - fee));
        assert_eq!(next_due_of(&s, id), START + n * MONTH);
    }
    // The 5th call fails: the 5th period is not due for another second.
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotDue)));
    assert_eq!(s.token.balance(&staff), 4 * (100 * USDC - fee));
    assert_eq!(s.client.escrow_of(&boss), 600 * USDC);
    assert_eq!(s.token.balance(&s.fee_recipient), 4 * fee);

    advance(&s, 1);
    assert!(s.client.disburse(&id));
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotDue)));
    // A batch catches up the same way: one period per item per call.
    advance(&s, 2 * MONTH);
    assert_eq!(
        s.client
            .disburse_batch(&soroban_sdk::vec![&s.env, id, id, id]),
        soroban_sdk::vec![&s.env, true, true, false]
    );
    assert_eq!(s.token.balance(&staff), 7 * (100 * USDC - fee));
}

#[test]
fn insufficient_escrow_returns_false_and_changes_nothing() {
    let s = setup();
    let (boss, staff, id) = funded_schedule(&s, 150 * USDC, 100 * USDC);
    let fee = 100 * USDC * FEE_BPS as i128 / 10_000;

    // First period: fine. Second: 50 USDC left, 100 needed.
    assert!(s.client.disburse(&id));
    advance(&s, MONTH);
    assert!(!s.client.disburse(&id));
    assert_eq!(
        last_events(&s),
        [events::DisburseFailed {
            employer: boss.clone(),
            schedule_id: id,
            amount: 100 * USDC,
            escrow: 50 * USDC,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    let schedule = s.client.get_schedule(&id);
    assert_eq!(schedule.status, ScheduleStatus::Active);
    assert_eq!(schedule.next_due, START + MONTH, "the period stays owed");
    assert_eq!(s.client.escrow_of(&boss), 50 * USDC);
    assert_eq!(s.client.total_escrow(), 50 * USDC);
    assert_eq!(s.token.balance(&staff), 100 * USDC - fee);
    assert_eq!(s.token.balance(&s.fee_recipient), fee);
    assert_eq!(s.token.balance(&s.contract), 50 * USDC);

    // Retrying without funding fails the same way, idempotently.
    assert!(!s.client.disburse(&id));
    assert_eq!(next_due_of(&s, id), START + MONTH);

    // Funding the escrow lets the owed period be paid.
    s.token_admin.mint(&boss, &(50 * USDC));
    s.client.fund(&boss, &(50 * USDC));
    assert!(s.client.disburse(&id));
    assert_eq!(s.client.escrow_of(&boss), 0);
    assert_eq!(s.token.balance(&staff), 2 * (100 * USDC - fee));
    assert_eq!(next_due_of(&s, id), START + 2 * MONTH);

    // An employer with no escrow at all: same outcome.
    let (_, _, unfunded) = funded_schedule(&s, 0, USDC);
    assert!(!s.client.disburse(&unfunded));
}

#[test]
fn fee_split_is_exact_including_rounding() {
    let s = setup();
    // 50 bps of 1.2345679 USDC = 61_728.395 stroops → 61_728 (rounded down).
    let amount = 12_345_679_i128;
    let (boss, staff, id) = funded_schedule(&s, 10 * USDC, amount);
    assert!(s.client.disburse(&id));
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728);
    assert_eq!(s.token.balance(&staff), amount - 61_728);
    assert_eq!(s.client.escrow_of(&boss), 10 * USDC - amount);
    assert_eq!(s.token.balance(&s.contract), 10 * USDC - amount);

    // Too small to produce a fee: the recipient gets everything.
    let (_, tiny_staff, tiny) = funded_schedule(&s, USDC, 199);
    assert!(s.client.disburse(&tiny));
    assert_eq!(s.token.balance(&tiny_staff), 199);
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728);

    // Fee changes apply to the next disbursement; the cap is honoured.
    s.client.set_fee(&crate::MAX_FEE_BPS);
    advance(&s, MONTH);
    assert!(s.client.disburse(&id));
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728 + 1_234_567);
    assert_eq!(s.token.balance(&staff), 2 * amount - 61_728 - 1_234_567);

    // Zero fee: nothing goes to the fee recipient.
    s.client.set_fee(&0);
    advance(&s, MONTH);
    assert!(s.client.disburse(&id));
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728 + 1_234_567);
    assert_eq!(s.token.balance(&staff), 3 * amount - 61_728 - 1_234_567);
    // Escrow invariant: contract balance == total escrow.
    assert_eq!(s.token.balance(&s.contract), s.client.total_escrow());
}

// ---- disburse_batch ------------------------------------------------------------------

#[test]
fn disburse_batch_enforces_the_limit_and_the_pause() {
    let s = setup();
    let (_boss, staff, id) = funded_schedule(&s, 1_000 * USDC, USDC);

    let mut too_many = soroban_sdk::Vec::new(&s.env);
    for _ in 0..=MAX_BATCH {
        too_many.push_back(id);
    }
    assert_eq!(
        s.client.try_disburse_batch(&too_many),
        Err(Ok(Error::BatchTooLarge))
    );
    assert_eq!(s.token.balance(&staff), 0);

    // Exactly MAX_BATCH ids is fine; the repeated id is paid once (the
    // schedule is due once) and then reported as not due.
    too_many.pop_back();
    let results = s.client.disburse_batch(&too_many);
    assert_eq!(results.len(), MAX_BATCH);
    assert!(results.get(0).unwrap());
    assert!(!results.get(1).unwrap());
    assert_eq!(
        s.token.balance(&staff),
        USDC - USDC * FEE_BPS as i128 / 10_000
    );

    s.client.pause();
    advance(&s, MONTH);
    assert_eq!(
        s.client.try_disburse_batch(&soroban_sdk::vec![&s.env, id]),
        Err(Ok(Error::Paused))
    );
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::Paused)));
    s.client.unpause();
    assert_eq!(
        s.client.disburse_batch(&soroban_sdk::vec![&s.env, id]),
        soroban_sdk::vec![&s.env, true]
    );
    assert_eq!(
        s.client.disburse_batch(&soroban_sdk::Vec::new(&s.env)),
        soroban_sdk::Vec::new(&s.env)
    );
}

#[test]
fn disburse_batch_returns_false_for_skipped_items_instead_of_failing() {
    let s = setup();
    let (boss_a, staff_a, due) = funded_schedule(&s, 1_000 * USDC, 100 * USDC);
    let (_boss_b, staff_b, unfunded) = funded_schedule(&s, 10 * USDC, 100 * USDC);
    let staff_c = Address::generate(&s.env);
    let not_due = s
        .client
        .create_schedules(
            &boss_a,
            &soroban_sdk::vec![&s.env, input(&s, &staff_c, 100 * USDC, MONTH, "later")],
            &(now(&s) + 1),
        )
        .get(0)
        .unwrap();
    let fee = 100 * USDC * FEE_BPS as i128 / 10_000;

    let results = s.client.disburse_batch(&soroban_sdk::vec![
        &s.env, due, unfunded, not_due, 999 // unknown id
    ]);
    assert_eq!(
        results,
        soroban_sdk::vec![&s.env, true, false, false, false]
    );
    assert_eq!(s.token.balance(&staff_a), 100 * USDC - fee);
    assert_eq!(s.token.balance(&staff_b), 0);
    assert_eq!(next_due_of(&s, unfunded), START);
    assert_eq!(s.token.balance(&staff_c), 0);
    assert_eq!(next_due_of(&s, not_due), START + 1);
    assert_eq!(s.client.escrow_of(&boss_a), 900 * USDC);
    assert_eq!(s.token.balance(&s.contract), s.client.total_escrow());

    // One second later the third item is due too.
    advance(&s, 1);
    assert_eq!(
        s.client
            .disburse_batch(&soroban_sdk::vec![&s.env, due, unfunded, not_due]),
        soroban_sdk::vec![&s.env, false, false, true]
    );
    assert_eq!(s.token.balance(&staff_c), 100 * USDC - fee);
}

// ---- employer pause / resume / cancel (L4) ---------------------------------------------

#[test]
fn only_the_employer_can_pause_resume_or_cancel() {
    let s = setup();
    let (boss, staff, id) = funded_schedule(&s, 1_000 * USDC, 100 * USDC);
    let (other_boss, _, other_id) = funded_schedule(&s, 1_000 * USDC, 100 * USDC);

    // Another employer signing for their own address but naming the
    // victim's schedule id: rejected by the ownership check.
    assert_eq!(
        s.client.try_pause_schedule(&other_boss, &id),
        Err(Ok(Error::NotEmployer))
    );
    assert_eq!(
        s.client.try_resume_schedule(&other_boss, &id),
        Err(Ok(Error::NotEmployer))
    );
    assert_eq!(
        s.client.try_cancel_schedule(&other_boss, &id),
        Err(Ok(Error::NotEmployer))
    );
    // L4: the recipient, the admin or a stranger signing on the employer's
    // behalf: rejected by `require_auth`.
    for signer in [&staff, &s.admin, &Address::generate(&s.env)] {
        mock_auth_for(
            &s,
            signer,
            "pause_schedule",
            (boss.clone(), id).into_val(&s.env),
        );
        assert!(matches!(
            s.client.try_pause_schedule(&boss, &id),
            Err(Err(_))
        ));
        mock_auth_for(
            &s,
            signer,
            "resume_schedule",
            (boss.clone(), id).into_val(&s.env),
        );
        assert!(matches!(
            s.client.try_resume_schedule(&boss, &id),
            Err(Err(_))
        ));
        mock_auth_for(
            &s,
            signer,
            "cancel_schedule",
            (boss.clone(), id).into_val(&s.env),
        );
        assert!(matches!(
            s.client.try_cancel_schedule(&boss, &id),
            Err(Err(_))
        ));
    }
    s.env.mock_all_auths();
    assert_eq!(status_of(&s, id), ScheduleStatus::Active);
    assert_eq!(status_of(&s, other_id), ScheduleStatus::Active);
    assert_eq!(
        s.client.try_pause_schedule(&boss, &99),
        Err(Ok(Error::ScheduleNotFound))
    );

    // The owner succeeds, with exactly their own authorization.
    s.client.pause_schedule(&boss, &id);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &boss,
            "pause_schedule",
            (boss.clone(), id).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::SchedulePaused {
            employer: boss.clone(),
            schedule_id: id,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(status_of(&s, id), ScheduleStatus::Paused);
    assert_eq!(status_of(&s, other_id), ScheduleStatus::Active);
}

#[test]
fn paused_schedule_is_not_disbursed_and_resume_keeps_next_due() {
    let s = setup();
    let (boss, staff, id) = funded_schedule(&s, 1_000 * USDC, 100 * USDC);
    let fee = 100 * USDC * FEE_BPS as i128 / 10_000;
    assert!(s.client.disburse(&id));

    s.client.pause_schedule(&boss, &id);
    advance(&s, 3 * MONTH);
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotActive)));
    assert_eq!(
        s.client.disburse_batch(&soroban_sdk::vec![&s.env, id]),
        soroban_sdk::vec![&s.env, false]
    );
    assert_eq!(s.token.balance(&staff), 100 * USDC - fee);

    // Resume keeps next_due: the three periods that fell due while paused
    // are still owed and are caught up one per call.
    s.client.resume_schedule(&boss, &id);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &boss,
            "resume_schedule",
            (boss.clone(), id).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::ScheduleResumed {
            employer: boss.clone(),
            schedule_id: id,
            next_due: START + MONTH,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(next_due_of(&s, id), START + MONTH);
    for n in 2..=4_i128 {
        assert!(s.client.disburse(&id));
        assert_eq!(s.token.balance(&staff), n * (100 * USDC - fee));
    }
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotDue)));
}

#[test]
fn status_transitions_follow_the_state_machine() {
    let s = setup();
    let (boss, _staff, id) = funded_schedule(&s, 1_000 * USDC, 100 * USDC);

    // Active: resume is invalid.
    assert_eq!(
        s.client.try_resume_schedule(&boss, &id),
        Err(Ok(Error::NotPaused))
    );
    s.client.pause_schedule(&boss, &id);
    // Paused: pause again is invalid; resume is valid.
    assert_eq!(
        s.client.try_pause_schedule(&boss, &id),
        Err(Ok(Error::NotActive))
    );
    s.client.resume_schedule(&boss, &id);
    assert_eq!(status_of(&s, id), ScheduleStatus::Active);

    // Cancel from Active; then nothing else is possible.
    s.client.cancel_schedule(&boss, &id);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &boss,
            "cancel_schedule",
            (boss.clone(), id).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::ScheduleCancelled {
            employer: boss.clone(),
            schedule_id: id,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(status_of(&s, id), ScheduleStatus::Cancelled);
    assert_eq!(
        s.client.try_cancel_schedule(&boss, &id),
        Err(Ok(Error::AlreadyCancelled))
    );
    assert_eq!(
        s.client.try_pause_schedule(&boss, &id),
        Err(Ok(Error::NotActive))
    );
    assert_eq!(
        s.client.try_resume_schedule(&boss, &id),
        Err(Ok(Error::NotPaused))
    );
    // A cancelled schedule cannot be disbursed, even when long overdue.
    advance(&s, 3 * MONTH);
    assert_eq!(s.client.try_disburse(&id), Err(Ok(Error::NotActive)));
    assert_eq!(
        s.client.disburse_batch(&soroban_sdk::vec![&s.env, id]),
        soroban_sdk::vec![&s.env, false]
    );
    // The escrow is untouched by the cancellation and can be withdrawn.
    assert_eq!(s.client.escrow_of(&boss), 1_000 * USDC);
    s.client.withdraw_escrow(&boss, &(1_000 * USDC));

    // Cancel from Paused is allowed too.
    let (boss2, _, id2) = funded_schedule(&s, 0, USDC);
    s.client.pause_schedule(&boss2, &id2);
    s.client.cancel_schedule(&boss2, &id2);
    assert_eq!(status_of(&s, id2), ScheduleStatus::Cancelled);
}

#[test]
fn employers_keep_control_while_the_contract_is_paused() {
    let s = setup();
    let (boss, _staff, a) = funded_schedule(&s, 100 * USDC, USDC);
    let staff_b = Address::generate(&s.env);
    let b = s
        .client
        .create_schedules(
            &boss,
            &soroban_sdk::vec![&s.env, input(&s, &staff_b, USDC, MONTH, "b")],
            &now(&s),
        )
        .get(0)
        .unwrap();
    s.client.pause_schedule(&boss, &b);

    s.client.pause();
    s.client.pause_schedule(&boss, &a);
    s.client.resume_schedule(&boss, &a);
    s.client.cancel_schedule(&boss, &a);
    s.client.resume_schedule(&boss, &b);
    s.client.cancel_schedule(&boss, &b);
    s.client.withdraw_escrow(&boss, &(100 * USDC));
    assert_eq!(status_of(&s, a), ScheduleStatus::Cancelled);
    assert_eq!(status_of(&s, b), ScheduleStatus::Cancelled);
    assert_eq!(s.client.escrow_of(&boss), 0);
    s.client.unpause();
}
