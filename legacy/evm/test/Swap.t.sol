// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";
import "../src/WETH9.sol";
import "../src/UniswapV2Factory.sol";
import "../src/UniswapV2Router02.sol";
import "../src/RubbiToken.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract SwapTest is Test {
    WETH9 public weth;
    UniswapV2Factory public factory;
    UniswapV2Router02 public router;
    RubbiToken public rub;

    address public alice = makeAddr("alice");
    address public bob = makeAddr("bob");

    function setUp() public {
        // Deploy WETH
        weth = new WETH9();

        // Deploy Factory
        factory = new UniswapV2Factory(address(this));

        // Deploy Router
        router = new UniswapV2Router02(address(factory), address(weth));

        // Deploy RUB token with 1M supply to this contract
        rub = new RubbiToken(1_000_000 ether);

        // Create WETH/RUB pair
        factory.createPair(address(weth), address(rub));
    }

    function test_PairCreated() public view {
        address pair = factory.getPair(address(weth), address(rub));
        assertTrue(pair != address(0), "Pair should exist");
        assertEq(factory.allPairsLength(), 1, "Should have 1 pair");
    }

    function test_AddLiquidity() public {
        address pair = factory.getPair(address(weth), address(rub));

        // Approve RUB to router
        rub.approve(address(router), 320_000 ether);

        // Add liquidity: 100 ETH + 320,000 RUB
        (uint256 amountToken, uint256 amountETH, uint256 liquidity) = router.addLiquidityETH{value: 100 ether}(
            address(rub),
            320_000 ether,
            0,
            0,
            address(this),
            block.timestamp + 600
        );

        assertTrue(amountToken > 0, "RUB should be deposited");
        assertTrue(amountETH > 0, "ETH should be deposited");
        assertTrue(liquidity > 0, "LP tokens should be minted");

        // Check reserves
        (uint112 reserve0, uint112 reserve1,) = IUniswapV2Pair(pair).getReserves();
        assertTrue(reserve0 > 0, "Reserve0 should be > 0");
        assertTrue(reserve1 > 0, "Reserve1 should be > 0");
    }

    function test_SwapETHForRUB() public {
        // First add liquidity
        rub.approve(address(router), 320_000 ether);
        router.addLiquidityETH{value: 100 ether}(
            address(rub),
            320_000 ether,
            0, 0,
            address(this),
            block.timestamp + 600
        );

        // Record balances before swap
        uint256 rubBefore = rub.balanceOf(alice);
        uint256 ethBefore = alice.balance;

        // Alice swaps 1 ETH for RUB
        vm.deal(alice, 10 ether);
        vm.startPrank(alice);

        address[] memory path = new address[](2);
        path[0] = address(weth);
        path[1] = address(rub);

        uint256[] memory amounts = router.swapExactETHForTokens{value: 1 ether}(
            0, // accept any amount
            path,
            alice,
            block.timestamp + 600
        );

        vm.stopPrank();

        // Alice should have received RUB
        uint256 rubAfter = rub.balanceOf(alice);
        assertTrue(rubAfter > rubBefore, "Alice should have received RUB");
        assertTrue(amounts[1] > 0, "Amount out should be > 0");

        // Output should be roughly 3200 RUB (1 ETH = 3200 RUB at our price)
        // Minus 0.3% fee
        uint256 rubReceived = rubAfter - rubBefore;
        assertTrue(rubReceived > 3000 ether, "Should receive roughly 3200 RUB (minus fee)");
        assertTrue(rubReceived < 3200 ether, "Should receive less than 3200 RUB (fee deducted)");
    }

    function test_SwapRUBForETH() public {
        // Add liquidity
        rub.approve(address(router), 320_000 ether);
        router.addLiquidityETH{value: 100 ether}(
            address(rub),
            320_000 ether,
            0, 0,
            address(this),
            block.timestamp + 600
        );

        // Give Alice some RUB
        rub.transfer(alice, 10_000 ether);

        // Alice swaps 3200 RUB for ETH
        vm.startPrank(alice);
        rub.approve(address(router), 3200 ether);

        address[] memory path = new address[](2);
        path[0] = address(rub);
        path[1] = address(weth);

        uint256 ethBefore = alice.balance;

        uint256[] memory amounts = router.swapExactTokensForETH(
            3200 ether,
            0,
            path,
            alice,
            block.timestamp + 600
        );

        vm.stopPrank();

        uint256 ethReceived = alice.balance - ethBefore;
        assertTrue(ethReceived > 0, "Alice should have received ETH");
        assertTrue(amounts[1] > 0, "Amount out should be > 0");
    }

    function test_GetAmountsOut() public {
        // Add liquidity first
        rub.approve(address(router), 320_000 ether);
        router.addLiquidityETH{value: 100 ether}(
            address(rub),
            320_000 ether,
            0, 0,
            address(this),
            block.timestamp + 600
        );

        address[] memory path = new address[](2);
        path[0] = address(weth);
        path[1] = address(rub);

        // Get quote for 1 ETH
        uint256[] memory amounts = router.getAmountsOut(1 ether, path);
        assertTrue(amounts[0] == 1 ether, "Input should be 1 ether");
        assertTrue(amounts[1] > 0, "Output should be > 0");

        // At 1 ETH = 3200 RUB with 0.3% fee:
        // amountInWithFee = 1e18 * 997 = 997e15
        // numerator = 997e15 * 320000e18 = 319040000e33
        // denominator = 100e18 * 1000 + 997e15 = 100000e18 + 997e15 = 100997e15
        // amountOut = 319040000e33 / 100997e15 ≈ 3158906...
        // Roughly ~3158 RUB
        assertTrue(amounts[1] > 3100 ether, "Should get roughly 3158 RUB");
        assertTrue(amounts[1] < 3200 ether, "Should be less than 3200 due to fee");
    }

    function test_OnlyOwnerCanSetFeeTo() public {
        vm.prank(alice);
        vm.expectRevert("UniswapV2: FORBIDDEN");
        factory.setFeeTo(alice);
    }

    receive() external payable {}
}
