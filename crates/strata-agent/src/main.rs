use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use commonware_runtime::{Runner as _, tokio};
use ::tokio::sync::watch;
use strata_core::{CoreState, Nonce, SoulHash, VectorRoot};
use strata_proof::{Keccak256Hasher, compute_root};
use strata_vector_db::{Config as JournaledConfig, VectorDB};

use alloy::primitives::{Address, FixedBytes};
use alloy::signers::local::PrivateKeySigner;
use strata_agent::agent::AgentConfig;
use strata_agent::batch::{self, BatchConfig, PendingBatch};
use strata_agent::embed::{self, ApiEmbedder};
use strata_agent::identity::{self, IdentityConfig};
use strata_agent::llm::LlmClient;
use strata_agent::poster::{self, PosterConfig};
use strata_agent::prover::{ProofLevel, ProverConfig};
use strata_agent::server::{self, AppState, PostingConfig};
use strata_agent::tools::ToolExecutor;

const DEFAULT_PORT: u16 = 3000;
const DEFAULT_SOUL: &str = include_str!("../soul.md");
const DEFAULT_POST_INTERVAL_SECS: u64 = 1800; // 30 minutes

fn main() {
    // Load .env file if present (best-effort).
    load_dotenv();

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    // --- LLM provider ---
    let llm_client = make_llm_client();

    // --- Embeddings provider ---
    let embedder = make_embedder();

    let soul = std::env::var("SOUL_FILE")
        .ok()
        .map(|path| std::fs::read_to_string(&path).expect("failed to read soul file"))
        .unwrap_or_else(|| DEFAULT_SOUL.to_string());

    let addr: std::net::SocketAddr = ([0, 0, 0, 0], port).into();
    eprintln!("starting strata-agent on {addr}");

    let contract_addr_env = std::env::var("CONTRACT_ADDRESS").ok();
    let reconstruct = std::env::var("RECONSTRUCT")
        .ok()
        .map(|v| v == "1" || v == "true");

    let post_interval_secs: u64 = std::env::var("POST_INTERVAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_POST_INTERVAL_SECS);

    let wal_path = std::env::var("WAL_PATH")
        .unwrap_or_else(|_| "./strata-batch.wal".into());

    let snapshot_path = PathBuf::from(
        std::env::var("SNAPSHOT_PATH").unwrap_or_else(|_| "./strata-snapshot.json".into()),
    );

    tokio::Runner::default().start(|context| async move {
        let db_config = make_db_config(&context);

        let (agent_state, executor) = if reconstruct == Some(true) {
            let mut db = VectorDB::new(context, db_config)
                .await
                .expect("failed to initialize VectorDB");

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

            let executor = ToolExecutor::new(db, embedder)
                .with_contents(reconstructed.contents)
                .expect("contents mismatch");
            (reconstructed.state, executor)
        } else if let Some(snap) = strata_agent::persist::load(&snapshot_path)
            .expect("failed to load snapshot")
        {
            // Restore from local snapshot: rebuild the MMR from entries.
            let mut db = VectorDB::new(context, db_config)
                .await
                .expect("failed to initialize VectorDB for snapshot restore");

            let local_soul_hash = strata_core::SoulHash::digest(soul.as_bytes());
            assert_eq!(
                local_soul_hash, snap.state.soul_hash,
                "local soul text does not match snapshot soul hash"
            );

            db.batch_append(snap.entries)
                .await
                .expect("snapshot batch_append failed");

            assert_eq!(
                snap.state.vector_index_root,
                db.root(),
                "snapshot root does not match reconstructed MMR root"
            );

            eprintln!(
                "restored from snapshot: {} entries, nonce {}",
                db.len(),
                snap.state.nonce.get()
            );

            let executor = ToolExecutor::new(db, embedder)
                .with_contents(snap.contents)
                .expect("snapshot contents mismatch");
            (snap.state, executor)
        } else {
            let db = VectorDB::new(context, db_config)
                .await
                .expect("failed to initialize VectorDB");
            let genesis = genesis_state(&soul);
            let executor = ToolExecutor::new(db, embedder);
            (genesis, executor)
        };

        // Wire optional on-chain posting.
        let posting = if let Ok(rpc_url) = std::env::var("RPC_URL") {
            let key_hex = std::env::var("OPERATOR_KEY")
                .expect("OPERATOR_KEY required with RPC_URL");
            let signer: PrivateKeySigner = key_hex.parse().expect("invalid OPERATOR_KEY");

            let contract_address = if let Ok(addr_str) = std::env::var("CONTRACT_ADDRESS") {
                addr_str.parse().expect("invalid CONTRACT_ADDRESS")
            } else {
                let genesis_root =
                    FixedBytes::from(*agent_state.vector_index_root.as_bytes());
                poster::deploy_contract(&rpc_url, signer.clone(), &soul, genesis_root)
                    .await
                    .expect("contract deploy failed")
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

        // Set up the batch background task if posting is configured.
        let (pending_batch, shutdown_tx): (Option<Arc<PendingBatch>>, Option<watch::Sender<bool>>) = if posting.is_some() {
            let pending = Arc::new(PendingBatch::default());
            let (tx, rx) = watch::channel(false);

            let batch_config = BatchConfig {
                interval: Duration::from_secs(post_interval_secs),
                wal_path: PathBuf::from(&wal_path),
            };

            eprintln!(
                "batch posting enabled: interval={}s, wal={}",
                post_interval_secs, wal_path
            );

            // Clone what the batch task needs. It borrows posting/prover from
            // the outer scope, but we need to move ownership.
            let batch_pending = Arc::clone(&pending);
            let batch_posting = PostingConfig {
                poster: PosterConfig {
                    rpc_url: posting.as_ref().unwrap().poster.rpc_url.clone(),
                    contract_address: posting.as_ref().unwrap().poster.contract_address,
                },
                signer: posting.as_ref().unwrap().signer.clone(),
            };
            let batch_prover = prover.clone();

            ::tokio::task::spawn(async move {
                batch::run(
                    batch_pending,
                    batch_posting,
                    batch_prover,
                    batch_config,
                    rx,
                )
                .await;
            });

            (Some(pending), Some(tx))
        } else {
            (None, None)
        };

        // ERC-8004 identity (optional — only needed for on-chain registration).
        // AGENT_ID=0 or unset triggers auto-mint of a new identity.
        let mut identity_config = match (
            std::env::var("REGISTRY_ADDRESS"),
            std::env::var("AGENT_BASE_URL"),
        ) {
            (Ok(reg), Ok(url)) => {
                let agent_id: u64 = std::env::var("AGENT_ID")
                    .ok()
                    .and_then(|id| id.parse().ok())
                    .unwrap_or(0);
                let registry_address: Address = reg.parse().expect("invalid REGISTRY_ADDRESS");
                eprintln!("identity: agent_id={agent_id} (0 = will mint)");
                IdentityConfig {
                    agent_id,
                    registry_address,
                    agent_base_url: url,
                    rpc_url: std::env::var("RPC_URL").unwrap_or_default(),
                }
            }
            _ => {
                eprintln!("identity: not configured (set REGISTRY_ADDRESS, AGENT_BASE_URL)");
                IdentityConfig {
                    agent_id: 0,
                    registry_address: Address::ZERO,
                    agent_base_url: String::new(),
                    rpc_url: String::new(),
                }
            }
        };

        let rollup_address = posting
            .as_ref()
            .map(|p| p.poster.contract_address)
            .unwrap_or(Address::ZERO);

        // Register on-chain identity (non-fatal, skipped if not configured).
        // agent_id==0 with a configured registry triggers auto-mint.
        if identity_config.registry_address != Address::ZERO {
            if let Ok(key_hex) = std::env::var("OPERATOR_KEY") {
                let signer: PrivateKeySigner = key_hex.parse().expect("invalid OPERATOR_KEY");
                match identity::register(&identity_config, signer, rollup_address).await {
                    Ok(id) => {
                        identity_config.agent_id = id;
                        eprintln!("ERC-8004 identity ready: agent #{id}");
                    }
                    Err(e) => eprintln!("ERC-8004 registration failed (non-fatal): {e}"),
                }
            }
        }

        let proofs_dir = std::env::current_dir()
            .expect("cannot determine cwd")
            .join("proofs");

        let mut app_state = AppState::new(
            AgentConfig {
                soul,
                state: agent_state,
            },
            llm_client,
            executor,
            pending_batch,
            identity_config,
            rollup_address,
        );
        app_state.proofs_dir = proofs_dir;
        app_state.snapshot_path = Some(snapshot_path.clone());
        let state = Arc::new(app_state);

        eprintln!("agent ready — POST http://{addr}/a2a");

        // Run server with graceful shutdown.
        let server_handle = ::tokio::task::spawn({
            let state = Arc::clone(&state);
            async move {
                server::run(state, addr).await.expect("server error");
            }
        });

        // Wait for Ctrl+C, then signal the batch task to flush.
        ::tokio::signal::ctrl_c().await.ok();
        eprintln!("shutting down...");
        if let Some(tx) = shutdown_tx {
            let _ = tx.send(true);
            // Give the batch task a moment to flush.
            ::tokio::time::sleep(Duration::from_secs(5)).await;
        }

        // Save snapshot before exit.
        let snap = state.snapshot().await;
        match strata_agent::persist::save(&snapshot_path, &snap) {
            Ok(()) => eprintln!(
                "snapshot saved: {} entries → {}",
                snap.entries.len(),
                snapshot_path.display()
            ),
            Err(e) => eprintln!("WARNING: failed to save snapshot: {e}"),
        }

        // Sync MMR journal to disk.
        if let Err(e) = state.sync_db().await {
            eprintln!("WARNING: failed to sync VectorDB: {e}");
        }

        server_handle.abort();
    });
}

/// Build the LLM client from environment variables.
fn make_llm_client() -> LlmClient {
    let env = |k: &str| std::env::var(k).ok();

    if let Some(base_url) = env("LLM_BASE_URL") {
        let api_key = env("LLM_API_KEY")
            .or_else(|| env("VENICE_API_KEY"))
            .expect("LLM_API_KEY (or VENICE_API_KEY) required with LLM_BASE_URL");
        let client = LlmClient::openai_compatible(base_url, api_key);
        let client = if let Some(model) = env("LLM_MODEL") {
            client.with_model(model)
        } else {
            client
        };
        eprintln!("llm: custom endpoint");
        return client;
    }

    if let Some(key) = env("VENICE_API_KEY") {
        let model = env("LLM_MODEL").unwrap_or_else(|| "llama-3.3-70b".into());
        let client = LlmClient::openai_compatible("https://api.venice.ai/api/v1", key)
            .with_model(&model);
        eprintln!("llm: venice ({model})");
        return client;
    }

    if let Some(key) = env("ANTHROPIC_API_KEY") {
        eprintln!("llm: anthropic");
        return LlmClient::anthropic(key);
    }

    panic!("No LLM provider configured. Set one of: LLM_BASE_URL+LLM_API_KEY, VENICE_API_KEY, or ANTHROPIC_API_KEY");
}

/// Build the embedding client from environment variables.
///
/// Priority: local model (EMBED_MODEL_DIR or default ./models/embed) → API embedders.
fn make_embedder() -> Box<dyn embed::Embedder> {
    let env = |k: &str| std::env::var(k).ok();

    // Try local embedder first (no API key needed).
    #[cfg(feature = "local-embed")]
    {
        let model_dir = env("EMBED_MODEL_DIR")
            .unwrap_or_else(|| "./models/embed".into());
        let dir = std::path::Path::new(&model_dir);
        if dir.join("model.onnx").exists() && dir.join("tokenizer.json").exists() {
            match embed::LocalEmbedder::mixedbread(&model_dir) {
                Ok(e) => {
                    eprintln!("embed: local ({})", model_dir);
                    return Box::new(e);
                }
                Err(e) => {
                    eprintln!("embed: local model failed to load: {e}, falling back to API");
                }
            }
        }
    }

    if let Some(base_url) = env("EMBED_BASE_URL") {
        let api_key = env("EMBED_API_KEY")
            .or_else(|| env("OPENAI_API_KEY"))
            .or_else(|| env("VENICE_API_KEY"))
            .expect("EMBED_API_KEY required with EMBED_BASE_URL");
        let mut e = ApiEmbedder::new(api_key).with_url(base_url);
        if let Some(model) = env("EMBED_MODEL") {
            e = e.with_model(model);
        }
        eprintln!("embed: custom endpoint");
        return Box::new(e);
    }

    if let Some(key) = env("OPENAI_API_KEY") {
        let mut e = ApiEmbedder::new(key);
        if let Some(model) = env("EMBED_MODEL") {
            e = e.with_model(model);
        }
        eprintln!("embed: openai");
        return Box::new(e);
    }

    if let Some(key) = env("VENICE_API_KEY") {
        let model = env("EMBED_MODEL").unwrap_or_else(|| "text-embedding-3-small".into());
        let e = ApiEmbedder::new(key)
            .with_url("https://api.venice.ai/api/v1/embeddings")
            .with_model(&model);
        eprintln!("embed: venice ({model})");
        return Box::new(e);
    }

    panic!("No embedding provider configured. Set EMBED_MODEL_DIR for local, or one of: EMBED_BASE_URL+EMBED_API_KEY, OPENAI_API_KEY, VENICE_API_KEY");
}

/// Best-effort .env file loader.
fn load_dotenv() {
    let Ok(content) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if std::env::var(key).is_err() {
                // SAFETY: called once at startup before any threads are spawned.
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
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
