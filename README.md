## start bitcoind

### install bitcoind
```
wget https://bitcoin.org/bin/bitcoin-core-27.0/bitcoin-27.0-x86_64-linux-gnu.tar.gz
tar zxvf bitcoin-27.0-x86_64-linux-gnu.tar.gz
sudo install -m 0755 -o root -g root -t /usr/local/bin bitcoin-27.0/bin/*
```

### start a bitcoind node:

set `~/.bitcoind/bitcoin.conf` as:
```
[regtest]
txindex=0
rest=1
server=1
rpcuser=prz
rpcpassword=prz
regtest=1
port=18446
rpcport=18447
```

### start
`bitcoind --regtest --txindex=false`

### generate blocks
```
bitcoin-cli -chain=regtest -rpcport=18447 -rpcuser=prz -rpcpassword=prz createwallet "my_new_wallet"
bitcoin-cli -chain=regtest -rpcport=18447 -rpcuser=prz -rpcpassword=prz -generate 3
```

```
bitcoin-cli -chain=regtest -rpcport=18447 -rpcuser=prz -rpcpassword=prz loadwallet  "my_new_wallet"
```


## start client

build with `cargo build` and run by:
```
RUST_LOG=spv=debug,client=debug ./target/debug/btc-spv --daemon-rpc-addr="192.168.35.216:18332" --http-addr="127.0.0.1:3022" --subclient-url="ws://192.168.200.16:9944" --device-owner="0x8fb4Be3a8ABa83a17ce4206e6C28a581D1EfE5A0" --watcher-device-id="0x37f1683e0f243a5151395597770f7b95425f42ef84ddbd15ecc099ba484e50c6" --cookie="btc:btc" --execution-rpc=https://eth-mainnet.g.alchemy.com/v2/{api-key} --consensus_rpc=http://127.0.0.1:3500
```

eth: sepolia & helios::consensus=debug log
```
RUST_LOG=spv=debug,client=debug,helios::consensus=debug ./target/debug/btc-spv --daemon-rpc-addr="192.168.35.216:18332" --http-addr="127.0.0.1:3022" --subclient-url="ws://192.168.200.16:9944" --device-owner="0x8fb4Be3a8ABa83a17ce4206e6C28a581D1EfE5A0" --watcher-device-id="0x37f1683e0f243a5151395597770f7b95425f42ef84ddbd15ecc099ba484e50c6" --cookie="btc:btc" --execution-rpc=https://eth-mainnet.g.alchemy.com/v2/{api-key}  --electrs-support --consensus-rpc=http://127.0.0.1:3500 --eth-network=sepolia
```

Use `--electrs-support` to only start the P2P service, providing blockheaders to electrs.

## merkle
getblock for txid
```
bitcoin-cli -chain=regtest -rpcport=18447 -rpcuser=prz -rpcpassword=prz getblock 0b93ed09ec301ef95f349dec8b66bea53702b36383e72449ee5cae312894e24f
```
response:
```
{
  "hash": "0b93ed09ec301ef95f349dec8b66bea53702b36383e72449ee5cae312894e24f",
  "confirmations": 1,
  "height": 315,
  "version": 805306368,
  "versionHex": "30000000",
  "merkleroot": "6378a3cab6e7fbcacc82ca5398cbb97b81a6560f8a5497107dccf72f2c818f93",
  "time": 1725202660,
  "mediantime": 1725202659,
  "nonce": 1,
  "bits": "207fffff",
  "difficulty": 4.656542373906925e-10,
  "chainwork": "0000000000000000000000000000000000000000000000000000000000000278",
  "nTx": 1,
  "previousblockhash": "36629e327ba04f36f09c75760efb01015cbf285475e19f47808f2eba2f60ad5f",
  "strippedsize": 214,
  "size": 250,
  "weight": 892,
  "tx": [
    "6378a3cab6e7fbcacc82ca5398cbb97b81a6560f8a5497107dccf72f2c818f93"
  ]
}
```

and then 

```
bitcoin-cli -chain=regtest -rpcport=18447 -rpcuser=prz -rpcpassword=prz gettxoutproof "[\"6378a3cab6e7fbcacc82ca5398cbb97b81a6560f8a5497107dccf72f2c818f93\"]" "0b93ed09ec301ef95f349dec8b66bea53702b36383e72449ee5cae312894e24f"
```

resp:
```
000000305fad602fba2e8f80479fe1755428bf5c0101fb0e76759cf0364fa07b329e6236938f812c2ff7cc7d1097548a0f56a6817bb9cb9853ca82cccafbe7b6caa37863e480d466ffff7f20010000000100000001938f812c2ff7cc7d1097548a0f56a6817bb9cb9853ca82cccafbe7b6caa378630101
```

verify:
```
 bitcoin-cli -chain=regtest -rpcport=18447 -rpcuser=prz -rpcpassword=prz verifytxoutproof "000000305fad602fba2e8f80479fe1755428bf5c0101fb0e76759cf0364fa07b329e6236938f812c2ff7cc7d1097548a0f56a6817bb9cb9853ca82cccafbe7b6caa37863e480d466ffff7f20010000000100000001938f812c2ff7cc7d1097548a0f56a6817bb9cb9853ca82cccafbe7b6caa378630101"
```

resp:
```
[
  "6378a3cab6e7fbcacc82ca5398cbb97b81a6560f8a5497107dccf72f2c818f93"
]
```




# Usage
```
curl http://127.0.0.1:3022/verify_tx/2518f2c6088e33465a2edfb7fe695db35be648d3c1290594428e8e5fd56a103d
```
If it is correct, it returns origin hash; if it is wrong, it returns an error.




```
occlum-cargo build --release &&\
rm -rf occlum_instance && mkdir occlum_instance && cd occlum_instance && \
occlum init && rm -rf image && \
new_json="$(jq '.resource_limits.kernel_space_stack_size = "30MB" |
                .resource_limits.kernel_space_heap_size = "256MB" |
                .resource_limits.kernel_space_heap_max_size = "4096MB" |
                .resource_limits.user_space_size = "1024MB" |
                .resource_limits.user_space_max_size = "4096MB" |
                .resource_limits.init_num_of_threads = 16 |
                .resource_limits.max_num_of_threads = 128 |
                .process.default_heap_size = "320MB" |
                .process.default_stack_size = "30MB" |
                .process.default_mmap_size = "2048MB" |
                .env.untrusted = ["EXAMPLE", "RUST_LOG"] |
                .env.default = ["HOME=/host"] |
                .metadata.debuggable = false |
                .feature.enable_edmm = true' Occlum.json)" && \
echo "${new_json}" > Occlum.json  && \
copy_bom -f ../rust-demo.yaml --root image --include-dir /opt/occlum/etc/template && \
occlum build && occlum package
```

 sgx run:
 ```
docker run --name=btc-spv -d --net=host --restart=unless-stopped  --device /dev/sgx/enclave --device /dev/sgx/provision -v /mnt/data/db:/root/occlum_instance/db  boolnetwork/btc-spv:1 bash -c "cd /root/occlum_instance;RUST_LOG=spv=debug occlum run /bin/btc-spv --daemon-rpc-addr="192.168.35.216:18332" --http-addr="127.0.0.1:3022" --subclient-url="ws://192.168.200.16:9944" --device-owner="0x8fb4be3a8aba83a17ce4206e6c28a581d1efe5a0" --watcher-device-id="0x8ac9a2b0d84bafa317b9286155baf1d4b728637a8235ce2d96564e84fab33ce8" --cookie="btc:btc" --sgx-enable --store=/host/db --thread=30"
 ```

 use nightly-2024-06-27

BTC:

 `curl -X POST -H 'Content-Type: application/json' --data '[{"jsonrpc":"2.0","method":"getbestblockhash","params":[],"id":1}]'  http://127.0.0.1:3022/btc`

 `curl -X POST -H 'Content-Type: application/json' --data '[{"jsonrpc":"2.0","method":"getblockheader","params":["000000000a0df79ad0764f681c2b645275afd9db40a4cfae272729e7ffeb09aa"],"id":1}]'  http://127.0.0.1:3022/btc`

`curl -X POST -H 'Content-Type: application/json' --data '[{"jsonrpc":"2.0","method":"getblockhash","params":[3200000],"id":1}]'  http://127.0.0.1:3022/btc`

ETH:

`curl -X POST -H 'Content-Type: application/json' --data '{"jsonrpc":"2.0","method":"eth_getLogs","params":[{"address": "0xdAC17F958D2ee523a2206206994597C13D831ec7", "fromBlock":"0x13dff46"}],"id":1}'  http://127.0.0.1:3022/eth2`

`curl -X POST -H 'Content-Type: application/json' --data '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["7457900"],"id":1}'  http://127.0.0.1:3023/eth2`

`curl -X POST -H 'Content-Type: application/json' --data '{"jsonrpc":"2.0","method":"eth_getTransactionReceipt","params":["0xdf5b7fc7d1ffb3db0b3e58228549687d2cb1436dc92aeab6b362f8472e1ee388"],"id":1}'  http://127.0.0.1:3023/optimism`



# start geth

`wget https://gethstore.blob.core.windows.net/builds/geth-linux-amd64-1.14.9-c350d3ac.tar.gz`
`tar -zxvf geth-linux-amd64-1.14.9-c350d3ac.tar.gz`
`geth --sepolia --authrpc.addr localhost --authrpc.port 8551 --authrpc.vhosts localhost --authrpc.jwtsecret /tmp/jwtsecret`


use docker:

`docker pull ethereum/client-go`
`
`docker run -d --name ethereum-node -v /Users/alice/ethereum:/root \
            -v /tmp/jwtsecret:/tmp/jwtsecret \
           -p 8545:8545 -p 30303:30303 -p 8551:8551 \
           ethereum/client-go --sepolia --authrpc.addr localhost \
           --authrpc.port 8551 --authrpc.vhosts localhost\
           --authrpc.jwtsecret /tmp/jwtsecret/jwtsecret`


`wget https://github.com/status-im/nimbus-eth2/releases/download/nightly/nimbus-eth2_Linux_amd64_nightly_latest.tar.gz `
` tar -zxvf nimbus-eth2_Linux_amd64_nightly_latest.tar.gz nimbus-eth2_Linux_amd64_20240927_744cc009`

`./run-sepolia-beacon-node.sh --web3-url=http://127.0.0.1:8551 --jwt-secret=/tmp/jwtsecret/jwtsecret`

`docker pull statusim/nimbus-eth2:amd64-latest`

```
mkdir data
docker run -it -d --name nimbus \
  -v ${PWD}/data:/home/user/data \
  -v /tmp/jwtsecret:/home/user/data2 \
  -p 9000:9000 -p 5052:5052 \
  statusim/nimbus-eth2:amd64-latest \
  --data-dir=data/beacon_node/mainnet_0 \
  --network=sepolia \
  --tcp-port=9000 --udp-port=9000\
  --rest  --rest-port=5052 \
   --web3-url==http://127.0.0.1:8551 \
  --jwt-secret=/home/user/data2/jwtsecret 
```