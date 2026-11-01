#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, AuthorizedFunction, Events as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env, Event, Symbol,
};

const FEE: i128 = 20_000_000; // 2 USDC
const MAX_PAYOUT: i128 = 1_500_000_000; // 150 USDC
const POOL_FLOAT: i128 = 5_000_000_000; // 500 USDC in Ardena's waiver wallet
const NOW: u64 = 1_800_000_000;
const SETTLE_AFTER: u64 = NOW + 86_400;

struct Setup<'a> {
    env: Env,
    admin: Address,
    approver: Address,
    renter: Address,
    business: Address,
    pool: Address,
    token: TokenClient<'a>,
    waivers: WaiverPoolClient<'a>,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let admin = Address::generate(&env);
    let approver = Address::generate(&env);
    let renter = Address::generate(&env);
    let business = Address::generate(&env);
    let pool = Address::generate(&env);

    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let minter = StellarAssetClient::new(&env, &sac.address());
    minter.mint(&renter, &(FEE * 10));
    minter.mint(&business, &(FEE * 10));
    minter.mint(&pool, &POOL_FLOAT);
    let token = TokenClient::new(&env, &sac.address());

    let contract_id = env.register(
        WaiverPool,
        (&admin, &approver, &sac.address(), &pool, MAX_PAYOUT),
    );
    let waivers = WaiverPoolClient::new(&env, &contract_id);

    Setup {
        env,
        admin,
        approver,
        renter,
        business,
        pool,
        token,
        waivers,
    }
}

fn collect(s: &Setup, id: u64) {
    s.waivers
        .collect(&id, &s.renter, &s.business, &FEE, &SETTLE_AFTER);
}

fn collect_and_claim(s: &Setup, id: u64) {
    collect(s, id);
    s.waivers.claim(&id, &s.business);
}

/// The last call required `who`'s signature on `function` of `contract`.
fn assert_auth_on(s: &Setup, who: &Address, contract: &Address, function: &str) {
    let auths = s.env.auths();
    let found = auths.iter().any(|(addr, inv)| {
        addr == who
            && matches!(&inv.function, AuthorizedFunction::Contract((c, f, _))
                if c == contract && *f == Symbol::new(&s.env, function))
    });
    assert!(
        found,
        "expected {function} to require auth from {who:?}, got {auths:?}"
    );
}

fn assert_last_auth(s: &Setup, who: &Address, function: &str) {
    assert_auth_on(s, who, &s.waivers.address, function);
}

// ── constructor ───────────────────────────────────────────────────────────

#[test]
fn constructor_stores_roles_and_cap() {
    let s = setup();
    assert_eq!(s.waivers.admin(), s.admin);
    assert_eq!(s.waivers.approver(), s.approver);
    assert_eq!(s.waivers.token(), s.token.address);
    assert_eq!(s.waivers.pool(), s.pool);
    assert_eq!(s.waivers.max_payout(), MAX_PAYOUT);
}

#[test]
#[should_panic]
fn constructor_rejects_non_positive_cap() {
    let env = Env::default();
    let a = Address::generate(&env);
    env.register(WaiverPool, (&a, &a, &a, &a, 0_i128));
}

// ── collect ───────────────────────────────────────────────────────────────

#[test]
fn collect_moves_fee_into_contract() {
    let s = setup();
    collect(&s, 1);
    assert_last_auth(&s, &s.renter, "collect");

    assert_eq!(s.token.balance(&s.waivers.address), FEE);
    assert_eq!(s.token.balance(&s.renter), FEE * 9);
    assert_eq!(
        s.waivers.get(&1),
        Waiver {
            payer: s.renter.clone(),
            business: s.business.clone(),
            fee: FEE,
            settle_after: SETTLE_AFTER,
            status: Status::Held,
            paid_out: 0,
        }
    );
}

#[test]
fn collect_emits_event() {
    let s = setup();
    collect(&s, 1);
    let expected = Collected {
        waiver_id: 1,
        payer: s.renter.clone(),
        business: s.business.clone(),
        fee: FEE,
        settle_after: SETTLE_AFTER,
    };
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.waivers.address),
        [expected.to_xdr(&s.env, &s.waivers.address)]
    );
}

#[test]
fn business_can_pay_its_own_waiver() {
    // A dashboard booking: the business collected KES from the walk-in renter
    // and pays the USDC fee itself.
    let s = setup();
    s.waivers
        .collect(&1, &s.business, &s.business, &FEE, &SETTLE_AFTER);
    assert_last_auth(&s, &s.business, "collect");
    assert_eq!(s.token.balance(&s.business), FEE * 9);
    assert_eq!(s.waivers.get(&1).payer, s.business);
}

#[test]
fn collect_rejects_reused_id() {
    let s = setup();
    collect(&s, 1);
    assert_eq!(
        s.waivers
            .try_collect(&1, &s.renter, &s.business, &FEE, &SETTLE_AFTER),
        Err(Ok(Error::WaiverExists.into()))
    );
}

#[test]
fn collect_rejects_reused_id_after_settlement() {
    let s = setup();
    collect(&s, 1);
    s.env.ledger().set_timestamp(SETTLE_AFTER);
    s.waivers.settle(&1);
    assert_eq!(
        s.waivers
            .try_collect(&1, &s.renter, &s.business, &FEE, &SETTLE_AFTER),
        Err(Ok(Error::WaiverExists.into()))
    );
}

#[test]
fn collect_rejects_non_positive_fee() {
    let s = setup();
    assert_eq!(
        s.waivers
            .try_collect(&1, &s.renter, &s.business, &0, &SETTLE_AFTER),
        Err(Ok(Error::InvalidAmount.into()))
    );
    assert_eq!(
        s.waivers
            .try_collect(&1, &s.renter, &s.business, &-1, &SETTLE_AFTER),
        Err(Ok(Error::InvalidAmount.into()))
    );
}

#[test]
fn collect_fails_without_funds() {
    let s = setup();
    let broke = Address::generate(&s.env);
    assert!(s
        .waivers
        .try_collect(&1, &broke, &s.business, &FEE, &SETTLE_AFTER)
        .is_err());
    assert_eq!(s.waivers.try_get(&1), Err(Ok(Error::WaiverNotFound.into())));
}

#[test]
#[should_panic]
fn collect_requires_payer_signature() {
    let s = setup();
    s.env.set_auths(&[]);
    collect(&s, 1);
}

// ── settle ────────────────────────────────────────────────────────────────

#[test]
fn settle_sends_fee_to_pool_after_window_without_any_signature() {
    let s = setup();
    collect(&s, 1);
    s.env.ledger().set_timestamp(SETTLE_AFTER);
    s.waivers.settle(&1);
    assert!(s.env.auths().is_empty());

    assert_eq!(s.token.balance(&s.pool), POOL_FLOAT + FEE);
    assert_eq!(s.token.balance(&s.waivers.address), 0);
    assert_eq!(s.waivers.get(&1).status, Status::Settled);
}

#[test]
fn settle_before_window_fails() {
    let s = setup();
    collect(&s, 1);
    s.env.ledger().set_timestamp(SETTLE_AFTER - 1);
    assert_eq!(
        s.waivers.try_settle(&1),
        Err(Ok(Error::SettleNotDue.into()))
    );
}

#[test]
fn settle_twice_fails() {
    let s = setup();
    collect(&s, 1);
    s.env.ledger().set_timestamp(SETTLE_AFTER);
    s.waivers.settle(&1);
    assert_eq!(
        s.waivers.try_settle(&1),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
fn settle_while_claimed_fails() {
    let s = setup();
    collect_and_claim(&s, 1);
    s.env.ledger().set_timestamp(SETTLE_AFTER);
    assert_eq!(
        s.waivers.try_settle(&1),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
fn settle_follows_a_moved_pool() {
    let s = setup();
    collect(&s, 1);
    let new_pool = Address::generate(&s.env);
    s.waivers.set_pool(&new_pool);
    assert_last_auth(&s, &s.admin, "set_pool");
    s.env.ledger().set_timestamp(SETTLE_AFTER);
    s.waivers.settle(&1);
    assert_eq!(s.token.balance(&new_pool), FEE);
}

// ── refund & settle date ──────────────────────────────────────────────────

#[test]
fn refund_needs_admin_and_returns_fee_to_payer() {
    let s = setup();
    collect(&s, 1);
    s.waivers.refund(&1);
    assert_last_auth(&s, &s.admin, "refund");
    assert_eq!(s.token.balance(&s.renter), FEE * 10);
    assert_eq!(s.waivers.get(&1).status, Status::Refunded);
}

#[test]
fn refund_of_business_paid_waiver_goes_to_business() {
    let s = setup();
    s.waivers
        .collect(&1, &s.business, &s.business, &FEE, &SETTLE_AFTER);
    s.waivers.refund(&1);
    assert_eq!(s.token.balance(&s.business), FEE * 10);
}

#[test]
fn refund_of_claimed_waiver_fails() {
    let s = setup();
    collect_and_claim(&s, 1);
    assert_eq!(
        s.waivers.try_refund(&1),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
#[should_panic]
fn refund_requires_admin_signature() {
    let s = setup();
    collect(&s, 1);
    s.env.set_auths(&[]);
    s.waivers.refund(&1);
}

#[test]
fn set_settle_after_moves_window() {
    let s = setup();
    collect(&s, 1);
    let later = SETTLE_AFTER + 3 * 86_400;
    s.waivers.set_settle_after(&1, &later);
    assert_last_auth(&s, &s.admin, "set_settle_after");
    assert_eq!(s.waivers.get(&1).settle_after, later);

    s.env.ledger().set_timestamp(SETTLE_AFTER);
    assert_eq!(
        s.waivers.try_settle(&1),
        Err(Ok(Error::SettleNotDue.into()))
    );
    // The business's claim window moves with it.
    s.waivers.claim(&1, &s.business);
}

#[test]
fn set_settle_after_on_finished_waiver_fails() {
    let s = setup();
    collect(&s, 1);
    s.waivers.refund(&1);
    assert_eq!(
        s.waivers.try_set_settle_after(&1, &SETTLE_AFTER),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

// ── claim ─────────────────────────────────────────────────────────────────

#[test]
fn business_can_claim_inside_window() {
    let s = setup();
    collect(&s, 1);
    s.waivers.claim(&1, &s.business);
    assert_last_auth(&s, &s.business, "claim");
    assert_eq!(s.waivers.get(&1).status, Status::Claimed);
}

#[test]
fn business_cannot_claim_after_window() {
    let s = setup();
    collect(&s, 1);
    s.env.ledger().set_timestamp(SETTLE_AFTER);
    assert_eq!(
        s.waivers.try_claim(&1, &s.business),
        Err(Ok(Error::ClaimWindowClosed.into()))
    );
}

#[test]
fn admin_can_claim_after_window() {
    let s = setup();
    collect(&s, 1);
    s.env.ledger().set_timestamp(SETTLE_AFTER + 10);
    s.waivers.claim(&1, &s.admin);
    assert_eq!(s.waivers.get(&1).status, Status::Claimed);
}

#[test]
fn outsider_cannot_claim() {
    let s = setup();
    collect(&s, 1);
    let outsider = Address::generate(&s.env);
    assert_eq!(
        s.waivers.try_claim(&1, &outsider),
        Err(Ok(Error::NotAParty.into()))
    );
    // Neither can the renter who paid the fee.
    assert_eq!(
        s.waivers.try_claim(&1, &s.renter),
        Err(Ok(Error::NotAParty.into()))
    );
}

#[test]
fn claim_twice_fails() {
    let s = setup();
    collect_and_claim(&s, 1);
    assert_eq!(
        s.waivers.try_claim(&1, &s.business),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

// ── pay_claim ─────────────────────────────────────────────────────────────

#[test]
fn pay_claim_tops_up_from_pool() {
    let s = setup();
    collect_and_claim(&s, 1);
    let amount = 500_000_000; // 50 USDC of damage on a 2 USDC fee
    s.waivers.pay_claim(&1, &amount);
    assert_last_auth(&s, &s.approver, "pay_claim");
    // The pool signed for its own top-up.
    assert_last_auth(&s, &s.pool, "pay_claim");
    let expected = PaidOut {
        waiver_id: 1,
        business: s.business.clone(),
        amount,
        from_pool: amount - FEE,
        fee_to_pool: 0,
    };
    let events = s.env.events().all().filter_by_contract(&s.waivers.address);
    assert_eq!(events, [expected.to_xdr(&s.env, &s.waivers.address)]);

    assert_eq!(s.token.balance(&s.business), FEE * 10 + amount);
    assert_eq!(s.token.balance(&s.pool), POOL_FLOAT - (amount - FEE));
    assert_eq!(s.token.balance(&s.waivers.address), 0);
    let w = s.waivers.get(&1);
    assert_eq!(w.status, Status::PaidOut);
    assert_eq!(w.paid_out, amount);
}

#[test]
fn pay_claim_below_fee_sends_the_rest_to_pool() {
    let s = setup();
    collect_and_claim(&s, 1);
    let amount = 5_000_000; // 0.5 USDC
    s.waivers.pay_claim(&1, &amount);

    assert_eq!(s.token.balance(&s.business), FEE * 10 + amount);
    assert_eq!(s.token.balance(&s.pool), POOL_FLOAT + (FEE - amount));
    assert_eq!(s.token.balance(&s.waivers.address), 0);
    // Nothing came out of the pool, so it was never asked to sign.
    let pool_signed = s.env.auths().iter().any(|(addr, _)| *addr == s.pool);
    assert!(!pool_signed);
}

#[test]
fn pay_claim_exactly_the_fee() {
    let s = setup();
    collect_and_claim(&s, 1);
    s.waivers.pay_claim(&1, &FEE);
    assert_eq!(s.token.balance(&s.business), FEE * 11);
    assert_eq!(s.token.balance(&s.pool), POOL_FLOAT);
}

#[test]
fn pay_claim_up_to_cap() {
    let s = setup();
    collect_and_claim(&s, 1);
    s.waivers.pay_claim(&1, &MAX_PAYOUT);
    assert_eq!(s.token.balance(&s.business), FEE * 10 + MAX_PAYOUT);
}

#[test]
fn pay_claim_over_cap_fails() {
    let s = setup();
    collect_and_claim(&s, 1);
    assert_eq!(
        s.waivers.try_pay_claim(&1, &(MAX_PAYOUT + 1)),
        Err(Ok(Error::OverCap.into()))
    );
    assert_eq!(s.waivers.get(&1).status, Status::Claimed);
}

#[test]
fn pay_claim_follows_a_changed_cap() {
    let s = setup();
    collect_and_claim(&s, 1);
    s.waivers.set_max_payout(&(MAX_PAYOUT / 2));
    assert_last_auth(&s, &s.admin, "set_max_payout");
    assert_eq!(
        s.waivers.try_pay_claim(&1, &(MAX_PAYOUT / 2 + 1)),
        Err(Ok(Error::OverCap.into()))
    );
    assert_eq!(
        s.waivers.try_set_max_payout(&0),
        Err(Ok(Error::InvalidAmount.into()))
    );
}

#[test]
fn pay_claim_rejects_non_positive_amount() {
    let s = setup();
    collect_and_claim(&s, 1);
    assert_eq!(
        s.waivers.try_pay_claim(&1, &0),
        Err(Ok(Error::InvalidAmount.into()))
    );
}

#[test]
fn pay_claim_without_claim_fails() {
    let s = setup();
    collect(&s, 1);
    assert_eq!(
        s.waivers.try_pay_claim(&1, &FEE),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
fn pay_claim_fails_whole_when_pool_is_short() {
    let s = setup();
    collect_and_claim(&s, 1);
    // Drain the pool: the top-up can't be made, so nothing moves at all.
    s.token.transfer(&s.pool, &s.admin, &POOL_FLOAT);
    assert!(s.waivers.try_pay_claim(&1, &MAX_PAYOUT).is_err());
    assert_eq!(s.token.balance(&s.business), FEE * 10);
    assert_eq!(s.token.balance(&s.waivers.address), FEE);
    assert_eq!(s.waivers.get(&1).status, Status::Claimed);
}

#[test]
#[should_panic]
fn pay_claim_requires_approver_signature() {
    let s = setup();
    collect_and_claim(&s, 1);
    s.env.set_auths(&[]);
    s.waivers.pay_claim(&1, &FEE);
}

// ── reject_claim ──────────────────────────────────────────────────────────

#[test]
fn reject_claim_sends_fee_to_pool() {
    let s = setup();
    collect_and_claim(&s, 1);
    s.waivers.reject_claim(&1);
    assert_last_auth(&s, &s.approver, "reject_claim");
    assert_eq!(s.token.balance(&s.pool), POOL_FLOAT + FEE);
    assert_eq!(s.token.balance(&s.business), FEE * 10);
    assert_eq!(s.waivers.get(&1).status, Status::Rejected);
}

#[test]
fn reject_without_claim_fails() {
    let s = setup();
    collect(&s, 1);
    assert_eq!(
        s.waivers.try_reject_claim(&1),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
#[should_panic]
fn reject_claim_requires_approver_signature() {
    let s = setup();
    collect_and_claim(&s, 1);
    s.env.set_auths(&[]);
    s.waivers.reject_claim(&1);
}

// ── isolation & admin ─────────────────────────────────────────────────────

#[test]
fn waivers_are_independent() {
    let s = setup();
    collect(&s, 1);
    collect(&s, 2);
    s.waivers.claim(&1, &s.business);
    s.env.ledger().set_timestamp(SETTLE_AFTER);
    s.waivers.settle(&2);

    assert_eq!(s.waivers.get(&1).status, Status::Claimed);
    assert_eq!(s.waivers.get(&2).status, Status::Settled);
    assert_eq!(s.token.balance(&s.waivers.address), FEE);
}

#[test]
fn get_unknown_waiver_fails() {
    let s = setup();
    assert_eq!(
        s.waivers.try_get(&42),
        Err(Ok(Error::WaiverNotFound.into()))
    );
}

#[test]
fn admin_can_rotate_roles() {
    let s = setup();
    let new_admin = Address::generate(&s.env);
    let new_approver = Address::generate(&s.env);
    s.waivers.set_approver(&new_approver);
    assert_last_auth(&s, &s.admin, "set_approver");
    s.waivers.set_admin(&new_admin);
    assert_last_auth(&s, &s.admin, "set_admin");
    assert_eq!(s.waivers.admin(), new_admin);
    assert_eq!(s.waivers.approver(), new_approver);
}

#[test]
#[should_panic]
fn set_max_payout_requires_admin_signature() {
    let s = setup();
    s.env.set_auths(&[]);
    s.waivers.set_max_payout(&1);
}

#[test]
#[should_panic]
fn set_pool_requires_admin_signature() {
    let s = setup();
    s.env.set_auths(&[]);
    s.waivers.set_pool(&s.renter);
}
