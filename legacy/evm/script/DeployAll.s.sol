// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import "../src/Authentication.sol";
import "../src/RubbiToken.sol";
import "../src/ModalContract.sol";
import "../src/SubscriptionService.sol";
import "../src/SalaryStreaming.sol";
import "../src/WETH9.sol";
import "../src/MockARB.sol";
import "../src/UniswapV2Factory.sol";
import "../src/UniswapV2Router02.sol";
import "../src/interfaces/IUniswapV2.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @notice Single-command deployment for the entire Rubbi protocol + Uniswap V2 DEX.
///
/// Deploys:
///   1. Core contracts - Authentication, RubbiToken, ModalContract, SubscriptionService, SalaryStreaming
///   2. DEX contracts - WETH9, MockARB, UniswapV2Factory, UniswapV2Router02
///   3. Liquidity pools - WETH/RUB + ARB/RUB (via MockARB)
///
/// Usage:
///   forge script script/DeployAll.s.sol --rpc-url <RPC> --broadcast --verify
///
/// Environment variables:
///   PRIVATE_KEY              - deployer private key (required)
///   INITIAL_RUB_SUPPLY       - RUB initial supply (default: 1,000,000,000 ether)
///   PAYMENT_ADDRESS          - subscription payment address (default: deployer)
///   LIQUIDITY_ETH_WEI        - ETH for WETH/RUB pair (default: 0.01 ether)
///   LIQUIDITY_RUB            - RUB for WETH/RUB pair (default: 1000 ether)
///   LIQUIDITY_ARB_WEI        - ARB for ARB/RUB pair (default: 100 ether)
///   LIQUIDITY_RUB_ARB        - RUB for ARB/RUB pair (default: 1000 ether)
///
contract DeployAllScript is Script {
    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(privateKey);

        uint256 initialSupply = vm.envOr("INITIAL_RUB_SUPPLY", uint256(1_000_000_000 ether));
        address paymentAddr   = vm.envOr("PAYMENT_ADDRESS", deployer);

        uint256 ethForPair    = vm.envOr("LIQUIDITY_ETH_WEI", uint256(0.01 ether));
        uint256 rubForEthPair = vm.envOr("LIQUIDITY_RUB",     uint256(1000 ether));
        uint256 arbForPair    = vm.envOr("LIQUIDITY_ARB_WEI", uint256(100 ether));
        uint256 rubForArbPair = vm.envOr("LIQUIDITY_RUB_ARB", uint256(1000 ether));

        // =============================================
        //  PHASE 1 - Deploy Core Rubbi Contracts
        // =============================================
        console.log("=============================================");
        console.log("  PHASE 1: Core Rubbi Contracts");
        console.log("=============================================");

        vm.startBroadcast(privateKey);

        Authentication authentication = new Authentication();
        console.log("  Authentication :", address(authentication));

        RubbiToken rubbiToken = new RubbiToken(initialSupply);
        console.log("  RubbiToken     :", address(rubbiToken));
        console.log("  Supply         :", initialSupply / 1e18, "RUB");

        ModalContract modalContract = new ModalContract(address(rubbiToken));
        console.log("  ModalContract  :", address(modalContract));

        SubscriptionService subscriptionService = new SubscriptionService(
            address(modalContract),
            paymentAddr
        );
        console.log("  SubscriptionSvc:", address(subscriptionService));

        SalaryStreaming salaryStreaming = new SalaryStreaming(address(modalContract));
        console.log("  SalaryStreaming:", address(salaryStreaming));

        // =============================================
        //  PHASE 2 - Deploy Swap Infrastructure
        // =============================================
        console.log("\n=============================================");
        console.log("  PHASE 2: Swap Infrastructure");
        console.log("=============================================");

        WETH9 weth = new WETH9();
        console.log("  WETH9          :", address(weth));

        MockARB mockArb = new MockARB();
        console.log("  MockARB        :", address(mockArb));
        console.log("  Supply         :", 1000000, "ARB");

        UniswapV2Factory factory = new UniswapV2Factory(deployer);
        console.log("  Factory        :", address(factory));

        UniswapV2Router02 router = new UniswapV2Router02(
            address(factory),
            address(weth)
        );
        console.log("  Router         :", address(router));

        // =============================================
        //  PHASE 3 - WETH/RUB Liquidity Pool
        // =============================================
        console.log("\n=============================================");
        console.log("  PHASE 3: WETH/RUB Liquidity Pool");
        console.log("=============================================");

        IERC20(address(rubbiToken)).approve(address(router), rubForEthPair);

        (uint256 rubUsed, uint256 ethUsed, uint256 lpEthRub) = router.addLiquidityETH{value: ethForPair}(
            address(rubbiToken),
            rubForEthPair,
            (rubForEthPair * 95) / 100,
            (ethForPair * 95) / 100,
            deployer,
            block.timestamp + 600
        );

        address wethRubPair = factory.getPair(address(weth), address(rubbiToken));
        console.log("  Pair           :", wethRubPair);
        console.log("  RUB deposited  :", rubUsed / 1e18);
        console.log("  ETH deposited  :", ethUsed / 1e18);
        console.log("  LP tokens minted:", lpEthRub);

        // =============================================
        //  PHASE 4 - ARB/RUB Liquidity Pool
        // =============================================
        console.log("\n=============================================");
        console.log("  PHASE 4: ARB/RUB Liquidity Pool");
        console.log("=============================================");

        IERC20(address(mockArb)).approve(address(router), arbForPair);
        IERC20(address(rubbiToken)).approve(address(router), rubForArbPair);

        (uint256 arbUsed, uint256 rubUsedArb, uint256 lpArbRub) = router.addLiquidity(
            address(mockArb),
            address(rubbiToken),
            arbForPair,
            rubForArbPair,
            (arbForPair * 95) / 100,
            (rubForArbPair * 95) / 100,
            deployer,
            block.timestamp + 600
        );

        address arbRubPair = factory.getPair(address(mockArb), address(rubbiToken));
        console.log("  Pair           :", arbRubPair);
        console.log("  ARB deposited  :", arbUsed / 1e18);
        console.log("  RUB deposited  :", rubUsedArb / 1e18);
        console.log("  LP tokens minted:", lpArbRub);

        vm.stopBroadcast();

        // =============================================
        //  PHASE 5 - Deployment Summary
        // =============================================
        _printSummary(
            address(authentication),
            address(rubbiToken),
            address(modalContract),
            address(subscriptionService),
            address(salaryStreaming),
            address(weth),
            address(mockArb),
            address(factory),
            address(router),
            wethRubPair,
            arbRubPair
        );

        _printFrontendEnv(
            address(rubbiToken),
            address(router),
            address(weth),
            address(mockArb),
            address(authentication),
            address(modalContract),
            address(subscriptionService),
            address(salaryStreaming)
        );

        _writeDeploymentJson(
            address(authentication),
            address(rubbiToken),
            address(modalContract),
            address(subscriptionService),
            address(salaryStreaming),
            address(weth),
            address(mockArb),
            address(factory),
            address(router),
            wethRubPair,
            arbRubPair
        );
    }

    function _printSummary(
        address authentication,
        address rubbiToken,
        address modalContract,
        address subscriptionService,
        address salaryStreaming,
        address weth,
        address mockArb,
        address factory,
        address router,
        address wethRubPair,
        address arbRubPair
    ) internal {
        console.log("\n==============================================");
        console.log("          RUBBI DEPLOYMENT COMPLETE           ");
        console.log("==============================================");
        console.log("  CORE CONTRACTS");
        console.log("  Authentication     :", authentication);
        console.log("  RubbiToken (RUB)   :", rubbiToken);
        console.log("  ModalContract      :", modalContract);
        console.log("  SubscriptionService:", subscriptionService);
        console.log("  SalaryStreaming    :", salaryStreaming);
        console.log("");
        console.log("  SWAP INFRASTRUCTURE");
        console.log("  WETH9              :", weth);
        console.log("  MockARB (ARB)      :", mockArb);
        console.log("  UniswapV2Factory   :", factory);
        console.log("  UniswapV2Router02  :", router);
        console.log("");
        console.log("  LIQUIDITY POOLS");
        console.log("  WETH/RUB Pair      :", wethRubPair);
        console.log("  ARB/RUB Pair       :", arbRubPair);
        console.log("==============================================");
    }

    function _printFrontendEnv(
        address rubbiToken,
        address router,
        address weth,
        address mockArb,
        address authentication,
        address modalContract,
        address subscriptionService,
        address salaryStreaming
    ) internal {
        console.log("\n-----------------------------------------------");
        console.log("  FRONTEND .env.local");
        console.log("-----------------------------------------------");
        console.log("  NEXT_PUBLIC_AUTH_CONTRACT_ADDRESS=", vm.toString(authentication));
        console.log("  NEXT_PUBLIC_RUBBI_TOKEN_ADDRESS=", vm.toString(rubbiToken));
        console.log("  NEXT_PUBLIC_MODAL_CONTRACT_ADDRESS=", vm.toString(modalContract));
        console.log("  NEXT_PUBLIC_SUBSCRIPTION_SERVICE_ADDRESS=", vm.toString(subscriptionService));
        console.log("  NEXT_PUBLIC_SALARY_STREAMING_ADDRESS=", vm.toString(salaryStreaming));
        console.log("  NEXT_PUBLIC_UNISWAP_V2_ROUTER=", vm.toString(router));
        console.log("  NEXT_PUBLIC_ARB_TOKEN_ADDRESS=", vm.toString(mockArb));
        console.log("  NEXT_PUBLIC_WETH_ADDRESS=", vm.toString(weth));
        console.log("-----------------------------------------------");
    }

    function _writeDeploymentJson(
        address authentication,
        address rubbiToken,
        address modalContract,
        address subscriptionService,
        address salaryStreaming,
        address weth,
        address mockArb,
        address factory,
        address router,
        address wethRubPair,
        address arbRubPair
    ) internal {
        string memory json = vm.serializeString("", "authentication", vm.toString(authentication));
        json = vm.serializeString(json, "rubbiToken", vm.toString(rubbiToken));
        json = vm.serializeString(json, "modalContract", vm.toString(modalContract));
        json = vm.serializeString(json, "subscriptionService", vm.toString(subscriptionService));
        json = vm.serializeString(json, "salaryStreaming", vm.toString(salaryStreaming));
        json = vm.serializeString(json, "weth", vm.toString(weth));
        json = vm.serializeString(json, "mockArb", vm.toString(mockArb));
        json = vm.serializeString(json, "factory", vm.toString(factory));
        json = vm.serializeString(json, "router", vm.toString(router));
        json = vm.serializeString(json, "wethRubPair", vm.toString(wethRubPair));
        json = vm.serializeString(json, "arbRubPair", vm.toString(arbRubPair));

        vm.writeJson(json, "deployment.json");
        console.log("\n  Deployment addresses saved to: deployment.json");
    }
}
