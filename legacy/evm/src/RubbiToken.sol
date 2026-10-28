// SPDX-License-Identifier: MIT
pragma solidity 0.8.20;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

error FaucetCooldownNotElapsed();
error MaxClaimsReached();

contract RubbiToken is ERC20, Ownable {
    uint256 public constant FAUCET_AMOUNT = 1000 * 10 ** 18;
    uint256 public constant FAUCET_COOLDOWN = 24 hours;
    uint256 public constant MAX_FAUCET_CLAIMS = 3; // ← NEW

    mapping(address => uint256) public lastFaucetClaim;
    mapping(address => uint256) public faucetClaimCount; // ← NEW

    event FaucetClaimed(address indexed claimer, uint256 amount);

    constructor(uint256 initialSupply) ERC20("Rubbi Token", "RUB") Ownable(msg.sender) {
        _mint(msg.sender, initialSupply);
    }

    function claimFaucet() external {
        if (faucetClaimCount[msg.sender] >= MAX_FAUCET_CLAIMS) {
            revert MaxClaimsReached(); // ← ENFORCES 3-CLAIM LIMIT
        }
        uint256 lastClaim = lastFaucetClaim[msg.sender];
        if (lastClaim != 0 && block.timestamp < lastClaim + FAUCET_COOLDOWN) {
            revert FaucetCooldownNotElapsed();
        }
        lastFaucetClaim[msg.sender] = block.timestamp;
        faucetClaimCount[msg.sender] += 1; // ← INCREMENT
        _mint(msg.sender, FAUCET_AMOUNT);
        emit FaucetClaimed(msg.sender, FAUCET_AMOUNT);
    }

    function timeUntilNextClaim(address user) external view returns (uint256) {
        if (faucetClaimCount[user] >= MAX_FAUCET_CLAIMS) return type(uint256).max;
        uint256 lastClaim = lastFaucetClaim[user];
        if (lastClaim == 0) return 0;
        uint256 nextClaim = lastClaim + FAUCET_COOLDOWN;
        if (block.timestamp >= nextClaim) return 0;
        return nextClaim - block.timestamp;
    }

    function mintTo(address to, uint256 amount) external onlyOwner {
        _mint(to, amount);
    }
}