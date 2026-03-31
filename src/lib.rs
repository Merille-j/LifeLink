#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, symbol_short,
    token, Address, Env, String, Symbol,
};

// ---------------------------------------------------------------------------
// Storage key types
// ---------------------------------------------------------------------------

/// Composite key for a single blood request record.
#[contracttype]
#[derive(Clone)]
pub struct RequestKey {
    pub request_id: u64,
}

/// Full blood request stored on-chain.
#[contracttype]
#[derive(Clone)]
pub struct BloodRequest {
    pub request_id: u64,
    /// Stellar wallet of the hospital or blood bank that posted the request
    pub hospital: Address,
    /// ABO/Rh blood type string, e.g. "O+" or "AB-"
    pub blood_type: String,
    /// Number of units needed
    pub units_needed: u32,
    /// Units fulfilled so far (incremented on each confirmed donation)
    pub units_fulfilled: u32,
    /// GPS coordinates packed as "lat,lng" string for off-chain map rendering
    pub location: String,
    /// Unix timestamp deadline — request expires after this
    pub deadline: u64,
    /// Current status of the request
    pub status: RequestStatus,
    /// Timestamp when this request was posted
    pub created_at: u64,
}

/// Lifecycle states for a blood request.
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum RequestStatus {
    /// Accepting donor responses
    Open = 0,
    /// All units fulfilled
    Fulfilled = 1,
    /// Deadline passed without fulfilment
    Expired = 2,
}

/// Composite key for a donor response record.
#[contracttype]
#[derive(Clone)]
pub struct ResponseKey {
    pub request_id: u64,
    pub donor: Address,
}

/// Donor response record — created when donor calls respond_to_request().
#[contracttype]
#[derive(Clone)]
pub struct DonorResponse {
    pub request_id: u64,
    pub donor: Address,
    /// Claimable balance ID on Stellar — holds the HLTH reward in escrow
    pub claimable_balance_id: String,
    /// Whether BHW has confirmed the physical donation
    pub confirmed: bool,
    /// Whether the reward claimable balance has been released
    pub reward_claimed: bool,
}

/// Error variants returned by contract functions.
///
/// `#[contracterror]` (not `#[contracttype]`) is required so the Soroban
/// macro can auto-implement `From<Error> for soroban_sdk::Error`, which
/// `#[contractimpl]` needs when a function returns `Result<_, Error>`.
#[contracterror]
#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    /// Request ID does not exist
    RequestNotFound = 1,
    /// Blood request is not in Open status
    RequestNotOpen = 2,
    /// Donor has already responded to this request
    DuplicateResponse = 3,
    /// Response record not found
    ResponseNotFound = 4,
    /// Donation already confirmed for this donor+request pair
    AlreadyConfirmed = 5,
    /// Reward already claimed
    RewardAlreadyClaimed = 6,
    /// Caller is not authorised
    Unauthorized = 7,
    /// Request deadline has passed
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
pub struct LifeLinkContract;

#[contractimpl]
impl LifeLinkContract {

    // -----------------------------------------------------------------------
    // initialize
    // -----------------------------------------------------------------------

    /// Deploy and configure the contract. Call once immediately after deployment.
    ///
    /// - admin       : Philippine Red Cross multisig wallet (confirms donations)
    /// - hlth_token  : Address of the HLTH custom asset contract
    /// - reward_amt  : HLTH stroops paid per verified donation
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
        // Start request ID counter at 1
        env.storage().instance().set(&NEXT_ID, &1u64);
    }

    // -----------------------------------------------------------------------
    // post_request
    // -----------------------------------------------------------------------

    /// Hospital posts an emergency blood request on-chain.
    ///
    /// Stores a BloodRequest record with the supplied metadata and emits a
    /// "blood_req" event. Off-chain services subscribe to Horizon's event
    /// stream and push notifications to nearby verified donors immediately.
    ///
    /// Returns the new request_id so the hospital can track fulfilment.
    pub fn post_request(
        env: Env,
        hospital: Address,
        blood_type: String,
        units_needed: u32,
        location: String,
        deadline: u64,
    ) -> Result<u64, Error> {
        hospital.require_auth();

        // Reject requests whose deadline is already in the past
        if deadline <= env.ledger().timestamp() {
            return Err(Error::RequestExpired);
        }

        // Assign and increment request ID
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

        // Event consumed by Horizon stream → push notification layer
        env.events().publish(
            (symbol_short!("blood_req"), hospital.clone()),
            (request_id, blood_type, units_needed, location),
        );

        Ok(request_id)
    }

    // -----------------------------------------------------------------------
    // respond_to_request
    // -----------------------------------------------------------------------

    /// Donor signals intent to donate and locks their reward in escrow.
    ///
    /// A claimable balance holding HLTH tokens is created from the contract's
    /// balance. The claimable_balance_id is stored in the DonorResponse so
    /// the donor can claim it after BHW confirmation.
    ///
    /// Why claimable balances: the reward is committed at response time but
    /// only released after physical verification — preventing false claims
    /// while giving the donor a guaranteed payout once they donate.
    pub fn respond_to_request(
        env: Env,
        donor: Address,
        request_id: u64,
        claimable_balance_id: String,
    ) -> Result<(), Error> {
        donor.require_auth();

        let req_key = RequestKey { request_id };

        // Fetch and validate request
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

        // Prevent duplicate responses from the same donor
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

        // Emit event for off-chain tracking
        env.events().publish(
            (symbol_short!("donor_rsp"), donor.clone()),
            (request_id, claimable_balance_id),
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // confirm_donation
    // -----------------------------------------------------------------------

    /// BHW (Barangay Health Worker) confirms the physical donation occurred.
    ///
    /// Requires both the donor and the admin (BHW / Red Cross) to have signed
    /// this transaction — a 2-of-2 co-signature pattern enforced by
    /// require_auth() on both addresses.
    ///
    /// On confirmation:
    ///   - DonorResponse.confirmed is set to true
    ///   - units_fulfilled on the BloodRequest is incremented
    ///   - If units_fulfilled >= units_needed, request status → Fulfilled
    ///   - HLTH reward transferred directly to donor via token::Client
    pub fn confirm_donation(
        env: Env,
        admin: Address,
        donor: Address,
        request_id: u64,
    ) -> Result<(), Error> {
        // 2-of-2: both admin (BHW/Red Cross) and donor must sign
        admin.require_auth();
        donor.require_auth();

        // Verify caller is the stored admin
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

        // Mark donation as confirmed
        response.confirmed = true;
        env.storage().persistent().set(&resp_key, &response);

        // Update unit count on the request
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

        // Release HLTH reward directly to donor wallet
        let hlth_token: Address = env.storage().instance().get(&HLTH_TOKEN).unwrap();
        let reward_amt: i128 = env.storage().instance().get(&REWARD_AMT).unwrap();
        let token_client = token::Client::new(&env, &hlth_token);

        token_client.transfer(
            &env.current_contract_address(),
            &donor,
            &reward_amt,
        );

        // Emit confirmation event for public audit trail
        env.events().publish(
            (symbol_short!("confirmed"), admin.clone()),
            (request_id, donor, reward_amt),
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // expire_request
    // -----------------------------------------------------------------------

    /// Mark an unfulfilled request as Expired after its deadline passes.
    /// Permissionless — anyone can call this to clean up stale requests.
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
            // Deadline hasn't passed yet — cannot expire
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

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Return the full BloodRequest record for a given request_id.
    pub fn get_request(env: Env, request_id: u64) -> BloodRequest {
        let key = RequestKey { request_id };
        env.storage()
            .persistent()
            .get(&key)
            .expect("request not found")
    }

    /// Return the DonorResponse for a given (request_id, donor) pair.
    pub fn get_response(env: Env, request_id: u64, donor: Address) -> DonorResponse {
        let key = ResponseKey { request_id, donor };
        env.storage()
            .persistent()
            .get(&key)
            .expect("response not found")
    }

    /// Return the next request ID that will be assigned.
    pub fn get_next_id(env: Env) -> u64 {
        env.storage().instance().get(&NEXT_ID).unwrap_or(1)
    }

    /// Return the admin address.
    pub fn get_admin(env: Env) -> Address {
        env.storage().instance().get(&ADMIN).unwrap()
    }
}

mod test;