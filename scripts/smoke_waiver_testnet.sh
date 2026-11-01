#!/usr/bin/env bash
# Exercise every waiver path against the deployed testnet contract with real
# testnet USDC, and log each transaction:
#
#   1. collect → settle                    (no claim, fee to the pool)
#   2. collect → claim → pay_claim         (6 USDC on a 2 USDC fee, pool tops up 4)
#   3. collect → claim → reject_claim      (fee to the pool)
#   4. collect → refund                    (booking cancelled, fee back to renter)
#   5. business pays → claim → pay_claim   (dashboard booking, 1 USDC claim, 1 USDC to pool)
#
# Testnet only. Needs scripts/deploy_waiver_pool.sh testnet to have run first.
set -euo pipefail

STELLAR="${STELLAR:-stellar}"
ADMIN_ID="${IDENTITY:-ardena-admin}"
POOL_ID="${POOL_IDENTITY:-ardena-waiver-pool}"
NET=testnet
USDC="USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5"
CONTRACT=waiver_pool_testnet
FEE=20000000 # 2 USDC
PYTHON="${PYTHON:-python3}" # needs stellar-sdk, for the two-signer pay_claim

cd "$(dirname "$0")/.."
mkdir -p deployments
LOG="deployments/smoke-waiver-testnet-$(date -u +%Y%m%dT%H%M%SZ).log"
exec > >(tee "$LOG") 2>&1

ensure_account() {
  local name="$1"
  if ! "$STELLAR" keys address "$name" >/dev/null 2>&1; then
    "$STELLAR" keys generate "$name" --network "$NET" --fund
    "$STELLAR" tx new change-trust --source "$name" --line "$USDC" --network "$NET" >/dev/null
  fi
}

invoke() {
  local source="$1"; shift
  "$STELLAR" contract invoke --id "$CONTRACT" --source "$source" --network "$NET" -- "$@"
}

buy_usdc() {
  # Swap XLM for testnet USDC on the DEX.
  local name="$1" min="$2"
  "$STELLAR" tx new path-payment-strict-send --source "$name" --network "$NET" \
    --send-asset native --send-amount 1000000000 \
    --destination "$("$STELLAR" keys address "$name")" --dest-asset "$USDC" --dest-min "$min"
}

usdc_balance() {
  "$STELLAR" contract invoke --id "$TOKEN" --source "$ADMIN_ID" --network "$NET" -- balance --id "$1"
}

ensure_account renter-test
ensure_account business-test
RENTER="$("$STELLAR" keys address renter-test)"
BUSINESS="$("$STELLAR" keys address business-test)"
POOL="$(invoke "$ADMIN_ID" pool | tr -d '"')"
TOKEN="$(invoke "$ADMIN_ID" token | tr -d '"')"
CONTRACT_ID="$("$STELLAR" contract alias show "$CONTRACT" --network "$NET")"

buy_usdc renter-test $((FEE * 4))
buy_usdc business-test $FEE
buy_usdc "$POOL_ID" 40000000

echo "pool balance before: $(usdc_balance "$POOL")"

NOW="$(date +%s)"
BASE=$((NOW * 10)) # fresh waiver ids on every run; ids are spent forever on-chain

echo "== 1. collect → settle (window already passed)"
invoke renter-test collect --waiver_id $((BASE + 1)) --payer "$RENTER" --business "$BUSINESS" \
  --fee $FEE --settle_after $((NOW - 60))
invoke "$ADMIN_ID" settle --waiver_id $((BASE + 1))
invoke "$ADMIN_ID" get --waiver_id $((BASE + 1))

echo "== 2. collect → claim → pay_claim (6 USDC, pool tops up 4)"
invoke renter-test collect --waiver_id $((BASE + 2)) --payer "$RENTER" --business "$BUSINESS" \
  --fee $FEE --settle_after $((NOW + 86400))
invoke business-test claim --waiver_id $((BASE + 2)) --by "$BUSINESS"
# Two signers (approver, and the pool for its top-up): the CLI signs only for
# its source account, so this goes through the same signing the backend does.
APPROVER_SECRET="$("$STELLAR" keys show "$ADMIN_ID")" POOL_SECRET="$("$STELLAR" keys show "$POOL_ID")" \
  "$PYTHON" -I scripts/pay_claim_testnet.py "$CONTRACT_ID" $((BASE + 2)) 60000000
invoke "$ADMIN_ID" get --waiver_id $((BASE + 2))

echo "== 3. collect → claim → reject_claim"
invoke renter-test collect --waiver_id $((BASE + 3)) --payer "$RENTER" --business "$BUSINESS" \
  --fee $FEE --settle_after $((NOW + 86400))
invoke business-test claim --waiver_id $((BASE + 3)) --by "$BUSINESS"
invoke "$ADMIN_ID" reject_claim --waiver_id $((BASE + 3))
invoke "$ADMIN_ID" get --waiver_id $((BASE + 3))

echo "== 4. collect → refund"
invoke renter-test collect --waiver_id $((BASE + 4)) --payer "$RENTER" --business "$BUSINESS" \
  --fee $FEE --settle_after $((NOW + 86400))
invoke "$ADMIN_ID" refund --waiver_id $((BASE + 4))
invoke "$ADMIN_ID" get --waiver_id $((BASE + 4))

echo "== 5. business pays (dashboard booking) → claim → pay_claim 1 USDC"
invoke business-test collect --waiver_id $((BASE + 5)) --payer "$BUSINESS" --business "$BUSINESS" \
  --fee $FEE --settle_after $((NOW + 86400))
invoke business-test claim --waiver_id $((BASE + 5)) --by "$BUSINESS"
invoke "$ADMIN_ID" pay_claim --waiver_id $((BASE + 5)) --amount 10000000
invoke "$ADMIN_ID" get --waiver_id $((BASE + 5))

echo "== over-cap claim is refused by the contract"
invoke renter-test collect --waiver_id $((BASE + 6)) --payer "$RENTER" --business "$BUSINESS" \
  --fee $FEE --settle_after $((NOW - 60))
invoke "$ADMIN_ID" claim --waiver_id $((BASE + 6)) --by "$("$STELLAR" keys address "$ADMIN_ID")"
if invoke "$ADMIN_ID" pay_claim --waiver_id $((BASE + 6)) --amount 1500000001 2>&1; then
  echo "UNEXPECTED: over-cap payout went through" >&2
  exit 1
fi
echo "refused as expected"
invoke "$ADMIN_ID" reject_claim --waiver_id $((BASE + 6))

# Pool: +2 (settle) -4 (top-up) +2 (reject) +1 (leftover fee) +2 (reject) = +3 USDC
echo "pool balance after: $(usdc_balance "$POOL")"
echo "contract balance after: $(usdc_balance "$CONTRACT_ID")"
