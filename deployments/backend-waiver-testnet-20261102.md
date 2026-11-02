# Backend integration: damage-waiver pool on testnet (2026-11-02)

The Ardena backend (`opabackend`, commit `abbf189`, `scripts/waiver_smoke.py`)
ran every waiver path against contract
[`CCTPRGEPD6HEIQBMMGCAWLQJ5QYTZ6G3SH67LUGJKYBBNSVN3BGSJLQI`](https://stellar.expert/explorer/testnet/contract/CCTPRGEPD6HEIQBMMGCAWLQJ5QYTZ6G3SH67LUGJKYBBNSVN3BGSJLQI)
with two fresh managed wallets created at **0 XLM**. Ardena's treasury paid
every transaction fee; the renter and business only signed their Soroban
authorisation entries. Ardena's waiver wallet (the pool) signed the one call
that drew on it.

| | |
|---|---|
| Renter | [`GDHUHTE22A7XV3YZ43WBEN7O6D5ECKVNOQ3T6T4VTHYJT53LIBAZEVGB`](https://stellar.expert/explorer/testnet/account/GDHUHTE22A7XV3YZ43WBEN7O6D5ECKVNOQ3T6T4VTHYJT53LIBAZEVGB) |
| Business | [`GCRDOFTBBE4UM3SFHTHEWLMXSIV56WSSDEWWDKZQ3Q5G6WZIT3GMFTQP`](https://stellar.expert/explorer/testnet/account/GCRDOFTBBE4UM3SFHTHEWLMXSIV56WSSDEWWDKZQ3Q5G6WZIT3GMFTQP) |
| Pool | [`GDVJ7JYLBASJZ7HN753MMMDHEKGLSDZUPNWJEX2HVVASY4I3EHUEH7I3`](https://stellar.expert/explorer/testnet/account/GDVJ7JYLBASJZ7HN753MMMDHEKGLSDZUPNWJEX2HVVASY4I3EHUEH7I3) |
| Max payout (read from the contract) | 150 USDC |

The renter was funded with 5 USDC. Each fee is 0.5 USDC.

| Step | Transaction |
|---|---|
| collect 0.5 (renter), already due | [`1eba8ba7…`](https://stellar.expert/explorer/testnet/tx/1eba8ba7336a2fd0a7b922b7e3332198d73e6fb1647dc9bae60472a8db365dd4) |
| settle (permissionless) → pool | [`58b94ac2…`](https://stellar.expert/explorer/testnet/tx/58b94ac2e0b88d15977ce7d3b0e03ae4e79db4c5bdac6b0b4ee22df4cf43108a) |
| collect 0.5 (renter) | [`f1c87f37…`](https://stellar.expert/explorer/testnet/tx/f1c87f375e531bfcfa2560a042e2ffe4f7a63dfbff6a3e665b21bb830c272e8f) |
| claim (business) | [`122b5592…`](https://stellar.expert/explorer/testnet/tx/122b5592b5e683a91c03f0d0587f0e83e44b22e39a04d358181a55805d80993c) |
| pay_claim 3 USDC (pool tops up 2.5) | [`8018c247…`](https://stellar.expert/explorer/testnet/tx/8018c2473579f5c2c9f3b5c290422355c454ca857ed0f10f66a443baf18f7ff7) |
| collect 0.5 (renter) | [`6bb8fb04…`](https://stellar.expert/explorer/testnet/tx/6bb8fb04dfc819d38fb4fe8e6861018ec84b76a12e0415ec0fc0475500cf2da2) |
| claim (business) | [`2df43466…`](https://stellar.expert/explorer/testnet/tx/2df43466f3462cb2069a07d73b0f65fedbb598956d9f601cebf4cad68ac879a9) |
| reject_claim → pool | [`762e59ab…`](https://stellar.expert/explorer/testnet/tx/762e59ab273f3f44cdf011ffa75bcff851a5551ee12ea9cc1623aa960b93ec28) |
| collect 0.5 (renter) | [`063a242b…`](https://stellar.expert/explorer/testnet/tx/063a242b672eb2a411ded24e7b51177bcb724d1c533a00298b471f83525e21fb) |
| refund (admin) → renter | [`2bdd475d…`](https://stellar.expert/explorer/testnet/tx/2bdd475d36c20f7013ec171043f780672d7b74e40a1c3d2b0cc2a14fc4c4ee4d) |
| collect 0.5 (business pays: dashboard booking) | [`3167dcb9…`](https://stellar.expert/explorer/testnet/tx/3167dcb9f9487e90b5d50bb2c591720c6ba3b1ddb4d888d5cecc6f18ed281fa4) |
| claim (business) | [`7b0e80c2…`](https://stellar.expert/explorer/testnet/tx/7b0e80c278eaa03162e62feba71fa50343a85746d47763f25d17b954add804d7) |
| pay_claim 0.2 USDC (0.3 of the fee → pool) | [`498fe4b4…`](https://stellar.expert/explorer/testnet/tx/498fe4b4de435d03c745248265de95956000c1371e5b6cc1142895d0d021c3ad) |

Ending balances, all as expected:

| Wallet | USDC | XLM |
|---|---|---|
| Renter | 3.5 (5 − three kept fees) | 0 |
| Business | 2.7 (+3 claim, −0.5 own fee, +0.2 claim) | 0 |
| Pool | −1.2 (+0.5 settle, −2.5 top-up, +0.5 reject, +0.3 leftover) | unchanged |
