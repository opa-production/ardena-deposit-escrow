#!/usr/bin/env bash
# Exercise every escrow path against the deployed testnet contract with real
# testnet USDC, and log each transaction:
#
#   1. lock → release         (window already passed, released permissionlessly)
#   2. lock → dispute → resolve (business claims 30%)
#   3. lock → refund          (booking cancelled)
#
# Testnet only. Needs scripts/deploy.sh testnet to have run first.
set -euo pipefail

STELLAR="${STELLAR:-stellar}"
ADMIN_ID="${IDENTITY:-ardena-admin}"
NET=testnet
USDC="USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5"
CONTRACT=deposit_escrow_testnet
DEPOSIT=50000000 # 5 USDC

cd "$(dirname "$0")/.."
mkdir -p deployments
LOG="deployments/smoke-testnet-$(date -u +%Y%m%dT%H%M%SZ).log"
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

ensure_account renter-test
ensure_account business-test
RENTER="$("$STELLAR" keys address renter-test)"
BUSINESS="$("$STELLAR" keys address business-test)"
ADMIN="$("$STELLAR" keys address "$ADMIN_ID")"

# Swap 100 XLM for testnet USDC on the DEX so the renter can pay three deposits.
"$STELLAR" tx new path-payment-strict-send --source renter-test --network "$NET" \
  --send-asset native --send-amount 1000000000 \
  --destination "$RENTER" --dest-asset "$USDC" --dest-min $((DEPOSIT * 3))

NOW="$(date +%s)"
BASE=$((NOW * 10)) # fresh escrow ids on every run; ids are spent forever on-chain

echo "== 1. lock → release (window already passed)"
invoke renter-test lock --escrow_id $((BASE + 1)) --renter "$RENTER" --business "$BUSINESS" \
  --amount $DEPOSIT --release_after $((NOW - 60))
invoke "$ADMIN_ID" release --escrow_id $((BASE + 1))
invoke "$ADMIN_ID" get --escrow_id $((BASE + 1))

echo "== 2. lock → dispute → resolve (30% to business)"
invoke renter-test lock --escrow_id $((BASE + 2)) --renter "$RENTER" --business "$BUSINESS" \
  --amount $DEPOSIT --release_after $((NOW + 86400))
invoke business-test dispute --escrow_id $((BASE + 2)) --by "$BUSINESS"
invoke "$ADMIN_ID" resolve --escrow_id $((BASE + 2)) --to_business $((DEPOSIT * 3 / 10))
invoke "$ADMIN_ID" get --escrow_id $((BASE + 2))

echo "== 3. lock → refund (booking cancelled)"
invoke renter-test lock --escrow_id $((BASE + 3)) --renter "$RENTER" --business "$BUSINESS" \
  --amount $DEPOSIT --release_after $((NOW + 86400))
invoke "$ADMIN_ID" refund --escrow_id $((BASE + 3))
invoke "$ADMIN_ID" get --escrow_id $((BASE + 3))

echo
echo "renter    $RENTER"
echo "business  $BUSINESS"
echo "admin     $ADMIN"
