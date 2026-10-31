# Testnet deployment

> **Stellar testnet only. Not audited.** The shared instance moves only
> testnet USDC from Circle's faucet. Its activity comes from dedicated demo
> accounts, not real users. Testnet is reset from time to time; a reset
> would remove these contracts and they would be redeployed with new ids.

[`deployments/testnet.json`](../deployments/testnet.json) is the source of
truth for the ids, hashes and roles below, and
[`deployments/testnet-demo.json`](../deployments/testnet-demo.json) for the
demo transactions. The entry points and events are documented in
[`CONTRACT_INTERFACE.md`](CONTRACT_INTERFACE.md).

## Network

| | |
|---|---|
| Network | Stellar testnet |
| Passphrase | `Test SDF Network ; September 2015` |
| RPC | `https://soroban-testnet.stellar.org` |
| USDC issuer (Circle, testnet) | [`GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5`](https://stellar.expert/explorer/testnet/account/GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5) |
| USDC Stellar Asset Contract | [`CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA`](https://stellar.expert/explorer/testnet/contract/CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA) |

## Contracts

| Contract | Contract ID | Release tag | WASM file | SHA-256 | Explorer |
|---|---|---|---|---|---|
| vault | `CBGARSQ7BY4345DNX76LWT4VXEHRRFJMNW46SCX7DRHBILL5V6RJN6KR` | pending first attested release | pending | `6b32de5ccff8f0fc28096e4e5b7ce9b16a1cbc997f4d0fa0c561aef4ec28e387` (local build, not attested) | [Stellar Expert](https://stellar.expert/explorer/testnet/contract/CBGARSQ7BY4345DNX76LWT4VXEHRRFJMNW46SCX7DRHBILL5V6RJN6KR) · [Stellar Lab](https://lab.stellar.org/r/testnet/contract/CBGARSQ7BY4345DNX76LWT4VXEHRRFJMNW46SCX7DRHBILL5V6RJN6KR) |
| subscriptions | `CDWMLKAUMXHLSBOKKCK5A4ECUP2VIIHXAL7MVQ5TEFLE3QUXZRMDC3PC` | pending first attested release | pending | `e194e739736bc1d562e247088210b34209fdfeceed580d4514a226116feecc44` (local build, not attested) | [Stellar Expert](https://stellar.expert/explorer/testnet/contract/CDWMLKAUMXHLSBOKKCK5A4ECUP2VIIHXAL7MVQ5TEFLE3QUXZRMDC3PC) · [Stellar Lab](https://lab.stellar.org/r/testnet/contract/CDWMLKAUMXHLSBOKKCK5A4ECUP2VIIHXAL7MVQ5TEFLE3QUXZRMDC3PC) |
| payroll | `CB76PQ2MTILAQJAHLMTRHYDODX7H73YUCTVVKYO46ME3MK6WUTELDGYU` | pending first attested release | pending | `ee0d2b3d64c09bb0a123f4ca60c05ea5e92ca7a265e0b225460a0b0202f6cd50` (local build, not attested) | [Stellar Expert](https://stellar.expert/explorer/testnet/contract/CB76PQ2MTILAQJAHLMTRHYDODX7H73YUCTVVKYO46ME3MK6WUTELDGYU) · [Stellar Lab](https://lab.stellar.org/r/testnet/contract/CB76PQ2MTILAQJAHLMTRHYDODX7H73YUCTVVKYO46ME3MK6WUTELDGYU) |

The contracts currently run the code deployed on 2026-10-27 by
`scripts/deploy-testnet.sh` from commit
[`2136695`](https://github.com/RubbiLabs/RL-smart-contracts/commit/2136695771f247cb18082bf2348dc8cd37e765d2),
built locally with `stellar contract build`. That build is reproducible (a
plain `stellar contract build` of the same source gives the same three
hashes), but it is not an attested release, so the explorers do not show the
source as verified yet. The first attested release will be installed in
place with `scripts/upgrade-testnet.sh`: the contract ids stay the same,
storage is kept, and only the code hash changes (see
[Release and upgrade](#release-and-upgrade-maintainers)).

Deployment transactions (each signed by the admin):

| Contract | Upload WASM | Create contract |
|---|---|---|
| vault | [`f218533a…`](https://stellar.expert/explorer/testnet/tx/f218533ab5b57cbe282797a2f214ee312b70706bd602839acc0515c426ea9fdd) | [`abfc7a79…`](https://stellar.expert/explorer/testnet/tx/abfc7a79c80edc4c7d220bfecdad149cd24d6511d6b57134b7d2e7b039e0aada) |
| subscriptions | [`a8f9ea88…`](https://stellar.expert/explorer/testnet/tx/a8f9ea88057e1c5730b1184a2e0750004811f0ebe77f105059656e92d155ee83) | [`d0e0645d…`](https://stellar.expert/explorer/testnet/tx/d0e0645d44d45cf8ea38231cb8717dbef5bf81c5982c23b32f7313badcf1edd2) |
| payroll | [`f5e43555…`](https://stellar.expert/explorer/testnet/tx/f5e43555f476096347084edede7af4a867cedfbc1e4ab2a17a9d913819ebf0ff) | [`95c3352a…`](https://stellar.expert/explorer/testnet/tx/95c3352a2fad51e568df348ef76937be68ce40916f608321f33456e6a9dbb26b) |

Constructor arguments: vault `(admin, operator, treasury, token = USDC SAC)`;
subscriptions and payroll `(admin, token = USDC SAC, fee_recipient, fee_bps =
50)`.

### Roles

Public addresses only. The keys are held by the maintainers in the stellar
CLI keystore and are never committed.

| Role | CLI identity | Address |
|---|---|---|
| admin (all three contracts) | `rubbi-admin` | [`GCW4BXKRNKDICUSCI3RVP3MYTPQWMZ7ABCGAF5CZIWZ22U6TBIRDYE6G`](https://stellar.expert/explorer/testnet/account/GCW4BXKRNKDICUSCI3RVP3MYTPQWMZ7ABCGAF5CZIWZ22U6TBIRDYE6G) |
| vault operator (signs card loads) | `rubbi-operator` | [`GASCJCM7CLBWKKJ7UKXISRX67YRQD2EVL57YKPRCCTTECEZQYRJEPNXN`](https://stellar.expert/explorer/testnet/account/GASCJCM7CLBWKKJ7UKXISRX67YRQD2EVL57YKPRCCTTECEZQYRJEPNXN) |
| vault treasury (receives loads, funds unloads) | `rubbi-treasury` | [`GDRCO5PHUXNGQSM5CB752AWDCYID2VCCCUNF2QVA3GAWV7UB6QMEB4T5`](https://stellar.expert/explorer/testnet/account/GDRCO5PHUXNGQSM5CB752AWDCYID2VCCCUNF2QVA3GAWV7UB6QMEB4T5) |
| keeper (calls `charge_batch` / `disburse_batch`) | `rubbi-keeper` | [`GAL5BE6C7TTJXOZ2OIZXDTJX6K56ZOBUGEFOF3GE2OQUXACSA34N3WSQ`](https://stellar.expert/explorer/testnet/account/GAL5BE6C7TTJXOZ2OIZXDTJX6K56ZOBUGEFOF3GE2OQUXACSA34N3WSQ) |
| fee recipient (subscriptions, payroll) | `rubbi-fees` | [`GCW2OS4LNJZAK2EUQR7GFCBL7U57UDKHPOFFUONBHGJKUX24AUF7ZZIC`](https://stellar.expert/explorer/testnet/account/GCW2OS4LNJZAK2EUQR7GFCBL7U57UDKHPOFFUONBHGJKUX24AUF7ZZIC) |

The keeper is not stored in any contract (charging and disbursing are
permissionless); it is the backend's account for those calls.

Demo accounts used by `scripts/demo-testnet.sh`:
[`demo-user`](https://stellar.expert/explorer/testnet/account/GBVURBT5SGUBGKB7YP27LLDPPQHN5NUUG3MLJESWSMR6ANCNISPWT4EJ)
(depositor, subscriber and demo employer),
[`demo-merchant`](https://stellar.expert/explorer/testnet/account/GDKM57Q3MDNHE7ZTZBT36LER7S7PE3KWBX656RJ2GUAJ5UBXFAYIMQUS)
and
[`demo-staff`](https://stellar.expert/explorer/testnet/account/GCVZT3EXYTNHU377WX4NR7FWTVVVPRDJS6Y6BAIBOB7UYUEFLL6GYRXJ)
(payroll recipient).

## Demo transactions

One row per function exercised by `scripts/demo-testnet.sh`, from
`deployments/testnet-demo.json`.

**Day 1**: run `20261028T032451Z-81569`, 2026-10-28 03:24:51–03:26:50 UTC.
This run predates hash recording in the script, so its hashes were
recovered from Horizon (the operations of the demo and role accounts in that
window, each decoded to its contract and function) and backfilled into
`testnet-demo.json`. All eleven transactions succeeded.

| Step | Contract | Function | Signed by | Transaction |
|---|---|---|---|---|
| 1 | vault | `deposit` (5 USDC) | demo-user | [`b06b879b…`](https://stellar.expert/explorer/testnet/tx/b06b879b89492bcef6264891508945ca359f46b47854df73b58f4cabada2659a) |
| 2 | vault | `withdraw` (1 USDC) | demo-user | [`7b9c2f0a…`](https://stellar.expert/explorer/testnet/tx/7b9c2f0aabbf6fb5129b424556b883863b592dabde97e42d13bb5cf2847b120d) |
| 3 | vault | `settle_card_spend` (2 USDC card load) | operator | [`2e449413…`](https://stellar.expert/explorer/testnet/tx/2e449413432723ced5903c16105ab8315b2d6e115fd041de5a8676410d538ba2) |
| 4 | vault | `credit_refund` (1 USDC card unload) | treasury | [`7de7066a…`](https://stellar.expert/explorer/testnet/tx/7de7066a8030ec778456f5853238fca87f425585d88df3429089d1de1298b336) |
| 5 | subscriptions | `add_merchant` | admin | [`9edb23fe…`](https://stellar.expert/explorer/testnet/tx/9edb23fe9bca8e580ad6bc076344c78f21a5c050032753d1c802bf7025945073) |
| 6 | subscriptions | `create_plan` (plan 1, 1 USDC / day) | demo-merchant | [`31c3ba16…`](https://stellar.expert/explorer/testnet/tx/31c3ba1687e0a765f6a332db27595eff09a46127580d6c613199c02daa6cc59e) |
| 7 | USDC SAC | `approve` (spender = subscriptions) | demo-user | [`da3d69c7…`](https://stellar.expert/explorer/testnet/tx/da3d69c7fb8c3e566a746549de65563b09b9ebc34ab6c74cafb6f97781b0df85) |
| 8 | subscriptions | `subscribe` (subscription 1, first period charged) | demo-user | [`758615ce…`](https://stellar.expert/explorer/testnet/tx/758615ce719e222f52138c1da4f61e32bc92db3867ccc536a608f34e42221329) |
| 9 | payroll | `fund` (3 USDC escrow) | demo-user | [`a1901714…`](https://stellar.expert/explorer/testnet/tx/a1901714214eb3fdfa8005afb2d57769cb344f34cd76543702adc978611e8e7d) |
| 10 | payroll | `create_schedules` (schedule 1, 1 USDC / day) | demo-user | [`b702ac7b…`](https://stellar.expert/explorer/testnet/tx/b702ac7bae65b3bf106208df396ed1d73c59c0749f0a4994764c4172e012958d) |
| 11 | payroll | `disburse` (first period: 0.995 USDC to demo-staff, 0.005 fee) | demo-user | [`0561d7c1…`](https://stellar.expert/explorer/testnet/tx/0561d7c1d952d3f1c5ccd3db528ff734892f2f12a2a4736a36c316e06bb0c242) |

**Day 2** (`scripts/demo-testnet.sh --day2`): charges the subscription's
second period and pays the schedule's second period.

| Step | Contract | Function | Signed by | Transaction |
|---|---|---|---|---|
| 12 | subscriptions | `charge` (subscription 1, second period) | demo-user (permissionless call) | **PENDING: Day 2 not run yet.** Fill in from `last_run.transactions` after the run. |
| 13 | payroll | `disburse` (schedule 1, second period) | demo-user (permissionless call) | **PENDING: Day 2 not run yet.** Fill in from `last_run.transactions` after the run. |

## Reading the deployment

Read-only calls are simulated, not submitted, so they are free. Any funded
testnet account works as the source:

```sh
stellar keys generate reader --network testnet --fund   # once

VAULT=CBGARSQ7BY4345DNX76LWT4VXEHRRFJMNW46SCX7DRHBILL5V6RJN6KR
SUBS=CDWMLKAUMXHLSBOKKCK5A4ECUP2VIIHXAL7MVQ5TEFLE3QUXZRMDC3PC
PAYROLL=CB76PQ2MTILAQJAHLMTRHYDODX7H73YUCTVVKYO46ME3MK6WUTELDGYU
DEMO_USER=GBVURBT5SGUBGKB7YP27LLDPPQHN5NUUG3MLJESWSMR6ANCNISPWT4EJ
call() { id="$1"; shift; stellar contract invoke --id "$id" --source-account reader --network testnet --send=no -- "$@"; }

call "$VAULT" admin
call "$VAULT" is_paused
call "$VAULT" balance --user "$DEMO_USER"
call "$VAULT" total_balances
call "$SUBS" fee_bps
call "$SUBS" get_plan --plan_id 1
call "$SUBS" get_subscription --sub_id 1
call "$PAYROLL" get_schedule --schedule_id 1
call "$PAYROLL" escrow_of --employer "$DEMO_USER"
```

Expected results (checked on 2026-10-31; balances and due dates move as the
demo and the app are used):

| Call | Result |
|---|---|
| `admin` | `"GCW4BXKRNKDICUSCI3RVP3MYTPQWMZ7ABCGAF5CZIWZ22U6TBIRDYE6G"` (same for all three contracts) |
| `is_paused` | `false` |
| `balance --user $DEMO_USER` | `"30000000"` (3 USDC; amounts are `i128` strings with 7 decimals) |
| `total_balances` | `"80000000"` (all users); equals the vault's USDC balance unless someone transfers USDC to it directly |
| `fee_bps` | `50` (0.5 %, also on payroll) |
| `get_plan --plan_id 1` | `{"active":true,"amount":"10000000","id":1,"merchant":"GDKM57Q3…","name":"Demo plan","period":86400}` |
| `get_subscription --sub_id 1` | `"status":"Active"`, `"next_due":1793244355` (2026-10-29 03:25:55 UTC; advances by one day per charge) |
| `get_schedule --schedule_id 1` | `"status":"Active"`, `"label":"Demo salary"`, `"next_due":1793244397` (advances by one day per disbursement) |
| `escrow_of --employer $DEMO_USER` | `"20000000"` (3 USDC funded, 1 USDC disbursed) |

An error such as `Error(Contract, #3)` is a contract error code; look it up
in the README's [error tables](../README.md#error-codes).

## TypeScript bindings

Generate typed clients from the deployed contracts (the spec is read from
the network):

```sh
stellar contract bindings typescript --network testnet \
  --contract-id CBGARSQ7BY4345DNX76LWT4VXEHRRFJMNW46SCX7DRHBILL5V6RJN6KR --output-dir packages/rubbi-vault
stellar contract bindings typescript --network testnet \
  --contract-id CDWMLKAUMXHLSBOKKCK5A4ECUP2VIIHXAL7MVQ5TEFLE3QUXZRMDC3PC --output-dir packages/rubbi-subscriptions
stellar contract bindings typescript --network testnet \
  --contract-id CB76PQ2MTILAQJAHLMTRHYDODX7H73YUCTVVKYO46ME3MK6WUTELDGYU --output-dir packages/rubbi-payroll
# then, in each package: npm install && npm run build
```

Each package has a client with one method per entry point, the contract's
types (`Plan`, `Subscription`, `Schedule`, …), the `Errors` map and the
testnet network settings. An upgrade keeps the interface identical, so
bindings stay valid across the planned release upgrade.

## Run your own instance

The shared instance's role keys belong to the maintainers. To exercise
admin, operator or treasury functions, deploy your own copy, where you hold
every role. In your own clone:

```sh
# five identities, funded by Friendbot
for name in rubbi-admin rubbi-operator rubbi-treasury rubbi-keeper rubbi-fees; do
  stellar keys generate "$name" --network testnet --fund
done

scripts/deploy-testnet.sh --dry-run   # print every command, deploy nothing
scripts/deploy-testnet.sh --force     # deployments/testnet.json already exists:
                                      # --force keeps it as testnet.<timestamp>.json
scripts/smoke-testnet.sh              # read-only checks against your deployment
scripts/demo-testnet.sh               # day 1 demo flow on your contracts
scripts/demo-testnet.sh --day2        # a day later
```

`demo-testnet.sh` stops and prints a faucet link while `demo-user` holds
less than 15 USDC; get testnet USDC from [Circle's faucet](https://faucet.circle.com)
(Stellar testnet). Do not commit your `deployments/*.json` back to this
repository. The [Deploy to testnet](../README.md#deploy-to-testnet) section
of the README explains every step.

## Release and upgrade (maintainers)

Contract changes reach the shared instance as attested releases, installed
in place with the contracts' admin-only `upgrade(new_wasm_hash)`. Contract
ids and storage are kept.

1. **Bump the version.** In the pull request with the contract change, set
   the same new version in `contracts/vault/Cargo.toml`,
   `contracts/subscriptions/Cargo.toml` and `contracts/payroll/Cargo.toml`
   (`cargo build` updates `Cargo.lock`). All three currently are `0.1.0`.
   If storage types or keys change, the PR must say how existing entries
   are read after the upgrade; the `wasm-upgrade-test` CI step exercises
   the upgrade path.
2. **Merge the PR** to `main` once CI is green.
3. **Tag the tip of `main`** with `v<version>`:
   `git tag -a v0.1.0 -m "v0.1.0" && git push origin v0.1.0`.
   The tag must equal `v` + the crate version, or the release names get a
   `_pkg<version>` suffix; tag the current tip of `main`, because the
   release workflow creates its per-package tags from there.
4. **Release workflow.** `.github/workflows/release.yml` builds each
   contract with `stellar contract build --optimize` (stellar-cli 28.1.0),
   embeds `source_repo=github:RubbiLabs/RL-smart-contracts`, publishes the
   releases `v<version>_rubbi-vault_cli28.1.0`,
   `v<version>_rubbi-subscriptions_cli28.1.0` and
   `v<version>_rubbi-payroll_cli28.1.0` (assets
   `rubbi-<contract>_v<version>.wasm`), sends each build to Stellar Expert
   for validation and creates a build attestation. Wait for all three jobs.
5. **Upgrade:** `scripts/upgrade-testnet.sh v<version> --dry-run`, then
   `scripts/upgrade-testnet.sh v<version>`. It downloads the three assets,
   prints their SHA-256, verifies the attestations, uploads each WASM,
   calls `upgrade` signed by `rubbi-admin`, checks each contract's on-chain
   WASM against its asset, and records tag, files, hashes and transaction
   hashes in `deployments/testnet.json`. Then run `scripts/smoke-testnet.sh`.
6. **Confirm on both explorers** that each contract shows its source as
   verified and points to this repository: the Stellar Expert and Stellar
   Lab contract pages linked in the table above.
7. **Update the docs:** this file (the contracts table, and move the
   replaced hashes to *Previous deployments*) and the README's
   [Deployed contract addresses](../README.md#deployed-contract-addresses).
   Commit `deployments/testnet.json` with them. Only mark a contract as
   verified after step 6.

## Previous deployments

None yet: the ids above are the first Rubbi deployment on Stellar
(2026-10-27). After the first upgrade, the code it replaces is listed here:

| Contract | WASM SHA-256 | Source | Status |
|---|---|---|---|
| vault | `6b32de5ccff8f0fc28096e4e5b7ce9b16a1cbc997f4d0fa0c561aef4ec28e387` | commit `2136695`, local `stellar contract build` | current |
| subscriptions | `e194e739736bc1d562e247088210b34209fdfeceed580d4514a226116feecc44` | commit `2136695`, local `stellar contract build` | current |
| payroll | `ee0d2b3d64c09bb0a123f4ca60c05ea5e92ca7a265e0b225460a0b0202f6cd50` | commit `2136695`, local `stellar contract build` | current |

The earlier Solidity contracts on Arbitrum are kept for reference under
[`legacy/evm/`](../legacy/evm); they are not part of this deployment.
