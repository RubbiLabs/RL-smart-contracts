// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";

import "../src/SubscriptionService.sol";
import "../src/ModalContract.sol";
import "../src/RubbiToken.sol";

contract SubscriptionServiceTest is Test {
    RubbiToken public token;
    SubscriptionService public subscriptionService;
    ModalContract public modalContract;
    address public paymentAddress;

    function setUp() public {
        paymentAddress = address(0x123);
        token = new RubbiToken(1_000_000 ether);
        modalContract = new ModalContract(address(token));
        subscriptionService = new SubscriptionService(address(modalContract), paymentAddress);

        token.approve(address(modalContract), 100 ether);
        modalContract.deposit(20 ether);
    }

    function testAddSubscriptionPlan() public {
        subscriptionService.addSubscriptionPlan("Basic Plan", 5 ether);

        SubscriptionService.SubscriptionPlan[] memory plans = subscriptionService.getAllSubscriptionPlans();

        assertEq(plans.length, 1);
        assertEq(plans[0].name, "Basic Plan");
        assertEq(plans[0].fee, 5 ether);
        assertTrue(plans[0].active);
    }

    function testStartSubscription() public {
        subscriptionService.addSubscriptionPlan("Basic Plan", 5 ether);

        subscriptionService.startSubscription(0, "user@example.com", "secret");

        SubscriptionService.Subscriber[] memory subscriptions =
            subscriptionService.getSubscriptionsOfAddress(address(this));

        assertEq(subscriptions.length, 1);
        assertTrue(subscriptions[0].active);
        assertEq(subscriptions[0].subPlanId, 0);
        assertEq(modalContract.getBalances(address(this)), 19 ether);
    }

    function testPauseSubscription() public {
        subscriptionService.addSubscriptionPlan("Basic Plan", 5 ether);
        subscriptionService.startSubscription(0, "user@example.com", "secret");

        subscriptionService.pauseSubscription(0);

        assertFalse(subscriptionService.getSubscriptionsOfAddress(address(this))[0].active);
        assertFalse(subscriptionService.activeSubscriptions(address(this), 0));
        assertTrue(subscriptionService.stoppedSubscriptions(address(this), 0));
    }

    function testResumeSubscription() public {
        subscriptionService.addSubscriptionPlan("Basic Plan", 5 ether);
        subscriptionService.startSubscription(0, "user@example.com", "secret");
        subscriptionService.pauseSubscription(0);

        subscriptionService.resumeSubscription(0);

        assertTrue(subscriptionService.getSubscriptionsOfAddress(address(this))[0].active);
        assertTrue(subscriptionService.activeSubscriptions(address(this), 0));
        assertFalse(subscriptionService.stoppedSubscriptions(address(this), 0));
    }

    function testProcessSubscriptionPayments() public {
        subscriptionService.addSubscriptionPlan("Basic Plan", 5 ether);
        subscriptionService.startSubscription(0, "user@example.com", "secret");

        uint256 paymentBalanceBefore = token.balanceOf(paymentAddress);
        subscriptionService.processSubscriptionPayments();

        assertEq(token.balanceOf(paymentAddress), paymentBalanceBefore + 5 ether);
        assertEq(modalContract.getBalances(address(this)), 14 ether);
    }
}
