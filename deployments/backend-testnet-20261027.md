# Backend integration on testnet — 2026-10-27

The Ardena backend's escrow client (`app/services/stellar_escrow.py`) driving the
testnet contract `CA2INW4KOK3JF3S6UYDFIUWJQ677SEQM2WFAFSHZYS3YPKULYWVJ6BD6`.

The renter and business are **managed wallets created by the backend with 0 XLM**:
the platform treasury sponsors their reserves and is the source account of every
contract call, so it pays every fee. The renter and business sign only their
Soroban authorisation entries.

- Renter `GCUSEN3RXJWBMA7M5MYC37QDE33AHEISXZMTFUCPZTIMUI7HHOJRNZWQ`
- Business `GBFOPGCFIGURYKKEVU2XZSSFUXZ4LZ3H4MRU45YB4DPVFCM7T6D5SMLJ`
- Fee payer / admin / approver `GAYMDOWLUS3VG33WQX556N2PZ3J22MMQC6VQUCZMRA5GDZY2B7NA67QD`

The renter was funded with 10 USDC (`5e423c6adcfc818a6ea06c7c50bdc40e204e58e31f4198aade2c76e4e6b2c202`).

| Flow | Step | Transaction |
|---|---|---|
| A | lock 2 USDC, already due | `e96dca9c2a0810cfe7f1e4c9470030537eded9c63d15e065a0564da1b8a22417` |
| A | release (permissionless) | `9656c14f800d9d8fc43e21ee64d36a54549bfafceb8a6fac1b5426521046c0bc` |
| B | lock 3 USDC | `20e5467f9e7e9cef2bb284855caebd7f76892cf4b5d19ce8229be3e950eb9795` |
| B | set_release_after (admin) | `fdabd386a018e53a39b244a1f04a11866c803ec6bd47051ad835323234c25d35` |
| B | dispute (business signs) | `a41a4a1da16d90a0e35bf887e67f52bbe7ac70f2261ba234e3c20a37fb82835a` |
| B | resolve 0.9 to business (approver) | `22b59620312432fb6a84aae7c6f5255e60a8fb75cf9692733fac8c6e05f111e6` |
| C | lock 1 USDC | `48f7623a5045f6a7ca154fb8f4bcbe0bc1e0d4e0114ae5b91b5f98c43223e397` |
| C | release before due → refused `ReleaseNotDue` | — |
| C | release_early (business signs) | `bf6661110da0e1d261d517c9e59b8d04be6142b072359c3dfb257806bc167cae` |
| D | lock 1 USDC | `3fe2abd3b37f1d0d55d59d5261cfa3f16e56599c3745339429bc353afa448e08` |
| D | dispute (admin) | `677d14b944f83ffe7040ef95ebb4fda7a4d87a3beccf024503ad42228364bf9f` |
| D | resolve 0 to business | `854184a2c4dc2c05db2e234642ea4431907ed447cc6f4e9c2bb8b6f1da9f870d` |
| F | lock 1 USDC | `3b29a3df75299740b65decf92f1324ca34eaab53f16b0eb2c85462606e66ca41` |
| F | refund (admin) | `23a9429bc405adace23e495b29a597767e70b152dcb9331973adce77baad4145` |
| F | lock again with the same id → refused `EscrowExists` | — |

End balances: renter 9.1 USDC, **0 XLM**; business 0.9 USDC, **0 XLM**.

## Fee payer separate from the admin

With a different account paying fees, the admin and approver sign address
authorisation entries instead of relying on the transaction signature:

| Step | Transaction |
|---|---|
| lock 0.5 USDC | `7ff78ff9228861d731463676e80a4522cb91e2f1455c236bcf4f23712c809e67` |
| set_release_after (admin auth entry) | `e56692c838f20c7444753ea6c8b8708a4c6f04e49246f93a635b8e426ebe1587` |
| dispute (admin auth entry) | `c2134a2d5311b1fd7b5b25ef692dba0451cd02e0d80a5ec3470e2e7f17849c78` |
| resolve (approver auth entry) | `edc1623d5f8dcfcdf9eee4cb5e6d7ef355227e7004b0dff16461412762ef3c5b` |

## Through the database layer

`app/services/deposit_escrow.py`, recording each hash in `deposit_escrow_txs`:

| Step | Transaction |
|---|---|
| lock 1.25 USDC (escrow 7234937802486271860) | `3e2b8e685d93be9b29e6fa50c6fd44fe9fd893dbd04d287042d6bce430a710ef` |
| set_release_after | `8ca1e9cf5450cf9a760b7ed418e0d242f98fd700d8813846b0d320328f631df4` |
| release, then sync_from_chain → released | `0d23068205135e82209f861f947f60622998a9d8f65bd28d3241d89e32ef316a` |

## On the staging server

`scripts/escrow_smoke.py` run on the staging deployment (testing.ardena.xyz),
with two fresh 0-XLM managed wallets:

- Renter `GDG6GFRIQDQQNKFDYCRKTDVW4K5URCTZNNFJKXOI6N5NCFKW6DDUA2P3`
- Business `GAZOOY2GR4VPZMVWFGZXFNCQCTOHVGRTQHRXSKWMFMZESPRSVIKGO3PO`

| Step | Transaction |
|---|---|
| lock 2 USDC, due now | `f4d9c24a713ed78488cf72e57030f072281721a7a106648d8860152151ea8662` |
| release (permissionless) | `b3dd79c0e2c6b9a7e1d4c7f793c35bdbbffbb0f2e425f69482cc363a55450800` |
| lock 3 USDC | `6be82a3c82b9d3ae1ecd794d588f46a327332360309097a9b492ce0777dc86ed` |
| dispute (business) | `9e6199eefffe70f92bc4dc5aa0ee363c7dbc0e63991831e47e3941c185c102e4` |
| resolve 0.9 to business | `fa4b91ad08aa86a0b675c8da01b14042004b345f7e318c073b7732d346d2ccfd` |
| lock 1 USDC | `3df40dda72109ac5f1993540b09f85d43ec9215279e0f0e3dc12c4b66bbb481b` |
| refund (admin) | `1e126bee58dbaf23dffea90e0d468f69f49e70689b1bb3a283cadd11774d0a94` |

End balances: renter 9.1 USDC, **0 XLM**; business 0.9 USDC, **0 XLM**.
