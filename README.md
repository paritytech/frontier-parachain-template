# Frontier Parachain Template 

The **Frontier Parachain Template** is a ready-to-use EVM-based parachain (based on the [Frontier project](https://github.com/paritytech/frontier)), pre-configured with the [Assets](https://paritytech.github.io/substrate/master/pallet_assets/index.html) pallet, a simple Governance system ([Collective](https://paritytech.github.io/substrate/master/pallet_collective/index.html) & [Motion](https://github.com/paritytech/extended-parachain-template/tree/main/pallets/motion) pallets), and EVM precompiles.

This is an ideal starting point for any Parachain project that needs to support legacy Solidity smart contracts, but that wants at the same time to benefit from the flexibility provided by Substrate, and the shared security of the Polkadot relay chain.

This template is maintained by the **Delivery Services** team at **Parity**.

## 🚀 Getting Started

### 🦀 Rust Setup

Make sure you have Rust installed along with everything that's needed to compile a substrate node. More details [here](./docs/rust-setup.md).

### 🔧 Build

1. Clone the frontier parachain template repository:

```sh
git clone https://github.com/paritytech/frontier-parachain-template
```

2. Use `cargo` to build the parachain node without launching it:

```sh
cargo build --release
```

### 🕸️ Run a local network
 You will need a compatible release of [Polkadot](https://github.com/paritytech/polkadot-sdk) to run a local network. You may also want to use [Zombienet](https://github.com/paritytech/zombienet/releases) (available for Linux and MacOS),  for spinning up a full fledged relay chain - parachain environment. You can find more information about running a local test network [HERE](./docs/zombienet.md)



👉 Learn more about parachains [here](https://wiki.polkadot.network/docs/learn-parachains), and parathreads [here](https://wiki.polkadot.network/docs/learn-parathreads).


🧙 Learn about how to use this template and run your own parachain testnet for it in the
[Devhub Cumulus Tutorial](https://docs.substrate.io/tutorials/v3/cumulus/start-relay/).