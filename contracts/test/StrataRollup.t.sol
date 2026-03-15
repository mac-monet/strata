// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test} from "forge-std/Test.sol";
import {StrataRollup} from "../src/StrataRollup.sol";

/// @dev Test harness that bypasses ZK verification.
contract MockStrataRollup is StrataRollup {
    constructor(
        string memory _soulText,
        address _operator,
        bytes32 _initialStateRoot
    )
        StrataRollup(
            _soulText,
            address(0), // no verifier needed for mock
            _operator,
            bytes32(0),
            bytes32(0),
            _initialStateRoot
        )
    {}

    /// @dev Always succeeds — skips ZK verification.
    function _verify(
        bytes calldata,
        bytes calldata
    ) internal pure override {
        // no-op: mock always passes
    }
}

contract StrataRollupTest is Test {
    MockStrataRollup public rollup;
    address public operator = address(0xBEEF);
    bytes32 public initialRoot = bytes32(uint256(0x1234));
    string public soulText = "You are a helpful assistant.";

    event StateTransition(uint64 indexed newNonce, bytes32 indexed newStateRoot);

    function setUp() public {
        rollup = new MockStrataRollup(
            soulText,
            operator,
            initialRoot
        );
    }

    /// @dev Build the 104-byte publicValues blob matching the guest layout.
    function _buildPublicValues(
        bytes32 oldRoot,
        bytes32 newRoot,
        uint64 newNonce,
        bytes32 _soulHash
    ) internal pure returns (bytes memory) {
        return abi.encodePacked(oldRoot, newRoot, newNonce, _soulHash);
    }

    function test_deployment() public view {
        assertEq(rollup.stateRoot(), initialRoot);
        assertEq(rollup.nonce(), 0);
        assertEq(rollup.operator(), operator);
        assertEq(rollup.soulHash(), keccak256(bytes(soulText)));
    }

    function test_submitTransition() public {
        bytes32 newRoot = bytes32(uint256(0xABCD));
        bytes memory pv = _buildPublicValues(
            initialRoot, newRoot, 1, keccak256(bytes(soulText))
        );

        vm.prank(operator);
        rollup.submitTransition(pv, "", "");

        assertEq(rollup.stateRoot(), newRoot);
        assertEq(rollup.nonce(), 1);
    }

    function test_submitTransition_emits_event() public {
        bytes32 newRoot = bytes32(uint256(0xABCD));
        bytes memory pv = _buildPublicValues(
            initialRoot, newRoot, 1, keccak256(bytes(soulText))
        );

        vm.expectEmit(true, true, false, false);
        emit StateTransition(1, newRoot);

        vm.prank(operator);
        rollup.submitTransition(pv, "", "");
    }

    function test_sequential_transitions() public {
        bytes32 currentRoot = initialRoot;
        bytes32 soul = keccak256(bytes(soulText));

        for (uint64 i = 1; i <= 5; i++) {
            bytes32 newRoot = bytes32(uint256(i * 0x100));
            bytes memory pv = _buildPublicValues(currentRoot, newRoot, i, soul);

            vm.prank(operator);
            rollup.submitTransition(pv, "", "");

            assertEq(rollup.nonce(), i);
            assertEq(rollup.stateRoot(), newRoot);
            currentRoot = newRoot;
        }
    }

    function test_revert_nonOperator() public {
        bytes memory pv = _buildPublicValues(
            initialRoot, bytes32(0), 1, keccak256(bytes(soulText))
        );

        vm.prank(address(0xDEAD));
        vm.expectRevert(StrataRollup.OnlyOperator.selector);
        rollup.submitTransition(pv, "", "");
    }

    function test_revert_publicValues_too_short() public {
        vm.prank(operator);
        vm.expectRevert(StrataRollup.InvalidPublicValues.selector);
        rollup.submitTransition("", "", "");

        // 32 bytes — was enough before, now too short (need 104)
        vm.prank(operator);
        vm.expectRevert(StrataRollup.InvalidPublicValues.selector);
        rollup.submitTransition(abi.encodePacked(bytes32(0)), "", "");

        // 103 bytes — one short
        vm.prank(operator);
        vm.expectRevert(StrataRollup.InvalidPublicValues.selector);
        rollup.submitTransition(new bytes(103), "", "");
    }

    function test_revert_wrong_oldRoot() public {
        bytes32 wrongOldRoot = bytes32(uint256(0xDEAD));
        bytes memory pv = _buildPublicValues(
            wrongOldRoot, bytes32(uint256(0xBEEF)), 1, keccak256(bytes(soulText))
        );

        vm.prank(operator);
        vm.expectRevert(StrataRollup.StateMismatch.selector);
        rollup.submitTransition(pv, "", "");
    }

    function test_revert_wrong_nonce() public {
        bytes memory pv = _buildPublicValues(
            initialRoot, bytes32(uint256(0xBEEF)), 99, keccak256(bytes(soulText))
        );

        vm.prank(operator);
        vm.expectRevert(StrataRollup.StateMismatch.selector);
        rollup.submitTransition(pv, "", "");
    }

    function test_revert_wrong_soulHash() public {
        bytes memory pv = _buildPublicValues(
            initialRoot, bytes32(uint256(0xBEEF)), 1, bytes32(uint256(0xBAD))
        );

        vm.prank(operator);
        vm.expectRevert(StrataRollup.StateMismatch.selector);
        rollup.submitTransition(pv, "", "");
    }

    function test_memoryContent_is_calldata_only() public {
        bytes32 newRoot = bytes32(uint256(0x9999));
        bytes memory pv = _buildPublicValues(
            initialRoot, newRoot, 1, keccak256(bytes(soulText))
        );
        bytes memory content = "some memory content for DA";

        vm.prank(operator);
        rollup.submitTransition(pv, "", content);

        assertEq(rollup.stateRoot(), newRoot);
    }
}
