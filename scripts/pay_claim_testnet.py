"""Submit one waiver `pay_claim` signed by both the approver and the pool.

The stellar CLI signs only for its source account, but a claim larger than the
fee needs the pool's signature too (it tops the payout up). This does what the
Ardena backend does: simulate, sign every auth entry the simulation asks for,
re-simulate, submit, wait.

    APPROVER_SECRET=S... POOL_SECRET=S... \
      python pay_claim_testnet.py <contract_id> <waiver_id> <amount_units>

Testnet only. Needs stellar-sdk.
"""
import os
import sys

from stellar_sdk import Keypair, Network, SorobanServer, TransactionBuilder, scval
from stellar_sdk import xdr as stellar_xdr
from stellar_sdk.address import Address
from stellar_sdk.auth import authorize_entry
from stellar_sdk.soroban_rpc import GetTransactionStatus

RPC = "https://soroban-testnet.stellar.org"
PASSPHRASE = Network.TESTNET_NETWORK_PASSPHRASE


def main(contract_id: str, waiver_id: int, amount: int) -> None:
    approver = Keypair.from_secret(os.environ["APPROVER_SECRET"])
    pool = Keypair.from_secret(os.environ["POOL_SECRET"])
    signers = {kp.public_key: kp for kp in (approver, pool)}
    server = SorobanServer(RPC)

    def build(auth=None):
        return (
            TransactionBuilder(server.load_account(approver.public_key), PASSPHRASE, base_fee=1_000)
            .append_invoke_contract_function_op(
                contract_id,
                "pay_claim",
                [scval.to_uint64(waiver_id), scval.to_int128(amount)],
                auth=auth,
            )
            .set_timeout(120)
            .build()
        )

    sim = server.simulate_transaction(build())
    if sim.error:
        sys.exit(f"simulation failed: {sim.error}")
    signed = []
    for raw in sim.results[0].auth or []:
        entry = stellar_xdr.SorobanAuthorizationEntry.from_xdr(raw)
        creds = entry.credentials
        if creds.type == stellar_xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS:
            who = Address.from_xdr_sc_address(creds.address.address).address
            print(f"auth entry signed by {who}")
            entry = authorize_entry(entry, signers[who], sim.latest_ledger + 100, PASSPHRASE)
        else:
            print("auth entry covered by the source account")
        signed.append(entry)

    tx = server.prepare_transaction(build(auth=signed))
    tx.sign(approver)
    sent = server.send_transaction(tx)
    result = server.poll_transaction(sent.hash)
    print(f"pay_claim {result.status.value}: {sent.hash}")
    if result.status != GetTransactionStatus.SUCCESS:
        sys.exit(1)


if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]), int(sys.argv[3]))
