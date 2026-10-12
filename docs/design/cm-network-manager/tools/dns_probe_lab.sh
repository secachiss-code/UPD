#!/bin/sh
# Лаборатория DNS туннеля приложения (I10.T02.a): типы запросов, NXDOMAIN, усечение, fake-ip, UDP не на 53.
# Запуск: HOME_REAL=$HOME unshare -U --map-root-user --map-auto -n -m sh dns_probe_lab.sh <каталог tools> <redir-host|fake-ip>
set -u; set -f
SRC=$1; MODE=$2
mount -t tmpfs tmpfs /mnt; W=/mnt; chmod 755 /mnt
cp $SRC/dns_probe_lab_srv.py $W/dsrv.py; cp ${MIHOMO:-$HOME_REAL/.cache/cm-cores/mihomo/mihomo} $W/mihomo; chmod 755 $W/mihomo
mount -t tmpfs tmpfs /run; mkdir -p /run/netns
ip link set lo up; ip netns add inet
ip link add up0 type veth peer name up0p; ip link set up0p netns inet
ip addr add 192.0.2.2/24 dev up0; ip link set up0 up
ip -n inet link set lo up; ip -n inet addr add 192.0.2.1/24 dev up0p; ip -n inet link set up0p up
ip -n inet addr add 198.51.100.2/32 dev lo
ip route add default via 192.0.2.1 dev up0
sysctl -qw net.ipv4.ip_forward=1
ip netns exec inet python3 $W/dsrv.py 198.51.100.2 & SRV=$!
sleep 0.5
ip netns add cm-0
ip link add cmv0h type veth peer name cmv0n; ip link set cmv0n netns cm-0
ip addr add 10.213.0.1/30 dev cmv0h; ip link set cmv0h up
ip -n cm-0 link set lo up; ip -n cm-0 addr add 10.213.0.2/30 dev cmv0n; ip -n cm-0 link set cmv0n up
ip -n cm-0 route add default via 10.213.0.1
ip netns exec cm-0 sysctl -qw net.ipv6.conf.all.disable_ipv6=1
ip tuntap add dev cmtun0 mode tun user 1; ip addr add 198.18.0.1/30 dev cmtun0; ip link set cmtun0 mtu 1400 up
ip route add blackhole default metric 200 table 100
ip route add default dev cmtun0 table 100
ip rule add iif cmv0h lookup 100 priority 1000
mkdir $W/a
extra=""; [ "$MODE" = fake-ip ] && extra='"fake-ip-range":"198.19.0.0/24",'
cat > $W/a/config.json <<J
{"mode":"direct","log-level":"warning","find-process-mode":"off","ipv6":false,"external-controller":"127.0.0.1:9091",
 "profile":{"store-fake-ip":false},
 "tun":{"enable":true,"device":"cmtun0","stack":"gvisor","auto-route":false,"auto-redirect":false,"auto-detect-interface":true,"mtu":1400,"inet4-address":["198.18.0.1/30"],"dns-hijack":["any:53","tcp://any:53"]},
 "dns":{"enable":true,"ipv6":false,"enhanced-mode":"$MODE",$extra"nameserver":["198.51.100.2"],"default-nameserver":["198.51.100.2"]}}
J
chown -R 1:1 $W/a
setpriv --reuid=1 --regid=1 --clear-groups --inh-caps=-all --bounding-set=-all -- $W/mihomo -d $W/a -f $W/a/config.json > $W/a.log 2>&1 & AP=$!
sleep 1.3
q() { ip netns exec cm-0 dig +time=2 +tries=1 @198.18.0.2 "$@" 2>&1 | awk '/status:/{s=$6} /flags:/{f=$0; sub(/.*flags: /,"",f); sub(/;.*/,"",f)} /^[^;].*IN/{a=a" ["$4" "$5" "$6" "$7"]"} /timed out|refused|reset/{e="ОШИБКА"} END{print s" flags="f" "a" "e}'; }
echo "## режим $MODE"
for t in A AAAA TXT MX SRV HTTPS SVCB; do echo "## $t: $(q probe.test $t)"; done
echo "## NXDOMAIN: $(q nx.probe.test A)"
echo "## усечение по UDP (TXT 200 байт, сервер ставит TC): $(q big.probe.test TXT | cut -c1-70)"
echo "## TCP к перехватчику: $(q +tcp probe.test A)"
echo "## DNS на чужой адрес из приложения (9.9.9.9): $(ip netns exec cm-0 dig +short +time=2 +tries=1 @9.9.9.9 probe.test A | tr '\n' ' ')"
echo "## UDP не на 53 (эхо 198.51.100.2:9999): $(ip netns exec cm-0 python3 -c 'import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.settimeout(3); s.sendto(b"ping",("198.51.100.2",9999)); print(s.recvfrom(64)[0].decode())' 2>&1 | tail -1)"
echo "## IPv6 из приложения: $(ip netns exec cm-0 curl -q -s -g --noproxy * -m 3 -o /dev/null -w '%{http_code}/%{exitcode}' http://[2001:db8:2::2]:8086/)"
C="curl -q -s --noproxy * -m 3"
echo "## сброс fake-ip: POST /cache/fakeip/flush → $($C -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:9091/cache/fakeip/flush); сброс DNS: POST /cache/dns/flush → $($C -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:9091/cache/dns/flush)"
if [ "$MODE" = fake-ip ]; then
  a1=$(ip netns exec cm-0 dig +short +time=2 +tries=1 @198.18.0.2 one.test A | tr '\n' ' ')
  $C -o /dev/null -X POST http://127.0.0.1:9091/cache/fakeip/flush
  a2=$(ip netns exec cm-0 dig +short +time=2 +tries=1 @198.18.0.2 two.test A | tr '\n' ' ')
  echo "## fake-ip: one.test → $a1; после сброса two.test → $a2"
fi
kill -9 $AP; kill $SRV
