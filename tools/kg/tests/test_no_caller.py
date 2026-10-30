"""The no-production-reference chain: SCIP reading, Rust reference attribution,
Lucene bytecode calls, and the Rust -> Lucene member mapping."""
import lucene_calls_kg as lc
import no_caller_kg as nc
import rust_refs_kg as rr
import scipio

P = "rust-analyzer cargo rucene 0.1.0 "


# ---------------------------------------------------------------------------
# scipio: a hand-encoded index round-trips
# ---------------------------------------------------------------------------

def _varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def _ld(num, payload):
    return _varint(num << 3 | 2) + _varint(len(payload)) + payload


def _vi(num, v):
    return _varint(num << 3) + _varint(v)


def _occ(rng, sym, roles=0, enc=None):
    b = _ld(1, b"".join(_varint(x) for x in rng)) + _ld(2, sym.encode())
    if roles:
        b += _vi(3, roles)
    if enc:
        b += _ld(7, b"".join(_varint(x) for x in enc))
    return b


def test_scip_reader_decodes_documents_occurrences_and_ranges():
    doc = (_ld(1, b"src/a.rs")
           + _ld(2, _occ([3, 4, 9], P + "a/Foo#", 1, [1, 0, 5, 1]))
           + _ld(2, _occ([7, 8, 7, 11], P + "a/bar().")))
    index = _ld(1, b"\x0a\x00") + _ld(2, doc)
    docs = scipio.read_index(index)
    assert len(docs) == 1 and docs[0].path == "src/a.rs"
    d, r = docs[0].occurrences
    assert d.range == (3, 4, 3, 9) and d.is_definition and d.enclosing == (1, 0, 5, 1)
    assert r.range == (7, 8, 7, 11) and not r.is_definition and r.symbol.endswith("bar().")


# ---------------------------------------------------------------------------
# rust_refs_kg: symbols and attribution
# ---------------------------------------------------------------------------

def test_item_of_reads_the_kg_relevant_descriptors():
    assert rr.item_of(P + "util/BytesRef#") == ("type", None, "BytesRef", ("util",))
    assert rr.item_of(P + "util/impl#[BytesRef]slice().") == ("fn", "BytesRef", "slice", ("util",))
    assert rr.item_of(P + "index/terms/TermsEnum#next().") == ("fn", "TermsEnum", "next", ("index", "terms"))
    assert rr.item_of(P + "a/impl#[Foo][Display]fmt().")[0] == "impl-trait"
    assert rr.item_of(P + "a/impl#[`Mgr<'a>`]new().") == ("fn", "Mgr", "new", ("a",))
    assert rr.item_of(P + "a/free().") == ("fn", None, "free", ("a",))
    assert rr.item_of(P + "a/Foo#field.") is None
    assert rr.item_of(P + "a/") is None
    assert rr.item_of("rust-analyzer cargo std https://x crate/") is None


SRC = """\
use crate::a::Foo;

pub struct Foo {
    x: u32,
}

impl Foo {
    pub fn new() -> Foo {
        Foo { x: 0 }
    }
}

impl std::fmt::Display for Foo {
    fn fmt(&self) {
        helper();
    }
}

pub fn helper() {}

pub fn user() {
    let _ = Foo::new();
}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        super::helper();
    }
}
"""


def _kg():
    return rr.KgIndex(types=[("RustStruct", "Foo", "src/a.rs", "rucene::a::Foo")],
                      fns=[("rucene::a::Foo::new", "src/a.rs", "Foo", "new"),
                           ("rucene::a::helper", "src/a.rs", None, "helper"),
                           ("rucene::a::user", "src/a.rs", None, "user")],
                      files=["src/a.rs"])


def _doc():
    O = scipio.Occurrence
    occ = [
        O((0, 0, 30, 0), P + "a/", 1, (0, 0, 30, 0)),
        O((0, 15, 0, 18), P + "a/Foo#"),                       # use: dropped
        O((2, 11, 2, 14), P + "a/Foo#", 1, (2, 0, 4, 1)),
        O((7, 11, 7, 14), P + "a/impl#[Foo]new().", 1, (7, 4, 9, 5)),
        O((7, 20, 7, 23), P + "a/Foo#"),                       # new -> Foo (own type)
        O((12, 31, 12, 34), P + "a/Foo#"),                     # impl header: self, dropped
        O((13, 7, 13, 10), P + "a/impl#[Foo][Display]fmt().", 1, (13, 4, 15, 5)),
        O((14, 8, 14, 14), P + "a/helper()."),                 # trait impl body -> Foo
        O((18, 7, 18, 13), P + "a/helper().", 1, (18, 0, 18, 18)),
        O((20, 7, 20, 11), P + "a/user().", 1, (20, 0, 22, 1)),
        O((21, 12, 21, 15), P + "a/Foo#"),                     # user -> Foo
        O((21, 17, 21, 20), P + "a/impl#[Foo]new()."),         # user -> Foo::new
        O((25, 4, 25, 9), P + "a/tests/", 1, (24, 0, 30, 1)),
        O((27, 7, 27, 8), P + "a/tests/t().", 1, (26, 4, 29, 5)),
        O((28, 15, 28, 21), P + "a/helper()."),                # test site
    ]
    return scipio.Document("src/a.rs", occ)


def test_attribution_credits_the_containing_item_and_classifies_test_sites():
    doc = _doc()
    got = sorted(rr.attribute_document(doc, SRC, _kg(), rr.build_def_index([doc])))
    foo = ("RustStruct", "Foo", "src/a.rs")
    new = ("RustFn", "rucene::a::Foo::new")
    helper = ("RustFn", "rucene::a::helper")
    user = ("RustFn", "rucene::a::user")
    assert (new, foo, False) in got                  # a method naming its own type
    assert (foo, helper, False) in got               # impl Display for Foo body -> Foo
    assert (user, foo, False) in got and (user, new, False) in got
    assert (("File", "src/a.rs"), helper, True) in got  # #[cfg(test)] mod tests: test site
    assert not any(t == foo and r == ("File", "src/a.rs") for r, t, _ in got)  # the `use`
    assert not any(r == t for r, t, _ in got)        # no self reference


def test_a_reference_from_tests_dir_is_a_test_site():
    O = scipio.Occurrence
    lib = scipio.Document("src/a.rs", [O((18, 7, 18, 13), P + "a/helper().", 1, (18, 0, 18, 18))])
    t = scipio.Document("tests/x.rs", [O((0, 0, 3, 0), P + "crate/", 1, (0, 0, 3, 0)),
                                       O((1, 3, 1, 4), P + "go().", 1, (1, 0, 2, 1)),
                                       O((1, 10, 1, 16), P + "a/helper().")])
    agg = rr.extract([lib, t], {"src/a.rs": SRC, "tests/x.rs": "\nfn go() { helper() }\n\n"}.get, _kg())
    assert dict(agg) == {(("File", "tests/x.rs"), ("RustFn", "rucene::a::helper")): [0, 1]}


def test_a_macro_generated_item_resolves_by_qualified_name():
    kg = rr.KgIndex([], [("rucene::h::list::List::add", "src/h/list.rs", "List", "add")], [])
    assert rr._target(P + "h/list/impl#[List]add().", "src/x.rs", kg, {}) == \
        ("RustFn", "rucene::h::list::List::add")


def test_macro_bodies_credit_the_invocation_file_lexically():
    macros = "macro_rules! make {\n    ($n:ident) => { fn g() { Util::grow(); let _: Entry; } };\n}\n"
    O = scipio.Occurrence
    docs = [scipio.Document("src/m.rs", []),
            scipio.Document("src/x.rs", [O((0, 0, 0, 4), P + "m/make!")])]
    kg = rr.KgIndex([("RustStruct", "Entry", "src/x.rs", "rucene::x::Entry")],
                    [("rucene::u::Util::grow", "src/u.rs", "Util", "grow")], [])
    agg = rr.macro_mentions(docs, {"src/m.rs": macros, "src/x.rs": "make!(A);\n"}.get, kg)
    assert dict(agg) == {
        (("File", "src/x.rs"), ("RustFn", "rucene::u::Util::grow")): [1, 0],
        (("File", "src/x.rs"), ("RustStruct", "Entry", "src/x.rs")): [1, 0]}


# ---------------------------------------------------------------------------
# lucene_calls_kg
# ---------------------------------------------------------------------------

def test_descriptor_and_signature_params_agree():
    assert lc.descriptor_params("(Lorg/a/B$C;[IJ)V") == ["C", "int[]", "long"]
    assert lc.signature_params("void f(final Map<K, V> m, @Nullable B.C c, int... xs)") == \
        ["Map", "C", "int[]"]
    assert lc.signature_params("int size()") == []


def test_match_member_prefers_types_then_arity():
    cands = [("A#f(int x)", "f", ["int"]), ("A#f(long x)", "f", ["long"]),
             ("A#g(T t)", "g", ["T"]), ("A#A(int n)", "A", ["int"])]
    assert lc.match_member(cands, "f", ["long"]) == (["A#f(long x)"], "exact")
    assert lc.match_member(cands, "g", ["Object"]) == (["A#g(T t)"], "exact")
    assert lc.match_member(cands, "A", ["String", "int", "int"], is_ctor=True) == (["A#A(int n)"], "synthetic")
    assert lc.match_member(cands, "h", []) == ([], None)


JAVAP = """\
Classfile jar:file:///x.jar!/org/apache/lucene/Base.class
  Compiled from "Base.java"
public abstract class org.apache.lucene.Base
  flags: (0x0421) ACC_PUBLIC, ACC_SUPER, ACC_ABSTRACT
  super_class: #2                         // java/lang/Object
{
  public abstract int next();
    descriptor: ()I
    flags: (0x0401) ACC_PUBLIC, ACC_ABSTRACT

}
Classfile jar:file:///x.jar!/org/apache/lucene/Impl.class
  Compiled from "Impl.java"
final class org.apache.lucene.Impl extends org.apache.lucene.Base
  flags: (0x0030) ACC_FINAL, ACC_SUPER
  super_class: #2                         // org/apache/lucene/Base
{
  org.apache.lucene.Impl();
    descriptor: ()V
    flags: (0x0000)
    Code:
         0: aload_0
         1: invokespecial #1                  // Method org/apache/lucene/Base."<init>":()V
         4: return

  public int next();
    descriptor: ()I
    flags: (0x0001) ACC_PUBLIC
    Code:
         0: iconst_0
         1: ireturn

}
Classfile jar:file:///x.jar!/org/apache/lucene/User.class
  Compiled from "User.java"
public class org.apache.lucene.User
  flags: (0x0021) ACC_PUBLIC, ACC_SUPER
  super_class: #2                         // java/lang/Object
{
  static int use(org.apache.lucene.Base);
    descriptor: (Lorg/apache/lucene/Base;)I
    flags: (0x0008) ACC_STATIC
    Code:
         0: aload_0
         1: invokevirtual #7                  // Method org/apache/lucene/Base.next:()I
         4: invokedynamic #9,  0              // InvokeDynamic #0:get:()Ljava/util/function/Supplier;
         9: ireturn

  private static java.lang.Object lambda$use$0();
    descriptor: ()Ljava/lang/Object;
    flags: (0x100a) ACC_PRIVATE, ACC_STATIC, ACC_SYNTHETIC
    Code:
         0: new           #3                  // class org/apache/lucene/Impl
         3: invokespecial #4                  // Method org/apache/lucene/Impl."<init>":()V
         6: areturn
}
SourceFile: "User.java"
BootstrapMethods:
  0: #30 REF_invokeStatic java/lang/invoke/LambdaMetafactory.metafactory:(...)
    Method arguments:
      #31 ()Ljava/lang/Object;
      #32 REF_invokeStatic org/apache/lucene/User.lambda$use$0:()Ljava/lang/Object;
      #33 ()Ljava/lang/Object;
"""


def test_bytecode_calls_overrides_and_lambda_credit():
    classes = lc.parse_javap(JAVAP)
    assert [c.name for c in classes] == ["org.apache.lucene.Base", "org.apache.lucene.Impl",
                                         "org.apache.lucene.User"]
    assert lc.top_level_visibility(classes) == {"org.apache.lucene.Base": "public",
                                                "org.apache.lucene.Impl": "package",
                                                "org.apache.lucene.User": "public"}
    hier = lc.Hierarchy(classes)
    kg = lc.KgMembers([
        ("org.apache.lucene.Base#next()", "org.apache.lucene.Base", "next", "method", "int next()"),
        ("org.apache.lucene.Impl#next()", "org.apache.lucene.Impl", "next", "method", "int next()"),
        ("org.apache.lucene.Impl#Impl()", "org.apache.lucene.Impl", "Impl", "constructor", "Impl()"),
        ("org.apache.lucene.User#use(Base b)", "org.apache.lucene.User", "use", "method", "int use(Base b)"),
    ], ["org.apache.lucene.Base", "org.apache.lucene.Impl", "org.apache.lucene.User"])
    calls, overrides, _ext, _stats = lc.build(classes, hier, kg, {})
    use = "org.apache.lucene.User#use(Base b)"
    assert calls[("Method", use, "org.apache.lucene.Base#next()", "invoke")] == [1, "exact"]
    # the lambda body's `new Impl()` is credited to `use`
    assert ("Method", use, "org.apache.lucene.Impl#Impl()", "invoke") in calls
    assert overrides == {("org.apache.lucene.Impl#next()", "org.apache.lucene.Base#next()")}


# ---------------------------------------------------------------------------
# no_caller_kg: the member mapping
# ---------------------------------------------------------------------------

def test_candidates_follow_the_depth_measure_and_keep_acronyms_whole():
    assert {"utf8_length", "get_utf8_length"} <= nc.candidates("getUTF8Length", "method")
    assert nc.candidates("IndexWriter", "constructor") == {"new"}
    assert "bar" in nc.candidates("isBar", "method")


def test_map_members_owner_then_inherited_then_free_fn():
    members = {"L.A": [("L.A#fooBar()", "fooBar", "method"), ("L.A#A()", "A", "constructor")],
               "L.Base": [("L.Base#close()", "close", "method")],
               "L.U": [("L.U#grow(int n)", "grow", "method")],
               "L.C": [("L.C#execute()", "execute", "method")]}
    owner = ("RustStruct", "A", "src/a.rs")
    fns = [("r::A::foo_bar", "foo_bar", "src/a.rs", owner),
           ("r::A::new", "new", "src/a.rs", owner),
           ("r::A::close", "close", "src/a.rs", owner),
           ("r::grow", "grow", "src/u.rs", None),
           ("r::execute", "execute", "src/c.rs", None),
           ("r::A::only_rust", "only_rust", "src/a.rs", owner)]
    rows = nc.map_members(fns, {owner: ["L.A"]}, {"src/u.rs": ["L.U"]},
                          {"r::execute": [("L.C", "member", "execute")]},
                          members, {"L.A": ["L.Base"]})
    assert sorted(rows) == sorted([
        ("r::A::foo_bar", "L.A#fooBar()", "owner"),
        ("r::A::new", "L.A#A()", "owner"),
        ("r::A::close", "L.Base#close()", "inherited"),
        ("r::grow", "L.U#grow(int n)", "free-fn"),
        ("r::execute", "L.C#execute()", "declared")])


def test_grouped_variant_is_the_list_query_regrouped():
    q = nc.by_subsystem(nc.QUERY_FNS)
    assert q.count("RETURN ") == 1 and "WITH subsystem, class, count(*) AS items" in q


def test_the_model_records_the_queries_the_report_runs():
    from pathlib import Path
    model = (Path(nc.__file__).resolve().parents[2] / "knowledge-model.md").read_text()
    for q in (nc.QUERY_FNS, nc.QUERY_TYPES, nc.QUERY_BY_SUBSYSTEM_SUFFIX):
        assert q.strip() in model
