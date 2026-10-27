// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import "../src/WETH9.sol";
import "../src/UniswapV2Factory.sol";
import "../src/UniswapV2Router02.sol";
import "../src/RubbiToken.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @notice Full deployment script for the Rubbi swap infrastructure.
///
/// Deploys:
/// 1. WETH9 (Wrapped ETH)
/// 2. UniswapV2Factory
/// 3. UniswapV2Router02
/// 4. RubbiToken (RUB) — if not already deployed
/// 5. Creates WETH/RUB pair
/// 6. Adds initial liquidity
///
/// Usage:
///   forge script script/DeploySwap.s.sol --rpc-url <RPC> --broadcast --verify
///
/// Environment variables:
///   PRIVATE_KEY                   — deployer private key
///   RUBBI_TOKEN_ADDRESS           — (optional) existing RUB token; if set, skips RUB deployment
///   INITIAL_RUB_SUPPLY            — (optional) RUB supply if deploying new token (default: 1B)
///   LIQUIDITY_RUB_AMOUNT          — RUB tokens for initial liquidity (default: 320000)
///   LIQUIDITY_ETH_AMOUNT_WEI      — ETH for initial liquidity in wei (default: 100 ether)
///
contract DeploySwapScript is Script {
    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(privateKey);

        vm.startBroadcast(privateKey);

        // ─────────────────────────────────────────────
        // 1. Deploy WETH9
        // ─────────────────────────────────────────────
        WETH9 weth = new WETH9();
        console.log("WETH9 deployed at:", address(weth));

        // ─────────────────────────────────────────────
        // 2. Deploy UniswapV2Factory
        // ─────────────────────────────────────────────
        UniswapV2Factory factory = new UniswapV2Factory(deployer);
        console.log("Factory deployed at:", address(factory));

        // ─────────────────────────────────────────────
        // 3. Deploy UniswapV2Router02
        // ─────────────────────────────────────────────
        UniswapV2Router02 router = new UniswapV2Router02(
            address(factory),
            address(weth)
        );
        console.log("Router deployed at:", address(router));

        // ─────────────────────────────────────────────
        // 4. Deploy or use existing RubbiToken
        // ─────────────────────────────────────────────
        RubbiToken rubToken;
        address existingRub = vm.envOr("RUBBI_TOKEN_ADDRESS", address(0));

        if (existingRub == address(0)) {
            uint256 initialSupply = vm.envOr("INITIAL_RUB_SUPPLY", uint256(1_000_000_000 ether));
            rubToken = new RubbiToken(initialSupply);
            console.log("RUB Token deployed at:", address(rubToken));
            console.log("Initial supply:", initialSupply / 1e18, "RUB");
        } else {
            rubToken = RubbiToken(existingRub);
            console.log("Using existing RUB Token at:", existingRub);
        }

        vm.stopBroadcast();

        // ─────────────────────────────────────────────
        // 5. Print deployment summary
        // ─────────────────────────────────────────────
        console.log("\n========================================");
        console.log("   DEPLOYMENT COMPLETE");
        console.log("========================================");
        console.log("WETH:   ", address(weth));
        console.log("Factory:", address(factory));
        console.log("Router: ", address(router));
        console.log("RUB:    ", address(rubToken));
        console.log("========================================");

        // ─────────────────────────────────────────────
        // 6. Create pair and add liquidity
        //    (must be in a separate broadcast since
        //     we need to fund the deployer with ETH
        //     and approve tokens)
        // ─────────────────────────────────────────────
        console.log("\nNext step: Run SetupPair.s.sol to create the pair and add liquidity.");
        console.log("Set these env vars:");
        console.log("  UNISWAP_V2_FACTORY=", vm.toString(address(factory)));
        console.log("  UNISWAP_V2_ROUTER=", vm.toString(address(router)));
        console.log("  RUBBI_TOKEN_ADDRESS=", vm.toString(address(rubToken)));
        console.log("  WETH_ADDRESS=", vm.toString(address(weth)));
    }
}
