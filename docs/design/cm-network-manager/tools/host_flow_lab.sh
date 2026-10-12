#!/bin/sh
# Лаборатория пути данных хоста и выхода ядра мимо туннеля хоста (I07). Сеть хоста не меняется.
# Запуск: HOME_REAL=$HOME unshare -U --map-root-user --map-auto -n -m sh host_flow_lab.sh <каталог tools> <none|name|auto>
set -u
set -f
SRC=$1; VAR=$2
mount -t tmpfs tmpfs /mnt; W=/mnt; chmod 755 /mnt
cp $SRC/host_flow_lab_srv.py $W/srv.py; cp ${MIHOMO:-$HOME_REAL/.cache/cm-cores/mihomo/mihomo} $W/mihomo; chmod 755 $W/mihomo; MIHOMO=$W/mihomo
C="curl -q -s --noproxy * -m 4"
URL=http://198.51.100.2:8080/
mount -t tmpfs tmpfs /run; mkdir -p /run/netns
ip link set lo up
ip netns add inet
ip link add up0 type veth peer name up0p; ip link set up0p netns inet
ip addr add 192.0.2.2/24 dev up0; ip link set up0 up
ip -n inet link set lo up; ip -n inet addr add 192.0.2.1/24 dev up0p; ip -n inet link set up0p up
ip -n inet addr add 198.51.100.2/32 dev lo
ip route add default via 192.0.2.1 dev up0
ip -6 addr add 2001:db8:1::2/64 dev up0 nodad; ip -n inet -6 addr add 2001:db8:1::1/64 dev up0p nodad; ip -n inet -6 addr add 2001:db8:2::2/128 dev lo
ip -6 route add default via 2001:db8:1::1 dev up0
URL6=http://[2001:db8:2::2]:8086/
sysctl -qw net.ipv4.ip_forward=1
ip netns exec inet python3 $W/srv.py $W/dns.log 198.51.100.2 192.0.2.1 & SRV=$!
sleep 0.5
say() { echo "## $*"; }
host6() { $C -g -o /dev/null -w '%{http_code}/%{exitcode}' $URL6; }
host() { $C -o /dev/null -w '%{http_code}/%{exitcode}' $URL; }
app() { ip netns exec cm-0 $C -o /dev/null -w '%{http_code}/%{exitcode}' $URL; }
tot() { $C http://127.0.0.1:$1/connections | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["downloadTotal"])' 2>/dev/null || echo dead; }
d() { dig +short +time=2 +tries=1 "$@" 2>&1 | tr '\n' ' '; }
cfg() { # каталог tun адрес порт вариант
  dir=$W/$1; mkdir -p $dir
  extra=""; auto=false
  [ "$5" = name ] && extra='"interface-name":"up0",'
  [ "$5" = auto ] && auto=true
  cat > $dir/config.json <<J
{"mode":"direct","log-level":"debug","find-process-mode":"off","ipv6":false,$extra
 "external-controller":"127.0.0.1:$4",
 "tun":{"enable":true,"device":"$2","stack":"gvisor","auto-route":false,"auto-redirect":false,"auto-detect-interface":$auto,"mtu":1400,"inet4-address":["$3"],"dns-hijack":["any:53","tcp://any:53"]},
 "dns":{"enable":true,"ipv6":false,"enhanced-mode":"redir-host","nameserver":["198.51.100.2"],"default-nameserver":["198.51.100.2"]}}
J
  chown -R 1:1 $dir
}
run() { setpriv --reuid=1 --regid=1 --clear-groups --inh-caps=-all --bounding-set=-all -- $MIHOMO -d $W/$1 -f $W/$1/config.json > $W/$1.log 2>&1 & echo $! > $W/$1.pid; sleep 1.2; }
host_up() {
  ip tuntap add dev cmtunh mode tun user 1; ip addr add 198.18.64.1/30 dev cmtunh; ip link set cmtunh mtu 1400 up
  ip route add default dev cmtunh table 90
}

say "0 без туннеля: хост $(host); dig напрямую: $(d @198.51.100.2 direct.test A); rp_filter all=$(sysctl -n net.ipv4.conf.all.rp_filter) default=$(sysctl -n net.ipv4.conf.default.rp_filter)"
# --- туннель хоста: правила и таблицы
ip link add cmsink type dummy && ip link set cmsink up
ip route add default dev cmsink table 91
host_up
ip rule add priority 1900 oif up0 lookup main
ip rule add priority 1990 lookup main suppress_prefixlength 0
ip rule add priority 2000 lookup 90
ip rule add priority 2001 lookup 91
cfg h cmtunh 198.18.64.1/30 9090 $VAR; run h
say "1 хост через туннель хоста ($VAR): $(host) H=$(tot 9090); CapEff ядра: $(grep CapEff /proc/$(cat $W/h.pid)/status | tr -d '\t')"
say "1c IPv6 хоста до правил: $(host6)"
ip -6 route add default dev cmsink table 91
ip -6 rule add priority 1900 oif up0 lookup main
ip -6 rule add priority 1990 lookup main suppress_prefixlength 0
ip -6 rule add priority 2001 lookup 91
say "1d IPv6 хоста после правил (приёмник): $(host6); привязанный к up0: $($C -g --interface up0 -o /dev/null -w '%{http_code}/%{exitcode}' $URL6); соседний адрес сети: $(ping -6 -c1 -W1 2001:db8:1::1 >/dev/null 2>&1 && echo ok || echo нет)"
say "1b маршруты: обычный: $(ip route get 198.51.100.2 | head -1); from 192.0.2.2: $(ip route get 198.51.100.2 from 192.0.2.2 | head -1); oif up0: $(ip route get 198.51.100.2 oif up0 | head -1)"
# --- туннель приложения 0 (команды продукта)
ip netns add cm-0
ip link add cmv0h type veth peer name cmv0n; ip link set cmv0n netns cm-0
ip addr add 10.213.0.1/30 dev cmv0h; ip link set cmv0h up
ip -n cm-0 link set lo up; ip -n cm-0 addr add 10.213.0.2/30 dev cmv0n; ip -n cm-0 link set cmv0n up
ip -n cm-0 route add default via 10.213.0.1
ip tuntap add dev cmtun0 mode tun user 1; ip addr add 198.18.0.1/30 dev cmtun0; ip link set cmtun0 mtu 1400 up
ip route add blackhole default metric 200 table 100
ip route add default dev cmtun0 table 100
ip rule add iif cmv0h lookup 100 priority 1000
cfg a cmtun0 198.18.0.1/30 9091 $VAR; run a
h0=$(tot 9090)
say "2 приложение через свой туннель ($VAR): $(app) A=$(tot 9091); H до=$h0 после=$(tot 9090)"
# --- DNS
: > $W/dns.log
say "3a dig из приложения: $(ip netns exec cm-0 dig +short +time=2 +tries=1 @198.18.0.2 app.test A | tr '\n' ' ')"
say "3b dig хоста на 9.9.9.9 (маршрут в TUN, перехват): $(d @9.9.9.9 host.test A)"
say "3c dig хоста на DNS локальной сети 192.0.2.1 до правила: $(d @192.0.2.1 lan.test A); у сервера на 192.0.2.1: $(grep -c . $W/dns.log)"
nft -f - <<N
table ip cmlab {
  chain out {
    type nat hook output priority -100; policy accept;
    oifname != "cmtunh" oifname != "lo" ip daddr != 198.51.100.2 meta l4proto { tcp, udp } th dport 53 counter dnat to 198.18.64.2
  }
}
N
: > $W/dns.log
say "3d то же после DNAT в output: $(d @192.0.2.1 lan2.test A); счётчик: $(nft list table ip cmlab | grep -o 'packets [0-9]*'); H видел: $(grep -c 'hijack udp:198.18.64.2' $W/h.log)"
# --- гибель ядра хоста и пропажа TUN
kill -9 $(cat $W/h.pid); sleep 0.3
say "4 ядро хоста убито: хост $(host), приложение $(app)"
ip link del cmtunh
say "5 TUN хоста удалён: хост $(host), приложение $(app); таблица 91: $(ip route show table 91)"
# --- обрыв и возврат внешнего интерфейса
ip link set up0 down; sleep 0.3
say "6a внешний интерфейс выключен: приложение $(app)"
ip link set up0 up; ip route add default via 192.0.2.1 dev up0; sleep 1.5
say "6b интерфейс возвращён: приложение $(app); правил oif up0: $(ip rule | grep -c 'oif up0')"
# --- смена внешнего интерфейса
ip link add up1 type veth peer name up1p; ip link set up1p netns inet
ip addr add 192.0.3.2/24 dev up1; ip link set up1 up
ip -n inet addr add 192.0.3.1/24 dev up1p; ip -n inet link set up1p up
ip route replace default via 192.0.3.1 dev up1
ip rule add priority 1901 oif up1 lookup main
sleep 2
say "7a default перешёл на up1, без действий: приложение $(app); напрямую через up1: $($C --interface up1 $URL)"
p=$($C -o /dev/null -w '%{http_code}' -X PATCH -d '{"interface-name":"up1"}' http://127.0.0.1:9091/configs)
say "7b PATCH interface-name=up1 → $p: приложение $(app)"
ip route replace default via 192.0.2.1 dev up0; sleep 2
# --- строгий rp_filter (значение интерфейса — максимум из all и своего, поэтому пишем во все)
rp() { set +f; for f in /proc/sys/net/ipv4/conf/*/rp_filter; do echo $1 > $f; done; set -f; }
p=$($C -o /dev/null -w '%{http_code}' -X PATCH -d '{"interface-name":"up0"}' http://127.0.0.1:9091/configs)
ip rule del priority 1900; ip rule del priority 1901; ip rule del priority 1990; ip rule del priority 2000; ip rule del priority 2001
rp 1
say "8a строгий rp_filter, только туннель приложения: приложение $(app)"
ip rule add priority 1100 from 10.213.0.2 lookup 100
say "8b + правило from 10.213.0.2 → таблица 100: приложение $(app)"
ip rule add priority 1900 oif up0 lookup main
ip rule add priority 1990 lookup main suppress_prefixlength 0
ip rule add priority 2000 lookup 90
ip rule add priority 2001 lookup 91
host_up; cfg h cmtunh 198.18.64.1/30 9090 $VAR; run h; rp 1
say "8c строгий rp_filter, туннель хоста поднят: хост $(host), приложение $(app)"
ip rule add priority 1902 from 192.0.2.2 lookup main
say "8c2 значения: all=$(cat /proc/sys/net/ipv4/conf/all/rp_filter) up0=$(cat /proc/sys/net/ipv4/conf/up0/rp_filter) cmtun0=$(cat /proc/sys/net/ipv4/conf/cmtun0/rp_filter); отброшено rp_filter: $(nstat -az TcpExtIPReversePathFilter | tail -1); маршрут обратной проверки: $(ip route get 198.51.100.2 from 192.0.2.2 | head -1)"
say "8d + правило from 192.0.2.2 → main: хост $(host), приложение $(app)"
rp 2
ip rule del priority 1902; ip rule del priority 1100
say "8e мягкий rp_filter, без правил from: хост $(host), приложение $(app)"
ip route del default dev cmsink table 91; ip route add blackhole default table 91; kill -9 $(cat $W/h.pid); ip link del cmtunh
say "9 blackhole вместо приёмника, TUN хоста удалён: хост $(host), приложение $(app); отброшено rp_filter: $(nstat -az TcpExtIPReversePathFilter | tail -1)"
say "журнал A: $(grep -i -E 'warn|err' $W/a.log | tail -3 | sed 's/time="[^"]*" //' | tr '\n' '|')"
kill -9 $(cat $W/a.pid) $(cat $W/h.pid) 2>/dev/null; kill $SRV
