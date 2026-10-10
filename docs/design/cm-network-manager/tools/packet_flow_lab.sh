#!/bin/sh
# Лаборатория пути данных: приложение в своём netns → veth → правило iif → таблица → TUN worker-а.
# Запуск: HOME_REAL=$HOME unshare -rn sh packet_flow_lab.sh <пустой каталог>
# curl вызывается с -q: ~/.curlrc пользователя может задавать прокси.
set -u
M=$HOME_REAL/.cache/cm-cores/mihomo/mihomo
D=$1
ip link set lo up
# «интернет»: отдельный netns с сервером
unshare -n sleep 600 & INET=$!
unshare -n sleep 600 & APP=$!
sleep 0.3
ip link add up0 type veth peer name up1; ip link set up1 netns $INET
ip addr add 198.51.100.1/24 dev up0; ip link set up0 up
nsenter -t $INET -n sh -c 'ip link set lo up; ip addr add 198.51.100.2/24 dev up1; ip link set up1 up; ip route add default via 198.51.100.1'
nsenter -t $INET -n python3 -m http.server 8080 --bind 198.51.100.2 >/dev/null 2>&1 & 
# приложение: netns с одним veth
ip link add cmv0h type veth peer name cmv0n; ip link set cmv0n netns $APP
ip addr add 10.213.0.1/30 dev cmv0h; ip link set cmv0h up
nsenter -t $APP -n sh -c 'ip link set lo up; ip addr add 10.213.0.2/30 dev cmv0n; ip link set cmv0n up; ip route add default via 10.213.0.1'
# TUN worker-а создаёт CM
ip tuntap add dev cmtun0 mode tun user 0
ip addr add 198.18.0.1/30 dev cmtun0; ip link set cmtun0 mtu 1400 up
sysctl -qw net.ipv4.ip_forward=1; sysctl -qw net.ipv4.conf.all.rp_filter=0 2>/dev/null
ip rule add iif cmv0h lookup 100 priority 100
ip route add default dev cmtun0 table 100
ip route add blackhole default metric 200 table 100
nft -f - <<N
table inet cm {
  chain forward {
    type filter hook forward priority filter; policy accept;
    iifname "cmv0h" oifname "cmtun0" counter accept
    iifname "cmtun0" oifname "cmv0h" counter accept
    iifname "cmv*" counter drop
    oifname "cmv*" counter drop
  }
}
N
mkdir -p $D/w; cat > $D/w/config.json <<C
{"mode":"direct","log-level":"warning","find-process-mode":"off","ipv6":false,"external-controller-unix":"$D/w/api.sock",
 "tun":{"enable":true,"device":"cmtun0","stack":"gvisor","auto-route":false,"auto-redirect":false,"auto-detect-interface":false,"mtu":1400,"inet4-address":["198.18.0.1/30"]}}
C
$M -d $D/w -f $D/w/config.json > $D/w/log 2>&1 & W=$!
for i in $(seq 1 50); do [ -S $D/w/api.sock ] && break; sleep 0.1; done; sleep 1
echo "A. app → internet через tun:"; nsenter -t $APP -n curl -q -s --noproxy '*' -m 5 -o /dev/null -w "%{http_code}\n" http://198.51.100.2:8080/ ; 
echo "   соединения ядра:"; curl -q -s --unix-socket $D/w/api.sock http://x/connections | python3 -c "import sys,json;d=json.load(sys.stdin);print('downloadTotal',d.get('downloadTotal'),'uploadTotal',d.get('uploadTotal'))"
echo "B. worker убит (kill -9):"; kill -9 $W; sleep 0.5
nsenter -t $APP -n curl -q -s --noproxy '*' -m 3 -o /dev/null -w "%{http_code}\n" http://198.51.100.2:8080/; echo "   curl rc=$?"
echo "C. TUN удалён совсем:"; ip link del cmtun0; ip route show table 100
nsenter -t $APP -n curl -q -s --noproxy '*' -m 3 -o /dev/null -w "%{http_code}\n" http://198.51.100.2:8080/; echo "   curl rc=$?"
echo "D. счётчики:"; nft list chain inet cm forward | grep -E "counter" 
echo "E. контроль — без правил CM прямой путь существует (нужен NAT наружу): "; ip rule del iif cmv0h lookup 100; nft add table ip nat; nft add chain ip nat post "{ type nat hook postrouting priority srcnat; }"; nft add rule ip nat post oifname up0 masquerade; nft delete table inet cm
nsenter -t $APP -n curl -q -s --noproxy '*' -m 3 -o /dev/null -w "%{http_code}\n" http://198.51.100.2:8080/
kill $INET $APP 2>/dev/null

