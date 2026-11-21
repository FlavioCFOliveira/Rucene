#!/usr/bin/env python3
"""Exec a command after declaring it traceable by any process of this user.

`kernel.yama.ptrace_scope = 1` lets a debugger attach only to its own
descendants, so a watchdog cannot take a thread dump of a test it did not
start. `prctl(PR_SET_PTRACER, PR_SET_PTRACER_ANY)` lifts that restriction for
this process only, and the declaration survives `execve` (verified 2026-11-12
with `gdb -p` on an exec'd child). Usage: ptracer_exec.py <command> [args...]
"""
import ctypes
import os
import sys

PR_SET_PTRACER = 0x59616D61
PR_SET_PTRACER_ANY = ctypes.c_ulong(-1).value

if len(sys.argv) < 2:
    sys.exit("usage: ptracer_exec.py <command> [args...]")
try:
    ctypes.CDLL(None, use_errno=True).prctl(
        PR_SET_PTRACER, ctypes.c_ulong(PR_SET_PTRACER_ANY), 0, 0, 0)
except OSError:
    pass  # no Yama: nothing to lift
os.execvp(sys.argv[1], sys.argv[1:])
