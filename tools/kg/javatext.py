#!/usr/bin/env python3
"""The Java lexical pass both Lucene-side scanners need.

Every scanner over the reference tree has to see code and only code: a `{` in a
string literal, a `"` in a line comment or an apostrophe in a Javadoc sentence
all derail brace counting and regex matching. Doing that with a sequence of
`re.sub` calls cannot work, because the four token classes are mutually
exclusive in *source order*, not in substitution order: stripping strings before
line comments makes a `//` comment holding an odd number of `"` open a phantom
string that swallows the code after it. That defect deleted
`Automaton#toDot` (`util/automaton/Automaton.java:579`, killed by the wrapped
comment at line 601) and `RegExp#parseCharExp`
(`util/automaton/RegExp.java:1525`) from the graph.

`strip_java` is therefore a single left-to-right scan, which is the only order
that is right. It lives here rather than in either caller so the loader
(`enrich_lucene_kg.py`) and the audit (`audit_kg.py`) share one implementation
instead of drifting apart -- the audit's independence from the loader rests on
its *scanner*, `JAVA_DECL` plus brace depth against the loader's declaration
regex plus a type stack, and that independence is untouched by agreeing on what
a token is.
"""


def strip_java(src: str) -> str:
    """Blank comments, strings and char literals, preserving length."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif c == "/" and i + 1 < n and src[i + 1] == "*":
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append(" " * (j - i))
            i = j
        elif c == '"':
            if src.startswith('""""', i) or src.startswith('"""', i):
                j = src.find('"""', i + 3)
                j = n if j < 0 else j + 3
            else:
                j = i + 1
                while j < n and src[j] != '"':
                    j += 2 if src[j] == "\\" else 1
                j += 1
            out.append(" " * (j - i))
            i = j
        elif c == "'":
            j = i + 1
            while j < n and src[j] != "'":
                j += 2 if src[j] == "\\" else 1
            j += 1
            out.append(" " * (j - i))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)
