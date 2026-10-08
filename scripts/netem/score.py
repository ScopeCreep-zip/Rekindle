#!/usr/bin/env python3
"""Score a call-quality run against the requirements (plan E4.3 Q0).

Reads a Rekindle log (the dev app's tracing output) and, optionally, the
phase markers rekindle-netem.sh wrote, and reports per phase:

- video: freezes, freeze time ratio, pauses, frame rate, post-FEC loss
  ("media quality: video", W3C freeze definitions);
- lip sync: skew p50/p95 and the share outside P.1305's +90/-185 ms
  ("media quality: lip sync");
- voice: estimated mouth-to-ear p50/p95/max against G.114's 150/400 ms,
  loss and jitter ("voice link");
- route: bandwidth estimate and dropped video ("media route").

Requirements (evidence/e4-3-call-quality-standards.md): R1 mouth-to-ear
<=400 ms (<=150 "transparent"), R3 skew within +90/-185 ms, R5 voice usable
at 3 % loss and video at 1 %, R11 freezes counted as W3C does.

Usage: score.py LOG [MARKERS]
Markers and logs are both UTC wall clock. Phases are tens of seconds long,
so the far machine's log scores against the harness machine's markers as
long as both clocks are NTP-synced.
"""

import re
import sys
from datetime import datetime, timezone

LINE = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+)Z\s+\w+\s+(\S+): (.*)$")
FIELD = re.compile(r"(\w+)=(Some\()?(-?\d+(?:\.\d+)?)\)?")


def parse_ts(text):
    head, frac = text.split(".")
    base = datetime.strptime(head, "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)
    return base.timestamp() + float("0." + frac.rstrip("Z"))


def fields(rest):
    return {m.group(1): float(m.group(3)) for m in FIELD.finditer(rest)}


def read_log(path):
    events = []
    with open(path, errors="replace") as f:
        for raw in f:
            m = LINE.match(raw.strip())
            if not m:
                continue
            ts, rest = parse_ts(m.group(1)), m.group(3)
            for kind, tag in (
                ("video", "media quality: video"),
                ("sync", "media quality: lip sync"),
                ("voice", "voice link"),
                ("route", "media route"),
            ):
                if rest.startswith(tag):
                    events.append((ts, kind, rest, fields(rest)))
                    break
    return events


def read_markers(path):
    phases = []
    with open(path) as f:
        rows = [line.split(" ", 1) for line in f if line.strip()]
    marks = [(parse_ts(ts), text.strip()) for ts, text in rows]
    starts = [(t, text[len("phase "):]) for t, text in marks if text.startswith("phase ")]
    end = marks[-1][0] if marks else None
    for i, (t, name) in enumerate(starts):
        until = starts[i + 1][0] if i + 1 < len(starts) else end
        phases.append((name, t, until))
    return phases


def pct(values, p):
    if not values:
        return None
    s = sorted(values)
    return s[min(len(s) - 1, int(round((len(s) - 1) * p / 100)))]


def stream_key(rest):
    m = re.search(r"stream=<?(\w+)>?", rest)
    return m.group(1) if m else "?"


def score(events, name, start, end):
    window = [e for e in events if start <= e[0] < end]
    out = [f"== {name}  ({end - start:.0f} s)"]

    # Video: cumulative counters per stream, differenced across the window.
    streams = {}
    for ts, kind, rest, f in window:
        if kind == "video":
            streams.setdefault(stream_key(rest), []).append(f)
    for key, rows in streams.items():
        a, b = rows[0], rows[-1]
        d = lambda k: b.get(k, 0) - a.get(k, 0)
        freeze_ratio = d("total_freezes_ms") / 1000 / max(end - start, 1)
        lost, done = d("frames_lost_post_fec"), d("frames_completed")
        out.append(
            f"  video {key[:8]}: freezes {d('freeze_count'):.0f} "
            f"({freeze_ratio:.1%} of the time), pauses {d('pause_count'):.0f}, "
            f"fps {b.get('fps', 0):.1f} (harmonic {b.get('harmonic_fps', 0):.1f}), "
            f"post-FEC loss {lost / (lost + done) if lost + done else 0:.2%} "
            f"[R5 video <=1%: {'PASS' if lost + done and lost / (lost + done) <= 0.01 else 'FAIL'}], "
            f"FEC-recovered {d('frames_recovered_by_fec'):.0f}"
        )

    skews = [f["skew_ms"] for _, k, _, f in window if k == "sync" and "skew_ms" in f]
    if skews:
        outside = sum(1 for s in skews if s > 90 or s < -185)
        out.append(
            f"  lip sync: p50 {pct(skews, 50):+.0f} ms, p5 {pct(skews, 5):+.0f}, "
            f"p95 {pct(skews, 95):+.0f}; outside +90/-185: {outside}/{len(skews)} "
            f"[R3: {'PASS' if outside == 0 else 'FAIL'}]"
        )

    m2e = [f["mouth_to_ear_ms"] for _, k, _, f in window if k == "voice" and "mouth_to_ear_ms" in f]
    loss = [f["loss_pct"] for _, k, _, f in window if k == "voice" and "loss_pct" in f]
    jitter = [f["jitter_ms"] for _, k, _, f in window if k == "voice" and "jitter_ms" in f]
    if m2e:
        p95 = pct(m2e, 95)
        verdict = "transparent" if p95 <= 150 else ("PASS" if p95 <= 400 else "FAIL")
        out.append(
            f"  voice mouth-to-ear: p50 {pct(m2e, 50):.0f} ms, p95 {p95:.0f}, max {max(m2e):.0f} "
            f"[R1 <=400: {verdict}]; loss p95 {pct(loss, 95) or 0:.1f}%, "
            f"jitter p95 {pct(jitter, 95) or 0:.0f} ms"
        )

    est = [f["estimate_kbps"] for _, k, _, f in window if k == "route" and "estimate_kbps" in f]
    # A running total per route: the window's drops are last minus first.
    drops = [f["dropped_video_frames"] for _, k, _, f in window if k == "route" and "dropped_video_frames" in f]
    if est:
        out.append(
            f"  route estimate: p10 {pct(est, 10):.0f} kbps, p50 {pct(est, 50):.0f}, "
            f"p90 {pct(est, 90):.0f}; video frames dropped at the sender {max(drops) - min(drops) if drops else 0:.0f}"
        )
    if len(out) == 1:
        out.append("  (no measurements in this window)")
    return "\n".join(out)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    events = read_log(sys.argv[1])
    if not events:
        sys.exit("no media measurements in the log")
    if len(sys.argv) > 2:
        phases = read_markers(sys.argv[2])
    else:
        phases = [("whole log", events[0][0], events[-1][0] + 1)]
    for name, start, end in phases:
        print(score(events, name, start, end))


if __name__ == "__main__":
    main()
