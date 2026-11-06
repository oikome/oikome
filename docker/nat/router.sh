#!/bin/sh
# A home router: forwards its home network ($HOME_NET, e.g. 192.168.10.)
# to the internet segment ($WAN_NET), rewriting the source address
# (MASQUERADE), and lets nothing in that wasn't asked for, except what
# miniupnpd maps once `nat-router upnp` starts it.
set -eu

iface_of() {
    ip -o -4 addr show | awk -v p="$1" 'index($4, p) == 1 { print $2; exit }'
}

if [ "${1:-}" = "upnp" ]; then
    lan=$(iface_of "$HOME_NET")
    wan=$(iface_of "$WAN_NET")
    lan_ip=$(ip -o -4 addr show dev "$lan" | awk '{ split($4, a, "/"); print a[1]; exit }')
    cat >/etc/miniupnpd/miniupnpd.conf <<CONF
ext_ifname=$wan
listening_ip=$lan
enable_upnp=yes
enable_natpmp=yes
secure_mode=yes
system_uuid=3d3cec3a-8cf0-11e0-98ee-001a6bd2d07b
allow 1024-65535 ${HOME_NET}0/24 1024-65535
deny 0-65535 0.0.0.0/0 0-65535
CONF
    echo "miniupnpd on $lan ($lan_ip), out through $wan"
    exec miniupnpd -d -f /etc/miniupnpd/miniupnpd.conf
fi

lan=$(iface_of "$HOME_NET")
wan=$(iface_of "$WAN_NET")
[ -n "$lan" ] && [ -n "$wan" ] || { echo "no home or internet interface"; exit 1; }

# Out: rewritten to the router's address. In: only replies, and what
# miniupnpd maps (its chains, filled when it runs).
iptables -t nat -N MINIUPNPD
iptables -t nat -N MINIUPNPD-POSTROUTING
iptables -t nat -A PREROUTING -i "$wan" -j MINIUPNPD
iptables -t nat -A POSTROUTING -o "$wan" -j MINIUPNPD-POSTROUTING
iptables -t nat -A POSTROUTING -o "$wan" -j MASQUERADE
iptables -N MINIUPNPD
iptables -P FORWARD DROP
iptables -A FORWARD -i "$lan" -o "$wan" -j ACCEPT
iptables -A FORWARD -i "$wan" -o "$lan" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A FORWARD -i "$wan" -o "$lan" -j MINIUPNPD
echo "routing $HOME_NET* out through $wan"
exec sleep infinity
