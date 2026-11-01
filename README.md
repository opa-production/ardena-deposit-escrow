# Ardena Deposit Escrow

[![CI](https://github.com/opa-production/ardena-deposit-escrow/actions/workflows/ci.yml/badge.svg)](https://github.com/opa-production/ardena-deposit-escrow/actions/workflows/ci.yml)

Soroban smart contracts that settle rental money in USDC on Stellar, built for
[Ardena](https://ardena.co.ke) and Ardena Business under the Stellar Instawards
programme:

- [`contracts/deposit_escrow`](contracts/deposit_escrow): security deposits
  (Deliverable 1: Security Deposit Escrow on Stellar).
- [`contracts/waiver_pool`](contracts/waiver_pool): damage-waiver fees and
  claim payouts (Deliverable 2), described [below](#damage-waiver-pool).

## How it works

One contract holds every booking's deposit, keyed by an escrow id that the Ardena
backend assigns.

```text
  lock ──► Locked ──release (after release_after, anyone)──► Released
             │    ──release_early (business waives claim)──► Released
             │    ──refund (admin, cancelled booking)──────► Refunded
             └──dispute (business or admin)──► Disputed ──resolve (approver)──► Resolved
```

| Function | Who signs | What it does |
|---|---|---|
| `lock(escrow_id, renter, business, amount, release_after)` | renter | Moves the deposit into the contract |
| `release(escrow_id)` | **anyone** | Returns the full deposit once `release_after` has passed with no dispute |
| `release_early(escrow_id)` | business | Business waives any claim; deposit returns now |
| `refund(escrow_id)` | admin | Booking cancelled; deposit returns in full |
| `set_release_after(escrow_id, ts)` | admin | Moves the release date (trip extended or ended early) |
| `dispute(escrow_id, by)` | business (inside the window) or admin | Freezes the deposit for a decision |
| `resolve(escrow_id, to_business)` | approver | Pays `to_business` to the business and the rest to the renter |
| `get(escrow_id)` | — | Read the escrow; renter and business can both check it |

### What a renter can verify without trusting Ardena

- Locked money can only leave to the renter, or to the business through an
  approver's `resolve`. No function pays any other address.
- Once `release_after` passes without a dispute, anyone can call `release`, so a
  refund does not depend on Ardena's backend.
- A business can only open a dispute before `release_after`.
- An escrow id can be used once, ever, so a replayed `lock` cannot take a second deposit.

### What the admin and approver keys can do

The contract trusts two keys, set at deployment. Here is their full power:

- **Admin** (Ardena operations): refund a locked deposit to the renter, move a
  deposit's release date, open a dispute, rotate the admin and approver keys, and
  **upgrade the contract code**.
- **Approver** (dispute decisions): split a disputed deposit between business and renter.

`upgrade` exists so a bug found on mainnet can be fixed without stranding locked
deposits in a broken contract. It also means the admin could, in principle,
replace the rules above. Every upgrade is a public on-chain transaction that
changes the contract's WASM hash, so anyone can see it happen and compare the new
code against this repository.

On testnet both roles share one key. On mainnet the admin and approver are
separate keys, held offline and used only through Ardena's backend signer, and
the deployed WASM hash is published below so it can be checked against a build
of this repository.

Every state change emits an event (`locked`, `released`, `refunded`, `disputed`,
`resolved`, `release_after_set`), which Ardena's reconciliation reads back.

## Damage waiver pool

A renter who buys the damage waiver pays a small fee at booking. For an Ardena
Business dashboard booking, the business pays it from its wallet after
collecting the renter's payment. The fee is held in the contract against the
booking. If no claim is made it settles to Ardena's waiver wallet (the *pool*).
If the business claims for damage and Ardena approves, the business is paid,
from the held fee first and the rest from the pool, up to a per-claim cap.

```text
  collect ──► Held ──settle (after settle_after, anyone)──► Settled   fee → pool
               │  ──refund (admin, cancelled booking)──────► Refunded  fee → payer
               └──claim (business or admin)──► Claimed ──pay_claim (approver)──► PaidOut
                                                       ──reject_claim (approver)─► Rejected  fee → pool
```

| Function | Who signs | What it does |
|---|---|---|
| `collect(waiver_id, payer, business, fee, settle_after)` | payer | Moves the fee into the contract |
| `settle(waiver_id)` | **anyone** | Sends an unclaimed fee to the pool once `settle_after` has passed |
| `refund(waiver_id)` | admin | Booking cancelled; the fee returns to whoever paid it |
| `set_settle_after(waiver_id, ts)` | admin | Moves the settle date (trip extended or ended early) |
| `claim(waiver_id, by)` | business (inside the window) or admin | Holds the fee for a damage decision |
| `pay_claim(waiver_id, amount)` | approver, plus the pool when `amount` exceeds the fee | Pays the business `amount` (≤ `max_payout`); leftover fee goes to the pool |
| `reject_claim(waiver_id)` | approver | Claim turned down; the fee goes to the pool |
| `set_max_payout(units)` / `set_pool(address)` | admin | Changes the per-claim cap or the pool wallet |
| `get(waiver_id)` | — | Read the waiver |

What a business or renter can check on-chain:

- A held fee can only leave to the pool, back to the payer, or to the business
  through an approver's `pay_claim`.
- No single claim pays more than `max_payout`, whatever the backend asks for.
- `pay_claim` is all or nothing: if the pool can't cover the top-up, nothing moves.
- Once `settle_after` passes with no claim, anyone can call `settle`.
- A business can only claim before `settle_after`.

The admin and approver keys have the same powers here as in the escrow (above),
plus setting the cap and the pool address; both changes emit events
(`max_payout_set`, `pool_set`). The cap starts at 150 USDC (KES 20,000).

## Build and test

Needs Rust ≥ 1.84 with the `wasm32v1-none` target, and the
[Stellar CLI](https://developers.stellar.org/docs/tools/cli).

```sh
cargo test                     # 74 unit tests (30 escrow, 44 waiver)
cargo llvm-cov --summary-only  # coverage
stellar contract build         # target/wasm32v1-none/release/{deposit_escrow,waiver_pool}.wasm
```

## Deploy

```sh
scripts/deploy.sh testnet          # deploys, logs to deployments/
scripts/smoke_testnet.sh           # runs every escrow path on testnet with real testnet USDC

scripts/deploy_waiver_pool.sh testnet   # deploys the waiver pool, logs to deployments/
PYTHON=path/to/python-with-stellar-sdk scripts/smoke_waiver_testnet.sh
```

## Deployments

### Testnet

| | |
|---|---|
| Contract | [`CA2INW4KOK3JF3S6UYDFIUWJQ677SEQM2WFAFSHZYS3YPKULYWVJ6BD6`](https://stellar.expert/explorer/testnet/contract/CA2INW4KOK3JF3S6UYDFIUWJQ677SEQM2WFAFSHZYS3YPKULYWVJ6BD6) |
| WASM sha256 | `4ac9adf2be33455ca7160010a645e386142f54949cfdcefca074f48eac18b33b` |
| USDC token | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` |
| Deploy log | [`deployments/testnet-20261023T230835Z.log`](deployments/testnet-20261023T230835Z.log) |

Smoke run ([log](deployments/smoke-testnet-20261023T230930Z.log)):

| Path | Transaction |
|---|---|
| lock → release | [lock](https://stellar.expert/explorer/testnet/tx/0fec20296ecf9c57e986d339b3a38d3e2840b63bdb0373d93da375c13fe05236) · [release](https://stellar.expert/explorer/testnet/tx/f545011818a6810e4f9deb7aca4aa438f2d0fbb63428b6996fe88fa86fe96aaa) |
| lock → dispute → resolve (30% to business) | [lock](https://stellar.expert/explorer/testnet/tx/861bfbb85e3568cc77c619b7911d1de3f998d85fb77d36585e4367c384d578dd) · [dispute](https://stellar.expert/explorer/testnet/tx/e511f84c39a6f61f78a952d4cf637628e42807ac64abe80f1cdab9c0d5594199) · [resolve](https://stellar.expert/explorer/testnet/tx/d239115c6967e2b8be8000f7cddf7b39f19e0ec989db67f9eefd3e3802591f0f) |
| lock → refund | [lock](https://stellar.expert/explorer/testnet/tx/d9eb655cc3b18096411e9cabcc9d5b6e103fea997db341538f113afabdc3281d) · [refund](https://stellar.expert/explorer/testnet/tx/0a22db2a1510322caeb3ef42a070c5d8f916be3ba40bd0cabe8c57fdafd0d994) |

Waiver pool:

| | |
|---|---|
| Contract | [`CCTPRGEPD6HEIQBMMGCAWLQJ5QYTZ6G3SH67LUGJKYBBNSVN3BGSJLQI`](https://stellar.expert/explorer/testnet/contract/CCTPRGEPD6HEIQBMMGCAWLQJ5QYTZ6G3SH67LUGJKYBBNSVN3BGSJLQI) |
| WASM sha256 | `9eeaacb7b61fada292062847ef11efae8847345df900ac98b727431a8b2b6a20` |
| Pool | [`GDVJ7JYLBASJZ7HN753MMMDHEKGLSDZUPNWJEX2HVVASY4I3EHUEH7I3`](https://stellar.expert/explorer/testnet/account/GDVJ7JYLBASJZ7HN753MMMDHEKGLSDZUPNWJEX2HVVASY4I3EHUEH7I3) |
| Max payout | 150 USDC |
| Deploy log | [`deployments/waiver-pool-testnet-20261101T194710Z.log`](deployments/waiver-pool-testnet-20261101T194710Z.log) |

Smoke run ([log](deployments/smoke-waiver-testnet-20261101T194853Z.log)); the pool ended 3 USDC up, the contract empty:

| Path | Transaction |
|---|---|
| collect → settle | [collect](https://stellar.expert/explorer/testnet/tx/9f6bcc00d04b8705e2a3b7c7c0a026f63eb7b72b5e5c11909a55c293aefaaa4f) · [settle](https://stellar.expert/explorer/testnet/tx/9b2da80462e226acfc13776f890ea9c67b9fa827240d80208aa922f1b3865285) |
| collect → claim → pay_claim (6 USDC on a 2 USDC fee, pool tops up 4) | [collect](https://stellar.expert/explorer/testnet/tx/16644f6b4f3858adb3fd18f4442d0eff51953b75b8f259acf0d40bbd208b05f4) · [claim](https://stellar.expert/explorer/testnet/tx/5c82b7bc973d6addb0cc320d78e6eb16b0eb263f5d70a85e44f8c7adeac0e4b7) · [pay_claim](https://stellar.expert/explorer/testnet/tx/a809907e2c92903ac92a0d6d175940eea30243455a2d6932502d187984e28c28) |
| collect → claim → reject_claim | [collect](https://stellar.expert/explorer/testnet/tx/5b6839dc8266f6acd9c2480221d363c412296e8bb20efe9d2ea8c1c9faccf202) · [claim](https://stellar.expert/explorer/testnet/tx/2dbc027657f5014f333170eb625ccfa08e5d2b3b2685bf6854cc0dcb95c99ae9) · [reject_claim](https://stellar.expert/explorer/testnet/tx/cae83b50800c306f2b5f4edacbe05d7d151d947498725a37334a52e4d3443dc7) |
| collect → refund | [collect](https://stellar.expert/explorer/testnet/tx/cad1b0cc71b23a56c24777c6bd75570cedb5e93cde00876997bc7cf69defc541) · [refund](https://stellar.expert/explorer/testnet/tx/e5e0c4d257d200c6128d26bd5dc50469181280990c16d3963930eee1800cd56c) |
| business pays (dashboard booking) → claim → pay_claim 1 USDC | [collect](https://stellar.expert/explorer/testnet/tx/8136b49eae1637655e282ed9d26715c4d6fd3590c87b9b2e61cd9e6d72ac28d7) · [claim](https://stellar.expert/explorer/testnet/tx/57fe32721feb2f23d9795ebcec78160fc0a7562fe92f28c41a912c588a979042) · [pay_claim](https://stellar.expert/explorer/testnet/tx/f6662dc58a2267a200a8660a5355182a36bfb94422ac0f3acb8f2de64c6ec44e) |
| over-cap payout (150.0000001 USDC) | refused in simulation with `OverCap` (#8); nothing submitted |

### Mainnet

Not yet deployed.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
Copyright 2026 Ardena Platforms Africa Ltd.
