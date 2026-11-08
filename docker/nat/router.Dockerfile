# A home router for the NAT check (tests/e2e/docker_nat.sh): Linux NAT
# (iptables MASQUERADE, nothing gets in unless asked for) and miniupnpd,
# started only when the check turns UPnP on.
FROM debian:bookworm-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        iproute2 iptables miniupnpd-iptables \
    && rm -rf /var/lib/apt/lists/* \
    # miniupnpd-iptables writes through the legacy iptables interface; the
    # router's own rules go there too, so both are in one table.
    && update-alternatives --set iptables /usr/sbin/iptables-legacy

COPY docker/nat/router.sh /usr/local/bin/nat-router
ENTRYPOINT ["nat-router"]
