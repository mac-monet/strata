//! ERC-8004 identity registration — sets agent URI and metadata on the registry.

use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, U256},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolValue,
};

use crate::error::AgentError;

sol! {
    #[sol(rpc)]
    interface IAgentRegistry {
        function setAgentURI(uint256 agentId, string newURI) external;
        function setMetadata(uint256 agentId, string metadataKey, bytes metadataValue) external;
        function getMetadata(uint256 agentId, string metadataKey) external view returns (bytes);
        function tokenURI(uint256 tokenId) external view returns (string);
    }
}

/// Configuration for ERC-8004 identity registration.
#[derive(Clone, Debug)]
pub struct IdentityConfig {
    pub agent_id: u64,
    pub registry_address: Address,
    pub agent_base_url: String,
    pub rpc_url: String,
}

/// Register the agent's URI and rollup contract metadata on the ERC-8004 registry.
/// Skips transactions if the on-chain values already match.
pub async fn register(
    config: &IdentityConfig,
    signer: PrivateKeySigner,
    rollup_address: Address,
) -> Result<(), AgentError> {
    let wallet = EthereumWallet::from(signer);
    let rpc_url: reqwest::Url = config
        .rpc_url
        .parse()
        .map_err(|e| AgentError::Identity(format!("invalid RPC URL: {e}")))?;
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url);

    let registry = IAgentRegistry::new(config.registry_address, &provider);
    let agent_id = U256::from(config.agent_id);

    let desired_uri = format!(
        "{}/.well-known/agent-registration.json",
        config.agent_base_url.trim_end_matches('/')
    );

    // 1. Check and set agent URI.
    let current_uri = match registry.tokenURI(agent_id).call().await {
        Ok(uri) => uri,
        Err(_) => String::new(),
    };

    if current_uri == desired_uri {
        eprintln!("agent URI already set, skipping");
    } else {
        eprintln!("setting agent URI: {desired_uri}");
        let tx_hash = registry
            .setAgentURI(agent_id, desired_uri)
            .send()
            .await
            .map_err(|e| AgentError::Identity(format!("setAgentURI send failed: {e}")))?
            .watch()
            .await
            .map_err(|e| AgentError::Identity(format!("setAgentURI watch failed: {e}")))?;
        eprintln!("setAgentURI confirmed: {tx_hash}");
    }

    // 2. Check and set rollup contract metadata.
    let desired_metadata = Bytes::from(rollup_address.abi_encode());

    let current_metadata = match registry
        .getMetadata(agent_id, "strata.rollupContract".to_string())
        .call()
        .await
    {
        Ok(meta) => meta,
        Err(_) => Bytes::new(),
    };

    if current_metadata == desired_metadata {
        eprintln!("strata.rollupContract metadata already set, skipping");
    } else {
        eprintln!("setting strata.rollupContract metadata for rollup {rollup_address}");
        let tx_hash = registry
            .setMetadata(
                agent_id,
                "strata.rollupContract".to_string(),
                desired_metadata,
            )
            .send()
            .await
            .map_err(|e| AgentError::Identity(format!("setMetadata send failed: {e}")))?
            .watch()
            .await
            .map_err(|e| AgentError::Identity(format!("setMetadata watch failed: {e}")))?;
        eprintln!("setMetadata confirmed: {tx_hash}");
    }

    Ok(())
}
