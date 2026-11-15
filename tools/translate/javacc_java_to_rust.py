#!/usr/bin/env python3
"""Translate the JavaCC 7 output Lucene 10.5.0 commits into Rust.

Input: the generated Java of a JavaCC 7 grammar of `lucene/queryparser`, as
Lucene commits it (`gradle/generation/javacc.gradle` edits JavaCC's output):
`<Parser>.java`, `<Parser>TokenManager.java`, `<Parser>Constants.java`,
`Token.java`, `TokenMgrError.java` and `ParseException.java`. Each parser is a
*profile* (below): `classic` (`queryparser.classic.QueryParser`),
`surround` (`queryparser.surround.parser.QueryParser`) and `standard`
(`queryparser.flexible.standard.parser.StandardSyntaxParser`).

Output, per profile:

* `<parser>_constants.rs`, wholly generated: the token kinds and lexical
  states as `pub const` items, `TOKEN_IMAGE`, and the interface as a trait
  whose associated constants are those items;
* `<parser>_token_manager.rs`, wholly generated: the token manager, every
  member translated statement for statement;
* `token.rs`, `token_mgr_error.rs`, `parse_exception.rs`, wholly generated
  from Rust templates of the JavaCC 7 classes: the Java file must equal the
  JavaCC 7 template token for token (its package line aside; the SHA-256 of
  its tokens is pinned below), otherwise the run fails. The flexible parser's
  `ParseException.java`, which Lucene edits into a `QueryNodeParseException`
  reporting an NLS message, has its own template and pin
  (`ParseException:flexible`);
* the regions between `// @generated begin <region>` and
  `// @generated end <region>` of the hand-written `<parser>.rs`: `state` (the
  JavaCC state, a struct), `productions` (the public members, as provided
  methods of the parser's `...Ops` trait) and `private` (the private members,
  as generic free functions). The user code of the grammar's `PARSER_BEGIN`
  section, which the generated class also holds, is translated by hand in
  that file; its Java tokens are pinned by SHA-256 below.

The grammar actions inside the productions are translated with the members
they call described per profile: the external methods (`EXTERNALS`), and for
the flexible parser the constructors (`news`), static and instance calls,
static fields, reference casts, `instanceof` tests, widening conversions and
enum types of its query node and interval function classes. The lookahead machinery maps to
Rust control flow: the `jj_3*` routines and `jj_scan_token` return
`Result<bool, Raised>`, `throw jj_ls` is `Err(Raised::LookaheadSuccess)`, and
the `try { return !jj_3_N(); } catch (LookaheadSuccess ls) { return true; }
finally { jj_save(..); }` of `jj_2_N` is a match that runs `jj_save` on every
outcome.

The translator understands the subset of Java those files use and nothing
more: any other construct, any member, type or call it has no rule for, stops
the run with an error naming the file and line. It never guesses.

Usage: python3 tools/translate/javacc_java_to_rust.py [LUCENE_CHECKOUT]
       (default /tmp/lucene1050); `--check` compares the outputs with the
       committed files instead of writing them and exits 1 on a difference;
       `--only <profile>` processes one profile; `--hashes` prints the
       SHA-256 pins.
"""

from __future__ import annotations

import hashlib
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = "tools/translate/javacc_java_to_rust.py"
CRATE = "crates/rucene-queryparser/src"
JAVA_ROOT = "lucene/queryparser/src/java/org/apache/lucene/queryparser"


class TranslationError(Exception):
    pass


def fail(where: str, message: str):
    raise TranslationError(f"{where}: {message}")


# ==========================================================================
# Lexer
# ==========================================================================

KEYWORDS = {
    "abstract", "boolean", "break", "byte", "case", "catch", "char", "class",
    "continue", "default", "do", "double", "else", "enum", "extends", "final",
    "finally", "float", "for", "if", "implements", "import", "instanceof",
    "int", "interface", "long", "new", "package", "private", "protected",
    "public", "return", "short", "static", "super", "switch", "synchronized",
    "this", "throw", "throws", "try", "void", "volatile", "while", "true",
    "false", "null", "transient",
}

OPERATORS = sorted(
    [
        ">>>=", "<<=", ">>=", ">>>", "...", "->", "::", "++", "--", "&&", "||",
        ">>", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
        "<<", "(", ")", "{", "}", "[", "]", ";", ",", ".", "@", "=", ">", "<",
        "!", "~", "?", ":", "+", "-", "*", "/", "&", "|", "^", "%",
    ],
    key=len,
    reverse=True,
)


@dataclass
class Tok:
    kind: str  # ident, keyword, int, long, char, string, op, eof
    text: str
    line: int
    value: object = None


def unescape_java(body: str, where: str) -> list:
    """The UTF-16 units of a Java string or char literal body."""
    out = []
    i = 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.extend(_utf16(c))
            i += 1
            continue
        i += 1
        e = body[i]
        simple = {"b": 8, "t": 9, "n": 10, "f": 12, "r": 13, '"': 34, "'": 39, "\\": 92, "s": 32}
        if e in simple:
            out.append(simple[e])
            i += 1
        elif e in "01234567":
            j = i
            limit = 3 if e in "0123" else 2
            while j < len(body) and j - i < limit and body[j] in "01234567":
                j += 1
            out.append(int(body[i:j], 8))
            i = j
        else:
            fail(where, f"unknown escape \\{e}")
    return out


def _utf16(c: str) -> list:
    b = c.encode("utf-16-le")
    return [int.from_bytes(b[k:k + 2], "little") for k in range(0, len(b), 2)]


def unicode_preprocess(src: str) -> str:
    """JLS 3.3: replace each \\uXXXX escape (preceded by an even number of
    backslashes) by its character."""
    out = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if c == "\\":
            # count backslashes
            j = i
            while j < n and src[j] == "\\":
                j += 1
            run = j - i
            if run % 2 == 1 and j < n and src[j] == "u":
                out.append("\\" * (run - 1))
                k = j
                while k < n and src[k] == "u":
                    k += 1
                out.append(chr(int(src[k:k + 4], 16)))
                i = k + 4
                continue
            out.append(src[i:j])
            i = j
            continue
        out.append(c)
        i += 1
    return "".join(out)


def lex(src: str, where: str) -> list:
    src = unicode_preprocess(src)
    toks = []
    i = 0
    line = 1
    n = len(src)
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if c in " \t\r\f":
            i += 1
            continue
        if src.startswith("//", i):
            while i < n and src[i] != "\n":
                i += 1
            continue
        if src.startswith("/*", i):
            j = src.index("*/", i + 2)
            line += src.count("\n", i, j)
            i = j + 2
            continue
        if c.isalpha() or c == "_" or c == "$":
            j = i
            while j < n and (src[j].isalnum() or src[j] in "_$"):
                j += 1
            word = src[i:j]
            toks.append(Tok("keyword" if word in KEYWORDS else "ident", word, line))
            i = j
            continue
        if c.isdigit():
            m = re.match(r"0[xX][0-9a-fA-F_]+[lL]?|[0-9][0-9_]*(\.[0-9]+)?([eE][+-]?[0-9]+)?[fFdDlL]?", src[i:])
            text = m.group(0)
            i += len(text)
            t = text.replace("_", "")
            if t[-1] in "lL":
                body = t[:-1]
                v = int(body, 16) if body[:2].lower() == "0x" else (int(body, 8) if len(body) > 1 and body[0] == "0" else int(body))
                toks.append(Tok("long", text, line, v))
            elif not t.lower().startswith("0x") and ("." in t or t[-1] in "fFdD" or "e" in t.lower()):
                toks.append(Tok("double" if t[-1] not in "fF" else "float", text, line, t.rstrip("fFdD")))
            else:
                v = int(t, 16) if t[:2].lower() == "0x" else (int(t, 8) if len(t) > 1 and t[0] == "0" else int(t))
                toks.append(Tok("int", text, line, v))
            continue
        if c == '"':
            j = i + 1
            while src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            toks.append(Tok("string", src[i:j + 1], line, unescape_java(src[i + 1:j], f"{where}:{line}")))
            i = j + 1
            continue
        if c == "'":
            j = i + 1
            while src[j] != "'":
                j += 2 if src[j] == "\\" else 1
            units = unescape_java(src[i + 1:j], f"{where}:{line}")
            if len(units) != 1:
                fail(f"{where}:{line}", "char literal of more than one unit")
            toks.append(Tok("char", src[i:j + 1], line, units[0]))
            i = j + 1
            continue
        for op in OPERATORS:
            if src.startswith(op, i):
                toks.append(Tok("op", op, line))
                i += len(op)
                break
        else:
            fail(f"{where}:{line}", f"unexpected character {c!r}")
    toks.append(Tok("eof", "", line))
    return toks


def token_hash(toks: list) -> str:
    h = hashlib.sha256()
    for t in toks:
        if t.kind == "eof":
            break
        h.update(t.text.encode("utf-8"))
        h.update(b"\x00")
    return h.hexdigest()


# ==========================================================================
# Parser (a Java subset) into a tuple AST
# ==========================================================================


@dataclass
class JType:
    name: str  # int, long, boolean, char, float, double, void, or a class name
    args: tuple = ()
    dims: int = 0

    def elem(self) -> "JType":
        return JType(self.name, self.args, self.dims - 1)

    def __str__(self):
        s = self.name
        if self.args:
            s += "<" + ",".join(str(a) for a in self.args) + ">"
        return s + "[]" * self.dims

    def is_prim(self):
        return self.dims == 0 and self.name in ("int", "long", "boolean", "char", "float", "double", "short", "byte")

    def __hash__(self):
        return hash(str(self))

    def __eq__(self, other):
        return isinstance(other, JType) and str(self) == str(other)


T_INT = JType("int")
T_LONG = JType("long")
T_BOOL = JType("boolean")
T_CHAR = JType("char")
T_FLOAT = JType("float")
T_DOUBLE = JType("double")
T_VOID = JType("void")
T_STRING = JType("String")
T_NULL = JType("null")


class Parser:
    def __init__(self, toks, where):
        self.toks = toks
        self.p = 0
        self.where = where

    def peek(self, k=0):
        return self.toks[self.p + k]

    def at(self, text, k=0):
        t = self.peek(k)
        return t.text == text and t.kind in ("op", "keyword")

    def next(self):
        t = self.toks[self.p]
        self.p += 1
        return t

    def expect(self, text):
        t = self.next()
        if t.text != text:
            self.err(f"expected {text!r}, got {t.text!r}", t)
        return t

    def err(self, msg, t=None):
        t = t or self.peek()
        fail(f"{self.where}:{t.line}", msg)

    def ident(self):
        t = self.next()
        if t.kind != "ident":
            self.err(f"expected an identifier, got {t.text!r}", t)
        return t.text

    # ---------------------------------------------------------------- types
    def qualified_name(self):
        parts = [self.ident()]
        while self.at(".") and self.peek(1).kind == "ident":
            self.next()
            parts.append(self.ident())
        return ".".join(parts)

    def parse_type(self):
        t = self.peek()
        if t.kind == "keyword" and t.text in ("int", "long", "boolean", "char", "void", "float", "double", "short", "byte"):
            self.next()
            name = t.text
            args = ()
        else:
            name = self.qualified_name()
            if name.startswith("java.util.") or name.startswith("java.io.") or name.startswith("java.lang."):
                name = name.split(".")[-1]
            args = ()
            if self.at("<"):
                self.next()
                lst = []
                if not self.at(">"):
                    lst.append(self.parse_type())
                    while self.at(","):
                        self.next()
                        lst.append(self.parse_type())
                self.expect(">")
                args = tuple(lst)
        dims = 0
        while self.at("[") and self.at("]", 1):
            self.next()
            self.next()
            dims += 1
        return JType(name, args, dims)

    def try_type_then_ident(self):
        """A local variable declaration starts with a type and a name."""
        save = self.p
        try:
            t = self.peek()
            if t.kind not in ("ident", "keyword"):
                return None
            if t.kind == "keyword" and t.text not in ("int", "long", "boolean", "char", "float", "double", "short", "byte"):
                return None
            ty = self.parse_type()
            if self.peek().kind == "ident" and (self.at("=", 1) or self.at(";", 1) or self.at(",", 1) or self.at(":", 1)):
                return ty
        except TranslationError:
            pass
        self.p = save
        return None

    # ---------------------------------------------------------- compilation
    def modifiers(self):
        mods = []
        while True:
            t = self.peek()
            if t.text == "@" and t.kind == "op":
                self.next()
                self.qualified_name()
                if self.at("("):
                    depth = 0
                    while True:
                        x = self.next()
                        if x.text == "(":
                            depth += 1
                        elif x.text == ")":
                            depth -= 1
                            if depth == 0:
                                break
                continue
            if t.kind == "keyword" and t.text in ("public", "private", "protected", "static", "final", "abstract", "synchronized", "transient", "volatile"):
                mods.append(self.next().text)
                continue
            return mods

    def compilation_unit(self):
        types = []
        while self.peek().kind != "eof":
            if self.at("package") or self.at("import"):
                while not self.at(";"):
                    self.next()
                self.next()
                continue
            types.append(self.type_decl())
        return types

    def type_decl(self):
        line = self.peek().line
        mods = self.modifiers()
        kw = self.next().text
        if kw not in ("class", "interface", "enum"):
            self.err(f"expected a type declaration, got {kw!r}")
        name = self.ident()
        extends = None
        implements = []
        if self.at("extends"):
            self.next()
            extends = self.parse_type()
        if self.at("implements"):
            self.next()
            implements.append(self.parse_type())
            while self.at(","):
                self.next()
                implements.append(self.parse_type())
        start = self.p
        self.expect("{")
        members = []
        if kw == "enum":
            consts = [self.ident()]
            while self.at(","):
                self.next()
                consts.append(self.ident())
            if self.at(";"):
                self.next()
            self.expect("}")
            return {"kind": "enum", "name": name, "mods": mods, "consts": consts, "line": line,
                    "toks": self.toks[start - 1:self.p]}
        while not self.at("}"):
            members.append(self.member(name, kw == "interface"))
        self.expect("}")
        return {"kind": kw, "name": name, "mods": mods, "extends": extends, "implements": implements,
                "members": members, "line": line}

    def member(self, class_name, interface):
        start = self.p
        line = self.peek().line
        if self.at(";"):
            self.next()
            return {"kind": "empty", "line": line, "toks": self.toks[start:self.p]}
        mods = self.modifiers()
        if self.at("{"):
            body = self.block()
            return {"kind": "init", "static": "static" in mods, "body": body, "line": line,
                    "toks": self.toks[start:self.p]}
        if self.at("class") or self.at("interface") or self.at("enum"):
            self.p = start
            decl = self.type_decl()
            decl["toks"] = self.toks[start:self.p]
            return {"kind": "type", "decl": decl, "line": line, "toks": self.toks[start:self.p]}
        # constructor?
        if self.peek().kind == "ident" and self.peek().text == class_name and self.at("(", 1):
            self.next()
            params = self.params()
            throws = self.throws()
            body = self.block()
            return {"kind": "ctor", "mods": mods, "params": params, "body": body, "line": line,
                    "toks": self.toks[start:self.p]}
        ty = self.parse_type()
        name = self.ident()
        if self.at("("):
            params = self.params()
            throws = self.throws()
            if self.at(";"):
                self.next()
                body = None
            else:
                body = self.block()
            return {"kind": "method", "mods": mods, "type": ty, "name": name, "params": params,
                    "throws": throws, "body": body, "line": line, "toks": self.toks[start:self.p]}
        decls = [(name, self.var_init(ty))]
        while self.at(","):
            self.next()
            n = self.ident()
            decls.append((n, self.var_init(ty)))
        self.expect(";")
        if interface:
            mods = mods + ["static", "final", "public"]
        return {"kind": "field", "mods": mods, "type": ty, "decls": decls, "line": line,
                "toks": self.toks[start:self.p]}

    def var_init(self, ty):
        dims = 0
        while self.at("["):
            self.next()
            self.expect("]")
            dims += 1
        if self.at("="):
            self.next()
            if self.at("{"):
                return self.array_init()
            return self.expr()
        return None

    def array_init(self):
        line = self.peek().line
        self.expect("{")
        elems = []
        while not self.at("}"):
            if self.at("{"):
                elems.append(self.array_init())
            else:
                elems.append(self.expr())
            if self.at(","):
                self.next()
        self.expect("}")
        return ("arrayinit", elems, line)

    def params(self):
        self.expect("(")
        ps = []
        while not self.at(")"):
            self.modifiers()
            ty = self.parse_type()
            name = self.ident()
            ps.append((ty, name))
            if self.at(","):
                self.next()
        self.expect(")")
        return ps

    def throws(self):
        ts = []
        if self.at("throws"):
            self.next()
            ts.append(self.qualified_name())
            while self.at(","):
                self.next()
                ts.append(self.qualified_name())
        return ts

    # ------------------------------------------------------------ statements
    def block(self):
        line = self.peek().line
        self.expect("{")
        stmts = []
        while not self.at("}"):
            stmts.append(self.statement())
        self.expect("}")
        return ("block", stmts, line)

    def statement(self):
        t = self.peek()
        line = t.line
        if self.at("{"):
            return self.block()
        if self.at(";"):
            self.next()
            return ("empty", line)
        if t.kind == "ident" and self.at(":", 1):
            label = self.next().text
            self.next()
            return ("labeled", label, self.statement(), line)
        if self.at("if"):
            self.next()
            self.expect("(")
            cond = self.expr()
            self.expect(")")
            then = self.statement()
            els = None
            if self.at("else"):
                self.next()
                els = self.statement()
            return ("if", cond, then, els, line)
        if self.at("while"):
            self.next()
            self.expect("(")
            cond = self.expr()
            self.expect(")")
            return ("while", cond, self.statement(), line)
        if self.at("do"):
            self.next()
            body = self.statement()
            self.expect("while")
            self.expect("(")
            cond = self.expr()
            self.expect(")")
            self.expect(";")
            return ("do", body, cond, line)
        if self.at("for"):
            self.next()
            self.expect("(")
            # for-each?
            save = self.p
            self.modifiers()
            ty = self.try_type_then_ident()
            if ty is not None and self.at(":", 1):
                name = self.ident()
                self.expect(":")
                iterable = self.expr()
                self.expect(")")
                return ("foreach", ty, name, iterable, self.statement(), line)
            self.p = save
            init = []
            if not self.at(";"):
                self.modifiers()
                ty = self.try_type_then_ident()
                if ty is not None:
                    init.append(self.local_decl_rest(ty, line))
                else:
                    init.append(("expr", self.expr(), line))
                    while self.at(","):
                        self.next()
                        init.append(("expr", self.expr(), line))
            self.expect(";")
            cond = None if self.at(";") else self.expr()
            self.expect(";")
            update = []
            if not self.at(")"):
                update.append(self.expr())
                while self.at(","):
                    self.next()
                    update.append(self.expr())
            self.expect(")")
            return ("for", init, cond, update, self.statement(), line)
        if self.at("switch"):
            self.next()
            self.expect("(")
            sel = self.expr()
            self.expect(")")
            self.expect("{")
            groups = []  # (labels, stmts); labels: list of exprs or "default"
            while not self.at("}"):
                labels = []
                while self.at("case") or self.at("default"):
                    if self.at("default"):
                        self.next()
                        self.expect(":")
                        labels.append("default")
                    else:
                        self.next()
                        labels.append(self.expr())
                        self.expect(":")
                stmts = []
                while not (self.at("case") or self.at("default") or self.at("}")):
                    stmts.append(self.statement())
                groups.append((labels, stmts))
            self.expect("}")
            return ("switch", sel, groups, line)
        if self.at("break"):
            self.next()
            label = None
            if self.peek().kind == "ident":
                label = self.next().text
            self.expect(";")
            return ("break", label, line)
        if self.at("continue"):
            self.next()
            label = None
            if self.peek().kind == "ident":
                label = self.next().text
            self.expect(";")
            return ("continue", label, line)
        if self.at("return"):
            self.next()
            e = None if self.at(";") else self.expr()
            self.expect(";")
            return ("return", e, line)
        if self.at("throw"):
            self.next()
            e = self.expr()
            self.expect(";")
            return ("throw", e, line)
        if self.at("try"):
            self.next()
            body = self.block()
            catches = []
            fin = None
            while self.at("catch"):
                self.next()
                self.expect("(")
                self.modifiers()
                types = [self.qualified_name()]
                while self.at("|"):
                    self.next()
                    types.append(self.qualified_name())
                name = self.ident()
                self.expect(")")
                catches.append((types, name, self.block()))
            if self.at("finally"):
                self.next()
                fin = self.block()
            return ("try", body, catches, fin, line)
        self.modifiers()
        ty = self.try_type_then_ident()
        if ty is not None:
            d = self.local_decl_rest(ty, line)
            self.expect(";")
            return d
        e = self.expr()
        self.expect(";")
        return ("expr", e, line)

    def local_decl_rest(self, ty, line):
        decls = [(self.ident(), None)]
        if self.at("="):
            self.next()
            decls[0] = (decls[0][0], self.array_init() if self.at("{") else self.expr())
        while self.at(","):
            self.next()
            n = self.ident()
            init = None
            if self.at("="):
                self.next()
                init = self.array_init() if self.at("{") else self.expr()
            decls.append((n, init))
        return ("local", ty, decls, line)

    # ----------------------------------------------------------- expressions
    BINARY = [
        ["||"], ["&&"], ["|"], ["^"], ["&"], ["==", "!="], ["<", ">", "<=", ">=", "instanceof"],
        ["<<", ">>", ">>>"], ["+", "-"], ["*", "/", "%"],
    ]
    ASSIGN = {"=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", ">>>="}

    def expr(self):
        lhs = self.ternary()
        t = self.peek()
        if t.kind == "op" and t.text in self.ASSIGN:
            self.next()
            rhs = self.expr()
            return ("assign", t.text, lhs, rhs, t.line)
        return lhs

    def ternary(self):
        c = self.binary(0)
        if self.at("?"):
            line = self.next().line
            a = self.expr()
            self.expect(":")
            b = self.ternary_or_assign()
            return ("cond", c, a, b, line)
        return c

    def ternary_or_assign(self):
        return self.expr()

    def binary(self, level):
        if level == len(self.BINARY):
            return self.unary()
        lhs = self.binary(level + 1)
        while True:
            t = self.peek()
            if (t.kind == "op" or t.text == "instanceof") and t.text in self.BINARY[level]:
                self.next()
                if t.text == "instanceof":
                    ty = self.parse_type()
                    lhs = ("instanceof", lhs, ty, t.line)
                    continue
                rhs = self.binary(level + 1)
                lhs = ("bin", t.text, lhs, rhs, t.line)
            else:
                return lhs

    def unary(self):
        t = self.peek()
        if t.kind == "op" and t.text in ("!", "~", "-", "+"):
            self.next()
            return ("unary", t.text, self.unary(), t.line)
        if t.kind == "op" and t.text in ("++", "--"):
            self.next()
            return ("preinc", t.text, self.unary(), t.line)
        if self.at("("):
            # cast?
            save = self.p
            self.next()
            nt = self.peek()
            if nt.kind == "keyword" and nt.text in ("int", "long", "char", "byte", "short", "float", "double", "boolean"):
                ty = self.parse_type()
                if self.at(")"):
                    self.next()
                    return ("cast", ty, self.unary(), t.line)
            elif nt.kind == "ident":
                try:
                    ty = self.parse_type()
                    if self.at(")") and (self.peek(1).kind in ("ident", "string") or self.at("(", 1) or self.at("this", 1) or self.at("new", 1)) and ty.name[0].isupper():
                        self.next()
                        return ("cast", ty, self.unary(), t.line)
                except TranslationError:
                    pass
            self.p = save
        return self.postfix(self.primary())

    def postfix(self, e):
        while True:
            t = self.peek()
            if self.at("."):
                self.next()
                if self.at("<"):
                    self.err("explicit type arguments")
                name = self.ident()
                if self.at("("):
                    e = ("call", e, name, self.args(), t.line)
                else:
                    e = ("field", e, name, t.line)
            elif self.at("["):
                self.next()
                idx = self.expr()
                self.expect("]")
                e = ("index", e, idx, t.line)
            elif t.kind == "op" and t.text in ("++", "--"):
                self.next()
                e = ("postinc", t.text, e, t.line)
            else:
                return e

    def args(self):
        self.expect("(")
        a = []
        while not self.at(")"):
            a.append(self.expr())
            if self.at(","):
                self.next()
        self.expect(")")
        return a

    def primary(self):
        t = self.next()
        line = t.line
        if t.kind in ("int", "long", "char", "string", "float", "double"):
            return ("lit", t.kind, t.value, line)
        if t.kind == "keyword":
            if t.text in ("true", "false"):
                return ("lit", "boolean", t.text == "true", line)
            if t.text == "null":
                return ("lit", "null", None, line)
            if t.text == "this":
                if self.at("("):
                    return ("thiscall", self.args(), line)
                return ("this", line)
            if t.text == "super":
                if self.at("("):
                    return ("supercall", self.args(), line)
                self.expect(".")
                name = self.ident()
                return ("supermethod", name, self.args(), line)
            if t.text == "new":
                ty = self.parse_type_no_dims()
                if self.at("["):
                    if self.at("]", 1):
                        dims = 0
                        while self.at("["):
                            self.next()
                            self.expect("]")
                            dims += 1
                        init = self.array_init()
                        return ("newarrayinit", JType(ty.name, ty.args, dims), init, line)
                    sizes = []
                    dims = 0
                    while self.at("["):
                        self.next()
                        if self.at("]"):
                            self.next()
                            dims += 1
                            continue
                        sizes.append(self.expr())
                        self.expect("]")
                        dims += 1
                    return ("newarray", JType(ty.name, ty.args, dims), sizes, line)
                args = self.args()
                if self.at("{"):
                    self.err("anonymous classes")
                return ("new", ty, args, line)
            if t.text in ("int", "long", "char", "boolean"):
                self.err(f"unexpected {t.text}", t)
        if t.kind == "op" and t.text == "(":
            e = self.expr()
            self.expect(")")
            return ("paren", e, line)
        if t.kind == "ident":
            if self.at("("):
                return ("call", None, t.text, self.args(), line)
            return ("name", t.text, line)
        self.err(f"unexpected token {t.text!r}", t)

    def parse_type_no_dims(self):
        t = self.peek()
        if t.kind == "keyword" and t.text in ("int", "long", "boolean", "char", "float", "double", "short", "byte"):
            self.next()
            return JType(t.text)
        name = self.qualified_name()
        name = name.split(".")[-1] if name.startswith("java.") else name
        args = ()
        if self.at("<"):
            self.next()
            lst = []
            if not self.at(">"):
                lst.append(self.parse_type())
                while self.at(","):
                    self.next()
                    lst.append(self.parse_type())
            self.expect(">")
            args = tuple(lst)
        return JType(name, args)


# ==========================================================================
# Profiles
# ==========================================================================


@dataclass
class Ext:
    """A member the generated code calls that the generated class does not
    define: how to call it in Rust."""

    rust: str  # a template with {this} and {0}, {1}, ... for the arguments
    params: list  # the parameter kinds, see `coerce_arg`
    ret: JType  # the Java return type
    ret_nullable: bool = False
    result: bool = False  # whether the Rust call returns a Result


@dataclass
class Profile:
    name: str
    java_pkg: str  # path under JAVA_ROOT
    parser_class: str
    rust_dir: str  # under CRATE
    java_package: str
    # type map for the classes the actions use: Java name -> (Rust type,
    # nullable)
    types: dict
    externals: dict  # Java name -> Ext
    ext_fields: dict  # Java field name -> (Rust read expression, JType, nullable)
    user_members: list  # the names of the hand-translated user members
    user_sha: str  # SHA-256 of the user members' tokens
    nullable_params: dict  # (method, param) -> True
    ops_trait: str
    base_ops: str
    tm_fallible: set = field(default_factory=set)
    # The classes the actions construct: "Class/arity" -> Ext ({0}, {1}, ...
    # the arguments). A JavaCC class listed here (`ParseException/1`)
    # replaces the JavaCC 7 rule for that constructor.
    news: dict = field(default_factory=dict)
    # The static methods the actions call on a class: "Class.method" -> Ext.
    static_calls: dict = field(default_factory=dict)
    # The methods the actions call on a value of a mapped type:
    # "Type.method" -> Ext, `{r}` being the receiver (a place, not cloned).
    instance_calls: dict = field(default_factory=dict)
    # The static fields the actions read by their qualified name, as Java
    # wrote it: "A.B.C" -> (Rust read expression, JType, nullable).
    static_fields: dict = field(default_factory=dict)
    # Reference casts `(Class) x`: Java class -> template over {0} (the
    # value), giving a value of the type `Class` maps to.
    casts: dict = field(default_factory=dict)
    # `x instanceof Class`: Java class -> template over {0} (a place).
    instanceofs: dict = field(default_factory=dict)
    # Widening reference conversions between mapped types whose Rust types
    # differ: (from, to) -> template over {0}.
    upcasts: dict = field(default_factory=dict)
    # Enum types the actions compare with `==` and `!=`.
    enums: set = field(default_factory=set)
    # The template of `ParseException.java`: "javacc" (the JavaCC 7 one) or
    # "flexible" (the flexible parser's subclass of
    # `QueryNodeParseException`, which reports an NLS message).
    parse_exception: str = "javacc"

    @property
    def tm_class(self) -> str:
        """The generated token manager class."""
        return f"{self.parser_class}TokenManager"


CLASSIC_EXTERNALS = {
    "addClause": Ext("{this}.add_clause(&mut {0}, {1}, {2}, {3})?", ["mutref", "val", "val", "val"], T_VOID, result=True),
    "addMultiTermClauses": Ext("{this}.add_multi_term_clauses(&mut {0}, {1})", ["mutref", "val"], T_VOID),
    "getBooleanQuery": Ext("{this}.get_boolean_query(&{0})?", ["ref"], JType("Query"), True, result=True),
    "discardEscapeChar": Ext("{this}.discard_escape_char(&{0})?", ["str"], T_STRING, result=True),
    "handleBoost": Ext("{this}.handle_boost({0}, {1}.as_ref())?", ["val", "val"], JType("Query"), True, result=True),
    "handleBareTokenQuery": Ext("{this}.handle_bare_token_query({0}.as_deref(), &{1}, {2}.as_ref(), {3}, {4}, {5}, {6})?", ["val", "token", "val", "val", "val", "val", "val"], JType("Query"), True, result=True),
    "getRangeQuery": Ext("{this}.get_range_query({0}.as_deref(), {1}.as_deref(), {2}.as_deref(), {3}, {4})?", ["val", "nstr", "nstr", "val", "val"], JType("Query"), True, result=True),
    "handleQuotedTerm": Ext("{this}.handle_quoted_term({0}.as_deref(), &{1}, {2}.as_ref())?", ["val", "token", "val"], JType("Query"), True, result=True),
    "getFieldQuery": Ext("{this}.get_field_query({0}.as_deref(), &{1}, {2})?", ["val", "str", "val"], JType("Query"), True, result=True),
    "allowedPostMultiTerm": Ext("allowed_post_multi_term({0})", ["val"], T_BOOL),
}

CLASSIC_EXT_FIELDS = {
    "splitOnWhitespace": ("{this}.query_parser().split_on_whitespace", T_BOOL, False),
    "CONJ_NONE": ("super::query_parser_base::CONJ_NONE", T_INT, False),
    "CONJ_AND": ("super::query_parser_base::CONJ_AND", T_INT, False),
    "CONJ_OR": ("super::query_parser_base::CONJ_OR", T_INT, False),
    "MOD_NONE": ("super::query_parser_base::MOD_NONE", T_INT, False),
    "MOD_NOT": ("super::query_parser_base::MOD_NOT", T_INT, False),
    "MOD_REQ": ("super::query_parser_base::MOD_REQ", T_INT, False),
}

SURROUND_EXTERNALS = {
    "getFieldsQuery": Ext("{this}.get_fields_query({0}, {1})", ["val", "nn"], JType("SrndQuery")),
    "getOrQuery": Ext("{this}.get_or_query({0}, {1}, &{2})?", ["nn", "val", "token"], JType("SrndQuery"), result=True),
    "getAndQuery": Ext("{this}.get_and_query({0}, {1}, &{2})?", ["nn", "val", "token"], JType("SrndQuery"), result=True),
    "getNotQuery": Ext("{this}.get_not_query({0}, &{1})?", ["nn", "token"], JType("SrndQuery"), result=True),
    "getDistanceQuery": Ext("{this}.get_distance_query({0}, {1}, &{2}, {3})?", ["nn", "val", "token", "val"], JType("SrndQuery"), result=True),
    "getTermQuery": Ext("{this}.get_term_query(&{0}, {1})", ["str", "val"], JType("SrndQuery")),
    "getPrefixQuery": Ext("{this}.get_prefix_query(&{0}, {1})?", ["str", "val"], JType("SrndQuery"), result=True),
    "getTruncQuery": Ext("{this}.get_trunc_query(&{0})?", ["str"], JType("SrndQuery"), result=True),
    "allowedSuffix": Ext("{this}.allowed_suffix(&{0})", ["str"], T_BOOL),
    "allowedTruncation": Ext("{this}.allowed_truncation(&{0})", ["str"], T_BOOL),
}

SURROUND_EXT_FIELDS = {
    "MINIMUM_PREFIX_LENGTH": ("MINIMUM_PREFIX_LENGTH", T_INT, False),
    "TRUNCATION_ERROR_MESSAGE": ("TRUNCATION_ERROR_MESSAGE.to_string()", T_STRING, False),
    "BOOST_ERROR_MESSAGE": ("BOOST_ERROR_MESSAGE.to_string()", T_STRING, False),
}

T_QUERY_NODE = JType("QueryNode")
T_CHAR_SEQUENCE = JType("CharSequence")
T_INTERVAL_FUNCTION = JType("IntervalFunction")
T_MODIFIER = JType("ModifierQueryNode.Modifier")
T_MESSAGE_IMPL = JType("MessageImpl")

# The flexible standard parser: the node and interval function classes its
# actions construct, each a `QueryNode` (an `Option<QueryNodeRef>`) or an
# `IntervalFunction` (an `Option<Arc<dyn IntervalFunction>>`) reference. The
# helpers `query_nodes`, `interval_functions`, `is_instance`, `cast_node`,
# `modifier_of` and the associated `parse_int` and `parse_float` are the
# hand-written part of `standard_syntax_parser.rs`.
_NODE = "Some({0} as QueryNodeRef)"
_FN = "Some(Arc::new({0}) as Arc<dyn IntervalFunction>)"
_CS = (T_CHAR_SEQUENCE, True)
_QN = (T_QUERY_NODE, True)
_IF = (T_INTERVAL_FUNCTION, True)


def _node(rust, params, result=True):
    return Ext(_NODE.replace("{0}", rust), params, T_QUERY_NODE, True, result=result)


def _fn(rust, params, result=True):
    return Ext(_FN.replace("{0}", rust), params, T_INTERVAL_FUNCTION, True, result=result)


STANDARD_EXTERNALS = {
    "discardEscapeChar": Ext("Some(EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from({0}))?.into_char_sequence())",
                             ["str"], T_CHAR_SEQUENCE, True, result=True),
    "parseInt": Ext("StandardSyntaxParser::parse_int(&{0})?", ["token"], T_INT, result=True),
    "parseFloat": Ext("StandardSyntaxParser::parse_float(&{0})?", ["token"], T_FLOAT, result=True),
}

STANDARD_NEWS = {
    "BooleanQueryNode/1": _node("BooleanQueryNode::new(Some(query_nodes({0})?))?", ["val"]),
    "OrQueryNode/1": _node("OrQueryNode::new(Some(query_nodes({0})?))?", ["val"]),
    "AndQueryNode/1": _node("AndQueryNode::new(Some(query_nodes({0})?))?", ["val"]),
    "ModifierQueryNode/2": _node("ModifierQueryNode::new({0}, {1})?", [_QN, (T_MODIFIER, False)]),
    "GroupQueryNode/1": Ext("Some(GroupQueryNode::new({0})?)", [_QN], JType("GroupQueryNode"), True, result=True),
    "MinShouldMatchNode/2": _node("MinShouldMatchNode::new({0}, {1})?", [(T_INT, False), (JType("GroupQueryNode"), True)]),
    "IntervalQueryNode/2": _node("IntervalQueryNode::new({0}, {1})?", [(T_STRING, True), _IF]),
    "BoostQueryNode/2": _node("BoostQueryNode::new({0}, {1})?", [_QN, (T_FLOAT, False)]),
    "FuzzyQueryNode/5": _node("FuzzyQueryNode::new({0}, {1}, {2}, {3}, {4})",
                              [_CS, _CS, (T_FLOAT, False), (T_INT, False), (T_INT, False)], result=False),
    "FieldQueryNode/4": _node("FieldQueryNode::new({0}, {1}, {2}, {3})", [_CS, _CS, (T_INT, False), (T_INT, False)],
                              result=False),
    "QuotedFieldQueryNode/4": _node("QuotedFieldQueryNode::new({0}, {1}, {2}, {3})",
                                    [_CS, _CS, (T_INT, False), (T_INT, False)], result=False),
    "RegexpQueryNode/4": _node("RegexpQueryNode::new({0}, &CharSequence::from({1}), ({2}) as usize, ({3}) as usize)?",
                               [_CS, "str", (T_INT, False), (T_INT, False)]),
    "TermRangeQueryNode/4": _node("TermRangeQueryNode::new({0}, {1}, {2}, {3})?",
                                  [_QN, _QN, (T_BOOL, False), (T_BOOL, False)]),
    "SlopQueryNode/2": _node("SlopQueryNode::new({0}, {1})?", [_QN, (T_INT, False)]),
    "AtLeast/2": _fn("AtLeast::new({0}, Some(interval_functions({1})?))?", [(T_INT, False), "val"]),
    "MaxWidth/2": _fn("MaxWidth::new({0}, {1})?", [(T_INT, False), _IF]),
    "MaxGaps/2": _fn("MaxGaps::new({0}, {1})?", [(T_INT, False), _IF]),
    "Unordered/1": _fn("Unordered::new(Some(interval_functions({0})?))?", ["val"]),
    "UnorderedNoOverlaps/2": _fn("UnorderedNoOverlaps::new({0}, {1})?", [_IF, _IF]),
    "Ordered/1": _fn("Ordered::new(Some(interval_functions({0})?))?", ["val"]),
    "Or/1": _fn("Or::new(Some(interval_functions({0})?))?", ["val"]),
    "Phrase/1": _fn("Phrase::new(Some(interval_functions({0})?))?", ["val"]),
    "Before/2": _fn("Before::new({0}, {1})?", [_IF, _IF]),
    "After/2": _fn("After::new({0}, {1})?", [_IF, _IF]),
    "Containing/2": _fn("Containing::new({0}, {1})?", [_IF, _IF]),
    "NotContaining/2": _fn("NotContaining::new({0}, {1})?", [_IF, _IF]),
    "ContainedBy/2": _fn("ContainedBy::new({0}, {1})?", [_IF, _IF]),
    "NotContainedBy/2": _fn("NotContainedBy::new({0}, {1})?", [_IF, _IF]),
    "Within/3": _fn("Within::new({0}, {1}, {2})?", [_IF, (T_INT, False), _IF]),
    "NotWithin/3": _fn("NotWithin::new({0}, {1}, {2})?", [_IF, (T_INT, False), _IF]),
    "Extend/3": _fn("Extend::new({0}, {1}, {2})?", [_IF, (T_INT, False), (T_INT, False)]),
    "Overlapping/2": _fn("Overlapping::new({0}, {1})?", [_IF, _IF]),
    "NonOverlapping/2": _fn("NonOverlapping::new({0}, {1})?", [_IF, _IF]),
    "Wildcard/2": _fn("Wildcard::new({0}, {1})", ["str", (T_INT, False)], result=False),
    "FuzzyTerm/3": _fn("FuzzyTerm::new({0}, {1}, {2})", ["str", "val", "val"], result=False),
    "AnalyzedText/1": _fn("AnalyzedText::new({0})", ["str"], result=False),
    "MessageImpl/1": Ext("MessageImpl::new({0})", [(T_STRING, True)], T_MESSAGE_IMPL),
    "ParseException/1": Ext("ParseException::with_message(Arc::new({0}))", [(T_MESSAGE_IMPL, False)],
                            JType("ParseException")),
}

STANDARD_STATIC_FIELDS = {
    "ModifierQueryNode.Modifier.MOD_NONE": ("Modifier::ModNone", T_MODIFIER, False),
    "ModifierQueryNode.Modifier.MOD_NOT": ("Modifier::ModNot", T_MODIFIER, False),
    "ModifierQueryNode.Modifier.MOD_REQ": ("Modifier::ModReq", T_MODIFIER, False),
    "org.apache.lucene.search.FuzzyQuery.defaultMaxEdits": ("fuzzy_query::DEFAULT_MAX_EDITS", T_INT, False),
    "QueryParserMessages.INVALID_SYNTAX_FUZZY_LIMITS": ("QueryParserMessages::INVALID_SYNTAX_FUZZY_LIMITS.get()", T_STRING, True),
    "QueryParserMessages.INVALID_SYNTAX_FUZZY_EDITS": ("QueryParserMessages::INVALID_SYNTAX_FUZZY_EDITS.get()", T_STRING, True),
}

PROFILES = [
    Profile(
        name="classic",
        java_pkg="classic",
        parser_class="QueryParser",
        rust_dir="classic",
        java_package="org.apache.lucene.queryparser.classic",
        types={
            "Query": ("Option<Arc<dyn Query>>", True),
            "BooleanClause": ("BooleanClause", False),
        },
        externals=CLASSIC_EXTERNALS,
        ext_fields=CLASSIC_EXT_FIELDS,
        user_members=["Operator", "DEFAULT_SPLIT_ON_WHITESPACE", "QueryParser(String,Analyzer)",
                      "setAutoGeneratePhraseQueries", "getSplitOnWhitespace", "setSplitOnWhitespace",
                      "splitOnWhitespace", "disallowedPostMultiTerm", "allowedPostMultiTerm"],
        user_sha="",
        nullable_params={("Query", "field"): True, ("Clause", "field"): True, ("Term", "field"): True,
                         ("MultiTerm", "field"): True, ("TopLevelQuery", "field"): True},
        ops_trait="QueryParserOps",
        base_ops="QueryParserBaseOps",
    ),
    Profile(
        name="surround",
        java_pkg="surround/parser",
        parser_class="QueryParser",
        rust_dir="surround/parser",
        java_package="org.apache.lucene.queryparser.surround.parser",
        types={
            "SrndQuery": ("Arc<dyn SrndQuery>", False),
        },
        externals=SURROUND_EXTERNALS,
        ext_fields=SURROUND_EXT_FIELDS,
        user_members=["MINIMUM_PREFIX_LENGTH", "MINIMUM_CHARS_IN_TRUNC", "TRUNCATION_ERROR_MESSAGE",
                      "BOOST_ERROR_MESSAGE", "TRUNCATOR", "ANY_CHAR", "FIELD_OPERATOR", "parse",
                      "QueryParser()", "parse2", "getFieldsQuery", "getOrQuery", "getAndQuery",
                      "getNotQuery", "getOpDistance", "checkDistanceSubQueries", "getDistanceQuery",
                      "getTermQuery", "allowedSuffix", "getPrefixQuery", "allowedTruncation",
                      "getTruncQuery"],
        user_sha="",
        nullable_params={},
        ops_trait="QueryParserOps",
        base_ops="",
    ),
    Profile(
        name="standard",
        java_pkg="flexible/standard/parser",
        parser_class="StandardSyntaxParser",
        rust_dir="flexible/standard/parser",
        java_package="org.apache.lucene.queryparser.flexible.standard.parser",
        types={
            "QueryNode": ("Option<QueryNodeRef>", True),
            "FieldQueryNode": ("Option<QueryNodeRef>", True),
            "TermRangeQueryNode": ("Option<QueryNodeRef>", True),
            "IntervalQueryNode": ("Option<QueryNodeRef>", True),
            "ModifierQueryNode": ("Option<QueryNodeRef>", True),
            "GroupQueryNode": ("Option<Arc<GroupQueryNode>>", True),
            "IntervalFunction": ("Option<Arc<dyn IntervalFunction>>", True),
            "CharSequence": ("Option<CharSequence>", True),
            "ModifierQueryNode.Modifier": ("Modifier", False),
            "MessageImpl": ("MessageImpl", False),
        },
        externals=STANDARD_EXTERNALS,
        ext_fields={},
        user_members=["StandardSyntaxParser()", "parse", "parseFloat", "parseInt"],
        user_sha="",
        nullable_params={},
        ops_trait="StandardSyntaxParserOps",
        base_ops="",
        news=STANDARD_NEWS,
        static_calls={
            "Collections.singletonList": Ext("vec![{0}]", [_QN], JType("List", (T_QUERY_NODE,))),
        },
        instance_calls={
            "CharSequence.toString": Ext("crate::javacc::nn({r}.as_ref())?.to_string()", [], T_STRING, result=True),
            "ModifierQueryNode.getModifier": Ext("modifier_of(&{r})?", [], T_MODIFIER, result=True),
        },
        static_fields=STANDARD_STATIC_FIELDS,
        casts={"ModifierQueryNode": "cast_node({0}, &MODIFIER_QUERY_NODE_CLASS)?"},
        instanceofs={"ModifierQueryNode": "is_instance(&{0}, &MODIFIER_QUERY_NODE_CLASS)"},
        upcasts={("GroupQueryNode", "QueryNode"): "{0}.map(|n| n as QueryNodeRef)"},
        enums={"ModifierQueryNode.Modifier"},
        parse_exception="flexible",
    ),
]

# SHA-256 of the Java tokens of the JavaCC 7 templates (the package statement
# removed) and of the user members of each parser, to pin the hand-written
# Rust they correspond to.
TEMPLATE_SHA = {
    "Token": "3cd61ae22718671ea69c5f1cb9f0b5eb91f4dbae31889b584018a0eb3306b671",
    "TokenMgrError": "7ae2f5c6bf062dd0fb14f849066508d55b2eccdf5317080983263c3458c22f0c",
    "ParseException": "d20c4dd0eb22bd3b4635a43afd18f2ff8e59bcbce09fd9d3692113f7e7e57fc2",
    "ParseException:flexible": "e5ff39d6a6eb60653cf4eb6b193585d7f3ced3ab6a201701722b19862c61ca08",
}
USER_SHA = {
    "classic": "dcd91f0c8f87c0627994e55a08b97bfffa7510b7977f5fa7a917d6a0d1999cc6",
    "surround": "01ada236a4427be580a15bb761613658a0632bd395b451e34593885c492cf5d4",
    "standard": "3e4fd443c53c5bbec151ce11501f3e9a02ecf02964d11e6dfa9068949d50bc4b",
}
PRINT_HASHES = False


def snake(name: str) -> str:
    """Java camelCase (and JavaCC's) to Rust snake_case."""
    special = {
        "ReInit": "re_init", "ReInitRounds": "re_init_rounds", "SwitchTo": "switch_to",
        "BeginToken": "begin_token", "GetImage": "get_image", "GetSuffix": "get_suffix",
        "Done": "done", "SkipLexicalActions": "skip_lexical_actions",
        "MoreLexicalActions": "more_lexical_actions", "TokenLexicalActions": "token_lexical_actions",
        "TopLevelQuery": "top_level_query", "jj_semLA": "jj_sem_la", "jj_lookingAhead": "jj_looking_ahead",
        "type": "r#type", "ref": "r#ref", "match": "r#match", "loop": "r#loop", "fn": "r#fn",
        "mod": "r#mod", "use": "r#use", "crate": "r#crate", "self": "this_", "move": "r#move",
        "in": "r#in", "impl": "r#impl", "trait": "r#trait", "struct": "r#struct", "where": "r#where",
    }
    if name in special:
        return special[name]
    s = re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", name)
    s = re.sub(r"(?<=[A-Z])([A-Z][a-z])", r"_\1", s)
    return s.lower()


def upper_snake(name: str) -> str:
    if name.isupper() or (name.replace("_", "").isupper()):
        return name
    return snake(name).upper()


# ==========================================================================
# Emitter
# ==========================================================================


class Scope:
    def __init__(self, parent=None):
        self.vars = {}
        self.parent = parent

    def get(self, name):
        s = self
        while s:
            if name in s.vars:
                return s.vars[name]
            s = s.parent
        return None

    def put(self, name, info):
        self.vars[name] = info

    def has_rust_name(self, rust):
        s = self
        while s:
            if any(v.rust == rust for v in s.vars.values()):
                return True
            s = s.parent
        return False


@dataclass
class Var:
    rust: str
    ty: JType
    nullable: bool
    mutated: bool = True
    byref: bool = False  # a `&mut` parameter (a Java List the callee mutates)


class Emitter:
    """Translates the members of one generated class."""

    def __init__(self, where, profile: Profile, cls, mode):
        self.where = where
        self.profile = profile
        self.cls = cls  # the class declaration
        self.mode = mode  # "tm" (token manager) or "parser"
        self.fields = {}  # java name -> (rust name, JType, nullable, static, rust type)
        self.methods = {}  # java name -> list of members
        self.static_arrays = {}
        self.nested = {}
        self.label_counter = 0
        self.tmp_counter = 0
        self.uses_raised = False
        self.fn_kind = None  # "result" or "raised" or "plain"
        self.current_method = None
        self.loop_stack = []  # (java label or None, kind, rust label for continue)
        self.switch_stack = []
        for m in cls["members"]:
            if m["kind"] == "field":
                for (n, init) in m["decls"]:
                    self.fields[n] = (m, init)
            elif m["kind"] == "method":
                self.methods.setdefault(m["name"], []).append(m)
            elif m["kind"] == "type":
                self.nested[m["decl"]["name"]] = m["decl"]

    def err(self, line, msg):
        fail(f"{self.where}:{line}", msg)

    def tmp(self, base="t"):
        self.tmp_counter += 1
        return f"__{base}{self.tmp_counter}"

    # --------------------------------------------------------------- types
    def rust_type(self, ty: JType, nullable: bool, line=0) -> str:
        if ty.dims == 0:
            if ty.name == "int":
                return "i32"
            if ty.name == "long":
                return "i64"
            if ty.name == "boolean":
                return "bool"
            if ty.name == "char":
                return "u16"
            if ty.name == "float":
                return "f32"
            if ty.name == "double":
                return "f64"
            if ty.name == "void":
                return "()"
            if ty.name == "String":
                return "Option<String>" if nullable else "String"
            if ty.name == "Token":
                return "Option<TokenRef>"
            if ty.name == "JJCalls":
                return "Option<JJCallsRef>"
            if ty.name == "CharStream":
                return "Box<dyn CharStream>"
            if ty.name == "StringBuilder":
                return "String"
            if ty.name in (self.profile.tm_class, "ParseException"):
                return ty.name
            if ty.name in ("List", "ArrayList"):
                inner = self.rust_type(ty.args[0], False, line)
                return f"Option<Vec<{inner}>>" if nullable else f"Vec<{inner}>"
            if ty.name in self.profile.types:
                rt, nl = self.profile.types[ty.name]
                return rt
            self.err(line, f"no Rust type for {ty}")
        inner = self.rust_type(ty.elem(), False, line) if not (ty.dims == 1 and ty.name in ("Token", "JJCalls")) else self.rust_type(ty.elem(), True, line)
        if ty.dims == 1 and ty.name == "String":
            inner = "Option<String>"
        return f"Option<Vec<{inner}>>" if nullable else f"Vec<{inner}>"

    def type_nullable(self, ty: JType) -> bool:
        """Whether values of `ty` are `Option` in Rust whatever the context."""
        if ty.dims == 0 and ty.name in ("Token", "JJCalls"):
            return True
        if ty.dims == 0 and ty.name in self.profile.types:
            return self.profile.types[ty.name][1]
        return False

    def default_value(self, ty: JType, nullable: bool, line) -> str:
        if ty.dims == 0:
            if ty.name in ("int", "long", "char"):
                return "0"
            if ty.name in ("float", "double"):
                return "0.0"
            if ty.name == "boolean":
                return "false"
        if nullable or self.type_nullable(ty):
            return "None"
        if ty.name == "StringBuilder":
            return "String::new()"
        self.err(line, f"no default for non-null {ty}")

    # ------------------------------------------------------------- statics
    def literal_array(self, ty: JType, init, line):
        """A static array initializer as Rust."""
        elems = init[1]
        et = ty.elem()
        out = []
        for e in elems:
            if isinstance(e, tuple) and e[0] == "arrayinit":
                self.err(line, "nested static arrays")
            out.append(self.const_expr(e, et, line))
        return out

    def const_expr(self, e, ty: JType, line):
        if e[0] == "lit":
            kind, v = e[1], e[2]
            if kind == "null":
                return "None"
            if kind == "string":
                s = rust_str(v)
                return f"Some({s})" if ty.name == "String" and self.static_string_nullable else s
            if kind == "int":
                return int_lit(v, ty)
            if kind == "long":
                return int_lit(v, T_LONG)
        if e[0] == "unary" and e[1] == "-" and e[2][0] == "lit":
            return "-" + self.const_expr(e[2], ty, line)
        self.err(line, f"unsupported constant expression {e[0]}")

    static_string_nullable = False

    # ===================================================== expression emitter
    # Each expression is emitted as (prelude statements, Rust expression,
    # JType, nullable). Side effects (assignments, ++/--) are hoisted into the
    # prelude in Java's evaluation order.

    def fn_err(self):
        return "Raised" if self.fn_kind == "raised" else "LuceneError"

    def q(self, s):
        """`?` with the conversion into the function's error type."""
        return s + "?"

    def this_ref(self, write=False):
        if self.mode == "tm":
            return "self"
        return "this.jj_mut()" if write else "this.jj()"

    def field_access(self, name, write, line):
        f = self.fields.get(name)
        if f is None:
            return None
        m, init = f
        rname = snake(name)
        if "static" in m["mods"]:
            return (upper_snake(name), m["type"], self.field_nullable(name), True)
        return (f"{self.this_ref(write)}.{rname}", m["type"], self.field_nullable(name), False)

    def field_nullable(self, name):
        m, init = self.fields[name]
        ty = m["type"]
        if self.type_nullable(ty):
            return True
        if name in ("jj_expentry",):
            return True
        if ty.name == "String" and ty.dims == 1 and "static" in m["mods"]:
            return False
        return False

    def emit_expr(self, e, scope, want=None):
        """-> (prelude: list[str], code: str, ty: JType, nullable: bool)"""
        try:
            return self.emit_expr_inner(e, scope, want)
        except TranslationError as ex:
            if str(ex).startswith(self.where):
                raise
            self.err(e[-1], str(ex))

    def emit_expr_inner(self, e, scope, want=None):
        k = e[0]
        line = e[-1]
        if k == "lit":
            kind, v = e[1], e[2]
            if kind == "int":
                return [], int_lit(v, T_INT), T_INT, False
            if kind == "long":
                return [], int_lit(v, T_LONG), T_LONG, False
            if kind == "char":
                return [], f"{v}u16", T_CHAR, False
            if kind == "boolean":
                return [], "true" if v else "false", T_BOOL, False
            if kind == "string":
                return [], f"{rust_str(v)}.to_string()", T_STRING, False
            if kind == "null":
                return [], "None", T_NULL, True
            if kind in ("double", "float"):
                lit = v if ("." in v or "e" in v.lower()) else v + ".0"
                return [], lit + ("f32" if kind == "float" else "f64"), T_FLOAT if kind == "float" else T_DOUBLE, False
        if k == "paren":
            p, c, t, n = self.emit_expr(e[1], scope, want)
            return p, f"({c})", t, n
        if k == "name":
            return self.emit_name(e[1], scope, line)
        if k == "this":
            self.err(line, "a bare `this`")
        if k == "field":
            return self.emit_field_read(e, scope)
        if k == "index":
            pa, ca, ta, na = self.emit_place(e[1], scope)
            pi, ci, ti, ni = self.emit_expr(e[2], scope)
            if ta.dims == 0:
                self.err(line, f"indexing a non-array {ta}")
            et = ta.elem()
            arr = ca
            if na:
                arr = f"crate::javacc::nn({ca}.as_ref())?"
            code = f"{arr}[({ci}) as usize]"
            static_str = et.name == "String" and et.dims == 0 and self.is_static(e[1])
            en = self.type_nullable(et) or (static_str and self.static_string_array(e[1]))
            if static_str:
                code += ".map(str::to_string)" if en else ".to_string()"
            elif not et.is_prim():
                code += ".clone()"
            if et.name == "String" and et.dims == 0 and not static_str:
                en = True
            return pa + pi, code, et, en
        if k == "call":
            return self.emit_call(e, scope)
        if k == "unary":
            p, c, t, n = self.emit_expr(e[2], scope)
            op = e[1]
            if op == "!":
                return p, f"!{wrap(c)}", T_BOOL, False
            if op == "-" and e[2][0] == "lit" and e[2][1] in ("int", "long"):
                return p, ("-" + c if not c.startswith("(") else f"{c}.wrapping_neg()"), promote(t), False
            if op == "-":
                t2 = promote(t)
                return p, f"{recv(cast_num(c, t, t2), t2)}.wrapping_neg()" if t2.name in ("int", "long") else f"-{wrap(c)}", t2, False
            if op == "~":
                t2 = promote(t)
                return p, f"!{wrap(cast_num(c, t, t2))}", t2, False
            return p, c, promote(t), False
        if k == "cast":
            p, c, t, n = self.emit_expr(e[2], scope)
            target = e[1]
            if target.is_prim():
                return p, cast_num(c, t, target), target, False
            if target.name in self.profile.casts and target.dims == 0:
                code = self.profile.casts[target.name].replace("{0}", c)
                return p, code, target, self.type_nullable(target)
            self.err(line, f"a reference cast to {target}")
        if k == "bin":
            return self.emit_binary(e, scope)
        if k == "cond":
            pc, cc, tc, nc = self.emit_expr(e[1], scope)
            pa, ca, ta, na = self.emit_expr(e[2], scope, want)
            pb, cb, tb, nb = self.emit_expr(e[3], scope, want)
            if pa or pb:
                self.err(line, "side effects in a conditional operand")
            ty = ta if ta != T_NULL else tb
            nullable = na or nb or ta == T_NULL or tb == T_NULL
            if nullable and not self.type_nullable(ty):
                ca = self.to_nullable(ca, ta, na)
                cb = self.to_nullable(cb, tb, nb)
            return pc, f"if {cc} {{ {ca} }} else {{ {cb} }}", ty, nullable
        if k in ("assign", "preinc", "postinc"):
            pre, code, ty, nullable = self.emit_side_effect(e, scope, as_value=True)
            return pre, code, ty, nullable
        if k == "new":
            return self.emit_new(e, scope)
        if k == "newarray":
            ty = e[1]
            if len(e[2]) == 1:
                ps, cs, ts, ns = self.emit_expr(e[2][0], scope)
                et = ty.elem()
                dv = self.default_value(et, self.type_nullable(et), line) if et.dims == 0 else "None"
                if et.dims > 0:
                    # every element is assigned before it is read
                    return ps, f"vec![Vec::new(); ({cs}) as usize]", ty, False
                return ps, f"vec![{dv}; ({cs}) as usize]", ty, False
            self.err(line, "multi-dimensional array creation")
        if k == "newarrayinit":
            ty = e[1]
            items = []
            pre = []
            for x in e[2][1]:
                p, c, t, n = self.emit_expr(x, scope)
                pre += p
                items.append(c)
            return pre, "vec![" + ", ".join(items) + "]", ty, False
        if k == "instanceof":
            target = e[2]
            if target.name in self.profile.instanceofs and target.dims == 0:
                p, c, t, n = self.emit_place(e[1], scope)
                return p, self.profile.instanceofs[target.name].replace("{0}", c), T_BOOL, False
            self.err(line, "instanceof")
        self.err(line, f"unsupported expression {k}")

    def is_static(self, e):
        if e[0] == "name" and e[1] in self.fields:
            return "static" in self.fields[e[1]][0]["mods"]
        return False

    def static_string_array(self, arr_expr):
        if arr_expr[0] == "name":
            f = self.fields.get(arr_expr[1])
            if f is not None:
                m, init = f
                if m["type"].name == "String" and m["type"].dims == 1:
                    return any(isinstance(x, tuple) and x[0] == "lit" and x[1] == "null" for x in (init[1] if init and init[0] == "arrayinit" else []))
        return False

    def to_nullable(self, code, ty, nullable):
        if nullable or ty == T_NULL:
            return code
        return f"Some({code})"

    def emit_name(self, name, scope, line):
        v = scope.get(name)
        if v is not None:
            code = v.rust
            if v.ty.name in ("CharStream", self.profile.tm_class, "Exception"):
                # passed on, never read twice
                return [], code, v.ty, v.nullable
            if not v.ty.is_prim():
                code = f"(*{code}).clone()" if v.byref else code + ".clone()"
            return [], code, v.ty, v.nullable
        fa = self.field_access(name, False, line)
        if fa is not None:
            code, ty, nullable, static = fa
            if not ty.is_prim():
                if static and ty.dims > 0:
                    code = code
                else:
                    code += ".clone()"
            return [], code, ty, nullable
        if name in self.profile.ext_fields:
            tmpl, ty, nl = self.profile.ext_fields[name]
            return [], tmpl.replace("{this}", "this"), ty, nl
        if name in self.constants:
            return [], f"qpc::{self.constants[name]}", T_INT, False
        if name == "tokenImage":
            return [], "qpc::TOKEN_IMAGE", JType("String", (), 1), False
        self.err(line, f"unknown name {name}")

    def emit_place(self, e, scope, write=False):
        """A variable or field as a place (no clone), else the value."""
        name = None
        if e[0] == "name":
            name = e[1]
        elif e[0] == "field" and e[1][0] == "this":
            name = e[2]
        if name is not None:
            v = scope.get(name)
            if v is not None:
                return [], (f"(*{v.rust})" if v.byref else v.rust), v.ty, v.nullable
            fa = self.field_access(name, write, e[-1])
            if fa is not None:
                code, ty, nullable, static = fa
                return [], code, ty, nullable
        return self.emit_expr(e, scope)

    def dotted(self, e, scope):
        """The qualified name a chain of field accesses on a name spells,
        when its first name is neither a variable nor a field."""
        parts = []
        while e[0] == "field":
            parts.append(e[2])
            e = e[1]
        if e[0] != "name" or scope.get(e[1]) is not None or e[1] in self.fields:
            return None
        parts.append(e[1])
        return ".".join(reversed(parts))

    def emit_field_read(self, e, scope):
        obj, name, line = e[1], e[2], e[3]
        if obj[0] == "this":
            return self.emit_name(name, scope, line)
        qualified = self.dotted(e, scope)
        if qualified is not None and qualified in self.profile.static_fields:
            code, ty, nl = self.profile.static_fields[qualified]
            return [], code, ty, nl
        if obj[0] == "name" and obj[1] == "TokenMgrError":
            return [], f"crate::{self.rust_mod()}::token_mgr_error::{name}", T_INT, False
        po, co, to, no = self.emit_place(obj, scope)
        if to.dims > 0 and name == "length":
            base = f"crate::javacc::nn({co}.as_ref())?" if no else co
            return po, f"({base}.len() as i32)", T_INT, False
        if to.name == "Token" and to.dims == 0:
            tok = f"crate::javacc::nn({co}.as_ref())?"
            ftypes = {"kind": T_INT, "beginLine": T_INT, "beginColumn": T_INT, "endLine": T_INT,
                      "endColumn": T_INT, "image": T_STRING, "next": JType("Token"), "specialToken": JType("Token")}
            if name not in ftypes:
                self.err(line, f"unknown Token field {name}")
            ft = ftypes[name]
            if name == "image":
                return po, f"{tok}.read(|o| o.image.clone())", T_STRING, True
            if ft.is_prim():
                return po, f"{tok}.read(|o| o.{snake(name)})", ft, False
            return po, f"{tok}.read(|o| o.{snake(name)}.clone())", ft, True
        if to.name == "JJCalls" and to.dims == 0:
            node = self.nested["JJCalls"]
            fields = {}
            for m in node["members"]:
                if m["kind"] == "field":
                    for (n, _) in m["decls"]:
                        fields[n] = m["type"]
            if name not in fields:
                self.err(line, f"unknown JJCalls field {name}")
            ft = fields[name]
            base = f"crate::javacc::nn({co}.as_ref())?"
            if ft.is_prim():
                return po, f"{base}.read(|o| o.{snake(name)})", ft, False
            return po, f"{base}.read(|o| o.{snake(name)}.clone())", ft, True
        self.err(line, f"field {name} of {to}")

    def rust_mod(self):
        return self.profile.rust_dir.replace("/", "::")

    constants = {}

    # ----------------------------------------------------------- binaries
    def emit_binary(self, e, scope):
        op, a, b, line = e[1], e[2], e[3], e[4]
        pa, ca, ta, na = self.emit_expr(a, scope)
        pb, cb, tb, nb = self.emit_expr(b, scope)
        if pb and op in ("&&", "||"):
            # a side effect in the right operand of a short-circuit operator
            self.err(line, "side effects in a short-circuit operand")
        pre = pa + pb
        if op in ("&&", "||"):
            return pre, f"{wrap(ca)} {op} {wrap(cb)}", T_BOOL, False
        if op in ("==", "!="):
            if ta == T_NULL or tb == T_NULL:
                other, on, ot = (cb, nb, tb) if ta == T_NULL else (ca, na, ta)
                if not on:
                    self.err(line, f"a null test of a value of the non-null type {ot}")
                return pre, f"{wrap(other)}.is_none()" if op == "==" else f"{wrap(other)}.is_some()", T_BOOL, False
            if ta.name in ("Token", "JJCalls") and ta.dims == 0:
                r = f"crate::javacc::opt_ptr_eq(&{ca}, &{cb})"
                return pre, r if op == "==" else f"!{r}", T_BOOL, False
            if ta.name == "String" or tb.name == "String":
                self.err(line, "reference comparison of strings")
            if ta == T_BOOL and tb == T_BOOL:
                return pre, f"{wrap(ca)} {op} {wrap(cb)}", T_BOOL, False
            if ta.name in self.profile.enums and ta == tb:
                return pre, f"{wrap(ca)} {op} {wrap(cb)}", T_BOOL, False
            t = binary_numeric(ta, tb)
            return pre, f"{wrap(cast_num(ca, ta, t))} {op} {wrap(cast_num(cb, tb, t))}", T_BOOL, False
        if op in ("<", ">", "<=", ">="):
            t = binary_numeric(ta, tb)
            return pre, f"{wrap(cast_num(ca, ta, t))} {op} {wrap(cast_num(cb, tb, t))}", T_BOOL, False
        if op == "+" and (ta.name == "String" or tb.name == "String") and ta.dims == 0 and tb.dims == 0:
            return pre, f"crate::javacc::concat(&[{self.as_string(ca, ta, na)}, {self.as_string(cb, tb, nb)}])", T_STRING, False
        if op in ("<<", ">>", ">>>"):
            t = promote(ta)
            lhs = cast_num(ca, ta, t)
            rhs = f"({cast_num(cb, tb, promote(tb))}) as u32"
            rt = "i32" if t == T_INT else "i64"
            ut = "u32" if t == T_INT else "u64"
            if op == "<<":
                return pre, f"{recv(lhs, t)}.wrapping_shl({rhs})", t, False
            if op == ">>":
                return pre, f"{recv(lhs, t)}.wrapping_shr({rhs})", t, False
            return pre, f"(({lhs}) as {ut}).wrapping_shr({rhs}) as {rt}", t, False
        if op in ("&", "|", "^"):
            if ta == T_BOOL and tb == T_BOOL:
                return pre, f"{wrap(ca)} {op} {wrap(cb)}", T_BOOL, False
            t = binary_numeric(ta, tb)
            return pre, f"{wrap(cast_num(ca, ta, t))} {op} {wrap(cast_num(cb, tb, t))}", t, False
        if op in ("+", "-", "*", "/", "%"):
            t = binary_numeric(ta, tb)
            x, y = cast_num(ca, ta, t), cast_num(cb, tb, t)
            if t.name in ("float", "double"):
                return pre, f"{wrap(x)} {op} {wrap(y)}", t, False
            fn = {"+": "wrapping_add", "-": "wrapping_sub", "*": "wrapping_mul", "/": "wrapping_div", "%": "wrapping_rem"}[op]
            return pre, f"{recv(x, t)}.{fn}({y})", t, False
        self.err(line, f"operator {op}")

    def as_string(self, code, ty, nullable):
        """A value as Java's string conversion renders it, as `&str`-able."""
        if ty.name == "String" and ty.dims == 0:
            if nullable:
                return f"&crate::javacc::string_value_of({code}.as_deref())"
            return f"&{code}"
        if ty == T_NULL:
            return '"null"'
        if ty.name in ("int", "long"):
            return f"&{wrap(code)}.to_string()"
        if ty.name == "char":
            return f"&crate::javacc::char_to_string({code})"
        if ty.name == "boolean":
            return f"&{wrap(code)}.to_string()"
        if ty.name == "float":
            return f"&rucene::util::java_fmt::float_to_string({code})"
        if ty.name == "Exception":
            return f"&{wrap(code)}.to_string()"
        if ty.name == "Token" and ty.dims == 0:
            # `String.valueOf(token)`: "null" for a null token, and a null
            # `toString()` (the image) is appended as "null" too
            return f"&crate::javacc::string_value_of({wrap(code)}.as_ref().and_then(|t| t.read(|o| o.image.clone())).as_deref())"
        self.err(0, f"string conversion of {ty}")

    # -------------------------------------------------------------- calls
    def emit_call(self, e, scope):
        recv, name, args, line = e[1], e[2], e[3], e[4]
        if recv is None or recv[0] == "this":
            if name in self.methods:
                return self.emit_own_call(name, args, scope, line)
            if name in self.profile.externals and self.mode == "parser":
                return self.emit_external(name, args, scope, line)
            self.err(line, f"unknown method {name}")
        # library calls
        if recv[0] == "name" and scope.get(recv[1]) is None and recv[1] not in self.fields \
                and f"{recv[1]}.{name}" in self.profile.static_calls:
            return self.apply_ext(self.profile.static_calls[f"{recv[1]}.{name}"], args, scope, line,
                                  f"{recv[1]}.{name}")
        if recv[0] == "name" and recv[1] in ("Token",) and name == "newToken":
            pre, cs = self.emit_args(args, scope, [T_INT, (T_STRING, True)], line)
            return pre, f"Some(Token::new_token({cs[0]}, {cs[1]}))", JType("Token"), True
        if recv[0] == "name" and recv[1] == "Float" and name == "parseFloat":
            pre, cs = self.emit_args(args, scope, [T_STRING], line)
            return pre, f"crate::javacc::parse_float(&{cs[0]})?", T_FLOAT, False
        if recv[0] == "name" and recv[1] == "Integer" and name == "parseInt":
            pre, cs = self.emit_args(args, scope, [T_STRING], line)
            return pre, f"crate::javacc::parse_int(&{cs[0]})?", T_INT, False
        if recv[0] == "lit" and recv[1] == "string" and name == "equals":
            pre, ca, ta, na = self.emit_expr(args[0], scope)
            lit = rust_str(recv[2])
            if na:
                return pre, f"({ca}.as_deref() == Some({lit}))", T_BOOL, False
            return pre, f"({ca} == {lit})", T_BOOL, False
        pr, cr, tr, nr = self.emit_place(recv, scope, write=True)
        if tr.dims == 0 and f"{tr.name}.{name}" in self.profile.instance_calls:
            pre, code, rt, rn = self.apply_ext(self.profile.instance_calls[f"{tr.name}.{name}"], args,
                                               scope, line, f"{tr.name}.{name}", receiver=cr)
            return pr + pre, code, rt, rn
        if tr.name == "String" and tr.dims == 0:
            base = f"crate::javacc::nn_str({cr}.as_deref())?" if nr else f"{cr}"
            if name == "length" and not args:
                return pr, f"crate::javacc::length(&{base})", T_INT, False
            if name == "substring":
                pre, cs = self.emit_args(args, scope, [T_INT] * len(args), line)
                if len(args) == 2:
                    return pr + pre, f"crate::javacc::substring(&{base}, {cs[0]}, {cs[1]})?", T_STRING, False
                return pr + pre, f"crate::javacc::substring_from(&{base}, {cs[0]})?", T_STRING, False
            self.err(line, f"String.{name}")
        if tr.name in ("List", "ArrayList") and tr.dims == 0:
            if recv[0] != "name" and not (recv[0] == "field" and recv[1][0] == "this"):
                self.err(line, "a list call on a computed receiver")
            target = self.lvalue_of(recv, scope, line)
            lst = f"crate::javacc::nn_mut({target}.as_mut())?" if nr else target
            if name == "size" and not args:
                return pr, f"({lst}.len() as i32)", T_INT, False
            if name == "add" and len(args) == 1:
                et = tr.args[0]
                pre, cs = self.emit_args(args, scope, [(et, self.type_nullable(et))], line)
                if re.search(r"\b(this|self)\b", cs[0]):
                    t = self.tmp("p")
                    pre.append(f"let {t} = {cs[0]};")
                    cs = [t]
                return pr + pre, f"{lst}.push({cs[0]})", T_VOID, False
            if name == "get" and len(args) == 1:
                pre, cs = self.emit_args(args, scope, [T_INT], line)
                et = tr.args[0]
                return pr + pre, f"{lst}[({cs[0]}) as usize].clone()", et, self.type_nullable(et)
            if name == "clear" and not args:
                return pr, f"{lst}.clear()", T_VOID, False
            if name == "set" and len(args) == 2:
                et = tr.args[0]
                pre, cs = self.emit_args(args, scope, [T_INT, (et, self.type_nullable(et))], line)
                v = self.tmp("p")
                pre.append(f"let {v} = {cs[1]};")
                return pr + pre, f"std::mem::replace(&mut {lst}[({cs[0]}) as usize], {v})", et, self.type_nullable(et)
            self.err(line, f"List.{name}")
        if tr.name == self.profile.tm_class and tr.dims == 0:
            return self.emit_tm_call(cr, name, args, scope, line, pr)
        if tr.name == "CharStream" and tr.dims == 0:
            return self.emit_charstream_call(cr, name, args, scope, line, pr)
        if tr.name in self.profile.types and name in ("setWeight", "getWeight"):
            pr, cr, tr, nr = self.emit_expr(recv, scope)
            if name == "getWeight":
                return pr, f"{cr}.get_weight()", T_FLOAT, False
            pre, cs = self.emit_args(args, scope, [T_FLOAT], line)
            return pr + pre, f"{cr}.set_weight({cs[0]})", T_VOID, False
        self.err(line, f"call {name} on {tr}")

    def emit_charstream_call(self, cr, name, args, scope, line, pr):
        target = cr
        table = {
            "readChar": ("read_char()?", T_CHAR),
            "BeginToken": ("begin_token()?", T_CHAR),
            "GetImage": ("get_image()", T_STRING),
            "getBeginLine": ("get_begin_line()", T_INT),
            "getBeginColumn": ("get_begin_column()", T_INT),
            "getEndLine": ("get_end_line()", T_INT),
            "getEndColumn": ("get_end_column()", T_INT),
        }
        if name in table:
            return pr, f"{target}.{table[name][0]}", table[name][1], False
        if name == "backup":
            pre, cs = self.emit_args(args, scope, [T_INT], line)
            return pr + pre, f"{target}.backup({cs[0]})", T_VOID, False
        self.err(line, f"CharStream.{name}")

    def emit_tm_call(self, cr, name, args, scope, line, pr):
        target = cr
        if name == "getNextToken":
            return pr, f"{target}.get_next_token()?", JType("Token"), True
        if name == "ReInit" and len(args) == 1:
            pre, ca, ta, na = self.emit_expr(args[0], scope)
            q = "?" if "ReInit/1/CharStream" in self.profile.tm_fallible else ""
            return pr + pre, f"{target}.re_init({ca}){q}", T_VOID, False
        self.err(line, f"{self.profile.tm_class}.{name}")

    def emit_args(self, args, scope, kinds, line):
        pre = []
        codes = []
        for a, kd in zip(args, kinds):
            p, c, t, n = self.emit_expr(a, scope)
            pre += p
            if isinstance(kd, tuple):
                target_ty, target_null = kd
            else:
                target_ty, target_null = kd, False
            codes.append(self.coerce(c, t, n, target_ty, target_null, line))
        return pre, codes

    def coerce(self, code, ty, nullable, target_ty, target_null, line):
        if ty == T_NULL:
            if not (target_null or self.type_nullable(target_ty)):
                self.err(line, f"null for a non-null {target_ty}")
            return "None"
        if target_ty.is_prim() and ty.is_prim():
            return cast_num(code, ty, target_ty)
        if target_ty.dims == 0 and ty.dims == 0 and target_ty.name != ty.name:
            types = self.profile.types
            if target_ty.name == "CharSequence" and ty.name == "String" and "CharSequence" in types:
                # a String is a CharSequence
                if nullable:
                    return f"{wrap(code)}.map(CharSequence::from)"
                return f"Some(CharSequence::from({code}))"
            if (ty.name, target_ty.name) in self.profile.upcasts:
                return self.profile.upcasts[(ty.name, target_ty.name)].replace("{0}", code)
            if ty.name in types and target_ty.name in types and types[ty.name][0] != types[target_ty.name][0]:
                self.err(line, f"no conversion from {ty} to {target_ty}")
        tn = target_null or self.type_nullable(target_ty)
        if tn and not nullable:
            return f"Some({code})"
        if not tn and nullable and not self.type_nullable(ty):
            if ty.name == "String":
                return f"crate::javacc::nn_str({code}.as_deref())?.to_string()"
            return f"crate::javacc::nn_owned({code})?"
        return code

    def lvalue_of(self, e, scope, line):
        if e[0] == "name":
            v = scope.get(e[1])
            if v is not None:
                return f"(*{v.rust})" if v.byref else v.rust
            fa = self.field_access(e[1], True, line)
            if fa is not None:
                return fa[0]
        if e[0] == "field" and e[1][0] == "this":
            fa = self.field_access(e[2], True, line)
            if fa is not None:
                return fa[0]
        self.err(line, "an unsupported assignment target")

    def method_sig(self, m):
        params = []
        for (ty, pname) in m["params"]:
            nullable = self.type_nullable(ty) or self.profile.nullable_params.get((m["name"], pname), False)
            params.append((ty, pname, nullable))
        return params

    @staticmethod
    def is_list(ty):
        return ty.dims == 0 and ty.name in ("List", "ArrayList")

    def borrows_this(self, code):
        """Whether evaluating `code` borrows the parser or token manager."""
        return re.search(r"\b(this|self)\b", code) is not None and "(" in code

    def hoist(self, pre, codes):
        """Java evaluates the arguments before the call: when one of them
        calls back into the receiver, every argument is evaluated into a
        local first, in order."""
        if not any(self.borrows_this(c) for c in codes):
            return codes
        out = []
        for c in codes:
            if re.fullmatch(r"-?[0-9]+|None|true|false|[a-z_][a-z0-9_]*", c):
                out.append(c)
                continue
            t = self.tmp("p")
            pre.append(f"let {t} = {c};")
            out.append(t)
        return out

    def emit_own_call(self, name, args, scope, line):
        cands = [m for m in self.methods[name] if len(m["params"]) == len(args)]
        if not cands:
            self.err(line, f"no overload of {name} with {len(args)} arguments")
        if len(cands) > 1:
            # by the type of the first argument
            p, c, t, n = self.emit_expr(args[0], scope)
            cands = [m for m in cands if m["params"][0][0].name == t.name] or cands
            if len(cands) > 1:
                self.err(line, f"ambiguous overload of {name}")
        m = cands[0]
        sig = self.method_sig(m)
        pre = []
        codes = []
        refs = []
        for a, (pty, pname, pnull) in zip(args, sig):
            if self.is_list(pty):
                refs.append(len(codes))
                codes.append(f"&mut {self.lvalue_of(a, scope, line)}")
                continue
            p, c, t, n = self.emit_expr(a, scope)
            pre += p
            codes.append(self.coerce(c, t, n, pty, pnull, line))
        if "static" not in m["mods"]:
            codes = self.hoist(pre, codes)
        rname = self.rust_method_name(m)
        kind = self.method_kind(m)
        call = self.call_syntax(m, rname, codes)
        if call.startswith(f"{rname}(") and scope.has_rust_name(rname):
            # a local of the same name (Java's `boost` beside `Boost()`)
            # shadows the function: name it by its path
            call = f"self::{call}"
        if kind == "raised":
            if self.fn_kind != "raised":
                self.err(line, f"calling the lookahead routine {name} outside one")
            call += "?"
        elif kind == "result":
            call += "?"
        rt = m["type"]
        return pre, call, rt, self.type_nullable(rt) or fkey(m) in self.nullable_returns

    def emit_external(self, name, args, scope, line):
        return self.apply_ext(self.profile.externals[name], args, scope, line, name)

    def apply_ext(self, ext, args, scope, line, name, receiver=None):
        """A call described by `ext`: the arguments converted by their
        kinds, then substituted in the template (`{this}`, `{r}` the
        receiver, `{0}`, `{1}`, ...)."""
        if len(args) != len(ext.params):
            self.err(line, f"{name} takes {len(ext.params)} arguments")
        pre = []
        codes = []
        for a, kind in zip(args, ext.params):
            p, c, t, n = self.emit_expr(a, scope)
            pre += p
            if kind == "mutref":
                codes.append(self.lvalue_of(a, scope, line))
            elif kind == "ref":
                codes.append(self.lvalue_of(a, scope, line))
            elif kind == "str":
                codes.append(c if not n else f"crate::javacc::nn_str({c}.as_deref())?.to_string()")
            elif kind == "nstr":
                codes.append(c if n else f"Some({c})")
            elif kind == "token":
                codes.append(f"crate::javacc::nn_owned({c})?")
            elif kind == "nn":
                codes.append(f"crate::javacc::nn_owned({c})?" if n else c)
            elif isinstance(kind, tuple):
                codes.append(self.coerce(c, t, n, kind[0], kind[1], line))
            elif kind == "val":
                codes.append(c)
            else:
                self.err(line, f"unknown argument kind {kind}")
        if "{this}" in ext.rust:
            vals = [i for i, k in enumerate(ext.params) if k not in ("mutref", "ref")]
            hoisted = self.hoist(pre, [codes[i] for i in vals])
            for i, h in zip(vals, hoisted):
                codes[i] = h
        call = ext.rust.replace("{this}", "this")
        if receiver is not None:
            call = call.replace("{r}", receiver)
        for i, c in enumerate(codes):
            call = call.replace("{" + str(i) + "}", c)
        return pre, call, ext.ret, ext.ret_nullable or self.type_nullable(ext.ret)

    def emit_new(self, e, scope):
        ty, args, line = e[1], e[2], e[3]
        key = f"{ty.name}/{len(args)}"
        if key in self.profile.news:
            return self.apply_ext(self.profile.news[key], args, scope, line, f"new {key}")
        if ty.name == "Token" and not args:
            return [], "Some(TokenRef::new(Token::new()))", ty, True
        if ty.name == "JJCalls" and not args:
            return [], "Some(JJCallsRef::new(JJCalls::default()))", ty, True
        if ty.name in ("ArrayList",) and not args:
            return [], "Vec::new()", JType("ArrayList", ty.args), False
        if ty.name == self.profile.tm_class and len(args) == 1:
            p, c, t, n = self.emit_expr(args[0], scope)
            return p, f"{self.profile.tm_class}::new({c})", ty, False
        if ty.name == "ParseException":
            if not args:
                return [], "ParseException::new()", ty, False
            if len(args) == 1:
                p, cs = self.emit_args(args, scope, [T_STRING], line)
                return p, f"ParseException::with_message({cs[0]})", ty, False
            if len(args) == 3:
                p, cs = self.emit_args(args, scope, [JType("Token"), JType("int", (), 2), JType("String", (), 1)], line)
                return p, f"ParseException::with_tokens(crate::javacc::nn({cs[0]}.as_ref())?.clone(), {cs[1]}, &{cs[2]})?", ty, False
        if ty.name == "TokenMgrError":
            if len(args) == 2:
                p, cs = self.emit_args(args, scope, [T_STRING, T_INT], line)
                return p, f"TokenMgrError::with_message({cs[0]}, {cs[1]})", ty, False
            if len(args) == 7:
                p, cs = self.emit_args(args, scope, [T_BOOL, T_INT, T_INT, T_INT, (T_STRING, True), T_INT, T_INT], line)
                return p, f"TokenMgrError::lexical({cs[0]}, {cs[1]}, {cs[2]}, {cs[3]}, {cs[4]}.as_deref(), {cs[5]}, {cs[6]})", ty, False
        if ty.name == "Error" and len(args) == 1:
            p, cs = self.emit_args(args, scope, [T_STRING], line)
            return p, f"crate::javacc::java_error({cs[0]})", ty, False
        self.err(line, f"new {ty}")

    # ------------------------------------------------------------ assignment
    def emit_side_effect(self, e, scope, as_value):
        """Assignments and ++/--: the prelude doing them, and (as a value) the
        expression reading the result."""
        k = e[0]
        line = e[-1]
        if k in ("preinc", "postinc"):
            op, target = e[1], e[2]
            pre, lv, ty, nl, write = self.target(target, scope, line)
            fn = "wrapping_add" if op == "++" else "wrapping_sub"
            one = "1"
            if k == "postinc" and as_value:
                t = self.tmp("v")
                pre.append(f"let {t} = {lv};")
                pre.append(write(f"{t}.{fn}({one})"))
                return pre, t, ty, False
            pre.append(write(f"{lv}.{fn}({one})"))
            return pre, lv, ty, False
        op, target, value = e[1], e[2], e[3]
        if target[0] == "field" and target[1][0] != "this":
            return self.emit_object_field_assign(e, scope, as_value)
        pre, lv, ty, nl, write = self.target(target, scope, line)
        pv, cv, tv, nv = self.emit_expr(value, scope)
        pre += pv
        if op == "=":
            rhs = self.coerce(cv, tv, nv, ty, nl, line)
        else:
            bop = op[:-1]
            if ty.name == "String" and bop == "+":
                rhs = f"crate::javacc::concat(&[{self.as_string(lv, ty, nl)}, {self.as_string(cv, tv, nv)}])"
                rhs = self.coerce(rhs, T_STRING, False, ty, nl, line)
            else:
                fake = ("bin", bop, ("rawexpr", lv, ty), ("rawexpr", cv, tv), line)
                _, code, rt, _ = self.emit_binary_raw(bop, lv, ty, cv, tv, line)
                rhs = cast_num(code, rt, ty)
        if as_value:
            t = self.tmp("a")
            pre.append(f"let {t} = {rhs};")
            pre.append(write(f"{t}.clone()" if not ty.is_prim() else t))
            return pre, t, ty, nl
        pre.append(write(rhs))
        return pre, "()", T_VOID, False

    def emit_binary_raw(self, op, ca, ta, cb, tb, line):
        if op in ("<<", ">>", ">>>"):
            t = promote(ta)
            lhs = cast_num(ca, ta, t)
            rhs = f"({cast_num(cb, tb, promote(tb))}) as u32"
            rt = "i32" if t == T_INT else "i64"
            ut = "u32" if t == T_INT else "u64"
            if op == "<<":
                return [], f"{recv(lhs, t)}.wrapping_shl({rhs})", t, False
            if op == ">>":
                return [], f"{recv(lhs, t)}.wrapping_shr({rhs})", t, False
            return [], f"(({lhs}) as {ut}).wrapping_shr({rhs}) as {rt}", t, False
        t = binary_numeric(ta, tb)
        x, y = cast_num(ca, ta, t), cast_num(cb, tb, t)
        if op in ("&", "|", "^"):
            return [], f"{wrap(x)} {op} {wrap(y)}", t, False
        fn = {"+": "wrapping_add", "-": "wrapping_sub", "*": "wrapping_mul", "/": "wrapping_div", "%": "wrapping_rem"}[op]
        return [], f"{recv(x, t)}.{fn}({y})", t, False

    def target(self, target, scope, line):
        """An assignable place: (prelude, read code, JType, nullable, write fn)."""
        if target[0] == "name" or (target[0] == "field" and target[1][0] == "this"):
            name = target[1] if target[0] == "name" else target[2]
            v = scope.get(name)
            if v is not None:
                rd = v.rust
                return [], rd, v.ty, v.nullable, (lambda val, r=rd: f"{r} = {val};")
            fa = self.field_access(name, True, line)
            if fa is not None:
                code, ty, nl, static = fa
                if static:
                    self.err(line, f"assignment to the static field {name}")
                rd_code = self.field_access(name, False, line)[0]
                rd = rd_code + ("" if ty.is_prim() else ".clone()")
                return [], rd, ty, nl, (lambda val, w=code: f"{w} = {val};")
            self.err(line, f"assignment to unknown {name}")
        if target[0] == "index":
            pa, ca, ta, na = self.emit_expr(target[1], scope)
            pi, ci, ti, ni = self.emit_expr(target[2], scope)
            pre = pa + pi
            idx = self.tmp("i")
            pre.append(f"let {idx} = ({ci}) as usize;")
            et = ta.elem()
            arr_w = self.lvalue_of(target[1], scope, line)
            arr_r = ca
            if na:
                arr_w = f"crate::javacc::nn_mut({arr_w}.as_mut())?"
                arr_r = f"crate::javacc::nn({ca}.as_ref())?"
            rd = f"{arr_r}[{idx}]" + ("" if et.is_prim() else ".clone()")
            return pre, rd, et, self.type_nullable(et), (lambda val, a=arr_w, i=idx: f"{a}[{i}] = {val};")
        self.err(line, "an unsupported assignment target")

    def emit_object_field_assign(self, e, scope, as_value):
        op, target, value, line = e[1], e[2], e[3], e[4]
        obj, name = target[1], target[2]
        po, co, to, no = self.emit_expr(obj, scope)
        pre = list(po)
        o = self.tmp("o")
        pre.append(f"let {o} = {co};")
        if to.name == "Token" and to.dims == 0:
            ftypes = {"kind": T_INT, "beginLine": T_INT, "beginColumn": T_INT, "endLine": T_INT,
                      "endColumn": T_INT, "image": T_STRING, "next": JType("Token"), "specialToken": JType("Token")}
            ft = ftypes.get(name)
            fnull = name in ("image", "next", "specialToken")
        elif to.name == "JJCalls" and to.dims == 0:
            node = self.nested["JJCalls"]
            ft = None
            for m in node["members"]:
                if m["kind"] == "field":
                    for (n, _) in m["decls"]:
                        if n == name:
                            ft = m["type"]
            fnull = ft is not None and not ft.is_prim()
        else:
            self.err(line, f"assignment to a field of {to}")
        if ft is None:
            self.err(line, f"unknown field {name}")
        pv, cv, tv, nv = self.emit_expr(value, scope)
        pre += pv
        place = f"crate::javacc::nn({o}.as_ref())?.lock().{snake(name)}"
        if op == "=":
            rhs = self.coerce(cv, tv, nv, ft, fnull, line)
        elif op == "+=" and ft.name == "String":
            cur = f"crate::javacc::nn({o}.as_ref())?.read(|o| o.{snake(name)}.clone())"
            rhs = f"Some(crate::javacc::concat(&[&crate::javacc::string_value_of({cur}.as_deref()), {self.as_string(cv, tv, nv)}]))"
        else:
            self.err(line, f"compound assignment {op} to a field")
        v = self.tmp("a")
        pre.append(f"let {v} = {rhs};")
        if as_value:
            pre.append(f"{place} = {v}.clone();")
            return pre, v, ft, fnull
        pre.append(f"{place} = {v};")
        return pre, "()", T_VOID, False

    # ============================================================ statements
    def emit_block(self, b, scope, out, indent):
        inner = Scope(scope)
        for s in b[1]:
            self.emit_stmt(s, inner, out, indent)

    def emit_stmt(self, s, scope, out, indent):
        k = s[0]
        pad = "    " * indent
        line = s[-1]
        if k == "block":
            out.append(pad + "{")
            self.emit_block(s, scope, out, indent + 1)
            out.append(pad + "}")
            return
        if k == "empty":
            return
        if k == "local":
            ty, decls = s[1], s[2]
            for (name, init) in decls:
                nullable = self.type_nullable(ty) or self.local_nullable(name, ty, init)
                rname = snake(name)
                if scope.get(name) is not None or rname in ("i", "j") and False:
                    pass
                rt = self.rust_type(ty, nullable, line)
                if init is None:
                    # Java's definite assignment guarantees a write before any
                    # read: the default value is never observed.
                    if ty.is_prim() or nullable or self.type_nullable(ty):
                        out.append(pad + f"let mut {rname}: {rt} = {self.default_value(ty, nullable, line)};")
                    else:
                        # no default: Rust's own definite-initialization
                        # analysis checks Java's
                        out.append(pad + f"let mut {rname}: {rt};")
                    scope.put(name, Var(rname, ty, nullable))
                    continue
                if isinstance(init, tuple) and init[0] == "arrayinit":
                    self.err(line, "a local array initializer")
                p, c, t, n = self.emit_expr(init, scope)
                for x in p:
                    out.append(pad + x)
                c = self.coerce(c, t, n, ty, nullable, line)
                out.append(pad + f"let mut {rname}: {rt} = {c};")
                scope.put(name, Var(rname, ty, nullable))
            return
        if k == "expr":
            e = s[1]
            if e[0] in ("assign", "preinc", "postinc"):
                pre, code, ty, nl = self.emit_side_effect(e, scope, as_value=False)
                for x in pre:
                    out.append(pad + x)
                return
            p, c, t, n = self.emit_expr(e, scope)
            for x in p:
                out.append(pad + x)
            if t == T_VOID or e[0] == "call":
                out.append(pad + f"{c};" if t == T_VOID else pad + f"let _ = {c};")
            else:
                self.err(line, "an expression statement that is not a call or an assignment")
            return
        if k == "if":
            cond, then, els = s[1], s[2], s[3]
            p, c, t, n = self.emit_cond(cond, scope)
            for x in p:
                out.append(pad + x)
            out.append(pad + f"if {c} {{")
            self.emit_body(then, scope, out, indent + 1)
            if els is None:
                out.append(pad + "}")
            elif els[0] == "if":
                out.append(pad + "} else {")
                self.emit_stmt(els, scope, out, indent + 1)
                out.append(pad + "}")
            else:
                out.append(pad + "} else {")
                self.emit_body(els, scope, out, indent + 1)
                out.append(pad + "}")
            return
        if k == "labeled":
            label, body = s[1], s[2]
            if body[0] in ("while", "for", "do", "foreach"):
                self.emit_loop(body, scope, out, indent, label)
                return
            self.err(line, "a label on a statement that is not a loop")
        if k in ("while", "for", "do", "foreach"):
            self.emit_loop(s, scope, out, indent, None)
            return
        if k == "switch":
            self.emit_switch(s, scope, out, indent)
            return
        if k == "break":
            label = s[1]
            if label is not None:
                for kind, lbl, rlabel in reversed(self.loop_stack):
                    if kind == "barrier":
                        self.err(line, "break out of a try body")
                    if lbl == label:
                        break
                out.append(pad + f"break '{snake(label)};")
                return
            # innermost breakable
            for kind, lbl, rlabel in reversed(self.loop_stack):
                if kind == "barrier":
                    self.err(line, "break out of a try body")
                if kind == "switch":
                    out.append(pad + f"break '{rlabel};")
                    return
                out.append(pad + (f"break '{rlabel};" if rlabel else "break;"))
                return
            self.err(line, "break outside a loop or switch")
        if k == "continue":
            label = s[1]
            for kind, lbl, rlabel in reversed(self.loop_stack):
                if kind == "barrier":
                    self.err(line, "continue out of a try body")
                if kind == "switch":
                    continue
                if label is not None and lbl != label:
                    continue
                cont = self.continue_target(kind, lbl, rlabel)
                out.append(pad + cont)
                return
            self.err(line, "continue outside a loop")
        if k == "return":
            if self.in_closure:
                self.err(line, "return inside a try body")
            e = s[1]
            if e is None:
                out.append(pad + ("return Ok(());" if self.fn_kind in ("result", "raised") else "return;"))
                return
            p, c, t, n = self.emit_expr(e, scope)
            for x in p:
                out.append(pad + x)
            rt = self.current_method_ret
            c = self.coerce(c, t, n, rt, self.current_ret_nullable, line)
            if self.fn_kind in ("result", "raised"):
                out.append(pad + f"return Ok({c});")
            else:
                out.append(pad + f"return {c};")
            return
        if k == "throw":
            e = s[1]
            if e[0] == "name" and e[1] == "jj_ls":
                if self.fn_kind != "raised":
                    self.err(line, "throw jj_ls outside a lookahead routine")
                out.append(pad + "return Err(Raised::LookaheadSuccess(LookaheadSuccess));")
                return
            p, c, t, n = self.emit_expr(e, scope)
            for x in p:
                out.append(pad + x)
            if t.name in ("ParseException", "TokenMgrError"):
                c = f"{c}.error()"
            elif t.name == "Error":
                pass
            else:
                self.err(line, f"throwing a {t}")
            if self.fn_kind not in ("result", "raised"):
                self.err(line, "throw in a function without a Result")
            out.append(pad + f"return Err({c}.into());")
            return
        if k == "try":
            self.emit_try(s, scope, out, indent)
            return
        self.err(line, f"unsupported statement {k}")

    def continue_target(self, kind, lbl, rlabel):
        if kind in ("for", "do"):
            return f"break '{rlabel}_body;"
        return f"continue '{rlabel};" if rlabel else "continue;"

    def emit_body(self, s, scope, out, indent):
        if s[0] == "block":
            self.emit_block(s, scope, out, indent)
        else:
            self.emit_stmt(s, Scope(scope), out, indent)

    def emit_cond(self, cond, scope):
        # `"" != null` and `true` stay as Java wrote them (constant true)
        if cond[0] == "bin" and cond[1] == "!=" and cond[2][0] == "lit" and cond[2][1] == "string" and cond[3][0] == "lit" and cond[3][1] == "null":
            return [], "true", T_BOOL, False
        return self.emit_expr(cond, scope)

    def new_label(self, base="l"):
        self.label_counter += 1
        return f"{base}{self.label_counter}"

    def emit_loop(self, s, scope, out, indent, label):
        pad = "    " * indent
        k = s[0]
        line = s[-1]
        rlabel = snake(label) if label else self.new_label()
        if k == "while":
            cond, body = s[1], s[2]
            p, c, t, n = self.emit_cond(cond, scope)
            self.loop_stack.append(("while", label, rlabel))
            out.append(pad + f"'{rlabel}: loop {{")
            for x in p:
                out.append(pad + "    " + x)
            if c != "true":
                out.append(pad + f"    if !({c}) {{ break; }}")
            self.emit_body(body, scope, out, indent + 1)
            out.append(pad + "}")
            self.loop_stack.pop()
            return
        if k == "do":
            body, cond = s[1], s[2]
            self.loop_stack.append(("do", label, rlabel))
            out.append(pad + f"'{rlabel}: loop {{")
            out.append(pad + f"    '{rlabel}_body: {{")
            self.emit_body(body, scope, out, indent + 2)
            out.append(pad + "    }")
            p, c, t, n = self.emit_cond(cond, scope)
            for x in p:
                out.append(pad + "    " + x)
            out.append(pad + f"    if !({c}) {{ break; }}")
            out.append(pad + "}")
            self.loop_stack.pop()
            return
        if k == "for":
            init, cond, update, body = s[1], s[2], s[3], s[4]
            inner = Scope(scope)
            out.append(pad + "{")
            for st in init:
                self.emit_stmt(st, inner, out, indent + 1)
            self.loop_stack.append(("for", label, rlabel))
            out.append(pad + f"    '{rlabel}: loop {{")
            if cond is not None:
                p, c, t, n = self.emit_cond(cond, inner)
                for x in p:
                    out.append(pad + "        " + x)
                out.append(pad + f"        if !({c}) {{ break; }}")
            out.append(pad + f"        '{rlabel}_body: {{")
            self.emit_body(body, inner, out, indent + 3)
            out.append(pad + "        }")
            for u in update:
                pre, code, ty, nl = self.emit_side_effect(u, inner, as_value=False) if u[0] in ("assign", "preinc", "postinc") else self.err(line, "a for update that is not an assignment")
                for x in pre:
                    out.append(pad + "        " + x)
            out.append(pad + "    }")
            out.append(pad + "}")
            self.loop_stack.pop()
            return
        if k == "foreach":
            ty, name, iterable, body = s[1], s[2], s[3], s[4]
            p, c, t, n = self.emit_expr(iterable, scope)
            for x in p:
                out.append(pad + x)
            lst = self.lvalue_of(iterable, scope, line)
            idx = self.tmp("k")
            inner = Scope(scope)
            rname = snake(name)
            nullable = self.type_nullable(ty)
            inner.put(name, Var(rname, ty, nullable))
            self.loop_stack.append(("while", label, rlabel))
            out.append(pad + f"let mut {idx} = 0usize;")
            out.append(pad + f"'{rlabel}: loop {{")
            out.append(pad + f"    if {idx} >= {lst}.len() {{ break; }}")
            out.append(pad + f"    let {rname}: {self.rust_type(ty, nullable, line)} = {lst}[{idx}].clone();")
            out.append(pad + f"    {idx} += 1;")
            self.emit_body(body, inner, out, indent + 1)
            out.append(pad + "}")
            self.loop_stack.pop()
            return

    def emit_switch(self, s, scope, out, indent):
        pad = "    " * indent
        sel, groups, line = s[1], s[2], s[3]
        p, c, t, n = self.emit_expr(sel, scope)
        for x in p:
            out.append(pad + x)
        label = self.new_label("sw")
        self.loop_stack.append(("switch", None, label))
        out.append(pad + f"'{label}: {{")
        out.append(pad + f"    match {c} {{")
        has_default = False
        for gi, (labels, stmts) in enumerate(groups):
            # a group without statements falls into the next: merge labels
            if not stmts and gi + 1 < len(groups):
                groups[gi + 1] = (labels + groups[gi + 1][0], groups[gi + 1][1])
                continue
            pats = []
            for lb in labels:
                if lb == "default":
                    pats.append("_")
                    has_default = True
                else:
                    pats.append(self.case_pattern(lb, line))
            if "_" in pats:
                pats = ["_"]
            # the last statement must leave the switch
            body = list(stmts)
            if body and body[-1][0] == "break" and body[-1][1] is None:
                body = body[:-1]
            elif body and body[-1][0] in ("return", "throw", "continue", "break"):
                pass
            elif body and body[-1][0] == "block" and body[-1][1] and body[-1][1][-1][0] in ("break", "return", "throw"):
                last = body[-1]
                inner = list(last[1])
                if inner[-1][0] == "break" and inner[-1][1] is None:
                    inner = inner[:-1]
                body = body[:-1] + [("block", inner, last[2])]
            elif gi + 1 < len(groups):
                self.err(line, "a switch case that falls through")
            out.append(pad + "        " + " | ".join(pats) + " => {")
            inner = Scope(scope)
            for st in body:
                self.emit_stmt(st, inner, out, indent + 3)
            out.append(pad + "        }")
        if not has_default:
            out.append(pad + "        _ => {}")
        out.append(pad + "    }")
        out.append(pad + "}")
        self.loop_stack.pop()

    def case_pattern(self, lb, line):
        if lb[0] == "lit" and lb[1] in ("int", "long"):
            return str(lb[2])
        if lb[0] == "lit" and lb[1] == "char":
            return str(lb[2])
        if lb[0] == "name":
            if lb[1] in self.constants:
                return f"qpc::{self.constants[lb[1]]}"
        if lb[0] == "unary" and lb[1] == "-" and lb[2][0] == "lit":
            return f"-{lb[2][2]}"
        self.err(line, "an unsupported case label")

    def emit_try(self, s, scope, out, indent):
        pad = "    " * indent
        body, catches, fin, line = s[1], s[2], s[3], s[4]
        # the `jj_2_N` pattern
        if fin is not None:
            if (len(body[1]) == 1 and body[1][0][0] == "return" and len(catches) == 1
                    and catches[0][0] == ["LookaheadSuccess"] and fin[0] == "block"):
                ret = body[1][0][1]
                p, c, t, n = self.emit_lookahead_value(ret, scope)
                for x in p:
                    out.append(pad + x)
                r = self.tmp("r")
                out.append(pad + f"let {r}: Result<bool> = match {c} {{")
                out.append(pad + "    Ok(v) => Ok(!v),")
                out.append(pad + "    Err(Raised::LookaheadSuccess(_)) => Ok(true),")
                out.append(pad + "    Err(Raised::Throwable(e)) => Err(e),")
                out.append(pad + "};")
                # the finally block
                self.emit_block(fin, scope, out, indent)
                out.append(pad + f"return {r};")
                return
            self.err(line, "try/finally outside the jj_2 pattern")
        if not catches:
            self.err(line, "try without catch")
        # The body runs in a closure returning `Result<(), E>`: a throw, or
        # an error a call raises, ends it as Java's throw ends the try block.
        # Java's try bodies here hold no return, break or continue.
        raised = any(t == "LookaheadSuccess" for (types, _, _) in catches for t in types) or self.calls_raised(body)
        err_ty = "Raised" if raised else "LuceneError"
        caught = self.tmp("caught")
        out.append(pad + f"let {caught}: Option<{err_ty}> = (|| -> std::result::Result<(), {err_ty}> {{")
        saved = (self.fn_kind, self.in_closure)
        self.fn_kind = "raised" if raised else "result"
        self.in_closure = True
        self.loop_stack.append(("barrier", None, None))
        self.emit_block(body, scope, out, indent + 1)
        self.loop_stack.pop()
        self.fn_kind, self.in_closure = saved
        out.append(pad + "    Ok(())")
        out.append(pad + "})().err();")
        out.append(pad + f"if let Some(__e) = {caught} {{")
        first = True
        for (types, name, cblock) in catches:
            conds = []
            for tname in types:
                conds.append(self.catch_test(tname, err_ty, line))
            cond = " || ".join(conds)
            out.append(pad + ("    if " if first else "    } else if ") + cond + " {")
            first = False
            inner = Scope(scope)
            rname = snake(name)
            if name in self.used_names(cblock):
                if err_ty != "LuceneError":
                    self.err(line, "a caught Raised used as a value")
                out.append(pad + f"        let {rname} = __e;")
                inner.put(name, Var(rname, JType("Exception"), False))
            self.emit_block(cblock, inner, out, indent + 2)
        out.append(pad + "    } else {")
        if err_ty == self.fn_err() and self.fn_kind in ("result", "raised"):
            out.append(pad + "        return Err(__e);")
        elif err_ty == "Raised" and self.fn_kind == "result":
            # the catch clauses above take every LookaheadSuccess
            out.append(pad + "        if let Raised::Throwable(t) = __e {")
            out.append(pad + "            return Err(t);")
            out.append(pad + "        }")
        elif err_ty == "LuceneError" and self.fn_kind == "raised":
            out.append(pad + "        return Err(__e.into());")
        else:
            self.err(line, "an uncaught throwable in a function without a Result")
        out.append(pad + "    }")
        out.append(pad + "}")

    in_closure = False

    def calls_raised(self, node):
        if isinstance(node, tuple) and node:
            if node[0] == "call" and node[1] is None and (node[2].startswith("jj_3") or node[2] == "jj_scan_token"):
                return True
            return any(self.calls_raised(y) for y in node)
        if isinstance(node, list):
            return any(self.calls_raised(y) for y in node)
        return False

    try_stack = []

    def catch_test(self, tname, err_ty, line):
        short = tname.split(".")[-1]
        if short == "LookaheadSuccess":
            return "matches!(__e, Raised::LookaheadSuccess(_))"
        if err_ty == "Raised":
            inner = "matches!(&__e, Raised::Throwable(t) if {})"
            if short == "IOException":
                return inner.format("t.is_io_exception()")
            if short == "Exception":
                return inner.format("crate::javacc::is_exception(t)")
            self.err(line, f"catch {tname}")
        if short == "IOException":
            return "__e.is_io_exception()"
        if short == "Exception":
            return "crate::javacc::is_exception(&__e)"
        self.err(line, f"catch {tname}")

    def used_names(self, node):
        names = set()

        def walk(x):
            if isinstance(x, tuple):
                if x and x[0] == "name":
                    names.add(x[1])
                for y in x:
                    walk(y)
            elif isinstance(x, list):
                for y in x:
                    walk(y)
        walk(node)
        return names

    def emit_lookahead_value(self, ret, scope):
        # `(!jj_3_N())`
        e = ret
        while e[0] == "paren":
            e = e[1]
        if e[0] == "unary" and e[1] == "!":
            e = e[2]
            while e[0] == "paren":
                e = e[1]
            if e[0] == "call" and e[1] is None:
                m = self.methods[e[2]][0]
                return [], f"{self.rust_method_name(m)}(this)", T_BOOL, False
        self.err(ret[-1], "an unexpected jj_2 body")

    def local_nullable(self, name, ty, init):
        if ty.name == "String" and ty.dims == 0:
            # a Java String reference may hold null
            return True
        if ty.dims > 0:
            if init is not None and init[0] == "lit" and init[1] == "null":
                return True
            return name in self.nullable_locals
        if ty.name in ("List", "ArrayList"):
            if init is not None and init[0] == "lit" and init[1] == "null":
                return True
            return name in self.nullable_locals
        return False

    nullable_locals = set()

    def scan_nullable_locals(self, body):
        """The locals of a String, array or list type a method assigns `null`
        or a nullable value to."""
        found = set()

        def nullable_value(v):
            if v[0] == "lit" and v[1] == "null":
                return True
            if v[0] == "index" and self.static_string_array(v[1]):
                return True
            if v[0] == "call" and v[1] is None and v[2] in self.methods:
                return any(fkey(m) in self.nullable_returns for m in self.methods[v[2]] if len(m["params"]) == len(v[3]))
            return False

        def walk(x):
            if isinstance(x, tuple) and x:
                if x[0] == "assign" and x[1] == "=" and x[2][0] == "name" and nullable_value(x[3]):
                    found.add(x[2][1])
                if x[0] == "local":
                    for (n, init) in x[2]:
                        if init is not None and isinstance(init, tuple) and nullable_value(init):
                            found.add(n)
                for y in x:
                    walk(y)
            elif isinstance(x, list):
                for y in x:
                    walk(y)
        walk(body)
        return found

    nullable_returns = set()

    def compute_nullable_returns(self):
        """The methods returning a list or an array that may return `null`:
        a `null` literal or a local that may hold `null`."""
        found = set()
        changed = True
        while changed:
            changed = False
            for ms in self.methods.values():
                for m in ms:
                    ty = m["type"]
                    if m["body"] is None or fkey(m) in found or not (self.is_list(ty) or ty.dims > 0):
                        continue
                    self.nullable_returns = found
                    nl = self.scan_nullable_locals(m["body"])

                    def walk(x):
                        if isinstance(x, tuple) and x:
                            if x[0] == "return" and x[1] is not None:
                                e = x[1]
                                while e[0] == "paren":
                                    e = e[1]
                                if (e[0] == "lit" and e[1] == "null") or (e[0] == "name" and e[1] in nl):
                                    return True
                            return any(walk(y) for y in x)
                        if isinstance(x, list):
                            return any(walk(y) for y in x)
                        return False
                    if walk(m["body"]):
                        found.add(fkey(m))
                        changed = True
        self.nullable_returns = found
        return found

    # ------------------------------------------------------------- methods
    def method_kind(self, m):
        name = m["name"]
        if name.startswith("jj_3") or name == "jj_scan_token":
            return "raised"
        if self.mode == "tm":
            # every token manager method that reads input, raises
            # TokenMgrError or calls one that does
            return "result" if self.tm_fallible(m) else "plain"
        return "result" if self.parser_fallible(m) else "plain"

    def tm_fallible(self, m):
        return fkey(m) in self.fallible

    def parser_fallible(self, m):
        return fkey(m) in self.fallible

    fallible = set()

    def compute_fallible(self):
        """A fixpoint over the methods: a method is fallible (returns a
        `Result`) if it throws, reads the input, dereferences a reference
        that may be null, or calls a fallible method."""
        fallible = set()
        members = [m for ms in self.methods.values() for m in ms if m["body"] is not None]
        changed = True
        while changed:
            changed = False
            for m in members:
                key = fkey(m)
                if key in fallible:
                    continue
                self.null_arrays = self.null_array_locals(m["body"])
                if self.body_fallible(m["body"], fallible):
                    fallible.add(key)
                    changed = True
        return fallible

    null_arrays = set()

    def null_array_locals(self, body):
        found = set()

        def walk(x):
            if isinstance(x, tuple) and x:
                if x[0] == "local" and x[1].dims > 0:
                    for (n, init) in x[2]:
                        if init is None or (isinstance(init, tuple) and init[0] == "lit" and init[1] == "null"):
                            found.add(n)
                for y in x:
                    walk(y)
            elif isinstance(x, list):
                for y in x:
                    walk(y)
        walk(body)
        return found

    INFALLIBLE_RECEIVER_CALLS = {
        "CharStream": {"getBeginLine", "getBeginColumn", "getEndLine", "getEndColumn", "GetImage", "backup"},
        "List": {"size", "add", "get", "clear"},
    }

    def nonnull_field_type(self, e):
        """The type of a non-null field `e` names, or None."""
        name = None
        if e[0] == "name":
            name = e[1]
        elif e[0] == "field" and e[1][0] == "this":
            name = e[2]
        if name is None or name not in self.fields:
            return None
        m, init = self.fields[name]
        ty = m["type"]
        if self.field_nullable(name) or self.type_nullable(ty):
            return None
        return ty

    def array_nonnull(self, e):
        if e[0] == "name":
            if e[1] in self.fields:
                return not self.field_nullable(e[1])
            return e[1] not in self.null_arrays
        if e[0] == "field" and e[1][0] == "this":
            return e[2] in self.fields and not self.field_nullable(e[2])
        return False

    def body_fallible(self, node, fallible):
        if isinstance(node, list):
            return any(self.body_fallible(y, fallible) for y in node)
        if not (isinstance(node, tuple) and node):
            return False
        k = node[0]
        if k == "throw":
            return True
        if k == "call":
            recv, name, args = node[1], node[2], node[3]
            if self.body_fallible(args, fallible):
                return True
            if recv is None or recv[0] == "this":
                if any(k.startswith(f"{name}/{len(args)}/") for k in fallible):
                    return True
                ext = self.profile.externals.get(name)
                return ext is not None and ext.result
            if recv[0] == "name" and recv[1] == "Token" and name == "newToken":
                return False
            if recv[0] == "lit" and recv[1] == "string" and name == "equals":
                return False
            if recv[0] == "name" and recv[1] in ("Float", "Integer"):
                return True
            ty = self.nonnull_field_type(recv)
            if ty is not None:
                if ty.name == self.profile.tm_class:
                    return name != "ReInit" or "ReInit/1/CharStream" in self.profile.tm_fallible
                key = "List" if self.is_list(ty) else ty.name
                if name in self.INFALLIBLE_RECEIVER_CALLS.get(key, ()):
                    return False
            return True
        if k == "field":
            obj, name = node[1], node[2]
            if obj[0] == "this":
                return False
            if obj[0] == "name" and obj[1] == "TokenMgrError":
                return False
            if name == "length" and self.array_nonnull(obj):
                return False
            return True
        if k == "index":
            if not self.array_nonnull(node[1]):
                return True
            return self.body_fallible(node[2], fallible)
        if k == "new":
            ext = self.profile.news.get(f"{node[1].name}/{len(node[2])}")
            if ext is not None and ext.result:
                return True
            if node[1].name == "ParseException" and len(node[2]) == 3:
                return True
            return self.body_fallible(node[2], fallible)
        if k == "cast" and not node[1].is_prim():
            return True
        return any(self.body_fallible(y, fallible) for y in node[1:])

    def rust_method_name(self, m):
        name = m["name"]
        if name == "ReInit":
            pt = m["params"][0][0].name if m["params"] else ""
            if len(m["params"]) == 2:
                return "re_init_with_lex_state"
            if pt == self.profile.tm_class:
                return "re_init_with_token_manager"
            return "re_init"
        if name == "trace_enabled":
            return "trace_enabled"
        return snake(name)

    def call_syntax(self, m, rname, codes):
        args = ", ".join(codes)
        if self.mode == "tm":
            if "static" in m["mods"]:
                return f"Self::{rname}({args})"
            return f"self.{rname}({args})"
        if self.is_public_parser_member(m):
            return f"this.{rname}({args})"
        return f"{rname}(this{', ' if args else ''}{args})"

    def is_public_parser_member(self, m):
        return "public" in m["mods"] and m["name"] not in ("ReInit",) or (m["name"] == "ReInit" and m["params"] and m["params"][0][0].name == self.profile.tm_class)

    def emit_method(self, m, out, indent, as_trait_method=False, as_free_fn=False):
        name = m["name"]
        self.current_method = name
        kind = self.method_kind(m)
        self.fn_kind = kind
        self.current_method_ret = m["type"]
        self.current_ret_nullable = self.type_nullable(m["type"]) or fkey(m) in self.nullable_returns
        self.nullable_locals = self.scan_nullable_locals(m["body"])
        self.label_counter = 0
        self.tmp_counter = 0
        scope = Scope()
        params = []
        for (ty, pname, pnull) in self.method_sig(m):
            rname = snake(pname)
            if self.is_list(ty):
                scope.put(pname, Var(rname, ty, False, byref=True))
                params.append(f"{rname}: &mut {self.rust_type(ty, False, m['line'])}")
                continue
            scope.put(pname, Var(rname, ty, pnull))
            params.append(f"mut {rname}: {self.rust_type(ty, pnull, m['line'])}")
        rt = self.rust_type(m["type"], self.current_ret_nullable, m["line"])
        if kind == "raised":
            ret = f"std::result::Result<{rt}, Raised>"
        elif kind == "result":
            ret = f"Result<{rt}>"
        else:
            ret = rt
        rname = self.rust_method_name(m)
        pad = "    " * indent
        doc = self.method_doc(m)
        for d in doc:
            out.append(pad + d)
        if self.has_unreachable_tail(m):
            out.append(pad + "#[allow(unreachable_code)]")
        out.append(pad + "#[allow(unused_mut, unused_variables, unused_parens, unused_assignments, unused_labels, clippy::all)]")
        if self.mode == "tm":
            vis = "pub " if ("public" in m["mods"] or "protected" in m["mods"]) else ("pub(crate) " if not ("private" in m["mods"]) else "")
            recv = "" if "static" in m["mods"] else ("&mut self" + (", " if params else ""))
            sig = f"{vis}fn {rname}({recv}{', '.join(params)})"
        elif as_trait_method:
            sig = f"fn {rname}(&mut self{', ' if params else ''}{', '.join(params)})"
        else:
            vis = "pub(crate) " if as_free_fn else ""
            sig = f"{vis}fn {rname}<P: {self.profile.ops_trait} + ?Sized>(this: &mut P{', ' if params else ''}{', '.join(params)})"
        if rt == "()" and kind == "plain":
            out.append(pad + sig + " {")
        else:
            out.append(pad + sig + f" -> {ret} {{")
        if as_trait_method:
            out.append(pad + "    let this = self;")
        body_out = []
        self.emit_block(m["body"], scope, body_out, indent + 1)
        out.extend(body_out)
        # the implicit return of a void method
        if m["type"] == T_VOID and kind in ("result", "raised"):
            if not (m["body"][1] and m["body"][1][-1][0] in ("return", "throw")):
                out.append(pad + "    Ok(())")
        out.append(pad + "}")
        out.append("")

    def has_unreachable_tail(self, m):
        b = m["body"][1]
        return bool(b) and b[-1][0] == "throw" and b[-1][1][0] == "new" and b[-1][1][1].name == "Error"

    def method_doc(self, m):
        name = m["name"]
        params = ", ".join(str(t) for (t, _) in m["params"])
        return [f"/// `{name}({params})` of the generated Java."]


# ==========================================================================
# helpers
# ==========================================================================


def fkey(m) -> str:
    """A member's key in the fallible set: name, arity, first parameter type."""
    first = m["params"][0][0].name if m["params"] else ""
    return f"{m['name']}/{len(m['params'])}/{first}"


def recv(code: str, ty: JType) -> str:
    """`code` as the receiver of a method call: a literal gets its type."""
    suffix = {"int": "i32", "long": "i64", "char": "u16"}.get(ty.name, "")
    if re.fullmatch(r"-?[0-9]+", code):
        return f"({code}{suffix})" if code.startswith("-") else f"{code}{suffix}"
    if re.fullmatch(r"-?[0-9]+(i32|i64|u16)", code):
        return f"({code})" if code.startswith("-") else code
    return wrap(code)


def wrap(code: str) -> str:
    if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_:.()]*", code) or re.fullmatch(r"-?[0-9]+(i32|i64|u64 as i64|u32 as i32)?", code):
        return code
    if code.startswith("(") and code.endswith(")") and balanced(code[1:-1]):
        return code
    return f"({code})"


def balanced(s):
    d = 0
    for c in s:
        if c == "(":
            d += 1
        elif c == ")":
            d -= 1
            if d < 0:
                return False
    return d == 0


def promote(t: JType) -> JType:
    if t.name in ("char", "byte", "short"):
        return T_INT
    return t


def binary_numeric(a: JType, b: JType) -> JType:
    a, b = promote(a), promote(b)
    order = ["int", "long", "float", "double"]
    if a.name not in order or b.name not in order:
        raise TranslationError(f"numeric operator on {a} and {b}")
    return a if order.index(a.name) >= order.index(b.name) else b


def cast_num(code: str, frm: JType, to: JType) -> str:
    if frm == to:
        return code
    rt = {"int": "i32", "long": "i64", "char": "u16", "float": "f32", "double": "f64", "byte": "i8", "short": "i16"}
    if to.name == "boolean" or frm.name == "boolean":
        if frm == to:
            return code
        raise TranslationError(f"cast between {frm} and {to}")
    if frm.name not in rt or to.name not in rt:
        raise TranslationError(f"cast from {frm} to {to}")
    if frm.name == "char" and to.name == "int":
        return f"i32::from({code})"
    if frm.name == "int" and to.name == "long":
        return f"i64::from({code})"
    if frm.name == "char" and to.name == "long":
        return f"i64::from({code})"
    return f"({wrap(code)} as {rt[to.name]})"


def int_lit(v: int, ty: JType) -> str:
    if ty.name == "long":
        if v > 0x7FFFFFFFFFFFFFFF:
            return f"(0x{v:x}u64 as i64)"
        return f"{v}i64" if v < 1 << 32 else f"0x{v:x}i64"
    if v > 0x7FFFFFFF:
        return f"(0x{v:x}u32 as i32)"
    return str(v)


def rust_str(units: list) -> str:
    s = "".join(chr(u) for u in units)
    out = ['"']
    for ch in s:
        o = ord(ch)
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\t":
            out.append("\\t")
        elif ch == "\r":
            out.append("\\r")
        elif o < 0x20 or o == 0x7F:
            out.append(f"\\u{{{o:x}}}")
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


LICENSE = """/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
"""


def generated_banner(profile, java_file):
    return (f"// @generated by {SCRIPT} from {JAVA_ROOT}/{profile.java_pkg}/{java_file}\n"
            f"// (Lucene 10.5.0, JavaCC 7 output). Do not edit.\n\n")


# ==========================================================================
# Constants
# ==========================================================================


def translate_constants(profile, lucene):
    java_file = f"{profile.parser_class}Constants.java"
    path = lucene / JAVA_ROOT / profile.java_pkg / java_file
    where = str(path.relative_to(lucene))
    toks = lex(path.read_text(encoding="utf-8"), where)
    types = Parser(toks, where).compilation_unit()
    if len(types) != 1 or types[0]["kind"] != "interface":
        fail(where, "expected one interface")
    iface = types[0]
    consts = []
    token_image = None
    for m in iface["members"]:
        if m["kind"] != "field":
            fail(where, f"unexpected member {m['kind']}")
        for (n, init) in m["decls"]:
            if m["type"] == T_INT and init[0] == "lit" and init[1] == "int":
                consts.append((n, init[2]))
            elif m["type"] == JType("String", (), 1) and n == "tokenImage":
                token_image = [e[2] for e in init[1]]
            else:
                fail(where, f"unexpected constant {n}")
    trait = iface["name"]
    lines = [LICENSE, generated_banner(profile, java_file)]
    lines.append(f"//! The port of `{java_file}`, of the Java package `{profile.java_package}`:\n")
    lines.append(f"//! the token kinds and lexical states of the grammar, and the images of\n//! the tokens.\n\n")
    names = {}
    for (n, v) in consts:
        rn = upper_snake(n) if not n.startswith("_") else "_" + upper_snake(n[1:]) if n[1:].isupper() else n
        rn = n if n.isupper() or n.startswith("_") else upper_snake(n)
        names[n] = rn
        lines.append(f"/// `{n}`.\n#[allow(non_upper_case_globals)]\npub const {rn}: i32 = {v};\n")
    lines.append(f"/// `tokenImage`: the literal values of the tokens, by kind.\n")
    lines.append(f"pub const TOKEN_IMAGE: [&str; {len(token_image)}] = [\n")
    for units in token_image:
        lines.append(f"    {rust_str(units)},\n")
    lines.append("];\n\n")
    lines.append(f"/// Token literal values and constants.\n///\n/// Equivalent to the interface `{profile.java_package}.{trait}`, generated by\n")
    lines.append(f"/// JavaCC: its fields are the constants of this module, which the trait gives\n/// the types that implement it as associated constants, as the Java interface\n/// gives them to its implementing classes.\n")
    lines.append(f"pub trait {trait} {{\n")
    for (n, v) in consts:
        lines.append(f"    /// `{n}`.\n    #[allow(non_upper_case_globals)]\n    const {names[n]}: i32 = {names[n]};\n")
    lines.append(f"    /// `tokenImage`.\n    const TOKEN_IMAGE: [&'static str; {len(token_image)}] = TOKEN_IMAGE;\n")
    lines.append("}\n")
    return "".join(lines), names


# ==========================================================================
# Token manager
# ==========================================================================


def translate_token_manager(profile, lucene, constants):
    java_file = f"{profile.parser_class}TokenManager.java"
    path = lucene / JAVA_ROOT / profile.java_pkg / java_file
    where = str(path.relative_to(lucene))
    toks = lex(path.read_text(encoding="utf-8"), where)
    types = Parser(toks, where).compilation_unit()
    cls = types[0]
    em = Emitter(where, profile, cls, "tm")
    em.constants = constants
    em.try_stack = []
    em.fallible = em.compute_fallible()
    profile.tm_fallible = em.fallible
    name = cls["name"]
    out = [LICENSE, generated_banner(profile, java_file)]
    out.append(f"//! The port of `{java_file}`, of the Java package `{profile.java_package}`:\n")
    out.append("//! the token manager JavaCC generates for the grammar.\n\n")
    out.append("#![allow(clippy::all, unused_parens, unused_mut, unused_labels, dead_code, non_snake_case)]\n\n")
    out.append("use rucene::error::{LuceneError, Result};\n\n")
    out.append("use crate::charstream::char_stream::CharStream;\n")
    out.append(f"use super::{snake(profile.parser_class)}_constants::{{self as qpc, {profile.parser_class}Constants}};\n")
    out.append("use super::token::{Token, TokenRef};\n")
    out.append("use super::token_mgr_error::TokenMgrError;\n\n")
    # statics
    statics = []
    fields = []
    for m in cls["members"]:
        if m["kind"] == "field":
            for (n, init) in m["decls"]:
                if "static" in m["mods"]:
                    statics.append((m, n, init))
                else:
                    fields.append((m, n, init))
        elif m["kind"] not in ("method", "ctor", "empty"):
            fail(f"{where}:{m['line']}", f"unexpected member {m['kind']}")
    for (m, n, init) in statics:
        ty = m["type"]
        rn = upper_snake(n)
        vis = "pub " if "public" in m["mods"] else ""
        if ty.dims != 1 or init is None or init[0] != "arrayinit":
            fail(f"{where}:{m['line']}", f"unexpected static {n}")
        if ty.name == "String":
            nullable = any(e[0] == "lit" and e[1] == "null" for e in init[1])
            em.static_string_nullable = nullable
            vals = em.literal_array(ty, init, m["line"])
            em.static_string_nullable = False
            et = "Option<&str>" if nullable else "&str"
            out.append(f"/// `{n}`.\n{vis}static {rn}: [{et}; {len(vals)}] = [{', '.join(vals)}];\n\n")
        else:
            vals = em.literal_array(ty, init, m["line"])
            rt = em.rust_type(ty.elem(), False, m["line"])
            out.append(f"/// `{n}`.\n{vis}static {rn}: [{rt}; {len(vals)}] = [{', '.join(vals)}];\n\n")
    # the struct
    out.append(f"/// Token manager.\n///\n/// Equivalent to `{profile.java_package}.{name}`, generated by JavaCC.\n")
    out.append("#[derive(Debug)]\n")
    out.append(f"pub struct {name} {{\n")
    struct_fields = []
    for (m, n, init) in fields:
        ty = m["type"]
        if n == "image" and init is not None and init[0] == "name" and init[1] == "jjimage":
            em.fields.pop("image")
            continue
        rn = snake(n)
        nullable = em.field_nullable(n)
        rt = em.rust_type(ty, nullable, m["line"])
        vis = "pub " if ("public" in m["mods"] or "protected" in m["mods"]) else "pub(crate) "
        out.append(f"    /// `{n}`.\n    {vis}{rn}: {rt},\n")
        struct_fields.append((m, n, init, rn, ty, nullable))
    out.append("}\n\n")
    out.append(f"impl {profile.parser_class}Constants for {name} {{}}\n\n")
    out.append(f"impl {name} {{\n")
    # constructors
    for m in cls["members"]:
        if m["kind"] != "ctor":
            continue
        ps = m["params"]
        if len(ps) == 1:
            rname = "new"
        elif len(ps) == 2:
            rname = "with_lex_state"
        else:
            fail(f"{where}:{m['line']}", "an unexpected constructor")
        em.fn_kind = "result" if len(ps) == 2 else "plain"
        em.current_method_ret = T_VOID
        em.current_ret_nullable = False
        em.nullable_locals = set()
        scope = Scope()
        params = []
        for (ty, pn) in ps:
            scope.put(pn, Var(snake(pn), ty, False))
            params.append(f"{snake(pn)}: {em.rust_type(ty, False, m['line'])}")
        doc = f"    /// `{name}({', '.join(str(t) for t, _ in ps)})`.\n"
        ret = f"Result<Self>" if em.fn_kind == "result" else "Self"
        out.append(doc)
        out.append("    #[allow(unused_mut, clippy::all)]\n")
        out.append(f"    pub fn {rname}({', '.join(params)}) -> {ret} {{\n")
        # the field initializers, then the body on a `self` value
        inits = []
        for (fm, n, init, rn, ty, nullable) in struct_fields:
            if init is None:
                if ty.name == "CharStream":
                    inits.append(f"{rn}: input_stream_init")
                    continue
                inits.append(f"{rn}: {em.default_value(ty, nullable, fm['line'])}")
            elif init[0] == "newarray":
                p, c, t, n2 = em.emit_expr(init, Scope())
                inits.append(f"{rn}: {c}")
            elif init[0] == "new" and init[1].name == "StringBuilder":
                inits.append(f"{rn}: String::new()")
            else:
                p, c, t, n2 = em.emit_expr(init, Scope())
                inits.append(f"{rn}: {c}")
        stream_param = [snake(pn) for (ty, pn) in ps if ty.name == "CharStream"][0]
        out.append(f"        let input_stream_init = {stream_param};\n")
        out.append(f"        let mut this_ = Self {{ {', '.join(inits)} }};\n")
        # translate the body against `this_` (assignments to input_stream
        # use the moved parameter)
        body_out = []
        em_scope = Scope()
        for (ty, pn) in ps:
            if ty.name != "CharStream":
                em_scope.put(pn, Var(snake(pn), ty, False))
        em.ctor_mode = True
        for st in m["body"][1]:
            if st[0] == "expr" and st[1][0] == "assign" and st[1][2] == ("name", "input_stream", st[1][2][-1]) if False else False:
                continue
            if st[0] == "expr" and st[1][0] == "assign" and st[1][2][0] == "name" and st[1][2][1] == "input_stream":
                continue
            if st[0] == "expr" and st[1][0] == "call" and st[1][2] == "ReInit" and len(st[1][3]) == 1:
                # `ReInit(stream)` in the two-argument constructor: the stream
                # is already in place
                body_out.append("        this_.re_init_fields();")
                continue
            em.emit_stmt(st, em_scope, body_out, 2)
        em.ctor_mode = False
        out.extend(x.replace("self.", "this_.") + "\n" for x in body_out)
        out.append("        Ok(this_)\n" if em.fn_kind == "result" else "        this_\n")
        out.append("    }\n\n")
    # ReInit's field part, for the constructor above
    out.append("    /// The part of `ReInit(CharStream)` after the stream is set.\n")
    out.append("    fn re_init_fields(&mut self) {\n")
    out.append("        self.jjmatched_pos = 0;\n        self.jjnew_state_cnt = 0;\n        self.cur_lex_state = self.default_lex_state;\n        self.re_init_rounds();\n")
    out.append("    }\n\n")
    for m in cls["members"]:
        if m["kind"] != "method":
            continue
        body = []
        em.emit_method(m, body, 1)
        out.extend(x + "\n" for x in body)
    out.append("}\n")
    text = "".join(out)
    if "qpc::" not in text.split("impl " + name + " {", 1)[1]:
        text = text.replace("{self as qpc, " + profile.parser_class + "Constants}", profile.parser_class + "Constants")
    return text


# ==========================================================================
# Templates (Token, TokenMgrError, ParseException)
# ==========================================================================


def check_template(profile, lucene, java_file):
    path = lucene / JAVA_ROOT / profile.java_pkg / java_file
    where = str(path.relative_to(lucene))
    text = path.read_text(encoding="utf-8")
    text = re.sub(r"^package [^;]+;", "", text, count=1, flags=re.M)
    toks = lex(text, where)
    h = token_hash(toks)
    key = java_file[:-5]
    if key == "ParseException" and profile.parse_exception == "flexible":
        key = "ParseException:flexible"
    if PRINT_HASHES:
        print(f"template {profile.name} {key} {h}")
        return
    if TEMPLATE_SHA[key] and TEMPLATE_SHA[key] != h:
        fail(where, f"not the JavaCC 7 template this translator knows (SHA-256 {h})")


def template_token(profile):
    pkg = profile.java_package
    return LICENSE + f"""// @generated by {SCRIPT} from the JavaCC 7 template of
// {JAVA_ROOT}/{profile.java_pkg}/Token.java (Lucene 10.5.0). Do not edit.

//! The port of `Token.java`, of the Java package `{pkg}`.

use std::fmt;

use crate::javacc::Shared;

/// A reference to a [`Token`]: Java's object reference, shared by the token
/// list, the parser and the actions that rewrite an image.
pub type TokenRef = Shared<Token>;

/// Describes the input token stream.
///
/// Equivalent to `{pkg}.Token`, generated by JavaCC. Java's tokens form a
/// list through `next`, shared and rewritten in place; here a token is reached
/// through a [`TokenRef`].
#[derive(Clone, Default)]
pub struct Token {{
    /// An integer that describes the kind of this token. This numbering
    /// system is determined by JavaCCParser, and a table of these numbers is
    /// stored in the file `...Constants.java`.
    pub kind: i32,
    /// The line number of the first character of this token.
    pub begin_line: i32,
    /// The column number of the first character of this token.
    pub begin_column: i32,
    /// The line number of the last character of this token.
    pub end_line: i32,
    /// The column number of the last character of this token.
    pub end_column: i32,
    /// The string image of the token; `None` for Java's `null`.
    pub image: Option<String>,
    /// A reference to the next regular (non-special) token from the input
    /// stream. If this is the last token from the input stream, or if the
    /// token manager has not read tokens beyond this one, this is `None`.
    pub next: Option<TokenRef>,
    /// The special token before this token, if any: the token manager's
    /// special (`SPECIAL_TOKEN`) tokens, linked backwards.
    pub special_token: Option<TokenRef>,
}}

impl fmt::Debug for Token {{
    /// The kind and image, not the linked tokens.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{
        f.debug_struct("Token")
            .field("kind", &self.kind)
            .field("image", &self.image)
            .finish_non_exhaustive()
    }}
}}

impl Token {{
    /// No-argument constructor.
    ///
    /// Equivalent to `new Token()`.
    pub fn new() -> Self {{
        Self::default()
    }}

    /// Constructs a new token for the specified kind, with a `null` image.
    ///
    /// Equivalent to `new Token(int)`.
    pub fn with_kind(kind: i32) -> Self {{
        Self::with_kind_and_image(kind, None)
    }}

    /// Constructs a new token for the specified kind and image.
    ///
    /// Equivalent to `new Token(int, String)`.
    pub fn with_kind_and_image(kind: i32, image: Option<String>) -> Self {{
        Self {{
            kind,
            image,
            ..Self::default()
        }}
    }}

    /// An optional attribute value of the token. Tokens which are not used
    /// as syntactic sugar often contain meaningful values used later by the
    /// compiler or interpreter; this attribute is not used by JavaCC and is
    /// always `None` here.
    ///
    /// Equivalent to `getValue()`.
    pub fn get_value(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {{
        None
    }}

    /// Returns the image.
    ///
    /// Equivalent to `toString()`, which returns `image` (`None` for `null`).
    pub fn to_java_string(&self) -> Option<String> {{
        self.image.clone()
    }}

    /// Returns a new token object, by default. However, if you want, you can
    /// create and return subclass objects based on the value of `of_kind`.
    ///
    /// Equivalent to `Token.newToken(int, String)`.
    pub fn new_token(of_kind: i32, image: Option<String>) -> TokenRef {{
        Shared::new(Self::with_kind_and_image(of_kind, image))
    }}

    /// Equivalent to `Token.newToken(int)`.
    pub fn new_token_kind(of_kind: i32) -> TokenRef {{
        Self::new_token(of_kind, None)
    }}
}}

impl fmt::Display for Token {{
    /// The image, as Java's string conversion of `toString()` prints it
    /// (`null` for none).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{
        f.write_str(self.image.as_deref().unwrap_or("null"))
    }}
}}
"""


def template_token_mgr_error(profile):
    pkg = profile.java_package
    return LICENSE + f"""// @generated by {SCRIPT} from the JavaCC 7 template of
// {JAVA_ROOT}/{profile.java_pkg}/TokenMgrError.java (Lucene 10.5.0). Do not edit.

//! The port of `TokenMgrError.java`, of the Java package `{pkg}`.

use std::sync::Arc;

use rucene::error::{{JavaThrowable, LuceneError, LuceneErrorKind}};

/// Lexical error occurred.
pub const LEXICAL_ERROR: i32 = 0;
/// An attempt was made to create a second instance of a static token
/// manager.
pub const STATIC_LEXER_ERROR: i32 = 1;
/// Tried to change to an invalid lexical state.
pub const INVALID_LEXICAL_STATE: i32 = 2;
/// Detected (and bailed out of) an infinite loop in the token manager.
pub const LOOP_DETECTED: i32 = 3;

/// The Java binary class name.
const CLASS: &str = "{pkg}.TokenMgrError";

/// Token Manager Error: a `java.lang.Error` the token manager throws.
///
/// Equivalent to `{pkg}.TokenMgrError`, generated by JavaCC; it is thrown as
/// a [`LuceneError`] of kind [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone)]
pub struct TokenMgrError {{
    message: Option<String>,
    /// Indicates the reason why the exception is thrown. It will have one of
    /// the above 4 values.
    error_code: i32,
}}

impl Default for TokenMgrError {{
    fn default() -> Self {{
        Self::new()
    }}
}}

impl TokenMgrError {{
    /// No arg constructor.
    ///
    /// Equivalent to `new TokenMgrError()`.
    pub fn new() -> Self {{
        Self {{
            message: None,
            error_code: 0,
        }}
    }}

    /// Constructor with message and reason.
    ///
    /// Equivalent to `new TokenMgrError(String, int)`.
    pub fn with_message(message: impl Into<String>, reason: i32) -> Self {{
        Self {{
            message: Some(message.into()),
            error_code: reason,
        }}
    }}

    /// Full constructor.
    ///
    /// Equivalent to `new TokenMgrError(boolean, int, int, int, String, int,
    /// int)`.
    #[allow(clippy::too_many_arguments)]
    pub fn lexical(
        eof_seen: bool,
        lex_state: i32,
        error_line: i32,
        error_column: i32,
        error_after: Option<&str>,
        cur_char: i32,
        reason: i32,
    ) -> Self {{
        Self::with_message(
            Self::lexical_err(eof_seen, lex_state, error_line, error_column, error_after, cur_char),
            reason,
        )
    }}

    /// Replaces unprintable characters by their escaped (or unicode escaped)
    /// equivalents in the given string.
    ///
    /// Equivalent to `addEscapes(String)`, over the UTF-16 units of `str`.
    pub fn add_escapes(str: &str) -> String {{
        crate::javacc::add_escapes(str)
    }}

    /// Returns a detailed message for the error when it is thrown by the
    /// token manager to indicate a lexical error.
    ///
    /// Equivalent to `LexicalErr(boolean, int, int, int, String, int)`: the
    /// character is printed as JavaCC 7 prints its `int` parameter, the
    /// decimal digits of `String.valueOf(int)`.
    pub fn lexical_err(
        eof_seen: bool,
        lex_state: i32,
        error_line: i32,
        error_column: i32,
        error_after: Option<&str>,
        cur_char: i32,
    ) -> String {{
        let encountered = if eof_seen {{
            "<EOF>".to_string()
        }} else {{
            format!("'{{}}' ({{}}),", Self::add_escapes(&cur_char.to_string()), cur_char)
        }};
        let after = match error_after {{
            None => String::new(),
            Some("") => String::new(),
            Some(a) => format!(" after prefix \\"{{}}\\"", Self::add_escapes(a)),
        }};
        let state = if lex_state == 0 {{
            String::new()
        }} else {{
            format!(" (in lexical state {{lex_state}})")
        }};
        format!(
            "Lexical error at line {{error_line}}, column {{error_column}}.  Encountered: {{encountered}}{{after}}{{state}}"
        )
    }}

    /// Equivalent to `getMessage()`.
    pub fn get_message(&self) -> Option<&str> {{
        self.message.as_deref()
    }}

    /// The `errorCode` field.
    pub fn error_code(&self) -> i32 {{
        self.error_code
    }}

    /// This error as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {{
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }}

    /// The `TokenMgrError` `error` stands for, when it is one.
    pub fn of(error: &LuceneError) -> Option<&TokenMgrError> {{
        match error.kind() {{
            LuceneErrorKind::Throwable(t) => t.as_any().downcast_ref(),
            _ => None,
        }}
    }}
}}

impl JavaThrowable for TokenMgrError {{
    fn java_class(&self) -> &'static str {{
        CLASS
    }}

    fn get_message(&self) -> Option<&str> {{
        self.message.as_deref()
    }}

    fn as_any(&self) -> &dyn std::any::Any {{
        self
    }}
}}
"""


def template_parse_exception(profile):
    pkg = profile.java_package
    return LICENSE + f"""// @generated by {SCRIPT} from the JavaCC 7 template of
// {JAVA_ROOT}/{profile.java_pkg}/ParseException.java (Lucene 10.5.0). Do not edit.

//! The port of `ParseException.java`, of the Java package `{pkg}`.

use std::sync::Arc;

use rucene::error::{{JavaThrowable, LuceneError, LuceneErrorKind, Result}};

use super::token::TokenRef;

/// The Java binary class name.
const CLASS: &str = "{pkg}.ParseException";

/// The end of line string for this machine: `System.getProperty(
/// "line.separator", "\\n")`, the JVM's platform default.
pub fn eol() -> &'static str {{
    if cfg!(windows) {{
        "\\r\\n"
    }} else {{
        "\\n"
    }}
}}

/// This exception is thrown when parse errors are encountered. You can
/// explicitly create objects of this exception type by calling the method
/// `generateParseException` in the generated parser.
///
/// Equivalent to `{pkg}.ParseException`, generated by JavaCC (a checked
/// `Exception`); it is thrown as a [`LuceneError`] of kind
/// [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone, Default)]
pub struct ParseException {{
    message: Option<String>,
    cause: Option<Arc<LuceneError>>,
    /// This is the last token that has been consumed successfully. If this
    /// object has been created due to a parse error, the token following
    /// this token will (therefore) be the first error token.
    pub current_token: Option<TokenRef>,
    /// Each entry in this array is an array of integers. Each array of
    /// integers represents a sequence of tokens (by their ordinal values)
    /// that is expected at this point of the parse.
    pub expected_token_sequences: Option<Vec<Vec<i32>>>,
    /// This is a reference to the `tokenImage` array of the generated parser
    /// within which the parse error occurred.
    pub token_image: Option<Vec<String>>,
}}

impl ParseException {{
    /// This constructor is used by the method `generateParseException` in
    /// the generated parser. Calling this constructor generates a new object
    /// of this type with the fields `current_token`,
    /// `expected_token_sequences` and `token_image` set.
    ///
    /// Equivalent to `new ParseException(Token, int[][], String[])`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `initialise` reaching the end of the
    /// token list.
    pub fn with_tokens(
        current_token_val: TokenRef,
        expected_token_sequences_val: Vec<Vec<i32>>,
        token_image_val: &[&str],
    ) -> Result<Self> {{
        let token_image: Vec<String> = token_image_val.iter().map(|s| s.to_string()).collect();
        let message =
            Self::initialise(&current_token_val, &expected_token_sequences_val, &token_image)?;
        Ok(Self {{
            message: Some(message),
            cause: None,
            current_token: Some(current_token_val),
            expected_token_sequences: Some(expected_token_sequences_val),
            token_image: Some(token_image),
        }})
    }}

    /// The following constructors are for use by you for whatever purpose
    /// you can think of. Constructing the exception in this manner makes the
    /// exception behave in the normal way - i.e., as documented in the class
    /// `Throwable`. The fields `current_token`, `expected_token_sequences`
    /// and `token_image` do not contain relevant information. The JavaCC
    /// generated code does not use these constructors.
    ///
    /// Equivalent to `new ParseException()`.
    pub fn new() -> Self {{
        Self::default()
    }}

    /// Constructor with message.
    ///
    /// Equivalent to `new ParseException(String)`.
    pub fn with_message(message: impl Into<String>) -> Self {{
        Self {{
            message: Some(message.into()),
            ..Self::default()
        }}
    }}

    /// Equivalent to `Throwable.initCause(Throwable)`, which Lucene calls on
    /// the exceptions it rethrows.
    pub fn init_cause(&mut self, cause: LuceneError) {{
        self.cause = Some(Arc::new(cause));
    }}

    /// Equivalent to `getMessage()`.
    pub fn get_message(&self) -> Option<&str> {{
        self.message.as_deref()
    }}

    /// It uses `current_token` and `expected_token_sequences` to generate a
    /// parse error message and returns it. If this object has been created
    /// due to a parse error, and you do not catch it (it gets thrown from the
    /// parser) the correct error message gets displayed.
    ///
    /// Equivalent to `initialise(Token, int[][], String[])`.
    fn initialise(
        current_token: &TokenRef,
        expected_token_sequences: &[Vec<i32>],
        token_image: &[String],
    ) -> Result<String> {{
        let eol = eol();
        let mut expected = String::new();
        let mut max_size = 0;
        for seq in expected_token_sequences {{
            if max_size < seq.len() {{
                max_size = seq.len();
            }}
            for &k in seq {{
                expected.push_str(&token_image[k as usize]);
                expected.push(' ');
            }}
            if seq[seq.len() - 1] != 0 {{
                expected.push_str("...");
            }}
            expected.push_str(eol);
            expected.push_str("    ");
        }}
        let mut retval = String::from("Encountered \\"");
        let mut tok = current_token.lock().next.clone();
        for i in 0..max_size {{
            if i != 0 {{
                retval.push(' ');
            }}
            let t = crate::javacc::nn(tok.as_ref())?.clone();
            let (kind, image, next) = {{
                let g = t.lock();
                (g.kind, g.image.clone(), g.next.clone())
            }};
            if kind == 0 {{
                retval.push_str(&token_image[0]);
                break;
            }}
            retval.push(' ');
            retval.push_str(&token_image[kind as usize]);
            retval.push_str(" \\"");
            retval.push_str(&add_escapes(crate::javacc::nn_str(image.as_deref())?));
            retval.push_str(" \\"");
            tok = next;
        }}
        let next = current_token.lock().next.clone();
        if let Some(next) = next {{
            let g = next.lock();
            retval.push_str(&format!(
                "\\" at line {{}}, column {{}}",
                g.begin_line, g.begin_column
            ));
        }}
        retval.push('.');
        retval.push_str(eol);
        if expected_token_sequences.is_empty() {{
            // Nothing to add here
        }} else {{
            if expected_token_sequences.len() == 1 {{
                retval.push_str("Was expecting:");
            }} else {{
                retval.push_str("Was expecting one of:");
            }}
            retval.push_str(eol);
            retval.push_str("    ");
            retval.push_str(&expected);
        }}
        Ok(retval)
    }}

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {{
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }}

    /// The `ParseException` `error` stands for, when it is one.
    pub fn of(error: &LuceneError) -> Option<&ParseException> {{
        match error.kind() {{
            LuceneErrorKind::Throwable(t) => t.as_any().downcast_ref(),
            _ => None,
        }}
    }}
}}

/// Used to convert raw characters to their escaped version when these raw
/// versions cannot be used as part of an ASCII string literal.
///
/// Equivalent to the package-private `ParseException.add_escapes(String)`,
/// over the UTF-16 units of `str`.
pub fn add_escapes(str: &str) -> String {{
    crate::javacc::add_escapes(str)
}}

impl JavaThrowable for ParseException {{
    fn java_class(&self) -> &'static str {{
        CLASS
    }}

    fn get_message(&self) -> Option<&str> {{
        self.message.as_deref()
    }}

    fn get_cause(&self) -> Option<&Arc<LuceneError>> {{
        self.cause.as_ref()
    }}

    fn as_any(&self) -> &dyn std::any::Any {{
        self
    }}
}}
"""


# ==========================================================================
# Parser
# ==========================================================================


def template_parse_exception_flexible(profile):
    """`ParseException.java` of the flexible standard parser: the JavaCC 7
    template with Lucene's edits (a `QueryNodeParseException` whose message
    is the NLS `INVALID_SYNTAX` message), pinned by its own SHA-256."""
    pkg = profile.java_package
    return LICENSE + f"""// @generated by {SCRIPT} from the JavaCC 7 template of
// {JAVA_ROOT}/{profile.java_pkg}/ParseException.java (Lucene 10.5.0, as
// Lucene edits it for the flexible parser). Do not edit.

//! The port of `ParseException.java`, of the Java package `{pkg}`.

use std::any::Any;
use std::sync::{{Arc, Once}};

use rucene::error::{{JavaThrowable, LuceneError, LuceneErrorKind, Result}};

use super::token::TokenRef;
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::query_node_parse_exception::QueryNodeParseException;
use crate::flexible::messages::message::Message;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::messages::nls_exception::NLSException;
use crate::jdk::object::JavaObject;

/// The Java binary class name.
const CLASS: &str = "{pkg}.ParseException";

/// The end of line string for this machine: the protected static `EOL`,
/// `System.getProperty("line.separator", "\\n")`, the JVM's platform
/// default.
pub fn eol() -> &'static str {{
    if cfg!(windows) {{
        "\\r\\n"
    }} else {{
        "\\n"
    }}
}}

/// Registers the class with [`QueryNodeParseException`], so that
/// `catch (QueryNodeParseException e)` and `catch (QueryNodeException e)`
/// ([`QueryNodeParseException::of`]) see through it.
fn register() {{
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {{
        QueryNodeParseException::register_subclass(|any| {{
            any.downcast_ref::<ParseException>().map(|e| &e.base)
        }});
    }});
}}

/// This exception is thrown when parse errors are encountered. You can
/// explicitly create objects of this exception type by calling the method
/// `generateParseException` in the generated parser.
///
/// Equivalent to `{pkg}.ParseException`, generated by JavaCC and edited by
/// Lucene: a `QueryNodeParseException` (embedded, reached through
/// [`query_node_parse_exception`](Self::query_node_parse_exception)); it is
/// thrown as a [`LuceneError`] of kind [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone)]
pub struct ParseException {{
    base: QueryNodeParseException,
    /// This is the last token that has been consumed successfully. If this
    /// object has been created due to a parse error, the token following
    /// this token will (therefore) be the first error token.
    pub current_token: Option<TokenRef>,
    /// Each entry in this array is an array of integers. Each array of
    /// integers represents a sequence of tokens (by their ordinal values)
    /// that is expected at this point of the parse.
    pub expected_token_sequences: Option<Vec<Vec<i32>>>,
    /// This is a reference to the `tokenImage` array of the generated parser
    /// within which the parse error occurred.
    pub token_image: Option<Vec<String>>,
}}

impl ParseException {{
    /// The exception over its `QueryNodeParseException` part.
    fn of_base(mut base: QueryNodeParseException) -> Self {{
        register();
        base.query_node_exception_mut().set_java_class(CLASS);
        Self {{
            base,
            current_token: None,
            expected_token_sequences: None,
            token_image: None,
        }}
    }}

    /// This constructor is used by the method `generateParseException` in
    /// the generated parser. Calling this constructor generates a new object
    /// of this type with the fields `current_token`,
    /// `expected_token_sequences` and `token_image` set.
    ///
    /// Equivalent to `new ParseException(Token, int[][], String[])`: the
    /// message is `INVALID_SYNTAX` with the text `initialise` builds.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `initialise` reaching the end of the
    /// token list.
    pub fn with_tokens(
        current_token_val: TokenRef,
        expected_token_sequences_val: Vec<Vec<i32>>,
        token_image_val: &[&str],
    ) -> Result<Self> {{
        let token_image: Vec<String> = token_image_val.iter().map(|s| s.to_string()).collect();
        let message =
            Self::initialise(&current_token_val, &expected_token_sequences_val, &token_image)?;
        let mut e = Self::of_base(QueryNodeParseException::new(Arc::new(MessageImpl::with_args(
            QueryParserMessages::INVALID_SYNTAX.get(),
            vec![JavaObject::from(message)],
        ))));
        e.current_token = Some(current_token_val);
        e.expected_token_sequences = Some(expected_token_sequences_val);
        e.token_image = Some(token_image);
        Ok(e)
    }}

    /// The following constructors are for use by you for whatever purpose
    /// you can think of. Constructing the exception in this manner makes the
    /// exception behave in the normal way - i.e., as documented in the class
    /// `Throwable`. The fields `errorToken`, `expected_token_sequences` and
    /// `token_image` do not contain relevant information. The JavaCC
    /// generated code does not use these constructors.
    ///
    /// Equivalent to `new ParseException()`: the message is `INVALID_SYNTAX`
    /// with the text `"Error"`.
    pub fn new() -> Self {{
        Self::of_base(QueryNodeParseException::new(Arc::new(MessageImpl::with_args(
            QueryParserMessages::INVALID_SYNTAX.get(),
            vec![JavaObject::from("Error")],
        ))))
    }}

    /// Constructor with message.
    ///
    /// Equivalent to `new ParseException(Message)`.
    pub fn with_message(message: Arc<dyn Message>) -> Self {{
        Self::of_base(QueryNodeParseException::new(message))
    }}

    /// The `QueryNodeParseException` this exception extends.
    pub fn query_node_parse_exception(&self) -> &QueryNodeParseException {{
        &self.base
    }}

    /// The `QueryNodeParseException` this exception extends, for writing
    /// its state (`setQuery`, `setNonLocalizedMessage`).
    pub fn query_node_parse_exception_mut(&mut self) -> &mut QueryNodeParseException {{
        &mut self.base
    }}

    /// It uses `current_token` and `expected_token_sequences` to generate a
    /// parse error message and returns it. If this object has been created
    /// due to a parse error, and you do not catch it (it gets thrown from the
    /// parser) the correct error message gets displayed.
    ///
    /// Equivalent to `initialise(Token, int[][], String[])`.
    fn initialise(
        current_token: &TokenRef,
        expected_token_sequences: &[Vec<i32>],
        token_image: &[String],
    ) -> Result<String> {{
        let eol = eol();
        let mut expected = String::new();
        let mut max_size = 0;
        for seq in expected_token_sequences {{
            if max_size < seq.len() {{
                max_size = seq.len();
            }}
            for &k in seq {{
                expected.push_str(&token_image[k as usize]);
                expected.push(' ');
            }}
            if seq[seq.len() - 1] != 0 {{
                expected.push_str("...");
            }}
            expected.push_str(eol);
            expected.push_str("    ");
        }}
        let mut retval = String::from("Encountered \\"");
        let mut tok = current_token.lock().next.clone();
        for i in 0..max_size {{
            if i != 0 {{
                retval.push(' ');
            }}
            let t = crate::javacc::nn(tok.as_ref())?.clone();
            let (kind, image, next) = {{
                let g = t.lock();
                (g.kind, g.image.clone(), g.next.clone())
            }};
            if kind == 0 {{
                retval.push_str(&token_image[0]);
                break;
            }}
            retval.push(' ');
            retval.push_str(&token_image[kind as usize]);
            retval.push_str(" \\"");
            retval.push_str(&add_escapes(crate::javacc::nn_str(image.as_deref())?));
            retval.push_str(" \\"");
            tok = next;
        }}
        let next = current_token.lock().next.clone();
        if let Some(next) = next {{
            let g = next.lock();
            retval.push_str(&format!(
                "\\" at line {{}}, column {{}}",
                g.begin_line, g.begin_column
            ));
        }}
        retval.push('.');
        retval.push_str(eol);
        if expected_token_sequences.is_empty() {{
            // Nothing to add here
        }} else {{
            if expected_token_sequences.len() == 1 {{
                retval.push_str("Was expecting:");
            }} else {{
                retval.push_str("Was expecting one of:");
            }}
            retval.push_str(eol);
            retval.push_str("    ");
            retval.push_str(&expected);
        }}
        Ok(retval)
    }}

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {{
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }}

    /// The `ParseException` `error` stands for, when it is one.
    pub fn of(error: &LuceneError) -> Option<&ParseException> {{
        match error.kind() {{
            LuceneErrorKind::Throwable(t) => t.as_any().downcast_ref(),
            _ => None,
        }}
    }}
}}

impl Default for ParseException {{
    fn default() -> Self {{
        Self::new()
    }}
}}

/// Used to convert raw characters to their escaped version when these raw
/// versions cannot be used as part of an ASCII string literal.
///
/// Equivalent to the package-private `ParseException.add_escapes(String)`,
/// over the UTF-16 units of `str`.
pub fn add_escapes(str: &str) -> String {{
    crate::javacc::add_escapes(str)
}}

impl NLSException for ParseException {{
    fn get_message_object(&self) -> Option<Arc<dyn Message>> {{
        self.base.get_message_object()
    }}
}}

impl JavaThrowable for ParseException {{
    fn java_class(&self) -> &'static str {{
        self.base.java_class()
    }}

    fn get_message(&self) -> Option<&str> {{
        self.base.get_message()
    }}

    fn get_cause(&self) -> Option<&Arc<LuceneError>> {{
        self.base.get_cause()
    }}

    fn as_any(&self) -> &dyn Any {{
        self
    }}
}}
"""


def translate_parser(profile, lucene, constants):
    java_file = f"{profile.parser_class}.java"
    path = lucene / JAVA_ROOT / profile.java_pkg / java_file
    where = str(path.relative_to(lucene))
    toks = lex(path.read_text(encoding="utf-8"), where)
    types = Parser(toks, where).compilation_unit()
    cls = types[0]
    # split the user members from the generated ones
    user_toks = []
    gen_members = []
    for m in cls["members"]:
        key = member_key(m, cls["name"])
        if key in profile.user_members:
            user_toks.extend(m["toks"])
        else:
            gen_members.append(m)
    found = {member_key(m, cls["name"]) for m in cls["members"]}
    for u in profile.user_members:
        if u not in found:
            fail(where, f"the user member {u} is missing")
    h = token_hash(user_toks + [Tok("eof", "", 0)])
    if PRINT_HASHES:
        print(f"user {profile.name} {h}")
        return {}
    if USER_SHA[profile.name] and USER_SHA[profile.name] != h:
        fail(where, f"the user code changed (SHA-256 {h}); retranslate it by hand and update USER_SHA")
    gen_cls = dict(cls)
    gen_cls["members"] = gen_members
    em = Emitter(where, profile, gen_cls, "parser")
    em.constants = constants
    em.try_stack = []
    em.fallible = em.compute_fallible()
    em.compute_nullable_returns()
    # ------------------------------------------------------------- state
    state = []
    state.append("/// The JavaCC state of the parser: the fields JavaCC generates.\n///\n")
    state.append("/// **Divergence.** Java declares these fields on the parser class itself;\n")
    state.append("/// they are gathered here so that the parser's generated code reaches them\n")
    state.append(f"/// through [`{profile.ops_trait}::jj`] whatever type implements the trait.\n")
    state.append("#[derive(Debug)]\npub struct JjState {\n")
    fields = []
    for m in gen_members:
        if m["kind"] != "field":
            continue
        for (n, init) in m["decls"]:
            if "static" in m["mods"]:
                continue
            ty = m["type"]
            rn = snake(n)
            nullable = em.field_nullable(n)
            rt = em.rust_type(ty, nullable, m["line"])
            vis = "pub " if "public" in m["mods"] else "pub(crate) "
            state.append(f"    /// `{n}`.\n    {vis}{rn}: {rt},\n")
            fields.append((m, n, init, rn, ty, nullable))
    state.append("}\n\n")
    # statics: jj_la1_N from the init methods
    statics = []
    for m in gen_members:
        if m["kind"] == "method" and m["name"].startswith("jj_la1_init_"):
            st = m["body"][1]
            if len(st) != 1 or st[0][0] != "expr" or st[0][1][0] != "assign" or st[0][1][3][0] != "newarrayinit":
                fail(where, f"unexpected {m['name']}")
            target = st[0][1][2][1]
            vals = [int_lit(x[2], T_INT) for x in st[0][1][3][2][1]]
            statics.append(f"/// `{target}`, as `{m['name']}()` sets it.\nstatic {upper_snake(target)}: [i32; {len(vals)}] = [{', '.join(vals)}];\n\n")
    state.extend(statics)
    # the JJCalls class
    jjcalls = em.nested.get("JJCalls")
    if jjcalls is not None:
        state.append("/// `JJCalls`, a node of the list of the lookahead calls `jj_save`\n/// records.\n///\n")
        state.append(f"/// Equivalent to `{profile.java_package}.{profile.parser_class}.JJCalls`, generated by JavaCC.\n")
        state.append("#[derive(Debug, Default)]\npub(crate) struct JJCalls {\n")
        for m in jjcalls["members"]:
            if m["kind"] == "field":
                for (n, _) in m["decls"]:
                    state.append(f"    /// `{n}`.\n    pub(crate) {snake(n)}: {em.rust_type(m['type'], False, m['line'])},\n")
        state.append("}\n\n/// A reference to a [`JJCalls`]: Java's object reference.\npub(crate) type JJCallsRef = crate::javacc::Shared<JJCalls>;\n\n")
    if "LookaheadSuccess" in em.nested:
        state.append("/// The control-flow throwable of the lookahead: `jj_scan_token` throws it\n")
        state.append("/// when the lookahead limit is reached with every token matched, and\n")
        state.append("/// `jj_2_*` and `jj_rescan_token` catch it.\n///\n")
        state.append(f"/// Equivalent to `{profile.java_package}.{profile.parser_class}.LookaheadSuccess`, generated by JavaCC (a\n")
        state.append("/// `java.lang.Error` without a stack trace, one shared instance `jj_ls`).\n")
        state.append("#[derive(Debug, Clone, Copy)]\npub(crate) struct LookaheadSuccess;\n\n")
        state.append("/// What a lookahead routine raises: Java's `throw jj_ls`, or any other\n/// throwable, which passes through.\n")
        state.append("#[derive(Debug)]\npub(crate) enum Raised {\n    /// `jj_ls`.\n    LookaheadSuccess(LookaheadSuccess),\n    /// Any other throwable.\n    Throwable(LuceneError),\n}\n\n")
        state.append("impl From<LuceneError> for Raised {\n    fn from(e: LuceneError) -> Self {\n        Raised::Throwable(e)\n    }\n}\n\n")
    # constructors as JjState functions
    for m in gen_members:
        if m["kind"] != "ctor":
            continue
        pt = m["params"][0][0].name
        rname = "with_char_stream" if pt == "CharStream" else "with_token_manager"
        state.append(f"impl JjState {{\n    /// The state `{profile.parser_class}({pt})` initializes.\n")
        state.append("    #[allow(unused_mut, clippy::all)]\n")
        p_rust = "Box<dyn CharStream>" if pt == "CharStream" else profile.tm_class
        state.append(f"    pub fn {rname}(stream: {p_rust}) -> Self {{\n")
        inits = []
        for (fm, n, init, rn, ty, nullable) in fields:
            if n == "token_source":
                inits.append(f"{rn}: {profile.tm_class + '::new(stream)' if pt == 'CharStream' else 'stream'}")
            elif init is None:
                inits.append(f"{rn}: {em.default_value(ty, nullable, fm['line'])}")
            else:
                p, c, t, n2 = em.emit_expr(init, Scope())
                inits.append(f"{rn}: {em.coerce(c, t, n2, ty, nullable, fm['line'])}")
        state.append(f"        let mut s = Self {{ {', '.join(inits)} }};\n")
        state.append("        s.reset();\n        s\n    }\n")
        state.append("}\n\n")
    # reset(): the statements every constructor and ReInit share after
    # setting token_source
    state.append("impl JjState {\n    /// The statements the constructors and `ReInit` run after setting\n    /// `token_source`.\n")
    state.append("    #[allow(unused_labels)]\n    pub(crate) fn reset(&mut self) {\n")
    reinit = [m for m in gen_members if m["kind"] == "method" and m["name"] == "ReInit" and m["params"][0][0].name == "CharStream"][0]
    rbody = []
    em.fn_kind = "plain"
    em.mode_state = True
    sc = Scope()
    for st in reinit["body"][1][1:]:
        em.emit_stmt(st, sc, rbody, 2)
    state.extend(x.replace("this.jj_mut()", "self").replace("this.jj()", "self") + "\n" for x in rbody)
    state.append("    }\n}\n\n")
    em.mode_state = False
    # ---------------------------------------------------------- productions
    prods = []
    priv = []
    for m in gen_members:
        if m["kind"] != "method":
            continue
        if m["name"].startswith("jj_la1_init_"):
            continue
        if m["name"] == "ReInit" and m["params"][0][0].name == "CharStream":
            # overrides QueryParserBase.ReInit: a free function
            out = []
            em.emit_method(m, out, 0, as_free_fn=True)
            priv.extend(out)
            continue
        if em.is_public_parser_member(m) and m["name"] != "ReInit" or (m["name"] == "ReInit"):
            out = []
            if m["name"] in ("TopLevelQuery",):
                em.emit_method(m, out, 0, as_free_fn=True)
                priv.extend(out)
            else:
                em.emit_method(m, out, 1, as_trait_method=True)
                prods.extend(out)
            continue
        out = []
        em.emit_method(m, out, 0)
        priv.extend(out)
    for m in gen_members:
        if m["kind"] == "field" and "static" in m["mods"]:
            for (n, init) in m["decls"]:
                if n in ("jj_la1_0", "jj_la1_1", "jj_la1_2", "jj_la1_3", "jj_ls"):
                    continue
                fail(where, f"unexpected static {n}")
        elif m["kind"] == "init":
            continue
        elif m["kind"] == "type" and m["decl"]["name"] in ("JJCalls", "LookaheadSuccess"):
            continue
        elif m["kind"] in ("method", "ctor", "field", "empty"):
            continue
        else:
            fail(where, f"unexpected member {m['kind']}")
    return {
        "state": "".join(state),
        "productions": "\n".join(prods),
        "private": "\n".join(priv),
    }


def member_key(m, class_name):
    if m["kind"] == "field":
        return m["decls"][0][0]
    if m["kind"] == "method":
        return m["name"]
    if m["kind"] == "ctor":
        return class_name + "(" + ",".join(t.name for (t, _) in m["params"]) + ")"
    if m["kind"] == "type":
        return m["decl"]["name"]
    return m["kind"]


# ==========================================================================
# Driver
# ==========================================================================


def rustfmt(text: str) -> str:
    with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False, encoding="utf-8") as f:
        f.write(text)
        name = f.name
    r = subprocess.run(["rustfmt", "--edition", "2021", name], capture_output=True, text=True)
    if r.returncode != 0:
        raise TranslationError(f"rustfmt failed on generated code:\n{r.stderr}\n{name}")
    out = Path(name).read_text(encoding="utf-8")
    Path(name).unlink()
    return out


def replace_regions(text: str, regions: dict, where: str) -> str:
    for name, body in regions.items():
        begin = f"// @generated begin {name}\n"
        end = f"// @generated end {name}\n"
        a = text.find(begin)
        b = text.find(end)
        if a < 0 or b < 0:
            fail(where, f"no region {name}")
        text = text[:a + len(begin)] + body + text[b:]
    return text


def main(argv):
    global PRINT_HASHES
    check = "--check" in argv
    PRINT_HASHES = "--hashes" in argv
    only = None
    if "--only" in argv:
        only = argv[argv.index("--only") + 1]
    positional = [a for a in argv[1:] if not a.startswith("--") and a != only]
    lucene = Path(positional[0] if positional else "/tmp/lucene1050")
    outputs = {}
    parser_files = set()
    for profile in PROFILES:
        if only and profile.name != only:
            continue
        for jf in ("Token.java", "TokenMgrError.java", "ParseException.java"):
            check_template(profile, lucene, jf)
        if PRINT_HASHES:
            translate_parser(profile, lucene, {})
            continue
        base = REPO / CRATE / profile.rust_dir
        consts_text, constants = translate_constants(profile, lucene)
        outputs[base / f"{snake(profile.parser_class)}_constants.rs"] = rustfmt(consts_text)
        outputs[base / f"{snake(profile.parser_class)}_token_manager.rs"] = rustfmt(translate_token_manager(profile, lucene, constants))
        outputs[base / "token.rs"] = rustfmt(template_token(profile))
        outputs[base / "token_mgr_error.rs"] = rustfmt(template_token_mgr_error(profile))
        if profile.parse_exception == "flexible":
            outputs[base / "parse_exception.rs"] = rustfmt(template_parse_exception_flexible(profile))
        else:
            outputs[base / "parse_exception.rs"] = rustfmt(template_parse_exception(profile))
        regions = translate_parser(profile, lucene, constants)
        parser_rs = base / f"{snake(profile.parser_class)}.rs"
        parser_files.add(parser_rs)
        current = parser_rs.read_text(encoding="utf-8")
        new = replace_regions(current, regions, str(parser_rs))
        outputs[parser_rs] = new
    if PRINT_HASHES:
        return 0
    status = 0
    for path, text in outputs.items():
        if path in parser_files:
            text = rustfmt(text)
        if check:
            if not path.exists() or path.read_text(encoding="utf-8") != text:
                print(f"differs: {path.relative_to(REPO)}")
                status = 1
        else:
            path.write_text(text, encoding="utf-8")
            print(f"wrote {path.relative_to(REPO)}")
    return status


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except TranslationError as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(2)
