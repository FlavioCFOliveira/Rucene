//! The port correspondence, from the crate's own declarations:
//!
//! * `PORTS` (`evidence: "doc-comment"`) — an item's doc says `Equivalent to
//!   \`org.apache.lucene.X\`` (or `Port of …`, or a lead-in of that family), or
//!   the module's `| Rucene | Apache Lucene Core 10.5.0 |` table names it;
//! * `SKELETON_OF` / `PLANS_PORT` (`evidence: "skeleton-doc-comment"`) — the
//!   ``Skeleton of `X`; ported by rmp task #N.`` paragraph;
//! * `Divergence` + `DECLARED_IN` / `CONCERNS` / `DIVERGES_FROM` — the declared
//!   divergences (`CLAUDE.md` §14.5);
//! * `PORTS_MEMBER` (`evidence: "name-mapping"`) — a `RustFn` and the Lucene
//!   members its name maps to, through its owner's `PORTS` (and the owner's
//!   Lucene ancestors), a member claim, or the file's ported class (free fns).
//!
//! Curated edges (other `evidence` values) are read, never written.

use crate::graph::Graph;
use crate::java::JavaSide;
use crate::layers::{edge, props, s, Desired};
use crate::reconcile::{Edge, EdgeSlice, NodeSlice, Props};
use crate::rust_ast::{Item, RFile, RustSide};
use rayon::prelude::*;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde_json::{json, Value};
use std::collections::VecDeque;

/// Lucene-side index: types and members, from the Java AST or from the graph.
pub struct Lucene {
    classes: HashSet<String>,
    /// Simple name → qualified names.
    by_simple: HashMap<String, Vec<String>>,
    packages: HashSet<String>,
    /// Class → (member name → member qualified names), methods and constructors.
    members: HashMap<String, HashMap<String, Vec<String>>>,
    /// Class → direct supertypes (EXTENDS + IMPLEMENTS).
    supers: HashMap<String, Vec<String>>,
    /// Class → its source file name (`Foo.java`), top-level types only.
    file_types: HashMap<String, Vec<String>>,
}

/// `cache`: where the graph-read rows are cached (keyed by the java layer's
/// fingerprint); `None` reads the graph.
pub fn lucene_index(
    g: &Graph,
    j: Option<&JavaSide>,
    cache: Option<&std::path::Path>,
) -> Result<Lucene, String> {
    let mut l = Lucene {
        classes: HashSet::default(),
        by_simple: HashMap::default(),
        packages: HashSet::default(),
        members: HashMap::default(),
        supers: HashMap::default(),
        file_types: HashMap::default(),
    };
    let add_class = |l: &mut Lucene, qn: &str, file: Option<&str>, top: bool| {
        l.classes.insert(qn.to_string());
        let simple = qn.rsplit(['.', '$']).next().unwrap_or(qn).to_string();
        l.by_simple.entry(simple).or_default().push(qn.to_string());
        if let Some(p) = qn.rsplit_once('.').map(|x| x.0) {
            if !p.contains('$') {
                l.packages.insert(p.to_string());
            }
        }
        if top {
            if let Some(f) = file {
                let fname = f.rsplit('/').next().unwrap_or(f).to_string();
                l.file_types.entry(fname).or_default().push(qn.to_string());
            }
        }
    };
    let add_member = |l: &mut Lucene, owner: &str, name: &str, qn: &str| {
        l.members
            .entry(owner.to_string())
            .or_default()
            .entry(name.to_string())
            .or_default()
            .push(qn.to_string());
    };
    match j {
        Some(j) => {
            let known: HashSet<String> = j
                .files
                .iter()
                .filter(|f| f.role == "src")
                .flat_map(|f| f.types.iter().map(|t| t.qn.clone()))
                .collect();
            let r = crate::java::Resolver { known: &known };
            for f in j.files.iter().filter(|f| f.role == "src") {
                for t in &f.types {
                    add_class(&mut l, &t.qn, Some(&f.rel), t.parent.is_none());
                    let sup: Vec<String> = t
                        .extends
                        .iter()
                        .chain(&t.implements)
                        .filter_map(|x| r.resolve(f, Some(&t.qn), x))
                        .collect();
                    l.supers.insert(t.qn.clone(), sup);
                }
                for m in &f.members {
                    if m.kind != "enumConstant" {
                        add_member(&mut l, &m.owner, &m.name, &m.qn);
                    }
                }
            }
        }
        None => {
            // Four independent reads, concurrently.
            let qs = [
                "MATCH (c:Class) RETURN c.qualifiedName, c.file, c.parentQualifiedName",
                "MATCH (m:Method) WHERE m.kind IN ['method','constructor','field'] RETURN m.parentQualifiedName, m.name, m.qualifiedName",
                "MATCH (a:Class)-[:EXTENDS]->(b:Class) RETURN a.qualifiedName, b.qualifiedName",
                "MATCH (a:Class)-[:IMPLEMENTS]->(b:Class) RETURN a.qualifiedName, b.qualifiedName",
            ];
            // The Lucene side changes only with the java layer's inputs: its rows
            // are cached on disk under that layer's fingerprint.
            let cached: Option<Vec<Vec<Vec<Value>>>> = cache
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .filter(|v: &Vec<Vec<Vec<Value>>>| v.len() == qs.len());
            let res: Vec<Result<Vec<Vec<Value>>, String>> = match cached {
                Some(v) => v.into_iter().map(Ok).collect(),
                None => {
                    let r: Vec<Result<Vec<Vec<Value>>, String>> =
                        qs.par_iter().map(|q| g.rows(q)).collect();
                    if let Some(p) = cache {
                        if r.iter().all(|x| x.is_ok()) {
                            let all: Vec<&Vec<Vec<Value>>> =
                                r.iter().map(|x| x.as_ref().unwrap()).collect();
                            if let Some(dir) = p.parent() {
                                let _ = std::fs::create_dir_all(dir);
                            }
                            let _ = std::fs::write(p, serde_json::to_vec(&all).unwrap_or_default());
                        }
                    }
                    r
                }
            };
            let mut res = res.into_iter();
            for r in res.next().unwrap()? {
                if let Some(q) = r.first().and_then(|v| v.as_str()) {
                    add_class(
                        &mut l,
                        q,
                        r.get(1).and_then(|v| v.as_str()),
                        r.get(2).is_none_or(|v| v.is_null()),
                    );
                }
            }
            for r in res.next().unwrap()? {
                if let (Some(o), Some(n), Some(q)) = (r[0].as_str(), r[1].as_str(), r[2].as_str()) {
                    add_member(&mut l, o, n, q);
                }
            }
            for rows in res {
                for r in rows? {
                    if let (Some(a), Some(b)) = (r[0].as_str(), r[1].as_str()) {
                        l.supers
                            .entry(a.to_string())
                            .or_default()
                            .push(b.to_string());
                    }
                }
            }
        }
    }
    Ok(l)
}

impl Lucene {
    fn empty() -> Lucene {
        Lucene {
            classes: HashSet::default(),
            by_simple: HashMap::default(),
            packages: HashSet::default(),
            members: HashMap::default(),
            supers: HashMap::default(),
            file_types: HashMap::default(),
        }
    }

    /// `a.b.C.D` / `a.b.C$D` → known class.
    fn fqn(&self, dotted: &str) -> Option<String> {
        let d = dotted.replace('$', ".");
        if self.classes.contains(dotted) {
            return Some(dotted.to_string());
        }
        let mut s = d.clone();
        if self.classes.contains(&s) {
            return Some(s);
        }
        while let Some(i) = s.rfind('.') {
            s.replace_range(i..i + 1, "$");
            if self.classes.contains(&s) {
                return Some(s);
            }
        }
        None
    }

    /// A doc reference (`org.x.Foo`, `org.x.Foo#bar(int)`, `org.x.Foo.bar()`,
    /// `Foo.Inner`) → (class, member).
    fn reference(&self, raw: &str, packages: &[String]) -> Option<(String, Option<String>)> {
        let r = raw.trim().trim_end_matches(['.', ',', ';', ':']);
        let r = r.split('(').next().unwrap_or(r).trim();
        // Drop type arguments: `Output<T>` → `Output`.
        let owned_r: String = {
            let mut out = String::new();
            let mut depth = 0;
            for c in r.chars() {
                match c {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    c if depth == 0 => out.push(c),
                    _ => {}
                }
            }
            out
        };
        let r = owned_r.trim();
        let (cls, member) = match r.split_once('#') {
            Some((c, m)) => (c, Some(m.to_string())),
            None => (r, None),
        };
        if cls.is_empty() || cls.contains(' ') {
            return None;
        }
        let candidates = |c: &str| -> Option<String> {
            if let Some(q) = self.fqn(c) {
                return Some(q);
            }
            for p in packages {
                if let Some(q) = self.fqn(&format!("{p}.{c}")) {
                    return Some(q);
                }
            }
            None
        };
        if let Some(q) = candidates(cls) {
            return Some((q, member));
        }
        // A simple name (`Foo`, `Foo.Inner`): its head unique, or unique inside the
        // packages the module names.
        if cls.chars().next().is_some_and(|c| c.is_uppercase()) {
            let (head, tail) = match cls.split_once('.') {
                Some((h, t)) => (h, format!("${}", t.replace('.', "$"))),
                None => (cls, String::new()),
            };
            if let Some(v) = self.by_simple.get(head) {
                let mut u: Vec<String> = v
                    .iter()
                    .map(|q| format!("{q}{tail}"))
                    .filter(|q| self.classes.contains(q))
                    .collect();
                u.sort();
                u.dedup();
                if u.len() == 1 {
                    return Some((u.remove(0), member));
                }
                let inp: Vec<&String> = u
                    .iter()
                    .filter(|q| packages.iter().any(|p| q.starts_with(&format!("{p}."))))
                    .collect();
                if inp.len() == 1 {
                    return Some((inp[0].clone(), member));
                }
            }
        }
        // `org.x.Foo.bar` — a member written with a dot.
        if member.is_none() {
            if let Some((c, m)) = cls.rsplit_once('.') {
                if m.chars().next().is_some_and(|ch| ch.is_lowercase()) {
                    if let Some(q) = candidates(c) {
                        return Some((q, Some(m.to_string())));
                    }
                }
            }
        }
        None
    }
}

/// A doc-comment block: consecutive `///` (outer) or `//!` (inner) lines.
struct DocBlock {
    inner: bool,
    /// (line number, text after the marker).
    lines: Vec<(usize, String)>,
    /// The documented declaration: (line, kind, name).
    decl: Option<(usize, &'static str, Option<String>)>,
}

fn doc_blocks(text: &str) -> Vec<DocBlock> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        let inner = t.starts_with("//!");
        let outer = t.starts_with("///") && !t.starts_with("////");
        if !inner && !outer {
            i += 1;
            continue;
        }
        let marker = if inner { "//!" } else { "///" };
        let mut b = DocBlock {
            inner,
            lines: Vec::new(),
            decl: None,
        };
        while i < lines.len() {
            let t = lines[i].trim_start();
            if !(t.starts_with(marker) && (inner || !t.starts_with("////"))) {
                break;
            }
            let rest = &t[3..];
            b.lines
                .push((i + 1, rest.strip_prefix(' ').unwrap_or(rest).to_string()));
            i += 1;
        }
        if !inner {
            // Skip attributes (possibly multi-line), plain comments and blank lines.
            let mut k = i;
            let mut depth: i32 = 0;
            while k < lines.len() {
                let t = lines[k].trim();
                if depth > 0 {
                    depth += t.matches('[').count() as i32 - t.matches(']').count() as i32;
                    k += 1;
                    continue;
                }
                if t.is_empty() || (t.starts_with("//") && !t.starts_with("///")) {
                    k += 1;
                    continue;
                }
                if t.starts_with("#[") {
                    depth = t.matches('[').count() as i32 - t.matches(']').count() as i32;
                    k += 1;
                    continue;
                }
                break;
            }
            if k < lines.len() {
                b.decl = Some(decl_of(k + 1, lines[k]));
            }
        }
        out.push(b);
    }
    out
}

/// Lexical kind and name of a declaration line.
fn decl_of(line: usize, l: &str) -> (usize, &'static str, Option<String>) {
    let mut t = l.trim();
    if let Some(r) = t.strip_prefix("pub") {
        t = r.trim_start();
        if t.starts_with('(') {
            if let Some(e) = t.find(')') {
                t = t[e + 1..].trim_start();
            }
        }
    }
    let mut words = t.split(|c: char| {
        c.is_whitespace()
            || c == '<'
            || c == '('
            || c == '{'
            || c == ':'
            || c == ';'
            || c == '='
            || c == ','
    });
    let mut kind = None;
    let mut name = None;
    for w in words.by_ref() {
        match w {
            "" | "unsafe" | "async" | "const" if kind.is_none() && w != "const" => continue,
            "extern" | "default" => continue,
            "fn" => kind = Some("fn"),
            "struct" => kind = Some("struct"),
            "union" => kind = Some("struct"),
            "enum" => kind = Some("enum"),
            "trait" => kind = Some("trait"),
            "type" => kind = Some("type"),
            "impl" => {
                return (line, "impl", None);
            }
            "const" => kind = Some("const"),
            "static" => kind = Some("static"),
            "mod" => kind = Some("mod"),
            "macro_rules!" => kind = Some("macro"),
            w if kind.is_some() => {
                if w == "mut" || w == "\"C\"" {
                    continue;
                }
                name = Some(w.trim_start_matches("r#").to_string());
                break;
            }
            w => {
                // A field (`name: T`) or an enum variant (`Name`, `Name(…)`, `Name {`).
                let first = w.trim_start_matches("r#");
                if first.chars().all(|c| c.is_alphanumeric() || c == '_') && !first.is_empty() {
                    return (line, "member", Some(first.to_string()));
                }
                return (line, "unknown", None);
            }
        }
        if kind.is_some() && name.is_some() {
            break;
        }
    }
    match kind {
        Some(k) => (line, k, name),
        None => (line, "unknown", None),
    }
}

/// Paragraphs of a block: (first line, joined text).
fn paragraphs(b: &DocBlock) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut cur: Option<(usize, String)> = None;
    for (ln, t) in &b.lines {
        if t.trim().is_empty() {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            continue;
        }
        match &mut cur {
            Some((_, s)) => {
                s.push(' ');
                s.push_str(t.trim());
            }
            None => cur = Some((*ln, t.trim().to_string())),
        }
    }
    if let Some(c) = cur {
        out.push(c);
    }
    out
}

/// The leads that make a doc paragraph a port claim.
const CLAIM_LEADS: &[&str] = &[
    "Equivalent to",
    "Port of",
    "Ported from",
    "Rust port of",
    "Rust equivalent of",
    "The declaration of",
    "The class declaration of",
    "Lucene Core equivalent",
    "Lucene equivalent",
];

fn first_backtick(s: &str) -> Option<&str> {
    let a = s.find('`')?;
    let rest = &s[a + 1..];
    let b = rest.find('`')?;
    Some(&rest[..b])
}

/// Every backticked span of a text.
fn backticks(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find('`') {
        let r = &rest[a + 1..];
        match r.find('`') {
            Some(b) => {
                out.push(&r[..b]);
                rest = &r[b + 1..];
            }
            None => break,
        }
    }
    out
}

/// Java package names a module doc mentions (context for simple names).
fn doc_packages(blocks: &[DocBlock], l: &Lucene) -> Vec<String> {
    let mut v = Vec::new();
    for b in blocks.iter().filter(|b| b.inner) {
        for (_, t) in &b.lines {
            for bt in backticks(t) {
                if l.packages.contains(bt) && !v.contains(&bt.to_string()) {
                    v.push(bt.to_string());
                }
            }
        }
    }
    v
}

/// A declared divergence: (id, properties, the item it concerns, cited Lucene
/// classes with their lines).
type PortRow = (String, &'static str, String, Props);

type DivergenceRow = (
    String,
    Props,
    Option<(String, &'static str)>,
    Vec<(String, String)>,
);

struct FileMap {
    /// (rust qn, rust label, class, props).
    ports: Vec<PortRow>,
    /// (rust qn, rust label, class, props, task).
    skeletons: Vec<(String, &'static str, String, Props, i64)>,
    divergences: Vec<DivergenceRow>,
}

/// The reference of a port claim: the first backticked span after a claim
/// phrase in the paragraph's first sentence ("Equivalent to `X`", "…, ported
/// from `X`", "Lucene Core equivalent: `X`", …).
fn lead_claim(par: &str) -> Option<&str> {
    let first = match par.find(". ") {
        Some(i) => &par[..i],
        None => par,
    };
    let lower = first.to_lowercase();
    let mut best: Option<usize> = None;
    for lead in CLAIM_LEADS {
        if let Some(i) = lower.find(&lead.to_lowercase()) {
            let end = i + lead.len();
            best = Some(best.map_or(end, |b: usize| b.min(end)));
        }
    }
    let at = best?;
    // Only a short qualifier may sit between the phrase and the reference.
    let rest = &first[at..];
    let tick = rest.find('`')?;
    if rest[..tick].split_whitespace().count() > 6 {
        return None;
    }
    first_backtick(rest)
}

fn file_mapping(f: &RFile, l: &Lucene) -> FileMap {
    let mut fm = FileMap {
        ports: Vec::new(),
        skeletons: Vec::new(),
        divergences: Vec::new(),
    };
    let blocks = doc_blocks(&f.text);
    let pkgs = doc_packages(&blocks, l);
    // Items by the line of their name (where the documented declaration sits).
    let mut by_line: HashMap<usize, Vec<&Item>> = HashMap::default();
    for it in &f.items {
        by_line.entry(it.line).or_default().push(it);
    }
    let item_at = |line: usize, name: Option<&str>| -> Option<&Item> {
        let v = by_line.get(&line)?;
        v.iter()
            .copied()
            .find(|it| name.is_none_or(|n| it.name == n))
    };
    for b in &blocks {
        let pars = paragraphs(b);
        let target = b
            .decl
            .as_ref()
            .and_then(|(ln, _, n)| item_at(*ln, n.as_deref()));
        // Port claims of an item's doc.
        if let Some(it) = target {
            for (ln, p) in &pars {
                if let Some(r) = lead_claim(p) {
                    if let Some((c, m)) = l.reference(r, &pkgs) {
                        let mut pr = props(&[
                            ("evidence", s("doc-comment")),
                            ("declaredAt", s(&format!("{}:{ln}", f.path))),
                        ]);
                        match &m {
                            Some(m) => {
                                pr.insert("claimLevel".into(), s("member"));
                                pr.insert(
                                    "declaredMember".into(),
                                    s(m.split('(').next().unwrap_or(m)),
                                );
                            }
                            None => {
                                pr.insert("claimLevel".into(), s("type"));
                            }
                        }
                        fm.ports.push((it.qn.clone(), it.label, c, pr));
                        break;
                    }
                }
            }
            for (ln, p) in &pars {
                if let Some(rest) = p.strip_prefix("Skeleton of `") {
                    let Some(end) = rest.find('`') else { continue };
                    let Some((c, _)) = l.reference(&rest[..end], &pkgs) else {
                        continue;
                    };
                    let task = rest[end..]
                        .split('#')
                        .nth(1)
                        .and_then(|x| {
                            x.trim_end_matches(|ch: char| !ch.is_ascii_digit())
                                .split(|ch: char| !ch.is_ascii_digit())
                                .next()
                        })
                        .and_then(|x| x.parse::<i64>().ok());
                    if let Some(t) = task {
                        let pr = props(&[
                            ("evidence", s("doc-comment")),
                            ("declaredAt", s(&format!("{}:{ln}", f.path))),
                            ("task", json!(t)),
                        ]);
                        fm.skeletons.push((it.qn.clone(), it.label, c, pr, t));
                    }
                }
            }
        }
        // A module-level claim lands on the file's item named like the class.
        if b.inner {
            for (ln, p) in &pars {
                let Some(r) = lead_claim(p) else { continue };
                let Some((c, None)) = l.reference(r, &pkgs) else {
                    continue;
                };
                let simple = c.rsplit(['.', '$']).next().unwrap_or(&c);
                if let Some(it) = f
                    .items
                    .iter()
                    .filter(|it| it.name == simple)
                    .min_by_key(|it| ((it.label == "RustFn") as u8, it.line))
                {
                    let pr = props(&[
                        ("evidence", s("doc-comment")),
                        ("declaredAt", s(&format!("{}:{ln}", f.path))),
                        ("claimLevel", s("type")),
                    ]);
                    fm.ports.push((it.qn.clone(), it.label, c.clone(), pr));
                }
            }
        }
        // Module tables: `| Rucene | Apache Lucene Core 10.5.0 |`.
        if b.inner {
            let mut in_table = false;
            for (ln, t) in &b.lines {
                let tt = t.trim();
                if tt.starts_with("| Rucene") {
                    in_table = true;
                    continue;
                }
                if !in_table {
                    continue;
                }
                if !tt.starts_with('|') {
                    in_table = false;
                    continue;
                }
                let cells: Vec<&str> = tt.trim_matches('|').split('|').map(str::trim).collect();
                if cells.len() < 2 || cells[0].starts_with("---") {
                    continue;
                }
                let rust: Vec<&str> = backticks(cells[0]);
                let java: Vec<&str> = backticks(cells[1]);
                let (Some(rn), Some(jn)) = (rust.first(), java.first()) else {
                    continue;
                };
                let rn = rn.trim_end_matches("()").rsplit("::").next().unwrap_or(rn);
                let Some(it) = f
                    .items
                    .iter()
                    .filter(|it| it.name == rn)
                    .min_by_key(|it| (it.label == "RustFn") as u8)
                else {
                    continue;
                };
                let simple_ok = |q: &String| pkgs.iter().any(|p| q.starts_with(&format!("{p}.")));
                let resolved = l.reference(jn, &pkgs).or_else(|| {
                    let c: Vec<&String> = l
                        .by_simple
                        .get(*jn)?
                        .iter()
                        .filter(|q| simple_ok(q))
                        .collect();
                    if c.len() == 1 {
                        Some((c[0].clone(), None))
                    } else {
                        None
                    }
                });
                if let Some((c, _)) = resolved {
                    let pr = props(&[
                        ("evidence", s("doc-comment")),
                        ("declaredAt", s(&format!("{}:{ln}", f.path))),
                        ("claimLevel", s("type")),
                    ]);
                    fm.ports.push((it.qn.clone(), it.label, c, pr));
                }
            }
        }
        // Declared divergences.
        let mut heading: Option<(usize, String, String)> = None; // (line, title, text)
        let flush_heading = |h: &mut Option<(usize, String, String)>, fm: &mut FileMap| {
            if let Some((ln, title, text)) = h.take() {
                fm.divergences
                    .push(divergence(f, b, ln, &title, &text, target, l));
            }
        };
        for (ln, p) in &pars {
            let pt = p.trim();
            if let Some(h) = pt.strip_prefix('#') {
                let title = h.trim_start_matches('#').trim().to_string();
                flush_heading(&mut heading, &mut fm);
                if title.starts_with("Divergence") {
                    heading = Some((*ln, title, String::new()));
                }
                continue;
            }
            if let Some((_, _, text)) = &mut heading {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(pt);
                continue;
            }
            let bold = pt
                .strip_prefix("**")
                .filter(|r| r.starts_with("Divergence"));
            if let Some(r) = bold {
                let title = r
                    .split("**")
                    .next()
                    .unwrap_or("")
                    .trim_end_matches([':', '.'])
                    .trim()
                    .to_string();
                fm.divergences
                    .push(divergence(f, b, *ln, &title, pt, target, l));
            } else if pt.starts_with("Divergence:") {
                fm.divergences
                    .push(divergence(f, b, *ln, "Divergence", pt, target, l));
            }
        }
        flush_heading(&mut heading, &mut fm);
    }
    fm
}

fn divergence(
    f: &RFile,
    b: &DocBlock,
    line: usize,
    title: &str,
    text: &str,
    target: Option<&Item>,
    l: &Lucene,
) -> DivergenceRow {
    let id = format!("{}:{line}", f.path);
    let (kind, name) = if b.inner {
        ("module", None)
    } else {
        match &b.decl {
            Some((_, k, n)) => (*k, n.clone()),
            None => ("unknown", None),
        }
    };
    let txt: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let txt: String = txt.chars().take(1500).collect();
    // `File.java[:lines]` citations.
    let mut refs: Vec<String> = Vec::new();
    let mut cited: Vec<(String, String)> = Vec::new();
    for w in txt.split(|c: char| {
        c.is_whitespace() || c == '`' || c == '(' || c == ')' || c == ',' || c == ';'
    }) {
        if let Some(i) = w.find(".java") {
            let fname = &w[..i + 5];
            let fname = fname.rsplit('/').next().unwrap_or(fname);
            if fname.len() <= 5 || !fname.chars().next().is_some_and(|c| c.is_uppercase()) {
                continue;
            }
            let lines = w[i + 5..]
                .strip_prefix(':')
                .map(|x| x.trim_end_matches(['.', ':']).to_string())
                .unwrap_or_default();
            let r = if lines.is_empty() {
                fname.to_string()
            } else {
                format!("{fname}:{lines}")
            };
            if !refs.contains(&r) {
                refs.push(r);
            }
            if let Some(types) = l.file_types.get(fname) {
                let top: Vec<&String> = types.iter().filter(|q| !q.contains('$')).collect();
                if top.len() == 1 {
                    cited.push((top[0].clone(), lines));
                }
            }
        }
    }
    let p = props(&[
        ("file", s(&f.path)),
        ("line", json!(line)),
        ("title", s(title)),
        ("text", s(&txt)),
        ("itemKind", s(kind)),
        (
            "item",
            name.clone().map(Value::String).unwrap_or(Value::Null),
        ),
        ("javaRefs", s(&refs.join("; "))),
    ]);
    let concerns = if b.inner {
        None
    } else {
        target.map(|it| (it.qn.clone(), it.label))
    };
    (id, p, concerns, cited)
}

fn camel(snake: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for (i, c) in snake.trim_start_matches("r#").chars().enumerate() {
        if c == '_' {
            if i > 0 {
                up = true;
            }
            continue;
        }
        if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `spec_only`: declare the slices with no data (no source or graph read), so
/// their current state can be read while the sources are being parsed.
pub fn layer(
    g: &Graph,
    d: &mut Desired,
    r: &RustSide,
    j: Option<&JavaSide>,
    spec_only: bool,
    pre: Option<Lucene>,
) -> Result<(), String> {
    let l = if spec_only {
        Lucene::empty()
    } else {
        match pre {
            Some(l) => l,
            None => lucene_index(g, j, None)?,
        }
    };
    let maps: Vec<FileMap> = if spec_only {
        Vec::new()
    } else {
        r.files.par_iter().map(|f| file_mapping(f, &l)).collect()
    };

    const SRC: [&str; 5] = ["RustStruct", "RustTrait", "RustEnum", "RustAlias", "RustFn"];
    let mut ports: HashMap<&str, Vec<Edge>> = HashMap::default();
    let mut skel: HashMap<&str, Vec<Edge>> = HashMap::default();
    let mut plans: Vec<Edge> = Vec::new();
    let div_i = d.node_slice(NodeSlice {
        label: "Divergence",
        key: "id",
        owned: vec![
            "file", "line", "title", "text", "itemKind", "item", "javaRefs",
        ],
        filter: "true".into(),
    });
    let mut declared_in = Vec::new();
    let mut concerns: HashMap<&str, Vec<Edge>> = HashMap::default();
    let mut diverges = Vec::new();
    // Type-level PORTS of each Rust item (for the member mapping), and member claims.
    let mut item_ports: HashMap<String, Vec<String>> = HashMap::default();
    let mut member_claims: Vec<(String, String, String)> = Vec::new();
    for fm in &maps {
        for (q, lab, c, p) in &fm.ports {
            if p.get("claimLevel") == Some(&s("member")) {
                if let Some(m) = p.get("declaredMember").and_then(|v| v.as_str()) {
                    member_claims.push((q.clone(), c.clone(), m.to_string()));
                }
            } else {
                item_ports.entry(q.clone()).or_default().push(c.clone());
            }
            ports
                .entry(lab)
                .or_default()
                .push(edge(s(q), s(c), p.clone()));
        }
        for (q, lab, c, p, t) in &fm.skeletons {
            skel.entry(lab)
                .or_default()
                .push(edge(s(q), s(c), p.clone()));
            let mut pp = p.clone();
            pp.remove("task");
            pp.insert("evidence".into(), s("skeleton-doc-comment"));
            plans.push(edge(json!(t), s(c), pp));
        }
        for (id, p, conc, cited) in &fm.divergences {
            d.nodes[div_i].1.insert(id.clone(), (s(id), p.clone()));
            declared_in.push(edge(s(id), p["file"].clone(), Props::new()));
            if let Some((q, lab)) = conc {
                concerns
                    .entry(lab)
                    .or_default()
                    .push(edge(s(id), s(q), Props::new()));
            }
            for (c, lines) in cited {
                diverges.push(edge(
                    s(id),
                    s(c),
                    props(&[(
                        "lines",
                        if lines.is_empty() {
                            Value::Null
                        } else {
                            s(lines)
                        },
                    )]),
                ));
            }
        }
    }
    // Curated type-level PORTS also anchor the member mapping (read concurrently).
    let curated: Vec<Result<Vec<Vec<Value>>, String>> = if spec_only { Vec::new() } else { SRC.to_vec() }
        .par_iter()
        .map(|lab| {
            g.rows(&format!(
                "MATCH (a:{lab})-[e:PORTS]->(c:Class) WHERE NOT (e.evidence = 'doc-comment' AND e.declarationSource IS NULL) AND coalesce(e.claimLevel, 'type') = 'type' RETURN a.qualifiedName, c.qualifiedName"
            ))
        })
        .collect();
    for rows in curated {
        for row in rows? {
            if let (Some(a), Some(c)) = (row[0].as_str(), row[1].as_str()) {
                item_ports
                    .entry(a.to_string())
                    .or_default()
                    .push(c.to_string());
            }
        }
    }
    for lab in SRC {
        let i = d.edge_slice(EdgeSlice {
            rel: "PORTS",
            src: (lab, "qualifiedName"),
            dst: ("Class", "qualifiedName"),
            owned: vec!["evidence", "declaredAt", "claimLevel", "declaredMember"],
            filter: "e.evidence = 'doc-comment' AND e.declarationSource IS NULL".into(),
        });
        d.edges[i].1 = ports.remove(lab).unwrap_or_default();
        let i = d.edge_slice(EdgeSlice {
            rel: "SKELETON_OF",
            src: (lab, "qualifiedName"),
            dst: ("Class", "qualifiedName"),
            owned: vec!["evidence", "declaredAt", "task"],
            filter: "true".into(),
        });
        d.edges[i].1 = skel.remove(lab).unwrap_or_default();
    }
    let i = d.edge_slice(EdgeSlice {
        rel: "PLANS_PORT",
        src: ("Task", "id"),
        dst: ("Class", "qualifiedName"),
        owned: vec!["evidence", "declaredAt"],
        filter: "e.evidence = 'skeleton-doc-comment'".into(),
    });
    d.edges[i].1 = plans;
    let i = d.edge_slice(EdgeSlice {
        rel: "DECLARED_IN",
        src: ("Divergence", "id"),
        dst: ("File", "path"),
        owned: vec![],
        filter: "true".into(),
    });
    d.edges[i].1 = declared_in;
    for lab in SRC {
        let i = d.edge_slice(EdgeSlice {
            rel: "CONCERNS",
            src: ("Divergence", "id"),
            dst: (lab, "qualifiedName"),
            owned: vec![],
            filter: "true".into(),
        });
        d.edges[i].1 = concerns.remove(lab).unwrap_or_default();
    }
    let i = d.edge_slice(EdgeSlice {
        rel: "DIVERGES_FROM",
        src: ("Divergence", "id"),
        dst: ("Class", "qualifiedName"),
        owned: vec!["lines"],
        filter: "true".into(),
    });
    d.edges[i].1 = diverges;

    // PORTS_MEMBER: by name.
    let find =
        |class: &str, name: &str| -> Option<&Vec<String>> { l.members.get(class)?.get(name) };
    let java_names = |fname: &str, class: &str| -> Vec<String> {
        let simple = class.rsplit(['.', '$']).next().unwrap_or(class).to_string();
        if fname == "new" {
            vec![simple]
        } else {
            vec![camel(fname)]
        }
    };
    let mut pm: Vec<Edge> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::default();
    let mut push = |pm: &mut Vec<Edge>, f: &str, m: &str, via: &str| {
        if seen.insert((f.to_string(), m.to_string())) {
            pm.push(edge(
                s(f),
                s(m),
                props(&[("evidence", s("name-mapping")), ("via", s(via))]),
            ));
        }
    };
    for (f, c, m) in &member_claims {
        for q in find(c, m).into_iter().flatten() {
            push(&mut pm, f, q, "declared");
        }
    }
    // Owner type qn per fn (as resolved by the DECLARES of the rust layer).
    let mut type_qn: HashMap<(String, String), String> = HashMap::default();
    let mut file_classes: HashMap<&str, Vec<String>> = HashMap::default();
    for f in &r.files {
        for it in &f.items {
            if it.label != "RustFn" {
                type_qn.insert((it.module.clone(), it.name.clone()), it.qn.clone());
                if let Some(cs) = item_ports.get(&it.qn) {
                    file_classes
                        .entry(f.path.as_str())
                        .or_default()
                        .extend(cs.iter().cloned());
                }
            }
        }
    }
    for f in &r.files {
        for it in f.items.iter().filter(|it| it.label == "RustFn") {
            match &it.owner {
                Some(o) => {
                    let Some(tq) = type_qn.get(&(it.module.clone(), o.clone())) else {
                        continue;
                    };
                    let Some(classes) = item_ports.get(tq) else {
                        continue;
                    };
                    for c in classes {
                        let names = java_names(&it.name, c);
                        let direct: Vec<&String> =
                            names.iter().filter_map(|n| find(c, n)).flatten().collect();
                        if !direct.is_empty() {
                            for q in direct {
                                push(&mut pm, &it.qn, q, "owner");
                            }
                            continue;
                        }
                        // The nearest Lucene ancestor that declares the name.
                        let mut queue: VecDeque<String> =
                            l.supers.get(c).cloned().unwrap_or_default().into();
                        let mut visited: HashSet<String> = HashSet::default();
                        while let Some(a) = queue.pop_front() {
                            if !visited.insert(a.clone()) {
                                continue;
                            }
                            let hit: Vec<&String> = java_names(&it.name, &a)
                                .iter()
                                .filter_map(|n| find(&a, n))
                                .flatten()
                                .collect::<Vec<_>>();
                            if !hit.is_empty() {
                                for q in hit {
                                    push(&mut pm, &it.qn, q, "inherited");
                                }
                                break;
                            }
                            queue.extend(l.supers.get(&a).cloned().unwrap_or_default());
                        }
                    }
                }
                None => {
                    let Some(classes) = file_classes.get(f.path.as_str()) else {
                        continue;
                    };
                    let mut cs = classes.clone();
                    cs.sort();
                    cs.dedup();
                    for c in cs {
                        for q in find(&c, &camel(&it.name)).into_iter().flatten() {
                            push(&mut pm, &it.qn, q, "free-fn");
                        }
                    }
                }
            }
        }
    }
    let i = d.edge_slice(EdgeSlice {
        rel: "PORTS_MEMBER",
        src: ("RustFn", "qualifiedName"),
        dst: ("Method", "qualifiedName"),
        owned: vec!["evidence", "via"],
        filter: "e.evidence = 'name-mapping'".into(),
    });
    d.edges[i].1 = pm;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lucene() -> Lucene {
        let mut l = Lucene::empty();
        for q in [
            "org.apache.lucene.util.BytesRef",
            "org.apache.lucene.index.MergeState",
            "org.apache.lucene.index.MergeState$DocMap",
            "com.ibm.icu.util.Output",
        ] {
            l.classes.insert(q.to_string());
            let simple = q.rsplit(['.', '$']).next().unwrap().to_string();
            l.by_simple.entry(simple).or_default().push(q.to_string());
        }
        l.packages.insert("org.apache.lucene.util".into());
        l
    }

    #[test]
    fn camel_maps_snake_case_to_java_names() {
        assert_eq!(camel("get_word_set"), "getWordSet");
        assert_eq!(camel("r#type"), "type");
        assert_eq!(camel("utf8_to_string"), "utf8ToString");
    }

    #[test]
    fn claims_are_found_after_a_lead_in_the_first_sentence() {
        assert_eq!(
            lead_claim("Equivalent to `org.apache.lucene.util.BytesRef`."),
            Some("org.apache.lucene.util.BytesRef")
        );
        assert_eq!(
            lead_claim("Impact bounds, ported from `org.x.Y`."),
            Some("org.x.Y")
        );
        assert_eq!(
            lead_claim("Equivalent to the private static nested class `A.B`, which"),
            Some("A.B")
        );
        assert_eq!(
            lead_claim("Lucene Core equivalent: `org.x.Z`."),
            Some("org.x.Z")
        );
        assert_eq!(
            lead_claim("Reads values. Equivalent to `org.x.Y`."),
            None,
            "only the first sentence counts"
        );
        assert_eq!(lead_claim("No claim here `org.x.Y`."), None);
    }

    #[test]
    fn references_resolve_fqns_nested_types_members_and_simple_names() {
        let l = lucene();
        assert_eq!(
            l.reference("org.apache.lucene.index.MergeState.DocMap", &[]),
            Some(("org.apache.lucene.index.MergeState$DocMap".into(), None))
        );
        assert_eq!(
            l.reference("org.apache.lucene.util.BytesRef#utf8ToString()", &[]),
            Some((
                "org.apache.lucene.util.BytesRef".into(),
                Some("utf8ToString".into())
            ))
        );
        assert_eq!(
            l.reference("org.apache.lucene.util.BytesRef.deepCopyOf", &[]),
            Some((
                "org.apache.lucene.util.BytesRef".into(),
                Some("deepCopyOf".into())
            ))
        );
        assert_eq!(
            l.reference("com.ibm.icu.util.Output<T>", &[]),
            Some(("com.ibm.icu.util.Output".into(), None))
        );
        assert_eq!(
            l.reference("MergeState.DocMap", &[]),
            Some(("org.apache.lucene.index.MergeState$DocMap".into(), None))
        );
        assert_eq!(l.reference("NoSuchType", &[]), None);
    }

    #[test]
    fn declarations_are_classified_lexically() {
        assert_eq!(
            decl_of(3, "pub(crate) fn foo<T>(x: T) {"),
            (3, "fn", Some("foo".into()))
        );
        assert_eq!(
            decl_of(1, "pub struct Bar {"),
            (1, "struct", Some("Bar".into()))
        );
        assert_eq!(decl_of(1, "impl<T> Foo for Bar<T> {"), (1, "impl", None));
        assert_eq!(
            decl_of(1, "    pub field_name: u32,"),
            (1, "member", Some("field_name".into()))
        );
        assert_eq!(
            decl_of(1, "pub const MAX: usize = 3;"),
            (1, "const", Some("MAX".into()))
        );
    }

    #[test]
    fn doc_blocks_attach_to_the_declaration_after_attributes() {
        let text =
            "/// Equivalent to `X`.\n#[derive(Debug,\n  Clone)]\npub struct S;\n//! module doc\n";
        let b = doc_blocks(text);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].decl, Some((4, "struct", Some("S".into()))));
        assert!(b[1].inner);
    }
}
