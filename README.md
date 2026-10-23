# Ardena Deposit Escrow

[![CI](https://github.com/opa-production/ardena-deposit-escrow/actions/workflows/ci.yml/badge.svg)](https://github.com/opa-production/ardena-deposit-escrow/actions/workflows/ci.yml)

A Soroban smart contract that holds rental security deposits in USDC on Stellar.
Built for [Ardena](https://ardena.co.ke) and Ardena Business under the Stellar
Instawards programme (Deliverable 1: Security Deposit Escrow on Stellar).

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

## Build and test

Needs Rust ≥ 1.84 with the `wasm32v1-none` target, and the
[Stellar CLI](https://developers.stellar.org/docs/tools/cli).

```sh
cargo test                     # 30 unit tests
cargo llvm-cov --summary-only  # coverage
stellar contract build         # target/wasm32v1-none/release/deposit_escrow.wasm
```

## Deploy

```sh
scripts/deploy.sh testnet          # deploys, logs to deployments/
scripts/smoke_testnet.sh           # runs every escrow path on testnet with real testnet USDC
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

### Mainnet

Not yet deployed.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
Copyright 2026 Ardena Platforms Africa Ltd.
