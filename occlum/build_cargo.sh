
rm -rf occlum_instance && mkdir occlum_instance && cd occlum_instance

occlum init && rm -rf image

new_json="$(jq '.resource_limits.kernel_space_stack_size = "32MB" |
                .resource_limits.kernel_space_heap_size = "256MB" |
                .resource_limits.kernel_space_heap_max_size = "6144MB" |
                .resource_limits.user_space_size = "1024MB" |
                .resource_limits.user_space_max_size = "8192MB" |
                .resource_limits.init_num_of_threads = 16 |
                .resource_limits.max_num_of_threads = 128 |
                .process.default_heap_size = "320MB" |
                .process.default_stack_size = "32MB" |
                .process.default_mmap_size = "2560MB" |
                .env.untrusted = ["EXAMPLE", "RUST_LOG"] |
                .env.default = ["HOME=/host"] |
                .metadata.debuggable = false |
                .feature.enable_edmm = true' Occlum.json)" && \
echo "${new_json}" > Occlum.json 


copy_bom -f ../rust-demo.yaml --root image --include-dir /opt/occlum/etc/template

cp ../sgx_default_qcnl.conf /etc

copy_bom -f ../rust-demo.yaml --root image --include-dir /opt/occlum/etc/template
