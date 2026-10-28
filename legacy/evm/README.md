## Foundry

**Foundry is a blazing fast, portable and modular toolkit for Ethereum application development written in Rust.**

Foundry consists of:

-   **Forge**: Ethereum testing framework (like Truffle, Hardhat and DappTools).
-   **Cast**: Swiss army knife for interacting with EVM smart contracts, sending transactions and getting chain data.
-   **Anvil**: Local Ethereum node, akin to Ganache, Hardhat Network.
-   **Chisel**: Fast, utilitarian, and verbose solidity REPL.

## Documentation

https://book.getfoundry.sh/

## Usage

### Build

```shell
$ forge build
```

### Test

```shell
$ forge test
```

### Format

```shell
$ forge fmt
```

### Gas Snapshots

```shell
$ forge snapshot
```

### Anvil

```shell
$ anvil
```

### Deploy

```shell
$ forge script script/Counter.s.sol:CounterScript --rpc-url <your_rpc_url> --private-key <your_private_key>
```

### Cast

```shell
$ cast <subcommand>
```

### Help

```shell
$ forge --help
$ anvil --help
$ cast --help
```


    "contractName": "Authentication",
      "contractAddress": "0xdad5e14c6015b40e451f6c4932c877b4189936bc",

      "contractName": "RubbiToken",
      "contractAddress": "0xa76255f814b9bf39f52e2a04cb5d5d7fa12d3113",

      "contractName": "ModalContract",
      "contractAddress": "0x8cbffa1c85e59878dc8cd7f7a05b53e25efbebcf",

      "contractName": "SubscriptionService",
      "contractAddress": "0xc1cb9ee3b2426c6cade12fcf9580dd80aad7aaec",

      "contractName": "SalaryStreaming",
      "contractAddress": "0x12fbe72987bff3653e84e03894eb085c760a8a0e",