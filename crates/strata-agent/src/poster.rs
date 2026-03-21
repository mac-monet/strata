//! L2 posting via alloy — deploys and submits transitions to `StrataRollup`.
//!
//! Uses Alloy's Anvil node bindings for local fork testing.

use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, FixedBytes},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
};
use commonware_codec::Encode;

use crate::error::AgentError;
use crate::pipeline::TransitionOutput;

// Generate type-safe bindings from the foundry JSON artifact.
// ABI + bytecode stay in sync with the Solidity source automatically.
sol! {
    #[sol(rpc, all_derives)]
    StrataRollup,
    "../../contracts/out/StrataRollup.sol/StrataRollup.json"
}

// Mock contract that skips ZK verification — used for demo and testing.
sol! {
    #[sol(rpc, all_derives)]
    MockStrataRollup,
    "../../contracts/out/StrataRollup.t.sol/MockStrataRollup.json"
}

/// Configuration for posting transitions on-chain.
#[derive(Clone, Debug)]
pub struct PosterConfig {
    /// RPC endpoint URL (e.g., local Anvil or Base).
    pub rpc_url: String,
    /// Deployed `StrataRollup` contract address.
    pub contract_address: Address,
}

/// Deploy a [`MockStrataRollup`] contract (skips ZK verification).
///
/// Takes fewer args than [`deploy_contract`] — no verifier or app commits.
pub async fn deploy_mock_contract(
    rpc_url: &str,
    signer: PrivateKeySigner,
    soul_text: &str,
    initial_state_root: FixedBytes<32>,
) -> Result<Address, AgentError> {
    let operator = signer.address();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(parse_rpc_url(rpc_url)?);

    let contract = MockStrataRollup::deploy(
        &provider,
        soul_text.to_string(),
        operator,
        initial_state_root,
    )
    .await
    .map_err(|e| AgentError::Poster(format!("mock deploy failed: {e}")))?;

    Ok(*contract.address())
}

fn parse_rpc_url(rpc_url: &str) -> Result<reqwest::Url, AgentError> {
    rpc_url
        .parse()
        .map_err(|e| AgentError::Poster(format!("invalid RPC URL: {e}")))
}

/// Deploy a new `StrataRollup` contract.
///
/// Returns the deployed contract address.
pub async fn deploy_contract(
    rpc_url: &str,
    signer: PrivateKeySigner,
    soul_text: &str,
    verifier: Address,
    app_exe_commit: FixedBytes<32>,
    app_vm_commit: FixedBytes<32>,
    initial_state_root: FixedBytes<32>,
) -> Result<Address, AgentError> {
    let operator = signer.address();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(parse_rpc_url(rpc_url)?);

    let contract = StrataRollup::deploy(
        &provider,
        soul_text.to_string(),
        verifier,
        operator,
        app_exe_commit,
        app_vm_commit,
        initial_state_root,
    )
    .await
    .map_err(|e| AgentError::Poster(format!("deploy failed: {e}")))?;

    Ok(*contract.address())
}

/// Submit a proven transition to the `StrataRollup` contract.
///
/// `proof_bytes` and `public_values` come from the prover output.
/// `TransitionRecord` is encoded via `commonware_codec::Encode` as
/// `memoryContent` calldata (posted for DA, not read on-chain).
pub async fn post(
    config: &PosterConfig,
    signer: PrivateKeySigner,
    proof_bytes: Vec<u8>,
    public_values: [u8; 104],
    transition: &TransitionOutput,
) -> Result<alloy::primitives::TxHash, AgentError> {
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(parse_rpc_url(&config.rpc_url)?);

    let contract = StrataRollup::new(config.contract_address, &provider);
    let memory_content = transition.record.encode();

    let tx_hash = contract
        .submitTransition(
            Bytes::copy_from_slice(&public_values),
            Bytes::from(proof_bytes),
            Bytes::from(memory_content),
        )
        .send()
        .await
        .map_err(|e| AgentError::Poster(format!("send failed: {e}")))?
        .watch()
        .await
        .map_err(|e| AgentError::Poster(format!("watch failed: {e}")))?;

    Ok(tx_hash)
}

/// Read the current nonce from the contract.
pub async fn read_nonce(config: &PosterConfig) -> Result<u64, AgentError> {
    let provider = ProviderBuilder::new().connect_http(parse_rpc_url(&config.rpc_url)?);
    let contract = StrataRollup::new(config.contract_address, &provider);
    let nonce = contract
        .nonce()
        .call()
        .await
        .map_err(|e| AgentError::Poster(format!("read nonce failed: {e}")))?;
    Ok(nonce)
}

/// Read the soul hash from the contract.
pub async fn read_soul_hash(config: &PosterConfig) -> Result<FixedBytes<32>, AgentError> {
    let provider = ProviderBuilder::new().connect_http(parse_rpc_url(&config.rpc_url)?);
    let contract = StrataRollup::new(config.contract_address, &provider);
    let hash = contract
        .soulHash()
        .call()
        .await
        .map_err(|e| AgentError::Poster(format!("read soul hash failed: {e}")))?;
    Ok(hash)
}

/// Read the current state root from the contract.
pub async fn read_state_root(config: &PosterConfig) -> Result<FixedBytes<32>, AgentError> {
    let provider = ProviderBuilder::new().connect_http(parse_rpc_url(&config.rpc_url)?);
    let contract = StrataRollup::new(config.contract_address, &provider);
    let root = contract
        .stateRoot()
        .call()
        .await
        .map_err(|e| AgentError::Poster(format!("read state root failed: {e}")))?;
    Ok(root)
}
