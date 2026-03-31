# LifeLink
> On-chain emergency blood supply matching with HLTH token rewards — built on Stellar Soroban.

## Problem
The Philippines faces a 500,000-unit annual blood shortage. Current donor matching
is entirely manual and phone-based — average response time is 4–8 hours.
In trauma cases, a patient may need blood in under 30 minutes.

## Solution
LifeLink anchors blood requests on Stellar's Soroban. Hospitals post O+/AB- requests
on-chain with GPS and a deadline. Verified donors respond in one tap, earning HLTH
tokens held in escrow. A BHW co-signs on-site confirmation, releasing the reward
automatically. Every match is publicly auditable on any Stellar explorer.

## Stellar features used
- Soroban smart contracts (request registry, response, 2-of-2 confirmation, reward)
- Custom HLTH token (donor reward, redeemable at RHUs)
- Claimable balances (reward held in escrow until BHW confirmation)
- Trustlines (donors opt into HLTH before earning)
- Stellar testnet (full demo environment)

## Prerequisites
- Rust + `wasm32-unknown-unknown` target
- Stellar CLI v22+
- Freighter wallet browser extension (set to Testnet)

## Build
```
cargo build --target wasm32-unknown-unknown --release
```

## Test
```
cargo test
```

## Deploy to testnet
```
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/lifelink.wasm \
  --source my-key \
  --network testnet
```

## Initialize
```
stellar contract invoke --id <CONTRACT_ID> --source my-key --network testnet \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --hlth_token <HLTH_TOKEN_CONTRACT_ADDRESS> \
  --reward_amt 50000000
```

## Post a blood request
```
stellar contract invoke --id <CONTRACT_ID> --source hospital-key --network testnet \
  -- post_request \
  --hospital <HOSPITAL_ADDRESS> \
  --blood_type '"O+"' \
  --units_needed 2 \
  --location '"14.5995,120.9842"' \
  --deadline 1700010000
```

## Confirm a donation (BHW + admin co-sign)
```
stellar contract invoke --id <CONTRACT_ID> --source admin-key --network testnet \
  -- confirm_donation \
  --admin <ADMIN_ADDRESS> \
  --donor <DONOR_ADDRESS> \
  --request_id 1
```

## License
MIT © 2026 LifeLink Contributors
