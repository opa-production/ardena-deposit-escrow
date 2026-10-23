#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, AuthorizedFunction, Events as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env, Event, Symbol,
};

const DEPOSIT: i128 = 75_000_000; // 7.5 USDC
const NOW: u64 = 1_800_000_000;
const RELEASE_AFTER: u64 = NOW + 86_400;

struct Setup<'a> {
    env: Env,
    admin: Address,
    approver: Address,
    renter: Address,
    business: Address,
    token: TokenClient<'a>,
    escrow: DepositEscrowClient<'a>,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let admin = Address::generate(&env);
    let approver = Address::generate(&env);
    let renter = Address::generate(&env);
    let business = Address::generate(&env);

    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    StellarAssetClient::new(&env, &sac.address()).mint(&renter, &(DEPOSIT * 10));
    let token = TokenClient::new(&env, &sac.address());

    let contract_id = env.register(DepositEscrow, (&admin, &approver, &sac.address()));
    let escrow = DepositEscrowClient::new(&env, &contract_id);

    Setup {
        env,
        admin,
        approver,
        renter,
        business,
        token,
        escrow,
    }
}

fn lock(s: &Setup, id: u64) {
    s.escrow
        .lock(&id, &s.renter, &s.business, &DEPOSIT, &RELEASE_AFTER);
}

/// The last call required `who`'s signature on `function` of the escrow contract.
fn assert_last_auth(s: &Setup, who: &Address, function: &str) {
    let auths = s.env.auths();
    let found = auths.iter().any(|(addr, inv)| {
        addr == who
            && matches!(&inv.function, AuthorizedFunction::Contract((c, f, _))
                if *c == s.escrow.address && *f == Symbol::new(&s.env, function))
    });
    assert!(
        found,
        "expected {function} to require auth from {who:?}, got {auths:?}"
    );
}

// ── lock ──────────────────────────────────────────────────────────────────

#[test]
fn lock_moves_deposit_into_escrow() {
    let s = setup();
    lock(&s, 1);
    assert_last_auth(&s, &s.renter, "lock");

    assert_eq!(s.token.balance(&s.escrow.address), DEPOSIT);
    assert_eq!(s.token.balance(&s.renter), DEPOSIT * 9);
    assert_eq!(
        s.escrow.get(&1),
        Escrow {
            renter: s.renter.clone(),
            business: s.business.clone(),
            amount: DEPOSIT,
            release_after: RELEASE_AFTER,
            status: Status::Locked,
            to_business: 0,
        }
    );
}

#[test]
fn lock_emits_event() {
    let s = setup();
    lock(&s, 1);
    let expected = Locked {
        escrow_id: 1,
        renter: s.renter.clone(),
        business: s.business.clone(),
        amount: DEPOSIT,
        release_after: RELEASE_AFTER,
    };
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.escrow.address),
        [expected.to_xdr(&s.env, &s.escrow.address)]
    );
}

#[test]
fn lock_rejects_reused_id() {
    let s = setup();
    lock(&s, 1);
    let res = s
        .escrow
        .try_lock(&1, &s.renter, &s.business, &DEPOSIT, &RELEASE_AFTER);
    assert_eq!(res, Err(Ok(Error::EscrowExists.into())));
}

#[test]
fn lock_rejects_reused_id_after_payout() {
    let s = setup();
    lock(&s, 1);
    s.env.ledger().set_timestamp(RELEASE_AFTER);
    s.escrow.release(&1);
    let res = s
        .escrow
        .try_lock(&1, &s.renter, &s.business, &DEPOSIT, &RELEASE_AFTER);
    assert_eq!(res, Err(Ok(Error::EscrowExists.into())));
}

#[test]
fn lock_rejects_non_positive_amount() {
    let s = setup();
    assert_eq!(
        s.escrow
            .try_lock(&1, &s.renter, &s.business, &0, &RELEASE_AFTER),
        Err(Ok(Error::InvalidAmount.into()))
    );
    assert_eq!(
        s.escrow
            .try_lock(&1, &s.renter, &s.business, &-5, &RELEASE_AFTER),
        Err(Ok(Error::InvalidAmount.into()))
    );
}

#[test]
fn lock_rejects_renter_as_business() {
    let s = setup();
    let res = s
        .escrow
        .try_lock(&1, &s.renter, &s.renter, &DEPOSIT, &RELEASE_AFTER);
    assert_eq!(res, Err(Ok(Error::SameParty.into())));
}

#[test]
fn lock_fails_without_funds() {
    let s = setup();
    let res = s
        .escrow
        .try_lock(&1, &s.renter, &s.business, &(DEPOSIT * 11), &RELEASE_AFTER);
    assert!(res.is_err());
    assert_eq!(s.token.balance(&s.renter), DEPOSIT * 10);
}

#[test]
#[should_panic]
fn lock_requires_renter_signature() {
    let s = setup();
    s.env.set_auths(&[]);
    lock(&s, 1);
}

#[test]
fn get_unknown_escrow_fails() {
    let s = setup();
    assert_eq!(s.escrow.try_get(&42), Err(Ok(Error::EscrowNotFound.into())));
}

// ── release ───────────────────────────────────────────────────────────────

#[test]
fn release_pays_renter_after_window_without_any_signature() {
    let s = setup();
    lock(&s, 1);
    s.env.ledger().set_timestamp(RELEASE_AFTER);
    s.env.set_auths(&[]); // permissionless: no one signs
    s.escrow.release(&1);

    assert_eq!(s.token.balance(&s.renter), DEPOSIT * 10);
    assert_eq!(s.token.balance(&s.escrow.address), 0);
    assert_eq!(s.escrow.get(&1).status, Status::Released);
}

#[test]
fn release_before_window_fails() {
    let s = setup();
    lock(&s, 1);
    s.env.ledger().set_timestamp(RELEASE_AFTER - 1);
    assert_eq!(
        s.escrow.try_release(&1),
        Err(Ok(Error::ReleaseNotDue.into()))
    );
}

#[test]
fn release_twice_fails() {
    let s = setup();
    lock(&s, 1);
    s.env.ledger().set_timestamp(RELEASE_AFTER);
    s.escrow.release(&1);
    assert_eq!(
        s.escrow.try_release(&1),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
fn release_while_disputed_fails() {
    let s = setup();
    lock(&s, 1);
    s.escrow.dispute(&1, &s.business);
    s.env.ledger().set_timestamp(RELEASE_AFTER + 10 * 86_400);
    assert_eq!(
        s.escrow.try_release(&1),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
fn release_early_needs_business_and_pays_renter() {
    let s = setup();
    lock(&s, 1);
    s.escrow.release_early(&1);
    assert_last_auth(&s, &s.business, "release_early");
    assert_eq!(s.token.balance(&s.renter), DEPOSIT * 10);
    assert_eq!(s.escrow.get(&1).status, Status::Released);
}

// ── refund ────────────────────────────────────────────────────────────────

#[test]
fn refund_needs_admin_and_returns_deposit() {
    let s = setup();
    lock(&s, 1);
    s.escrow.refund(&1);
    assert_last_auth(&s, &s.admin, "refund");
    assert_eq!(s.token.balance(&s.renter), DEPOSIT * 10);
    assert_eq!(s.escrow.get(&1).status, Status::Refunded);
}

#[test]
fn refund_of_disputed_deposit_fails() {
    let s = setup();
    lock(&s, 1);
    s.escrow.dispute(&1, &s.business);
    assert_eq!(
        s.escrow.try_refund(&1),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

// ── set_release_after ─────────────────────────────────────────────────────

#[test]
fn set_release_after_moves_window() {
    let s = setup();
    lock(&s, 1);
    let later = RELEASE_AFTER + 3 * 86_400;
    s.escrow.set_release_after(&1, &later);
    assert_last_auth(&s, &s.admin, "set_release_after");
    assert_eq!(s.escrow.get(&1).release_after, later);

    s.env.ledger().set_timestamp(RELEASE_AFTER);
    assert_eq!(
        s.escrow.try_release(&1),
        Err(Ok(Error::ReleaseNotDue.into()))
    );
    s.env.ledger().set_timestamp(later);
    s.escrow.release(&1);
}

#[test]
fn set_release_after_on_settled_escrow_fails() {
    let s = setup();
    lock(&s, 1);
    s.escrow.refund(&1);
    assert_eq!(
        s.escrow.try_set_release_after(&1, &NOW),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

// ── dispute ───────────────────────────────────────────────────────────────

#[test]
fn business_can_dispute_inside_window() {
    let s = setup();
    lock(&s, 1);
    s.escrow.dispute(&1, &s.business);
    assert_last_auth(&s, &s.business, "dispute");
    assert_eq!(s.escrow.get(&1).status, Status::Disputed);
}

#[test]
fn business_cannot_dispute_after_window() {
    let s = setup();
    lock(&s, 1);
    s.env.ledger().set_timestamp(RELEASE_AFTER);
    assert_eq!(
        s.escrow.try_dispute(&1, &s.business),
        Err(Ok(Error::DisputeWindowClosed.into()))
    );
}

#[test]
fn admin_can_dispute_after_window() {
    let s = setup();
    lock(&s, 1);
    s.env.ledger().set_timestamp(RELEASE_AFTER + 1);
    s.escrow.dispute(&1, &s.admin);
    assert_eq!(s.escrow.get(&1).status, Status::Disputed);
}

#[test]
fn outsider_cannot_dispute() {
    let s = setup();
    lock(&s, 1);
    let outsider = Address::generate(&s.env);
    assert_eq!(
        s.escrow.try_dispute(&1, &outsider),
        Err(Ok(Error::NotAParty.into()))
    );
    // Neither can the renter freeze their own deposit.
    assert_eq!(
        s.escrow.try_dispute(&1, &s.renter),
        Err(Ok(Error::NotAParty.into()))
    );
}

#[test]
fn dispute_twice_fails() {
    let s = setup();
    lock(&s, 1);
    s.escrow.dispute(&1, &s.business);
    assert_eq!(
        s.escrow.try_dispute(&1, &s.business),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

// ── resolve ───────────────────────────────────────────────────────────────

#[test]
fn resolve_splits_between_business_and_renter() {
    let s = setup();
    lock(&s, 1);
    s.escrow.dispute(&1, &s.business);
    let claim = 30_000_000;
    s.escrow.resolve(&1, &claim);
    assert_last_auth(&s, &s.approver, "resolve");

    assert_eq!(s.token.balance(&s.business), claim);
    assert_eq!(s.token.balance(&s.renter), DEPOSIT * 9 + (DEPOSIT - claim));
    assert_eq!(s.token.balance(&s.escrow.address), 0);
    let e = s.escrow.get(&1);
    assert_eq!(e.status, Status::Resolved);
    assert_eq!(e.to_business, claim);
}

#[test]
fn resolve_full_forfeit_and_full_rejection() {
    let s = setup();
    lock(&s, 1);
    lock(&s, 2);
    s.escrow.dispute(&1, &s.business);
    s.escrow.dispute(&2, &s.business);

    s.escrow.resolve(&1, &DEPOSIT); // claim approved in full
    s.escrow.resolve(&2, &0); // claim rejected

    assert_eq!(s.token.balance(&s.business), DEPOSIT);
    assert_eq!(s.token.balance(&s.renter), DEPOSIT * 9);
}

#[test]
fn resolve_rejects_out_of_range_split() {
    let s = setup();
    lock(&s, 1);
    s.escrow.dispute(&1, &s.business);
    assert_eq!(
        s.escrow.try_resolve(&1, &(DEPOSIT + 1)),
        Err(Ok(Error::InvalidSplit.into()))
    );
    assert_eq!(
        s.escrow.try_resolve(&1, &-1),
        Err(Ok(Error::InvalidSplit.into()))
    );
}

#[test]
fn resolve_without_dispute_fails() {
    let s = setup();
    lock(&s, 1);
    assert_eq!(
        s.escrow.try_resolve(&1, &0),
        Err(Ok(Error::InvalidStatus.into()))
    );
}

#[test]
#[should_panic]
fn resolve_requires_approver_signature() {
    let s = setup();
    lock(&s, 1);
    s.escrow.dispute(&1, &s.business);
    s.env.set_auths(&[]);
    s.escrow.resolve(&1, &0);
}

// ── isolation & admin ─────────────────────────────────────────────────────

#[test]
fn escrows_are_independent() {
    let s = setup();
    lock(&s, 1);
    lock(&s, 2);
    s.escrow.dispute(&1, &s.business);
    s.env.ledger().set_timestamp(RELEASE_AFTER);
    s.escrow.release(&2);

    assert_eq!(s.escrow.get(&1).status, Status::Disputed);
    assert_eq!(s.escrow.get(&2).status, Status::Released);
    assert_eq!(s.token.balance(&s.escrow.address), DEPOSIT);
}

#[test]
fn admin_can_rotate_roles() {
    let s = setup();
    let new_admin = Address::generate(&s.env);
    let new_approver = Address::generate(&s.env);
    s.escrow.set_approver(&new_approver);
    assert_last_auth(&s, &s.admin, "set_approver");
    s.escrow.set_admin(&new_admin);
    assert_last_auth(&s, &s.admin, "set_admin");
    assert_eq!(s.escrow.admin(), new_admin);
    assert_eq!(s.escrow.approver(), new_approver);
    assert_eq!(s.escrow.token(), s.token.address);
}
