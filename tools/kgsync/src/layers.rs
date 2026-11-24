//! Layer orchestration: each layer fills the desired state of the slices it
//! owns; node slices are reconciled first (their key sets resolve the edges),
//! then edge slices.

use crate::graph::Graph;
use crate::reconcile::{self, Edge, EdgeSlice, NodeSlice, Props, Report, Stamp};
use crate::rust_ast::{self, RustSide};
use crate::Opts;
use rayon::prelude::*;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde_json::{json, Value};

use std::time::Instant;

pub type NodeMap = HashMap<String, (Value, Props)>;

#[derive(Default)]
pub struct Desired {
    pub nodes: Vec<(NodeSlice, NodeMap)>,
    pub edges: Vec<(EdgeSlice, Vec<Edge>)>,
}

impl Desired {
    pub fn node_slice(&mut self, s: NodeSlice) -> usize {
        self.nodes.push((s, NodeMap::default()));
        self.nodes.len() - 1
    }
    pub fn edge_slice(&mut self, s: EdgeSlice) -> usize {
        self.edges.push((s, Vec::new()));
        self.edges.len() - 1
    }
}

pub fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

pub fn props(pairs: &[(&str, Value)]) -> Props {
    let mut p = Props::new();
    for (k, v) in pairs {
        p.insert((*k).to_string(), v.clone());
    }
    p
}

pub fn edge(src: Value, dst: Value, p: Props) -> Edge {
    Edge { src, dst, props: p }
}

/// Identity of every label: the property a UNIQUE constraint guards.
pub const IDENTITY: &[(&str, &str)] = &[
    ("Project", "name"),
    ("Module", "name"),
    ("Package", "name"),
    ("File", "path"),
    ("Class", "qualifiedName"),
    ("Method", "qualifiedName"),
    ("TestClass", "qualifiedName"),
    ("TestMethod", "qualifiedName"),
    ("TestResource", "path"),
    ("RustStruct", "qualifiedName"),
    ("RustTrait", "qualifiedName"),
    ("RustEnum", "qualifiedName"),
    ("RustAlias", "qualifiedName"),
    ("RustFn", "qualifiedName"),
    ("Component", "name"),
    ("Divergence", "id"),
    ("Task", "id"),
    ("Commit", "hash"),
    ("Decision", "name"),
    ("Defect", "name"),
    ("Feature", "name"),
];

pub fn identity(label: &str) -> &'static str {
    IDENTITY
        .iter()
        .find(|(l, _)| *l == label)
        .map(|(_, k)| *k)
        .unwrap_or("name")
}

/// Declares a UNIQUE constraint on every identity (its hash index serves the
/// seeks) and drops the plain indexes it supersedes.
pub fn schema(g: &Graph) -> Result<(), String> {
    let existing: HashSet<String> = g
        .rows("SHOW CONSTRAINTS")?
        .into_iter()
        .filter_map(|r| r.first().and_then(|v| v.as_str()).map(String::from))
        .collect();
    for (l, k) in IDENTITY {
        let name = format!("uniq_{}_{}", l.to_lowercase(), k.to_lowercase());
        if existing.contains(&name) {
            continue;
        }
        let dups = g.rows(&format!("MATCH (n:{l}) WITH n.{k} AS v, count(*) AS c WHERE v IS NOT NULL AND c > 1 RETURN count(v)"))?;
        if dups
            .first()
            .and_then(|r| r.first())
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0
        {
            return Err(format!(
                "{l}.{k} has duplicate values; constraint not declared"
            ));
        }
        g.ddl(&format!(
            "CREATE CONSTRAINT {name} IF NOT EXISTS FOR (n:{l}) REQUIRE n.{k} IS UNIQUE"
        ))?;
        println!("schema: constraint {name}");
    }
    for r in g.rows("SHOW INDEXES")? {
        let name = r.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !name.starts_with("idx_") {
            continue;
        }
        let label = r
            .get(4)
            .and_then(|v| v.get(0))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let prop = r
            .get(5)
            .and_then(|v| v.get(0))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Superseded: same property as the constraint's index, or a non-identity
        // property of a label whose identity is now guarded (Rust types' `name`).
        if IDENTITY.iter().any(|(l, _)| *l == label) {
            g.ddl(&format!("DROP INDEX {name} IF EXISTS"))?;
            println!(
                "schema: dropped index {name} ({label}.{prop}); identity served by its constraint"
            );
        }
    }
    Ok(())
}

const RUST_TYPES: [&str; 4] = ["RustStruct", "RustTrait", "RustEnum", "RustAlias"];

fn src_origin() -> String {
    "coalesce(n.origin, 'source') = 'source'".into()
}

/// The Rucene side: files, items, declarations, trait impls, `use` dependencies,
/// crate containment.
pub fn rust_layer(d: &mut Desired, r: &RustSide, crate_modules: &[(String, String)]) {
    let file_slice = d.node_slice(NodeSlice {
        label: "File",
        key: "path",
        owned: vec!["name", "kind", "language", "modulePath", "crate", "loc"],
        filter: "n.language = 'Rust'".into(),
    });
    let mut type_slices = HashMap::default();
    for l in RUST_TYPES {
        let i = d.node_slice(NodeSlice {
            label: l,
            key: "qualifiedName",
            owned: vec![
                "name",
                "file",
                "kind",
                "visibility",
                "scope",
                "origin",
                "language",
            ],
            filter: src_origin(),
        });
        type_slices.insert(l, i);
    }
    let fn_slice = d.node_slice(NodeSlice {
        label: "RustFn",
        key: "qualifiedName",
        owned: vec![
            "name",
            "file",
            "owner",
            "kind",
            "visibility",
            "scope",
            "signature",
            "origin",
            "language",
        ],
        filter: src_origin(),
    });

    // Type index for owner and trait resolution.
    let mut type_label: HashMap<String, &'static str> = HashMap::default();
    let mut by_crate_name: HashMap<(String, String), Vec<String>> = HashMap::default();
    let mut by_name: HashMap<String, Vec<String>> = HashMap::default();
    for f in &r.files {
        for it in &f.items {
            if it.label != "RustFn" {
                type_label.entry(it.qn.clone()).or_insert(it.label);
                let c = f.crate_name.clone().unwrap_or_default();
                by_crate_name
                    .entry((c, it.name.clone()))
                    .or_default()
                    .push(it.qn.clone());
                by_name
                    .entry(it.name.clone())
                    .or_default()
                    .push(it.qn.clone());
            }
        }
    }
    let resolve = |module: &str, krate: &str, name: &str, want: Option<&str>| -> Option<String> {
        let ok = |qn: &String| want.is_none_or(|w| type_label.get(qn) == Some(&w));
        let local = format!("{module}::{name}");
        if type_label.contains_key(&local) && ok(&local) {
            return Some(local);
        }
        let pick = |v: Option<&Vec<String>>| -> Option<String> {
            let c: Vec<&String> = v?.iter().filter(|q| ok(q)).collect();
            let mut u: Vec<&String> = c.clone();
            u.sort();
            u.dedup();
            if u.len() == 1 {
                Some(u[0].clone())
            } else {
                None
            }
        };
        pick(by_crate_name.get(&(krate.to_string(), name.to_string())))
            .or_else(|| pick(by_name.get(name)))
    };

    let mut declares: HashMap<(&str, &str), Vec<Edge>> = HashMap::default();
    let mut implements: HashMap<&str, Vec<Edge>> = HashMap::default();
    let mut deps: Vec<Edge> = Vec::new();
    let mut contains: Vec<Edge> = Vec::new();
    let crate_names: HashSet<&str> = r.crates.iter().map(|(c, _)| c.as_str()).collect();

    for f in &r.files {
        let file_name = f.path.rsplit('/').next().unwrap_or(&f.path);
        d.nodes[file_slice].1.insert(
            f.path.clone(),
            (
                s(&f.path),
                props(&[
                    ("name", s(file_name)),
                    ("kind", s(f.kind)),
                    ("language", s("Rust")),
                    (
                        "modulePath",
                        f.module_path
                            .clone()
                            .map(Value::String)
                            .unwrap_or(Value::Null),
                    ),
                    (
                        "crate",
                        f.crate_name
                            .clone()
                            .map(Value::String)
                            .unwrap_or(Value::Null),
                    ),
                    ("loc", json!(f.loc)),
                ]),
            ),
        );
        // Crate module containment: the longest crate path that prefixes the file.
        if let Some((m, _)) = crate_modules
            .iter()
            .filter(|(_, p)| p == "." || f.path.starts_with(&format!("{p}/")))
            .max_by_key(|(_, p)| if p == "." { 0 } else { p.len() })
        {
            contains.push(edge(s(m), s(&f.path), Props::new()));
        }
        let krate = f.crate_name.clone().unwrap_or_default();
        for it in &f.items {
            let mut p = props(&[
                ("name", s(&it.name)),
                ("file", s(&f.path)),
                ("kind", s(it.kind)),
                ("visibility", s(it.vis)),
                ("scope", s(it.scope)),
                ("origin", s("source")),
                ("language", s("Rust")),
            ]);
            if it.label == "RustFn" {
                p.insert(
                    "owner".into(),
                    it.owner.clone().map(Value::String).unwrap_or(Value::Null),
                );
                p.insert(
                    "signature".into(),
                    it.signature
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
                d.nodes[fn_slice]
                    .1
                    .entry(it.qn.clone())
                    .or_insert((s(&it.qn), p));
            } else {
                d.nodes[type_slices[it.label]]
                    .1
                    .entry(it.qn.clone())
                    .or_insert((s(&it.qn), p));
            }
            declares.entry(("File", it.label)).or_default().push(edge(
                s(&f.path),
                s(&it.qn),
                Props::new(),
            ));
            if let (Some(o), "RustFn") = (&it.owner, it.label) {
                let want = if it.kind == "trait-method" {
                    Some("RustTrait")
                } else {
                    None
                };
                if let Some(oq) = resolve(&it.module, &krate, o, want) {
                    let ol = type_label[&oq];
                    declares.entry((ol, "RustFn")).or_default().push(edge(
                        s(&oq),
                        s(&it.qn),
                        Props::new(),
                    ));
                }
            }
        }
        for im in &f.impls {
            let (Some(tq), Some(trq)) = (
                resolve(&im.module, &krate, &im.ty, None),
                resolve(&im.module, &krate, &im.tr, Some("RustTrait")),
            ) else {
                continue;
            };
            let tl = type_label[&tq];
            implements
                .entry(tl)
                .or_default()
                .push(edge(s(&tq), s(&trq), Props::new()));
        }
        // `use` → file dependencies.
        let mut seen = HashSet::default();
        for (module, segs) in &f.uses {
            if let Some(target) = resolve_use(module, &krate, segs, &crate_names, &r.module_file) {
                if target != f.path && seen.insert(target.clone()) {
                    deps.push(edge(s(&f.path), s(&target), Props::new()));
                }
            }
        }
    }

    for ((sl, dl), v) in declares {
        let filter = if sl == "File" {
            "coalesce(b.origin, 'source') = 'source'".to_string()
        } else {
            "coalesce(a.origin, 'source') = 'source' AND coalesce(b.origin, 'source') = 'source'"
                .to_string()
        };
        let i = d.edge_slice(EdgeSlice {
            rel: "DECLARES",
            src: (sl, identity(sl)),
            dst: (dl, identity(dl)),
            owned: vec![],
            filter,
        });
        d.edges[i].1 = v;
    }
    // Slices with no desired edge still need reconciling (to delete stale ones).
    for sl in ["File"].into_iter().chain(RUST_TYPES) {
        for dl in RUST_TYPES.into_iter().chain(["RustFn"]) {
            if (sl == "File" || dl == "RustFn")
                && !d
                    .edges
                    .iter()
                    .any(|(e, _)| e.rel == "DECLARES" && e.src.0 == sl && e.dst.0 == dl)
            {
                let filter = "coalesce(b.origin, 'source') = 'source'".to_string();
                d.edge_slice(EdgeSlice {
                    rel: "DECLARES",
                    src: (sl, identity(sl)),
                    dst: (dl, identity(dl)),
                    owned: vec![],
                    filter,
                });
            }
        }
    }
    for sl in RUST_TYPES {
        let i = d.edge_slice(EdgeSlice {
            rel: "IMPLEMENTS",
            src: (sl, "qualifiedName"),
            dst: ("RustTrait", "qualifiedName"),
            owned: vec![],
            filter: "coalesce(a.origin, 'source') = 'source' AND coalesce(b.origin, 'source') = 'source'".into(),
        });
        d.edges[i].1 = implements.remove(sl).unwrap_or_default();
    }
    let i = d.edge_slice(EdgeSlice {
        rel: "DEPENDS_ON",
        src: ("File", "path"),
        dst: ("File", "path"),
        owned: vec![],
        filter: "a.language = 'Rust' AND b.language = 'Rust'".into(),
    });
    d.edges[i].1 = deps;
    let i = d.edge_slice(EdgeSlice {
        rel: "CONTAINS",
        src: ("Module", "name"),
        dst: ("File", "path"),
        owned: vec![],
        filter: "a.kind = 'crate' AND b.language = 'Rust'".into(),
    });
    d.edges[i].1 = contains;
}

/// Resolves a `use` path to the file of the longest module prefix it names.
fn resolve_use(
    module: &str,
    krate: &str,
    segs: &[String],
    crates: &HashSet<&str>,
    mf: &HashMap<String, String>,
) -> Option<String> {
    let first = segs.first()?;
    let mut base: Vec<String> = match first.as_str() {
        "crate" => vec![krate.to_string()],
        "self" => module.split("::").map(String::from).collect(),
        "super" => {
            let mut m: Vec<String> = module.split("::").map(String::from).collect();
            m.pop();
            m
        }
        "std" | "core" | "alloc" => return None,
        c if crates.contains(c) => vec![c.to_string()],
        _ => {
            // A path relative to the current module (2018 edition `use` of a child).
            let mut m: Vec<String> = module.split("::").map(String::from).collect();
            m.push(first.clone());
            if !mf.contains_key(&m.join("::")) {
                return None;
            }
            m.pop();
            m
        }
    };
    let mut rest = &segs[if matches!(first.as_str(), "crate" | "self")
        || crates.contains(first.as_str()) && first != krate
        || first == krate
    {
        1
    } else {
        0
    }..];
    if first == "super" {
        rest = &segs[1..];
        let mut r = rest;
        while r.first().is_some_and(|x| x == "super") {
            base.pop();
            r = &r[1..];
        }
        rest = r;
    }
    let mut best = mf.get(&base.join("::")).cloned();
    let mut cur = base;
    for seg in rest {
        cur.push(seg.clone());
        match mf.get(&cur.join("::")) {
            Some(f) => best = Some(f.clone()),
            None => break,
        }
    }
    best
}

pub struct Prefetch {
    nodes: HashMap<String, reconcile::NodeCurrent>,
    edges: HashMap<String, reconcile::EdgeCurrent>,
    keys: HashMap<&'static str, HashSet<String>>,
}

fn node_sig(s: &NodeSlice) -> String {
    format!("{}|{}|{}|{}", s.label, s.key, s.filter, s.owned.join(","))
}

fn edge_sig(s: &EdgeSlice) -> String {
    format!(
        "{}|{:?}|{:?}|{}|{}",
        s.rel,
        s.src,
        s.dst,
        s.filter,
        s.owned.join(",")
    )
}

/// Reads the current state of every declared slice, concurrently.
fn prefetch_all(g: &Graph, spec: &Desired) -> Result<Prefetch, String> {
    let nodes: Vec<(String, Result<reconcile::NodeCurrent, String>)> = spec
        .nodes
        .par_iter()
        .map(|(sl, _)| (node_sig(sl), reconcile::read_nodes(g, sl)))
        .collect();
    let edges: Vec<(String, Result<reconcile::EdgeCurrent, String>)> = spec
        .edges
        .par_iter()
        .map(|(sl, _)| (edge_sig(sl), reconcile::read_edges(g, sl)))
        .collect();
    let node_labels: HashSet<&'static str> = spec.nodes.iter().map(|(sl, _)| sl.label).collect();
    let mut missing: Vec<&'static str> = spec
        .edges
        .iter()
        .flat_map(|(e, _)| [e.src.0, e.dst.0])
        .filter(|l| !node_labels.contains(l))
        .collect();
    missing.sort();
    missing.dedup();
    let keys: Vec<(&'static str, Result<HashSet<String>, String>)> =
        missing.par_iter().map(|l| (*l, read_keys(g, l))).collect();
    let mut p = Prefetch {
        nodes: HashMap::default(),
        edges: HashMap::default(),
        keys: HashMap::default(),
    };
    for (k, r) in nodes {
        p.nodes.insert(k, r?);
    }
    for (k, r) in edges {
        p.edges.insert(k, r?);
    }
    for (k, r) in keys {
        p.keys.insert(k, r?);
    }
    Ok(p)
}

fn read_keys(g: &Graph, label: &str) -> Result<HashSet<String>, String> {
    let k = identity(label);
    Ok(g.rows(&format!("MATCH (n:{label}) RETURN n.{k}"))?
        .into_iter()
        .filter_map(|r| r.into_iter().next())
        .filter(|v| !v.is_null())
        .map(|v| match v {
            Value::String(s) => s,
            o => o.to_string(),
        })
        .collect())
}

pub fn run(g: &Graph, o: &Opts, stamp: &Stamp) -> Result<Vec<Report>, String> {
    let asked = |l: &str| o.layers.iter().any(|x| x == l);
    let t = Instant::now();

    // Fingerprints: a layer whose inputs are unchanged since the last sync is
    // skipped (its slices are already the desired state); --full re-derives all.
    let stored: HashMap<String, String> = g
        .rows("MATCH (p:Project {name:'Rucene'}) RETURN p.kgsyncRust, p.kgsyncJava, p.kgsyncMap, p.kgsyncGit")?
        .into_iter()
        .next()
        .map(|r| {
            ["rust", "java", "map", "git"]
                .iter()
                .zip(r)
                .filter_map(|(k, v)| v.as_str().map(|x| (k.to_string(), x.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let fp_rust = rust_ast::fingerprint(&o.repo);
    let jplan = if asked("java") || asked("map") {
        Some(crate::java::plan(g, &o.repo)?)
    } else {
        None
    };
    let fp_java = jplan
        .as_ref()
        .map(|p| p.fingerprint.clone())
        .unwrap_or_default();
    let fp_map = format!("{fp_rust}:{fp_java}");
    let lucene_cache = o.repo.parent().unwrap_or(&o.repo).join(format!(
        ".rucene-kg-target/kgsync-cache/lucene-{fp_java}.json"
    ));
    let fp_git = format!(
        "{}:{}",
        stamp.commit,
        crate::gitlayer::tasks_fingerprint(&o.roadmap)
    );
    let mut fps: Vec<(&str, String)> = Vec::new();
    let run_layer = |l: &str, fp: &str| -> bool {
        if !asked(l) {
            return false;
        }
        let same = stored.get(l).map(|x| x == fp).unwrap_or(false);
        if same && !o.full {
            if o.verbose {
                eprintln!("layer {l}: inputs unchanged, skipped");
            }
            return false;
        }
        true
    };
    let do_rust = run_layer("rust", &fp_rust);
    let do_java = run_layer("java", &fp_java);
    let do_map = run_layer("map", &fp_map);
    let do_git = run_layer("git", &fp_git);
    for (l, f, on) in [
        ("rust", &fp_rust, do_rust),
        ("java", &fp_java, do_java),
        ("map", &fp_map, do_map),
        ("git", &fp_git, do_git),
    ] {
        if on {
            fps.push((l, f.clone()));
        }
    }
    let want = |l: &str| match l {
        "rust" => do_rust,
        "java" => do_java,
        "map" => do_map,
        "git" => do_git,
        other => asked(other),
    };

    // The slices every layer will reconcile, declared with no data: their
    // current state is read while the sources are parsed.
    let crate_modules: Vec<(String, String)> = if do_rust {
        g.rows("MATCH (m:Module) WHERE m.kind = 'crate' RETURN m.name, m.path")?
            .into_iter()
            .filter_map(|r| {
                Some((
                    r.first()?.as_str()?.to_string(),
                    r.get(1)?.as_str()?.to_string(),
                ))
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut spec = Desired::default();
    let empty_rust = RustSide::default();
    if do_rust {
        rust_layer(&mut spec, &empty_rust, &crate_modules);
    }
    if do_java {
        crate::java::layer(&mut spec, &crate::java::JavaSide::default());
    }
    if do_map {
        crate::mapping::layer(g, &mut spec, &empty_rust, None, true, None)?;
    }
    if do_git {
        crate::gitlayer::layer(g, &mut spec, &o.repo, &o.roadmap, true)?;
    }

    // 1. Read (graph) ∥ extract + fill (sources), concurrently.
    let t_read = Instant::now();
    let spec_ref = &spec;
    let (prefetch, filled) = rayon::join(
        || prefetch_all(g, spec_ref),
        || -> Result<Desired, String> {
            let need_rust = want("rust") || want("map");
            // The Lucene index the mapping needs comes from the graph when the
            // Java layer is skipped: it is read while the sources are parsed.
            let ((rust, java), lucene) = rayon::join(
                || {
                    rayon::join(
                        || {
                            if need_rust {
                                Some(rust_ast::extract(&o.repo))
                            } else {
                                None
                            }
                        },
                        || match (want("java"), jplan) {
                            (true, Some(p)) => Some(crate::java::extract(g, &o.repo, p)),
                            _ => None,
                        },
                    )
                },
                || {
                    if want("map") && !want("java") {
                        Some(crate::mapping::lucene_index(g, None, Some(&lucene_cache)))
                    } else {
                        None
                    }
                },
            );
            let lucene = match lucene {
                Some(l) => Some(l?),
                None => None,
            };
            let java = match java {
                Some(j) => Some(j?),
                None => None,
            };
            if o.verbose {
                eprintln!("extract: {:.2} s", t.elapsed().as_secs_f64());
            }
            if let Some(r) = &rust {
                let errs: Vec<&str> = r
                    .files
                    .iter()
                    .filter(|f| f.parse_error.is_some())
                    .map(|f| f.path.as_str())
                    .collect();
                if !errs.is_empty() {
                    eprintln!(
                        "kgsync: {} Rust files did not parse (File node only): {}",
                        errs.len(),
                        errs.join(", ")
                    );
                }
            }
            let mut d = Desired::default();
            if want("rust") {
                rust_layer(&mut d, rust.as_ref().unwrap(), &crate_modules);
            }
            if let Some(j) = &java {
                crate::java::layer(&mut d, j);
            }
            if want("map") {
                crate::mapping::layer(
                    g,
                    &mut d,
                    rust.as_ref().unwrap(),
                    java.as_ref(),
                    false,
                    lucene,
                )?;
            }
            if want("git") {
                crate::gitlayer::layer(g, &mut d, &o.repo, &o.roadmap, false)?;
            }
            if o.verbose {
                eprintln!("desired state: {:.2} s", t.elapsed().as_secs_f64());
            }
            Ok(d)
        },
    );
    let mut d = filled?;
    let mut prefetch = prefetch?;
    if o.verbose {
        eprintln!("read ∥ extract: {:.2} s", t_read.elapsed().as_secs_f64());
    }
    if let Ok(dir) = std::env::var("KGSYNC_DUMP") {
        for (slice, want) in &d.nodes {
            let mut out = String::new();
            for (kv, p) in want.values() {
                let mut p = p.clone();
                p.insert(slice.key.to_string(), kv.clone());
                out.push_str(&Value::Object(p).to_string());
                out.push('\n');
            }
            let _ = std::fs::write(format!("{dir}/{}.jsonl", slice.label), out);
        }
    }
    // Slices the spec pass did not declare (none expected) are read now.
    let node_cur: Vec<Result<reconcile::NodeCurrent, String>> = d
        .nodes
        .iter()
        .map(|(sl, _)| {
            prefetch
                .nodes
                .remove(&node_sig(sl))
                .map(Ok)
                .unwrap_or_else(|| reconcile::read_nodes(g, sl))
        })
        .collect();
    let edge_cur: Vec<Result<reconcile::EdgeCurrent, String>> = d
        .edges
        .iter()
        .map(|(sl, _)| {
            prefetch
                .edges
                .remove(&edge_sig(sl))
                .map(Ok)
                .unwrap_or_else(|| reconcile::read_edges(g, sl))
        })
        .collect();
    let node_labels: HashSet<&'static str> = d.nodes.iter().map(|(sl, _)| sl.label).collect();
    let mut missing: Vec<&'static str> = d
        .edges
        .iter()
        .flat_map(|(e, _)| [e.src.0, e.dst.0])
        .filter(|l| !node_labels.contains(l))
        .collect();
    missing.sort();
    missing.dedup();
    let other_keys: Vec<(&'static str, Result<HashSet<String>, String>)> = missing
        .iter()
        .map(|l| {
            (
                *l,
                prefetch
                    .keys
                    .remove(*l)
                    .map(Ok)
                    .unwrap_or_else(|| read_keys(g, l)),
            )
        })
        .collect();

    // 2. Plan, in parallel.
    let node_cur: Vec<reconcile::NodeCurrent> = node_cur.into_iter().collect::<Result<_, _>>()?;
    let edge_cur: Vec<reconcile::EdgeCurrent> = edge_cur.into_iter().collect::<Result<_, _>>()?;
    let t_plan = Instant::now();
    let mut node_plans: Vec<reconcile::NodePlan> = d
        .nodes
        .par_iter()
        .zip(node_cur.par_iter())
        .map(|((sl, want), c)| reconcile::plan_nodes(sl, want, c, stamp))
        .collect();
    // Endpoint keys once applied, per label, as views (no copy).
    let mut before: HashMap<&'static str, &HashSet<String>> = HashMap::default();
    let mut gone: HashMap<&'static str, HashSet<String>> = HashMap::default();
    let mut desired: HashMap<&'static str, Vec<&NodeMap>> = HashMap::default();
    for (((sl, want), c), p) in d.nodes.iter().zip(&node_cur).zip(&node_plans) {
        before.entry(sl.label).or_insert(&c.all);
        gone.entry(sl.label)
            .or_default()
            .extend(p.deleted.iter().cloned());
        desired.entry(sl.label).or_default().push(want);
    }
    let mut extra: HashMap<&'static str, HashSet<String>> = HashMap::default();
    for (l, r) in other_keys {
        extra.insert(l, r?);
    }
    let view = |l: &'static str| reconcile::KeyView {
        before: before.get(l).copied(),
        gone: gone.get(l),
        desired: desired.get(l).cloned().unwrap_or_default(),
        extra: extra.get(l),
    };
    let edges = std::mem::take(&mut d.edges);
    let (slices, wants): (Vec<EdgeSlice>, Vec<Vec<Edge>>) = edges.into_iter().unzip();
    let mut edge_plans: Vec<reconcile::EdgePlan> = slices
        .par_iter()
        .zip(wants.into_par_iter())
        .zip(edge_cur.into_par_iter())
        .map(|((sl, want), c)| {
            reconcile::plan_edges(sl, want, c, &view(sl.src.0), &view(sl.dst.0), stamp)
        })
        .collect();
    if o.verbose {
        eprintln!("plan phase: {:.2} s", t_plan.elapsed().as_secs_f64());
    }

    if o.verbose {
        eprintln!("plan: {:.2} s", t.elapsed().as_secs_f64());
    }
    // 3. Apply, sequentially: nodes, then edges.
    let mut reports = Vec::new();
    for ((sl, _), p) in d.nodes.iter().zip(node_plans.iter_mut()) {
        reconcile::apply_nodes(g, sl, p)?;
        reports.push(p.report.clone());
    }
    for (sl, p) in slices.iter().zip(edge_plans.iter_mut()) {
        reconcile::apply_edges(g, sl, p, stamp)?;
        reports.push(p.report.clone());
    }
    if o.verbose {
        eprintln!("apply: {:.2} s", t.elapsed().as_secs_f64());
    }
    if !o.dry_run {
        // Record what is now verified: the commit, and each re-derived layer's inputs.
        let mut set = format!(
            "p.syncedCommit = '{}', p.syncedDate = '{}'",
            stamp.commit, stamp.date
        );
        for (l, f) in &fps {
            let k = format!("kgsync{}{}", l[..1].to_uppercase(), &l[1..]);
            set.push_str(&format!(", p.{k} = '{f}'"));
        }
        g.write(&format!("MATCH (p:Project {{name:'Rucene'}}) SET {set}"))?;
    }
    Ok(reports)
}
