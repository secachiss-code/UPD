#!/bin/sh
# Лаборатория: входящий TUN закреплённого Xray для сети приложения (I05.T02.a). Сеть хоста не меняется.
# Запуск: HOME_REAL=$HOME unshare -U --map-root-user --map-auto -n -m sh xray_tun_lab.sh <каталог tools>
set -u; set -f
SRC=$1
mount -t tmpfs tmpfs /mnt; W=/mnt; chmod 755 /mnt
cp $SRC/host_flow_lab_srv.py $W/srv.py; cp ${XRAY:-$HOME_REAL/.cache/cm-cores/xray/xray} $W/xray; chmod 755 $W/xray
C="curl -q -s --noproxy * -m 4"; URL=http://198.51.100.2:8080/
mount -t tmpfs tmpfs /run; mkdir -p /run/netns
ip link set lo up; ip netns add inet
ip link add up0 type veth peer name up0p; ip link set up0p netns inet
ip addr add 192.0.2.2/24 dev up0; ip link set up0 up
ip -n inet link set lo up; ip -n inet addr add 192.0.2.1/24 dev up0p; ip -n inet link set up0p up
ip -n inet addr add 198.51.100.2/32 dev lo
ip route add default via 192.0.2.1 dev up0
sysctl -qw net.ipv4.ip_forward=1
ip netns exec inet python3 $W/srv.py $W/dns.log 198.51.100.2 & SRV=$!
sleep 0.5
ip netns add cm-0
ip link add cmv0h type veth peer name cmv0n; ip link set cmv0n netns cm-0
ip addr add 10.213.0.1/30 dev cmv0h; ip link set cmv0h up
ip -n cm-0 link set lo up; ip -n cm-0 addr add 10.213.0.2/30 dev cmv0n; ip -n cm-0 link set cmv0n up
ip -n cm-0 route add default via 10.213.0.1
ip tuntap add dev cmtun0 mode tun user 1; ip addr add 198.18.0.1/30 dev cmtun0; ip link set cmtun0 mtu 1400 up
ip route add blackhole default metric 200 table 100
ip route add default dev cmtun0 table 100
ip rule add iif cmv0h lookup 100 priority 1000
mkdir $W/x
cat > $W/x/config.json <<J
{"log":{"loglevel":"${LEVEL:-warning}"},
 "inbounds":[{"tag":"tun","protocol":"tun","settings":{"name":"cmtun0","MTU":1400},"sniffing":{"enabled":false}}],
 "outbounds":[{"tag":"direct","protocol":"freedom","streamSettings":{"sockopt":{"interface":"up0"}}}]}
J
chown -R 1:1 $W/x
setpriv --reuid=1 --regid=1 --clear-groups --inh-caps=-all --bounding-set=-all -- $W/xray run -c $W/x/config.json > $W/x.log 2>&1 & XP=$!
sleep 1.5
app() { ip netns exec cm-0 $C -o /dev/null -w '%{http_code}/%{exitcode}' $URL; }
echo "## 1 Xray жив: $(kill -0 $XP 2>/dev/null && echo да || echo нет); CapEff: $(grep CapEff /proc/$XP/status 2>/dev/null | tr -d '\t')"
echo "## 2 TCP из приложения: $(app)"
echo "## 3 UDP из приложения (DNS на 198.51.100.2 напрямую по UDP): $(ip netns exec cm-0 dig +short +time=2 +tries=1 @198.51.100.2 udp.test A | tr '\n' ' ')"
echo "## 4 TCP DNS: $(ip netns exec cm-0 dig +tcp +short +time=2 +tries=1 @198.51.100.2 tcp.test A 2>&1 | tr '\n' ' ')"
kill -9 $XP; sleep 0.3
echo "## 5 Xray убит: $(app); устройство: $(ip -br link show cmtun0 | awk '{print $1, $2}')"
echo "## журнал: $(tail -5 $W/x.log | tr '\n' '|')"
kill $SRV
