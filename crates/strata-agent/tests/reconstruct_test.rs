mod common;

use alloy::{
    network::EthereumWallet,
    node_bindings::Anvil,
    primitives::{Bytes, FixedBytes},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
};
use commonware_codec::Encode;
use commonware_runtime::{deterministic, Runner as _};
use strata_core::{BinaryEmbedding, ContentHash, MemoryEntry, MemoryId, Nonce};
use strata_vector_db::VectorDB;

use strata_agent::pipeline;
use strata_agent::poster::PosterConfig;
use strata_agent::reconstruct;

// Use the MockStrataRollup which skips ZK verification.
sol! {
    #[sol(rpc, all_derives)]
    MockStrataRollup,
    "../../contracts/out/StrataRollup.t.sol/MockStrataRollup.json"
}

fn make_entry(id: u64, text: &[u8]) -> MemoryEntry {
    MemoryEntry::new(
        MemoryId::new(id),
        BinaryEmbedding::test_from_id(id),
        ContentHash::digest(text),
    )
}

#[tokio::test]
async fn reconstruct_matches_posted_state() {
    let anvil = Anvil::new().try_spawn().expect("failed to spawn anvil");
    let rpc_url = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();
    let operator = signer.address();

    let soul_text = "test-soul";

    // Run the deterministic part (VectorDB + pipeline) synchronously,
    // collect transition outputs, then post them with tokio.
    let (transitions, texts) = deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("reconstruct", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        let mut state = common::genesis_state();
        let mut contents: Vec<String> = Vec::new();
        let mut transitions = Vec::new();

        // First transition: 1 entry
        let snap1 = pipeline::snapshot(state, &db);
        let entry0 = make_entry(0, b"first memory");
        db.append(entry0).await.unwrap();
        contents.push("first memory".to_string());
        let out1 = pipeline::finalize(&snap1, &db, &contents).unwrap();
        state = out1.new_state;
        transitions.push(out1);

        // Second transition: 2 entries
        let snap2 = pipeline::snapshot(state, &db);
        let entry1 = make_entry(1, b"second memory");
        let entry2 = make_entry(2, b"third memory");
        db.append(entry1).await.unwrap();
        db.append(entry2).await.unwrap();
        contents.push("second memory".to_string());
        contents.push("third memory".to_string());
        let out2 = pipeline::finalize(&snap2, &db, &contents).unwrap();
        transitions.push(out2);

        db.destroy().await.unwrap();
        (transitions, contents)
    });

    // Deploy MockStrataRollup with the genesis root
    let genesis_root = FixedBytes::from(*transitions[0].old_state.vector_index_root.as_bytes());
    let wallet = EthereumWallet::from(signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet.clone())
        .connect_http(rpc_url.parse().unwrap());

    let contract = MockStrataRollup::deploy(
        &provider,
        soul_text.to_string(),
        operator,
        genesis_root,
    )
    .await
    .expect("deploy failed");

    let contract_address = *contract.address();

    // Post both transitions
    for transition in &transitions {
        let memory_content = transition.record.encode();
        contract
            .submitTransition(
                Bytes::copy_from_slice(&transition.public_values),
                Bytes::new(), // empty proof — mock verifier
                Bytes::from(memory_content),
            )
            .send()
            .await
            .expect("send failed")
            .watch()
            .await
            .expect("watch failed");
    }

    let config = PosterConfig {
        rpc_url: rpc_url.clone(),
        contract_address,
    };

    // Reconstruct
    let reconstructed = reconstruct::reconstruct(&config)
        .await
        .expect("reconstruction failed");

    // Verify soul hash
    assert_eq!(
        reconstructed.state.soul_hash,
        strata_core::SoulHash::digest(soul_text.as_bytes()),
    );

    // Verify nonce
    assert_eq!(reconstructed.state.nonce, Nonce::new(2));

    // Verify entries count (1 from first + 2 from second = 3)
    assert_eq!(reconstructed.entries.len(), 3);
    assert_eq!(reconstructed.contents.len(), 3);

    // Verify content texts match
    assert_eq!(reconstructed.contents, texts);

    // Create fresh VectorDB, batch_append, verify root matches on-chain
    deterministic::Runner::default().start(|context| async move {
        let db_config = common::make_config("reconstruct-verify", &context);
        let mut db = VectorDB::new(context, db_config).await.unwrap();

        db.batch_append(reconstructed.entries)
            .await
            .expect("batch append failed");

        assert_eq!(
            db.root().as_bytes(),
            reconstructed.state.vector_index_root.as_bytes(),
            "reconstructed root does not match on-chain state root"
        );

        db.destroy().await.unwrap();
    });
}

#[tokio::test]
async fn reconstruct_zero_transitions() {
    let anvil = Anvil::new().try_spawn().expect("failed to spawn anvil");
    let rpc_url = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();
    let operator = signer.address();

    let soul_text = "test-soul";
    let genesis_root = strata_proof::compute_root::<strata_proof::Keccak256Hasher>(0, &[]);
    let genesis_root_fixed = FixedBytes::from(genesis_root);

    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse().unwrap());

    let contract = MockStrataRollup::deploy(
        &provider,
        soul_text.to_string(),
        operator,
        genesis_root_fixed,
    )
    .await
    .expect("deploy failed");

    let config = PosterConfig {
        rpc_url,
        contract_address: *contract.address(),
    };

    let reconstructed = reconstruct::reconstruct(&config)
        .await
        .expect("reconstruction failed");

    assert_eq!(reconstructed.state.nonce, Nonce::new(0));
    assert_eq!(reconstructed.entries.len(), 0);
    assert_eq!(reconstructed.contents.len(), 0);
    assert_eq!(
        reconstructed.state.soul_hash,
        strata_core::SoulHash::digest(soul_text.as_bytes()),
    );

    // Verify empty VectorDB root matches
    deterministic::Runner::default().start(|context| async move {
        let db_config = common::make_config("reconstruct-zero", &context);
        let db = VectorDB::new(context, db_config).await.unwrap();

        assert_eq!(
            db.root().as_bytes(),
            reconstructed.state.vector_index_root.as_bytes(),
        );

        db.destroy().await.unwrap();
    });
}
