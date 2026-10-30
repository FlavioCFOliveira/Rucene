#!/usr/bin/env python3
"""A minimal reader for SCIP indexes, the format `rust-analyzer scip` writes.

SCIP (`scip.proto`, github.com/sourcegraph/scip) is a protobuf message. Only
the fields the KG tools read are decoded, with a hand-written wire-format
reader, so the tools need no `protobuf` package:

* `Index.documents` (2);
* `Document.relative_path` (1), `occurrences` (2), `symbols` (3);
* `Occurrence.range` (1), `symbol` (2), `symbol_roles` (3),
  `enclosing_range` (7);
* `SymbolInformation.symbol` (1), `relationships` (4), `kind` (5),
  `display_name` (6), `enclosing_symbol` (8);
* `Relationship.symbol` (1), `is_reference` (2), `is_implementation` (3),
  `is_type_definition` (4), `is_definition` (5).

Every other field is skipped by wire type, as protobuf requires.

A SCIP range is `[startLine, startChar, endChar]` (one line) or
`[startLine, startChar, endLine, endChar]`; lines and characters are 0-based.
`decode_range` always returns the four-element form.
"""
from dataclasses import dataclass, field

ROLE_DEFINITION = 0x1
ROLE_IMPORT = 0x2


def _varint(buf, i):
    shift = 0
    out = 0
    while True:
        b = buf[i]
        i += 1
        out |= (b & 0x7F) << shift
        if b < 0x80:
            return out, i
        shift += 7


def _fields(buf, start=0, end=None):
    """Yield `(field_number, wire_type, value)`; a length-delimited value is a
    `(start, end)` slice of `buf`, a varint an int."""
    i = start
    end = len(buf) if end is None else end
    while i < end:
        key, i = _varint(buf, i)
        num, wt = key >> 3, key & 7
        if wt == 0:
            v, i = _varint(buf, i)
            yield num, wt, v
        elif wt == 2:
            n, i = _varint(buf, i)
            yield num, wt, (i, i + n)
            i += n
        elif wt == 1:
            yield num, wt, buf[i:i + 8]
            i += 8
        elif wt == 5:
            yield num, wt, buf[i:i + 4]
            i += 4
        else:
            raise ValueError(f"unsupported protobuf wire type {wt} at byte {i}")


def _packed_int32(buf, s, e):
    out = []
    i = s
    while i < e:
        v, i = _varint(buf, i)
        if v >= 1 << 31:  # int32 negatives are sign-extended to 64 bits
            v -= 1 << 64
        out.append(v)
    return out


def decode_range(r):
    """A SCIP range as `(startLine, startChar, endLine, endChar)`."""
    if not r:
        return None
    if len(r) == 3:
        return (r[0], r[1], r[0], r[2])
    return tuple(r[:4])


@dataclass
class Occurrence:
    range: tuple
    symbol: str
    roles: int = 0
    enclosing: tuple = None

    @property
    def is_definition(self):
        return bool(self.roles & ROLE_DEFINITION)


@dataclass
class Relationship:
    symbol: str
    is_reference: bool = False
    is_implementation: bool = False
    is_type_definition: bool = False
    is_definition: bool = False


@dataclass
class SymbolInfo:
    symbol: str
    kind: int = 0
    display_name: str = ""
    enclosing_symbol: str = ""
    relationships: list = field(default_factory=list)


@dataclass
class Document:
    path: str
    occurrences: list = field(default_factory=list)
    symbols: list = field(default_factory=list)


def _str(buf, s, e):
    return bytes(buf[s:e]).decode("utf-8")


def _occurrence(buf, s, e):
    rng, sym, roles, enc = None, "", 0, None
    for num, wt, v in _fields(buf, s, e):
        if num == 1:
            rng = _packed_int32(buf, *v) if wt == 2 else [v]
        elif num == 2:
            sym = _str(buf, *v)
        elif num == 3:
            roles = v
        elif num == 7:
            enc = decode_range(_packed_int32(buf, *v)) if wt == 2 else None
    return Occurrence(decode_range(rng), sym, roles, enc)


def _relationship(buf, s, e):
    rel = Relationship("")
    for num, _wt, v in _fields(buf, s, e):
        if num == 1:
            rel.symbol = _str(buf, *v)
        elif num == 2:
            rel.is_reference = bool(v)
        elif num == 3:
            rel.is_implementation = bool(v)
        elif num == 4:
            rel.is_type_definition = bool(v)
        elif num == 5:
            rel.is_definition = bool(v)
    return rel


def _symbol_info(buf, s, e):
    info = SymbolInfo("")
    for num, _wt, v in _fields(buf, s, e):
        if num == 1:
            info.symbol = _str(buf, *v)
        elif num == 4:
            info.relationships.append(_relationship(buf, *v))
        elif num == 5:
            info.kind = v
        elif num == 6:
            info.display_name = _str(buf, *v)
        elif num == 8:
            info.enclosing_symbol = _str(buf, *v)
    return info


def _document(buf, s, e):
    doc = Document("")
    for num, _wt, v in _fields(buf, s, e):
        if num == 1:
            doc.path = _str(buf, *v)
        elif num == 2:
            doc.occurrences.append(_occurrence(buf, *v))
        elif num == 3:
            doc.symbols.append(_symbol_info(buf, *v))
    return doc


def read_index(data):
    """Every `Document` of a SCIP index given as bytes."""
    buf = memoryview(data)
    return [_document(buf, *v) for num, wt, v in _fields(buf) if num == 2 and wt == 2]


def load(path):
    with open(path, "rb") as fh:
        return read_index(fh.read())
