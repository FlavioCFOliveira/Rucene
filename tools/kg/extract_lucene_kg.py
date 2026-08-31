#!/usr/bin/env python3
"""
Extrai a estrutura do Apache Lucene Core 10.5.0 (packages, ficheiros, tipos,
imports, extends/implements) e emite comandos Cypher para popular o KG.
"""
import os
import re
import sys
from pathlib import Path
from collections import defaultdict

import argparse

parser = argparse.ArgumentParser()
parser.add_argument('--source-root',
                    default='/tmp/lucene1050/lucene/core/src/java/org/apache/lucene')
parser.add_argument('--lucene-root', default='/tmp/lucene1050',
                    help='clone root the file paths are made relative to')
parser.add_argument('--output-dir', default='/tmp/lucene_kg')
parser.add_argument('--commit', default='UNKNOWN')
parser.add_argument('--date', default='UNKNOWN')

# Defaults are the path CLAUDE.md 16.1 names; both are overridable so the
# survey can be replayed against any checkout of the reference clone.
ROOT = Path('/tmp/lucene1050/lucene/core/src/java/org/apache/lucene')
LUCENE_ROOT = Path('/tmp/lucene1050')
CORE_PREFIX = 'org.apache.lucene'

args = None

def clean_for_brace_count(text):
    """Return a copy of text with strings and comments replaced by spaces,
    so that brace counting is not confused by their contents."""
    out = list(text)
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        # line comment
        if c == '/' and i + 1 < n and text[i + 1] == '/':
            j = i
            while j < n and text[j] != '\n':
                out[j] = ' '
                j += 1
            i = j
            continue
        # block comment
        if c == '/' and i + 1 < n and text[i + 1] == '*':
            j = i
            while j + 1 < n and not (text[j] == '*' and text[j + 1] == '/'):
                out[j] = ' '
                j += 1
            if j + 1 < n:
                out[j] = ' '
                out[j + 1] = ' '
                j += 2
            i = j
            continue
        # text block (Java 15+) - """ ... """
        if c == '"' and i + 2 < n and text[i + 1] == '"' and text[i + 2] == '"':
            j = i + 3
            while j + 2 < n and not (text[j] == '"' and text[j + 1] == '"' and text[j + 2] == '"'):
                if text[j] != '\n':
                    out[j] = ' '
                j += 1
            if j + 2 < n:
                out[j] = ' '
                out[j + 1] = ' '
                out[j + 2] = ' '
                j += 3
            i = j
            continue
        # string literal
        if c == '"':
            out[i] = ' '
            j = i + 1
            while j < n:
                if text[j] == '\\':
                    out[j] = ' '
                    if j + 1 < n:
                        out[j + 1] = ' '
                        j += 2
                    continue
                if text[j] == '"':
                    out[j] = ' '
                    j += 1
                    break
                if text[j] != '\n':
                    out[j] = ' '
                j += 1
            i = j
            continue
        # char literal
        if c == "'":
            out[i] = ' '
            j = i + 1
            while j < n:
                if text[j] == '\\':
                    out[j] = ' '
                    if j + 1 < n:
                        out[j + 1] = ' '
                        j += 2
                    continue
                if text[j] == "'":
                    out[j] = ' '
                    j += 1
                    break
                if text[j] != '\n':
                    out[j] = ' '
                j += 1
            i = j
            continue
        i += 1
    return ''.join(out)


def discover():
    packages = set()
    files = []
    types = []        # dicts: package, name, kind, qualified, file, extends, implements, modifiers
    pkg_deps = defaultdict(set)  # package -> imported packages (within core)
    type_extends = []
    type_implements = []

    for dirpath, dirnames, filenames in os.walk(ROOT):
        rel_dir = Path(dirpath).relative_to(LUCENE_ROOT)
        rel_dir_str = str(rel_dir).replace('/', '.')
        # packages are directories under lucene/core/src/java/org/apache/lucene
        if rel_dir_str.startswith('lucene.core.src.java.org.apache.lucene'):
            pkg_candidate = rel_dir_str.replace('lucene.core.src.java.', '')
            packages.add(pkg_candidate)

        for fn in filenames:
            if not fn.endswith('.java'):
                continue
            file_path = str(rel_dir / fn)
            full_path = Path(dirpath) / fn
            text = full_path.read_text(encoding='utf-8', errors='ignore')

            # package declaration
            m = re.search(r'\n\s*package\s+([a-zA-Z0-9_.]+)\s*;', text)
            if not m:
                continue
            pkg = m.group(1)
            packages.add(pkg)

            files.append({
                'path': file_path,
                'name': fn,
                'package': pkg,
            })

            # imports
            for imp in re.findall(r'\n\s*import\s+(?:static\s+)?([a-zA-Z0-9_.]+(?:\.\*)?)\s*;', text):
                if imp.startswith(CORE_PREFIX):
                    # imported package is the prefix up to the last capitalised class
                    # e.g. org.apache.lucene.index.IndexWriter -> org.apache.lucene.index
                    parts = imp.split('.')
                    if parts[-1] == '*':
                        imported_pkg = '.'.join(parts[:-1])
                    else:
                        # find last part that looks like a package (lowercase first letter)
                        pkg_parts = []
                        for p in parts:
                            if p and p[0].islower():
                                pkg_parts.append(p)
                            else:
                                break
                        imported_pkg = '.'.join(pkg_parts)
                    if imported_pkg and imported_pkg != pkg and imported_pkg.startswith(CORE_PREFIX):
                        pkg_deps[pkg].add(imported_pkg)

            # type declarations (top-level only)
            # regex: optional modifiers, then (class|interface|enum|record|@interface), name,
            # optional generics, extends, implements, permits
            decl_re = re.compile(
                r'^\s*(?P<mods>(?:public\s+|protected\s+|private\s+|abstract\s+|final\s+|static\s+|strictfp\s+|sealed\s+|non-sealed\s+)*)'
                r'(?P<kind>class|interface|enum|record|@interface)\s+(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)'
                r'(?P<params>\([^)]*\))?\s*'
                r'(?P<generics>[<][^;{}]*[>])?\s*'
                r'(?P<extends>\s+extends\s+(?P<ext>[^{<]+))?'
                r'(?P<impls>\s+implements\s+(?P<impl_list>[^{<]+))?'
                r'(?P<permits>\s+permits\s+(?P<permits_list>[^{<]+))?'
                r'\s*[{<]',
                re.MULTILINE,
            )
            clean = clean_for_brace_count(text)
            for dm in decl_re.finditer(text):
                # top-level: count braces before this declaration in cleaned text; depth must be 0
                prefix = clean[:dm.start()]
                depth = prefix.count('{') - prefix.count('}')
                if depth != 0:
                    continue
                name = dm.group('name')
                kind_raw = dm.group('kind')
                if kind_raw == '@interface':
                    kind = 'annotation'
                elif kind_raw == 'enum':
                    kind = 'enum'
                elif kind_raw == 'interface':
                    kind = 'interface'
                elif kind_raw == 'record':
                    kind = 'record'
                else:
                    kind = 'class'
                # detect exception by name convention or superclass
                is_exception = name.endswith('Exception') or name.endswith('Error')
                ext_clause = dm.group('ext')
                ext = None
                if ext_clause:
                    # take first identifier, strip generics
                    ext = ext_clause.strip().split()[0].split('<')[0].strip()
                    if ext in ('Object', 'Enum', 'Throwable', 'Exception', 'RuntimeException', 'Error'):
                        ext = 'java.lang.' + ext
                    elif '.' not in ext:
                        # resolve against imports / same package
                        ext_qualified = resolve_type_name(text, pkg, ext)
                        if ext_qualified:
                            ext = ext_qualified
                    if 'Throwable' in ext or 'Exception' in ext or 'Error' in ext:
                        is_exception = True
                if is_exception and kind == 'class':
                    kind = 'exception'

                impls = []
                if dm.group('impl_list'):
                    for raw in dm.group('impl_list').split(','):
                        raw = raw.strip().split('<')[0].strip()
                        if not raw:
                            continue
                        if '.' not in raw:
                            q = resolve_type_name(text, pkg, raw)
                            if q:
                                impls.append(q)
                            else:
                                impls.append(raw)
                        else:
                            impls.append(raw)

                qualified = pkg + '.' + name
                types.append({
                    'package': pkg,
                    'name': name,
                    'kind': kind,
                    'qualified': qualified,
                    'file': file_path,
                    'extends': ext,
                    'implements': impls,
                })
                if ext and ext.startswith(CORE_PREFIX):
                    type_extends.append((qualified, ext))
                for impl in impls:
                    if impl.startswith(CORE_PREFIX):
                        type_implements.append((qualified, impl))

    return {
        'packages': sorted(packages),
        'files': files,
        'types': types,
        'pkg_deps': {k: sorted(v) for k, v in sorted(pkg_deps.items())},
        'type_extends': type_extends,
        'type_implements': type_implements,
    }


def resolve_type_name(text, pkg, simple_name):
    """Tenta resolver um tipo simples (sem qualificação) usando imports ou package."""
    # same package
    if re.search(r'\b' + re.escape(simple_name) + r'\b', text):
        # check imports
        for imp in re.findall(r'\n\s*import\s+([a-zA-Z0-9_.]+)\s*;', text):
            if imp.endswith('.' + simple_name):
                return imp
            if imp.endswith('.*'):
                base = imp[:-2]
                candidate = base + '.' + simple_name
                # cannot verify existence, but likely
                return candidate
    # fallback same package
    return pkg + '.' + simple_name


def escape(s):
    return s.replace('\\', '\\\\').replace("'", "\\'")


def emit_cypher(data, commit, gdate):
    sections = {}
    sections['nodes_packages'] = [f"MERGE (p{i}:Package {{name:'{escape(pkg)}'}})" for i, pkg in enumerate(data['packages'], 1)]
    sections['nodes_files'] = [f"MERGE (f{i}:File {{path:'{escape(f['path'])}'}})" for i, f in enumerate(data['files'], 1)]
    sections['nodes_types'] = [f"MERGE (t{i}:Class {{qualifiedName:'{escape(t['qualified'])}'}})" for i, t in enumerate(data['types'], 1)]

    edges = []
    for i, pkg in enumerate(data['packages'], 1):
        edges.append(f"MATCH (m:Module {{name:'lucene/core',kind:'maven-module'}}), (p{i}:Package {{name:'{escape(pkg)}'}}) MERGE (m)-[:CONTAINS]->(p{i})")
    sections['edges_module_contains'] = edges

    edges = []
    for i, f in enumerate(data['files'], 1):
        edges.append(f"MATCH (p:Package {{name:'{escape(f['package'])}'}}), (f{i}:File {{path:'{escape(f['path'])}'}}) MERGE (p)-[:CONTAINS]->(f{i})")
    sections['edges_package_contains_file'] = edges

    edges = []
    for i, t in enumerate(data['types'], 1):
        edges.append(f"MATCH (f:File {{path:'{escape(t['file'])}'}}), (t{i}:Class {{qualifiedName:'{escape(t['qualified'])}'}}) MERGE (f)-[:DECLARES]->(t{i})")
    sections['edges_file_declares_type'] = edges

    edges = []
    for i, (pkg, deps) in enumerate(data['pkg_deps'].items(), 1):
        for j, dep in enumerate(deps):
            if dep in data['packages']:
                edges.append(f"MATCH (a{i}_{j}:Package {{name:'{escape(pkg)}'}}), (b{i}_{j}:Package {{name:'{escape(dep)}'}}) MERGE (a{i}_{j})-[:DEPENDS_ON]->(b{i}_{j})")
    sections['edges_package_depends'] = edges

    type_set = {t['qualified'] for t in data['types']}
    edges = []
    for i, (a, b) in enumerate(data['type_extends'], 1):
        if b in type_set:
            edges.append(f"MATCH (ta{i}:Class {{qualifiedName:'{escape(a)}'}}), (tb{i}:Class {{qualifiedName:'{escape(b)}'}}) MERGE (ta{i})-[:EXTENDS]->(tb{i})")
    sections['edges_type_extends'] = edges

    edges = []
    for i, (a, b) in enumerate(data['type_implements'], 1):
        if b in type_set:
            edges.append(f"MATCH (tia{i}:Class {{qualifiedName:'{escape(a)}'}}), (tib{i}:Class {{qualifiedName:'{escape(b)}'}}) MERGE (tia{i})-[:IMPLEMENTS]->(tib{i})")
    sections['edges_type_implements'] = edges

    updates = []
    for pkg in data['packages']:
        short = pkg.split('.')[-1]
        path = 'lucene/core/src/java/' + pkg.replace('.', '/')
        updates.append(f"MATCH (p:Package {{name:'{escape(pkg)}'}}) SET p.shortName='{escape(short)}', p.path='{escape(path)}', p.gitCommit='{commit}', p.gitDate='{gdate}'")
    for f in data['files']:
        updates.append(f"MATCH (f:File {{path:'{escape(f['path'])}'}}) SET f.name='{escape(f['name'])}', f.kind='source', f.package='{escape(f['package'])}', f.gitCommit='{commit}', f.gitDate='{gdate}'")
    for t in data['types']:
        updates.append(f"MATCH (t:Class {{qualifiedName:'{escape(t['qualified'])}'}}) SET t.name='{escape(t['name'])}', t.kind='{escape(t['kind'])}', t.file='{escape(t['file'])}', t.package='{escape(t['package'])}', t.gitCommit='{commit}', t.gitDate='{gdate}'")

    return sections, updates


if __name__ == '__main__':
    args = parser.parse_args()
    ROOT = Path(args.source_root)
    data = discover()
    print(f"// packages={len(data['packages'])} files={len(data['files'])} types={len(data['types'])} pkg_deps={sum(len(v) for v in data['pkg_deps'].values())} extends={len(data['type_extends'])} implements={len(data['type_implements'])}", file=sys.stderr)
    if '--json' in sys.argv:
        import json
        json.dump(data, sys.stdout, indent=2)
    else:
        import pathlib
        commit = args.commit
        gdate = args.date
        out_dir = pathlib.Path(args.output_dir)
        out_dir.mkdir(exist_ok=True)
        sections, updates = emit_cypher(data, commit, gdate)
        for name, lines in sections.items():
            (out_dir / f'{name}.cypher').write_text('\n'.join(lines), encoding='utf-8')
            print(f"// wrote {name}.cypher: {len(lines)} lines", file=sys.stderr)
        (out_dir / 'update.cypher').write_text('\n'.join(updates), encoding='utf-8')
        print(f"// wrote update.cypher: {len(updates)} lines", file=sys.stderr)
