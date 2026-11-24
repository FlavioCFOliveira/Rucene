//! The Rucene side, from the Rust AST (`syn`): crates and their targets, the
//! module tree (followed through `mod` declarations, so every module path is the
//! compiler's), the declared items, inherent and trait `impl`s, `use` imports and
//! the doc-comment blocks the mapping layer reads.

use rayon::prelude::*;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Item {
    pub label: &'static str,
    pub qn: String,
    pub name: String,
    pub kind: &'static str,
    pub owner: Option<String>,
    pub vis: &'static str,
    pub scope: &'static str,
    pub signature: Option<String>,
    /// Line of the item's name (1-based).
    pub line: usize,
    /// Module path of the enclosing module.
    pub module: String,
}

#[derive(Debug, Clone)]
pub struct ImplTrait {
    pub module: String,
    pub ty: String,
    pub tr: String,
}

#[derive(Debug, Default)]
pub struct RFile {
    pub path: String,
    pub crate_name: Option<String>,
    pub module_path: Option<String>,
    pub kind: &'static str,
    pub loc: usize,
    pub items: Vec<Item>,
    pub impls: Vec<ImplTrait>,
    /// Inherent impl owners: (module, type name).
    pub inherent: Vec<(String, String)>,
    /// `use` paths, each as segments, with the module that holds the `use`.
    pub uses: Vec<(String, Vec<String>)>,
    pub parse_error: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone)]
struct Target {
    crate_name: String,
    root: PathBuf,
    order: usize,
    lib: bool,
    package: String,
}

/// Minimal reader of the Cargo manifest keys that define targets.
fn manifest_targets(dir: &Path, order: &mut usize) -> Vec<Target> {
    let text = match fs::read_to_string(dir.join("Cargo.toml")) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut section = String::new();
    let mut package = String::new();
    let mut lib_path: Option<String> = None;
    let mut lib_name: Option<String> = None;
    let mut autotests = true;
    let mut blocks: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with("[[") {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            blocks.push((section.clone(), BTreeMap::new()));
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim().to_string();
        let v = v.trim().trim_matches('"').to_string();
        match section.as_str() {
            "package" if k == "name" => package = v,
            "package" if k == "autotests" => autotests = v != "false",
            "lib" if k == "path" => lib_path = Some(v),
            "lib" if k == "name" => lib_name = Some(v),
            "test" | "bin" | "bench" | "example" => {
                if let Some(b) = blocks.last_mut() {
                    b.1.insert(k, v);
                }
            }
            _ => {}
        }
    }
    if package.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let package_c = package.clone();
    let package = &package_c;
    let mut push = |name: String, root: PathBuf, lib: bool, out: &mut Vec<Target>| {
        if root.exists() {
            *order += 1;
            out.push(Target {
                crate_name: name.replace('-', "_"),
                root,
                order: *order,
                lib,
                package: package.replace('-', "_"),
            });
        }
    };
    let lib = dir.join(lib_path.unwrap_or_else(|| "src/lib.rs".into()));
    push(
        lib_name.unwrap_or_else(|| package.clone()),
        lib,
        true,
        &mut out,
    );
    let mut explicit: HashSet<PathBuf> = HashSet::default();
    let mut tests: Vec<(String, PathBuf)> = Vec::new();
    for (sec, kv) in &blocks {
        let Some(name) = kv.get("name") else { continue };
        let default = match sec.as_str() {
            "test" => format!("tests/{name}.rs"),
            "bin" => format!("src/bin/{name}.rs"),
            "bench" => format!("benches/{name}.rs"),
            _ => format!("examples/{name}.rs"),
        };
        let p = dir.join(kv.get("path").cloned().unwrap_or(default));
        explicit.insert(p.clone());
        tests.push((name.clone(), p));
    }
    if dir.join("src/main.rs").exists() {
        tests.push((package.clone(), dir.join("src/main.rs")));
    }
    for (sub, auto) in [
        ("tests", autotests),
        ("benches", true),
        ("examples", true),
        ("src/bin", true),
    ] {
        if !auto {
            continue;
        }
        let Ok(rd) = fs::read_dir(dir.join(sub)) else {
            continue;
        };
        let mut found: Vec<(String, PathBuf)> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().is_some_and(|x| x == "rs") {
                found.push((p.file_stem().unwrap().to_string_lossy().into(), p));
            } else if p.is_dir() && p.join("main.rs").exists() {
                found.push((
                    p.file_name().unwrap().to_string_lossy().into(),
                    p.join("main.rs"),
                ));
            }
        }
        found.sort();
        for (n, p) in found {
            if !explicit.contains(&p) {
                tests.push((n, p));
            }
        }
    }
    tests.sort_by(|a, b| a.0.cmp(&b.0));
    for (n, p) in tests {
        push(n, p, false, &mut out);
    }
    out
}

fn cache_file(repo: &Path) -> PathBuf {
    let base = std::env::var("KGSYNC_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            repo.parent()
                .unwrap_or(repo)
                .join(".rucene-kg-target/kgsync-cache")
        });
    base.join(format!(
        "rust-extract-{}.jsonl",
        crate::FINGERPRINT_VERSION.replace('/', "_")
    ))
}

fn file_stamp(p: &Path) -> String {
    match fs::metadata(p) {
        Ok(m) => format!(
            "{}:{}",
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ),
        Err(_) => String::new(),
    }
}

/// The `&'static str` values the extractor emits.
fn intern(s: &str) -> &'static str {
    const KNOWN: &[&str] = &[
        "RustStruct",
        "RustTrait",
        "RustEnum",
        "RustAlias",
        "RustFn",
        "struct",
        "union",
        "enum",
        "alias",
        "trait",
        "trait-method",
        "function",
        "method",
        "test",
        "pub",
        "private",
        "crate",
        "source",
    ];
    KNOWN
        .iter()
        .find(|k| **k == s)
        .copied()
        .unwrap_or_else(|| Box::leak(s.to_string().into_boxed_str()))
}

fn to_cache(f: &RFile, decls: &[ModDecl], stamp: &str) -> Value {
    let items: Vec<Value> = f
        .items
        .iter()
        .map(|i| {
            json!([
                i.label,
                i.qn,
                i.name,
                i.kind,
                i.owner,
                i.vis,
                i.scope,
                i.signature,
                i.line,
                i.module
            ])
        })
        .collect();
    json!({
        "path": f.path,
        "stamp": stamp,
        "kind": f.kind,
        "err": f.parse_error,
        "items": items,
        "impls": f.impls.iter().map(|i| json!([i.module, i.ty, i.tr])).collect::<Vec<_>>(),
        "inh": f.inherent,
        "uses": f.uses,
        "decls": decls,
    })
}

fn from_cache(v: &Value) -> Option<(RFile, Vec<ModDecl>)> {
    let st = |x: &Value| x.as_str().map(String::from);
    let mut f = RFile {
        path: st(&v["path"])?,
        kind: intern(v["kind"].as_str()?),
        parse_error: st(&v["err"]),
        ..Default::default()
    };
    for i in v["items"].as_array()? {
        let a = i.as_array()?;
        f.items.push(Item {
            label: intern(a[0].as_str()?),
            qn: st(&a[1])?,
            name: st(&a[2])?,
            kind: intern(a[3].as_str()?),
            owner: st(&a[4]),
            vis: intern(a[5].as_str()?),
            scope: intern(a[6].as_str()?),
            signature: st(&a[7]),
            line: a[8].as_u64()? as usize,
            module: st(&a[9])?,
        });
    }
    for i in v["impls"].as_array()? {
        let a = i.as_array()?;
        f.impls.push(ImplTrait {
            module: st(&a[0])?,
            ty: st(&a[1])?,
            tr: st(&a[2])?,
        });
    }
    f.inherent = serde_json::from_value(v["inh"].clone()).ok()?;
    f.uses = serde_json::from_value(v["uses"].clone()).ok()?;
    let decls: Vec<ModDecl> = serde_json::from_value(v["decls"].clone()).ok()?;
    Some((f, decls))
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn list_rs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if p.is_dir() {
                if name == "target" || name.starts_with('.') || name == "node_modules" {
                    continue;
                }
                stack.push(p);
            } else if name.ends_with(".rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let t = a.meta.to_token_stream_string();
            t.contains("test") && !t.contains("not")
        }
    })
}

trait TokStr {
    fn to_token_stream_string(&self) -> String;
}
impl TokStr for syn::Meta {
    fn to_token_stream_string(&self) -> String {
        quote::ToTokens::to_token_stream(self).to_string()
    }
}

fn is_test_fn(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test"))
}

fn path_attr(attrs: &[syn::Attribute]) -> Option<String> {
    for a in attrs {
        if a.path().is_ident("path") {
            if let syn::Meta::NameValue(nv) = &a.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                {
                    return Some(s.value());
                }
            }
        }
    }
    None
}

fn id(i: &syn::Ident) -> String {
    use syn::ext::IdentExt;
    i.unraw().to_string()
}

fn vis(v: &syn::Visibility) -> &'static str {
    match v {
        syn::Visibility::Inherited => "private",
        _ => "pub",
    }
}

fn type_name(t: &syn::Type) -> Option<String> {
    match t {
        syn::Type::Path(p) => p.path.segments.last().map(|s| id(&s.ident)),
        syn::Type::Reference(r) => type_name(&r.elem),
        syn::Type::Paren(p) => type_name(&p.elem),
        syn::Type::Group(g) => type_name(&g.elem),
        syn::Type::TraitObject(o) => o.bounds.iter().find_map(|b| match b {
            syn::TypeParamBound::Trait(t) => t.path.segments.last().map(|s| id(&s.ident)),
            _ => None,
        }),
        _ => None,
    }
}

/// The declaration, from the line of its first non-attribute token to the
/// body's opening brace (or `;`), whitespace-normalised.
fn signature(
    lines: &[&str],
    start: proc_macro2::LineColumn,
    end: proc_macro2::LineColumn,
) -> String {
    let mut s = String::new();
    for ln in start.line..=end.line {
        let Some(l) = lines.get(ln - 1) else { break };
        let from = if ln == start.line {
            byte_at(l, start.column)
        } else {
            0
        };
        let to = if ln == end.line {
            byte_at(l, end.column)
        } else {
            l.len()
        };
        if from <= to {
            s.push_str(&l[from..to]);
            s.push(' ');
        }
    }
    let joined: Vec<&str> = s.split_whitespace().collect();
    let out = joined
        .join(" ")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" ,", ",");
    out.trim_end_matches([',', ' ', '{', ';'])
        .trim_end_matches(',')
        .to_string()
}

fn sig_start(sig: &syn::Signature) -> proc_macro2::LineColumn {
    if let Some(t) = &sig.constness {
        return t.span.start();
    }
    if let Some(t) = &sig.asyncness {
        return t.span.start();
    }
    if let Some(t) = &sig.unsafety {
        return t.span.start();
    }
    if let Some(a) = &sig.abi {
        return a.extern_token.span.start();
    }
    sig.fn_token.span.start()
}

fn byte_at(l: &str, col: usize) -> usize {
    l.char_indices().nth(col).map(|(i, _)| i).unwrap_or(l.len())
}

struct Walker<'a> {
    lines: Vec<&'a str>,
    file: &'a mut RFile,
}

impl Walker<'_> {
    /// The declaration text from its first token to the body's `{` (or `;`).
    /// Token positions only: `Spanned::span()` re-prints the whole item.
    fn fn_sig(&self, v: &syn::Visibility, sig: &syn::Signature, end: proc_macro2::Span) -> String {
        let start = match v {
            syn::Visibility::Public(p) => p.span.start(),
            syn::Visibility::Restricted(r) => r.pub_token.span.start(),
            syn::Visibility::Inherited => sig_start(sig),
        };
        signature(&self.lines, start, end.start())
    }

    fn items(&mut self, items: &[syn::Item], module: &str, test: bool) {
        for it in items {
            self.item(it, module, test);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        label: &'static str,
        name: String,
        kind: &'static str,
        owner: Option<String>,
        v: &'static str,
        test: bool,
        sig: Option<String>,
        line: usize,
        module: &str,
    ) {
        let qn = match &owner {
            Some(o) if label == "RustFn" => format!("{module}::{o}::{name}"),
            _ => format!("{module}::{name}"),
        };
        self.file.items.push(Item {
            label,
            qn,
            name,
            kind,
            owner,
            vis: v,
            scope: if test { "test" } else { "crate" },
            signature: sig,
            line,
            module: module.to_string(),
        });
    }

    fn item(&mut self, it: &syn::Item, module: &str, test: bool) {
        use syn::Item::*;
        match it {
            Struct(s) => {
                let t = test || is_cfg_test(&s.attrs);
                self.push(
                    "RustStruct",
                    id(&s.ident),
                    "struct",
                    None,
                    vis(&s.vis),
                    t,
                    None,
                    s.ident.span().start().line,
                    module,
                )
            }
            Union(s) => {
                let t = test || is_cfg_test(&s.attrs);
                self.push(
                    "RustStruct",
                    id(&s.ident),
                    "union",
                    None,
                    vis(&s.vis),
                    t,
                    None,
                    s.ident.span().start().line,
                    module,
                )
            }
            Enum(s) => {
                let t = test || is_cfg_test(&s.attrs);
                self.push(
                    "RustEnum",
                    id(&s.ident),
                    "enum",
                    None,
                    vis(&s.vis),
                    t,
                    None,
                    s.ident.span().start().line,
                    module,
                )
            }
            Type(s) => {
                let t = test || is_cfg_test(&s.attrs);
                self.push(
                    "RustAlias",
                    id(&s.ident),
                    "alias",
                    None,
                    vis(&s.vis),
                    t,
                    None,
                    s.ident.span().start().line,
                    module,
                )
            }
            Trait(tr) => {
                let t = test || is_cfg_test(&tr.attrs);
                let v = vis(&tr.vis);
                let name = id(&tr.ident);
                self.push(
                    "RustTrait",
                    name.clone(),
                    "trait",
                    None,
                    v,
                    t,
                    None,
                    tr.ident.span().start().line,
                    module,
                );
                for ti in &tr.items {
                    if let syn::TraitItem::Fn(f) = ti {
                        let tt = t || is_cfg_test(&f.attrs);
                        let end = match (&f.default, &f.semi_token) {
                            (Some(b), _) => b.brace_token.span.open(),
                            (None, Some(semi)) => semi.span,
                            (None, None) => f.sig.fn_token.span,
                        };
                        let sig = signature(&self.lines, sig_start(&f.sig), end.start());
                        self.push(
                            "RustFn",
                            id(&f.sig.ident),
                            "trait-method",
                            Some(name.clone()),
                            v,
                            tt,
                            Some(sig),
                            f.sig.ident.span().start().line,
                            module,
                        );
                    }
                }
            }
            Fn(f) => {
                let t = test || is_cfg_test(&f.attrs);
                let kind = if is_test_fn(&f.attrs) {
                    "test"
                } else {
                    "function"
                };
                let sig = self.fn_sig(&f.vis, &f.sig, f.block.brace_token.span.open());
                self.push(
                    "RustFn",
                    id(&f.sig.ident),
                    kind,
                    None,
                    vis(&f.vis),
                    t,
                    Some(sig),
                    f.sig.ident.span().start().line,
                    module,
                )
            }
            Impl(im) => {
                let t = test || is_cfg_test(&im.attrs);
                let Some(ty) = type_name(&im.self_ty) else {
                    return;
                };
                match &im.trait_ {
                    None => {
                        self.file.inherent.push((module.to_string(), ty.clone()));
                        for ii in &im.items {
                            if let syn::ImplItem::Fn(f) = ii {
                                let tt = t || is_cfg_test(&f.attrs);
                                let kind = if is_test_fn(&f.attrs) {
                                    "test"
                                } else {
                                    "method"
                                };
                                let sig =
                                    self.fn_sig(&f.vis, &f.sig, f.block.brace_token.span.open());
                                self.push(
                                    "RustFn",
                                    id(&f.sig.ident),
                                    kind,
                                    Some(ty.clone()),
                                    vis(&f.vis),
                                    tt,
                                    Some(sig),
                                    f.sig.ident.span().start().line,
                                    module,
                                );
                            }
                        }
                    }
                    Some((_, path, _)) => {
                        if let Some(seg) = path.segments.last() {
                            self.file.impls.push(ImplTrait {
                                module: module.to_string(),
                                ty,
                                tr: id(&seg.ident),
                            });
                        }
                    }
                }
            }
            Mod(m) => {
                if let Some((_, content)) = &m.content {
                    let t = test || is_cfg_test(&m.attrs);
                    let sub = format!("{module}::{}", m.ident);
                    self.items(content, &sub, t);
                }
            }
            Use(u) => {
                let mut acc = Vec::new();
                flatten_use(&u.tree, &mut Vec::new(), &mut acc);
                for p in acc {
                    self.file.uses.push((module.to_string(), p));
                }
            }
            _ => {}
        }
    }
}

fn flatten_use(t: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match t {
        syn::UseTree::Path(p) => {
            prefix.push(id(&p.ident));
            flatten_use(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut v = prefix.clone();
            v.push(id(&n.ident));
            out.push(v);
        }
        syn::UseTree::Rename(r) => {
            let mut v = prefix.clone();
            v.push(id(&r.ident));
            out.push(v);
        }
        syn::UseTree::Glob(_) => out.push(prefix.clone()),
        syn::UseTree::Group(g) => {
            for i in &g.items {
                flatten_use(i, prefix, out);
            }
        }
    }
}

/// `mod x;` declarations of a parsed file: (name, #[path], directory segments
/// of the enclosing inline modules, module segments of those modules).
type ModDecl = (String, Option<String>, Vec<String>, Vec<String>);

fn mod_decls(
    items: &[syn::Item],
    dirs: &mut Vec<String>,
    mods: &mut Vec<String>,
    out: &mut Vec<ModDecl>,
) {
    for it in items {
        if let syn::Item::Mod(m) = it {
            match &m.content {
                None => out.push((
                    id(&m.ident),
                    path_attr(&m.attrs),
                    dirs.clone(),
                    mods.clone(),
                )),
                Some((_, inner)) => {
                    dirs.push(path_attr(&m.attrs).unwrap_or_else(|| id(&m.ident)));
                    mods.push(id(&m.ident));
                    mod_decls(inner, dirs, mods, out);
                    dirs.pop();
                    mods.pop();
                }
            }
        }
    }
}

#[derive(Default)]
pub struct RustSide {
    pub files: Vec<RFile>,
    /// Module path → file path (crate roots, file modules and inline modules).
    pub module_file: HashMap<String, String>,
    pub crates: Vec<(String, String)>,
}

/// Hash of every Rust input: each `.rs` and `Cargo.toml` file's path, size and
/// modification time (a stat walk, no read).
pub fn fingerprint(repo: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut files = list_rs(&repo.join("src"));
    for sub in ["tests", "benches", "examples", "crates"] {
        files.extend(list_rs(&repo.join(sub)));
    }
    files.push(repo.join("Cargo.toml"));
    if let Ok(rd) = fs::read_dir(repo.join("crates")) {
        files.extend(rd.flatten().map(|e| e.path().join("Cargo.toml")));
    }
    files.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::FINGERPRINT_VERSION.hash(&mut h);
    for f in files {
        if let Ok(m) = fs::metadata(&f) {
            f.hash(&mut h);
            m.len().hash(&mut h);
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}

pub fn extract(repo: &Path) -> RustSide {
    // 1. Targets of every manifest (root package first, then members by path).
    let mut order = 0;
    let mut targets = manifest_targets(repo, &mut order);
    let mut member_dirs: Vec<PathBuf> = fs::read_dir(repo.join("crates"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.join("Cargo.toml").exists())
                .collect()
        })
        .unwrap_or_default();
    member_dirs.sort();
    for d in &member_dirs {
        targets.extend(manifest_targets(d, &mut order));
    }
    // Library crates first, so a file shared with a test target keeps the library path.
    targets.sort_by_key(|t| (!t.lib, t.order));
    // Same-named targets of different packages (e.g. two `tests/binary.rs`)
    // would share module paths: qualify those with `@<package>`.
    let mut packages_of: HashMap<String, HashSet<String>> = HashMap::default();
    for t in &targets {
        packages_of
            .entry(t.crate_name.clone())
            .or_default()
            .insert(t.package.clone());
    }
    for t in &mut targets {
        if !t.lib && packages_of[&t.crate_name].len() > 1 {
            t.crate_name = format!("{}@{}", t.crate_name, t.package);
        }
    }

    // 2. Parse every .rs file in parallel.
    let mut paths = list_rs(&repo.join("src"));
    for sub in ["tests", "benches", "examples"] {
        paths.extend(list_rs(&repo.join(sub)));
    }
    for d in &member_dirs {
        paths.extend(list_rs(d));
    }
    paths.sort();
    paths.dedup();
    // One parallel pass: parse, `mod` declarations, items under a file-local
    // module root `@` (prefixed with the real module path after the walk).
    struct Parsed {
        path: PathBuf,
        decls: Vec<ModDecl>,
        file: RFile,
        stamp: String,
        fresh: bool,
    }
    // Per-file extraction cache: a file whose size and mtime are unchanged is
    // not parsed again (its items are cached `@`-rooted, before prefixing).
    let cache_path = cache_file(repo);
    let cache: HashMap<String, Value> = fs::read_to_string(&cache_path)
        .map(|t| {
            t.par_lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter_map(|v| Some((v.get("path")?.as_str()?.to_string(), v)))
                .collect::<Vec<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    let mut parsed: Vec<Parsed> = paths
        .par_iter()
        .map(|p| {
            let text = fs::read_to_string(p).unwrap_or_default();
            let stamp = file_stamp(p);
            let relp = rel(repo, p);
            if let Some(hit) = cache
                .get(&relp)
                .filter(|v| v.get("stamp").and_then(|x| x.as_str()) == Some(stamp.as_str()))
            {
                if let Some((mut f, decls)) = from_cache(hit) {
                    f.loc = text.lines().count();
                    f.text = text;
                    return Parsed {
                        path: p.clone(),
                        decls,
                        file: f,
                        stamp,
                        fresh: false,
                    };
                }
            }
            let path = rel(repo, p);
            let kind = if path.starts_with("tests/")
                || path.contains("/tests/")
                || path.starts_with("benches/")
                || path.contains("/benches/")
            {
                "test"
            } else {
                "source"
            };
            let mut f = RFile {
                path,
                kind,
                loc: text.lines().count(),
                ..Default::default()
            };
            let mut decls = Vec::new();
            match syn::parse_file(&text) {
                Ok(ast) => {
                    mod_decls(&ast.items, &mut Vec::new(), &mut Vec::new(), &mut decls);
                    let lines: Vec<&str> = text.lines().collect();
                    let mut w = Walker {
                        lines,
                        file: &mut f,
                    };
                    w.items(&ast.items, "@", false);
                }
                Err(e) => f.parse_error = Some(e.to_string()),
            }
            f.text = text;
            Parsed {
                path: p.clone(),
                decls,
                file: f,
                stamp,
                fresh: true,
            }
        })
        .collect();
    if parsed.iter().any(|p| p.fresh) || cache.len() != parsed.len() {
        let lines: Vec<String> = parsed
            .par_iter()
            .map(|p| to_cache(&p.file, &p.decls, &p.stamp).to_string())
            .collect();
        if let Some(dir) = cache_path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&cache_path, lines.join("\n"));
    }
    let index: HashMap<PathBuf, usize> = parsed
        .iter()
        .enumerate()
        .map(|(i, p)| (p.path.clone(), i))
        .collect();

    // Module tree walk from each target root.
    let mut assigned: HashMap<usize, (String, String)> = HashMap::default();
    let mut module_file: HashMap<String, String> = HashMap::default();
    let mut crates = Vec::new();
    for t in &targets {
        let Some(&ri) = index.get(&t.root) else {
            continue;
        };
        if assigned.contains_key(&ri) {
            continue;
        }
        crates.push((t.crate_name.clone(), rel(repo, &t.root)));
        let mut stack: Vec<(usize, String, PathBuf)> = vec![(
            ri,
            t.crate_name.clone(),
            t.root.parent().unwrap().to_path_buf(),
        )];
        while let Some((fi, mpath, dir)) = stack.pop() {
            if assigned.contains_key(&fi) {
                continue;
            }
            assigned.insert(fi, (t.crate_name.clone(), mpath.clone()));
            module_file
                .entry(mpath.clone())
                .or_insert_with(|| parsed[fi].file.path.clone());
            for (name, pattr, inline_dirs, inline) in parsed[fi].decls.clone() {
                let mut base = dir.clone();
                for seg in &inline_dirs {
                    base = base.join(seg);
                }
                let cand: Vec<PathBuf> = match &pattr {
                    Some(p) => vec![if inline.is_empty() {
                        parsed[fi].path.parent().unwrap().join(p)
                    } else {
                        base.join(p)
                    }],
                    None => vec![
                        base.join(format!("{name}.rs")),
                        base.join(&name).join("mod.rs"),
                    ],
                };
                let mut sub = mpath.clone();
                for seg in &inline {
                    sub.push_str("::");
                    sub.push_str(seg);
                }
                sub.push_str("::");
                sub.push_str(&name);
                for c in cand {
                    let c = normalize(&c);
                    if let Some(&ci) = index.get(&c) {
                        let child_dir = if c.file_name().is_some_and(|f| f == "mod.rs") {
                            c.parent().unwrap().to_path_buf()
                        } else {
                            c.with_extension("")
                        };
                        stack.push((ci, sub.clone(), child_dir));
                        break;
                    }
                }
            }
        }
    }

    // Prefix the real module paths (in parallel); unreachable files keep no items.
    parsed.par_iter_mut().enumerate().for_each(|(i, p)| {
        let f = &mut p.file;
        match assigned.get(&i) {
            Some((c, m)) => {
                f.crate_name = Some(c.clone());
                f.module_path = Some(m.clone());
                let fix = |s: &mut String| {
                    if let Some(rest) = s.strip_prefix('@') {
                        *s = format!("{m}{rest}");
                    }
                };
                for it in &mut f.items {
                    fix(&mut it.module);
                    fix(&mut it.qn);
                }
                for im in &mut f.impls {
                    fix(&mut im.module);
                }
                for (m2, _) in &mut f.inherent {
                    fix(m2);
                }
                for (m2, _) in &mut f.uses {
                    fix(m2);
                }
            }
            None => {
                f.items.clear();
                f.impls.clear();
                f.inherent.clear();
                f.uses.clear();
            }
        }
    });
    let files: Vec<RFile> = parsed.into_iter().map(|p| p.file).collect();
    for f in &files {
        // Inline modules map to their file.
        for it in &f.items {
            module_file
                .entry(it.module.clone())
                .or_insert_with(|| f.path.clone());
        }
    }
    RustSide {
        files,
        module_file,
        crates,
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_tree_items_and_signatures_follow_the_compiler() {
        let dir = std::env::temp_dir().join(format!("kgsync-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src/a")).unwrap();
        fs::create_dir_all(dir.join("src/inner_dir")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            dir.join("src/lib.rs"),
            "pub mod a;\nmod r#impl;\n#[path = \"inner_dir\"]\nmod inner { pub mod deep; }\n#[cfg(test)]\nmod tests { #[test] fn t() {} }\n",
        )
        .unwrap();
        fs::write(dir.join("src/a.rs"), "pub mod b;\npub struct S;\nimpl S {\n    pub fn new(\n        x: u32,\n    ) -> Self where Self: Sized, {\n        S\n    }\n}\npub trait T { fn m(&self); }\nimpl T for S { fn m(&self) {} }\n").unwrap();
        fs::write(dir.join("src/a/b.rs"), "pub(crate) const fn f() {}\n").unwrap();
        fs::write(dir.join("src/impl.rs"), "pub enum E { V }\n").unwrap();
        fs::write(dir.join("src/inner_dir/deep.rs"), "pub type Al = u8;\n").unwrap();
        let r = extract(&dir);
        let qns: HashSet<String> = r
            .files
            .iter()
            .flat_map(|f| f.items.iter().map(|i| i.qn.clone()))
            .collect();
        for q in [
            "my_crate::a::S",
            "my_crate::a::S::new",
            "my_crate::a::T",
            "my_crate::a::T::m",
            "my_crate::a::b::f",
            "my_crate::impl::E",
            "my_crate::inner::deep::Al",
            "my_crate::tests::t",
        ] {
            assert!(qns.contains(q), "missing {q} in {qns:?}");
        }
        let new = r
            .files
            .iter()
            .flat_map(|f| &f.items)
            .find(|i| i.qn == "my_crate::a::S::new")
            .unwrap();
        assert_eq!(
            new.signature.as_deref(),
            Some("pub fn new(x: u32,) -> Self where Self: Sized")
        );
        let t = r
            .files
            .iter()
            .flat_map(|f| &f.items)
            .find(|i| i.qn == "my_crate::tests::t")
            .unwrap();
        assert_eq!((t.kind, t.scope), ("test", "test"));
        let a = r.files.iter().find(|f| f.path == "src/a.rs").unwrap();
        assert_eq!(
            a.impls.len(),
            1,
            "trait impl recorded, its methods not modelled"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
