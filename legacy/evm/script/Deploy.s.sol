// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import "../src/SalaryStreaming.sol";
import "../src/Authentication.sol";
import "../src/ModalContract.sol";
import "../src/RubbiToken.sol";
import "../src/SubscriptionService.sol";

contract DeployScript is Script {
    Authentication authentication;
    SalaryStreaming salaryStreaming;
    SubscriptionService subscriptionService;
    ModalContract modalContract;
    RubbiToken rubbiToken;

    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address owner_address = address(0x84b5545E9Bdc62A65bB2353a4cC112e3C6f3ae96);
        // console.log("account", account);

        vm.startBroadcast(privateKey);

        authentication = new Authentication();

        rubbiToken = new RubbiToken(1000000000001e8);

        modalContract = new ModalContract(address(rubbiToken));

        subscriptionService = new SubscriptionService(address(modalContract), address(owner_address));
        
        salaryStreaming = new SalaryStreaming(address(modalContract));

        vm.stopBroadcast();
    }

    function writeAddressesToFile(address addr, string memory text) public {
        string memory filename = "./deployed_contracts.txt";

        vm.writeLine(filename, "-------------------------------------------------");
        vm.writeLine(filename, text);
        vm.writeLine(filename, vm.toString(addr));
        vm.writeLine(filename, "-------------------------------------------------");
    }
}
