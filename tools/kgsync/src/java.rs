//! The reference side, from the Java AST (javac, `java/JavaAst.java`): every
//! module of every reference project the graph lists (`Module`, kind
//! `maven-module`), its packages, files, types (nested ones as `Outer$Inner`),
//! members (methods, constructors, fields, enum constants) and, for the modules
//! with a test surface, the test classes and test methods.

use crate::graph::Graph;
use crate::layers::{edge, identity, props, s, Desired};
use crate::reconcile::{EdgeSlice, NodeSlice, Props};
use rayon::prelude::*;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct JType {
    pub qn: String,
    pub name: String,
    pub kind: String,
    pub parent: Option<String>,
    pub vis: String,
    pub is_abstract: bool,
    pub extends: Vec<String>,
    pub implements: Vec<String>,
}

pub struct JMember {
    pub qn: String,
    pub name: String,
    pub kind: String,
    pub mods: String,
    pub owner: String,
    pub ret: String,
    pub sig: String,
    pub annots: Vec<String>,
}

pub struct JFile {
    pub rel: String,
    pub module: String,
    pub role: String,
    pub pkg: String,
    pub loc: usize,
    pub imports: Vec<String>,
    pub idents: Vec<String>,
    pub types: Vec<JType>,
    pub members: Vec<JMember>,
}

#[derive(Default)]
pub struct JavaSide {
    pub files: Vec<JFile>,
    /// Every `TestClass` key in the graph (framework test bases included).
    pub test_class_keys: HashSet<String>,
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn st(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn java_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            java_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "java") {
            out.push(p);
        }
    }
}

/// Compiles `JavaAst.java` once into the build directory; recompiles when the
/// source is newer than the class.
fn javaast_classes(repo: &Path) -> Result<PathBuf, String> {
    let src = repo.join("tools/kgsync/java/JavaAst.java");
    let out = PathBuf::from(std::env::var("KGSYNC_JAVA_OUT").unwrap_or_else(|_| {
        repo.parent()
            .unwrap_or(repo)
            .join(".rucene-kg-target/javaast")
            .to_string_lossy()
            .into()
    }));
    let class = out.join("JavaAst.class");
    let stale = match (std::fs::metadata(&src), std::fs::metadata(&class)) {
        (Ok(a), Ok(b)) => a.modified().ok() > b.modified().ok(),
        _ => true,
    };
    if stale {
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        // The JDK may ship without a `javac` launcher; its module is enough.
        let st = Command::new("java")
            .args([
                "-m",
                "jdk.compiler/com.sun.tools.javac.Main",
                "-proc:none",
                "-nowarn",
                "-d",
            ])
            .arg(&out)
            .arg(&src)
            .status()
            .map_err(|e| format!("javac: {e}"))?;
        if !st.success() {
            return Err("javac failed on JavaAst.java".into());
        }
    }
    Ok(out)
}

pub struct JavaPlan {
    lines: String,
    /// Hash of every input: the configuration, the extractor, the module rows
    /// and each source file's path, size and modification time.
    pub fingerprint: String,
}

pub fn plan(g: &Graph, repo: &Path) -> Result<JavaPlan, String> {
    let cfg: Value = serde_json::from_str(
        &std::fs::read_to_string(repo.join("tools/kgsync/references.json"))
            .map_err(|e| format!("references.json: {e}"))?,
    )
    .map_err(|e| format!("references.json: {e}"))?;
    let mut roots: HashMap<String, PathBuf> = HashMap::default();
    for p in cfg["projects"].as_array().cloned().unwrap_or_default() {
        let root = PathBuf::from(st(&p, "root"));
        if !root.exists() {
            if let Some(cmd) = p["clone"].as_array() {
                let argv: Vec<String> = cmd
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                eprintln!("kgsync: cloning {}", st(&p, "project"));
                let _ = Command::new(&argv[0]).args(&argv[1..]).status();
            }
        }
        if !root.exists() {
            return Err(format!(
                "reference sources of '{}' are missing at {}: the java layer needs every reference project (it never deletes on a partial read)",
                st(&p, "project"),
                root.display()
            ));
        }
        roots.insert(st(&p, "project"), root);
    }
    let rows = g.rows(
        "MATCH (m:Module) WHERE m.kind = 'maven-module' RETURN m.name, m.path, m.project, m.inScope, m.scopeFiles",
    )?;
    let test_modules: HashSet<String> = g
        .rows("MATCH (t:TestClass) RETURN DISTINCT t.module")?
        .into_iter()
        .filter_map(|r| r.first().and_then(|v| v.as_str()).map(String::from))
        .collect();
    let mut lines = String::new();
    let mut seen = HashSet::default();
    for r in rows {
        let name = r[0].as_str().unwrap_or("").to_string();
        let path = r[1].as_str().unwrap_or(".").to_string();
        let project = r[2].as_str().unwrap_or("").to_string();
        let Some(root) = roots.get(&project) else {
            return Err(format!(
                "module {name}: project '{project}' has no root in references.json"
            ));
        };
        let base = if path == "." {
            root.clone()
        } else {
            root.join(&path)
        };
        let mut files: Vec<(PathBuf, &str)> = Vec::new();
        match r.get(4).and_then(|v| v.as_str()) {
            Some(list) if !list.trim().is_empty() => {
                for f in list.split(',').map(str::trim).filter(|f| !f.is_empty()) {
                    files.push((root.join(f), "src"));
                }
            }
            _ => {
                // Every source root under src/ (java, java21, tools/java, main/java, …)
                // except the test trees.
                let mut v = Vec::new();
                if let Ok(rd) = std::fs::read_dir(base.join("src")) {
                    let mut dirs: Vec<PathBuf> = rd
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .collect();
                    dirs.sort();
                    for d in dirs {
                        let n = d.file_name().unwrap().to_string_lossy().to_string();
                        if n.starts_with("test") {
                            continue;
                        }
                        java_files(&d, &mut v);
                    }
                }
                files.extend(v.into_iter().map(|p| (p, "src")));
                if test_modules.contains(&name) {
                    let mut t = Vec::new();
                    for d in ["src/test/java", "src/test"] {
                        if base.join(d).exists() {
                            java_files(&base.join(d), &mut t);
                            break;
                        }
                    }
                    files.extend(t.into_iter().map(|p| (p, "test")));
                }
            }
        }
        for (abs, role) in files {
            if !abs.exists() {
                return Err(format!(
                    "module {name}: listed file missing: {}",
                    abs.display()
                ));
            }
            let rel = abs
                .strip_prefix(root)
                .unwrap_or(&abs)
                .to_string_lossy()
                .to_string();
            if seen.insert(rel.clone()) {
                lines.push_str(&format!("{name}\t{}\t{rel}\t{role}\n", abs.display()));
            }
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    crate::FINGERPRINT_VERSION.hash(&mut h);
    std::fs::read(repo.join("tools/kgsync/references.json"))
        .unwrap_or_default()
        .hash(&mut h);
    std::fs::read(repo.join("tools/kgsync/java/JavaAst.java"))
        .unwrap_or_default()
        .hash(&mut h);
    let mut test_sorted: Vec<&String> = test_modules.iter().collect();
    test_sorted.sort();
    test_sorted.hash(&mut h);
    let mut sorted_lines: Vec<&str> = lines.lines().collect();
    sorted_lines.sort_unstable();
    for l in sorted_lines {
        l.hash(&mut h);
        if let Some(abs) = l.split('\t').nth(1) {
            if let Ok(m) = std::fs::metadata(abs) {
                m.len().hash(&mut h);
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .hash(&mut h);
            }
        }
    }
    Ok(JavaPlan {
        lines,
        fingerprint: format!("{:016x}", h.finish()),
    })
}

pub fn extract(g: &Graph, repo: &Path, plan: JavaPlan) -> Result<JavaSide, String> {
    let JavaPlan { lines, .. } = plan;
    let classes = javaast_classes(repo)?;
    let mut child = Command::new("java")
        .arg("-XX:+UseParallelGC")
        .arg("-XX:TieredStopAtLevel=1")
        .arg("-cp")
        .arg(&classes)
        .arg("JavaAst")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("java: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let feed = std::thread::spawn(move || stdin.write_all(lines.as_bytes()));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = feed.join();
    if !out.status.success() {
        return Err("JavaAst failed".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let files: Vec<JFile> = text
        .par_lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| JFile {
            rel: st(&v, "rel"),
            module: st(&v, "module"),
            role: st(&v, "role"),
            pkg: st(&v, "pkg"),
            loc: v["loc"].as_u64().unwrap_or(0) as usize,
            imports: strs(&v["imports"]),
            idents: strs(&v["idents"]),
            types: v["types"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|t| JType {
                            qn: st(t, "qn"),
                            name: st(t, "name"),
                            kind: st(t, "kind"),
                            parent: t["parent"].as_str().map(String::from),
                            vis: st(t, "vis"),
                            is_abstract: t["abstract"].as_bool().unwrap_or(false),
                            extends: strs(&t["extends"]),
                            implements: strs(&t["implements"]),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            members: v["members"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|m| JMember {
                            qn: st(m, "qn"),
                            name: st(m, "name"),
                            kind: st(m, "kind"),
                            mods: st(m, "mods"),
                            owner: st(m, "owner"),
                            ret: st(m, "ret"),
                            sig: st(m, "sig"),
                            annots: strs(&m["annots"]),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    let test_class_keys = g
        .rows("MATCH (t:TestClass) RETURN t.qualifiedName")?
        .into_iter()
        .filter_map(|r| r.first().and_then(|v| v.as_str()).map(String::from))
        .collect();
    Ok(JavaSide {
        files,
        test_class_keys,
    })
}

/// Resolves a type reference as written in `file` (inside type `ctx`) to a
/// known type's qualified name.
pub struct Resolver<'a> {
    pub known: &'a HashSet<String>,
}

impl Resolver<'_> {
    pub fn resolve(&self, f: &JFile, ctx: Option<&str>, written: &str) -> Option<String> {
        let base: String = written
            .split('<')
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches("[]")
            .trim()
            .to_string();
        if base.is_empty() {
            return None;
        }
        let segs: Vec<&str> = base.split('.').collect();
        let head = segs[0];
        let tail: String = segs[1..].iter().map(|x| format!("${x}")).collect();
        // Nested in the current type or one of its enclosing types.
        let mut c = ctx.map(String::from);
        while let Some(q) = c {
            let cand = format!("{q}${head}{tail}");
            if self.known.contains(&cand) {
                return Some(cand);
            }
            c = q.rfind('$').map(|i| q[..i].to_string());
        }
        for imp in &f.imports {
            if imp.starts_with("static ") {
                continue;
            }
            if let Some(last) = imp.rsplit('.').next() {
                if last == head {
                    if let Some(q) = self.fqn(&format!(
                        "{imp}{}",
                        segs[1..]
                            .iter()
                            .map(|x| format!(".{x}"))
                            .collect::<String>()
                    )) {
                        return Some(q);
                    }
                }
            }
        }
        if !f.pkg.is_empty() {
            let cand = format!("{}.{head}{tail}", f.pkg);
            if self.known.contains(&cand) {
                return Some(cand);
            }
        }
        for imp in &f.imports {
            if let Some(p) = imp.strip_suffix(".*") {
                if imp.starts_with("static ") {
                    continue;
                }
                if let Some(q) = self.fqn(&format!("{p}.{base}")) {
                    return Some(q);
                }
            }
        }
        if segs.len() > 1 {
            return self.fqn(&base);
        }
        None
    }

    /// `a.b.C.D` → the known `a.b.C$D` (or `a.b.C.D` itself).
    pub fn fqn(&self, dotted: &str) -> Option<String> {
        if self.known.contains(dotted) {
            return Some(dotted.to_string());
        }
        let mut s = dotted.to_string();
        while let Some(i) = s.rfind('.') {
            s.replace_range(i..i + 1, "$");
            if self.known.contains(&s) {
                return Some(s);
            }
        }
        None
    }
}

pub fn layer(d: &mut Desired, j: &JavaSide) {
    let pkg_i = d.node_slice(NodeSlice {
        label: "Package",
        key: "name",
        owned: vec!["path", "shortName"],
        filter: "true".into(),
    });
    let file_i = d.node_slice(NodeSlice {
        label: "File",
        key: "path",
        owned: vec!["name", "kind", "module", "package", "loc"],
        filter: "n.path ENDS WITH '.java' AND NOT n.path STARTS WITH 'tests/'".into(),
    });
    let class_i = d.node_slice(NodeSlice {
        label: "Class",
        key: "qualifiedName",
        owned: vec![
            "name",
            "kind",
            "file",
            "module",
            "package",
            "parentQualifiedName",
            "javaVisibility",
        ],
        filter: "n.module IS NOT NULL".into(),
    });
    let member_i = d.node_slice(NodeSlice {
        label: "Method",
        key: "qualifiedName",
        owned: vec![
            "name",
            "kind",
            "modifiers",
            "parentQualifiedName",
            "returnType",
            "signature",
        ],
        filter: "true".into(),
    });
    let tclass_i = d.node_slice(NodeSlice {
        label: "TestClass",
        key: "qualifiedName",
        owned: vec![
            "name",
            "kind",
            "file",
            "module",
            "package",
            "isAbstract",
            "testMethodCount",
        ],
        filter: "n.file CONTAINS '/src/test/'".into(),
    });
    let tmethod_i = d.node_slice(NodeSlice {
        label: "TestMethod",
        key: "qualifiedName",
        owned: vec!["name", "file", "module", "parentQualifiedName"],
        filter: "n.file CONTAINS '/src/test/'".into(),
    });

    let known: HashSet<String> = j
        .files
        .iter()
        .filter(|f| f.role == "src")
        .flat_map(|f| f.types.iter().map(|t| t.qn.clone()))
        .collect();
    let mut known_tests: HashSet<String> = j
        .files
        .iter()
        .filter(|f| f.role == "test")
        .flat_map(|f| f.types.iter().map(|t| t.qn.clone()))
        .collect();
    known_tests.extend(j.test_class_keys.iter().cloned());
    let rs = Resolver { known: &known };
    let rt = Resolver {
        known: &known_tests,
    };

    let mut e: BTreeMap<(&str, &str, &str), Vec<crate::reconcile::Edge>> = BTreeMap::new();
    let mut pkg_modules: HashSet<(String, String)> = HashSet::default();
    let mut pkg_deps: HashSet<(String, String)> = HashSet::default();
    // A class is an exception when its superclass chain reaches Throwable: an
    // unresolved (JDK or external) superclass named *Exception/*Error/Throwable,
    // or a resolved superclass that is itself an exception.
    let mut sup: HashMap<String, Result<String, String>> = HashMap::default();
    for f in j.files.iter().filter(|f| f.role == "src") {
        for t in &f.types {
            if let Some(x) = t.extends.first() {
                sup.insert(
                    t.qn.clone(),
                    rs.resolve(f, Some(&t.qn), x)
                        .ok_or_else(|| x.split('<').next().unwrap_or("").to_string()),
                );
            }
        }
    }
    let mut exceptions: HashSet<String> = HashSet::default();
    for q in sup.keys() {
        let mut cur = q.clone();
        for _ in 0..64 {
            match sup.get(&cur) {
                Some(Ok(next)) => cur = next.clone(),
                Some(Err(ext)) => {
                    let last = ext.rsplit('.').next().unwrap_or(ext);
                    if last.ends_with("Exception") || last.ends_with("Error") || last == "Throwable"
                    {
                        exceptions.insert(q.clone());
                    }
                    break;
                }
                None => break,
            }
        }
    }
    let class_pkg: HashMap<&str, &str> = j
        .files
        .iter()
        .filter(|f| f.role == "src")
        .flat_map(|f| f.types.iter().map(move |t| (t.qn.as_str(), f.pkg.as_str())))
        .collect();

    for f in &j.files {
        let fname = f.rel.rsplit('/').next().unwrap_or(&f.rel);
        let dir = f.rel.rsplit_once('/').map(|x| x.0).unwrap_or("");
        let mut fp = props(&[
            ("name", s(fname)),
            ("kind", s(if f.role == "test" { "test" } else { "source" })),
            ("module", s(&f.module)),
            (
                "package",
                if f.pkg.is_empty() {
                    Value::Null
                } else {
                    s(&f.pkg)
                },
            ),
            ("loc", json!(f.loc)),
        ]);
        if fname == "module-info.java" {
            fp.insert("kind".into(), s("module-descriptor"));
        }
        d.nodes[file_i].1.insert(f.rel.clone(), (s(&f.rel), fp));
        if fname == "module-info.java" {
            e.entry(("Module", "CONTAINS", "File"))
                .or_default()
                .push(edge(s(&f.module), s(&f.rel), Props::new()));
            continue;
        }
        if !f.pkg.is_empty() {
            let short = f.pkg.rsplit('.').next().unwrap_or(&f.pkg);
            d.nodes[pkg_i].1.entry(f.pkg.clone()).or_insert((
                s(&f.pkg),
                props(&[("path", s(dir)), ("shortName", s(short))]),
            ));
            if pkg_modules.insert((f.module.clone(), f.pkg.clone())) {
                e.entry(("Module", "CONTAINS", "Package"))
                    .or_default()
                    .push(edge(s(&f.module), s(&f.pkg), Props::new()));
            }
            e.entry(("Package", "CONTAINS", "File"))
                .or_default()
                .push(edge(s(&f.pkg), s(&f.rel), Props::new()));
        }
        if f.role == "test" {
            for t in &f.types {
                let tests: Vec<&JMember> = f
                    .members
                    .iter()
                    .filter(|m| {
                        m.owner == t.qn
                            && m.kind == "method"
                            && (m.name.starts_with("test") || m.annots.iter().any(|a| a == "Test"))
                    })
                    .collect();
                d.nodes[tclass_i].1.insert(
                    t.qn.clone(),
                    (
                        s(&t.qn),
                        props(&[
                            ("name", s(&t.name)),
                            ("kind", s(&t.kind)),
                            ("file", s(&f.rel)),
                            ("module", s(&f.module)),
                            ("package", s(&f.pkg)),
                            ("isAbstract", json!(t.is_abstract)),
                            ("testMethodCount", json!(tests.len())),
                        ]),
                    ),
                );
                for m in tests {
                    let q = format!("{}#{}", t.qn, m.name);
                    d.nodes[tmethod_i].1.entry(q.clone()).or_insert((
                        s(&q),
                        props(&[
                            ("name", s(&m.name)),
                            ("file", s(&f.rel)),
                            ("module", s(&f.module)),
                            ("parentQualifiedName", s(&t.qn)),
                        ]),
                    ));
                    e.entry(("TestClass", "DECLARES", "TestMethod"))
                        .or_default()
                        .push(edge(s(&t.qn), s(&q), Props::new()));
                }
                for x in &t.extends {
                    if let Some(q) = rt.resolve(f, Some(&t.qn), x) {
                        e.entry(("TestClass", "EXTENDS", "TestClass"))
                            .or_default()
                            .push(edge(s(&t.qn), s(&q), Props::new()));
                    }
                }
            }
            continue;
        }
        let mut file_deps: HashSet<String> = HashSet::default();
        for imp in &f.imports {
            if imp.starts_with("static ") {
                if let Some(q) = rs.fqn(
                    imp.trim_start_matches("static ")
                        .rsplit_once('.')
                        .map(|x| x.0)
                        .unwrap_or(""),
                ) {
                    file_deps.insert(q);
                }
            } else if !imp.ends_with(".*") {
                if let Some(q) = rs.fqn(imp) {
                    file_deps.insert(q);
                }
            }
        }
        for id in &f.idents {
            if let Some(q) = rs.resolve(f, None, id) {
                file_deps.insert(q);
            }
        }
        for t in &f.types {
            let kind = if t.kind == "class" && exceptions.contains(&t.qn) {
                "exception"
            } else {
                t.kind.as_str()
            };
            d.nodes[class_i].1.insert(
                t.qn.clone(),
                (
                    s(&t.qn),
                    props(&[
                        ("name", s(&t.name)),
                        ("kind", s(kind)),
                        ("file", s(&f.rel)),
                        ("module", s(&f.module)),
                        ("package", s(&f.pkg)),
                        (
                            "parentQualifiedName",
                            t.parent.clone().map(Value::String).unwrap_or(Value::Null),
                        ),
                        ("javaVisibility", s(&t.vis)),
                    ]),
                ),
            );
            match &t.parent {
                None => {
                    e.entry(("File", "DECLARES", "Class"))
                        .or_default()
                        .push(edge(s(&f.rel), s(&t.qn), Props::new()));
                    for q in &file_deps {
                        if q != &t.qn && !q.starts_with(&format!("{}$", t.qn)) {
                            e.entry(("Class", "DEPENDS_ON", "Class"))
                                .or_default()
                                .push(edge(s(&t.qn), s(q), Props::new()));
                        }
                    }
                }
                Some(p) => e
                    .entry(("Class", "NESTED_IN", "Class"))
                    .or_default()
                    .push(edge(s(&t.qn), s(p), Props::new())),
            }
            for x in &t.extends {
                if let Some(q) = rs.resolve(f, Some(&t.qn), x) {
                    e.entry(("Class", "EXTENDS", "Class"))
                        .or_default()
                        .push(edge(s(&t.qn), s(&q), Props::new()));
                }
            }
            for x in &t.implements {
                if let Some(q) = rs.resolve(f, Some(&t.qn), x) {
                    e.entry(("Class", "IMPLEMENTS", "Class"))
                        .or_default()
                        .push(edge(s(&t.qn), s(&q), Props::new()));
                }
            }
        }
        for q in &file_deps {
            if let Some(p) = class_pkg.get(q.as_str()) {
                if *p != f.pkg && !f.pkg.is_empty() {
                    pkg_deps.insert((f.pkg.clone(), p.to_string()));
                }
            }
        }
        for m in &f.members {
            d.nodes[member_i].1.entry(m.qn.clone()).or_insert((
                s(&m.qn),
                props(&[
                    ("name", s(&m.name)),
                    ("kind", s(&m.kind)),
                    ("modifiers", s(&m.mods)),
                    ("parentQualifiedName", s(&m.owner)),
                    ("returnType", s(&m.ret)),
                    ("signature", s(&m.sig)),
                ]),
            ));
            e.entry(("Class", "DECLARES", "Method"))
                .or_default()
                .push(edge(s(&m.owner), s(&m.qn), Props::new()));
        }
    }
    for (a, b) in pkg_deps {
        e.entry(("Package", "DEPENDS_ON", "Package"))
            .or_default()
            .push(edge(s(&a), s(&b), Props::new()));
    }
    let slices: [(&str, &str, &str, &str); 12] = [
        ("Module", "CONTAINS", "Package", "a.kind = 'maven-module'"),
        ("Module", "CONTAINS", "File", "a.kind = 'maven-module'"),
        ("Package", "CONTAINS", "File", "true"),
        ("File", "DECLARES", "Class", "true"),
        ("Class", "DECLARES", "Method", "true"),
        ("Class", "NESTED_IN", "Class", "true"),
        ("Class", "EXTENDS", "Class", "true"),
        ("Class", "IMPLEMENTS", "Class", "true"),
        ("Class", "DEPENDS_ON", "Class", "true"),
        ("Package", "DEPENDS_ON", "Package", "true"),
        (
            "TestClass",
            "DECLARES",
            "TestMethod",
            "b.file CONTAINS '/src/test/'",
        ),
        (
            "TestClass",
            "EXTENDS",
            "TestClass",
            "a.file CONTAINS '/src/test/'",
        ),
    ];
    for (sl, rel, dl, filter) in slices {
        let rel: &'static str = match rel {
            "CONTAINS" => "CONTAINS",
            "DECLARES" => "DECLARES",
            "NESTED_IN" => "NESTED_IN",
            "EXTENDS" => "EXTENDS",
            "IMPLEMENTS" => "IMPLEMENTS",
            _ => "DEPENDS_ON",
        };
        let sl: &'static str = leak(sl);
        let dl: &'static str = leak(dl);
        let i = d.edge_slice(EdgeSlice {
            rel,
            src: (sl, identity(sl)),
            dst: (dl, identity(dl)),
            owned: vec![],
            filter: filter.into(),
        });
        d.edges[i].1 = e.remove(&(sl, rel, dl)).unwrap_or_default();
    }
}

fn leak(x: &str) -> &'static str {
    match x {
        "Module" => "Module",
        "Package" => "Package",
        "File" => "File",
        "Class" => "Class",
        "Method" => "Method",
        "TestClass" => "TestClass",
        _ => "TestMethod",
    }
}
