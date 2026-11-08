#!/bin/sh
# Route through the home router ($GATEWAY), as a computer at home does,
# then run the node.
set -eu
if [ -n "${GATEWAY:-}" ]; then
    ip route replace default via "$GATEWAY"
fi
exec oikome "$@"
