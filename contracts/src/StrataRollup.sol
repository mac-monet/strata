// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title StrataRollup
/// @notice Rollup contract for a single Strata agent. Stores the latest proven
///         state root and accepts operator-signed state transitions.
///         Verification is off-chain via STARK proofs; on-chain ZK verification
///         can be added by overriding `_verify` in a derived contract.
contract StrataRollup {
    /// @notice Current MMR root of the agent's vector index.
    bytes32 public stateRoot;

    /// @notice Monotonically increasing transition counter.
    uint64 public nonce;

    /// @notice Keccak256 hash of the soul document text.
    bytes32 public soulHash;

    /// @notice Address authorized to submit transitions.
    address public operator;

    /// @notice Emitted on every successful state transition.
    event StateTransition(
        uint64 indexed newNonce,
        bytes32 indexed newStateRoot
    );

    error OnlyOperator();
    error InvalidPublicValues();
    error StateMismatch();

    constructor(
        string memory _soulText,
        address _operator,
        bytes32 _initialStateRoot
    ) {
        soulHash = keccak256(bytes(_soulText));
        operator = _operator;
        stateRoot = _initialStateRoot;
    }

    /// @notice Submit a proven state transition (single or batch).
    /// @param publicValues Public values from the ZK proof (112 bytes):
    ///        [0..32]   oldRoot    — must match current stateRoot
    ///        [32..64]  newRoot    — the post-transition state root
    ///        [64..72]  startNonce — must equal current nonce + 1 (u64 BE)
    ///        [72..80]  endNonce   — must be >= startNonce (u64 BE)
    ///        [80..112] soulHash   — must match contract's soulHash
    /// @param proofData The ZK proof bytes.
    /// @dev The third parameter is raw memory content posted as calldata
    ///      for reconstruction/DA. Not read on-chain.
    function submitTransition(
        bytes calldata publicValues,
        bytes calldata proofData,
        bytes calldata /* memoryContent */
    ) external {
        if (msg.sender != operator) revert OnlyOperator();
        if (publicValues.length < 112) revert InvalidPublicValues();

        // Verify the ZK proof.
        _verify(publicValues, proofData);

        // Extract and validate public values.
        bytes32 oldRoot = bytes32(publicValues[:32]);
        bytes32 newRoot = bytes32(publicValues[32:64]);
        uint64 startNonce = uint64(bytes8(publicValues[64:72]));
        uint64 endNonce = uint64(bytes8(publicValues[72:80]));
        bytes32 proofSoulHash = bytes32(publicValues[80:112]);

        // Verify state continuity: proof must chain from current on-chain state.
        if (oldRoot != stateRoot) revert StateMismatch();
        if (startNonce != nonce + 1) revert StateMismatch();
        if (endNonce < startNonce) revert StateMismatch();
        if (proofSoulHash != soulHash) revert StateMismatch();

        nonce = endNonce;
        stateRoot = newRoot;

        emit StateTransition(nonce, newRoot);
    }

    /// @dev Verification hook — no-op for now. Override in a derived contract
    ///      (e.g. VerifiedStrataRollup) to add on-chain ZK proof verification
    ///      via OpenVM Halo2 or a proving network.
    function _verify(
        bytes calldata, /* publicValues */
        bytes calldata  /* proofData */
    ) internal view virtual {
        // Intentionally empty — verification is off-chain via STARK proofs.
    }
}
