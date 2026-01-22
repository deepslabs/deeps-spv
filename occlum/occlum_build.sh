#!/bin/bash

occlum-cargo build --release --features=test && \
rm -rf occlum_instance && mkdir occlum_instance && cd occlum_instance && \
occlum init && rm -rf image && \
new_json="$(jq '.resource_limits.kernel_space_stack_size = "48MB" |
                .resource_limits.kernel_space_heap_size = "512MB" |
                .resource_limits.kernel_space_heap_max_size = "8192MB" |
                .resource_limits.user_space_size = "1024MB" |
                .resource_limits.user_space_max_size = "8192MB" |
                .resource_limits.init_num_of_threads = 16 |
                .resource_limits.max_num_of_threads = 256 |
                .process.default_heap_size = "5000MB" |
                .process.default_stack_size = "64MB" |
                .process.default_mmap_size = "4096MB" |
                .env.untrusted = ["EXAMPLE", "RUST_LOG"] |
                .env.default = ["HOME=/host"] |
                .metadata.debuggable = false |
                .feature.enable_edmm = true' Occlum.json)" && \
echo "${new_json}" > Occlum.json  && \
copy_bom -f ../rust-demo.yaml --root image --include-dir /opt/occlum/etc/template && \
occlum build && occlum package \
RUST_BACKTRACE=full RUST_LOG=client=debug,spv=debug occlum run /bin/sxn-spv --daemon-rpc-addr="192.168.36.15:18332" --http-addr="127.0.0.1:3025" --subclient-url="ws://192.168.200.17:9944" --device-owner="0xEE18f03D43fC684030bF8ff5424005f933675128" --watcher-device-id="0x0000000000000000000000000000000000000000000000000000000000000000" --cookie="btc:btc" --execution-rpc=http://192.168.41.21:8545 --consensus-rpc=http://192.168.41.21:3500 --sgx-enable --store=/host/db --eth-network=sepolia


RUST_LOG=client=debug,spv=debug occlum run /bin/sxn-spv --daemon-rpc-addr="192.168.36.146:28332" --doge-daemon-rpc-addr="192.168.36.15:44555" --doge-network-type=dogecoin_testnet --doge-cookie="asahi:asahi" --http-addr="0.0.0.0:3023" --subclient-url="ws://192.168.200.17:9944" --device-owner="0xEE18f03D43fC684030bF8ff5424005f933675128" --watcher-device-id="0x0000000000000000000000000000000000000000000000000000000000000000" --cookie="btc:btc" --execution-rpc=http://192.168.41.21:8545 --consensus-rpc=http://192.168.41.21:3500 --sgx-enable --store=/host/db --eth-network=sepolia