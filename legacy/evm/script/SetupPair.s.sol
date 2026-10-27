// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import "../src/interfaces/IUniswapV2.sol";
import "../src/WETH9.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @notice Utility script to add liquidity to existing pairs.
///
/// Only needed if you want to add more liquidity after initial deployment.
/// DeployAll.s.sol handles initial pair creation and liquidity.
///
/// Usage:
///   forge script script/SetupPair.s.sol --rpc-url <RPC> --broadcast
///
contract SetupPairScript is Script {
    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address routerAddr = vm.envAddress("UNISWAP_V2_ROUTER");
        address rubToken = vm.envAddress("RUBBI_TOKEN_ADDRESS");
        address wethAddr = vm.envAddress("WETH_ADDRESS");

        uint256 rubAmount = vm.envOr("LIQUIDITY_RUB", uint256(1000 ether));
        uint256 ethAmount = vm.envOr("LIQUIDITY_ETH_WEI", uint256(0.01 ether));

        IUniswapV2Router02 router = IUniswapV2Router02(routerAddr);

        vm.startBroadcast(privateKey);

        IERC20(rubToken).approve(routerAddr, rubAmount);

        (uint256 amountToken, uint256 amountETH, uint256 liquidity) = router.addLiquidityETH{value: ethAmount}(
            rubToken,
            rubAmount,
            (rubAmount * 95) / 100,
            (ethAmount * 95) / 100,
            msg.sender,
            block.timestamp + 600
        );

        vm.stopBroadcast();

        console.log("WETH/RUB liquidity added:");
        console.log("  RUB:", amountToken / 1e18);
        console.log("  ETH:", amountETH / 1e18);
        console.log("  LP:", liquidity);
    }
}
