"""Lexical helpers for reading Java source files as `javac` does (JLS chapter 3).

Shared by the generators of this directory, which turn Lucene's generated Java
sources (JFlex tables, Unicode test data) into Rust.
"""
import re


def utf16_to_str(units: list) -> str:
    """Decode UTF-16 code units, failing on an unpaired surrogate."""
    data = b"".join(u.to_bytes(2, "little") for u in units)
    return data.decode("utf-16-le", errors="strict")


def rust_string_literal(s: str) -> str:
    """A Rust string literal for `s`: printable ASCII as is, the rest escaped."""
    out = []
    for ch in s:
        o = ord(ch)
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif 0x20 <= o < 0x7F:
            out.append(ch)
        else:
            out.append("\\u{%X}" % o)
    return '"' + "".join(out) + '"'


def unicode_preprocess(src: str) -> str:
    """JLS 3.3: translate `\\uXXXX` escapes (any number of `u`s) before lexing.

    A backslash preceded by an odd number of contiguous backslashes is not an
    escape start (JLS 3.3).
    """
    out = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if c == "\\":
            # count preceding backslashes to know whether this one is eligible
            j = i - 1
            k = 0
            while j >= 0 and src[j] == "\\":
                k += 1
                j -= 1
            if k % 2 == 0 and i + 1 < n and src[i + 1] == "u":
                m = i + 1
                while m < n and src[m] == "u":
                    m += 1
                hexdigits = src[m:m + 4]
                if len(hexdigits) != 4 or not re.fullmatch(r"[0-9a-fA-F]{4}", hexdigits):
                    raise SystemExit(f"malformed unicode escape at {i}")
                out.append(chr(int(hexdigits, 16)))
                i = m + 4
                continue
        out.append(c)
        i += 1
    return "".join(out)


def string_literal_body(body: str) -> list:
    """JLS 3.10.6/3.10.7: the UTF-16 code units of one string literal body.

    `body` has already had its Unicode escapes translated. Only the escapes
    JFlex emits are accepted (octal escapes); anything else fails loudly.
    """
    units = []
    i = 0
    n = len(body)
    while i < n:
        c = body[i]
        if c == "\\":
            i += 1
            d = body[i]
            if d in "01234567":
                maxlen = 3 if d in "0123" else 2
                j = i
                while j < n and j - i < maxlen and body[j] in "01234567":
                    j += 1
                units.append(int(body[i:j], 8))
                i = j
                continue
            simple = {"b": 8, "t": 9, "n": 10, "f": 12, "r": 13, '"': 34, "'": 39, "\\": 92}
            if d in simple:
                units.append(simple[d])
                i += 1
                continue
            raise SystemExit(f"unsupported escape \\{d}")
        if c in "\r\n":
            raise SystemExit("line terminator inside a string literal")
        cp = ord(c)
        if cp > 0xFFFF:
            hi = 0xD800 + ((cp - 0x10000) >> 10)
            lo = 0xDC00 + ((cp - 0x10000) & 0x3FF)
            units.extend([hi, lo])
        else:
            units.append(cp)
        i += 1
    return units
