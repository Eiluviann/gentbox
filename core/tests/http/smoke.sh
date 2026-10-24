#!/usr/bin/env bash
# The PR 2b verification script: starts gentboxd on a temp state folder,
# checks the hello proof, posts one message per type and blocking level over
# TCP and the Unix socket and checks the fields that come back, long-polls,
# rescinds, and stops the daemon cleanly.
#
# Usage: core/tests/http/smoke.sh [path/to/gentboxd]
# Needs curl, jq and openssl.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
binary="${1:-$here/../../target/debug/gentboxd}"
state="$(mktemp -d)"
log="$state/gentboxd.log"
failures=0

cleanup() {
	if [[ -n "${pid:-}" ]] && kill -0 "$pid" 2>/dev/null; then
		kill -TERM "$pid" 2>/dev/null || true
		wait "$pid" 2>/dev/null || true
	fi
	rm -rf "$state"
}
trap cleanup EXIT

check() {
	local name="$1" actual="$2" expected="$3"
	if [[ "$actual" == "$expected" ]]; then
		printf 'ok   %s\n' "$name"
	else
		printf 'FAIL %s: got %s, want %s\n' "$name" "$actual" "$expected"
		failures=$((failures + 1))
	fi
}

GENTBOX_LOG=warn "$binary" --state-dir "$state" --config-dir "$state" 2>"$log" &
pid=$!
for _ in $(seq 1 200); do
	[[ -s "$state/API/port" ]] && break
	sleep 0.05
done
[[ -s "$state/API/port" ]] || { echo "gentboxd did not start"; cat "$log"; exit 1; }

port="$(cat "$state/API/port")"
token="$(cat "$state/API/token")"
socket="$state/API/hub.sock"
base="http://127.0.0.1:$port"
auth=(-H "Authorization: Bearer $token" -H "Content-Type: application/json")

# hello: the proof is HMAC-SHA256(token, "gentbox-hello-v1\0" + port + "\0" +
# nonce), base64url without padding.
nonce="$(openssl rand -base64 24 | tr '+/' '-_' | tr -d '=')"
proof="$(curl -sS -X POST "$base/v1/hello" -H 'Content-Type: application/json' -d "{\"nonce\":\"$nonce\"}" | jq -r .proof)"
expected="$(printf 'gentbox-hello-v1\0%s\0%s' "$port" "$nonce" | openssl dgst -sha256 -hmac "$token" -binary | openssl base64 -A | tr '+/' '-_' | tr -d '=')"
check "hello proof" "$proof" "$expected"

check "no token is 401" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$base/v1/messages" -H 'Content-Type: application/json' -d '{}')" "401"
check "foreign Host is 400" "$(curl -s -o /dev/null -w '%{http_code}' "${auth[@]}" -H 'Host: evil.example' "$base/v1/topics")" "400"

post() {
	curl -sS -X POST "$base/v1/messages" "${auth[@]}" -d "$1"
}

message() {
	local type="$1" blocking="$2" conversation="$3" extra="${4:-}"
	printf '{"version":1,"source":{"agent":"smoke-test"},"conversation":{"id":"%s","cwd":"/tmp/gentbox"},"type":"%s","blocking":"%s","title":"Pick the release name","awaitsResponse":true%s}' \
		"$conversation" "$type" "$blocking" "$extra"
}

steps=',"steps":[{"text":"Open the release page"}]'
rows=(
	"decision_required blocking"
	"decision_required deferrable"
	"decision_required non_blocking"
	"action_required blocking"
	"action_required deferrable"
	"action_required non_blocking"
	"task_completed non_blocking"
	"generic non_blocking"
)
index=0
for row in "${rows[@]}"; do
	read -r type blocking <<<"$row"
	extra=""
	[[ "$type" == "action_required" ]] && extra="$steps"
	reply="$(post "$(message "$type" "$blocking" "row-$index" "$extra")")"
	check "$type/$blocking type" "$(jq -r .type <<<"$reply")" "$type"
	check "$type/$blocking blocking" "$(jq -r .blocking <<<"$reply")" "$blocking"
	check "$type/$blocking ids" "$(jq -r '[.id, .topicId, .threadId] | map(length) | join(",")' <<<"$reply")" "26,26,26"
	expected_state="open"
	[[ "$type" == "generic" ]] && expected_state="acknowledged"
	check "$type/$blocking state" "$(jq -r .state <<<"$reply")" "$expected_state"
	# gentboxd has no UI: nothing can answer, so nobody should wait.
	check "$type/$blocking shouldWait" "$(jq -r .shouldWait <<<"$reply")" "false"
	index=$((index + 1))
done

reply="$(post "$(message decision_required blocking poll-me)")"
id="$(jq -r .id <<<"$reply")"
check "long-poll pending" "$(curl -sS "${auth[@]}" "$base/v1/messages/$id/response?wait=1" | jq -r .status)" "pending"
check "rescind" "$(curl -sS -X DELETE "${auth[@]}" -H 'Gentbox-Agent: smoke-test' "$base/v1/messages/$id" | jq -r .status)" "rescinded"
check "poll after rescind" "$(curl -sS "${auth[@]}" "$base/v1/messages/$id/response" | jq -r .rescindReason)" "resolved_elsewhere"

reply="$(curl -sS --unix-socket "$socket" -X POST "http://localhost/v1/messages" "${auth[@]}" -d "$(message task_completed non_blocking over-socket)")"
check "socket post" "$(jq -r .type <<<"$reply")" "task_completed"

warning="$(post '{"version":1,"source":{"agent":"smoke-test"},"conversation":{"id":"guard"},"type":"task_completed","title":"cargo test -p gentbox-ingest"}' | jq -r '.warnings[0].code')"
check "title guard warning" "$warning" "title_looks_like_command"

kill -TERM "$pid"
wait "$pid" && status=0 || status=$?
check "clean exit" "$status" "0"
check "port removed" "$([[ -e "$state/API/port" ]] && echo present || echo absent)" "absent"
check "socket removed" "$([[ -e "$socket" ]] && echo present || echo absent)" "absent"
pid=""

if ((failures > 0)); then
	echo "$failures check(s) failed"
	cat "$log"
	exit 1
fi
echo "all checks passed"
