"""A content-addressed disk memo for the pure text functions of the KG tools.

The survey, `expanded_evidence_kg.py`, `rust_refs_kg.py` and the member triage
mask and scan every Rust file of the workspace, and the expanded crates, once
each per sync (measured on the incremental sync of `9d38f1b`, under cProfile:
`extract_rucene_kg.mask` 158 s of the survey's 281 s, 85 s of
`expanded_evidence_kg`'s 120 s, 53 s of `rust_refs_kg`'s 150 s), although a
commit changes a few files. These functions are pure: their result depends on
their arguments and on the code of their module, nothing else. `disk_memo`
stores each result under the sha256 of the function's module source, its name
and its pickled arguments, so a result is reused only for the same text read by
the same code -- exactly what recomputing it would return. A code change misses
every entry; a file the commit changed misses its own.

Entries go to `<kg-target>/cache/<function>/<sha>.pkl` (written to a temporary
name, then renamed, so a reader never sees half an entry). `KG_CACHE=off`
disables the memo.
"""
import functools
import hashlib
import os
import pickle
import sys
from pathlib import Path

from kgio import kg_target

_CODE = {}


def _code_hash(module_name):
    if module_name not in _CODE:
        mod = sys.modules[module_name]
        h = hashlib.sha256(Path(mod.__file__).read_bytes())
        h.update(Path(__file__).read_bytes())
        _CODE[module_name] = h.hexdigest()
    return _CODE[module_name]


def disk_memo(fn):
    """Memoize `fn` on disk, keyed by its module's source and its arguments."""
    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        if os.environ.get("KG_CACHE") == "off":
            return fn(*args, **kwargs)
        h = hashlib.sha256(_code_hash(fn.__module__).encode())
        h.update(fn.__qualname__.encode())
        h.update(pickle.dumps((args, sorted(kwargs.items())), protocol=4))
        path = kg_target() / "cache" / fn.__qualname__ / f"{h.hexdigest()}.pkl"
        try:
            with path.open("rb") as f:
                return pickle.load(f)
        except (FileNotFoundError, EOFError, pickle.UnpicklingError):
            pass
        value = fn(*args, **kwargs)
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(f".{os.getpid()}.tmp")
        with tmp.open("wb") as f:
            pickle.dump(value, f, protocol=4)
        tmp.replace(path)
        return value
    return wrapper
