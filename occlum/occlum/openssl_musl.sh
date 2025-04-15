#!/bin/bash
set -e

# 1. Download and install OpenSSL 1.1.1
rm -rf deps/openssl && mkdir -p deps/openssl
pushd deps/openssl
git clone https://github.com/openssl/openssl .
git checkout tags/OpenSSL_1_1_1 -b OpenSSL_1_1_1
CC=occlum-gcc ./config \
  --prefix=/usr/local/occlum/x86_64-linux-musl \
  --openssldir=/usr/local/occlum/ssl \
  --with-rand-seed=rdcpu \
  no-zlib no-async no-tests
make -j
sudo make install
popd

# 2. Export Openssl envs
export X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_INCLUDE_DIR=/usr/local/occlum/x86_64-linux-musl/include/openssl
export X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_LIB_DIR=/usr/local/occlum/x86_64-linux-musl/lib