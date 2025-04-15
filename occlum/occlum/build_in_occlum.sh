#!/bin/bash
set -e

# compile key_server
pushd ..
occlum-cargo build --release
popd