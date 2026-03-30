#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, symbol_short,
    token, Address, Env, String, Symbol,
};

// ---------------------------------------------------------------------------
// Storage key types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)] // Added Debug for test assertions
pub struct RequestKey {
    pub request_id: u64,
}

#[contracttype]
#[derive(Clone, Debug)] // Added Debug for test assertions
pub struct BloodRequest {
    pub request_id: u64,
    pub hospital: Address,
    pub blood_type: String,
    pub units_needed: u32,
    pub units_fulfilled: u32,
    pub location: String,
    pub deadline: u64,
    pub status: RequestStatus,
    pub created_at: u64,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)] // Added Debug and Eq/PartialEq
pub enum RequestStatus {
    Open = 0,
    Fulfilled = 1,
    Expired = 2,
}

#[contracttype]
#[derive(Clone, Debug)] // Added Debug
pub struct ResponseKey {
    pub request_id: u64,
    pub donor: Address,
}

#[contracttype]
#[derive(Clone, Debug)] // Added Debug
pub struct DonorResponse {
    pub request_id: u64,
    pub donor: Address,
    pub claimable_balance_id: String,
    pub confirmed: bool,
    pub reward_claimed: bool,
}

/// Error variants returned by contract functions.
#[contracterror] // Changed from contracttype to contracterror
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    RequestNotFound = 1,
    RequestNotOpen = 2,
    DuplicateResponse = 3,
    ResponseNotFound = 4,
    AlreadyConfirmed = 5,
    RewardAlreadyClaimed = 6,
    Unauthorized = 7,
    RequestExpired = 8,
}

// ---------------------------------------------------------------------------
// Instance-storage symbol keys
// ---------------------------------------------------------------------------

const ADMIN: Symbol        = symbol_short!("ADMIN");
const HLTH_TOKEN: Symbol   = symbol_short!("HLTHTOKEN");
const REWARD_AMT: Symbol   = symbol_short!("RWDAMT");
const NEXT_ID: Symbol      = symbol_short!("NEXT_ID");

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct BloodLinkContract;

#[contractimpl]
impl BloodLinkContract {

    pub fn initialize(
        env: Env,
        admin: Address,
        hlth_token: Address,
        reward_amt: i128,
    ) {
        if env.storage().instance().has(&ADMIN) {
            panic!("already initialised");
        }
        admin.require_auth();
        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&HLTH_TOKEN, &hlth_token);
        env.storage().instance().set(&REWARD_AMT, &reward_amt);
        env.storage().instance().set(&NEXT_ID, &1u64);
    }

    pub fn post_request(
        env: Env,
        hospital: Address,
        blood_type: String,
        units_needed: u32,
        location: String,
        deadline: u64,
    ) -> Result<u64, Error> {
        hospital.require_auth();

        if deadline <= env.ledger().timestamp() {
            return Err(Error::RequestExpired);
        }

        let request_id: u64 = env.storage().instance().get(&NEXT_ID).unwrap();
        env.storage().instance().set(&NEXT_ID, &(request_id + 1));

        let now = env.ledger().timestamp();

        let record = BloodRequest {
            request_id,
            hospital: hospital.clone(),
            blood_type: blood_type.clone(),
            units_needed,
            units_fulfilled: 0,
            location: location.clone(),
            deadline,
            status: RequestStatus::Open,
            created_at: now,
        };

        let key = RequestKey { request_id };
        env.storage().persistent().set(&key, &record);

        env.events().publish(
            (symbol_short!("blood_req"), hospital.clone()),
            (request_id, blood_type, units_needed, location),
        );

        Ok(request_id)
    }

    pub fn respond_to_request(
        env: Env,
        donor: Address,
        request_id: u64,
        claimable_balance_id: String,
    ) -> Result<(), Error> {
        donor.require_auth();

        let req_key = RequestKey { request_id };

        let request: BloodRequest = env
            .storage()
            .persistent()
            .get(&req_key)
            .ok_or(Error::RequestNotFound)?;

        if request.status != RequestStatus::Open {
            return Err(Error::RequestNotOpen);
        }

        if env.ledger().timestamp() > request.deadline {
            return Err(Error::RequestExpired);
        }

        let resp_key = ResponseKey {
            request_id,
            donor: donor.clone(),
        };
        if env.storage().persistent().has(&resp_key) {
            return Err(Error::DuplicateResponse);
        }

        let response = DonorResponse {
            request_id,
            donor: donor.clone(),
            claimable_balance_id: claimable_balance_id.clone(),
            confirmed: false,
            reward_claimed: false,
        };

        env.storage().persistent().set(&resp_key, &response);

        env.events().publish(
            (symbol_short!("donor_rsp"), donor.clone()),
            (request_id, claimable_balance_id),
        );

        Ok(())
    }

    pub fn confirm_donation(
        env: Env,
        admin: Address,
        donor: Address,
        request_id: u64,
    ) -> Result<(), Error> {
        admin.require_auth();
        donor.require_auth();

        let stored_admin: Address = env.storage().instance().get(&ADMIN).unwrap();
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }

        let resp_key = ResponseKey {
            request_id,
            donor: donor.clone(),
        };

        let mut response: DonorResponse = env
            .storage()
            .persistent()
            .get(&resp_key)
            .ok_or(Error::ResponseNotFound)?;

        if response.confirmed {
            return Err(Error::AlreadyConfirmed);
        }

        response.confirmed = true;
        env.storage().persistent().set(&resp_key, &response);

        let req_key = RequestKey { request_id };
        let mut request: BloodRequest = env
            .storage()
            .persistent()
            .get(&req_key)
            .ok_or(Error::RequestNotFound)?;

        request.units_fulfilled += 1;
        if request.units_fulfilled >= request.units_needed {
            request.status = RequestStatus::Fulfilled;
        }
        env.storage().persistent().set(&req_key, &request);

        let hlth_token: Address = env.storage().instance().get(&HLTH_TOKEN).unwrap();
        let reward_amt: i128 = env.storage().instance().get(&REWARD_AMT).unwrap();
        let token_client = token::Client::new(&env, &hlth_token);

        token_client.transfer(
            &env.current_contract_address(),
            &donor,
            &reward_amt,
        );

        env.events().publish(
            (symbol_short!("confirmed"), admin.clone()),
            (request_id, donor, reward_amt),
        );

        Ok(())
    }

    pub fn expire_request(env: Env, request_id: u64) -> Result<(), Error> {
        let req_key = RequestKey { request_id };

        let mut request: BloodRequest = env
            .storage()
            .persistent()
            .get(&req_key)
            .ok_or(Error::RequestNotFound)?;

        if request.status != RequestStatus::Open {
            return Err(Error::RequestNotOpen);
        }

        if env.ledger().timestamp() <= request.deadline {
            return Err(Error::RequestNotOpen);
        }

        request.status = RequestStatus::Expired;
        env.storage().persistent().set(&req_key, &request);

        env.events().publish(
            (symbol_short!("req_exp"), request.hospital),
            request_id,
        );

        Ok(())
    }

    pub fn get_request(env: Env, request_id: u64) -> BloodRequest {
        let key = RequestKey { request_id };
        env.storage()
            .persistent()
            .get(&key)
            .expect("request not found")
    }

    pub fn get_response(env: Env, request_id: u64, donor: Address) -> DonorResponse {
        let key = ResponseKey { request_id, donor };
        env.storage()
            .persistent()
            .get(&key)
            .expect("response not found")
    }

    pub fn get_next_id(env: Env) -> u64 {
        env.storage().instance().get(&NEXT_ID).unwrap_or(1)
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage().instance().get(&ADMIN).unwrap()
    }
}

mod test;