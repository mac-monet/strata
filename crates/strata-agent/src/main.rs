use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::sync::Arc;

use commonware_runtime::{Runner as _, tokio};
use strata_core::{CoreState, Nonce, SoulHash, VectorRoot};
use strata_proof::{Keccak256Hasher, compute_root};
use strata_vector_db::{Config as JournaledConfig, VectorDB};

use alloy::primitives::{Address, FixedBytes};
use alloy::signers::local::PrivateKeySigner;
use strata_agent::agent::AgentConfig;
use strata_agent::embed::ApiEmbedder;
use strata_agent::llm::LlmClient;
use strata_agent::poster::{self, PosterConfig};
use strata_agent::prover::{ProofLevel, ProverConfig};
use strata_agent::server::{self, AppState, PostingConfig};
use strata_agent::tools::ToolExecutor;

const DEFAULT_PORT: u16 = 3000;
const DEFAULT_SOUL: &str = include_str!("../soul.md");

fn main() {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let anthropic_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY must be set");

    let openai_key = std::env::var("OPENAI_API_KEY")
        .expect("OPENAI_API_KEY must be set (for embeddings)");

    let soul = std::env::var("SOUL_FILE")
        .ok()
        .map(|path| std::fs::read_to_string(&path).expect("failed to read soul file"))
        .unwrap_or_else(|| DEFAULT_SOUL.to_string());

    let llm_client = LlmClient::anthropic(anthropic_key);
    let embedder = ApiEmbedder::new(openai_key);

    let addr: std::net::SocketAddr = ([0, 0, 0, 0], port).into();
    eprintln!("starting strata-agent on {addr}");

    let contract_addr_env = std::env::var("CONTRACT_ADDRESS").ok();
    let reconstruct = std::env::var("RECONSTRUCT")
        .ok()
        .map(|v| v == "1" || v == "true");

    tokio::Runner::default().start(|context| async move {
        let db_config = make_db_config(&context);
        let mut db = VectorDB::new(context, db_config)
            .await
            .expect("failed to initialize VectorDB");

        let (agent_state, executor) = if reconstruct == Some(true) {
            let address: Address = contract_addr_env
                .as_deref()
                .expect("CONTRACT_ADDRESS required for reconstruction")
                .parse()
                .expect("invalid CONTRACT_ADDRESS");
            let rpc_url = std::env::var("RPC_URL").expect("RPC_URL required for reconstruction");
            let config = PosterConfig {
                rpc_url,
                contract_address: address,
            };

            assert!(
                db.is_empty(),
                "VectorDB is not empty — reconstruction requires a fresh database"
            );

            eprintln!("reconstructing state from contract {address}...");
            let reconstructed = strata_agent::reconstruct::reconstruct(&config)
                .await
                .expect("reconstruction failed");

            let local_soul_hash = strata_core::SoulHash::digest(soul.as_bytes());
            assert_eq!(
                local_soul_hash, reconstructed.state.soul_hash,
                "local soul text does not match on-chain soul hash"
            );

            db.batch_append(reconstructed.entries)
                .await
                .expect("batch append failed");

            assert_eq!(
                db.root().as_bytes(),
                reconstructed.state.vector_index_root.as_bytes(),
                "reconstructed root does not match on-chain state root"
            );

            eprintln!(
                "reconstruction complete: {} entries, nonce {}",
                db.len(),
                reconstructed.state.nonce.get()
            );

            let executor = ToolExecutor::new(db, Box::new(embedder))
                .with_contents(reconstructed.contents)
                .expect("contents mismatch");
            (reconstructed.state, executor)
        } else {
            let genesis = genesis_state(&soul);
            let executor = ToolExecutor::new(db, Box::new(embedder));
            (genesis, executor)
        };

        // Wire optional on-chain posting (mock proofs).
        let posting = if let Ok(rpc_url) = std::env::var("RPC_URL") {
            let key_hex = std::env::var("OPERATOR_KEY")
                .expect("OPERATOR_KEY required with RPC_URL");
            let signer: PrivateKeySigner = key_hex.parse().expect("invalid OPERATOR_KEY");

            let contract_address = if let Ok(addr_str) = std::env::var("CONTRACT_ADDRESS") {
                addr_str.parse().expect("invalid CONTRACT_ADDRESS")
            } else {
                // Auto-deploy mock contract
                // TODO: nonce mismatch if used after reconstruction — the mock
                // contract starts at nonce 0 but the agent's state may be ahead.
                let genesis_root =
                    FixedBytes::from(*agent_state.vector_index_root.as_bytes());
                poster::deploy_mock_contract(&rpc_url, signer.clone(), &soul, genesis_root)
                    .await
                    .expect("mock deploy failed")
            };

            eprintln!("posting to contract {contract_address}");
            Some(PostingConfig {
                poster: PosterConfig {
                    rpc_url,
                    contract_address,
                },
                signer,
            })
        } else {
            None
        };

        // Wire optional ZK prover.
        let prover = if let Ok(dir) = std::env::var("PROVER_DIR") {
            let proof_level = std::env::var("PROOF_LEVEL")
                .map(|s| ProofLevel::from_str(&s))
                .unwrap_or_default();
            let config = ProverConfig::new(PathBuf::from(&dir), proof_level);
            eprintln!("prover enabled: {} (level: {})", dir, config.proof_level.as_str());
            Some(config)
        } else {
            None
        };

        let state = Arc::new(AppState::new(
            AgentConfig {
                soul,
                state: agent_state,
            },
            llm_client,
            executor,
            posting,
            prover,
        ));

        eprintln!("agent ready — POST http://{addr}/a2a");
        server::run(state, addr).await.expect("server error");
    });
}

fn genesis_state(soul: &str) -> CoreState {
    let root = compute_root::<Keccak256Hasher>(0, &[]);
    CoreState {
        soul_hash: SoulHash::digest(soul.as_bytes()),
        vector_index_root: VectorRoot::new(root),
        nonce: Nonce::new(0),
    }
}

fn make_db_config(context: &tokio::Context) -> JournaledConfig {
    let page_size = NonZeroU16::new(4096).unwrap();
    let page_cache_size = NonZeroUsize::new(64).unwrap();
    JournaledConfig {
        journal_partition: "strata-journal".into(),
        metadata_partition: "strata-meta".into(),
        items_per_blob: NonZeroU64::new(10_000).unwrap(),
        write_buffer: NonZeroUsize::new(4096).unwrap(),
        thread_pool: None,
        page_cache: commonware_runtime::buffer::paged::CacheRef::from_pooler(
            context,
            page_size,
            page_cache_size,
        ),
    }
}
