#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{
        Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, MockAuth,
        MockAuthInvoke,
    },
    token::{StellarAssetClient, TokenClient},
    xdr::ScErrorType,
    Address, ConversionError, Env, Event as _, IntoVal, InvokeError, String, Symbol,
};

use crate::{
    events, Error, Merchant, Plan, SubscriptionsContract, SubscriptionsContractClient,
    MAX_NAME_LEN, MAX_PERIOD, MIN_PERIOD,
};

/// Default fee used by the tests: 50 bps = 0.5%.
const FEE_BPS: u32 = 50;
/// One USDC in its smallest unit (7 decimals on Stellar).
const USDC: i128 = 10_000_000;
/// A 30-day plan period, in seconds.
const MONTH: u64 = 30 * 86_400;

/// Everything a test needs: the env, the roles, the token and the contract.
struct Setup {
    env: Env,
    admin: Address,
    fee_recipient: Address,
    token: TokenClient<'static>,
    token_admin: StellarAssetClient<'static>,
    contract: Address,
    client: SubscriptionsContractClient<'static>,
}

fn setup() -> Setup {
    let env = Env::default();
    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let token_issuer = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(token_issuer);
    let token = TokenClient::new(&env, &sac.address());
    let token_admin = StellarAssetClient::new(&env, &sac.address());

    let contract = env.register(
        SubscriptionsContract,
        (&admin, &sac.address(), &fee_recipient, FEE_BPS),
    );
    let client = SubscriptionsContractClient::new(&env, &contract);

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

fn name(s: &Setup, text: &str) -> String {
    String::from_str(&s.env, text)
}

/// An allowlisted, active merchant with a payout address (all auths mocked).
fn new_merchant(s: &Setup) -> (Address, Address) {
    let merchant = Address::generate(&s.env);
    let payout = Address::generate(&s.env);
    s.env.mock_all_auths();
    s.client.add_merchant(&merchant, &payout);
    (merchant, payout)
}

/// A merchant with one active plan: `amount` per `MONTH`.
fn merchant_with_plan(s: &Setup, amount: i128) -> (Address, Address, u32) {
    let (merchant, payout) = new_merchant(s);
    let plan_id = s
        .client
        .create_plan(&merchant, &name(s, "Basic"), &amount, &MONTH);
    (merchant, payout, plan_id)
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
    let _ = &s.token_admin;
}

#[test]
#[should_panic]
fn constructor_rejects_fee_over_cap() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    env.register(
        SubscriptionsContract,
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

// ---- merchants ---------------------------------------------------------------

#[test]
fn add_merchant_requires_exact_admin_auth_and_stores_record() {
    let s = setup();
    let merchant = Address::generate(&s.env);
    let payout = Address::generate(&s.env);
    assert_eq!(s.client.get_merchant(&merchant), None);

    s.env.mock_all_auths();
    s.client.add_merchant(&merchant, &payout);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &s.admin,
            "add_merchant",
            (merchant.clone(), payout.clone()).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::MerchantAdded {
            merchant: merchant.clone(),
            payout: payout.clone(),
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(
        s.client.get_merchant(&merchant),
        Some(Merchant {
            payout: payout.clone(),
            active: true
        })
    );

    // Adding the same merchant twice is rejected and changes nothing.
    assert_eq!(
        s.client
            .try_add_merchant(&merchant, &Address::generate(&s.env)),
        Err(Ok(Error::MerchantExists))
    );
    assert_eq!(s.client.get_merchant(&merchant).unwrap().payout, payout);
}

#[test]
fn merchant_functions_by_non_admin_fail() {
    let s = setup();
    let (merchant, payout) = new_merchant(&s);
    let other = Address::generate(&s.env);

    for signer in [&merchant, &other, &s.fee_recipient] {
        mock_auth_for(
            &s,
            signer,
            "add_merchant",
            (other.clone(), payout.clone()).into_val(&s.env),
        );
        assert!(matches!(
            s.client.try_add_merchant(&other, &payout),
            Err(Err(_))
        ));
        mock_auth_for(
            &s,
            signer,
            "set_merchant_active",
            (merchant.clone(), false).into_val(&s.env),
        );
        assert!(matches!(
            s.client.try_set_merchant_active(&merchant, &false),
            Err(Err(_))
        ));
    }
    s.env.mock_all_auths();
    assert_eq!(s.client.get_merchant(&other), None);
    assert!(s.client.get_merchant(&merchant).unwrap().active);
}

#[test]
fn set_merchant_active_toggles_and_emits() {
    let s = setup();
    let (merchant, payout) = new_merchant(&s);

    s.client.set_merchant_active(&merchant, &false);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &s.admin,
            "set_merchant_active",
            (merchant.clone(), false).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::MerchantActiveSet {
            merchant: merchant.clone(),
            active: false,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(
        s.client.get_merchant(&merchant),
        Some(Merchant {
            payout: payout.clone(),
            active: false
        })
    );
    s.client.set_merchant_active(&merchant, &true);
    assert!(s.client.get_merchant(&merchant).unwrap().active);

    assert_eq!(
        s.client
            .try_set_merchant_active(&Address::generate(&s.env), &true),
        Err(Ok(Error::MerchantNotFound))
    );
}

// ---- plans ---------------------------------------------------------------------

#[test]
fn create_plan_requires_exact_merchant_auth_and_assigns_sequential_ids() {
    let s = setup();
    let (merchant, _payout) = new_merchant(&s);
    let plan_name = name(&s, "Basic");

    let id = s
        .client
        .create_plan(&merchant, &plan_name, &(5 * USDC), &MONTH);
    assert_eq!(id, 1);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &merchant,
            "create_plan",
            (merchant.clone(), plan_name.clone(), 5 * USDC, MONTH).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::PlanCreated {
            merchant: merchant.clone(),
            plan_id: 1,
            name: plan_name.clone(),
            amount: 5 * USDC,
            period: MONTH,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(
        s.client.get_plan(&1),
        Plan {
            id: 1,
            merchant: merchant.clone(),
            name: plan_name,
            amount: 5 * USDC,
            period: MONTH,
            active: true,
        }
    );

    // Ids are global and sequential across merchants.
    let (other, _) = new_merchant(&s);
    assert_eq!(
        s.client
            .create_plan(&other, &name(&s, "Pro"), &USDC, &MIN_PERIOD),
        2
    );
    assert_eq!(
        s.client
            .create_plan(&merchant, &name(&s, "Yearly"), &USDC, &MAX_PERIOD),
        3
    );
    assert_eq!(s.client.get_plan(&2).merchant, other);
    assert_eq!(s.client.try_get_plan(&4), Err(Ok(Error::PlanNotFound)));
}

#[test]
fn create_plan_by_another_signer_fails() {
    let s = setup();
    let (merchant, _payout) = new_merchant(&s);
    let plan_name = name(&s, "Basic");

    for signer in [&s.admin, &Address::generate(&s.env)] {
        mock_auth_for(
            &s,
            signer,
            "create_plan",
            (merchant.clone(), plan_name.clone(), USDC, MONTH).into_val(&s.env),
        );
        assert!(matches!(
            s.client
                .try_create_plan(&merchant, &plan_name, &USDC, &MONTH),
            Err(Err(_))
        ));
    }
    s.env.mock_all_auths();
    assert_eq!(s.client.try_get_plan(&1), Err(Ok(Error::PlanNotFound)));
}

#[test]
fn inactive_or_unknown_merchant_cannot_create_plans() {
    let s = setup();
    let (merchant, _payout) = new_merchant(&s);
    let plan_name = name(&s, "Basic");

    // Unknown merchant.
    assert_eq!(
        s.client
            .try_create_plan(&Address::generate(&s.env), &plan_name, &USDC, &MONTH),
        Err(Ok(Error::MerchantNotFound))
    );
    // Deactivated merchant.
    s.client.set_merchant_active(&merchant, &false);
    assert_eq!(
        s.client
            .try_create_plan(&merchant, &plan_name, &USDC, &MONTH),
        Err(Ok(Error::MerchantInactive))
    );
    // Reactivated: works again.
    s.client.set_merchant_active(&merchant, &true);
    assert_eq!(
        s.client.create_plan(&merchant, &plan_name, &USDC, &MONTH),
        1
    );
}

#[test]
fn create_plan_validates_name_amount_and_period() {
    let s = setup();
    let (merchant, _payout) = new_merchant(&s);

    let ok_name = name(&s, &"a".repeat(MAX_NAME_LEN as usize));
    let long_name = name(&s, &"a".repeat(MAX_NAME_LEN as usize + 1));
    assert_eq!(
        s.client
            .try_create_plan(&merchant, &long_name, &USDC, &MONTH),
        Err(Ok(Error::NameTooLong))
    );
    for amount in [0_i128, -1, i128::MIN] {
        assert_eq!(
            s.client
                .try_create_plan(&merchant, &ok_name, &amount, &MONTH),
            Err(Ok(Error::InvalidAmount))
        );
    }
    for period in [0_u64, MIN_PERIOD - 1, MAX_PERIOD + 1, u64::MAX] {
        assert_eq!(
            s.client
                .try_create_plan(&merchant, &ok_name, &USDC, &period),
            Err(Ok(Error::InvalidPeriod))
        );
    }
    // Rejected calls did not consume an id.
    assert_eq!(s.client.create_plan(&merchant, &ok_name, &USDC, &MONTH), 1);
}

#[test]
fn set_plan_active_is_owner_only_and_emits() {
    let s = setup();
    let (merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let (other, _) = new_merchant(&s);

    // Another merchant (even an allowlisted one) does not own the plan.
    assert_eq!(
        s.client.try_set_plan_active(&other, &plan_id, &false),
        Err(Ok(Error::NotPlanOwner))
    );
    assert_eq!(
        s.client.try_set_plan_active(&merchant, &99, &false),
        Err(Ok(Error::PlanNotFound))
    );
    // Admin cannot sign on the merchant's behalf.
    mock_auth_for(
        &s,
        &s.admin,
        "set_plan_active",
        (merchant.clone(), plan_id, false).into_val(&s.env),
    );
    assert!(matches!(
        s.client.try_set_plan_active(&merchant, &plan_id, &false),
        Err(Err(_))
    ));
    s.env.mock_all_auths();
    assert!(s.client.get_plan(&plan_id).active);

    s.client.set_plan_active(&merchant, &plan_id, &false);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &merchant,
            "set_plan_active",
            (merchant.clone(), plan_id, false).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::PlanActiveSet {
            merchant: merchant.clone(),
            plan_id,
            active: false,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert!(!s.client.get_plan(&plan_id).active);

    // A deactivated merchant can still deactivate / reactivate its plans.
    s.client.set_merchant_active(&merchant, &false);
    s.client.set_plan_active(&merchant, &plan_id, &true);
    assert!(s.client.get_plan(&plan_id).active);
}

// ---- admin model -----------------------------------------------------------------

#[test]
fn admin_only_functions_reject_other_signers() {
    let s = setup();
    let (merchant, _payout) = new_merchant(&s);
    let stranger = Address::generate(&s.env);
    let fake_hash = soroban_sdk::BytesN::from_array(&s.env, &[0xAB; 32]);

    for signer in [&merchant, &s.fee_recipient, &stranger] {
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
