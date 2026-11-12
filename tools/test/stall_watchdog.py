#!/usr/bin/env python3
"""Early detection of stalled test processes, with thread dumps.

A timeout only reacts at its deadline; this watchdog looks at every running
test process every `--interval` seconds and classifies it from its own
counters (`/proc/<pid>/stat`, `/proc/<pid>/task/*/stat`):

* BLOCKED  -- CPU time of the test and its descendants (e.g. the JVM of a Java
  oracle) grew by less than `--idle-cpu` seconds over the last
  `--blocked-after` seconds and no thread of that process tree is running: a deadlock or a lost
  wake-up, which never progresses. The watchdog takes a full thread dump with
  gdb (`thread apply all bt`) and, with `--kill-blocked`, kills the test so the
  run goes on (the runner reports it as a failure).
* SPINNING -- one thread has been on CPU continuously for `--spin-after`
  seconds. Two gdb dumps `--spin-gap` seconds apart are taken; identical
  innermost Rucene frames mark it as a probable livelock. The test is left to
  the runner's timeout: a slow test and a loop look alike from outside.
* GROWING  -- resident memory above `--rss-alert` MiB and higher at every one
  of the last `--rss-samples` samples: a probable leak. One dump is taken.

Attaching needs the test to be traceable: `ptracer_exec.py` (wired into
`~/.local/bin/rucene-mem-runner`) declares it with `PR_SET_PTRACER_ANY`.
Alerts go to `--log` (one line `ALERT <kind> pid=... name=...` followed by the
dump), so a waiter on that file notifies at the first alert.

Usage: stall_watchdog.py --log FILE [--match REGEX] [--kill-blocked]
"""
import argparse
import os
import re
import signal
import subprocess
import time
from collections import defaultdict, deque

CLK = os.sysconf("SC_CLK_TCK")


def stat_fields(path):
    with open(path) as f:
        s = f.read()
    # comm may contain spaces/parentheses: split after the last ')'
    rest = s[s.rindex(")") + 2:].split()
    return rest  # rest[0] is state (field 3)


def proc_cpu(pid):
    r = stat_fields(f"/proc/{pid}/stat")
    return (int(r[11]) + int(r[12])) / CLK  # utime + stime


def proc_rss_mib(pid):
    with open(f"/proc/{pid}/statm") as f:
        return int(f.read().split()[1]) * os.sysconf("SC_PAGE_SIZE") / 2**20


def thread_states(pid):
    out = {}
    for tid in os.listdir(f"/proc/{pid}/task"):
        try:
            r = stat_fields(f"/proc/{pid}/task/{tid}/stat")
            out[int(tid)] = (r[0], (int(r[11]) + int(r[12])) / CLK)
        except (FileNotFoundError, ProcessLookupError, ValueError):
            pass
    return out


def descendants(pid):
    """All descendant pids (children of every thread, recursively)."""
    out, stack = [], [pid]
    while stack:
        p = stack.pop()
        try:
            for tid in os.listdir(f"/proc/{p}/task"):
                try:
                    with open(f"/proc/{p}/task/{tid}/children") as f:
                        kids = [int(k) for k in f.read().split()]
                except OSError:
                    kids = []
                out.extend(kids)
                stack.extend(kids)
        except OSError:
            pass
    return out


def tree_cpu_and_running(pid):
    """CPU seconds and whether any thread runs, over the process and its
    descendants: a test waiting on the JVM of a Java oracle is not blocked
    while that JVM works (false positive observed 2026-11-12)."""
    cpu, running = 0.0, False
    for p in [pid] + descendants(pid):
        try:
            cpu += proc_cpu(p)
            running = running or any(s == "R" for s, _ in thread_states(p).values())
        except (FileNotFoundError, ProcessLookupError, ValueError):
            pass
    return cpu, running


def name_of(pid):
    try:
        return os.path.basename(os.readlink(f"/proc/{pid}/exe"))
    except OSError:
        return "?"


def gdb_dump(pid, depth=40):
    try:
        r = subprocess.run(
            ["gdb", "-p", str(pid), "-batch", "-ex", "set pagination off",
             "-ex", f"thread apply all bt {depth}"],
            capture_output=True, text=True, timeout=180)
        lines = [l for l in r.stdout.splitlines() if l.startswith(("Thread", "#"))]
        return "\n".join(lines) if lines else (r.stderr.strip()[-2000:] or "no frames")
    except subprocess.TimeoutExpired:
        return "gdb timed out"


RUCENE_FRAME = re.compile(r" in ((?:rucene|rucene_test_framework|rucene_codecs|randomizedtesting)::\S+)")


def inner_frames(dump, n=6):
    """Innermost project frames of the busiest-looking threads, for comparison."""
    return tuple(RUCENE_FRAME.findall(dump)[:n])


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--log", required=True)
    ap.add_argument("--match", default=r"/\.rucene-dev-target/debug/deps/")
    ap.add_argument("--interval", type=float, default=15)
    ap.add_argument("--blocked-after", type=float, default=90)
    ap.add_argument("--idle-cpu", type=float, default=0.5)
    ap.add_argument("--spin-after", type=float, default=300)
    ap.add_argument("--spin-gap", type=float, default=30)
    ap.add_argument("--rss-alert", type=float, default=2048)
    ap.add_argument("--rss-samples", type=int, default=8)
    ap.add_argument("--kill-blocked", action="store_true")
    ap.add_argument("--exit-when-idle", type=float, default=0,
                    help="exit after this many seconds with no matching process (0: never)")
    a = ap.parse_args()
    pat = re.compile(a.match)
    hist = defaultdict(lambda: deque(maxlen=64))   # pid -> (t, cpu)
    rss = defaultdict(lambda: deque(maxlen=a.rss_samples))
    spin_since = {}                                 # (pid, tid) -> t when continuously running
    alerted = defaultdict(set)                      # pid -> kinds already reported
    idle_since = time.time()

    def alert(kind, pid, detail, dump):
        with open(a.log, "a") as f:
            f.write(f"ALERT {kind} pid={pid} name={name_of(pid)} t={time.strftime('%H:%M:%S')} {detail}\n")
            f.write(dump.rstrip() + "\n----\n")

    while True:
        now = time.time()
        pids = []
        for d in os.listdir("/proc"):
            if not d.isdigit():
                continue
            try:
                cmd = open(f"/proc/{d}/cmdline", "rb").read().replace(b"\0", b" ").decode(errors="replace")
            except OSError:
                continue
            if pat.search(cmd) and "stall_watchdog" not in cmd and "ptracer_exec" not in cmd.split(" ")[0]:
                pids.append(int(d))
        if pids:
            idle_since = now
        elif a.exit_when_idle and now - idle_since > a.exit_when_idle:
            return
        live = set(pids)
        for gone in [p for p in hist if p not in live]:
            hist.pop(gone, None); rss.pop(gone, None); alerted.pop(gone, None)
            for k in [k for k in spin_since if k[0] == gone]:
                spin_since.pop(k)
        for pid in pids:
            try:
                cpu, tree_running = tree_cpu_and_running(pid)
                threads = thread_states(pid)
                rss[pid].append(proc_rss_mib(pid))
            except (FileNotFoundError, ProcessLookupError, ValueError):
                continue
            h = hist[pid]
            h.append((now, cpu))
            # BLOCKED: no CPU progress over the window and nobody running
            old = [x for x in h if now - x[0] >= a.blocked_after]
            if old and "BLOCKED" not in alerted[pid]:
                t0, c0 = old[-1]
                if cpu - c0 < a.idle_cpu and not tree_running:
                    alerted[pid].add("BLOCKED")
                    alert("BLOCKED", pid, f"cpu+{cpu - c0:.2f}s in {now - t0:.0f}s, "
                          f"{len(threads)} threads", gdb_dump(pid))
                    if a.kill_blocked:
                        try:
                            os.kill(pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        continue
            # SPINNING: a thread continuously on CPU
            for tid, (state, _) in threads.items():
                key = (pid, tid)
                if state == "R":
                    spin_since.setdefault(key, now)
                else:
                    spin_since.pop(key, None)
            spinning = [k for k, t in spin_since.items() if k[0] == pid and now - t >= a.spin_after]
            if spinning and "SPINNING" not in alerted[pid]:
                alerted[pid].add("SPINNING")
                d1 = gdb_dump(pid)
                time.sleep(a.spin_gap)
                d2 = gdb_dump(pid)
                same = inner_frames(d1) and inner_frames(d1) == inner_frames(d2)
                alert("SPINNING", pid, f"thread(s) {[k[1] for k in spinning]} on CPU "
                      f">= {a.spin_after:.0f}s; innermost frames "
                      f"{'IDENTICAL (probable livelock)' if same else 'changed (progressing)'}",
                      d1 + "\n==== after gap ====\n" + d2)
            # GROWING: monotonic RSS above the threshold
            r = list(rss[pid])
            if (len(r) == a.rss_samples and r[-1] > a.rss_alert
                    and all(x < y for x, y in zip(r, r[1:])) and "GROWING" not in alerted[pid]):
                alerted[pid].add("GROWING")
                alert("GROWING", pid, f"rss {r[0]:.0f}->{r[-1]:.0f} MiB over "
                      f"{a.rss_samples} samples", gdb_dump(pid))
        time.sleep(a.interval)


if __name__ == "__main__":
    main()
