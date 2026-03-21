//! Tests for the poster module — contract ABI encoding and Anvil integration.

use alloy::{
    node_bindings::Anvil,
    primitives::{Address, Bytes, FixedBytes},
    signers::local::PrivateKeySigner,
};
use strata_agent::poster;

/// Verify public values layout: 104 bytes with correct field positions.
#[test]
fn public_values_layout() {
    let mut pv = [0u8; 104];

    let old_root = [1u8; 32];
    let new_root = [2u8; 32];
    let nonce: u64 = 42;
    let soul_hash = [3u8; 32];

    pv[0..32].copy_from_slice(&old_root);
    pv[32..64].copy_from_slice(&new_root);
    pv[64..72].copy_from_slice(&nonce.to_be_bytes());
    pv[72..104].copy_from_slice(&soul_hash);

    assert_eq!(&pv[0..32], &old_root);
    assert_eq!(&pv[32..64], &new_root);
    assert_eq!(u64::from_be_bytes(pv[64..72].try_into().unwrap()), 42);
    assert_eq!(&pv[72..104], &soul_hash);
}

/// Verify the sol! macro generates valid ABI-encoded calldata.
#[test]
fn submit_transition_encoding() {
    use alloy::sol_types::SolCall;

    let call = poster::StrataRollup::submitTransitionCall {
        publicValues: Bytes::from(vec![0u8; 104]),
        proofData: Bytes::new(),
        _2: Bytes::new(),
    };

    let encoded = call.abi_encode();
    assert!(encoded.len() >= 4 + 32 * 3);
    assert_eq!(
        &encoded[..4],
        &poster::StrataRollup::submitTransitionCall::SELECTOR
    );
}

/// Deploy StrataRollup to a local Anvil instance and verify state.
#[tokio::test]
async fn deploy_to_anvil() {
    let anvil = Anvil::new().try_spawn().expect("failed to spawn anvil");
    let rpc_url = anvil.endpoint();

    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();
    let initial_root = FixedBytes::from([0xABu8; 32]);

    let address = poster::deploy_contract(
        &rpc_url,
        signer.clone(),
        "test soul",
        Address::ZERO,
        FixedBytes::ZERO,
        FixedBytes::ZERO,
        initial_root,
    )
    .await
    .expect("deploy failed");

    assert_ne!(address, Address::ZERO);

    let config = poster::PosterConfig {
        rpc_url: rpc_url.clone(),
        contract_address: address,
    };

    let nonce = poster::read_nonce(&config).await.expect("read nonce failed");
    assert_eq!(nonce, 0);

    let root = poster::read_state_root(&config)
        .await
        .expect("read state root failed");
    assert_eq!(root, initial_root);
}
