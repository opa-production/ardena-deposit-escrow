#!/usr/bin/env bash
# Build and deploy the damage-waiver pool contract.
#
#   scripts/deploy_waiver_pool.sh testnet
#       admin = approver = the ardena-admin identity; the pool is the
#       ardena-waiver-pool identity, created and given a USDC trustline if missing
#   APPROVER=G... POOL=G... scripts/deploy_waiver_pool.sh mainnet
#
# MAX_PAYOUT caps one claim, in USDC base units (7 decimals). The default,
# 150 USDC, is KES 20,000 at the backend's USDC_PER_KSH of 0.0075; the admin
# can change it later with set_max_payout.
#
# Writes the deploy log to deployments/waiver-pool-<network>-<timestamp>.log.
set -euo pipefail

NETWORK="${1:?usage: deploy_waiver_pool.sh testnet|mainnet}"
IDENTITY="${IDENTITY:-ardena-admin}"
POOL_IDENTITY="${POOL_IDENTITY:-ardena-waiver-pool}"
MAX_PAYOUT="${MAX_PAYOUT:-1500000000}"
STELLAR="${STELLAR:-stellar}"

case "$NETWORK" in
  testnet) USDC_ISSUER="GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5" ;;
  mainnet) USDC_ISSUER="GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN" ;;
  *) echo "network must be testnet or mainnet" >&2; exit 1 ;;
esac

cd "$(dirname "$0")/.."
mkdir -p deployments
LOG="deployments/waiver-pool-${NETWORK}-$(date -u +%Y%m%dT%H%M%SZ).log"
exec > >(tee "$LOG") 2>&1

if ! "$STELLAR" keys address "$IDENTITY" >/dev/null 2>&1; then
  if [ "$NETWORK" = mainnet ]; then
    echo "identity $IDENTITY does not exist; import the funded mainnet admin key first" >&2
    exit 1
  fi
  "$STELLAR" keys generate "$IDENTITY" --network testnet --fund
fi
ADMIN="$("$STELLAR" keys address "$IDENTITY")"
APPROVER="${APPROVER:-$ADMIN}"

if [ -z "${POOL:-}" ]; then
  if [ "$NETWORK" = mainnet ]; then
    echo "set POOL to Ardena's mainnet waiver wallet (it needs a USDC trustline)" >&2
    exit 1
  fi
  if ! "$STELLAR" keys address "$POOL_IDENTITY" >/dev/null 2>&1; then
    "$STELLAR" keys generate "$POOL_IDENTITY" --network testnet --fund
    "$STELLAR" tx new change-trust --source "$POOL_IDENTITY" --line "USDC:$USDC_ISSUER" \
      --network testnet >/dev/null
  fi
  POOL="$("$STELLAR" keys address "$POOL_IDENTITY")"
fi

"$STELLAR" contract build --package waiver_pool
WASM=target/wasm32v1-none/release/waiver_pool.wasm
WASM_HASH="$(sha256sum "$WASM" | cut -d' ' -f1)"

"$STELLAR" contract asset deploy --asset "USDC:$USDC_ISSUER" --source "$IDENTITY" --network "$NETWORK" 2>/dev/null || true
TOKEN="$("$STELLAR" contract id asset --asset "USDC:$USDC_ISSUER" --network "$NETWORK")"

CONTRACT_ID="$("$STELLAR" contract deploy \
  --wasm "$WASM" \
  --source "$IDENTITY" \
  --network "$NETWORK" \
  --alias "waiver_pool_$NETWORK" \
  -- --admin "$ADMIN" --approver "$APPROVER" --token "$TOKEN" \
     --pool "$POOL" --max_payout "$MAX_PAYOUT")"

echo
echo "network      $NETWORK"
echo "wasm sha256  $WASM_HASH"
echo "contract     $CONTRACT_ID"
echo "admin        $ADMIN"
echo "approver     $APPROVER"
echo "pool         $POOL"
echo "max payout   $MAX_PAYOUT"
echo "usdc token   $TOKEN"
echo "deployed at  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
