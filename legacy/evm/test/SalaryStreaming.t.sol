// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";

import {SalaryStreaming} from "../src/SalaryStreaming.sol";
import {ModalContract} from "../src/ModalContract.sol";
import {RubbiToken} from "../src/RubbiToken.sol";

contract SalaryStreamingTest is Test {
    SalaryStreaming public salaryStreaming;
    ModalContract public modalContract;
    RubbiToken public token;

    address public recipient1 = address(1);
    address public recipient2 = address(2);

    function setUp() public {
        token = new RubbiToken(1_000_000 ether);
        modalContract = new ModalContract(address(token));
        salaryStreaming = new SalaryStreaming(address(modalContract));

        token.approve(address(modalContract), 100 ether);
        modalContract.deposit(20 ether);
    }

    function testCreateDailyStreams() public {
        SalaryStreaming.StreamDetails[] memory streamDetails = new SalaryStreaming.StreamDetails[](2);
        streamDetails[0] = SalaryStreaming.StreamDetails("Alice", recipient1, 1 ether);
        streamDetails[1] = SalaryStreaming.StreamDetails("Bob", recipient2, 2 ether);

        salaryStreaming.createStream(streamDetails, SalaryStreaming.IntervalType.Daily);

        SalaryStreaming.Stream[] memory streams = salaryStreaming.getAllDailyStreams();
        assertEq(streams.length, 2);
        assertEq(streams[0].recipient, recipient1);
        assertEq(streams[0].amount, 1 ether);
        assertEq(streams[0].name, "Alice");
        assertTrue(streams[0].active);
        assertEq(streams[1].recipient, recipient2);
        assertEq(streams[1].amount, 2 ether);
        assertEq(streams[1].name, "Bob");
    }

    function testPauseAndResumeDailyStream() public {
        SalaryStreaming.StreamDetails[] memory streamDetails = new SalaryStreaming.StreamDetails[](1);
        streamDetails[0] = SalaryStreaming.StreamDetails("Alice", recipient1, 1 ether);

        salaryStreaming.createStream(streamDetails, SalaryStreaming.IntervalType.Daily);

        salaryStreaming.pauseDailyStream(0);
        assertFalse(salaryStreaming.getAllDailyStreams()[0].active);

        salaryStreaming.resumeDailyStream(0);
        assertTrue(salaryStreaming.getAllDailyStreams()[0].active);
    }

    function testDisburseDailyTransfersTokensToRecipient() public {
        SalaryStreaming.StreamDetails[] memory streamDetails = new SalaryStreaming.StreamDetails[](1);
        streamDetails[0] = SalaryStreaming.StreamDetails("Alice", recipient1, 1 ether);

        salaryStreaming.createStream(streamDetails, SalaryStreaming.IntervalType.Daily);

        uint256 recipientBalanceBefore = token.balanceOf(recipient1);
        salaryStreaming.disburseDaily();

        assertEq(token.balanceOf(recipient1), recipientBalanceBefore + 1 ether);
        assertEq(modalContract.getBalances(address(this)), 16 ether);
    }
}
