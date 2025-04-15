echo -e "nameserver 8.8.8.8\nnameserver 8.8.4.4" > ./occlum_instance/image/etc/resolv.conf
echo -e "hosts:	files dns" > ./occlum_instance/image/etc/nsswitch.conf
cp /lib/x86_64-linux-gnu/{libnss_dns.so.2,libnss_files.so.2,libresolv.so.2} ./occlum_instance/image/opt/occlum/glibc/lib