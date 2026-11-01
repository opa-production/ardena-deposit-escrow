//! Ardena damage-waiver fees and claim payouts.
//!
//! A renter (or, for a dashboard booking, the business on the renter's behalf)
//! pays a small waiver fee at booking. The fee is held here against the
//! booking, keyed by a waiver id the Ardena backend assigns, and moves through:
//!
//! ```text
//!   collect ──► Held ──settle (after settle_after, anyone)──► Settled   fee → pool
//!                │  ──refund (admin, cancelled booking)──────► Refunded  fee → payer
//!                └──claim (business or admin)──► Claimed ──pay_claim (approver)──► PaidOut
//!                                                        ──reject_claim (approver)─► Rejected  fee → pool
//! ```
//!
//! The pool is Ardena's waiver wallet. Settled fees go there, and it tops up
//! approved claims that cost more than the fee: `pay_claim` pays the business
//! the held fee first and the rest from the pool, in one transaction, so a
//! claim is either paid in full or not at all.
//!
//! The guarantees a business or renter can check on-chain:
//! - A held fee can only leave to the pool, back to the payer, or to the
//!   business through an approver's `pay_claim`. No function pays anyone else.
//! - No claim pays more than `max_payout`, whatever the backend asks for.
//! - Once `settle_after` passes with no claim, *anyone* can call `settle`.
//! - A business can only claim before `settle_after`.
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, BytesN, ContractExecutable, Env,
};

// Ledgers close roughly every 5 seconds.
const DAY_IN_LEDGERS: u32 = 17_280;
// Same lifetimes as the deposit escrow: a waiver lives about as long as a
// deposit (the trip plus its inspection window), so keep its record alive for
// 120 days after every write and re-extend once fewer than 60 days remain.
const WAIVER_TTL_THRESHOLD: u32 = 60 * DAY_IN_LEDGERS;
const WAIVER_TTL_EXTEND_TO: u32 = 120 * DAY_IN_LEDGERS;
const INSTANCE_TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_TTL_EXTEND_TO: u32 = 90 * DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    WaiverExists = 1,
    WaiverNotFound = 2,
    InvalidAmount = 3,
    InvalidStatus = 4,
    SettleNotDue = 5,
    ClaimWindowClosed = 6,
    NotAParty = 7,
    OverCap = 8,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Held = 0,
    Claimed = 1,
    Settled = 2,
    PaidOut = 3,
    Rejected = 4,
    Refunded = 5,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Waiver {
    /// Who paid the fee: the renter, or the business for a dashboard booking.
    /// A refund goes back here.
    pub payer: Address,
    /// The business the waiver covers; an approved claim pays it.
    pub business: Address,
    /// Token base units (USDC has 7 decimals on Stellar).
    pub fee: i128,
    /// Unix seconds after which an unclaimed fee can be settled by anyone.
    pub settle_after: u64,
    pub status: Status,
    /// Set by `pay_claim`: what the business received in total.
    pub paid_out: i128,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Approver,
    Token,
    Pool,
    MaxPayout,
    Waiver(u64),
}

#[contractevent(topics = ["collected"])]
pub struct Collected {
    #[topic]
    pub waiver_id: u64,
    pub payer: Address,
    pub business: Address,
    pub fee: i128,
    pub settle_after: u64,
}

#[contractevent(topics = ["settled"])]
pub struct Settled {
    #[topic]
    pub waiver_id: u64,
    pub pool: Address,
    pub fee: i128,
}

#[contractevent(topics = ["refunded"])]
pub struct Refunded {
    #[topic]
    pub waiver_id: u64,
    pub payer: Address,
    pub fee: i128,
}

#[contractevent(topics = ["claimed"])]
pub struct Claimed {
    #[topic]
    pub waiver_id: u64,
    pub by: Address,
}

#[contractevent(topics = ["paid_out"])]
pub struct PaidOut {
    #[topic]
    pub waiver_id: u64,
    pub business: Address,
    pub amount: i128,
    /// The part of `amount` that came from the pool rather than the held fee.
    pub from_pool: i128,
    /// The part of the fee left over after the payout, sent to the pool.
    pub fee_to_pool: i128,
}

#[contractevent(topics = ["rejected"])]
pub struct Rejected {
    #[topic]
    pub waiver_id: u64,
    pub pool: Address,
    pub fee: i128,
}

#[contractevent(topics = ["settle_after_set"])]
pub struct SettleAfterSet {
    #[topic]
    pub waiver_id: u64,
    pub settle_after: u64,
}

#[contractevent(topics = ["max_payout_set"])]
pub struct MaxPayoutSet {
    pub max_payout: i128,
}

#[contractevent(topics = ["pool_set"])]
pub struct PoolSet {
    pub pool: Address,
}

#[contract]
pub struct WaiverPool;

#[contractimpl]
impl WaiverPool {
    /// `admin` runs the booking lifecycle (refunds, settle dates, the cap and
    /// pool); `approver` decides claims. `pool` is Ardena's waiver wallet.
    /// `max_payout` caps a single claim, in token base units.
    pub fn __constructor(
        env: Env,
        admin: Address,
        approver: Address,
        token: Address,
        pool: Address,
        max_payout: i128,
    ) {
        if max_payout <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Approver, &approver);
        storage.set(&DataKey::Token, &token);
        storage.set(&DataKey::Pool, &pool);
        storage.set(&DataKey::MaxPayout, &max_payout);
    }

    /// Move the waiver fee from `payer` into the contract for waiver `waiver_id`.
    pub fn collect(
        env: Env,
        waiver_id: u64,
        payer: Address,
        business: Address,
        fee: i128,
        settle_after: u64,
    ) {
        payer.require_auth();
        if fee <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let key = DataKey::Waiver(waiver_id);
        // An id is spent forever once used, so a replayed or duplicated
        // collect can never take a second fee.
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, Error::WaiverExists);
        }

        token_client(&env).transfer(&payer, &env.current_contract_address(), &fee);

        let waiver = Waiver {
            payer: payer.clone(),
            business: business.clone(),
            fee,
            settle_after,
            status: Status::Held,
            paid_out: 0,
        };
        save(&env, waiver_id, &waiver);
        Collected {
            waiver_id,
            payer,
            business,
            fee,
            settle_after,
        }
        .publish(&env);
    }

    /// Send an unclaimed fee to the pool once its claim window has passed.
    /// Callable by anyone.
    pub fn settle(env: Env, waiver_id: u64) {
        let mut waiver = load(&env, waiver_id);
        require_status(&env, &waiver, Status::Held);
        if env.ledger().timestamp() < waiver.settle_after {
            panic_with_error!(&env, Error::SettleNotDue);
        }
        let pool = pool(&env);
        token_client(&env).transfer(&env.current_contract_address(), &pool, &waiver.fee);
        waiver.status = Status::Settled;
        save(&env, waiver_id, &waiver);
        Settled {
            waiver_id,
            pool,
            fee: waiver.fee,
        }
        .publish(&env);
    }

    /// Booking cancelled before the trip: the fee goes back to whoever paid it.
    pub fn refund(env: Env, waiver_id: u64) {
        admin(&env).require_auth();
        let mut waiver = load(&env, waiver_id);
        require_status(&env, &waiver, Status::Held);
        token_client(&env).transfer(&env.current_contract_address(), &waiver.payer, &waiver.fee);
        waiver.status = Status::Refunded;
        save(&env, waiver_id, &waiver);
        Refunded {
            waiver_id,
            payer: waiver.payer.clone(),
            fee: waiver.fee,
        }
        .publish(&env);
    }

    /// Move the settle date, e.g. when a trip is extended or ends early.
    pub fn set_settle_after(env: Env, waiver_id: u64, settle_after: u64) {
        admin(&env).require_auth();
        let mut waiver = load(&env, waiver_id);
        require_status(&env, &waiver, Status::Held);
        waiver.settle_after = settle_after;
        save(&env, waiver_id, &waiver);
        SettleAfterSet {
            waiver_id,
            settle_after,
        }
        .publish(&env);
    }

    /// Hold the fee until the approver decides a damage claim. The business
    /// may only claim before `settle_after`; the admin may at any time while held.
    pub fn claim(env: Env, waiver_id: u64, by: Address) {
        by.require_auth();
        let mut waiver = load(&env, waiver_id);
        require_status(&env, &waiver, Status::Held);
        if by == waiver.business {
            if env.ledger().timestamp() >= waiver.settle_after {
                panic_with_error!(&env, Error::ClaimWindowClosed);
            }
        } else if by != admin(&env) {
            panic_with_error!(&env, Error::NotAParty);
        }
        waiver.status = Status::Claimed;
        save(&env, waiver_id, &waiver);
        Claimed { waiver_id, by }.publish(&env);
    }

    /// Pay an approved claim: `amount` to the business, from the held fee
    /// first and the rest from the pool. Any fee left over goes to the pool.
    /// The pool must sign the call when it tops up.
    pub fn pay_claim(env: Env, waiver_id: u64, amount: i128) {
        approver(&env).require_auth();
        let mut waiver = load(&env, waiver_id);
        require_status(&env, &waiver, Status::Claimed);
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if amount > max_payout(&env) {
            panic_with_error!(&env, Error::OverCap);
        }

        let token = token_client(&env);
        let contract = env.current_contract_address();
        let pool = pool(&env);
        let from_fee = amount.min(waiver.fee);
        let from_pool = amount - from_fee;
        let fee_to_pool = waiver.fee - from_fee;
        token.transfer(&contract, &waiver.business, &from_fee);
        if from_pool > 0 {
            // The pool signs this call, which covers the transfer out of it.
            pool.require_auth();
            token.transfer(&pool, &waiver.business, &from_pool);
        }
        if fee_to_pool > 0 {
            token.transfer(&contract, &pool, &fee_to_pool);
        }

        waiver.status = Status::PaidOut;
        waiver.paid_out = amount;
        save(&env, waiver_id, &waiver);
        PaidOut {
            waiver_id,
            business: waiver.business.clone(),
            amount,
            from_pool,
            fee_to_pool,
        }
        .publish(&env);
    }

    /// Turn a claim down: the fee settles to the pool as if never claimed.
    pub fn reject_claim(env: Env, waiver_id: u64) {
        approver(&env).require_auth();
        let mut waiver = load(&env, waiver_id);
        require_status(&env, &waiver, Status::Claimed);
        let pool = pool(&env);
        token_client(&env).transfer(&env.current_contract_address(), &pool, &waiver.fee);
        waiver.status = Status::Rejected;
        save(&env, waiver_id, &waiver);
        Rejected {
            waiver_id,
            pool,
            fee: waiver.fee,
        }
        .publish(&env);
    }

    pub fn get(env: Env, waiver_id: u64) -> Waiver {
        load(&env, waiver_id)
    }

    pub fn admin(env: Env) -> Address {
        admin(&env)
    }

    pub fn approver(env: Env) -> Address {
        approver(&env)
    }

    pub fn token(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Token).unwrap()
    }

    pub fn pool(env: Env) -> Address {
        pool(&env)
    }

    pub fn max_payout(env: Env) -> i128 {
        max_payout(&env)
    }

    pub fn set_max_payout(env: Env, max_payout: i128) {
        admin(&env).require_auth();
        if max_payout <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxPayout, &max_payout);
        MaxPayoutSet { max_payout }.publish(&env);
    }

    /// Point settlements and top-ups at a different Ardena wallet.
    pub fn set_pool(env: Env, pool: Address) {
        admin(&env).require_auth();
        env.storage().instance().set(&DataKey::Pool, &pool);
        PoolSet { pool }.publish(&env);
    }

    pub fn set_admin(env: Env, new_admin: Address) {
        admin(&env).require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);
    }

    pub fn set_approver(env: Env, new_approver: Address) {
        admin(&env).require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Approver, &new_approver);
    }

    /// Replace the contract code. Kept so a bug found on mainnet can be fixed
    /// without stranding held fees in a dead contract.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        admin(&env).require_auth();
        env.deployer()
            .update_current_contract(ContractExecutable::Wasm(new_wasm_hash));
    }
}

fn admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn approver(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Approver).unwrap()
}

fn pool(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Pool).unwrap()
}

fn max_payout(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::MaxPayout).unwrap()
}

fn token_client(env: &Env) -> token::Client<'_> {
    let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
    token::Client::new(env, &token)
}

fn load(env: &Env, waiver_id: u64) -> Waiver {
    env.storage()
        .persistent()
        .get(&DataKey::Waiver(waiver_id))
        .unwrap_or_else(|| panic_with_error!(env, Error::WaiverNotFound))
}

fn save(env: &Env, waiver_id: u64, waiver: &Waiver) {
    let key = DataKey::Waiver(waiver_id);
    let persistent = env.storage().persistent();
    persistent.set(&key, waiver);
    persistent.extend_ttl(&key, WAIVER_TTL_THRESHOLD, WAIVER_TTL_EXTEND_TO);
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

fn require_status(env: &Env, waiver: &Waiver, expected: Status) {
    if waiver.status != expected {
        panic_with_error!(env, Error::InvalidStatus);
    }
}

mod test;
