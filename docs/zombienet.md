## Zombienet Installation:

You can download executables of the Zombienet CLI from [paritytech/zombienet/releases](https://github.com/paritytech/zombienet/releases)


- Download the latest Zombienet CLI according to your operating system.

 💡 Tip: If you want the executable to be available system-wide then make sure you place it in one of your `$PATH` directories.
```sh
wget https://github.com/paritytech/zombienet/releases/download/v1.3.71/zombienet-macos
chmod +x zombienet-macos 
cp zombienet-macos /usr/local/bin
```
Then invoke it anywhere like :
```sh 
# Mac Os
zombienet-macos --help
```

```sh 
# Linux
zombienet-linux-x64 --help
```

You should see some similar output to indicate it's installed and working properly:
```sh
Usage: zombienet [options] [command]

Options:
  -c, --spawn-concurrency <concurrency>  Number of concurrent spawning process to launch, default is 1
  -p, --provider <provider>              Override provider to use (choices: "podman", "kubernetes", "native")
  -m, --monitor                          Start as monitor, do not auto cleanup network
  -h, --help                             display help for command

Commands:
  spawn <networkConfig> [creds]          Spawn the network defined in the config
  test <testFile> [runningNetworkSpec]   Run tests on the network defined
  setup <binaries...>                    Setup is meant for downloading and making dev environment of Zombienet ready
  version                                Prints zombienet version
  help [command]                         display help for command

```

## Setting up Zombienet config:

You may use the provided reference implementation from the repository or make your own. We provide a simple configuration for you [zombienet-config.toml](../zombienet-config.toml) which spins up two validators for the relay chain, and one collator for your parachain to get you quickly upto speed.

⚠️ Note: The path of the polkadot executable used there is `./bin/polkadot` which means you need to have a folder called `bin` inside the repository directory which contains your compiled polkadot binary named as `polkadot`. In addition to that since version 1.1.0, you will also need to place the `polkadot-execute-worker` and the `polkadot-prepare-worker` binaries to be inside `bin`. As a user you don't need to interact with these but these are required to start up the main relay chain `polkadot` software. You can find all three [here](https://github.com/paritytech/polkadot-sdk/releases)














More instructions here: [Simulate parachains in a test network
](https://docs.substrate.io/test/simulate-parachains/)