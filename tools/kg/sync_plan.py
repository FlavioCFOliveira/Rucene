#!/usr/bin/env python3
"""Decide how `sync_commit.sh` syncs a commit: in full, or incrementally.

A full sync rebuilds every layer of the graph from its sources. Most of them do
not change between two commits: the Lucene and library surveys read pinned
clones, and a commit touches a few crates. An incremental sync re-derives only
what the commit's diff can change, and must leave the graph a full sync would
leave (`compare_kg.py` proves it).

**Eligibility.** The incremental mode needs a base it can trust:

* the state file `<kg-target>/sync_state.json`, written by `record` as the last
  step of a sync whose audit passed, names the commit that sync left the graph
  at -- and the graph's `Project.syncedCommit` must be that same commit, so a
  sync run or interrupted since (which never records) disqualifies it;
* the **fingerprint** recorded there must equal the current one: every file of
  `tools/kg` but its tests, every clone `port_modules_kg.repo_roots` names (its
  `HEAD` and, when dirty, its changes; a root that is no git checkout by the
  size and mtime of every file), the nightly `rustc`, `rust-analyzer` and `rmp`
  versions. A tool, scope, extractor or toolchain change re-derives everything,
  so it is a full sync, as the sprint close is (`--full`);
* the base commit must exist in the repository.

Anything else is a full sync, and the reason is printed.

**What the diff changes.** `git diff --name-only --no-renames <base> <commit>`
assigns each path to a workspace crate: a path under `crates/<dir>/` is that
member's; `src/`, `tests/`, `benches/`, `examples/`, `build.rs` and the root
`Cargo.toml` are the root crate's; `Cargo.lock`, `rust-toolchain*` and
`.cargo/` change every crate (**global**). Any other path (a README, `specs/`,
`tools/`) belongs to no crate, unless a crate reads it through an
`include!`/`include_str!`/`include_bytes!` literal, which makes it that
crate's. Two sets follow:

* **expand**: the crates whose macro expansion (`-Zunpretty=expanded`) may
  differ -- a changed crate, and every crate that depends (transitively,
  dev-dependencies included) on a crate whose changed files define a
  `macro_rules!` before or after the change: an expansion reads its
  dependencies only through the macros it invokes;
* **index**: the crates whose rust-analyzer occurrences may differ -- a changed
  crate and every crate that depends on one (a signature change re-resolves
  its callers). It is empty when no crate changed.

Usage:
    sync_plan.py plan <commit> --roadmap R [--full] [--pretend-unchanged CRATE]
        prints `KEY=value` lines for the shell and writes
        `<kg-target>/sync_plan_<short>.json`
    sync_plan.py record <commit> --roadmap R
        writes the state file after a sync whose audit passed

`--pretend-unchanged` drops a changed crate from both sets: the positive control
of the equivalence check, never a mode to sync with.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from kgio import kg_target, read  # noqa: E402
import port_modules_kg as pm  # noqa: E402

KG_DIR = Path(__file__).resolve().parent
REPO = KG_DIR.parents[1]
STATE = "sync_state.json"
ROOT_CRATE = "rucene"
ROOT_PATHS = ("src/", "tests/", "benches/", "examples/")
ROOT_FILES = ("build.rs", "Cargo.toml")
GLOBAL_PATHS = ("Cargo.lock", ".cargo/", "rust-toolchain", "rust-toolchain.toml")
RE_INCLUDE = re.compile(r'include(?:_str|_bytes)?!\s*\(\s*"([^"]+)"\s*\)')
RE_DEP_TABLE = re.compile(r"^\[(dependencies|dev-dependencies|build-dependencies)\]\s*$")
RE_DEP_LINE = re.compile(r"^([A-Za-z0-9_-]+)\s*=")


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd or REPO, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed: {r.stderr.strip()[:300]}")
    return r.stdout


def _version(cmd):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        return (r.stdout or r.stderr).strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        return f"unavailable: {e}"


def _tree_digest(root: Path, h):
    """Into `h`: a git checkout's `HEAD` and dirty files (a root inside a
    clone, as `icu4j/` of the ICU repository, counts the clone's), else every
    file's path, size and mtime."""
    if subprocess.run(["git", "-C", str(root), "rev-parse", "--git-dir"],
                      capture_output=True).returncode == 0:
        h.update(git("rev-parse", "HEAD", cwd=root).encode())
        dirty = git("status", "--porcelain", cwd=root)
        h.update(dirty.encode())
        top = Path(git("rev-parse", "--show-toplevel", cwd=root).strip())
        for line in dirty.splitlines():
            p = top / line[3:].strip()
            if p.is_file():
                h.update(p.read_bytes())
        return
    for p in sorted(root.rglob("*")):
        if p.is_file():
            st = p.stat()
            h.update(f"{p.relative_to(root)}\0{st.st_size}\0{st.st_mtime_ns}\n".encode())


def fingerprint(roots=None):
    """The sha256 of everything an incremental sync assumes unchanged."""
    h = hashlib.sha256()
    for p in sorted(KG_DIR.rglob("*")):
        rel = p.relative_to(KG_DIR)
        if (p.is_file() and rel.parts[0] not in ("tests", "__pycache__")
                and p.suffix not in (".pyc",)):
            h.update(str(rel).encode() + b"\0" + p.read_bytes())
    roots = roots or pm.repo_roots(os.environ.get("LUCENE_ROOT"),
                                   os.environ.get(pm.RT_ROOT_ENV))
    for name in sorted(roots):
        h.update(f"[{name}]".encode())
        if Path(roots[name]).is_dir():
            _tree_digest(Path(roots[name]), h)
        else:
            h.update(b"missing")
    for cmd in (["rustc", "+nightly", "-V"], ["rust-analyzer", "--version"],
                ["rmp", "--version"]):
        h.update(_version(cmd).encode())
    return h.hexdigest()


# ---------------------------------------------------------------------------
# The workspace at one commit: crates, their directories and dependencies
# ---------------------------------------------------------------------------

def _show(commit, path):
    r = subprocess.run(["git", "show", f"{commit}:{path}"], cwd=REPO,
                       capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None


def workspace(commit):
    """`(dirs, deps)`: `{crate: dir}` (the root crate's dir is `""`) and
    `{crate: {workspace crates it depends on}}`, dev- and build-dependencies
    included, read from the commit's manifests."""
    dirs = {ROOT_CRATE: ""}
    manifests = {ROOT_CRATE: _show(commit, "Cargo.toml") or ""}
    for path in git("ls-tree", "--name-only", f"{commit}:crates").split():
        text = _show(commit, f"crates/{path}/Cargo.toml")
        if text is None:
            continue
        m = re.search(r'^\[package\][^\[]*?^\s*name\s*=\s*"([^"]+)"', text, re.M | re.S)
        if m:
            dirs[m.group(1)] = f"crates/{path}"
            manifests[m.group(1)] = text
    deps = {}
    for crate, text in manifests.items():
        found, table = set(), False
        for line in text.splitlines():
            s = line.strip()
            if s.startswith("["):
                table = bool(RE_DEP_TABLE.match(s))
                continue
            m = RE_DEP_LINE.match(s) if table else None
            if m and m.group(1) in dirs and m.group(1) != crate:
                found.add(m.group(1))
        deps[crate] = found
    return dirs, deps


def crate_of(path, dirs):
    """The crate a changed path belongs to, `"*"` for a global path, or `None`."""
    if path in GLOBAL_PATHS or path.startswith(GLOBAL_PATHS):
        return "*"
    for crate, d in dirs.items():
        if d and path.startswith(d + "/"):
            return crate
    if path.startswith(ROOT_PATHS) or path in ROOT_FILES:
        return ROOT_CRATE
    return None


def includes(commit, dirs):
    """`{included path: crate}` for every literal `include*!` of the commit's
    Rust files, resolved against the including file's directory."""
    out = {}
    files = git("ls-tree", "-r", "--name-only", commit).splitlines()
    for f in files:
        if not f.endswith(".rs"):
            continue
        crate = crate_of(f, dirs)
        if crate in (None, "*"):
            continue
        text = _show(commit, f) or ""
        if "include" not in text:
            continue
        for lit in RE_INCLUDE.findall(text):
            target = os.path.normpath(os.path.join(os.path.dirname(f), lit))
            out[target] = crate
    return out


def dependents(crates, deps):
    """`crates` and every crate that depends on one of them, transitively."""
    out, grew = set(crates), True
    while grew:
        grew = False
        for c, ds in deps.items():
            if c not in out and ds & out:
                out.add(c)
                grew = True
    return out


def defines_macro(base, commit, path):
    return any("macro_rules!" in (_show(c, path) or "") for c in (base, commit))


def affected(base, commit, changed, pretend_unchanged=()):
    """`(changed crates, expand set, index set, unassigned paths)` for the diff."""
    dirs, deps = workspace(commit)
    base_dirs, base_deps = workspace(base)
    for c, d in base_dirs.items():
        dirs.setdefault(c, d)
    for c, ds in base_deps.items():
        deps[c] = deps.get(c, set()) | ds
    inc = {**includes(base, dirs), **includes(commit, dirs)}
    crates, macro_crates, loose = set(), set(), []
    for path in changed:
        c = crate_of(path, dirs) or inc.get(path)
        if c is None:
            loose.append(path)
            continue
        if c == "*":
            crates |= set(dirs)
            macro_crates |= set(dirs)
            continue
        crates.add(c)
        if path.endswith(".rs") and defines_macro(base, commit, path):
            macro_crates.add(c)
    crates -= set(pretend_unchanged)
    macro_crates -= set(pretend_unchanged)
    expand = crates | (dependents(macro_crates, deps) - set(pretend_unchanged))
    index = dependents(crates, deps) - set(pretend_unchanged) if crates else set()
    return crates, expand, index, loose


# ---------------------------------------------------------------------------
# State and plan
# ---------------------------------------------------------------------------

def state_path():
    return kg_target() / STATE


def synced_commit(roadmap):
    rows = read(roadmap, "MATCH (p:Project {name: 'Rucene'}) RETURN p.syncedCommit")
    return rows[0][0] if rows else None


def plan(commit, roadmap, force_full=False, pretend_unchanged=()):
    full_hash = git("rev-parse", "--verify", f"{commit}^{{commit}}").strip()
    out = {"commit": full_hash, "mode": "full", "reason": None, "base": None,
           "changed": [], "crates": [], "expand": [], "index": [], "loose": []}
    st = None
    if state_path().is_file():
        st = json.loads(state_path().read_text(encoding="utf-8"))
    synced = synced_commit(roadmap)
    fp = fingerprint()
    out["fingerprint"] = fp
    if force_full:
        out["reason"] = "--full"
    elif st is None:
        out["reason"] = f"no state file ({state_path()})"
    elif st.get("roadmap") != roadmap:
        out["reason"] = f"state recorded for roadmap {st.get('roadmap')!r}"
    elif st.get("commit") != synced:
        out["reason"] = (f"graph syncedCommit {str(synced)[:7]} is not the recorded "
                         f"{str(st.get('commit'))[:7]}")
    elif st.get("fingerprint") != fp:
        out["reason"] = "tools, clones or toolchain changed since the recorded sync"
    elif subprocess.run(["git", "cat-file", "-e", f"{st['commit']}^{{commit}}"],
                        cwd=REPO, capture_output=True).returncode != 0:
        out["reason"] = f"base commit {st['commit'][:7]} not in the repository"
    else:
        base = st["commit"]
        changed = git("diff", "--name-only", "--no-renames", base, full_hash).split()
        crates, expand, index, loose = affected(base, full_hash, changed, pretend_unchanged)
        out.update(mode="incremental", base=base, baseShort=base[:7], changed=changed,
                   crates=sorted(crates), expand=sorted(expand), index=sorted(index),
                   loose=loose, refs=st.get("refs"),
                   pretendUnchanged=sorted(pretend_unchanged))
    path = kg_target() / f"sync_plan_{full_hash[:7]}.json"
    path.write_text(json.dumps(out, indent=1), encoding="utf-8")
    return out, path


def record(commit, roadmap, refs):
    full_hash = git("rev-parse", "--verify", f"{commit}^{{commit}}").strip()
    if synced_commit(roadmap) != full_hash:
        raise SystemExit(f"graph syncedCommit is not {full_hash[:7]}: nothing recorded")
    st = {"commit": full_hash, "roadmap": roadmap, "fingerprint": fingerprint(),
          "refs": refs}
    tmp = state_path().with_suffix(".tmp")
    tmp.write_text(json.dumps(st, indent=1), encoding="utf-8")
    tmp.replace(state_path())
    print(f"sync state recorded: {full_hash[:7]} -> {state_path()}", file=sys.stderr)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("plan")
    p.add_argument("commit")
    p.add_argument("--roadmap", required=True)
    p.add_argument("--full", action="store_true")
    p.add_argument("--pretend-unchanged", action="append", default=[])
    r = sub.add_parser("record")
    r.add_argument("commit")
    r.add_argument("--roadmap", required=True)
    r.add_argument("--refs", required=True)
    a = ap.parse_args(argv)
    if a.cmd == "record":
        record(a.commit, a.roadmap, a.refs)
        return
    out, path = plan(a.commit, a.roadmap, a.full, tuple(a.pretend_unchanged))
    print(f"MODE={out['mode']}")
    print(f"PLAN={path}")
    if out["mode"] == "incremental":
        print(f"BASE={out['base']}")
        print(f"EXPAND={','.join(out['expand'])}")
        print(f"INDEX={','.join(out['index'])}")
        print(f"PREV_REFS={out['refs'] or ''}")
        print(f"# incremental from {out['base'][:7]}: {len(out['changed'])} paths changed; "
              f"crates {out['crates'] or 'none'}; re-expand {out['expand'] or 'none'}; "
              f"re-index {out['index'] or 'none'}"
              + (f"; PRETENDING UNCHANGED {out['pretendUnchanged']}"
                 if out['pretendUnchanged'] else ""))
    else:
        print(f"# full sync: {out['reason']}")


if __name__ == "__main__":
    main()
