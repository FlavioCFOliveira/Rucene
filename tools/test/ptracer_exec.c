/*
 * Exec a command after declaring it traceable by any process of this user.
 * Compiled replacement for ptracer_exec.py (same contract, no interpreter
 * start-up inside each test's systemd scope; task #608 R3).
 *
 * `kernel.yama.ptrace_scope = 1` lets a debugger attach only to its own
 * descendants, so tools/test/stall_watchdog.py cannot take a thread dump of a
 * test it did not start. `prctl(PR_SET_PTRACER, PR_SET_PTRACER_ANY)` lifts
 * that restriction for this process only, and the declaration survives
 * `execve`.
 *
 * Behaviour kept identical to ptracer_exec.py:
 * - a prctl failure is ignored (no Yama: nothing to lift);
 * - SIGPIPE and SIGXFSZ are ignored before the exec, because the Python
 *   interpreter ignores both at start-up and ignored dispositions survive
 *   `execve` (observed: SigIgn 0x1001000 in the exec'd process);
 * - `execvp` searches PATH; usage errors and exec failures exit with 1.
 *
 * Built by tools/test/build_ptracer_exec.sh, called from run_suite.sh.
 * Usage: ptracer_exec <command> [args...]
 */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <unistd.h>

#ifndef PR_SET_PTRACER
#define PR_SET_PTRACER 0x59616d61
#endif
#ifndef PR_SET_PTRACER_ANY
#define PR_SET_PTRACER_ANY ((unsigned long)-1)
#endif

int main(int argc, char **argv) {
  if (argc < 2) {
    fputs("usage: ptracer_exec <command> [args...]\n", stderr);
    return 1;
  }
  (void)prctl(PR_SET_PTRACER, PR_SET_PTRACER_ANY, 0, 0, 0);
  signal(SIGPIPE, SIG_IGN);
  signal(SIGXFSZ, SIG_IGN);
  execvp(argv[1], argv + 1);
  fprintf(stderr, "ptracer_exec: %s: %s\n", argv[1], strerror(errno));
  return 1;
}
