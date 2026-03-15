// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title StrataRollup
/// @notice Rollup contract for a single Strata agent. Stores the latest proven
///         state root and accepts ZK-proven state transitions.
///
///         The verifier interface will be replaced with the real OpenVM Halo2
///         verifier once the proving pipeline is integrated.
contract StrataRollup {
    /// @notice Current MMR root of the agent's vector index.
    bytes32 public stateRoot;

    /// @notice Monotonically increasing transition counter.
    uint64 public nonce;

    /// @notice Keccak256 hash of the soul document text.
    bytes32 public soulHash;

    /// @notice Address authorized to submit transitions.
    address public operator;

    /// @notice Address of the ZK verifier contract (OpenVM Halo2).
    address public verifier;

    /// @notice Commitment to the guest executable (app_exe_commit).
    bytes32 public appExeCommit;

    /// @notice Commitment to the VM configuration (app_vm_commit).
    bytes32 public appVmCommit;

    /// @notice Emitted on every successful state transition.
    event StateTransition(
        uint64 indexed newNonce,
        bytes32 indexed newStateRoot
    );

    error OnlyOperator();
    error InvalidPublicValues();
    error StateMismatch();
    error VerificationFailed();

    constructor(
        string memory _soulText,
        address _verifier,
        address _operator,
        bytes32 _appExeCommit,
        bytes32 _appVmCommit,
        bytes32 _initialStateRoot
    ) {
        soulHash = keccak256(bytes(_soulText));
        verifier = _verifier;
        operator = _operator;
        appExeCommit = _appExeCommit;
        appVmCommit = _appVmCommit;
        stateRoot = _initialStateRoot;
    }

    /// @notice Submit a proven state transition.
    /// @param publicValues Public values from the ZK proof (104 bytes):
    ///        [0..32]   oldRoot  — must match current stateRoot
    ///        [32..64]  newRoot  — the post-transition state root
    ///        [64..72]  nonce    — must equal current nonce + 1 (u64 BE)
    ///        [72..104] soulHash — must match contract's soulHash
    /// @param proofData The ZK proof bytes.
    /// @dev The third parameter is raw memory content posted as calldata
    ///      for reconstruction/DA. Not read on-chain.
    function submitTransition(
        bytes calldata publicValues,
        bytes calldata proofData,
        bytes calldata /* memoryContent */
    ) external {
        if (msg.sender != operator) revert OnlyOperator();
        if (publicValues.length < 104) revert InvalidPublicValues();

        // Verify the ZK proof.
        _verify(publicValues, proofData);

        // Extract and validate public values.
        bytes32 oldRoot = bytes32(publicValues[:32]);
        bytes32 newRoot = bytes32(publicValues[32:64]);
        uint64 proofNonce = uint64(bytes8(publicValues[64:72]));
        bytes32 proofSoulHash = bytes32(publicValues[72:104]);

        // Verify state continuity: proof must chain from current on-chain state.
        if (oldRoot != stateRoot) revert StateMismatch();
        if (proofNonce != nonce + 1) revert StateMismatch();
        if (proofSoulHash != soulHash) revert StateMismatch();

        nonce = proofNonce;
        stateRoot = newRoot;

        emit StateTransition(nonce, newRoot);
    }

    /// @dev Internal verification hook. Override in tests with a mock.
    function _verify(
        bytes calldata publicValues,
        bytes calldata proofData
    ) internal view virtual {
        if (verifier == address(0)) revert VerificationFailed();

        (bool success, ) = verifier.staticcall(
            abi.encodeWithSignature(
                "verify(bytes,bytes,bytes32,bytes32)",
                publicValues,
                proofData,
                appExeCommit,
                appVmCommit
            )
        );
        if (!success) revert VerificationFailed();
    }
}
