#!/usr/bin/env python3
"""
Enriches the Rucene Knowledge Graph with deeper Apache Lucene Core 10.5.0
structure that the original regex-based extractor did not capture:

1. JPMS module-info semantics (exports, opens, requires, uses, provides).
2. Missing package-level DEPENDS_ON edges from src/java21 sources.
3. External EXTENDS/IMPLEMENTS targets (java.lang.*, java.io.*, etc.) recorded
   as node properties so they are not lost.

The script emits Cypher files and can optionally feed them to `rmp graph`.
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import extract_lucene_kg as e

BASE_ROOT = Path('/tmp/lucene-10.5.0')
CORE_ROOT_JAVA = BASE_ROOT / 'lucene/core/src/java'
CORE_ROOT_JAVA21 = BASE_ROOT / 'lucene/core/src/java21'
MODULE_INFO = CORE_ROOT_JAVA / 'module-info.java'

COMMIT = 'be7ac4c97f0481b9435bf76869b2fc117de271c5'
GDATE = '2026-07-30'


def escape(s):
    return s.replace('\\', '\\\\').replace("'", "\\'")


def discover_both_roots():
    """Run the extractor over src/java and src/java21 and merge results."""
    merged = {
        'packages': set(),
        'files': [],
        'types': [],
        'pkg_deps': {},
        'type_extends': [],
        'type_implements': [],
    }
    seen_qualified = set()
    seen_files = set()
    for root in [CORE_ROOT_JAVA / 'org' / 'apache' / 'lucene',
                 CORE_ROOT_JAVA21 / 'org' / 'apache' / 'lucene']:
        e.ROOT = root
        d = e.discover()
        merged['packages'].update(d['packages'])
        for f in d['files']:
            if f['path'] not in seen_files:
                seen_files.add(f['path'])
                merged['files'].append(f)
        for t in d['types']:
            if t['qualified'] not in seen_qualified:
                seen_qualified.add(t['qualified'])
                merged['types'].append(t)
        for pkg, deps in d['pkg_deps'].items():
            merged['pkg_deps'].setdefault(pkg, set()).update(deps)
        merged['type_extends'].extend(d['type_extends'])
        merged['type_implements'].extend(d['type_implements'])
    # dedupe and sort
    merged['packages'] = sorted(merged['packages'])
    merged['type_extends'] = list(dict.fromkeys(merged['type_extends']))
    merged['type_implements'] = list(dict.fromkeys(merged['type_implements']))
    return merged


def parse_module_info():
    text = MODULE_INFO.read_text(encoding='utf-8', errors='ignore')
    # strip comments
    text = re.sub(r'/\*.*?\*/', '', text, flags=re.S)
    text = re.sub(r'//.*?$', '', text, flags=re.M)

    # Join lines that are split before a semicolon (multiline exports, opens, provides)
    raw_lines = text.splitlines()
    joined = []
    buf = ''
    for line in raw_lines:
        s = line.strip()
        if not s:
            continue
        if buf:
            buf += ' ' + s
        else:
            buf = s
        if ';' in s or '{' in s or '}' in s:
            joined.append(buf)
            buf = ''
    if buf:
        joined.append(buf)

    exports = []
    opens = []
    requires = []
    provides = []  # list of (service_interface, [impl_class, ...])
    uses = []

    for line in joined:
        line = line.rstrip(';').strip()
        if line.startswith('exports '):
            rest = line[len('exports '):].strip()
            m = re.match(r'([^\s]+)(?:\s+to\s+(.+))?', rest)
            if m:
                exports.append((m.group(1), [x.strip() for x in m.group(2).split(',') if x.strip()] if m.group(2) else []))
        elif line.startswith('opens '):
            rest = line[len('opens '):].strip()
            m = re.match(r'([^\s]+)(?:\s+to\s+(.+))?', rest)
            if m:
                opens.append((m.group(1), [x.strip() for x in m.group(2).split(',') if x.strip()] if m.group(2) else []))
        elif line.startswith('requires '):
            rest = line[len('requires '):].strip()
            m = re.match(r'(static\s+)?(transitive\s+)?([^\s]+)', rest)
            if m:
                requires.append((m.group(3), bool(m.group(1)), bool(m.group(2))))
        elif line.startswith('provides '):
            rest = line[len('provides '):].strip()
            m = re.match(r'([^\s]+)\s+with\s+(.+)', rest)
            if m:
                iface = m.group(1).strip()
                impls = [x.strip() for x in m.group(2).split(',') if x.strip()]
                provides.append((iface, impls))
        elif line.startswith('uses '):
            uses.append(line[len('uses '):].strip())

    return {
        'exports': exports,
        'opens': opens,
        'requires': requires,
        'provides': provides,
        'uses': uses,
    }


def generate_module_info_cypher(mod):
    """Return (create_statements, update_statements) for JPMS metadata."""
    rel_path = str(MODULE_INFO.relative_to(BASE_ROOT))

    create_lines = []
    update_lines = []

    # Nodes
    create_lines.append(
        f"MERGE (f:File {{path:'{escape(rel_path)}'}})"
    )
    create_lines.append(
        f"MERGE (mod:Feature {{name:'JPMS module descriptor', qualifiedName:'lucene.core.module-info'}})"
    )
    # Edges
    create_lines.append(
        f"MATCH (m:Module {{name:'lucene/core'}}), (f:File {{path:'{escape(rel_path)}'}}) "
        f"MERGE (m)-[:CONTAINS]->(f)"
    )
    create_lines.append(
        f"MATCH (f:File {{path:'{escape(rel_path)}'}}), "
        f"(mod:Feature {{name:'JPMS module descriptor'}}) "
        f"MERGE (f)-[:SPECIFIED_IN]->(mod)"
    )

    # Properties
    update_lines.append(
        f"MATCH (f:File {{path:'{escape(rel_path)}'}}) "
        f"SET f.name='module-info.java', f.kind='config', f.role='module-descriptor', "
        f"f.moduleName='org.apache.lucene.core', f.gitCommit='{COMMIT}', f.gitDate='{GDATE}'"
    )
    update_lines.append(
        f"MATCH (mod:Feature {{name:'JPMS module descriptor'}}) "
        f"SET mod.description='JPMS module descriptor for org.apache.lucene.core', "
        f"mod.moduleName='org.apache.lucene.core', mod.gitCommit='{COMMIT}', mod.gitDate='{GDATE}'"
    )

    # EXPORTS edges (module descriptor -> package)
    for pkg, targets in mod['exports']:
        create_lines.append(
            f"MATCH (mod:Feature {{name:'JPMS module descriptor'}}), "
            f"(p:Package {{name:'{escape(pkg)}'}}) "
            f"MERGE (mod)-[:EXPORTS {{to:{json.dumps(targets)}}}]->(p)"
        )

    # OPENS edges
    for pkg, targets in mod['opens']:
        create_lines.append(
            f"MATCH (mod:Feature {{name:'JPMS module descriptor'}}), "
            f"(p:Package {{name:'{escape(pkg)}'}}) "
            f"MERGE (mod)-[:OPENS {{to:{json.dumps(targets)}}}]->(p)"
        )

    # REQUIRES edges (module descriptor -> required module name as a Feature node)
    for req, is_static, is_transitive in mod['requires']:
        create_lines.append(
            f"MERGE (req:Feature {{name:'Required module: {escape(req)}', qualifiedName:'module.requires.{escape(req)}'}})"
        )
        update_lines.append(
            f"MATCH (req:Feature {{qualifiedName:'module.requires.{escape(req)}'}}) "
            f"SET req.kind='external-module', req.gitCommit='{COMMIT}', req.gitDate='{GDATE}'"
        )
        create_lines.append(
            f"MATCH (mod:Feature {{name:'JPMS module descriptor'}}), "
            f"(req:Feature {{qualifiedName:'module.requires.{escape(req)}'}}) "
            f"MERGE (mod)-[:REQUIRES {{static:{str(is_static).lower()}, transitive:{str(is_transitive).lower()}}}]->(req)"
        )

    # USES edges (module descriptor -> service interface Class)
    for iface in mod['uses']:
        create_lines.append(
            f"MATCH (mod:Feature {{name:'JPMS module descriptor'}}), "
            f"(c:Class {{qualifiedName:'{escape(iface)}'}}) "
            f"MERGE (mod)-[:USES]->(c)"
        )

    # PROVIDES edges: module -> service interface, and interface -> implementations
    for iface, impls in mod['provides']:
        create_lines.append(
            f"MATCH (mod:Feature {{name:'JPMS module descriptor'}}), "
            f"(c:Class {{qualifiedName:'{escape(iface)}'}}) "
            f"MERGE (mod)-[:PROVIDES]->(c)"
        )
        for impl in impls:
            create_lines.append(
                f"MATCH (iface:Class {{qualifiedName:'{escape(iface)}'}}), "
                f"(impl:Class {{qualifiedName:'{escape(impl)}'}}) "
                f"MERGE (iface)-[:PROVIDED_BY]->(impl)"
            )

    return create_lines, update_lines


def generate_missing_edges_cypher(data):
    """Compare discovered edges with the KG and emit only the missing ones."""
    lines = []
    type_set = {t['qualified'] for t in data['types']}
    pkg_set = set(data['packages'])

    # Current KG edges
    def kg_edges(rel):
        res = subprocess.run(
            ['rmp', 'graph', 'query', '-r', 'rucene',
             '--query', f'MATCH (a)-[r:{rel}]->(b) RETURN a.name, b.name'],
            capture_output=True, text=True, check=True,
        )
        return {(a, b) for a, b in json.loads(res.stdout)['rows']}

    kg_depends = kg_edges('DEPENDS_ON')
    for pkg, deps in data['pkg_deps'].items():
        for dep in deps:
            if dep in pkg_set and (pkg, dep) not in kg_depends:
                lines.append(
                    f"MATCH (a:Package {{name:'{escape(pkg)}'}}), "
                    f"(b:Package {{name:'{escape(dep)}'}}) "
                    f"MERGE (a)-[:DEPENDS_ON]->(b)"
                )

    kg_extends = kg_edges('EXTENDS')
    for a, b in data['type_extends']:
        if b in type_set and (a, b) not in kg_extends:
            lines.append(
                f"MATCH (ta:Class {{qualifiedName:'{escape(a)}'}}), "
                f"(tb:Class {{qualifiedName:'{escape(b)}'}}) "
                f"MERGE (ta)-[:EXTENDS]->(tb)"
            )

    kg_implements = kg_edges('IMPLEMENTS')
    for a, b in data['type_implements']:
        if b in type_set and (a, b) not in kg_implements:
            lines.append(
                f"MATCH (ta:Class {{qualifiedName:'{escape(a)}'}}), "
                f"(tb:Class {{qualifiedName:'{escape(b)}'}}) "
                f"MERGE (ta)-[:IMPLEMENTS]->(tb)"
            )

    return lines


# Common java.lang / java.io / java.util types that appear as super-types in Lucene.
JAVA_LANG_TYPES = {
    'Object', 'Class', 'Enum', 'Throwable', 'Exception', 'RuntimeException', 'Error',
    'Thread', 'Runnable', 'Iterable', 'Comparable', 'Cloneable', 'Serializable',
    'Number', 'Boolean', 'Byte', 'Character', 'Double', 'Float', 'Integer', 'Long', 'Short',
    'String', 'StringBuilder', 'StringBuffer', 'System',
    'IllegalArgumentException', 'IllegalStateException', 'NullPointerException',
    'IndexOutOfBoundsException', 'UnsupportedOperationException', 'IOException',
}
JAVA_UTIL_TYPES = {
    'Iterator', 'List', 'Map', 'Set', 'Collection', 'Collections', 'Arrays',
    'Objects', 'Comparator', 'function.Function', 'function.Supplier', 'function.Consumer',
}
JAVA_IO_TYPES = {'Closeable', 'Flushable', 'DataInput', 'DataOutput', 'Serializable'}


def resolve_external_type(simple_name, imports, pkg):
    """Try to resolve a simple type to an external java.* type or return None."""
    if not simple_name:
        return None
    # direct import match
    for imp in imports:
        if imp.endswith('.' + simple_name):
            return imp
    # java.lang implicit
    if simple_name in JAVA_LANG_TYPES:
        return 'java.lang.' + simple_name
    # common java.util / java.io implicit (only if no same-package class exists; we treat as external)
    if simple_name in JAVA_UTIL_TYPES:
        return 'java.util.' + simple_name
    if simple_name in JAVA_IO_TYPES:
        return 'java.io.' + simple_name
    return None


def scan_external_supertypes():
    """Scan all Java files and return external extends/implements targets per type."""
    decl_re = re.compile(
        r'^\s*(?:(?:public|protected|private|abstract|final|static|strictfp|sealed|non-sealed)\s+)*'
        r'(class|interface|enum|record|@interface)\s+([A-Za-z_$][A-Za-z0-9_$]*)'
        r'(?:\s*<[^;{}]*>)?'
        r'(?:\s+extends\s+([^{<]+))?'
        r'(?:\s+implements\s+([^{<]+))?'
        r'\s*[{<]',
        re.MULTILINE,
    )
    ext = {}
    impl = {}
    for root in [CORE_ROOT_JAVA, CORE_ROOT_JAVA21]:
        for dp, _, fs in os.walk(root):
            for f in fs:
                if not f.endswith('.java'):
                    continue
                path = Path(dp) / f
                text = path.read_text(encoding='utf-8', errors='ignore')
                m_pkg = re.search(r'\n?\s*package\s+([a-zA-Z0-9_.]+)\s*;', text)
                if not m_pkg:
                    continue
                pkg = m_pkg.group(1)
                imports = re.findall(r'\n\s*import\s+(?:static\s+)?([a-zA-Z0-9_.]+(?:\.\*)?)\s*;', text)
                for m in decl_re.finditer(text):
                    name = m.group(2)
                    qn = pkg + '.' + name
                    # extends
                    if m.group(3):
                        for raw in m.group(3).split(','):
                            raw = raw.strip().split('<')[0].strip()
                            resolved = resolve_external_type(raw, imports, pkg)
                            if resolved:
                                ext.setdefault(qn, set()).add(resolved)
                            elif '.' not in raw:
                                # same-package candidate: if not in our type set, mark as external unknown
                                local = pkg + '.' + raw
                                # we cannot know, skip for now
                                pass
                    # implements
                    if m.group(4):
                        for raw in m.group(4).split(','):
                            raw = raw.strip().split('<')[0].strip()
                            resolved = resolve_external_type(raw, imports, pkg)
                            if resolved:
                                impl.setdefault(qn, set()).add(resolved)
    return {k: sorted(v) for k, v in ext.items()}, {k: sorted(v) for k, v in impl.items()}


def generate_external_type_cypher(data):
    """Record external EXTENDS/IMPLEMENTS targets as node properties."""
    ext, impl = scan_external_supertypes()
    lines = []
    for qn, targets in ext.items():
        lines.append(
            f"MATCH (c:Class {{qualifiedName:'{escape(qn)}'}}) "
            f"SET c.extendsExternal={json.dumps(targets)}, c.gitCommit='{COMMIT}', c.gitDate='{GDATE}'"
        )
    for qn, targets in impl.items():
        lines.append(
            f"MATCH (c:Class {{qualifiedName:'{escape(qn)}'}}) "
            f"SET c.implementsExternal={json.dumps(targets)}, c.gitCommit='{COMMIT}', c.gitDate='{GDATE}'"
        )
    return lines


def run_rmp(mode, filepath):
    subprocess.run(
        [sys.executable, str(Path(__file__).parent / 'run_kg_batches.py'),
         mode, 'rucene', filepath],
        check=True,
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output-dir', default='/tmp/lucene_kg_enrich')
    parser.add_argument('--run', action='store_true', help='Execute Cypher against rmp')
    args = parser.parse_args()

    out_dir = Path(args.output_dir)
    out_dir.mkdir(exist_ok=True)

    print('Discovering structure from src/java and src/java21...', file=sys.stderr)
    data = discover_both_roots()
    print(
        f"Found {len(data['packages'])} packages, {len(data['files'])} files, "
        f"{len(data['types'])} types, {sum(len(v) for v in data['pkg_deps'].values())} package deps, "
        f"{len(data['type_extends'])} extends, {len(data['type_implements'])} implements.",
        file=sys.stderr,
    )

    print('Parsing module-info.java...', file=sys.stderr)
    mod = parse_module_info()
    print(
        f"exports={len(mod['exports'])} opens={len(mod['opens'])} requires={len(mod['requires'])} "
        f"provides={len(mod['provides'])} uses={len(mod['uses'])}",
        file=sys.stderr,
    )

    module_create, module_update = generate_module_info_cypher(mod)
    missing_cypher = generate_missing_edges_cypher(data)
    external_cypher = generate_external_type_cypher(data)

    module_create_file = out_dir / 'module_info_create.cypher'
    module_update_file = out_dir / 'module_info_update.cypher'
    missing_file = out_dir / 'missing_edges.cypher'
    external_file = out_dir / 'external_types.cypher'

    module_create_file.write_text('\n'.join(module_create), encoding='utf-8')
    module_update_file.write_text('\n'.join(module_update), encoding='utf-8')
    missing_file.write_text('\n'.join(missing_cypher), encoding='utf-8')
    external_file.write_text('\n'.join(external_cypher), encoding='utf-8')

    print(f"Wrote {module_create_file} ({len(module_create)} statements)", file=sys.stderr)
    print(f"Wrote {module_update_file} ({len(module_update)} statements)", file=sys.stderr)
    print(f"Wrote {missing_file} ({len(missing_cypher)} statements)", file=sys.stderr)
    print(f"Wrote {external_file} ({len(external_cypher)} statements)", file=sys.stderr)

    if args.run:
        print('Running module-info CREATE Cypher...', file=sys.stderr)
        run_rmp('create', str(module_create_file))
        print('Running module-info UPDATE Cypher...', file=sys.stderr)
        run_rmp('update', str(module_update_file))
        print('Running missing edges Cypher...', file=sys.stderr)
        run_rmp('create', str(missing_file))
        print('Running external type Cypher...', file=sys.stderr)
        run_rmp('update', str(external_file))
        print('Enrichment complete.', file=sys.stderr)


if __name__ == '__main__':
    main()
