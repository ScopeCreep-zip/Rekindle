#!/usr/bin/env bash
# Network impairment harness for call-quality runs (plan E4.3 Q0).
#
# Shapes THIS machine's access link in both directions with tc: egress on
# the default-route interface, ingress through an ifb device. The other
# machine stays unshaped, so this link is the call's bottleneck. Each
# scenario follows a test from RFC 8867 (capacity schedules) or RFC 8868
# §4 (delay, loss, queue), and writes timestamped phase markers that
# scripts/netem/score.py aligns with the app's log.
#
# Usage (Linux, root):
#   sudo scripts/netem/rekindle-netem.sh list
#   sudo scripts/netem/rekindle-netem.sh run <scenario> [markers-file]
#   sudo scripts/netem/rekindle-netem.sh apply rate=2000 delay=50 jitter=10 loss=1
#   sudo scripts/netem/rekindle-netem.sh clear
#
# Environment:
#   REF_KBPS   RFC 8867's reference bottleneck capacity, kbps (default 3000).
#              The RFC sets 1 Mbps for its media range and specifies every
#              schedule as a ratio "to allow flexible scaling of capacity
#              values along with media source rate range" (§4.2). Voice on
#              a Veilid route costs ~750 kbps a peer on the wire
#              (evidence/e4-3-bandwidth-owner-design.md #9), so 3 Mbps keeps
#              the RFC's ratios meaningful for audio plus video.
#   IFACE      interface to shape (default: the default route's).
#
# Approximations, stated: netem's "normal" jitter is not truncated (RFC 8868
# asks for a truncated Gaussian, std 5 ms, cut at 3 std), and tbf's
# "latency" is the tail-drop queue size. The six TS 26.114 delay/error
# profiles are per-packet traces netem cannot replay; they need a trace
# player (a later Q0 item in the plan).

set -euo pipefail

REF_KBPS="${REF_KBPS:-3000}"
IFACE="${IFACE:-$(ip route get 1.1.1.1 | awk '{for (i = 1; i < NF; i++) if ($i == "dev") print $(i + 1); exit}')}"
IFB="ifb-rekindle"
MARKERS=""

die() { echo "rekindle-netem: $*" >&2; exit 1; }
[[ $(uname -s) == Linux ]] || die "Linux only (tc/netem)"
[[ $EUID -eq 0 ]] || die "run as root (sudo)"
[[ -n $IFACE ]] || die "no default-route interface; set IFACE"

now() { date -u +%Y-%m-%dT%H:%M:%S.%3NZ; }

mark() {
  local line
  line="$(now) $*"
  echo "$line"
  [[ -n $MARKERS ]] && echo "$line" >>"$MARKERS"
  return 0
}

clear_all() {
  tc qdisc del dev "$IFACE" root 2>/dev/null || true
  tc qdisc del dev "$IFACE" ingress 2>/dev/null || true
  if ip link show "$IFB" >/dev/null 2>&1; then
    tc qdisc del dev "$IFB" root 2>/dev/null || true
    ip link del "$IFB" 2>/dev/null || true
  fi
}

ensure_ifb() {
  modprobe ifb numifbs=0 2>/dev/null || true
  ip link show "$IFB" >/dev/null 2>&1 || ip link add "$IFB" type ifb
  ip link set "$IFB" up
  if ! tc qdisc show dev "$IFACE" ingress | grep -q ingress; then
    tc qdisc add dev "$IFACE" handle ffff: ingress
    tc filter add dev "$IFACE" parent ffff: matchall action mirred egress redirect dev "$IFB"
  fi
}

# shape <dev> rate=<kbps> delay=<ms> jitter=<ms> loss=<pct> ge=<p%,r%> queue=<ms>
# netem (delay, jitter, loss) at the root, tbf (rate, tail-drop queue) as
# its child.
shape_dev() {
  local dev="$1"; shift
  local rate="" delay=0 jitter=0 loss="" ge="" queue=300 kv
  for kv in "$@"; do
    case "$kv" in
      rate=*) rate="${kv#rate=}" ;;
      delay=*) delay="${kv#delay=}" ;;
      jitter=*) jitter="${kv#jitter=}" ;;
      loss=*) loss="${kv#loss=}" ;;
      ge=*) ge="${kv#ge=}" ;;
      queue=*) queue="${kv#queue=}" ;;
      *) die "unknown parameter: $kv" ;;
    esac
  done
  local netem=(delay "${delay}ms")
  if [[ $jitter != 0 ]]; then netem+=("${jitter}ms" distribution normal); fi
  if [[ -n $loss ]]; then netem+=(loss random "${loss}%"); fi
  if [[ -n $ge ]]; then netem+=(loss gemodel "${ge%,*}%" "${ge#*,}%"); fi
  # A large netem limit, so tbf below owns the queue.
  tc qdisc replace dev "$dev" root handle 1: netem limit 100000 "${netem[@]}"
  if [[ -n $rate ]]; then
    # Burst: at least one MTU, or 10 ms at rate.
    local burst=$((rate * 10 / 8))
    ((burst < 1600)) && burst=1600
    tc qdisc replace dev "$dev" parent 1:1 handle 10: tbf rate "${rate}kbit" burst "$burst" latency "${queue}ms"
  else
    tc qdisc del dev "$dev" parent 1:1 handle 10: 2>/dev/null || true
  fi
}

apply() {
  ensure_ifb
  shape_dev "$IFACE" "$@"
  shape_dev "$IFB" "$@"
  mark "apply iface=$IFACE $*"
}

ref() { echo $((REF_KBPS * $1 / 100)); }

phase() {
  local seconds="$1"; shift
  apply "$@"
  sleep "$seconds"
}

scenario() {
  case "$1" in
    baseline)
      # No impairment: the route as it is (R1/R3 baseline, Q0.6).
      mark "phase baseline begin"
      clear_all
      sleep 120 ;;
    rfc8867-5.1)
      # §5.1 variable capacity, one flow: 1.0 / 2.5 / 0.6 / 1.0 x ref at
      # 0/40/60/80 s, 100 s; one-way delay 50 ms; jitter up to 30 ms (§4.2).
      mark "phase rfc8867-5.1 cap=1.0"; phase 40 rate="$(ref 100)" delay=50 jitter=10
      mark "phase rfc8867-5.1 cap=2.5"; phase 20 rate="$(ref 250)" delay=50 jitter=10
      mark "phase rfc8867-5.1 cap=0.6"; phase 20 rate="$(ref 60)" delay=50 jitter=10
      mark "phase rfc8867-5.1 cap=1.0b"; phase 20 rate="$(ref 100)" delay=50 jitter=10 ;;
    rfc8868-delay)
      # §4.1 one-way propagation delay 0 / 50 / 150 / 300 ms, 60 s each.
      for d in 0 50 150 300; do
        mark "phase rfc8868-delay delay=$d"; phase 60 delay="$d"
      done ;;
    rfc8868-loss)
      # §4.2 independent loss 1 / 5 / 10 / 20 %, 60 s each, 50 ms delay.
      for l in 1 5 10 20; do
        mark "phase rfc8868-loss loss=$l"; phase 60 delay=50 loss="$l"
      done ;;
    rfc8868-burst)
      # §4.2 Gilbert-Elliott bursts: p=1 %, r=25 % gives ~3.8 % loss in
      # bursts of 4 packets on average; then p=3 %, r=25 % (~10.7 %).
      mark "phase rfc8868-burst ge=1,25"; phase 60 delay=50 ge=1,25
      mark "phase rfc8868-burst ge=3,25"; phase 60 delay=50 ge=3,25 ;;
    rfc8868-queue)
      # §4.3 queue 70 ms / 400 ms / 1500 ms (bufferbloat) at the reference
      # capacity, 60 s each.
      for q in 70 400 1500; do
        mark "phase rfc8868-queue queue=$q"; phase 60 rate="$(ref 100)" delay=50 queue="$q"
      done ;;
    all)
      for s in baseline rfc8867-5.1 rfc8868-delay rfc8868-loss rfc8868-burst rfc8868-queue; do
        scenario "$s"
      done ;;
    *) die "unknown scenario: $1 (try: list)" ;;
  esac
}

case "${1:-}" in
  list)
    echo "baseline rfc8867-5.1 rfc8868-delay rfc8868-loss rfc8868-burst rfc8868-queue all" ;;
  run)
    [[ -n ${2:-} ]] || die "run <scenario> [markers-file]"
    MARKERS="${3:-netem-markers-$(date -u +%Y%m%dT%H%M%SZ).txt}"
    trap 'clear_all; mark "end"' EXIT
    mark "run $2 iface=$IFACE ref_kbps=$REF_KBPS"
    scenario "$2" ;;
  apply)
    shift; apply "$@" ;;
  clear)
    clear_all; echo "cleared $IFACE" ;;
  *)
    sed -n '2,30p' "$0"; exit 1 ;;
esac
