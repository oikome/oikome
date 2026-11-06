#!/usr/bin/env bash
# Copyright 2025 The Oikome Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# End-to-end check of reaching across networks without a relay (3.12), on
# docker/nat/compose.yml: two homes behind Linux NAT routers and a contact
# on the internet segment.
#
#  1. Neither router opens a port: node-a (sharing) keeps a place at the
#     contact node-c; node-b meets node-a there, the two punch through
#     their routers, and node-b reads node-a's collection directly.
#  2. node-c stops helping: node-b, started again, can't reach node-a.
#  3. router-a turns UPnP on: node-a's router opens its port, and node-b
#     reads through it, without node-c.
#
# Every step prints what was expected and what happened; the first
# mismatch fails the run. Invite codes and login links are credentials:
# they stay in shell variables and inside the containers, never printed.
#
#   E2E_BUILD=1     build the images first (default)
#   E2E_LOG_DIR=d   on failure, write the containers' logs to d/nat.log

set -euo pipefail
cd "$(dirname "$0")/../.."

COMPOSE=(docker compose -f docker/nat/compose.yml)
LOG_DIR=${E2E_LOG_DIR:-e2e-logs}

cleanup() {
    local status=$?
    if [[ $status -ne 0 ]]; then
        mkdir -p "$LOG_DIR"
        "${COMPOSE[@]}" logs --no-color >"$LOG_DIR/nat.log" 2>&1 || true
        echo "container logs: $LOG_DIR/nat.log"
    fi
    "${COMPOSE[@]}" down -v >/dev/null 2>&1 || true
    exit "$status"
}
trap cleanup EXIT

fail() {
    echo "FAIL $*"
    exit 1
}

check() {
    local what=$1 expected=$2 actual=$3
    if [[ "$actual" == "$expected" ]]; then
        printf 'ok   %s\n     expected: %s\n     actual:   %s\n' "$what" "$expected" "$actual"
    else
        fail "$(printf '%s\n     expected: %s\n     actual:   %s' "$what" "$expected" "$actual")"
    fi
}

check_contains() {
    local what=$1 expected=$2 actual=$3
    if [[ "$actual" == *"$expected"* ]]; then
        printf 'ok   %s\n     expected to contain: %s\n' "$what" "$expected"
    else
        fail "$(printf '%s\n     expected to contain: %s\n     actual: %s' "$what" "$expected" "$actual")"
    fi
}

# wait_for <description> <seconds> <command...>: retry until it succeeds.
wait_for() {
    local what=$1 seconds=$2
    shift 2
    local deadline=$((SECONDS + seconds))
    until "$@" >/dev/null 2>&1; do
        ((SECONDS < deadline)) || fail "$what (not within ${seconds}s)"
        sleep 1
    done
    echo "ok   $what"
}

logs_contain() {
    "${COMPOSE[@]}" logs --no-color "$1" 2>/dev/null | grep -q "$2"
}

# Run SQL as the owner of a node; prints rows as tab-separated values.
sql() {
    "${COMPOSE[@]}" exec -T "$1" oikome sql --raw "$2"
}

# api <node> <operation> <json>: call the owner's API inside the node's
# container, logged in with the link the node keeps there (read there,
# never printed). Prints the JSON answer.
api() {
    "${COMPOSE[@]}" exec -T "$1" sh -c '
        base=http://127.0.0.1:7340
        jar=/tmp/oikome-session
        if [ ! -s "$jar" ]; then
            curl -sf -o /dev/null -c "$jar" "$(cat /data/.oikome/web/login)"
        fi
        curl -s -b "$jar" -H "Origin: $base" -H "content-type: application/json" \
            --data-binary "$2" "$base/api/$1"
    ' sh "$2" "$3"
}

# field <name>: the first string value of a JSON field on standard input.
field() {
    sed -n "s/.*\"$1\":\"\\([^\"]*\\)\".*/\\1/p" | head -n 1
}

reachable() {
    api "$1" network '{}' | field reachable
}

is_reachable() {
    [[ "$(reachable "$1")" == "$2" ]]
}

can_read() {
    [[ "$(sql "$1" "SELECT count(*) FROM blog.posts" 2>/dev/null)" == "2" ]]
}

# share <owner> <space> <reader-did>: share a collection, allowing it to be
# reached from outside this once; prints the invite code (for a variable,
# not a screen).
share() {
    api "$1" share_space "{\"space\":\"$2\",\"did\":\"$3\",\"reach\":{\"allow\":true}}" | field invite
}

# accept <node> <invite>: add an invite and accept it.
accept() {
    local id
    id=$(api "$1" add_invite "{\"invite\":\"$2\"}" | field id)
    [[ -n "$id" ]] || fail "$1 couldn't add the invite"
    api "$1" accept_invite "{\"id\":\"$id\"}" >/dev/null
}

if [[ ${E2E_BUILD:-1} == 1 ]]; then
    docker compose -f docker/compose.yml build node-a
    "${COMPOSE[@]}" build
fi

echo "== two homes behind routers that open nothing, and a contact on the internet"
"${COMPOSE[@]}" up -d router-a router-b
wait_for "router-a routes home-a" 60 logs_contain router-a "routing"
wait_for "router-b routes home-b" 60 logs_contain router-b "routing"
"${COMPOSE[@]}" up -d node-a node-b node-c
for node in node-a node-b node-c; do
    wait_for "$node is up" 120 logs_contain "$node" "serving the web API"
done
DID_A=$("${COMPOSE[@]}" exec -T node-a oikome identity)
DID_B=$("${COMPOSE[@]}" exec -T node-b oikome identity)
DID_C=$("${COMPOSE[@]}" exec -T node-c oikome identity)
echo "node-a: $DID_A"
echo "node-b: $DID_B"
echo "node-c: $DID_C"

# Everyone knows everyone as a contact.
api node-a add_contact "{\"did\":\"$DID_B\",\"name\":\"Bob\"}" >/dev/null
api node-a add_contact "{\"did\":\"$DID_C\",\"name\":\"Cleo\"}" >/dev/null
api node-b add_contact "{\"did\":\"$DID_C\",\"name\":\"Cleo\"}" >/dev/null
api node-c add_contact "{\"did\":\"$DID_A\",\"name\":\"Ana\"}" >/dev/null
api node-c add_contact "{\"did\":\"$DID_B\",\"name\":\"Bob\"}" >/dev/null
check "node-c, on the internet, is reached directly" "direct" "$(reachable node-c)"

# node-a and node-b learn where node-c is from its invites.
sql node-c "CREATE SPACE hello" >/dev/null
sql node-c "CREATE TABLE hello.notes (id BIGINT NOT NULL)" >/dev/null
INVITE=$(share node-c hello "$DID_A")
accept node-a "$INVITE"
INVITE=$(share node-c hello "$DID_B")
accept node-b "$INVITE"

echo "== 1. node-a's router opens nothing; node-b meets node-a at node-c"
sql node-a "CREATE SPACE blog" >/dev/null
sql node-a "CREATE TABLE blog.posts (id BIGINT NOT NULL, title VARCHAR)" >/dev/null
sql node-a "INSERT INTO blog.posts VALUES (1, 'Hello'), (2, 'World')" >/dev/null
INVITE=$(share node-a blog "$DID_B")
[[ -n "$INVITE" ]] || fail "node-a couldn't share blog"
wait_for "node-a is reached through node-c" 90 is_reachable node-a through_contact
check_contains "Settings name the contact" "\"through\":\"Cleo\"" "$(api node-a network '{}')"
accept node-b "$INVITE"
wait_for "node-b reads node-a's blog across both routers" 90 can_read node-b
# Through node-c they only met: the data came over their own connection.
wait_for "they connected directly after meeting" 30 logs_contain node-b "connected directly after meeting"

echo "== 2. node-c stops helping: node-b, started again, can't reach node-a"
api node-c set_network '{"helpContacts":false}' >/dev/null
"${COMPOSE[@]}" restart node-b >/dev/null
wait_for "node-b is up again" 120 logs_contain node-b "serving the web API"
if sql node-b "SELECT count(*) FROM blog.posts" >/dev/null 2>&1; then
    fail "node-b read node-a's blog with node-c not helping"
fi
echo "ok   node-b can't reach node-a without a contact that helps"

echo "== 3. router-a turns UPnP on: node-a's port is opened, node-b reads through it"
"${COMPOSE[@]}" exec -d router-a nat-router upnp
wait_for "node-a's router opened its port" 120 is_reachable node-a direct
check_contains "router-a maps the port to node-a" "192.168.10.10:4001" \
    "$("${COMPOSE[@]}" exec -T router-a iptables -t nat -S MINIUPNPD)"
# A new invite carries the address the router opened.
INVITE=$(share node-a blog "$DID_B")
accept node-b "$INVITE"
wait_for "node-b reads node-a's blog through the opened port" 90 can_read node-b

echo "== all checks passed"
