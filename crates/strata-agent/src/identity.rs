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
        function register(string agentURI) external returns (uint256 agentId);
        function setAgentURI(uint256 agentId, string newURI) external;
        function setMetadata(uint256 agentId, string metadataKey, bytes metadataValue) external;
        function getMetadata(uint256 agentId, string metadataKey) external view returns (bytes);
        function tokenURI(uint256 tokenId) external view returns (string);
        function balanceOf(address owner) external view returns (uint256);
        function tokenOfOwnerByIndex(address owner, uint256 index) external view returns (uint256);
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
///
/// If `agent_id` is 0, mints a new agent identity and returns the new ID.
/// Otherwise updates the existing identity. Skips transactions if values already match.
pub async fn register(
    config: &IdentityConfig,
    signer: PrivateKeySigner,
    rollup_address: Address,
) -> Result<u64, AgentError> {
    let signer_address = signer.address();
    let wallet = EthereumWallet::from(signer);
    let rpc_url: reqwest::Url = config
        .rpc_url
        .parse()
        .map_err(|e| AgentError::Identity(format!("invalid RPC URL: {e}")))?;
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url);

    let registry = IAgentRegistry::new(config.registry_address, &provider);

    let desired_uri = format!(
        "{}/.well-known/agent-registration.json",
        config.agent_base_url.trim_end_matches('/')
    );

    // If no agent_id, find an existing token or mint a new one.
    let agent_id = if config.agent_id == 0 {
        // Check if the operator already owns a token on this registry.
        let owner = signer_address;
        let balance = registry
            .balanceOf(owner)
            .call()
            .await
            .unwrap_or_default();

        if balance > U256::ZERO {
            // Reuse the first token owned by this wallet.
            let token_id = registry
                .tokenOfOwnerByIndex(owner, U256::ZERO)
                .call()
                .await
                .map_err(|e| AgentError::Identity(format!("tokenOfOwnerByIndex failed: {e}")))?;
            let id: u64 = token_id.try_into().map_err(|_| {
                AgentError::Identity(format!("agent_id too large: {token_id}"))
            })?;
            eprintln!("found existing ERC-8004 agent #{id} owned by {owner}");

            // Ensure URI is up to date.
            let current_uri = registry.tokenURI(token_id).call().await.unwrap_or_default();
            if current_uri != desired_uri {
                eprintln!("updating agent URI: {desired_uri}");
                let tx_hash = registry
                    .setAgentURI(token_id, desired_uri)
                    .send()
                    .await
                    .map_err(|e| AgentError::Identity(format!("setAgentURI send failed: {e}")))?
                    .watch()
                    .await
                    .map_err(|e| AgentError::Identity(format!("setAgentURI watch failed: {e}")))?;
                eprintln!("setAgentURI confirmed: {tx_hash}");
            }

            token_id
        } else {
            // No existing token — mint a new one.
            eprintln!("minting new ERC-8004 agent identity...");
            let receipt = registry
                .register(desired_uri.clone())
                .send()
                .await
                .map_err(|e| AgentError::Identity(format!("register send failed: {e}")))?
                .get_receipt()
                .await
                .map_err(|e| AgentError::Identity(format!("register receipt failed: {e}")))?;

            let token_id = receipt
                .inner
                .logs()
                .iter()
                .find_map(|log| {
                    // ERC-721 Transfer(address,address,uint256) — topic[3] is tokenId
                    if log.topics().len() == 4 {
                        Some(U256::from_be_bytes(log.topics()[3].0))
                    } else {
                        None
                    }
                })
                .ok_or_else(|| AgentError::Identity("no Transfer event in register receipt".into()))?;

            let id: u64 = token_id.try_into().map_err(|_| {
                AgentError::Identity(format!("agent_id too large: {token_id}"))
            })?;
            eprintln!("minted ERC-8004 agent #{id} (tx: {})", receipt.transaction_hash);
            U256::from(id)
        }
    } else {
        let agent_id = U256::from(config.agent_id);

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

        agent_id
    };

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

    let id: u64 = agent_id.try_into().unwrap_or(0);
    Ok(id)
}
