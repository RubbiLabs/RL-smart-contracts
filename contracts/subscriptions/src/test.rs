#![cfg(test)]

extern crate std;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error,
    testutils::{
        Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, Ledger as _, MockAuth,
        MockAuthInvoke,
    },
    token::{StellarAssetClient, TokenClient},
    xdr::ScErrorType,
    Address, ConversionError, Env, Event as _, IntoVal, InvokeError, String, Symbol,
};

use crate::{
    events, Error, Merchant, Plan, SubStatus, Subscription, SubscriptionsContract,
    SubscriptionsContractClient, MAX_BATCH, MAX_NAME_LEN, MAX_PERIOD, MIN_PERIOD,
};

/// Default fee used by the tests: 50 bps = 0.5%.
const FEE_BPS: u32 = 50;
/// One USDC in its smallest unit (7 decimals on Stellar).
const USDC: i128 = 10_000_000;
/// A 30-day plan period, in seconds.
const MONTH: u64 = 30 * 86_400;
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
    client: SubscriptionsContractClient<'static>,
}

/// The default setup: the token is the SDK's test Stellar Asset Contract.
fn setup() -> Setup {
    let env = new_env();
    let token_issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(token_issuer);
    build(env, sac.address())
}

/// Like `setup`, but the token is [`MockToken`], which can be told to reject
/// one address the way the real SAC rejects an account without a USDC
/// trustline. Returns the mock's client as well.
fn setup_with_mock_token() -> (Setup, MockTokenClient<'static>) {
    let env = new_env();
    let mock = env.register(MockToken, ());
    let mock_client = MockTokenClient::new(&env, &mock);
    (build(env, mock), mock_client)
}

fn new_env() -> Env {
    let env = Env::default();
    env.ledger().with_mut(|li| {
        li.timestamp = START;
        li.sequence_number = 1_000;
    });
    env
}

fn build(env: Env, token_address: Address) -> Setup {
    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let token = TokenClient::new(&env, &token_address);
    let token_admin = StellarAssetClient::new(&env, &token_address);

    let contract = env.register(
        SubscriptionsContract,
        (&admin, &token_address, &fee_recipient, FEE_BPS),
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

// ---- mock token ----------------------------------------------------------------
//
// The SDK's test Stellar Asset Contract only ever sees contract addresses
// (`testutils::Address::generate` never creates G-accounts), so it cannot
// model a recipient without a USDC trustline. `MockToken` implements the
// token functions the subscriptions contract uses and rejects transfers to, and
// reads of, one configurable "unpayable" address with a contract error, the
// way the real SAC fails for an account without a trustline
// (`Error(Contract, #13)`, "trustline entry is missing for account"). The
// mock uses code 100 so it cannot be mistaken for a subscriptions error code.

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MockTokenError {
    /// Stands in for the SAC's `TrustlineMissingError`.
    Unpayable = 100,
    InsufficientBalance = 101,
    InsufficientAllowance = 102,
}

#[contracttype]
#[derive(Clone)]
pub enum MockKey {
    Balance(Address),
    Allowance(Address, Address),
    Unpayable,
    RejectTransfer(Address),
}

#[contract]
pub struct MockToken;

#[contractimpl]
impl MockToken {
    /// Make `addr` unable to hold the token. Pass a fresh address to make
    /// everyone payable again.
    pub fn set_unpayable(env: Env, addr: Address) {
        env.storage().instance().set(&MockKey::Unpayable, &addr);
    }

    /// Make `addr` reject incoming transfers (simulating e.g. trustline limit
    /// exceeded where authorized is true, but transfer fails).
    pub fn set_reject_transfer(env: Env, addr: Address) {
        env.storage().instance().set(&MockKey::RejectTransfer(addr), &true);
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        require_payable(&env, &to);
        set_balance(&env, &to, balance_of(&env, &to) + amount);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        require_payable(&env, &id);
        balance_of(&env, &id)
    }

    pub fn authorized(env: Env, id: Address) -> bool {
        require_payable(&env, &id);
        true
    }

    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, _live_until: u32) {
        from.require_auth();
        env.storage()
            .instance()
            .set(&MockKey::Allowance(from, spender), &amount);
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .instance()
            .get(&MockKey::Allowance(from, spender))
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        move_tokens(&env, &from, &to, amount);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        let allowance = Self::allowance(env.clone(), from.clone(), spender.clone());
        if allowance < amount {
            panic_with_error!(&env, MockTokenError::InsufficientAllowance);
        }
        env.storage().instance().set(
            &MockKey::Allowance(from.clone(), spender),
            &(allowance - amount),
        );
        move_tokens(&env, &from, &to, amount);
    }
}

fn require_payable(env: &Env, addr: &Address) {
    let unpayable: Option<Address> = env.storage().instance().get(&MockKey::Unpayable);
    if unpayable.as_ref() == Some(addr) {
        panic_with_error!(env, MockTokenError::Unpayable);
    }
}

fn balance_of(env: &Env, addr: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&MockKey::Balance(addr.clone()))
        .unwrap_or(0)
}

fn set_balance(env: &Env, addr: &Address, amount: i128) {
    env.storage()
        .instance()
        .set(&MockKey::Balance(addr.clone()), &amount);
}

fn move_tokens(env: &Env, from: &Address, to: &Address, amount: i128) {
    require_payable(env, to);
    if env
        .storage()
        .instance()
        .get::<_, bool>(&MockKey::RejectTransfer(to.clone()))
        .unwrap_or(false)
    {
        panic_with_error!(env, MockTokenError::Unpayable);
    }
    let from_balance = balance_of(env, from);
    if from_balance < amount {
        panic_with_error!(env, MockTokenError::InsufficientBalance);
    }
    set_balance(env, from, from_balance - amount);
    set_balance(env, to, balance_of(env, to) + amount);
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

/// A wallet holding `balance` tokens that has approved this contract to
/// spend `allowance` until ledger `live_until` (all auths mocked).
fn subscriber_with(s: &Setup, balance: i128, allowance: i128, live_until: u32) -> Address {
    let user = Address::generate(&s.env);
    s.env.mock_all_auths();
    if balance > 0 {
        s.token_admin.mint(&user, &balance);
    }
    if allowance > 0 {
        s.token.approve(&user, &s.contract, &allowance, &live_until);
    }
    user
}

/// A wallet with `balance` tokens and an allowance of the same size that
/// stays valid for the whole test.
fn subscriber(s: &Setup, balance: i128) -> Address {
    subscriber_with(s, balance, balance, far_future_ledger(s))
}

/// A ledger sequence far enough ahead to outlive every test.
fn far_future_ledger(s: &Setup) -> u32 {
    s.env.ledger().sequence() + 5_000_000
}

fn status_of(s: &Setup, sub_id: u64) -> SubStatus {
    s.client.get_subscription(&sub_id).status
}

fn next_due_of(s: &Setup, sub_id: u64) -> u64 {
    s.client.get_subscription(&sub_id).next_due
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

// ---- subscribe -----------------------------------------------------------------

#[test]
fn subscribe_charges_first_period_and_sets_next_due() {
    let s = setup();
    let (merchant, payout, plan_id) = merchant_with_plan(&s, 100 * USDC);
    let user = subscriber(&s, 1_000 * USDC);

    let sub_id = s.client.subscribe(&user, &plan_id);
    assert_eq!(sub_id, 1);

    // Exact auth: the subscriber authorizes `subscribe` and nothing else.
    // The `transfer_from` is authorized by the contract itself as spender.
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &user,
            "subscribe",
            (user.clone(), plan_id).into_val(&s.env)
        )
    );
    // Events of the call (asserted before any further invocation).
    let fee = 100 * USDC * FEE_BPS as i128 / 10_000;
    assert_eq!(
        last_events(&s),
        [
            events::Subscribed {
                subscriber: user.clone(),
                sub_id,
                plan_id,
                next_due: START + MONTH,
            }
            .to_xdr(&s.env, &s.contract),
            events::Charged {
                subscriber: user.clone(),
                sub_id,
                plan_id,
                merchant: merchant.clone(),
                amount: 100 * USDC,
                fee,
                next_due: START + MONTH,
            }
            .to_xdr(&s.env, &s.contract),
        ]
    );

    // Fee split: 0.5% of 100 USDC = 0.5 USDC.
    assert_eq!(fee, 5_000_000);
    assert_eq!(s.token.balance(&user), 900 * USDC);
    assert_eq!(s.token.balance(&payout), 100 * USDC - fee);
    assert_eq!(s.token.balance(&s.fee_recipient), fee);
    assert_eq!(s.token.balance(&s.contract), 0);
    // The allowance was consumed by exactly the plan amount.
    assert_eq!(s.token.allowance(&user, &s.contract), 900 * USDC);

    assert_eq!(
        s.client.get_subscription(&sub_id),
        Subscription {
            id: sub_id,
            subscriber: user.clone(),
            plan_id,
            next_due: START + MONTH,
            status: SubStatus::Active,
            created_at: START,
        }
    );
    assert_eq!(s.client.sub_id_of(&user, &plan_id), Some(sub_id));
}

#[test]
fn subscribe_by_another_signer_fails() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, 10 * USDC);

    for signer in [&s.admin, &Address::generate(&s.env)] {
        mock_auth_for(
            &s,
            signer,
            "subscribe",
            (user.clone(), plan_id).into_val(&s.env),
        );
        assert!(matches!(
            s.client.try_subscribe(&user, &plan_id),
            Err(Err(_))
        ));
    }
    s.env.mock_all_auths();
    assert_eq!(s.client.sub_id_of(&user, &plan_id), None);
    assert_eq!(s.token.balance(&user), 10 * USDC);
}

#[test]
fn duplicate_subscribe_is_rejected() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, 10 * USDC);

    let sub_id = s.client.subscribe(&user, &plan_id);
    assert_eq!(
        s.client.try_subscribe(&user, &plan_id),
        Err(Ok(Error::AlreadySubscribed))
    );
    // Still rejected while past due or paused: only cancel frees the slot.
    advance(&s, MONTH);
    s.token.approve(&user, &s.contract, &0, &0);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::PastDue);
    assert_eq!(
        s.client.try_subscribe(&user, &plan_id),
        Err(Ok(Error::AlreadySubscribed))
    );
    assert_eq!(s.token.balance(&user), 9 * USDC);

    // A different plan of the same merchant is fine.
    let (merchant, _, _) = merchant_with_plan(&s, USDC);
    let other_plan = s
        .client
        .create_plan(&merchant, &name(&s, "Other"), &USDC, &MONTH);
    s.token
        .approve(&user, &s.contract, &USDC, &far_future_ledger(&s));
    assert_eq!(s.client.subscribe(&user, &other_plan), 2);
}

#[test]
fn subscribe_rejects_inactive_plan_or_merchant_and_unknown_plan() {
    let s = setup();
    let (merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, 10 * USDC);

    assert_eq!(
        s.client.try_subscribe(&user, &99),
        Err(Ok(Error::PlanNotFound))
    );

    s.client.set_plan_active(&merchant, &plan_id, &false);
    assert_eq!(
        s.client.try_subscribe(&user, &plan_id),
        Err(Ok(Error::PlanInactive))
    );
    s.client.set_plan_active(&merchant, &plan_id, &true);

    s.client.set_merchant_active(&merchant, &false);
    assert_eq!(
        s.client.try_subscribe(&user, &plan_id),
        Err(Ok(Error::MerchantInactive))
    );
    s.client.set_merchant_active(&merchant, &true);

    assert_eq!(s.token.balance(&user), 10 * USDC);
    assert_eq!(s.client.sub_id_of(&user, &plan_id), None);
    assert_eq!(s.client.subscribe(&user, &plan_id), 1);
}

#[test]
fn subscribe_with_failed_pull_fails_and_writes_nothing() {
    let s = setup();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);

    // No allowance at all.
    let no_allowance = subscriber_with(&s, 100 * USDC, 0, 0);
    assert_eq!(
        s.client.try_subscribe(&no_allowance, &plan_id),
        Err(Ok(Error::PaymentFailed))
    );
    // Allowance smaller than the plan amount.
    let small_allowance = subscriber_with(&s, 100 * USDC, 10 * USDC - 1, far_future_ledger(&s));
    assert_eq!(
        s.client.try_subscribe(&small_allowance, &plan_id),
        Err(Ok(Error::PaymentFailed))
    );
    // Enough allowance, not enough balance.
    let poor = subscriber_with(&s, 10 * USDC - 1, 10 * USDC, far_future_ledger(&s));
    assert_eq!(
        s.client.try_subscribe(&poor, &plan_id),
        Err(Ok(Error::PaymentFailed))
    );

    for user in [&no_allowance, &small_allowance, &poor] {
        assert_eq!(s.client.sub_id_of(user, &plan_id), None);
    }
    assert_eq!(
        s.client.try_get_subscription(&1),
        Err(Ok(Error::SubscriptionNotFound))
    );
    assert_eq!(s.token.balance(&payout), 0);
    assert_eq!(s.token.balance(&s.fee_recipient), 0);
    assert_eq!(s.token.balance(&s.contract), 0);
    // The rejected calls did not consume a subscription id.
    let ok = subscriber(&s, 10 * USDC);
    assert_eq!(s.client.subscribe(&ok, &plan_id), 1);
}

// ---- charge: time gate (L5) ------------------------------------------------------

#[test]
fn charge_before_due_fails_and_after_due_succeeds_once_per_period() {
    let s = setup();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let fee = 10 * USDC * FEE_BPS as i128 / 10_000;

    // L5: not due yet, even one second before.
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::NotDue)));
    advance(&s, MONTH - 1);
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::NotDue)));
    assert_eq!(s.token.balance(&user), 990 * USDC);

    // Due: charged exactly once, then not due again until the next period.
    advance(&s, 1);
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&user), 980 * USDC);
    assert_eq!(s.token.balance(&payout), 2 * (10 * USDC - fee));
    assert_eq!(s.token.balance(&s.fee_recipient), 2 * fee);
    assert_eq!(next_due_of(&s, sub_id), START + 2 * MONTH);
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::NotDue)));
    assert_eq!(s.token.balance(&user), 980 * USDC);

    // `charge` is permissionless: no authorization is required at all.
    advance(&s, MONTH);
    s.env.mock_auths(&[]);
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.env.auths(), std::vec![]);
    assert_eq!(s.token.balance(&user), 970 * USDC);

    assert_eq!(
        s.client.try_charge(&99),
        Err(Ok(Error::SubscriptionNotFound))
    );
}

#[test]
fn charge_with_insufficient_allowance_sets_past_due_and_returns_false() {
    let s = setup();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    // Allowance covers exactly the first period.
    let user = subscriber_with(&s, 1_000 * USDC, 10 * USDC, far_future_ledger(&s));
    let sub_id = s.client.subscribe(&user, &plan_id);
    let payout_after_first = s.token.balance(&payout);
    let fees_after_first = s.token.balance(&s.fee_recipient);

    advance(&s, MONTH);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(
        last_events(&s),
        [events::ChargeFailed {
            subscriber: user.clone(),
            sub_id,
            plan_id,
            amount: 10 * USDC,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    let sub = s.client.get_subscription(&sub_id);
    assert_eq!(sub.status, SubStatus::PastDue);
    assert_eq!(
        sub.next_due,
        START + MONTH,
        "next_due is not advanced on failure"
    );
    assert_eq!(s.token.balance(&user), 990 * USDC);
    assert_eq!(s.token.balance(&payout), payout_after_first);
    assert_eq!(s.token.balance(&s.fee_recipient), fees_after_first);

    // Retrying while still unfunded fails again, idempotently.
    assert!(!s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::PastDue);

    // Topping up the allowance lets the next charge succeed and recover.
    s.token
        .approve(&user, &s.contract, &(100 * USDC), &far_future_ledger(&s));
    assert!(s.client.charge(&sub_id));
    let sub = s.client.get_subscription(&sub_id);
    assert_eq!(sub.status, SubStatus::Active);
    assert_eq!(sub.next_due, START + 2 * MONTH);
    assert_eq!(s.token.balance(&user), 980 * USDC);
}

#[test]
fn charge_with_insufficient_balance_sets_past_due_and_returns_false() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    // Big allowance, but only one period's worth of tokens.
    let user = subscriber_with(&s, 10 * USDC, 1_000 * USDC, far_future_ledger(&s));
    let sub_id = s.client.subscribe(&user, &plan_id);
    assert_eq!(s.token.balance(&user), 0);

    advance(&s, MONTH);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::PastDue);
    assert_eq!(s.token.balance(&user), 0);
    // The allowance was not consumed by the failed pull.
    assert_eq!(s.token.allowance(&user, &s.contract), 990 * USDC);

    s.token_admin.mint(&user, &(10 * USDC));
    assert!(s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);
    assert_eq!(s.token.balance(&user), 0);
}

#[test]
fn charge_with_expired_allowance_sets_past_due_and_returns_false() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    // The allowance expires before the second period is due: MONTH of
    // 5-second ledgers is 518_400 ledgers.
    let expiry = s.env.ledger().sequence() + 100;
    let user = subscriber_with(&s, 1_000 * USDC, 1_000 * USDC, expiry);
    let sub_id = s.client.subscribe(&user, &plan_id);

    advance(&s, MONTH);
    assert!(s.env.ledger().sequence() > expiry);
    assert_eq!(s.token.allowance(&user, &s.contract), 0);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::PastDue);
    assert_eq!(s.token.balance(&user), 990 * USDC);

    // Re-approving recovers the subscription.
    s.token
        .approve(&user, &s.contract, &(1_000 * USDC), &far_future_ledger(&s));
    assert!(s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);
}

// ---- no back-charging -------------------------------------------------------------

#[test]
fn no_back_charging_after_a_long_gap() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);

    // The keeper was down for three months (plus a bit).
    advance(&s, 3 * MONTH + 12_345);
    assert!(s.client.charge(&sub_id));
    // Charged once, not three times...
    assert_eq!(s.token.balance(&user), 980 * USDC);
    // ...and the next charge is one full period from now, not from the
    // old next_due.
    assert_eq!(next_due_of(&s, sub_id), now(&s) + MONTH);
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::NotDue)));
    assert_eq!(s.token.balance(&user), 980 * USDC);

    // When the gap is shorter than a period, next_due advances by exactly
    // one period from the old next_due (the schedule does not drift).
    advance(&s, MONTH + 1_000);
    let due_before = next_due_of(&s, sub_id);
    assert!(s.client.charge(&sub_id));
    assert_eq!(next_due_of(&s, sub_id), due_before + MONTH);

    // Past due for a long time, then funded: still a single charge.
    s.token.approve(&user, &s.contract, &0, &0);
    advance(&s, MONTH);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::PastDue);
    advance(&s, 5 * MONTH);
    s.token
        .approve(&user, &s.contract, &(1_000 * USDC), &far_future_ledger(&s));
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&user), 960 * USDC);
    assert_eq!(next_due_of(&s, sub_id), now(&s) + MONTH);
}

// ---- fee split ------------------------------------------------------------------

#[test]
fn fee_split_is_exact_including_rounding() {
    let s = setup();
    s.env.mock_all_auths();
    // 50 bps of 1.2345679 USDC = 61_728.395 stroops → 61_728 (rounded down).
    let amount = 12_345_679_i128;
    let (merchant, payout, plan_id) = merchant_with_plan(&s, amount);
    let user = subscriber(&s, 10 * USDC);

    let sub_id = s.client.subscribe(&user, &plan_id);
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728);
    assert_eq!(s.token.balance(&payout), amount - 61_728);
    assert_eq!(s.token.balance(&user), 10 * USDC - amount);
    assert_eq!(s.token.balance(&s.contract), 0);

    // Amounts too small to produce a fee: the merchant gets everything.
    let tiny_plan = s
        .client
        .create_plan(&merchant, &name(&s, "Tiny"), &199, &MONTH);
    s.client.subscribe(&user, &tiny_plan);
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728);
    assert_eq!(s.token.balance(&payout), amount - 61_728 + 199);

    // Fee changes apply to the next charge; the cap (10%) is honoured.
    s.client.set_fee(&crate::MAX_FEE_BPS);
    advance(&s, MONTH);
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728 + 1_234_567);
    assert_eq!(
        s.token.balance(&payout),
        amount - 61_728 + 199 + amount - 1_234_567
    );

    // Zero fee: nothing goes to the fee recipient.
    s.client.set_fee(&0);
    advance(&s, MONTH);
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&s.fee_recipient), 61_728 + 1_234_567);
    assert_eq!(s.token.balance(&s.contract), 0);
}

// ---- inactive merchant: charge is skipped ---------------------------------------

#[test]
fn charge_skips_subscriptions_of_inactive_merchants_until_reactivated() {
    let s = setup();
    let (merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let payout_before = s.token.balance(&payout);
    let fees_before = s.token.balance(&s.fee_recipient);

    s.client.set_merchant_active(&merchant, &false);
    advance(&s, MONTH);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(
        last_events(&s),
        [events::ChargeSkipped {
            subscriber: user.clone(),
            sub_id,
            reason: Symbol::new(&s.env, "merchant_inactive"),
        }
        .to_xdr(&s.env, &s.contract)]
    );
    // No transfer, no state change: not PastDue, next_due unchanged.
    let sub = s.client.get_subscription(&sub_id);
    assert_eq!(sub.status, SubStatus::Active);
    assert_eq!(sub.next_due, START + MONTH);
    assert_eq!(s.token.balance(&user), 990 * USDC);
    assert_eq!(s.token.balance(&payout), payout_before);
    assert_eq!(s.token.balance(&s.fee_recipient), fees_before);

    // Reactivated: the next charge succeeds (once, no back-charging).
    s.client.set_merchant_active(&merchant, &true);
    assert!(s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);
    assert_eq!(s.token.balance(&user), 980 * USDC);
    assert_eq!(next_due_of(&s, sub_id), START + 2 * MONTH);
}

// ---- unpayable payout address: nothing pulled, allowance untouched ----------------

#[test]
fn add_merchant_rejects_a_payout_address_that_cannot_receive_usdc() {
    let (s, mock) = setup_with_mock_token();
    let merchant = Address::generate(&s.env);
    let payout = Address::generate(&s.env);
    mock.set_unpayable(&payout);

    s.env.mock_all_auths();
    assert_eq!(
        s.client.try_add_merchant(&merchant, &payout),
        Err(Ok(Error::PayoutUnavailable))
    );
    assert_eq!(s.client.get_merchant(&merchant), None);

    // With a payable payout address the same merchant is accepted.
    let payable = Address::generate(&s.env);
    s.client.add_merchant(&merchant, &payable);
    assert_eq!(s.client.get_merchant(&merchant).unwrap().payout, payable);
}

#[test]
fn subscribe_fails_cleanly_when_the_payout_address_cannot_receive_usdc() {
    let (s, mock) = setup_with_mock_token();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 100 * USDC);

    // The payout address became unpayable after onboarding (trustline
    // removed or frozen).
    mock.set_unpayable(&payout);
    assert_eq!(
        s.client.try_subscribe(&user, &plan_id),
        Err(Ok(Error::PayoutUnavailable))
    );
    // Nothing pulled, nothing written, allowance untouched, no id consumed.
    assert_eq!(s.token.balance(&user), 100 * USDC);
    assert_eq!(s.token.allowance(&user, &s.contract), 100 * USDC);
    assert_eq!(s.client.sub_id_of(&user, &plan_id), None);
    assert_eq!(s.token.balance(&s.contract), 0);
    assert_eq!(s.token.balance(&s.fee_recipient), 0);

    mock.set_unpayable(&Address::generate(&s.env));
    assert_eq!(s.client.subscribe(&user, &plan_id), 1);
}

#[test]
fn charge_skips_when_the_payout_address_cannot_receive_usdc() {
    let (s, mock) = setup_with_mock_token();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let payout_before = s.token.balance(&payout);
    let fees_before = s.token.balance(&s.fee_recipient);
    let allowance_before = s.token.allowance(&user, &s.contract);
    assert_eq!(allowance_before, 990 * USDC);

    mock.set_unpayable(&payout);
    advance(&s, MONTH);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(
        last_events(&s),
        [events::ChargeSkipped {
            subscriber: user.clone(),
            sub_id,
            reason: Symbol::new(&s.env, "payout_unavailable"),
        }
        .to_xdr(&s.env, &s.contract)]
    );
    // Same handling as an inactive merchant: no pull, no allowance consumed,
    // not PastDue, next_due unchanged, no funds moved.
    let sub = s.client.get_subscription(&sub_id);
    assert_eq!(sub.status, SubStatus::Active);
    assert_eq!(sub.next_due, START + MONTH);
    assert_eq!(s.token.balance(&user), 990 * USDC);
    assert_eq!(s.token.allowance(&user, &s.contract), allowance_before);
    assert_eq!(s.token.balance(&s.fee_recipient), fees_before);
    assert_eq!(s.token.balance(&s.contract), 0);
    assert!(
        s.token.try_balance(&payout).is_err(),
        "like the SAC, unreadable"
    );

    // Once the payout address can receive USDC, the next charge succeeds
    // (once, no back-charging).
    mock.set_unpayable(&Address::generate(&s.env));
    assert_eq!(s.token.balance(&payout), payout_before);
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&user), 980 * USDC);
    assert_eq!(s.token.allowance(&user, &s.contract), 980 * USDC);
    assert_eq!(next_due_of(&s, sub_id), START + 2 * MONTH);
}

#[test]
fn charge_batch_charges_everyone_else_when_one_payout_is_unpayable() {
    let (s, mock) = setup_with_mock_token();
    let (_merchant_a, payout_a, plan_a) = merchant_with_plan(&s, 10 * USDC);
    let (_merchant_b, payout_b, plan_b) = merchant_with_plan(&s, 20 * USDC);
    let (_merchant_c, payout_c, plan_c) = merchant_with_plan(&s, 30 * USDC);
    let user_a = subscriber(&s, 1_000 * USDC);
    let user_b = subscriber(&s, 1_000 * USDC);
    let user_c = subscriber(&s, 1_000 * USDC);
    let sub_a = s.client.subscribe(&user_a, &plan_a);
    let sub_b = s.client.subscribe(&user_b, &plan_b);
    let sub_c = s.client.subscribe(&user_c, &plan_c);
    let (payout_a_before, payout_c_before) =
        (s.token.balance(&payout_a), s.token.balance(&payout_c));
    let fees_before = s.token.balance(&s.fee_recipient);
    let fee = |amount: i128| amount * FEE_BPS as i128 / 10_000;

    mock.set_unpayable(&payout_b);
    advance(&s, MONTH);
    assert_eq!(
        s.client
            .charge_batch(&soroban_sdk::vec![&s.env, sub_a, sub_b, sub_c]),
        soroban_sdk::vec![&s.env, true, false, true]
    );
    let batch_events = last_events(&s);
    assert_eq!(batch_events.events().len(), 3);
    assert_eq!(
        batch_events.events()[1],
        events::ChargeSkipped {
            subscriber: user_b.clone(),
            sub_id: sub_b,
            reason: Symbol::new(&s.env, "payout_unavailable"),
        }
        .to_xdr(&s.env, &s.contract)
    );
    // A and C charged and paid out; B untouched in every respect.
    assert_eq!(s.token.balance(&user_a), 980 * USDC);
    assert_eq!(s.token.balance(&user_c), 940 * USDC);
    assert_eq!(
        s.token.balance(&payout_a),
        payout_a_before + 10 * USDC - fee(10 * USDC)
    );
    assert_eq!(
        s.token.balance(&payout_c),
        payout_c_before + 30 * USDC - fee(30 * USDC)
    );
    assert_eq!(
        s.token.balance(&s.fee_recipient),
        fees_before + fee(10 * USDC) + fee(30 * USDC)
    );
    assert_eq!(s.token.balance(&user_b), 980 * USDC);
    assert_eq!(s.token.allowance(&user_b, &s.contract), 980 * USDC);
    let sub = s.client.get_subscription(&sub_b);
    assert_eq!(sub.status, SubStatus::Active);
    assert_eq!(sub.next_due, START + MONTH);
    assert_eq!(s.token.balance(&s.contract), 0);
}

#[test]
fn unpayable_fee_recipient_aborts_the_whole_charge() {
    // The fee recipient is admin-controlled and the deploy script creates
    // its trustline, so there is no per-item handling for it: the call
    // fails as a whole and the host rolls back the pull and the payout.
    let (s, mock) = setup_with_mock_token();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let payout_before = s.token.balance(&payout);
    advance(&s, MONTH);
    mock.set_unpayable(&s.fee_recipient);

    assert_eq!(
        s.client.try_charge(&sub_id),
        Err(Err(InvokeError::Contract(MockTokenError::Unpayable as u32)))
    );
    assert_eq!(s.token.balance(&user), 990 * USDC);
    assert_eq!(s.token.allowance(&user, &s.contract), 990 * USDC);
    assert_eq!(s.token.balance(&payout), payout_before);
    assert_eq!(s.token.balance(&s.contract), 0);
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);
    assert_eq!(next_due_of(&s, sub_id), START + MONTH);
}

// ---- charge_batch -----------------------------------------------------------------

#[test]
fn charge_batch_enforces_the_limit_and_the_pause() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, 100 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    advance(&s, MONTH);

    let mut too_many = soroban_sdk::Vec::new(&s.env);
    for _ in 0..=MAX_BATCH {
        too_many.push_back(sub_id);
    }
    assert_eq!(too_many.len(), MAX_BATCH + 1);
    assert_eq!(
        s.client.try_charge_batch(&too_many),
        Err(Ok(Error::BatchTooLarge))
    );
    assert_eq!(s.token.balance(&user), 99 * USDC);

    // Exactly MAX_BATCH ids is fine; the same id repeated is charged once
    // and then reported as not due.
    let mut max = soroban_sdk::Vec::new(&s.env);
    for _ in 0..MAX_BATCH {
        max.push_back(sub_id);
    }
    let results = s.client.charge_batch(&max);
    assert_eq!(results.len(), MAX_BATCH);
    assert!(results.get(0).unwrap());
    assert!(!results.get(1).unwrap());
    assert_eq!(s.token.balance(&user), 98 * USDC);

    // Paused fails the whole batch, as does a single charge.
    s.client.pause();
    advance(&s, MONTH);
    assert_eq!(
        s.client
            .try_charge_batch(&soroban_sdk::vec![&s.env, sub_id]),
        Err(Ok(Error::Paused))
    );
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::Paused)));
    assert_eq!(
        s.client.try_subscribe(&subscriber(&s, USDC), &plan_id),
        Err(Ok(Error::Paused))
    );
    s.client.unpause();
    assert_eq!(
        s.client.charge_batch(&soroban_sdk::vec![&s.env, sub_id]),
        soroban_sdk::vec![&s.env, true]
    );
    // An empty batch is a no-op.
    assert_eq!(
        s.client.charge_batch(&soroban_sdk::Vec::new(&s.env)),
        soroban_sdk::Vec::new(&s.env)
    );
}

#[test]
fn charge_batch_returns_false_for_skipped_items_instead_of_failing() {
    let s = setup();
    let (merchant_a, payout_a, plan_a) = merchant_with_plan(&s, 10 * USDC);
    let (merchant_b, payout_b, plan_b) = merchant_with_plan(&s, 20 * USDC);

    let due_a = subscriber(&s, 1_000 * USDC);
    let sub_due_a = s.client.subscribe(&due_a, &plan_a); // 1: due, charged
    let unfunded = subscriber_with(&s, 1_000 * USDC, 10 * USDC, far_future_ledger(&s));
    let sub_unfunded = s.client.subscribe(&unfunded, &plan_a); // 2: pull fails
    let due_b = subscriber(&s, 1_000 * USDC);
    let sub_due_b = s.client.subscribe(&due_b, &plan_b); // 3: inactive merchant
    advance(&s, MONTH);
    let late = subscriber(&s, 1_000 * USDC);
    let sub_not_due = s.client.subscribe(&late, &plan_a); // 4: not due
    let payout_a_before = s.token.balance(&payout_a);
    let payout_b_before = s.token.balance(&payout_b);

    s.client.set_merchant_active(&merchant_b, &false);
    let ids = soroban_sdk::vec![
        &s.env,
        sub_due_a,
        sub_unfunded,
        sub_due_b,
        sub_not_due,
        999, // unknown id
    ];
    let results = s.client.charge_batch(&ids);
    assert_eq!(
        results,
        soroban_sdk::vec![&s.env, true, false, false, false, false]
    );

    // 1: charged.
    assert_eq!(s.token.balance(&due_a), 980 * USDC);
    assert_eq!(status_of(&s, sub_due_a), SubStatus::Active);
    // 2: past due, nothing pulled.
    assert_eq!(s.token.balance(&unfunded), 990 * USDC);
    assert_eq!(status_of(&s, sub_unfunded), SubStatus::PastDue);
    // 3: skipped because the merchant is inactive: untouched.
    assert_eq!(s.token.balance(&due_b), 980 * USDC);
    let sub_b = s.client.get_subscription(&sub_due_b);
    assert_eq!(sub_b.status, SubStatus::Active);
    assert_eq!(sub_b.next_due, START + MONTH);
    assert_eq!(s.token.balance(&payout_b), payout_b_before);
    // 4: not due: untouched.
    assert_eq!(s.token.balance(&late), 990 * USDC);
    assert_eq!(next_due_of(&s, sub_not_due), START + 2 * MONTH);
    // Only one period of plan A was paid out in the batch.
    let fee_a = 10 * USDC * FEE_BPS as i128 / 10_000;
    assert_eq!(
        s.token.balance(&payout_a),
        payout_a_before + 10 * USDC - fee_a
    );
    assert_eq!(s.token.balance(&s.contract), 0);

    // Reactivating merchant B and re-running the batch charges B's sub, and
    // the recovered unfunded sub once it is re-approved.
    s.client.set_merchant_active(&merchant_b, &true);
    s.token.approve(
        &unfunded,
        &s.contract,
        &(100 * USDC),
        &far_future_ledger(&s),
    );
    let results = s.client.charge_batch(&ids);
    assert_eq!(
        results,
        soroban_sdk::vec![&s.env, false, true, true, false, false]
    );
    assert_eq!(s.token.balance(&due_b), 960 * USDC);
    assert_eq!(status_of(&s, sub_unfunded), SubStatus::Active);
    assert_eq!(s.token.balance(&unfunded), 980 * USDC);
    let _ = (merchant_a, payout_a_before);
}

// ---- subscriber pause / resume / cancel (L7) -----------------------------------------

#[test]
fn only_the_subscriber_can_pause_resume_or_cancel() {
    let s = setup();
    let (merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, 100 * USDC);
    let other = subscriber(&s, 100 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let other_sub = s.client.subscribe(&other, &plan_id);

    // Another subscriber signing for their own address but naming the
    // victim's subscription id: rejected by the ownership check (L7: the
    // subscription is addressed by its own id, never by a plan index).
    for (f, res) in [
        (
            "pause_subscription",
            s.client.try_pause_subscription(&other, &sub_id),
        ),
        (
            "resume_subscription",
            s.client.try_resume_subscription(&other, &sub_id),
        ),
        (
            "cancel_subscription",
            s.client.try_cancel_subscription(&other, &sub_id),
        ),
    ] {
        assert_eq!(res, Err(Ok(Error::NotSubscriber)), "{f}");
    }
    // Admin, merchant or a stranger signing on the subscriber's behalf:
    // rejected by `require_auth`.
    for signer in [&s.admin, &merchant, &Address::generate(&s.env)] {
        for f in [
            "pause_subscription",
            "resume_subscription",
            "cancel_subscription",
        ] {
            mock_auth_for(&s, signer, f, (user.clone(), sub_id).into_val(&s.env));
            let res = match f {
                "pause_subscription" => s.client.try_pause_subscription(&user, &sub_id),
                "resume_subscription" => s.client.try_resume_subscription(&user, &sub_id),
                _ => s.client.try_cancel_subscription(&user, &sub_id),
            };
            assert!(matches!(res, Err(Err(_))), "{f} by wrong signer");
        }
    }
    s.env.mock_all_auths();
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);
    assert_eq!(status_of(&s, other_sub), SubStatus::Active);
    assert_eq!(
        s.client.try_pause_subscription(&user, &99),
        Err(Ok(Error::SubscriptionNotFound))
    );

    // The owner succeeds, with exactly their own authorization.
    s.client.pause_subscription(&user, &sub_id);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &user,
            "pause_subscription",
            (user.clone(), sub_id).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::SubscriptionPaused {
            subscriber: user.clone(),
            sub_id,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(status_of(&s, sub_id), SubStatus::Paused);
    assert_eq!(status_of(&s, other_sub), SubStatus::Active);
}

#[test]
fn paused_subscription_is_not_charged_and_resume_never_sets_next_due_in_the_past() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);

    // Pause before the period elapsed: resume keeps the original due date.
    s.client.pause_subscription(&user, &sub_id);
    advance(&s, MONTH / 2);
    s.client.resume_subscription(&user, &sub_id);
    assert_eq!(
        last_events(&s),
        [events::SubscriptionResumed {
            subscriber: user.clone(),
            sub_id,
            next_due: START + MONTH,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(next_due_of(&s, sub_id), START + MONTH);
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);

    // Pause across several due dates: not chargeable meanwhile, and resume
    // moves next_due to now (nothing is owed for the paused time).
    s.client.pause_subscription(&user, &sub_id);
    advance(&s, 3 * MONTH);
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::NotChargeable)));
    assert_eq!(
        s.client.charge_batch(&soroban_sdk::vec![&s.env, sub_id]),
        soroban_sdk::vec![&s.env, false]
    );
    assert_eq!(s.token.balance(&user), 990 * USDC);
    s.client.resume_subscription(&user, &sub_id);
    assert_eq!(next_due_of(&s, sub_id), now(&s));
    // Due immediately after resuming: one charge, then one period ahead.
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&user), 980 * USDC);
    assert_eq!(next_due_of(&s, sub_id), now(&s) + MONTH);
}

#[test]
fn status_transitions_follow_the_state_machine() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber_with(&s, 1_000 * USDC, 10 * USDC, far_future_ledger(&s));
    let sub_id = s.client.subscribe(&user, &plan_id);

    // Active: resume is invalid, pause is valid.
    assert_eq!(
        s.client.try_resume_subscription(&user, &sub_id),
        Err(Ok(Error::NotPaused))
    );
    s.client.pause_subscription(&user, &sub_id);
    // Paused: pause again is invalid.
    assert_eq!(
        s.client.try_pause_subscription(&user, &sub_id),
        Err(Ok(Error::NotPausable))
    );
    s.client.resume_subscription(&user, &sub_id);

    // PastDue (allowance exhausted): can be paused and resumed.
    advance(&s, MONTH);
    assert!(!s.client.charge(&sub_id));
    assert_eq!(status_of(&s, sub_id), SubStatus::PastDue);
    s.client.pause_subscription(&user, &sub_id);
    assert_eq!(status_of(&s, sub_id), SubStatus::Paused);
    s.client.resume_subscription(&user, &sub_id);
    // Resuming a formerly past-due sub makes it Active again; it is still
    // due, so the next charge (now funded) succeeds.
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);
    s.token
        .approve(&user, &s.contract, &(100 * USDC), &far_future_ledger(&s));
    assert!(s.client.charge(&sub_id));

    // Cancel from Active; then everything else is invalid.
    s.client.cancel_subscription(&user, &sub_id);
    assert_eq!(status_of(&s, sub_id), SubStatus::Cancelled);
    assert_eq!(
        s.client.try_cancel_subscription(&user, &sub_id),
        Err(Ok(Error::AlreadyCancelled))
    );
    assert_eq!(
        s.client.try_pause_subscription(&user, &sub_id),
        Err(Ok(Error::NotPausable))
    );
    assert_eq!(
        s.client.try_resume_subscription(&user, &sub_id),
        Err(Ok(Error::NotPaused))
    );
    advance(&s, MONTH);
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::NotChargeable)));

    // Cancel from Paused and from PastDue are both allowed.
    let paused = s.client.subscribe(&user, &plan_id);
    s.client.pause_subscription(&user, &paused);
    s.client.cancel_subscription(&user, &paused);
    assert_eq!(status_of(&s, paused), SubStatus::Cancelled);
    s.token
        .approve(&user, &s.contract, &(10 * USDC), &far_future_ledger(&s));
    let past_due = s.client.subscribe(&user, &plan_id);
    advance(&s, MONTH);
    assert!(!s.client.charge(&past_due));
    assert_eq!(status_of(&s, past_due), SubStatus::PastDue);
    s.client.cancel_subscription(&user, &past_due);
    assert_eq!(status_of(&s, past_due), SubStatus::Cancelled);
}

#[test]
fn cancel_frees_the_slot_and_resubscribe_works() {
    let s = setup();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    assert_eq!(s.client.sub_id_of(&user, &plan_id), Some(sub_id));

    s.client.cancel_subscription(&user, &sub_id);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &user,
            "cancel_subscription",
            (user.clone(), sub_id).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::SubscriptionCancelled {
            subscriber: user.clone(),
            sub_id,
        }
        .to_xdr(&s.env, &s.contract)]
    );
    assert_eq!(s.client.sub_id_of(&user, &plan_id), None);
    // The cancelled subscription stays readable by id (L10).
    assert_eq!(status_of(&s, sub_id), SubStatus::Cancelled);
    let payout_before = s.token.balance(&payout);

    // Re-subscribing creates a new subscription (new id) and charges the
    // first period again.
    let new_id = s.client.subscribe(&user, &plan_id);
    assert_eq!(new_id, 2);
    assert_eq!(s.client.sub_id_of(&user, &plan_id), Some(new_id));
    assert_eq!(status_of(&s, new_id), SubStatus::Active);
    assert_eq!(s.token.balance(&user), 980 * USDC);
    assert!(s.token.balance(&payout) > payout_before);
    // The old one is still cancelled and inert.
    advance(&s, MONTH);
    assert_eq!(s.client.try_charge(&sub_id), Err(Ok(Error::NotChargeable)));
    assert!(s.client.charge(&new_id));
}

#[test]
fn subscribers_keep_control_while_the_contract_is_paused() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, 100 * USDC);
    let a = s.client.subscribe(&user, &plan_id);
    let (merchant_b, _, _) = merchant_with_plan(&s, USDC);
    let plan_b = s
        .client
        .create_plan(&merchant_b, &name(&s, "B"), &USDC, &MONTH);
    let b = s.client.subscribe(&user, &plan_b);
    s.client.pause_subscription(&user, &b);

    s.client.pause();
    s.client.pause_subscription(&user, &a);
    s.client.resume_subscription(&user, &a);
    s.client.cancel_subscription(&user, &a);
    s.client.resume_subscription(&user, &b);
    s.client.cancel_subscription(&user, &b);
    assert_eq!(status_of(&s, a), SubStatus::Cancelled);
    assert_eq!(status_of(&s, b), SubStatus::Cancelled);
    s.client.unpause();
}

// ---- admin two-step transfer -----------------------------------------------------

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

    // Neither the old admin nor a stranger can accept for the proposed admin.
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
    let merchant = Address::generate(&s.env);
    let payout = Address::generate(&s.env);
    mock_auth_for(
        &s,
        &s.admin,
        "add_merchant",
        (merchant.clone(), payout.clone()).into_val(&s.env),
    );
    assert!(matches!(
        s.client.try_add_merchant(&merchant, &payout),
        Err(Err(_))
    ));
    mock_auth_for(
        &s,
        &new_admin,
        "add_merchant",
        (merchant.clone(), payout.clone()).into_val(&s.env),
    );
    s.client.add_merchant(&merchant, &payout);
    s.env.mock_all_auths();
    assert!(s.client.get_merchant(&merchant).is_some());
}

// ---- fee recipient rotation --------------------------------------------------------

#[test]
fn set_fee_recipient_redirects_future_fees() {
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, 100 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let fee = 100 * USDC * FEE_BPS as i128 / 10_000;
    assert_eq!(s.token.balance(&s.fee_recipient), fee);

    let new_recipient = Address::generate(&s.env);
    s.client.set_fee_recipient(&new_recipient);
    assert_eq!(
        s.env.auths(),
        expected_auth(
            &s,
            &s.admin,
            "set_fee_recipient",
            (new_recipient.clone(),).into_val(&s.env)
        )
    );
    assert_eq!(s.client.fee_recipient(), new_recipient);

    advance(&s, MONTH);
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&s.fee_recipient), fee);
    assert_eq!(s.token.balance(&new_recipient), fee);
}

// ---- every state change emits its event -----------------------------------------------

#[test]
fn every_state_change_emits_its_event() {
    let s = setup();
    let merchant = Address::generate(&s.env);
    let payout = Address::generate(&s.env);
    let new_recipient = Address::generate(&s.env);
    let new_admin = Address::generate(&s.env);
    let user = subscriber(&s, 1_000 * USDC);
    let plan_name = name(&s, "Basic");
    s.env.mock_all_auths();

    macro_rules! expect_event {
        ($call:expr, $event:expr) => {
            $call;
            assert_eq!(last_events(&s), [$event.to_xdr(&s.env, &s.contract)]);
        };
    }

    expect_event!(
        s.client.add_merchant(&merchant, &payout),
        events::MerchantAdded {
            merchant: merchant.clone(),
            payout: payout.clone(),
        }
    );
    expect_event!(
        s.client.set_merchant_active(&merchant, &false),
        events::MerchantActiveSet {
            merchant: merchant.clone(),
            active: false,
        }
    );
    s.client.set_merchant_active(&merchant, &true);
    expect_event!(
        s.client
            .create_plan(&merchant, &plan_name, &(10 * USDC), &MONTH),
        events::PlanCreated {
            merchant: merchant.clone(),
            plan_id: 1,
            name: plan_name.clone(),
            amount: 10 * USDC,
            period: MONTH,
        }
    );
    expect_event!(
        s.client.set_plan_active(&merchant, &1, &false),
        events::PlanActiveSet {
            merchant: merchant.clone(),
            plan_id: 1,
            active: false,
        }
    );
    s.client.set_plan_active(&merchant, &1, &true);

    // subscribe emits two events: subscribed, then charged.
    s.client.subscribe(&user, &1);
    assert_eq!(
        last_events(&s),
        [
            events::Subscribed {
                subscriber: user.clone(),
                sub_id: 1,
                plan_id: 1,
                next_due: START + MONTH,
            }
            .to_xdr(&s.env, &s.contract),
            events::Charged {
                subscriber: user.clone(),
                sub_id: 1,
                plan_id: 1,
                merchant: merchant.clone(),
                amount: 10 * USDC,
                fee: 10 * USDC * FEE_BPS as i128 / 10_000,
                next_due: START + MONTH,
            }
            .to_xdr(&s.env, &s.contract),
        ]
    );
    advance(&s, MONTH);
    expect_event!(
        assert!(s.client.charge(&1)),
        events::Charged {
            subscriber: user.clone(),
            sub_id: 1,
            plan_id: 1,
            merchant: merchant.clone(),
            amount: 10 * USDC,
            fee: 10 * USDC * FEE_BPS as i128 / 10_000,
            next_due: START + 2 * MONTH,
        }
    );
    advance(&s, MONTH);
    s.token.approve(&user, &s.contract, &0, &0);
    expect_event!(
        assert!(!s.client.charge(&1)),
        events::ChargeFailed {
            subscriber: user.clone(),
            sub_id: 1,
            plan_id: 1,
            amount: 10 * USDC,
        }
    );
    s.client.set_merchant_active(&merchant, &false);
    expect_event!(
        assert!(!s.client.charge(&1)),
        events::ChargeSkipped {
            subscriber: user.clone(),
            sub_id: 1,
            reason: Symbol::new(&s.env, "merchant_inactive"),
        }
    );
    s.client.set_merchant_active(&merchant, &true);
    expect_event!(
        s.client.pause_subscription(&user, &1),
        events::SubscriptionPaused {
            subscriber: user.clone(),
            sub_id: 1,
        }
    );
    expect_event!(
        s.client.resume_subscription(&user, &1),
        events::SubscriptionResumed {
            subscriber: user.clone(),
            sub_id: 1,
            next_due: now(&s),
        }
    );
    expect_event!(
        s.client.cancel_subscription(&user, &1),
        events::SubscriptionCancelled {
            subscriber: user.clone(),
            sub_id: 1,
        }
    );
    expect_event!(s.client.set_fee(&10), events::FeeSet { fee_bps: 10 });
    expect_event!(
        s.client.set_fee_recipient(&new_recipient),
        events::FeeRecipientSet {
            fee_recipient: new_recipient.clone(),
        }
    );
    expect_event!(s.client.pause(), events::Paused {});
    expect_event!(s.client.unpause(), events::Unpaused {});
    expect_event!(
        s.client.propose_admin(&new_admin),
        events::AdminProposed {
            pending_admin: new_admin.clone(),
        }
    );
    expect_event!(
        s.client.accept_admin(),
        events::AdminTransferred {
            new_admin: new_admin.clone(),
            old_admin: s.admin.clone(),
        }
    );

    // Reads and rejected calls emit nothing.
    s.client.get_plan(&1);
    assert_eq!(last_events(&s).events().len(), 0);
    assert!(s.client.try_charge(&1).is_err());
    assert_eq!(last_events(&s).events().len(), 0);
    assert!(s.client.try_subscribe(&user, &99).is_err());
    assert_eq!(last_events(&s).events().len(), 0);
}

// ---- no PII on-chain (L6): enforced by the types ------------------------------------------

/// The on-chain types carry no personal data. This is checked against the
/// contract's own type spec (the XDR the SDK embeds in the wasm), so a field
/// added to any of these types shows up here, and by exhaustive
/// destructuring, which fails to compile if a field is added.
#[test]
fn on_chain_types_have_no_pii_fields() {
    use soroban_sdk::xdr::{Limits, ReadXdr, ScSpecEntry};

    fn field_names(spec: &[u8]) -> std::vec::Vec<std::string::String> {
        match ScSpecEntry::from_xdr(spec, Limits::none()).unwrap() {
            ScSpecEntry::UdtStructV0(st) => st
                .fields
                .iter()
                .map(|f| f.name.to_utf8_string_lossy())
                .collect(),
            other => panic!("expected a struct spec, got {other:?}"),
        }
    }

    // (The SDK sorts struct fields alphabetically in the spec.)
    assert_eq!(field_names(&Merchant::spec_xdr()), ["active", "payout"]);
    assert_eq!(
        field_names(&Plan::spec_xdr()),
        ["active", "amount", "id", "merchant", "name", "period"]
    );
    assert_eq!(
        field_names(&Subscription::spec_xdr()),
        [
            "created_at",
            "id",
            "next_due",
            "plan_id",
            "status",
            "subscriber"
        ]
    );

    // Nothing that looks like personal data anywhere in the type specs.
    for spec in [
        Merchant::spec_xdr().as_slice(),
        Plan::spec_xdr().as_slice(),
        Subscription::spec_xdr().as_slice(),
    ] {
        for field in field_names(spec) {
            for forbidden in ["email", "password", "phone", "user_name", "username", "pin"] {
                assert!(
                    !field.to_lowercase().contains(forbidden),
                    "PII-looking field `{field}`"
                );
            }
        }
    }

    // Exhaustive destructuring: adding a field breaks this test at compile
    // time, forcing a conscious review.
    let s = setup();
    let (merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let Merchant { payout, active } = s.client.get_merchant(&merchant).unwrap();
    let _ = (payout, active);
    let Plan {
        id,
        merchant: _,
        name,
        amount,
        period,
        active,
    } = s.client.get_plan(&plan_id);
    let _ = (id, name, amount, period, active);
    let Subscription {
        id,
        subscriber,
        plan_id,
        next_due,
        status,
        created_at,
    } = s.client.get_subscription(&sub_id);
    let _ = (id, subscriber, plan_id, next_due, status, created_at);
}

// ---- upgrade -------------------------------------------------------------------------

/// Positive upgrade path. Uploads this contract's own optimized wasm
/// (built by `stellar contract build`) and upgrades to it, which exercises
/// the full admin-authorized `update_current_contract` path.
#[cfg(feature = "wasm-upgrade-test")]
#[test]
fn upgrade_by_admin_replaces_the_executable() {
    const WASM: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/wasm32v1-none/release/rubbi_subscriptions.wasm"
    ));
    let s = setup();
    let (_merchant, _payout, plan_id) = merchant_with_plan(&s, USDC);
    let user = subscriber(&s, 100 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);

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
        expected_auth(
            &s,
            &s.admin,
            "upgrade",
            (wasm_hash.clone(),).into_val(&s.env)
        )
    );
    assert_eq!(
        last_events(&s),
        [events::Upgraded { wasm_hash }.to_xdr(&s.env, &s.contract)]
    );

    // Storage survived and the (now wasm) contract still works.
    s.env.mock_all_auths();
    assert_eq!(s.client.admin(), s.admin);
    assert_eq!(status_of(&s, sub_id), SubStatus::Active);
    advance(&s, MONTH);
    assert!(s.client.charge(&sub_id));
    assert_eq!(s.token.balance(&user), 98 * USDC);
}

#[test]
fn charge_skips_and_refunds_when_merchant_payout_transfer_fails() {
    let (s, mock) = setup_with_mock_token();
    let (_merchant, payout, plan_id) = merchant_with_plan(&s, 10 * USDC);
    let user = subscriber(&s, 1_000 * USDC);
    let sub_id = s.client.subscribe(&user, &plan_id);
    let allowance_before = s.token.allowance(&user, &s.contract);
    assert_eq!(allowance_before, 990 * USDC);

    // Simulate trustline limit exceeded on payout: authorized returns true, but transfer rejects
    mock.set_reject_transfer(&payout);
    advance(&s, MONTH);
    assert!(!s.client.charge(&sub_id));

    assert_eq!(
        last_events(&s),
        [events::ChargeSkipped {
            subscriber: user.clone(),
            sub_id,
            reason: Symbol::new(&s.env, "payout_failed"),
        }
        .to_xdr(&s.env, &s.contract)]
    );

    // Pulled tokens were refunded to subscriber, status and next_due unchanged
    let sub = s.client.get_subscription(&sub_id);
    assert_eq!(sub.status, SubStatus::Active);
    assert_eq!(sub.next_due, START + MONTH);
    assert_eq!(s.token.balance(&user), 990 * USDC);
    assert_eq!(s.token.balance(&s.contract), 0);
}

#[test]
fn charge_batch_charges_everyone_else_when_one_merchant_payout_transfer_fails() {
    let (s, mock) = setup_with_mock_token();
    let (_merchant_a, payout_a, plan_a) = merchant_with_plan(&s, 10 * USDC);
    let (_merchant_b, payout_b, plan_b) = merchant_with_plan(&s, 20 * USDC);
    let (_merchant_c, payout_c, plan_c) = merchant_with_plan(&s, 30 * USDC);
    let user_a = subscriber(&s, 1_000 * USDC);
    let user_b = subscriber(&s, 1_000 * USDC);
    let user_c = subscriber(&s, 1_000 * USDC);
    let sub_a = s.client.subscribe(&user_a, &plan_a);
    let sub_b = s.client.subscribe(&user_b, &plan_b);
    let sub_c = s.client.subscribe(&user_c, &plan_c);

    let payout_a_before = s.token.balance(&payout_a);
    let payout_c_before = s.token.balance(&payout_c);
    let fee = |amount: i128| amount * FEE_BPS as i128 / 10_000;

    mock.set_reject_transfer(&payout_b);
    advance(&s, MONTH);
    assert_eq!(
        s.client
            .charge_batch(&soroban_sdk::vec![&s.env, sub_a, sub_b, sub_c]),
        soroban_sdk::vec![&s.env, true, false, true]
    );

    // A and C charged and paid out; B skipped with refund and status untouched
    assert_eq!(s.token.balance(&user_a), 980 * USDC);
    assert_eq!(s.token.balance(&user_b), 980 * USDC);
    assert_eq!(s.token.balance(&user_c), 940 * USDC);
    assert_eq!(
        s.token.balance(&payout_a),
        payout_a_before + 10 * USDC - fee(10 * USDC)
    );
    assert_eq!(
        s.token.balance(&payout_c),
        payout_c_before + 30 * USDC - fee(30 * USDC)
    );
    assert_eq!(s.token.balance(&s.contract), 0);
}
