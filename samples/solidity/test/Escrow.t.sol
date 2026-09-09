// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {MilestoneEscrow} from "../src/Escrow.sol";

contract EscrowTest is Test {
    MilestoneEscrow internal escrow;

    address internal owner = address(this);
    address internal payer = address(0xA11CE);
    address internal payee = address(0xB0B);

    function setUp() public {
        escrow = new MilestoneEscrow();
        vm.deal(payer, 10 ether);
    }

    function test_DepositIncreasesTheDepositorsBalance() public {
        vm.prank(payer);
        escrow.deposit{value: 1 ether}();

        assertEq(escrow.balanceOf(payer), 1 ether);
    }

    function test_ADepositOfNothingIsRefused() public {
        vm.prank(payer);
        vm.expectRevert();
        escrow.deposit{value: 0}();
    }

    function test_OnlyTheOwnerCanRelease() public {
        vm.prank(payer);
        escrow.deposit{value: 1 ether}();

        vm.prank(payee);
        vm.expectRevert();
        escrow.release(0);
    }

    function test_BalanceOfAnAddressThatNeverPaidIsZero() public view {
        assertEq(escrow.balanceOf(payee), 0);
    }

    /// forge-config: default.fuzz.runs = 256
    function testFuzz_DepositThenBalanceIsWhatWasSent(uint96 amount) public {
        vm.assume(amount > 0);
        vm.deal(payer, amount);

        vm.prank(payer);
        escrow.deposit{value: amount}();

        assertEq(escrow.balanceOf(payer), amount);
    }
}
