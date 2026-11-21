#!/usr/bin/env python3
# Licensed to the Apache Software Foundation (ASF) under one or more
# contributor license agreements.  See the NOTICE file distributed with
# this work for additional information regarding copyright ownership.
# The ASF licenses this file to You under the Apache License, Version 2.0
# (the "License"); you may not use this file except in compliance with
# the License.  You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""Translates the regular test classes of the per-language packages of
Lucene 10.5.0 `lucene/analysis/common/src/test` (rmp #468-#471) into Rust
test suites of `crates/rucene-analysis-common`.

The test classes of these packages are short and regular: analyzer tests
(`checkOneTerm`, `assertAnalyzesTo`, `checkRandomData` over a language
analyzer), stem-filter tests (an anonymous `Analyzer` over a `MockTokenizer`
and the filter, `assertVocabulary` over the package's test data) and
factory tests (`tokenFilterFactory(...).create(...)`, bogus arguments).

The translator parses each class into a small Java AST and emits the Rust
suite statement for statement. It accepts only the Java constructs it knows
how to translate; any other construct rejects the whole class, which is then
ported by hand (`--list` prints both sets). It never guesses: an unknown
class, constructor, method or argument shape is a rejection.

Usage:
    lang_tests.py            write the generated suites
    lang_tests.py --check    exit 1 when the committed output is stale
    lang_tests.py --list     print the generated and rejected classes
"""

import os
import re
import subprocess
import sys

LUCENE = os.environ.get("LUCENE_SRC", "/tmp/lucene1050")
TEST_ROOT = LUCENE + "/lucene/analysis/common/src/test/org/apache/lucene/analysis/"
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CRATE = REPO + "/crates/rucene-analysis-common"
OUT = CRATE + "/tests/analysis_lang_generated/mod.rs"

# The test classes of each task (the `-ac` of rmp #468-#471), by package.
TASKS = {
    468: "ca da nl fr gl de it no pt es sv",
    469: "br el id ga ro tr",
    470: "hy eu bg cz et fi hu lv lt ru sr",
    471: "ar bn hi in ne fa ckb ta te",
}

# Test classes of these packages that belong to other tasks.
FOREIGN = {"TestLithuanianStemming"}  # rmp #455 (Snowball), ported in T4


class Reject(Exception):
    """A construct the translator does not handle: the class is hand-ported."""


# ---------------------------------------------------------------------------
# Lexer
# ---------------------------------------------------------------------------

TOKEN_RE = re.compile(
    r"""
    (?P<ws>\s+)
  | (?P<jdoc>/\*\*.*?\*/)
  | (?P<bcom>/\*.*?\*/)
  | (?P<lcom>//[^\n]*)
  | (?P<str>"(?:\\.|[^"\\\n])*")
  | (?P<chr>'(?:\\.|[^'\\\n])+')
  | (?P<num>0[xX][0-9a-fA-F_]+[lL]?|\d[\d_]*(?:\.\d+)?[lLfFdD]?)
  | (?P<ann>@[A-Za-z_][A-Za-z_0-9.]*)
  | (?P<id>[A-Za-z_$][A-Za-z_0-9$]*)
  | (?P<op>->|\+\+|--|==|!=|<=|>=|&&|\|\||[-+*/%<>=!&|^~?:;,.(){}\[\]])
    """,
    re.S | re.X,
)


def unescape_java(lit):
    """The UTF-16 string of a Java string or char literal body."""
    out = []
    i = 0
    while i < len(lit):
        c = lit[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        n = lit[i + 1]
        if n == "u":
            j = i + 1
            while lit[j] == "u":
                j += 1
            out.append(chr(int(lit[j : j + 4], 16)))
            i = j + 4
        elif n in "btnfr\"'\\":
            out.append({"b": "\b", "t": "\t", "n": "\n", "f": "\f", "r": "\r"}.get(n, n))
            i += 2
        elif n in "01234567":
            j = i + 1
            while j < len(lit) and j < i + 4 and lit[j] in "01234567":
                j += 1
            out.append(chr(int(lit[i + 1 : j], 8)))
            i = j
        else:
            raise Reject("escape \\" + n)
    # combine surrogate pairs; reject lone surrogates
    s = "".join(out)
    res = []
    k = 0
    while k < len(s):
        o = ord(s[k])
        if 0xD800 <= o <= 0xDBFF and k + 1 < len(s) and 0xDC00 <= ord(s[k + 1]) <= 0xDFFF:
            res.append(chr(0x10000 + ((o - 0xD800) << 10) + (ord(s[k + 1]) - 0xDC00)))
            k += 2
        elif 0xD800 <= o <= 0xDFFF:
            raise Reject("lone surrogate in a literal")
        else:
            res.append(s[k])
            k += 1
    return "".join(res)


def lex(src):
    toks = []
    pos = 0
    while pos < len(src):
        m = TOKEN_RE.match(src, pos)
        if not m:
            raise Reject("lex error at " + repr(src[pos : pos + 20]))
        kind = m.lastgroup
        text = m.group(kind)
        pos = m.end()
        if kind in ("ws", "bcom"):
            continue
        toks.append((kind, text))
    toks.append(("eof", ""))
    return toks


# ---------------------------------------------------------------------------
# Parser (a Java subset)
# ---------------------------------------------------------------------------


class Parser:
    def __init__(self, toks):
        self.t = toks
        self.i = 0
        self.anon = 0

    def peek(self, k=0):
        return self.t[self.i + k]

    def at(self, text, k=0):
        return self.t[self.i + k][1] == text and self.t[self.i + k][0] in ("op", "id")

    def next(self):
        tok = self.t[self.i]
        self.i += 1
        return tok

    def expect(self, text):
        tok = self.next()
        if tok[1] != text:
            raise Reject("expected %r, got %r" % (text, tok[1]))
        return tok

    def ident(self):
        tok = self.next()
        if tok[0] != "id":
            raise Reject("expected an identifier, got %r" % (tok[1],))
        return tok[1]

    def comments(self):
        """Line comments and javadoc before the next token."""
        out = []
        while self.peek()[0] in ("lcom", "jdoc"):
            out.append(self.next())
        return out

    # -- compilation unit -----------------------------------------------------

    def unit(self):
        imports = {}
        package = None
        cls = None
        while self.peek()[0] != "eof":
            self.comments()
            if self.at("package"):
                self.next()
                package = self.qualified()
                self.expect(";")
            elif self.at("import"):
                self.next()
                static = False
                if self.at("static"):
                    self.next()
                    static = True
                name = self.qualified(allow_star=True)
                self.expect(";")
                if not static:
                    imports[name.rsplit(".", 1)[1]] = name
            elif self.peek()[0] == "eof":
                break
            else:
                cls = self.class_decl()
                self.comments()
                if self.peek()[0] != "eof":
                    raise Reject("more than one top-level type")
        return package, imports, cls

    def qualified(self, allow_star=False):
        parts = [self.ident()]
        while self.at("."):
            self.next()
            if allow_star and self.at("*"):
                self.next()
                parts.append("*")
                break
            parts.append(self.ident())
        return ".".join(parts)

    def annotations(self):
        anns = []
        while self.peek()[0] == "ann":
            a = self.next()[1]
            if self.at("("):
                raise Reject("annotation with arguments: " + a)
            anns.append(a)
        return anns

    def modifiers(self):
        mods = []
        while self.peek()[1] in ("public", "private", "protected", "static", "final", "abstract"):
            mods.append(self.next()[1])
        return mods

    def class_decl(self):
        anns = self.annotations()
        if [a for a in anns if a != "@Deprecated"]:
            raise Reject("class annotations " + " ".join(anns))
        self.modifiers()
        self.expect("class")
        name = self.ident()
        self.expect("extends")
        base = self.qualified()
        self.expect("{")
        members = []
        while not self.at("}"):
            members.append(self.member())
        self.expect("}")
        return {"name": name, "base": base, "members": members, "deprecated": "@Deprecated" in anns}

    def type_(self):
        name = self.qualified()
        if self.at("<"):
            depth = 0
            while True:
                tok = self.next()[1]
                if tok == "<":
                    depth += 1
                elif tok == ">":
                    depth -= 1
                    if depth == 0:
                        break
            name += "<>"
        while self.at("[") and self.at("]", 1):
            self.next()
            self.next()
            name += "[]"
        return name

    def member(self):
        docs = self.comments()
        anns = self.annotations()
        docs += self.comments()
        mods = self.modifiers()
        if self.at("class") or self.at("interface"):
            raise Reject("nested type")
        if self.at("void"):
            self.next()
            rtype = "void"
        else:
            rtype = self.type_()
        name = self.ident()
        if self.at("("):
            self.next()
            params = []
            while not self.at(")"):
                if self.at("final"):
                    self.next()
                ptype = self.type_()
                params.append((ptype, self.ident()))
                if self.at(","):
                    self.next()
            self.expect(")")
            if self.at("throws"):
                self.next()
                self.qualified()
                while self.at(","):
                    self.next()
                    self.qualified()
            body = self.block()
            return {
                "kind": "method",
                "docs": docs,
                "anns": anns,
                "mods": mods,
                "rtype": rtype,
                "name": name,
                "params": params,
                "body": body,
            }
        names = [name]
        init = None
        if self.at("="):
            raise Reject("field initializer")
        while self.at(","):
            self.next()
            names.append(self.ident())
        self.expect(";")
        return {"kind": "field", "mods": mods, "type": rtype, "names": names}

    # -- statements -----------------------------------------------------------

    def block(self):
        self.expect("{")
        stmts = []
        while True:
            coms = [c[1] for c in self.comments() if c[0] == "lcom"]
            if self.at("}"):
                break
            st = self.statement()
            st["comments"] = coms
            stmts.append(st)
        self.expect("}")
        return stmts

    def is_local_decl(self):
        k = 0
        if self.at("final"):
            k = 1
        if self.peek(k)[0] != "id":
            return False
        k += 1
        while self.at(".", k) and self.peek(k + 1)[0] == "id":
            k += 2
        if self.at("<", k):
            depth = 0
            while True:
                tok = self.peek(k)[1]
                k += 1
                if tok == "<":
                    depth += 1
                elif tok == ">":
                    depth -= 1
                    if depth == 0:
                        break
        while self.at("[", k) and self.at("]", k + 1):
            k += 2
        return self.peek(k)[0] == "id" and self.peek(k + 1)[1] in ("=", ";", ",")

    def statement(self):
        if self.peek()[1] in ("for", "while", "if", "try", "do", "switch", "synchronized", "throw"):
            raise Reject("statement " + self.peek()[1])
        if self.at("return"):
            self.next()
            e = self.expr()
            self.expect(";")
            return {"kind": "return", "expr": e}
        if self.is_local_decl():
            if self.at("final"):
                self.next()
            ltype = self.type_()
            name = self.ident()
            if not self.at("="):
                raise Reject("local without initializer")
            self.next()
            e = self.expr()
            self.expect(";")
            return {"kind": "local", "type": ltype, "name": name, "expr": e}
        e = self.expr()
        if self.at("="):
            self.next()
            if e[0] != "name":
                raise Reject("assignment to a non-name")
            v = self.expr()
            self.expect(";")
            return {"kind": "assign", "name": e[1], "expr": v}
        self.expect(";")
        return {"kind": "expr", "expr": e}

    # -- expressions ----------------------------------------------------------

    def expr(self):
        return self.binary(0)

    PREC = [("||",), ("&&",), ("==", "!="), ("<", ">", "<=", ">="), ("+", "-"), ("*", "/", "%")]

    def binary(self, level):
        if level == len(self.PREC):
            return self.unary()
        left = self.binary(level + 1)
        while self.peek()[0] == "op" and self.peek()[1] in self.PREC[level]:
            op = self.next()[1]
            right = self.binary(level + 1)
            left = ("bin", op, left, right)
        return left

    def unary(self):
        if self.at("-") or self.at("!"):
            op = self.next()[1]
            return ("unary", op, self.unary())
        if self.at("(") and self.peek(1)[0] == "id" and self.at(")", 2) and self.peek(3)[0] in ("id",) or (
            self.at("(") and self.peek(1)[0] == "id" and self.at(")", 2) and self.at("(", 3)
        ):
            self.next()
            t = self.ident()
            self.expect(")")
            return ("cast", t, self.unary())
        return self.postfix(self.primary())

    def args(self):
        self.expect("(")
        out = []
        while not self.at(")"):
            out.append(self.expr())
            if self.at(","):
                self.next()
        self.expect(")")
        return out

    def primary(self):
        tok = self.peek()
        if tok[0] == "str":
            self.next()
            return ("str", unescape_java(tok[1][1:-1]))
        if tok[0] == "chr":
            self.next()
            return ("chr", unescape_java(tok[1][1:-1]))
        if tok[0] == "num":
            self.next()
            return ("num", tok[1])
        if self.at("true") or self.at("false"):
            self.next()
            return ("bool", tok[1] == "true")
        if self.at("null"):
            self.next()
            return ("null",)
        if self.at("new"):
            self.next()
            t = self.type_()
            if self.at("["):
                raise Reject("sized array")
            if t.endswith("[]"):
                self.expect("{")
                elems = []
                while not self.at("}"):
                    elems.append(self.expr())
                    if self.at(","):
                        self.next()
                self.expect("}")
                return ("newarray", t, elems)
            a = self.args()
            if self.at("{"):
                self.anon += 1
                n = self.anon
                self.next()
                members = []
                while not self.at("}"):
                    members.append(self.member())
                self.expect("}")
                return ("anon", t, a, members, n)
            return ("new", t, a)
        if self.at("("):
            # lambda `() -> ...`
            if self.at(")", 1) and self.at("->", 2):
                self.next()
                self.next()
                self.next()
                if self.at("{"):
                    return ("lambda", self.block())
                return ("lambda", [{"kind": "expr", "expr": self.expr(), "comments": []}])
            self.next()
            e = self.expr()
            self.expect(")")
            return ("paren", e)
        if tok[0] == "id":
            self.next()
            if self.at("("):
                return ("call", None, tok[1], self.args())
            return ("name", tok[1])
        raise Reject("unexpected token %r" % (tok[1],))

    def postfix(self, e):
        while True:
            if self.at("."):
                self.next()
                if self.at("<"):
                    # explicit type arguments: `CharArrayMap.<String>emptyMap()`
                    while not self.at(">"):
                        self.next()
                    self.next()
                if self.at("class"):
                    self.next()
                    e = ("classlit", e)
                    continue
                name = self.ident()
                if self.at("("):
                    e = ("call", e, name, self.args())
                else:
                    e = ("field", e, name)
            elif self.at("["):
                raise Reject("array indexing")
            else:
                return e


# ---------------------------------------------------------------------------
# Rust emission
# ---------------------------------------------------------------------------

RUST_KEYWORDS = {"in", "ref", "type", "fn", "mod", "use", "impl", "match", "move", "self", "loop", "box"}


def snake(name):
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    s = s.lower()
    return s + "_" if s in RUST_KEYWORDS else s


def rust_str(s):
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
        elif 0x20 <= o < 0x7F:
            out.append(ch)
        else:
            out.append("\\u{%X}" % o)
    out.append('"')
    return "".join(out)


def crate_module_of(java_fqcn):
    """The Rust module path of a class of `lucene-analysis-common`, checked
    to exist."""
    pkg, cls = java_fqcn.rsplit(".", 1)
    if not pkg.startswith("org.apache.lucene.analysis."):
        raise Reject("class outside the analysis packages: " + java_fqcn)
    sub = pkg[len("org.apache.lucene.analysis.") :].replace(".", "/")
    path = "%s/src/analysis/%s/%s.rs" % (CRATE, sub, snake(cls).rstrip("_"))
    if not os.path.exists(path):
        raise Reject("no Rust module for " + java_fqcn)
    rsub = "::".join(snake(p) if p not in ("in",) else "r#in" for p in sub.split("/"))
    return "rucene_analysis_common::analysis::%s::%s" % (rsub, snake(cls).rstrip("_")), path


def has_fn(path, fn):
    with open(path, encoding="utf-8") as f:
        return re.search(r"pub fn %s\b" % re.escape(fn), f.read()) is not None


# Classes of `lucene-core` / the test framework the tests use, by simple name.
MOCK_PATTERNS = {"WHITESPACE", "KEYWORD", "SIMPLE"}


class Emitter:
    def __init__(self, package, imports, cls, task):
        self.package = package
        self.imports = imports
        self.cls = cls
        self.task = task
        self.uses = set()
        self.fields = {}
        self.helpers = {}
        self.anon_names = {}

    # -- names ----------------------------------------------------------------

    def fqcn(self, simple):
        if simple in self.imports:
            return self.imports[simple]
        return self.package + "." + simple

    def analyzer_class(self, simple):
        """The Rust path of the analyzer class `simple`, or None."""
        if not simple.endswith("Analyzer"):
            return None
        fq = self.fqcn(simple)
        mod, path = crate_module_of(fq)
        self.uses.add("%s::%s" % (mod, simple))
        return simple, path

    def filter_ctor(self, simple, nargs):
        """The Rust constructor function of the filter class `simple` taking
        `nargs` Java arguments."""
        fq = self.fqcn(simple)
        if fq == "org.apache.lucene.analysis.LowerCaseFilter":
            self.uses.add("rucene::analysis::new_lower_case_filter")
            if nargs != 1:
                raise Reject("LowerCaseFilter arity")
            return "new_lower_case_filter"
        mod, path = crate_module_of(fq)
        if fq == "org.apache.lucene.analysis.core.LowerCaseFilter":
            self.uses.add(mod + "::LowerCaseFilter")
            return "LowerCaseFilter::new"
        fn = "new_" + snake(simple).rstrip("_")
        variants = {
            ("NorwegianLightStemFilter", 2): fn + "_with_flags",
            ("NorwegianMinimalStemFilter", 2): fn + "_with_flags",
            ("IndonesianStemFilter", 2): fn + "_with_derivational",
        }
        if nargs > 1 and simple != "SetKeywordMarkerFilter":
            if (simple, nargs) not in variants:
                raise Reject("filter constructor %s/%d" % (simple, nargs))
            fn = variants[(simple, nargs)]
        if not has_fn(path, fn):
            raise Reject("no %s in %s" % (fn, path))
        self.uses.add("%s::%s" % (mod, fn))
        return fn

    def local(self, name):
        return snake(name)

    # -- class ----------------------------------------------------------------

    def emit(self):
        cls = self.cls
        base = cls["base"]
        builder = {
            "BaseTokenStreamTestCase": "btstc_class",
            "BaseTokenStreamFactoryTestCase": "factory_class",
        }.get(base)
        if builder is None:
            raise Reject("base class " + base)
        methods = []
        for m in cls["members"]:
            if m["kind"] == "field":
                if m["type"] != "Analyzer" or "static" in m["mods"]:
                    raise Reject("field of type " + m["type"])
                for n in m["names"]:
                    self.fields[n] = "analyzer"
            else:
                methods.append(m)
        for m in methods:
            if not m["name"].startswith("test") or m["params"]:
                if m["name"] not in ("setUp", "tearDown"):
                    self.helpers[m["name"]] = m
        fns = []
        registrations = []
        for m in methods:
            name = m["name"]
            for a in m["anns"]:
                if a != "@Override":
                    raise Reject("method annotation " + a)
            if name in ("setUp", "tearDown"):
                fns.append(self.method_fn(m, hook=True))
                registrations.append('.method(hook("%s", %s))' % (name, snake(name)))
            elif name.startswith("test") and not m["params"] and "public" in m["mods"]:
                fns.append(self.method_fn(m))
                registrations.append('.method(t("%s", %s))' % (name, snake(name)))
            elif "private" in m["mods"] or not name.startswith("test"):
                fns.append(self.method_fn(m))
            else:
                raise Reject("method " + name)
        mod = snake(cls["name"])
        out = []
        out.append("// " + "-" * 77)
        out.append("// " + cls["name"])
        out.append("// " + "-" * 77)
        out.append("")
        if cls["deprecated"]:
            out.append("// `@Deprecated` in Lucene: it exercises a deprecated filter.")
        out.append("mod %s {" % mod)
        if cls["deprecated"]:
            out.append("#![allow(deprecated, unused_imports, clippy::redundant_clone)]")
        else:
            out.append("#![allow(unused_imports, clippy::redundant_clone)]")
        out.append("use super::*;")
        for u in sorted(self.uses):
            out.append("use %s;" % u)
        out.append("")
        if any("PKG" in f for f in fns):
            out.append('const PKG: &str = "%s";' % self.package)
            out.append("")
        for f in self.fields:
            up = snake(f).upper()
            out.append("thread_local! {")
            out.append("    /// The `%s` field." % f)
            out.append(
                "    static %s: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };" % up
            )
            out.append("}")
            out.append("fn %s() -> Arc<dyn Analyzer> {" % snake(f))
            out.append('    %s.with(|f| Arc::clone(f.borrow().as_ref().expect("setUp")))' % up)
            out.append("}")
            out.append("fn set_%s(a: Arc<dyn Analyzer>) {" % snake(f).rstrip("_"))
            out.append("    %s.with(|f| *f.borrow_mut() = Some(a));" % up)
            out.append("}")
            out.append("")
        out.extend(fns)
        out.append("pub fn class() -> Arc<Class> {")
        out.append('    %s("%s.%s")' % (builder, self.package, cls["name"]))
        for r in registrations:
            out.append("        " + r)
        out.append("        .build()")
        out.append("}")
        out.append("}")
        out.append("")
        out.append("#[test]")
        out.append("fn %s() {" % mod)
        out.append("    run(%s::class);" % mod)
        out.append("}")
        out.append("")
        return "\n".join(out)

    def method_fn(self, m, hook=False):
        self.vars = {}
        for ptype, pname in m["params"]:
            if ptype == "String":
                self.vars[pname] = "str"
            elif ptype == "Analyzer":
                self.vars[pname] = "analyzer"
            else:
                raise Reject("helper parameter " + ptype)
        if m["rtype"] not in ("void", "Analyzer"):
            raise Reject("helper return type " + m["rtype"])
        self.in_components = False
        lines = []
        for d in m["docs"]:
            if d[0] == "jdoc":
                for l in doc_lines(d[1]):
                    lines.append("/// " + l if l else "///")
            else:
                lines.append(d[1])
        params = ", ".join(
            "%s: %s" % (self.local(p), "&str" if t == "String" else "Arc<dyn Analyzer>")
            for t, p in m["params"]
        )
        if m["rtype"] == "Analyzer":
            body = m["body"]
            if len(body) != 1 or body[0]["kind"] != "return" or body[0]["expr"][0] != "anon":
                raise Reject("analyzer helper body")
            lines.append("fn %s(%s) -> Arc<dyn Analyzer> {" % (snake(m["name"]), params))
            lines.append("    " + self.anon_analyzer(body[0]["expr"]))
            lines.append("}")
            lines.append("")
            return "\n".join(lines)
        lines.append("fn %s(%s) -> Result<()> {" % (snake(m["name"]), params))
        body = self.stmts(m["body"])
        body = fix_mut(body)
        lines.extend("    " + l for l in body)
        lines.append("    Ok(())")
        lines.append("}")
        lines.append("")
        return "\n".join(lines)

    def stmts(self, stmts):
        out = []
        for st in stmts:
            for c in st.get("comments", []):
                out.append(c)
            out.extend(self.stmt(st))
        return out

    # -- statements -----------------------------------------------------------

    def stmt(self, st):
        k = st["kind"]
        if k == "local":
            name = st["name"]
            code, kind = self.expr(st["expr"], want=self.decl_kind(st["type"]))
            self.vars[name] = kind
            return ["let mut %s%s = %s;" % (self.local(name), self.annot(kind), code)]
        if k == "assign":
            name = st["name"]
            if name in self.fields and name not in self.vars:
                code, kind = self.expr(st["expr"], want="analyzer")
                if kind != "analyzer":
                    raise Reject("field assignment of " + kind)
                return ["set_%s(%s);" % (snake(name).rstrip("_"), code)]
            if name not in self.vars:
                raise Reject("assignment to unknown " + name)
            code, kind = self.expr(st["expr"], want=self.vars[name])
            self.vars[name] = kind
            return ["let mut %s%s = %s;" % (self.local(name), self.annot(kind), code)]
        if k == "expr":
            return self.expr_stmt(st["expr"])
        if k == "return":
            raise Reject("return outside createComponents")
        raise Reject("statement kind " + k)

    def annot(self, kind):
        return {
            "analyzer": ": Arc<dyn Analyzer>",
            "stream": ": Box<dyn TokenStream>",
            "reader": ": Box<dyn CharReader>",
        }.get(kind, "")

    def decl_kind(self, jtype):
        if jtype.endswith("Analyzer"):
            return "analyzer"
        return {
            "CharArraySet": "charset",
            "TokenStream": "stream",
            "TokenFilter": "stream",
            "Tokenizer": "tok",
            "Reader": "reader",
            "Random": "random",
            "IllegalArgumentException": "msg",
            "InputStream": "input",
        }.get(jtype, jtype)

    def expr_stmt(self, e):
        if e[0] == "call":
            _, obj, name, args = e
            if obj is None:
                return self.free_call(name, args)
            if name == "close" and not args:
                if obj[0] == "name":
                    k = self.kind_of(obj[1])
                    if k == "analyzer":
                        return ["%s.close();" % self.value(obj[1])]
                    if k == "input":
                        return ["drop(%s);" % self.local(obj[1])]
                if obj[0] == "new":
                    a = self.analyzer_class(obj[1])
                    if a and not obj[2]:
                        return ["%s::new().close();" % a[0]]
                raise Reject("close of " + repr(obj))
            if name == "setReader" and obj[0] == "name" and self.kind_of(obj[1]) == "tok":
                (r,) = args
                return ["%s.set_reader(%s)?;" % (self.local(obj[1]), self.reader(r))]
            if name == "add" and obj[0] == "name" and self.kind_of(obj[1]) == "charset":
                (s,) = args
                return ["%s.add(%s);" % (self.local(obj[1]), self.str_(s))]
            if name == "setUp" and obj == ("name", "super"):
                return ["LuceneTestCase::super_set_up();"]
            if name == "tearDown" and obj == ("name", "super"):
                return ["LuceneTestCase::super_tear_down();"]
            if obj == ("name", "IOUtils") and name == "close":
                out = []
                for a in args:
                    if a[0] != "name" or self.kind_of(a[1]) != "analyzer":
                        raise Reject("IOUtils.close of non-analyzer")
                    out.append("%s.close();" % self.value(a[1]))
                return out
            if (
                obj[0] == "paren"
                and obj[1][0] == "cast"
                and obj[1][1] == "Tokenizer"
                and name == "setReader"
            ):
                target = obj[1][2]
                if target[0] != "name" or self.kind_of(target[1]) != "tok":
                    raise Reject("setReader target")
                (r,) = args
                return ["%s.set_reader(%s)?;" % (self.local(target[1]), self.reader(r))]
        raise Reject("expression statement " + repr(e)[:80])

    def free_call(self, name, args):
        if name == "checkOneTerm":
            a, i, o = args
            return ["B::check_one_term(&*%s, %s, %s)?;" % (self.analyzer(a), self.str_(i), self.str_(o))]
        if name == "assertAnalyzesTo" and len(args) == 4:
            a, i, o, p = args
            return [
                "B::assert_analyzes_to_pos_increments(&*%s, %s, %s, %s)?;"
                % (self.analyzer(a), self.str_(i), self.strs(o), self.ints(p))
            ]
        if name == "assertAnalyzesTo":
            if len(args) != 3:
                raise Reject("assertAnalyzesTo/%d" % len(args))
            a, i, o = args
            return ["B::assert_analyzes_to(&*%s, %s, %s)?;" % (self.analyzer(a), self.str_(i), self.strs(o))]
        if name == "checkRandomData":
            if len(args) != 3:
                raise Reject("checkRandomData/%d" % len(args))
            r, a, n = args
            return ["B::check_random_data(&*%s, &%s, %s)?;" % (self.random(r), self.analyzer(a), self.int_(n))]
        if name == "assertTokenStreamContents":
            if len(args) != 2:
                raise Reject("assertTokenStreamContents/%d" % len(args))
            s, o = args
            return ["B::assert_token_stream_contents(&mut *%s, %s)?;" % (self.stream(s), self.strs(o))]
        if name == "assertVocabulary":
            a = self.analyzer(args[0])
            if len(args) == 3:
                return [
                    "VocabularyAssert::assert_vocabulary_zip_tsv(&*%s, &%s, %s)?;"
                    % (a, self.data_path(args[1]), self.str_(args[2]))
                ]
            if len(args) == 4:
                return [
                    "VocabularyAssert::assert_vocabulary_zip(&*%s, &%s, %s, %s)?;"
                    % (a, self.data_path(args[1]), self.str_(args[2]), self.str_(args[3]))
                ]
            if len(args) == 2:
                return ["VocabularyAssert::assert_vocabulary_tsv(&*%s, %s)?;" % (a, self.input(args[1]))]
            raise Reject("assertVocabulary/%d" % len(args))
        if name == "assertTrue" and len(args) == 2:
            m, c = args
            return ["Assert::assert_true_msg(&%s, %s);" % (self.message(m), self.cond(c))]
        if name == "assertTrue":
            (c,) = args
            return ["Assert::assert_true(%s);" % self.cond(c)]
        if name == "assertEquals" and len(args) == 2:
            exp, act = args
            if (
                exp[0] == "new"
                and exp[1] == "BytesRef"
                and len(exp[2]) == 1
                and act[0] == "call"
                and act[2] == "normalize"
                and len(act[3]) == 2
            ):
                self.uses.add("rucene::util::BytesRef")
                return [
                    "Assert::assert_true(BytesRef::from_char_sequence(%s) == %s.normalize(%s, %s)?);"
                    % (self.str_(exp[2][0]), self.analyzer(act[1]), self.str_(act[3][0]), self.str_(act[3][1]))
                ]
            raise Reject("assertEquals shape")
        if name in self.helpers:
            h = self.helpers[name]
            if len(h["params"]) != len(args) or h["rtype"] != "void":
                raise Reject("helper call")
            rargs = [
                self.str_(a) if t == "String" else "Arc::clone(&%s)" % self.analyzer(a)
                for (t, _), a in zip(h["params"], args)
            ]
            return ["%s(%s)?;" % (snake(name), ", ".join(rargs))]
        raise Reject("call " + name)

    # -- expressions ----------------------------------------------------------

    def kind_of(self, name):
        if name in self.vars:
            return self.vars[name]
        if name in self.fields:
            return "analyzer"
        raise Reject("unknown name " + name)

    def value(self, name):
        if name in self.vars:
            return self.local(name)
        if name in self.fields:
            return snake(name) + "()"
        raise Reject("unknown name " + name)

    def str_(self, e):
        if e[0] == "str":
            return rust_str(e[1])
        if e[0] == "name" and self.kind_of(e[1]) == "str":
            return self.local(e[1])
        if e[0] == "bin" and e[1] == "+":
            s = self.concat(e)
            if s is not None:
                return rust_str(s)
        raise Reject("string expression " + repr(e)[:60])

    def concat(self, e):
        if e[0] == "str":
            return e[1]
        if e[0] == "bin" and e[1] == "+":
            l = self.concat(e[2])
            r = self.concat(e[3])
            if l is not None and r is not None:
                return l + r
        return None

    def strs(self, e):
        if e[0] != "newarray" or e[1] != "String[]":
            raise Reject("string array " + repr(e)[:60])
        return "&[%s]" % ", ".join(self.str_(x) for x in e[2])

    def ints(self, e):
        if e[0] != "newarray" or e[1] != "int[]":
            raise Reject("int array " + repr(e)[:60])
        return "&[%s]" % ", ".join(self.int_(x) for x in e[2])

    def int_(self, e):
        if e[0] == "num" and re.fullmatch(r"\d+", e[1]):
            return e[1]
        if e[0] == "name" and e[1] == "RANDOM_MULTIPLIER":
            return "mult()"
        if e[0] == "bin" and e[1] in ("*", "+"):
            return "%s %s %s" % (self.int_(e[2]), e[1], self.int_(e[3]))
        raise Reject("int expression " + repr(e)[:60])

    def random(self, e):
        if e == ("call", None, "random", []):
            return "random()"
        if e[0] == "name" and self.kind_of(e[1]) == "random":
            return self.local(e[1])
        raise Reject("random expression")

    def analyzer(self, e):
        if e[0] == "name" and self.kind_of(e[1]) == "analyzer":
            return self.value(e[1])
        if e[0] == "new" and self.analyzer_class(e[1]):
            code, _ = self.new_expr(e, "analyzer")
            return "(%s as Arc<dyn Analyzer>)" % code
        raise Reject("analyzer expression " + repr(e)[:60])

    def data_path(self, e):
        if e[0] == "call" and e[1] is None and e[2] == "getDataPath":
            (n,) = e[3]
            return "data_path(PKG, %s)?" % self.str_(n)
        raise Reject("data path " + repr(e)[:60])

    def input(self, e):
        if (
            e[0] == "call"
            and e[1] == ("name", "Files")
            and e[2] == "newInputStream"
        ):
            (p,) = e[3]
            return "std::fs::File::open(%s)?" % self.data_path(p)
        if e[0] == "name" and self.kind_of(e[1]) == "input":
            return "&mut %s" % self.local(e[1])
        raise Reject("input stream " + repr(e)[:60])

    def message(self, e):
        # "Got " + expected.getMessage()
        if (
            e[0] == "bin"
            and e[1] == "+"
            and e[2][0] == "str"
            and e[3][0] == "call"
            and e[3][2] == "getMessage"
            and e[3][1][0] == "name"
            and self.kind_of(e[3][1][1]) == "msg"
        ):
            return 'format!("%s{}", %s)' % (rust_str(e[2][1])[1:-1].replace("{", "{{").replace("}", "}}"), self.local(e[3][1][1]))
        raise Reject("message " + repr(e)[:60])

    def cond(self, e):
        # expected.getMessage().contains("...")
        if (
            e[0] == "call"
            and e[2] == "contains"
            and e[1][0] == "call"
            and e[1][2] == "getMessage"
            and e[1][1][0] == "name"
            and self.kind_of(e[1][1][1]) == "msg"
        ):
            (s,) = e[3]
            return "%s.contains(%s)" % (self.local(e[1][1][1]), self.str_(s))
        raise Reject("condition " + repr(e)[:60])

    def reader(self, e):
        if e[0] == "new" and e[1] == "StringReader":
            (s,) = e[2]
            return "Box::new(StringCharReader::new(%s))" % self.str_(s)
        if e[0] == "name" and self.kind_of(e[1]) == "reader":
            return self.local(e[1])
        if e[0] == "call" and e[2] == "create" and e[1][0] == "call" and e[1][2] == "charFilterFactory":
            (r,) = e[3]
            return "%s.create(%s)?" % (self.factory(e[1], "char"), self.reader(r))
        raise Reject("reader " + repr(e)[:60])

    def factory(self, e, kind):
        _, obj, name, args = e
        if obj is not None:
            raise Reject("factory receiver")
        strs = ", ".join(self.str_(a) for a in args[1:])
        if name == "tokenFilterFactory" and kind == "filter":
            return "token_filter_factory(PKG, %s, &[%s])?" % (self.str_(args[0]), strs)
        if name == "charFilterFactory" and kind == "char":
            return "char_filter_factory(PKG, %s, &[%s])?" % (self.str_(args[0]), strs)
        raise Reject("factory " + name)

    def stream(self, e):
        """A `Box<dyn TokenStream>` expression for `e` (consumes a local)."""
        code, kind = self.expr(e, want="stream")
        if kind == "stream":
            return code
        if kind == "tok":
            return "boxed(%s)" % code
        if kind == "otok":
            return "shared(&%s)" % code
        raise Reject("stream of kind " + kind)

    def charset(self, e):
        code, kind = self.expr(e, want="charset")
        if kind != "charset":
            raise Reject("charset of kind " + kind)
        return code

    def expr(self, e, want=None):
        k = e[0]
        if k == "name":
            kind = self.kind_of(e[1])
            if kind == "charset":
                return self.local(e[1]) + ".clone()", "charset"
            return self.value(e[1]), kind
        if k == "field":
            if e == ("field", ("name", "CharArraySet"), "EMPTY_SET"):
                return "CharArraySet::empty_set()", "charset"
            raise Reject("field " + repr(e))
        if k == "str" and want == "str":
            return rust_str(e[1]), "str"
        if k == "new":
            return self.new_expr(e, want)
        if k == "anon":
            return self.anon_analyzer(e), "analyzer"
        if k == "call":
            _, obj, name, args = e
            if obj is None and name == "random" and not args:
                return "random()", "random"
            if obj is None and name in self.helpers and self.helpers[name]["rtype"] == "Analyzer" and not args:
                return "%s()" % snake(name), "analyzer"
            if obj is None and name == "whitespaceMockTokenizer":
                (r,) = args
                if r[0] == "str" or (r[0] == "name" and self.kind_of(r[1]) == "str"):
                    return "B::whitespace_mock_tokenizer_str(%s)?" % self.str_(r), "tok"
                return "B::whitespace_mock_tokenizer(%s)?" % self.reader(r), "tok"
            if obj is None and name == "expectThrows":
                cls, lam = args
                if cls != ("classlit", ("name", "IllegalArgumentException")) or lam[0] != "lambda":
                    raise Reject("expectThrows shape")
                body = lam[1]
                if len(body) != 1 or body[0]["kind"] != "expr":
                    raise Reject("expectThrows body")
                inner = body[0]["expr"]
                if inner[0] != "call" or inner[2] not in ("tokenFilterFactory", "charFilterFactory"):
                    raise Reject("expectThrows call")
                kind = "filter" if inner[2] == "tokenFilterFactory" else "char"
                f = self.factory(inner, kind)
                assert f.endswith("?")
                return "iae_message(|| %s)" % f[:-1], "msg"
            if name == "create" and obj is not None and obj[0] == "call" and obj[2] == "tokenFilterFactory":
                (s,) = args
                return "%s.create(%s)?" % (self.factory(obj, "filter"), self.stream(s)), "stream"
            if (
                name == "getDefaultStopSet"
                and obj is not None
                and obj[0] == "name"
                and not args
                and self.analyzer_class(obj[1])
            ):
                return "%s::get_default_stop_set().clone()" % obj[1], "charset"
            if (
                name == "getResourceAsStream"
                and obj == ("call", None, "getClass", [])
            ):
                (n,) = args
                return "resource_stream(PKG, %s)?" % self.str_(n), "input"
        if k == "call" and e[2] == "create" and e[1] is not None and e[1][0] == "call" and e[1][2] == "charFilterFactory":
            return self.reader(e), "reader"
        if k == "newarray":
            raise Reject("array value")
        raise Reject("expression " + repr(e)[:80])

    def new_expr(self, e, want):
        _, t, args = e
        if t == "CharArraySet":
            if (
                len(args) == 2
                and args[0][0] == "call"
                and args[0][1] is None
                and args[0][2] == "asSet"
                and args[1][0] == "bool"
            ):
                words = ", ".join(self.str_(w) for w in args[0][3])
                return "char_set(&[%s], %s)" % (words, "true" if args[1][1] else "false"), "charset"
            if len(args) == 2 and args[0][0] == "num" and args[1][0] == "bool":
                return (
                    "CharArraySet::new(%s, %s)" % (self.int_(args[0]), "true" if args[1][1] else "false"),
                    "charset",
                )
            raise Reject("CharArraySet constructor")
        if t == "StringReader":
            return self.reader(e), "reader"
        a = self.analyzer_class(t)
        if a:
            simple, path = a
            if len(args) == 0:
                ctor = "%s::new()" % simple
            elif len(args) == 1:
                ctor = "%s::with_stop_words(%s)" % (simple, self.charset(args[0]))
            elif len(args) == 2 and simple == "DutchAnalyzer":
                ctor = "%s::with_stem_exclusion_table(%s, &%s)" % (
                    simple,
                    self.charset(args[0]),
                    self.charset(args[1]).replace(".clone()", ""),
                )
            elif (
                len(args) == 3
                and simple == "DutchAnalyzer"
                and args[2] == ("call", ("name", "CharArrayMap"), "emptyMap", [])
            ):
                self.uses.add("rucene::analysis::CharArrayMap")
                ctor = "%s::with_stem_override_dict(%s, &%s, &CharArrayMap::empty_map())?" % (
                    simple,
                    self.charset(args[0]),
                    self.charset(args[1]).replace(".clone()", ""),
                )
            elif len(args) == 2:
                if not has_fn(path, "with_stem_exclusion_set"):
                    raise Reject("no with_stem_exclusion_set on " + simple)
                ctor = "%s::with_stem_exclusion_set(%s, &%s)" % (
                    simple,
                    self.charset(args[0]),
                    self.charset(args[1]).replace(".clone()", ""),
                )
            else:
                raise Reject("analyzer constructor arity")
            return "Arc::new(%s)" % ctor, "analyzer"
        if t == "MockTokenizer" and not args:
            # MockTokenizer() = this(WHITESPACE, true)
            code = "MockTokenizer::new(Arc::clone(&WHITESPACE), true)?"
            if self.in_components:
                return "own(%s)" % code, "otok"
            return code, "tok"
        if t == "MockTokenizer":
            if len(args) != 2 or args[0][0] != "field" or args[0][1] != ("name", "MockTokenizer"):
                raise Reject("MockTokenizer constructor")
            pat = args[0][2]
            if pat not in MOCK_PATTERNS or args[1][0] != "bool":
                raise Reject("MockTokenizer pattern")
            code = "MockTokenizer::new(Arc::clone(&%s), %s)?" % (pat, "true" if args[1][1] else "false")
            if self.in_components:
                return "own(%s)" % code, "otok"
            return code, "tok"
        if t in ("KeywordTokenizer", "LetterTokenizer"):
            if args:
                raise Reject(t + " args")
            if t == "LetterTokenizer":
                mod, _ = crate_module_of(self.fqcn(t))
                self.uses.add(mod + "::new_letter_tokenizer")
                code = "new_letter_tokenizer()"
            else:
                code = "new_keyword_tokenizer()"
            if self.in_components:
                return "own(%s)" % code, "otok"
            return code, "tok"
        if t.endswith("Filter"):
            fn = self.filter_ctor(t, len(args))
            rest = []
            for a in args[1:]:
                if a[0] == "bool":
                    rest.append("true" if a[1] else "false")
                elif a[0] == "name" and a[1] in ("BOKMAAL", "NYNORSK"):
                    # statically imported from NorwegianLightStemmer
                    self.uses.add(
                        "rucene_analysis_common::analysis::no::norwegian_light_stemmer::%s" % a[1]
                    )
                    rest.append(a[1])
                else:
                    rest.append(self.charset(a))
            code = "%s(%s)?" % (fn, ", ".join([self.stream(args[0])] + rest))
            return "boxed(%s)" % code, "stream"
        raise Reject("new " + t)

    # -- anonymous analyzers --------------------------------------------------

    def anon_analyzer(self, e):
        _, t, args, members, n = e
        if t != "Analyzer" or args:
            raise Reject("anonymous " + t)
        if len(members) != 1 or members[0]["kind"] != "method" or members[0]["name"] != "createComponents":
            raise Reject("anonymous analyzer members")
        body = members[0]["body"]
        saved = (dict(self.vars), self.in_components)
        self.in_components = True
        lines = []
        for st in body[:-1]:
            for c in st.get("comments", []):
                lines.append(c)
            if st["kind"] == "assign":
                if st["name"] not in self.vars or self.vars[st["name"]] != "stream":
                    raise Reject("createComponents assignment")
                st = dict(st, type="TokenStream")
            elif st["kind"] != "local":
                raise Reject("createComponents statement " + st["kind"])
            code, kind = self.expr(st["expr"], want=self.decl_kind(st["type"]))
            if kind == "tok":
                raise Reject("tokenizer in createComponents")
            self.vars[st["name"]] = kind
            lines.append("let %s = %s;" % (self.local(st["name"]), code))
        ret = body[-1]
        if ret["kind"] != "return":
            raise Reject("createComponents must end in return")
        r = ret["expr"]
        if r[0] != "new" or r[1] != "TokenStreamComponents":
            raise Reject("createComponents return")
        if len(r[2]) == 2:
            src, sink_e = r[2]
        elif len(r[2]) == 1:
            src, sink_e = r[2][0], r[2][0]
        else:
            raise Reject("TokenStreamComponents arity")
        if src[0] != "name" or self.kind_of(src[1]) != "otok":
            raise Reject("components source")
        if sink_e == src:
            lines.append(
                "let sink_: Arc<AtomicRefCell<dyn TokenStream>> = %s.clone();" % self.local(src[1])
            )
            lines.append("Ok(components(%s, sink_))" % self.local(src[1]))
        else:
            stream = self.stream(sink_e)
            lines.append("let stream = %s;" % stream)
            lines.append(
                "Ok(components(%s, rucene_test_framework::analysis::sink(BoxedStream(stream))))"
                % self.local(src[1])
            )
        self.vars, self.in_components = saved
        name = "%s.%s$%d" % (self.package, self.cls["name"], n)
        inner = "\n".join("        " + l for l in lines)
        return 'FnAnalyzer::new("%s", move |_| {\n%s\n    })\n    .arc()' % (name, inner)


def doc_lines(jdoc):
    body = jdoc[3:-2]
    out = []
    for l in body.split("\n"):
        l = l.strip()
        if l.startswith("*"):
            l = l[1:]
            if l.startswith(" "):
                l = l[1:]
        out.append(l.rstrip())
    while out and not out[0]:
        out.pop(0)
    while out and not out[-1]:
        out.pop()
    text = []
    for l in out:
        l = re.sub(r"\{@link ([^}]*)\}", r"`\1`", l)
        l = re.sub(r"\{@code ([^}]*)\}", r"`\1`", l)
        text.append(l)
    return text


MUTATING = (".add(", ".set_reader(")


def fix_mut(lines):
    """Drops `mut` from locals never mutated (rustc warns otherwise)."""
    out = []
    joined = "\n".join(lines)
    for idx, l in enumerate(lines):
        m = re.match(r"(\s*)let mut ([a-z_0-9]+)(.*)", l, re.S)
        if m:
            name = m.group(2)
            rest = "\n".join(lines[idx + 1 :])
            # until the next shadowing `let`
            nxt = re.search(r"let mut %s\b" % name, rest)
            scope = rest[: nxt.start()] if nxt else rest
            used_mut = (
                re.search(r"&mut \*?%s\b" % name, scope)
                or any(re.search(r"\b%s%s" % (name, re.escape(op)), scope) for op in MUTATING)
            )
            if not used_mut:
                l = "%slet %s%s" % (m.group(1), name, m.group(3))
        out.append(l)
    return out


HEADER = """\
// @generated by tools/translate/lang_tests.py from the test classes of
// Lucene 10.5.0 lucene/analysis/common/src/test; do not edit.
//
// Licensed to the Apache Software Foundation (ASF) under one or more
// contributor license agreements.  See the NOTICE file distributed with
// this work for additional information regarding copyright ownership.
// The ASF licenses this file to You under the Apache License, Version 2.0
// (the "License"); you may not use this file except in compliance with
// the License.  You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The test classes of the per-language packages (rmp #468-#471) that
//! `tools/translate/lang_tests.py` translates statement for statement; the
//! others are ported by hand in `analysis_lang_tests.rs`.

#![allow(unused_imports, clippy::needless_borrow)]

use std::cell::RefCell;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, CharArraySet, CharReader, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
use rucene_analysis_common::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD, SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::vocabulary_assert::VocabularyAssert;
use rucene_test_framework::analysis::{components, shared, sink, BaseTokenStreamTestCase};
use rucene_test_framework::util::LuceneTestCase;

use crate::lang_support::{char_filter_factory, char_set, data_path, mult, random, resource_stream};
use crate::support::{boxed, btstc_class, factory_class, hook, iae_message, own, run, t, token_filter_factory, FnAnalyzer};

type B = BaseTokenStreamTestCase;

"""


def classes():
    out = []
    for task, pkgs in TASKS.items():
        for p in pkgs.split():
            d = TEST_ROOT + p
            for f in sorted(os.listdir(d)):
                if f.startswith("Test") and f.endswith(".java") and f[:-5] not in FOREIGN:
                    out.append((task, p, d + "/" + f))
    return out


def translate(path, task):
    with open(path, encoding="utf-8") as f:
        src = f.read()
    package, imports, cls = Parser(lex(src)).unit()
    try:
        return Emitter(package, imports, cls, task).emit()
    except ValueError as e:
        # an argument list of an unexpected arity
        raise Reject("arity: %s" % e)


def rustfmt(code):
    r = subprocess.run(
        ["rustfmt", "--edition", "2021", "--emit", "stdout", "--quiet"],
        input=code.encode(),
        capture_output=True,
    )
    if r.returncode != 0:
        sys.stderr.write(r.stderr.decode())
        raise SystemExit("rustfmt failed")
    return r.stdout.decode()


def main():
    args = sys.argv[1:]
    generated, rejected = [], []
    parts = [HEADER]
    for task, pkg, path in classes():
        name = os.path.basename(path)[:-5]
        try:
            parts.append(translate(path, task))
            generated.append((task, name))
        except Reject as e:
            rejected.append((task, name, str(e)))
    if "--list" in args:
        for task, name in generated:
            print("generated #%d %s" % (task, name))
        for task, name, why in rejected:
            print("by-hand   #%d %s: %s" % (task, name, why))
        print("%d generated, %d by hand" % (len(generated), len(rejected)))
        return
    code = rustfmt("\n".join(parts))
    if "--check" in args:
        with open(OUT, encoding="utf-8") as f:
            if f.read() != code:
                raise SystemExit("%s is stale: run tools/translate/lang_tests.py" % OUT)
        print("current: %d generated, %d by hand" % (len(generated), len(rejected)))
        return
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(code)
    print("wrote %s: %d generated, %d by hand" % (OUT, len(generated), len(rejected)))


if __name__ == "__main__":
    main()
