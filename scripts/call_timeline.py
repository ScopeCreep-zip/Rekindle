#!/usr/bin/env python3
"""One call's timeline from a Rekindle log, against a known good.

Lines up, per 5 s window, what each layer did, so a bad stretch shows which
layer moved first:

- network: Veilid attachment state and public-internet readiness;
- routes: our own route (re)allocations, the peer's route updates,
  "could not get remote private route" send failures, dropped Veilid events;
- session: media-ready, participants added / timed out;
- transport: datagrams sent, feedback reports received, reported
  received / lost, estimate, video queue, send hand-off times;
- media: voice packets received, video frames completed / lost, freezes,
  mouth-to-ear, lip sync.

Each window is marked against the known good (evidence/e4-3-call-quality-
standards.md R1–R11 and the transport targets from RFC 8888 / libwebrtc):
feedback every 50–250 ms (>= ~15 reports per 5 s), voice ~250 packets per
5 s while the peer talks, voice loss <= 3 %, video loss <= 1 %, no freeze,
mouth-to-ear <= 400 ms, no route failure, no participant timeout.

Usage: call_timeline.py LOG [--from HH:MM:SS] [--to HH:MM:SS]
"""

import re
import sys
from collections import defaultdict

TS = re.compile(r"^\d{4}-\d{2}-\d{2}T(\d{2}):(\d{2}):(\d{2})\.\d+Z\s+(\w+)\s+(\S+): (.*)$")
NUM = lambda name, rest: (lambda m: float(m.group(1)) if m else None)(
    re.search(r"\b" + name + r"=(?:Some\()?(-?\d+(?:\.\d+)?)", rest)
)

EVENTS = [
    ("attach", r"network attachment changed state=(\S+) public_internet_ready=(\S+)"),
    ("own-route", r"own route allocated class=(\S+)"),
    ("peer-route", r"voice peer route updated"),
    ("no-remote-route", r"could not get remote private route"),
    ("update-dropped", r"Veilid update channel full — dropped event event=\"?(\w+)"),
    ("media-ready", r"media-ready transition .*ready=(\w+) reason=(\S+)"),
    ("peer-added", r"added voice peer"),
    ("peer-removed", r"removed voice peer|expired ghost voice peer"),
    ("participant-timeout", r"voice participant timed out"),
    ("participant-new", r"new voice participant"),
    ("send-failing", r"media send failing"),
    ("veilid-error", r"ERROR veilid_api"),
]


def secs(h, m, s):
    return int(h) * 3600 + int(m) * 60 + int(s)


def clock(t):
    return f"{t // 3600:02d}:{t % 3600 // 60:02d}:{t % 60:02d}"


def median(values):
    v = sorted(values)
    return v[len(v) // 2] if v else 0


def summarize(win):
    """The call against the known good, over windows with media."""
    rows = [w for w in win.values() if "route:feedback_reports" in w]
    if not rows:
        return
    fb = [w["route:feedback_reports"] for w in rows]
    rx = [w.get("rx:voice_pkts", 0) for w in rows]
    est = [w.get("route:estimate_kbps", 0) for w in rows]
    vq = [w.get("route:video_queue_ms", 0) for w in rows]
    ev = defaultdict(float)
    for w in win.values():
        for k, v in w.items():
            if k.startswith("ev:"):
                ev[k[3:]] += v
    good_fb = sum(1 for x in fb if x >= 15) / len(fb)
    print(
        f"\n# summary: {len(rows)} windows | feedback>=15/5s in {good_fb:.0%} (median {median(fb):.0f}, good ~20)"
        f" | voice rx median {median(rx):.0f}/5s (good ~250 while talking)"
        f" | estimate median {median(est):.0f} kbps | video queue median {median(vq):.0f} ms"
        f" | route failures {ev['no-remote-route']:.0f}, participant timeouts {ev['participant-timeout']:.0f},"
        f" dropped Veilid events {ev['update-dropped']:.0f}, own-route allocations {ev['own-route']:.0f},"
        f" peer route updates {ev['peer-route']:.0f}"
    )


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        sys.exit(1)
    path = args[0]
    lo = hi = None
    if "--from" in args:
        lo = secs(*args[args.index("--from") + 1].split(":"))
    if "--to" in args:
        hi = secs(*args[args.index("--to") + 1].split(":"))

    win = defaultdict(lambda: defaultdict(float))
    events = []
    with open(path, errors="replace") as f:
        for raw in f:
            m = TS.match(raw.rstrip("\n"))
            if not m:
                continue
            t = secs(m.group(1), m.group(2), m.group(3))
            if (lo is not None and t < lo) or (hi is not None and t > hi):
                continue
            level, rest = m.group(4), m.group(6)
            w = win[t - t % 5]
            for name, pat in EVENTS:
                if name == "veilid-error" and level == "ERROR" and m.group(5) == "veilid_api":
                    events.append((t, name, rest[:80]))
                    break
                e = re.search(pat, rest)
                if e and name != "veilid-error":
                    w["ev:" + name] += 1
                    if name not in ("no-remote-route", "send-failing", "participant-timeout"):
                        events.append((t, name, " ".join(g for g in e.groups() if g)))
                    break
            if rest.startswith("media route"):
                for k in ("estimate_kbps", "video_queue_ms", "feedback_reports",
                          "reported_received", "reported_lost", "datagrams", "backoff_cuts"):
                    v = NUM(k, rest)
                    if v is not None:
                        w["route:" + k] = v
                v = NUM("voice_send_max_ms", rest)
                w["route:voice_send_max"] = v if v is not None else NUM("send_max_ms", rest) or 0
                w["route:video_send_max"] = NUM(" send_max_ms", " " + rest) or 0
            elif rest.startswith("transport feedback window"):
                w["fb:built"] += NUM("built", rest) or 0
                w["fb:accepted"] += NUM("accepted_from_peer", rest) or 0
            elif "voice receive loop stats" in rest:
                w["rx:voice_pkts"] += NUM("self.packets_received", rest) or NUM("packets_received", rest) or 0
            elif rest.startswith("media quality: video"):
                for k in ("frames_completed", "frames_lost_post_fec", "freeze_count", "total_pauses_ms"):
                    w["video:" + k] = NUM(k, rest) or 0
            elif rest.startswith("voice link peer"):
                v = NUM("mouth_to_ear_ms", rest)
                if v is not None:
                    w["voice:m2e"] = v
                v = NUM("loss_pct", rest)
                if v is not None:
                    w["voice:loss"] = v

    print(f"# {path}")
    print("time     | est  vq   fb_rx fb_rcv/lost sent | voice_rx m2e loss | video +done +lost frz | events / verdict")
    prev = {}
    for t in sorted(win):
        w = win[t]
        if not any(k.startswith(("route:", "rx:", "video:", "ev:")) for k in w):
            continue
        d = lambda k: w[k] - prev.get(k, w[k]) if k in w else 0
        sent = d("route:datagrams")
        vdone, vlost, frz = d("video:frames_completed"), d("video:frames_lost_post_fec"), d("video:freeze_count")
        bad = []
        if "route:feedback_reports" in w and w["route:feedback_reports"] < 15:
            bad.append("FEEDBACK")
        if w.get("voice:loss", 0) > 3:
            bad.append("VOICE-LOSS")
        if vdone + vlost > 0 and vlost / (vdone + vlost) > 0.01:
            bad.append("VIDEO-LOSS")
        if frz > 0:
            bad.append("FREEZE")
        if w.get("voice:m2e", 0) > 400:
            bad.append("M2E")
        evs = [f"{k[3:]}x{int(v)}" for k, v in w.items() if k.startswith("ev:")]
        if any(e.startswith(("no-remote-route", "participant-timeout", "send-failing", "update-dropped")) for e in evs):
            bad.append("ROUTE/SESSION")
        print(
            f"{clock(t)} | {w.get('route:estimate_kbps', 0):4.0f} {w.get('route:video_queue_ms', 0):4.0f}"
            f" {w.get('route:feedback_reports', 0):5.0f} {w.get('route:reported_received', 0):4.0f}/{w.get('route:reported_lost', 0):<4.0f}"
            f" {sent:4.0f} | {w.get('rx:voice_pkts', 0):5.0f} {w.get('voice:m2e', 0):4.0f} {w.get('voice:loss', 0):3.0f}"
            f" | {vdone:4.0f} {vlost:4.0f} {frz:3.0f} | {' '.join(evs)} {'OK' if not bad else '!' + ','.join(bad)}"
        )
        for k, v in w.items():
            if k.startswith(("route:datagrams", "video:")):
                prev[k] = v
    summarize(win)
    print("\n# events")
    for t, name, detail in events:
        print(f"{clock(t)} {name} {detail}")


if __name__ == "__main__":
    main()
