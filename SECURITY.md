# Security policy

Rubbi moves real money. The vault holds users' USDC, the subscriptions contract
pulls from users' allowances, the payroll contract holds employers' escrow, and
the backend signs with operator, keeper and treasury keys. Please treat any
weakness in these paths as sensitive.

## Reporting a vulnerability

**Do not open a public issue or pull request for a security problem.**

Report it privately through
[GitHub Security Advisories](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
on this repository ("Security" tab → "Report a vulnerability"). If you cannot
use that form, contact the maintainers directly:

- Security contact: **TODO(maintainers): add an email address or other private channel**

Include what you found, where (repository, file, function, deployed contract
id), how to reproduce it, and what you think the impact is. A proof of concept
against **testnet** is welcome. Never test against mainnet deployments or
other people's funds.

## What to expect

| Step | Target |
|---|---|
| Acknowledgement | within 3 working days |
| Initial assessment and severity | within 7 days |
| Fix or mitigation plan | depends on severity; critical issues first |
| Public disclosure | coordinated with you after a fix is deployed |

We will keep you informed and credit you in the fix (unless you prefer not to
be named). There is no bug bounty programme at the moment.

## Scope

In scope:

- the Soroban contracts in this repository (`vault`, `subscriptions`, `payroll`)
  and their deployed testnet instances listed in the README;
- the deploy and smoke scripts under `scripts/`;
- the Rubbi backend ([RL-api](https://github.com/RubbiLabs/RL-api)) and web app
  ([RL-frontend](https://github.com/RubbiLabs/RL-frontend)): report issues in
  those repositories, or here if you are unsure.

Out of scope: the Stellar network, Soroban host or SDKs themselves (report to
the [Stellar Development Foundation](https://stellar.org/security)), Circle's
USDC, third-party wallets, and the card issuer's own systems.

## Good practice for contributors

- Never commit secrets or private keys. Only `.env.example` files with
  placeholders belong in the repository.
- Testnet role keys live in the Stellar CLI keystore (`stellar keys ...`), not
  in files.
- Report a leaked key immediately so it can be rotated (`set_operator`,
  `set_treasury`, admin transfer).
