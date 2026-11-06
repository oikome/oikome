# An Oikome node behind a home router, for the NAT check: the node image,
# with `ip` to route through the router and curl to drive the owner's API
# from inside the container (it only listens on the container's loopback).
FROM oikome:e2e

RUN apt-get update \
    && apt-get install -y --no-install-recommends iproute2 curl \
    && rm -rf /var/lib/apt/lists/*

COPY docker/nat/node.sh /usr/local/bin/nat-node
ENTRYPOINT ["nat-node"]
