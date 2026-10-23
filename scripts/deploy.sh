#!/usr/bin/env bash
# Build and deploy the deposit escrow contract.
#
#   scripts/deploy.sh testnet            # admin = approver = the ardena-admin identity
#   APPROVER=G... scripts/deploy.sh mainnet
#
# Writes the deploy log to deployments/<network>-<timestamp>.log; the log is the
# evidence of what was deployed, from which wasm hash, with which roles.
set -euo pipefail

NETWORK="${1:?usage: deploy.sh testnet|mainnet}"
IDENTITY="${IDENTITY:-ardena-admin}"
STELLAR="${STELLAR:-stellar}"

case "$NETWORK" in
  testnet) USDC_ISSUER="GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5" ;;
  mainnet) USDC_ISSUER="GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN" ;;
  *) echo "network must be testnet or mainnet" >&2; exit 1 ;;
esac

cd "$(dirname "$0")/.."
mkdir -p deployments
LOG="deployments/${NETWORK}-$(date -u +%Y%m%dT%H%M%SZ).log"
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

"$STELLAR" contract build
WASM=target/wasm32v1-none/release/deposit_escrow.wasm
WASM_HASH="$(sha256sum "$WASM" | cut -d' ' -f1)"

# The USDC Stellar Asset Contract. Deploying it is idempotent in effect: if it
# already exists the call fails and the id is still derivable.
"$STELLAR" contract asset deploy --asset "USDC:$USDC_ISSUER" --source "$IDENTITY" --network "$NETWORK" 2>/dev/null || true
TOKEN="$("$STELLAR" contract id asset --asset "USDC:$USDC_ISSUER" --network "$NETWORK")"

CONTRACT_ID="$("$STELLAR" contract deploy \
  --wasm "$WASM" \
  --source "$IDENTITY" \
  --network "$NETWORK" \
  --alias "deposit_escrow_$NETWORK" \
  -- --admin "$ADMIN" --approver "$APPROVER" --token "$TOKEN")"

echo
echo "network      $NETWORK"
echo "wasm sha256  $WASM_HASH"
echo "contract     $CONTRACT_ID"
echo "admin        $ADMIN"
echo "approver     $APPROVER"
echo "usdc token   $TOKEN"
echo "deployed at  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
