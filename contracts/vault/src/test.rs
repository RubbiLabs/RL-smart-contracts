#![cfg(test)]

extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{VaultContract, VaultContractClient};

#[test]
fn constructor_stores_roles_and_token() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let treasury = Address::generate(&env);
    let token = Address::generate(&env);

    let vault = env.register(VaultContract, (&admin, &operator, &treasury, &token));
    let client = VaultContractClient::new(&env, &vault);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.operator(), operator);
    assert_eq!(client.treasury(), treasury);
    assert_eq!(client.token(), token);
    assert_eq!(client.pending_admin(), None);
    assert!(!client.is_paused());
    assert_eq!(client.total_balances(), 0);
}
