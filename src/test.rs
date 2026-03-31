#[cfg(test)]
mod tests {
    use soroban_sdk::unwrap::UnwrapOptimized;
use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token, Address, Env, String,
    };

    use crate::{LifeLinkContract, LifeLinkContractClient, Error, RequestStatus};

    const REWARD_HLTH: i128 = 50_000_000; // 50 HLTH tokens in stroops

    fn create_token(env: &Env, admin: &Address, recipient: &Address, amount: i128) -> Address {
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        let token_admin = token::StellarAssetClient::new(env, &token_id.address());
        token_admin.mint(recipient, &amount);
        token_id.address()
    }

    fn setup() -> (
        Env,
        Address,   // admin (Red Cross)
        Address,   // hospital
        Address,   // donor
        Address,   // contract
        Address,   // hlth_token
        LifeLinkContractClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin    = Address::generate(&env);
        let hospital = Address::generate(&env);
        let donor    = Address::generate(&env);

        let contract_id = env.register(LifeLinkContract, ());
        let client = LifeLinkContractClient::new(&env, &contract_id);

        // Mint HLTH supply to contract for rewards
        let hlth_token = create_token(&env, &admin, &contract_id, 10_000_000_000);

        env.ledger().with_mut(|info| {
            info.timestamp = 1_700_000_000;
        });

        client.initialize(&admin, &hlth_token, &REWARD_HLTH);

        (env, admin, hospital, donor, contract_id, hlth_token, client)
    }

    // -----------------------------------------------------------------------
    // Test 1 — Happy path
    // Hospital posts request → donor responds → BHW confirms → HLTH reward paid
    // -----------------------------------------------------------------------
    #[test]
    fn test_full_donation_flow_happy_path() {
        let (env, admin, hospital, donor, _contract_id, hlth_token, client) = setup();

        let deadline = env.ledger().timestamp() + 3600; // 1 hour from now

        // Hospital posts emergency O+ request
        let request_id = client.post_request(
            &hospital,
            &String::from_str(&env, "O+"),
            &2u32,
            &String::from_str(&env, "14.5995,120.9842"),
            &deadline,
        );
        assert_eq!(request_id, 1u64);

        // Donor responds with a mock claimable balance ID
        let cb_id = String::from_str(&env, "claimable_balance_mock_001");
        client.respond_to_request(&donor, &request_id, &cb_id);

        // Check donor balance before confirmation
        let token_client = token::Client::new(&env, &hlth_token);
        let balance_before = token_client.balance(&donor);

        // BHW + admin co-sign confirmation
        client.confirm_donation(&admin, &donor, &request_id);

        // Donor should have received exactly REWARD_HLTH
        let balance_after = token_client.balance(&donor);
        assert_eq!(
            balance_after - balance_before,
            REWARD_HLTH,
            "donor should receive exactly the HLTH reward"
        );

        // Verify the response record shows confirmed = true
        let response = client.get_response(&request_id, &donor);
        assert!(response.confirmed, "response should be marked confirmed");

        // Verify request still Open (only 1 of 2 units fulfilled)
        let request = client.get_request(&request_id);
        assert_eq!(request.units_fulfilled, 1u32);
        assert_eq!(request.status, RequestStatus::Open);
    }

    // -----------------------------------------------------------------------
    // Test 2 — Edge case
    // A duplicate donor response to the same request must be rejected.
    // -----------------------------------------------------------------------
    #[test]
    fn test_duplicate_donor_response_rejected() {
        let (env, _admin, hospital, donor, _contract_id, _hlth_token, client) = setup();

        let deadline = env.ledger().timestamp() + 3600;

        let request_id = client.post_request(
            &hospital,
            &String::from_str(&env, "AB-"),
            &1u32,
            &String::from_str(&env, "10.3157,123.8854"),
            &deadline,
        );

        let cb_id = String::from_str(&env, "claimable_balance_mock_002");

        // First response — should succeed
        client.respond_to_request(&donor, &request_id, &cb_id);

        // Second response from the same donor — must fail
        let result = client.try_respond_to_request(
            &donor,
            &request_id,
            &String::from_str(&env, "claimable_balance_mock_003"),
        );

        assert!(result.is_err(), "duplicate response should return an error");

        // Fix: correct method name is unwrap_optimized, not unwrap_optimistic
        let contract_err = result.err().unwrap().unwrap_optimized();
        assert_eq!(
            contract_err,
            Error::DuplicateResponse,
            "error must be DuplicateResponse"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3 — State verification
    // Storage correctly reflects request metadata and status after posting.
    // -----------------------------------------------------------------------
    #[test]
    fn test_request_storage_state_after_posting() {
        let (env, _admin, hospital, _donor, _contract_id, _hlth_token, client) = setup();

        let deadline = env.ledger().timestamp() + 7200;

        let request_id = client.post_request(
            &hospital,
            &String::from_str(&env, "B+"),
            &3u32,
            &String::from_str(&env, "7.0731,125.6128"),
            &deadline,
        );

        let stored = client.get_request(&request_id);

        assert_eq!(stored.request_id, request_id);
        assert_eq!(stored.hospital, hospital, "hospital address must match");
        assert_eq!(
            stored.blood_type,
            String::from_str(&env, "B+"),
            "blood type must match"
        );
        assert_eq!(stored.units_needed, 3u32, "units_needed must be 3");
        assert_eq!(stored.units_fulfilled, 0u32, "no units fulfilled yet");
        assert_eq!(stored.status, RequestStatus::Open, "status must be Open");
        assert_eq!(stored.deadline, deadline, "deadline must be stored correctly");
        assert_eq!(
            stored.created_at,
            1_700_000_000u64,
            "created_at must match mocked ledger timestamp"
        );
    }
}