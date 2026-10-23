//! Ardena security-deposit escrow.
//!
//! One contract holds every booking's deposit, keyed by an escrow id that the
//! Ardena backend assigns. A deposit moves through:
//!
//! ```text
//!   lock ──► Locked ──release (after release_after, anyone)──► Released
//!              │    ──release_early (business waives claim)──► Released
//!              │    ──refund (admin, cancelled booking)──────► Refunded
//!              └──dispute (business or admin)──► Disputed ──resolve (approver)──► Resolved
//! ```
//!
//! The guarantees a renter can check on-chain, without trusting Ardena:
//! - Locked money can only leave to the renter, or to the business through an
//!   approver's `resolve`. No function pays anyone else.
//! - Once `release_after` passes with no dispute, *anyone* can call `release`,
//!   so the refund does not depend on Ardena's backend being up or willing.
//! - A business can only dispute before `release_after`.
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, BytesN, ContractExecutable, Env,
};

// Ledgers close roughly every 5 seconds.
const DAY_IN_LEDGERS: u32 = 17_280;
// A rental plus its inspection window rarely exceeds a few weeks; keep escrow
// entries alive for 120 days after every write, and re-extend once fewer than
// 60 days remain, so a long trip can never let its deposit's record expire.
const ESCROW_TTL_THRESHOLD: u32 = 60 * DAY_IN_LEDGERS;
const ESCROW_TTL_EXTEND_TO: u32 = 120 * DAY_IN_LEDGERS;
const INSTANCE_TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_TTL_EXTEND_TO: u32 = 90 * DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    EscrowExists = 1,
    EscrowNotFound = 2,
    InvalidAmount = 3,
    InvalidStatus = 4,
    ReleaseNotDue = 5,
    DisputeWindowClosed = 6,
    NotAParty = 7,
    InvalidSplit = 8,
    SameParty = 9,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Locked = 0,
    Disputed = 1,
    Released = 2,
    Resolved = 3,
    Refunded = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Escrow {
    pub renter: Address,
    pub business: Address,
    /// Token base units (USDC has 7 decimals on Stellar).
    pub amount: i128,
    /// Unix seconds after which an undisputed deposit can be released by anyone.
    pub release_after: u64,
    pub status: Status,
    /// Set by `resolve`: how much of `amount` went to the business.
    pub to_business: i128,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Approver,
    Token,
    Escrow(u64),
}

#[contractevent(topics = ["locked"])]
pub struct Locked {
    #[topic]
    pub escrow_id: u64,
    pub renter: Address,
    pub business: Address,
    pub amount: i128,
    pub release_after: u64,
}

#[contractevent(topics = ["released"])]
pub struct Released {
    #[topic]
    pub escrow_id: u64,
    pub renter: Address,
    pub amount: i128,
}

#[contractevent(topics = ["refunded"])]
pub struct Refunded {
    #[topic]
    pub escrow_id: u64,
    pub renter: Address,
    pub amount: i128,
}

#[contractevent(topics = ["disputed"])]
pub struct Disputed {
    #[topic]
    pub escrow_id: u64,
    pub by: Address,
}

#[contractevent(topics = ["resolved"])]
pub struct Resolved {
    #[topic]
    pub escrow_id: u64,
    pub to_business: i128,
    pub to_renter: i128,
}

#[contractevent(topics = ["release_after_set"])]
pub struct ReleaseAfterSet {
    #[topic]
    pub escrow_id: u64,
    pub release_after: u64,
}

#[contract]
pub struct DepositEscrow;

#[contractimpl]
impl DepositEscrow {
    /// `admin` runs the booking lifecycle (refunds, release dates); `approver`
    /// decides disputes. They may be the same account, but keeping them apart
    /// lets dispute decisions sit behind a separate key.
    pub fn __constructor(env: Env, admin: Address, approver: Address, token: Address) {
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Approver, &approver);
        storage.set(&DataKey::Token, &token);
    }

    /// Move `amount` from the renter into escrow for booking `escrow_id`.
    pub fn lock(
        env: Env,
        escrow_id: u64,
        renter: Address,
        business: Address,
        amount: i128,
        release_after: u64,
    ) {
        renter.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if renter == business {
            panic_with_error!(&env, Error::SameParty);
        }
        let key = DataKey::Escrow(escrow_id);
        // An id is spent forever once used, even after its deposit is paid out,
        // so a replayed or duplicated lock can never collect a second deposit.
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, Error::EscrowExists);
        }

        token_client(&env).transfer(&renter, &env.current_contract_address(), &amount);

        let escrow = Escrow {
            renter: renter.clone(),
            business: business.clone(),
            amount,
            release_after,
            status: Status::Locked,
            to_business: 0,
        };
        save(&env, escrow_id, &escrow);
        Locked {
            escrow_id,
            renter,
            business,
            amount,
            release_after,
        }
        .publish(&env);
    }

    /// Return the full deposit to the renter once the inspection window has
    /// passed without a dispute. Callable by anyone.
    pub fn release(env: Env, escrow_id: u64) {
        let mut escrow = load(&env, escrow_id);
        require_status(&env, &escrow, Status::Locked);
        if env.ledger().timestamp() < escrow.release_after {
            panic_with_error!(&env, Error::ReleaseNotDue);
        }
        pay_renter_in_full(&env, escrow_id, &mut escrow, Status::Released);
    }

    /// The business waives any claim and lets the deposit go back now.
    pub fn release_early(env: Env, escrow_id: u64) {
        let mut escrow = load(&env, escrow_id);
        escrow.business.require_auth();
        require_status(&env, &escrow, Status::Locked);
        pay_renter_in_full(&env, escrow_id, &mut escrow, Status::Released);
    }

    /// Booking cancelled before the trip: the whole deposit goes back.
    pub fn refund(env: Env, escrow_id: u64) {
        admin(&env).require_auth();
        let mut escrow = load(&env, escrow_id);
        require_status(&env, &escrow, Status::Locked);
        pay_renter_in_full(&env, escrow_id, &mut escrow, Status::Refunded);
    }

    /// Move the release date, e.g. when a trip is extended or ends early.
    pub fn set_release_after(env: Env, escrow_id: u64, release_after: u64) {
        admin(&env).require_auth();
        let mut escrow = load(&env, escrow_id);
        require_status(&env, &escrow, Status::Locked);
        escrow.release_after = release_after;
        save(&env, escrow_id, &escrow);
        ReleaseAfterSet {
            escrow_id,
            release_after,
        }
        .publish(&env);
    }

    /// Freeze the deposit until the approver decides. The business may only
    /// dispute inside the inspection window; the admin may at any time while locked.
    pub fn dispute(env: Env, escrow_id: u64, by: Address) {
        by.require_auth();
        let mut escrow = load(&env, escrow_id);
        require_status(&env, &escrow, Status::Locked);
        if by == escrow.business {
            if env.ledger().timestamp() >= escrow.release_after {
                panic_with_error!(&env, Error::DisputeWindowClosed);
            }
        } else if by != admin(&env) {
            panic_with_error!(&env, Error::NotAParty);
        }
        escrow.status = Status::Disputed;
        save(&env, escrow_id, &escrow);
        Disputed { escrow_id, by }.publish(&env);
    }

    /// Settle a dispute: `to_business` goes to the business, the rest to the renter.
    pub fn resolve(env: Env, escrow_id: u64, to_business: i128) {
        approver(&env).require_auth();
        let mut escrow = load(&env, escrow_id);
        require_status(&env, &escrow, Status::Disputed);
        if to_business < 0 || to_business > escrow.amount {
            panic_with_error!(&env, Error::InvalidSplit);
        }
        let to_renter = escrow.amount - to_business;
        let token = token_client(&env);
        let contract = env.current_contract_address();
        if to_business > 0 {
            token.transfer(&contract, &escrow.business, &to_business);
        }
        if to_renter > 0 {
            token.transfer(&contract, &escrow.renter, &to_renter);
        }
        escrow.status = Status::Resolved;
        escrow.to_business = to_business;
        save(&env, escrow_id, &escrow);
        Resolved {
            escrow_id,
            to_business,
            to_renter,
        }
        .publish(&env);
    }

    pub fn get(env: Env, escrow_id: u64) -> Escrow {
        load(&env, escrow_id)
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
    /// without stranding locked deposits in a dead contract.
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

fn token_client(env: &Env) -> token::Client<'_> {
    let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
    token::Client::new(env, &token)
}

fn load(env: &Env, escrow_id: u64) -> Escrow {
    env.storage()
        .persistent()
        .get(&DataKey::Escrow(escrow_id))
        .unwrap_or_else(|| panic_with_error!(env, Error::EscrowNotFound))
}

fn save(env: &Env, escrow_id: u64, escrow: &Escrow) {
    let key = DataKey::Escrow(escrow_id);
    let persistent = env.storage().persistent();
    persistent.set(&key, escrow);
    persistent.extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_EXTEND_TO);
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

fn require_status(env: &Env, escrow: &Escrow, expected: Status) {
    if escrow.status != expected {
        panic_with_error!(env, Error::InvalidStatus);
    }
}

fn pay_renter_in_full(env: &Env, escrow_id: u64, escrow: &mut Escrow, final_status: Status) {
    token_client(env).transfer(
        &env.current_contract_address(),
        &escrow.renter,
        &escrow.amount,
    );
    escrow.status = final_status;
    save(env, escrow_id, escrow);
    match final_status {
        Status::Refunded => Refunded {
            escrow_id,
            renter: escrow.renter.clone(),
            amount: escrow.amount,
        }
        .publish(env),
        _ => Released {
            escrow_id,
            renter: escrow.renter.clone(),
            amount: escrow.amount,
        }
        .publish(env),
    }
}

mod test;
