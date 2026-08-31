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

# The reference clone and the provenance stamp are settable from the command
# line (see `main`). The defaults are the path `CLAUDE.md` 16.1 names and the
# commit of the original survey; hard-coding `/tmp/lucene-10.5.0` left this
# script unable to run at all once the clone lived at the documented path.
BASE_ROOT = Path('/tmp/lucene1050')
CORE_ROOT_JAVA = BASE_ROOT / 'lucene/core/src/java'
CORE_ROOT_JAVA21 = BASE_ROOT / 'lucene/core/src/java21'
MODULE_INFO = CORE_ROOT_JAVA / 'module-info.java'

COMMIT = 'be7ac4c97f0481b9435bf76869b2fc117de271c5'
GDATE = '2026-07-30'


def configure(lucene_root: str, commit: str, date: str):
    """Point the module at a clone and a provenance stamp."""
    global BASE_ROOT, CORE_ROOT_JAVA, CORE_ROOT_JAVA21, MODULE_INFO, COMMIT, GDATE
    BASE_ROOT = Path(lucene_root)
    CORE_ROOT_JAVA = BASE_ROOT / 'lucene/core/src/java'
    CORE_ROOT_JAVA21 = BASE_ROOT / 'lucene/core/src/java21'
    MODULE_INFO = CORE_ROOT_JAVA / 'module-info.java'
    COMMIT = commit
    GDATE = date
    # The imported extractor makes its file paths relative to this root.
    e.LUCENE_ROOT = BASE_ROOT


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
        # A record carries a component list between its name and its body, and a
        # sealed type carries a `permits` clause: without these two, 51 of
        # Lucene 10.5.0's 54 nested records and `DocIdSetBuilder.BulkAdder`
        # were invisible to this extractor.
        r'(?:\s*\([^;{}]*\))?'
        r'(?:\s+extends\s+([^{<]+))?'
        r'(?:\s+implements\s+([^{<]+))?'
        r'(?:\s+permits\s+[^{]+)?'
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
                # strip text blocks so they are not parsed as declarations
                text = re.sub(r'"""\s*\n[\s\S]*?\n\s*"""', '""', text)
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


def extract_inner_classes():
    """
    Extract all nested type declarations from src/java and src/java21,
    including multi-level nesting.

    Returns a list of dicts with keys: qualifiedName, name, kind, file, package,
    parentQualifiedName.
    """
    decl_re = re.compile(
        r'^\s*(?:(?:public|protected|private|abstract|final|static|strictfp|sealed|non-sealed)\s+)*'
        r'(class|interface|enum|record|@interface)\s+([A-Za-z_$][A-Za-z0-9_$]*)'
        r'(?:\s*<[^;{}]*>)?'
        # A record carries a component list between its name and its body, and a
        # sealed type carries a `permits` clause: without these two, 51 of
        # Lucene 10.5.0's 54 nested records and `DocIdSetBuilder.BulkAdder`
        # were invisible to this extractor.
        r'(?:\s*\([^;{}]*\))?'
        r'(?:\s+extends\s+[^{<]+)?'
        r'(?:\s+implements\s+[^{<]+)?'
        r'(?:\s+permits\s+[^{]+)?'
        r'\s*[{<]',
        re.MULTILINE,
    )

    def clean_text(text):
        # Order matters: remove text blocks/strings first (they can contain comment-like
        # tokens), then comments, then char literals (so an apostrophe inside a removed
        # comment does not start a multi-line char match).
        t = re.sub(r'"""\s*\n[\s\S]*?\n\s*"""', '""', text)
        t = re.sub(r'"(?:\\.|[^"\\])*"', '""', t)
        t = re.sub(r'//.*?$', '', t, flags=re.M)
        t = re.sub(r'/\*.*?\*/', '', t, flags=re.S)
        t = re.sub(r"'(?:\\.|[^'\\\n])*'", "''", t)
        return t

    def parent_qualified(pkg, stack):
        return pkg + '.' + '$'.join(stack)

    inners = []
    for root in [CORE_ROOT_JAVA, CORE_ROOT_JAVA21]:
        for dp, _, fs in os.walk(root):
            for f in fs:
                if not f.endswith('.java'):
                    continue
                path = Path(dp) / f
                rel_path = str(path.relative_to(BASE_ROOT))
                text = path.read_text(encoding='utf-8', errors='ignore')
                m_pkg = re.search(r'\n?\s*package\s+([a-zA-Z0-9_.]+)\s*;', text)
                if not m_pkg:
                    continue
                pkg = m_pkg.group(1)
                clean = clean_text(text)

                stack = []  # stack of simple type names currently open
                for m in decl_re.finditer(clean):
                    prefix = clean[:m.start()]
                    depth = prefix.count('{') - prefix.count('}')
                    name = m.group(2)
                    kind_raw = m.group(1)
                    kind = {'class': 'class', 'interface': 'interface', 'enum': 'enum',
                            'record': 'record', '@interface': 'annotation'}[kind_raw]

                    # Pop stack to the current depth. Depth 0 = file scope.
                    while stack and len(stack) > max(depth, 0):
                        stack.pop()

                    if depth == 0:
                        stack = [name]
                    elif stack:
                        parent = parent_qualified(pkg, stack)
                        qualified = parent + '$' + name
                        inners.append({
                            'qualifiedName': qualified,
                            'name': name,
                            'kind': kind,
                            'file': rel_path,
                            'package': pkg,
                            'parentQualifiedName': parent,
                        })
                        stack.append(name)
                    else:
                        # Should not happen unless a declaration appears at depth >0
                        # without a top-level type (e.g. local scope); skip.
                        pass
    return inners


def generate_inner_class_cypher(inners):
    """Generate Cypher to create inner Class nodes and NESTED_IN edges."""
    create_lines = []
    for inn in inners:
        create_lines.append(
            f"MERGE (c:Class {{qualifiedName:'{escape(inn['qualifiedName'])}'}})"
        )
        create_lines.append(
            f"MATCH (c:Class {{qualifiedName:'{escape(inn['qualifiedName'])}'}}), "
            f"(p:Class {{qualifiedName:'{escape(inn['parentQualifiedName'])}'}}) "
            f"MERGE (c)-[:NESTED_IN]->(p)"
        )
    update_lines = []
    for inn in inners:
        update_lines.append(
            f"MATCH (c:Class {{qualifiedName:'{escape(inn['qualifiedName'])}'}}) "
            f"SET c.name='{escape(inn['name'])}', c.kind='{escape(inn['kind'])}', "
            f"c.file='{escape(inn['file'])}', c.package='{escape(inn['package'])}', "
            f"c.parentQualifiedName='{escape(inn['parentQualifiedName'])}', "
            f"c.gitCommit='{COMMIT}', c.gitDate='{GDATE}'"
        )
    return create_lines, update_lines


def extract_members():
    """
    Extract public/protected methods, constructors, and fields from all
    top-level and inner type declarations in src/java and src/java21.

    Returns a dict with keys 'methods', 'constructors', 'fields'. Each value is
    a list of dicts with: qualifiedName, name, signature, kind, file, package,
    parentQualifiedName, modifiers, returnType (for methods/fields).
    """
    decl_re = re.compile(
        r'^\s*(?:(?:public|protected|private|abstract|final|static|strictfp|sealed|non-sealed)\s+)*'
        r'(class|interface|enum|record|@interface)\s+([A-Za-z_$][A-Za-z0-9_$]*)'
        r'(?:\s*<[^;{}]*>)?'
        # A record carries a component list between its name and its body, and a
        # sealed type carries a `permits` clause: without these two, 51 of
        # Lucene 10.5.0's 54 nested records and `DocIdSetBuilder.BulkAdder`
        # were invisible to this extractor.
        r'(?:\s*\([^;{}]*\))?'
        r'(?:\s+extends\s+[^{<]+)?'
        r'(?:\s+implements\s+[^{<]+)?'
        r'(?:\s+permits\s+[^{]+)?'
        r'\s*[{<]',
        re.MULTILINE,
    )

    # Method/constructor/field regex
    # Group 1: modifiers, Group 2: return type (optional), Group 3: name, Group 4: params
    member_re = re.compile(
        r'^\s*(?P<mods>(?:public\s+|protected\s+|private\s+|static\s+|final\s+|abstract\s+|synchronized\s+|native\s+|default\s+|volatile\s+|transient\s+)+)'
        r'(?:(?P<annots>@[A-Za-z_$][A-Za-z0-9_$\.]*(?:\([^\)]*\))?\s+)*)'
        r'(?P<type>[<>\?\[\]A-Za-z0-9_.,\s]*?)?\s*'
        r'(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)\s*'
        r'(?P<params>\([^\)]*\))?\s*'
        r'(?P<throws>throws\s+[^{;]+)?\s*'
        r'(?P<end>[;{])',
        re.MULTILINE,
    )

    def clean_text(text):
        # Order matters: remove text blocks/strings first, then comments, then chars.
        t = re.sub(r'"""\s*\n[\s\S]*?\n\s*"""', '""', text)
        t = re.sub(r'"(?:\\.|[^"\\])*"', '""', t)
        t = re.sub(r'//.*?$', '', t, flags=re.M)
        t = re.sub(r'/\*.*?\*/', '', t, flags=re.S)
        t = re.sub(r"'(?:\\.|[^'\\\n])*'", "''", t)
        return t

    methods = []
    constructors = []
    fields = []

    def is_type_like(s):
        # Heuristic for recognising a Java type token. It covers primitives, known
        # types, generic/array types, package-qualified types, and simple class
        # names that follow the UpperCamelCase convention.
        s = s.strip()
        if not s:
            return False
        if s in ('void', 'boolean', 'byte', 'char', 'short', 'int', 'long', 'float', 'double',
                 'String', 'Object', 'Class'):
            return True
        # generic or array type
        if '<' in s or '[' in s:
            return True
        # package-qualified type (e.g. org.apache.lucene.index.IndexWriter)
        if '.' in s:
            return True
        # simple class type by convention (starts with uppercase letter)
        if s[0].isupper():
            return True
        return False

    def normalize_ws(s):
        return ' '.join(s.split()) if s else ''

    def split_top_level_commas(s):
        """Split a comma-separated declaration list, ignoring commas inside
        generic brackets, parentheses or square brackets."""
        depth = 0
        parts = []
        cur = []
        for ch in s:
            if ch in '<([':
                depth += 1
            elif ch in '>)]':
                depth -= 1
            elif ch == ',' and depth == 0:
                parts.append(''.join(cur))
                cur = []
                continue
            cur.append(ch)
        parts.append(''.join(cur))
        return parts

    def extract_from_body(clean, body_start, body_end, parent_qualified):
        # body_start/body_end are indices in the already-cleaned text, so slice
        # from clean directly; using original text indices would be misaligned.
        clean_body = clean[body_start:body_end]
        for m in member_re.finditer(clean_body):
            # Only keep members declared directly in this type body. Declarations
            # nested inside methods/blocks (local variables, anonymous classes, etc.)
            # are at a positive brace depth and must be ignored.
            prefix = clean_body[:m.start()]
            member_depth = prefix.count('{') - prefix.count('}')
            if member_depth != 0:
                continue

            name = m.group('name')
            if name in ('if', 'while', 'for', 'switch', 'catch', 'synchronized', 'try', 'finally'):
                continue
            mods = normalize_ws(m.group('mods'))
            params = m.group('params')
            end = m.group('end')
            type_str = normalize_ws(m.group('type')) if m.group('type') else ''

            # skip enum constant blocks that look like methods
            if parent_qualified.endswith(')') or not name:
                continue

            params_norm = normalize_ws(params) if params else None
            if params_norm is not None:
                # constructor: name equals last segment of parent (before $ if inner)
                parent_name = parent_qualified.split('$')[-1].split('.')[-1]
                params_body = params_norm[1:-1]
                qualified = f"{parent_qualified}#{name}({params_body})"
                sig = f"{name}({params_body})"
                if name == parent_name:
                    constructors.append({
                        'qualifiedName': qualified,
                        'name': name,
                        'signature': sig,
                        'kind': 'constructor',
                        'modifiers': mods,
                        'parentQualifiedName': parent_qualified,
                    })
                else:
                    if not is_type_like(type_str):
                        continue
                    methods.append({
                        'qualifiedName': qualified,
                        'name': name,
                        'signature': f"{type_str} {sig}" if type_str else sig,
                        'kind': 'method',
                        'modifiers': mods,
                        'returnType': type_str,
                        'parentQualifiedName': parent_qualified,
                    })
            elif end == ';':
                # The regex may have matched the LAST variable in a multi-variable
                # field declaration (e.g. "long a, b, c;"). Reconstruct the full
                # declaration and split on top-level commas to capture every field.
                decl_start = m.start()
                decl_end = clean_body.find(';', decl_start)
                if decl_end == -1:
                    decl_end = len(clean_body)
                full_decl = clean_body[decl_start:decl_end]
                # strip leading modifiers/annotations portion is already captured; keep it
                # Remove annotations so the type part is easier to parse.
                decl_no_annot = re.sub(r'@[A-Za-z_$][A-Za-z0-9_$\.]*(?:\([^)]*\))?\s*', '', full_decl)
                # Drop the modifier keywords to leave "Type var1, var2, ..."
                decl_no_mods = re.sub(
                    r'^\s*(?:public\s+|protected\s+|private\s+|static\s+|final\s+|abstract\s+|synchronized\s+|native\s+|default\s+|volatile\s+|transient\s+)+',
                    '',
                    decl_no_annot,
                )
                # If there is an initializer (= ...), drop it for naming purposes.
                decl_no_init = re.sub(r'\s*=\s*[^,;]+', '', decl_no_mods)
                parts = split_top_level_commas(decl_no_init)
                if not parts:
                    continue
                # First part contains the type plus the first variable name.
                first = parts[0].strip()
                m_first = re.match(r'^(.*?)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*$', first)
                if not m_first:
                    continue
                base_type = normalize_ws(m_first.group(1))
                if not is_type_like(base_type):
                    continue
                var_names = [normalize_ws(m_first.group(2))]
                for part in parts[1:]:
                    part = part.strip().rstrip(';').strip()
                    if not part:
                        continue
                    m_var = re.match(r'^([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:=.*)?$', part)
                    if m_var:
                        var_names.append(m_var.group(1))
                for var_name in var_names:
                    fields.append({
                        'qualifiedName': f"{parent_qualified}#{var_name}",
                        'name': var_name,
                        'signature': f"{base_type} {var_name}",
                        'kind': 'field',
                        'modifiers': mods,
                        'returnType': base_type,
                        'parentQualifiedName': parent_qualified,
                    })

    for root in [CORE_ROOT_JAVA, CORE_ROOT_JAVA21]:
        for dp, _, fs in os.walk(root):
            for f in fs:
                if not f.endswith('.java'):
                    continue
                path = Path(dp) / f
                rel_path = str(path.relative_to(BASE_ROOT))
                text = path.read_text(encoding='utf-8', errors='ignore')
                m_pkg = re.search(r'\n?\s*package\s+([a-zA-Z0-9_.]+)\s*;', text)
                if not m_pkg:
                    continue
                pkg = m_pkg.group(1)
                clean = clean_text(text)

                # Track type bodies so we can extract members within each.
                # We find each type declaration start and the matching closing brace.
                stack = []  # list of (qualifiedName, brace_start)
                for m in decl_re.finditer(clean):
                    prefix = clean[:m.start()]
                    depth = prefix.count('{') - prefix.count('}')
                    name = m.group(2)

                    # Determine the start of this declaration's body.
                    # The decl_re match ends at the character after the opening '{'
                    # (or '<' for generics), so m.end()-1 is the brace itself.
                    body_start = m.end() - 1
                    if clean[body_start] != '{':
                        body_start = clean.find('{', m.end())
                    if body_start == -1:
                        # no body (e.g. annotation method without default)
                        body_start = None
                    else:
                        body_start += 1  # after the opening brace

                    # Close types that have ended before this new declaration
                    while stack and len(stack) > max(depth, 0):
                        prev_qualified, prev_body_start = stack.pop()
                        if prev_body_start is not None:
                            # find matching close brace starting from prev_body_start
                            i = prev_body_start
                            d = 1
                            n = len(clean)
                            while i < n and d > 0:
                                if clean[i] == '{':
                                    d += 1
                                elif clean[i] == '}':
                                    d -= 1
                                i += 1
                            prev_body_end = i
                            extract_from_body(clean, prev_body_start, prev_body_end, prev_qualified)

                    if depth == 0:
                        stack = [(pkg + '.' + name, body_start)]
                    elif stack:
                        parent = stack[-1][0]
                        qualified = f"{parent}${name}"
                        stack.append((qualified, body_start))

                # Finalise remaining types at end of file
                while stack:
                    prev_qualified, prev_body_start = stack.pop()
                    if prev_body_start is not None:
                        i = prev_body_start
                        d = 1
                        n = len(clean)
                        while i < n and d > 0:
                            if clean[i] == '{':
                                d += 1
                            elif clean[i] == '}':
                                d -= 1
                            i += 1
                        prev_body_end = i
                        extract_from_body(clean, prev_body_start, prev_body_end, prev_qualified)

    def dedupe_by_qn(items):
        seen = set()
        out = []
        for item in items:
            qn = item['qualifiedName']
            if qn in seen:
                continue
            seen.add(qn)
            out.append(item)
        return out

    return {
        'methods': dedupe_by_qn(methods),
        'constructors': dedupe_by_qn(constructors),
        'fields': dedupe_by_qn(fields),
    }


def generate_member_cypher(members):
    """Generate Cypher to create Method/Constructor/Field nodes and DECLARES edges."""
    create_lines = []
    update_lines = []
    for m in members:
        qn = escape(m['qualifiedName'])
        create_lines.append(
            f"MERGE (m:Method {{qualifiedName:'{qn}'}})"
        )
        create_lines.append(
            f"MATCH (c:Class {{qualifiedName:'{escape(m['parentQualifiedName'])}'}}), "
            f"(m:Method {{qualifiedName:'{qn}'}}) "
            f"MERGE (c)-[:DECLARES]->(m)"
        )
        props = {
            'name': m['name'],
            'kind': m['kind'],
            'signature': m['signature'],
            'modifiers': m.get('modifiers', ''),
            'parentQualifiedName': m['parentQualifiedName'],
            'gitCommit': COMMIT,
            'gitDate': GDATE,
        }
        if m.get('returnType'):
            props['returnType'] = m['returnType']
        set_clause = ', '.join(f"m.{k}='{escape(str(v))}'" for k, v in props.items())
        update_lines.append(
            f"MATCH (m:Method {{qualifiedName:'{qn}'}}) SET {set_clause}"
        )
    return create_lines, update_lines


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
    parser.add_argument('--lucene-root', default=str(BASE_ROOT),
                        help='Apache Lucene 10.5.0 clone (default: %(default)s)')
    parser.add_argument('--commit', default=COMMIT,
                        help='Rucene commit to stamp the provenance with')
    parser.add_argument('--date', default=GDATE, help='ISO date of --commit')
    args = parser.parse_args()

    configure(args.lucene_root, args.commit, args.date)
    if not CORE_ROOT_JAVA.is_dir():
        raise SystemExit(f'no Lucene core sources under {CORE_ROOT_JAVA}')

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

    print('Extracting inner classes...', file=sys.stderr)
    inners = extract_inner_classes()
    inner_create, inner_update = generate_inner_class_cypher(inners)
    print(f"Found {len(inners)} inner/nested type declarations.", file=sys.stderr)

    print('Extracting members (methods, constructors, fields)...', file=sys.stderr)
    members = extract_members()
    method_create, method_update = generate_member_cypher(members['methods'])
    constructor_create, constructor_update = generate_member_cypher(members['constructors'])
    field_create, field_update = generate_member_cypher(members['fields'])
    print(
        f"Found {len(members['methods'])} methods, {len(members['constructors'])} constructors, "
        f"{len(members['fields'])} fields.",
        file=sys.stderr,
    )

    module_create_file = out_dir / 'module_info_create.cypher'
    module_update_file = out_dir / 'module_info_update.cypher'
    missing_file = out_dir / 'missing_edges.cypher'
    external_file = out_dir / 'external_types.cypher'
    inner_create_file = out_dir / 'inner_classes_create.cypher'
    inner_update_file = out_dir / 'inner_classes_update.cypher'
    method_create_file = out_dir / 'methods_create.cypher'
    method_update_file = out_dir / 'methods_update.cypher'
    constructor_create_file = out_dir / 'constructors_create.cypher'
    constructor_update_file = out_dir / 'constructors_update.cypher'
    field_create_file = out_dir / 'fields_create.cypher'
    field_update_file = out_dir / 'fields_update.cypher'

    module_create_file.write_text('\n'.join(module_create), encoding='utf-8')
    module_update_file.write_text('\n'.join(module_update), encoding='utf-8')
    missing_file.write_text('\n'.join(missing_cypher), encoding='utf-8')
    external_file.write_text('\n'.join(external_cypher), encoding='utf-8')
    inner_create_file.write_text('\n'.join(inner_create), encoding='utf-8')
    inner_update_file.write_text('\n'.join(inner_update), encoding='utf-8')
    method_create_file.write_text('\n'.join(method_create), encoding='utf-8')
    method_update_file.write_text('\n'.join(method_update), encoding='utf-8')
    constructor_create_file.write_text('\n'.join(constructor_create), encoding='utf-8')
    constructor_update_file.write_text('\n'.join(constructor_update), encoding='utf-8')
    field_create_file.write_text('\n'.join(field_create), encoding='utf-8')
    field_update_file.write_text('\n'.join(field_update), encoding='utf-8')

    print(f"Wrote {module_create_file} ({len(module_create)} statements)", file=sys.stderr)
    print(f"Wrote {module_update_file} ({len(module_update)} statements)", file=sys.stderr)
    print(f"Wrote {missing_file} ({len(missing_cypher)} statements)", file=sys.stderr)
    print(f"Wrote {external_file} ({len(external_cypher)} statements)", file=sys.stderr)
    print(f"Wrote {inner_create_file} ({len(inner_create)} statements)", file=sys.stderr)
    print(f"Wrote {inner_update_file} ({len(inner_update)} statements)", file=sys.stderr)
    print(f"Wrote {method_create_file} ({len(method_create)} statements)", file=sys.stderr)
    print(f"Wrote {method_update_file} ({len(method_update)} statements)", file=sys.stderr)
    print(f"Wrote {constructor_create_file} ({len(constructor_create)} statements)", file=sys.stderr)
    print(f"Wrote {constructor_update_file} ({len(constructor_update)} statements)", file=sys.stderr)
    print(f"Wrote {field_create_file} ({len(field_create)} statements)", file=sys.stderr)
    print(f"Wrote {field_update_file} ({len(field_update)} statements)", file=sys.stderr)

    if args.run:
        print('Running module-info CREATE Cypher...', file=sys.stderr)
        run_rmp('create', str(module_create_file))
        print('Running module-info UPDATE Cypher...', file=sys.stderr)
        run_rmp('update', str(module_update_file))
        print('Running missing edges Cypher...', file=sys.stderr)
        run_rmp('create', str(missing_file))
        print('Running external type Cypher...', file=sys.stderr)
        run_rmp('update', str(external_file))
        print('Running inner-class CREATE Cypher...', file=sys.stderr)
        run_rmp('create', str(inner_create_file))
        print('Running inner-class UPDATE Cypher...', file=sys.stderr)
        run_rmp('update', str(inner_update_file))
        print('Running method CREATE Cypher...', file=sys.stderr)
        run_rmp('create', str(method_create_file))
        print('Running method UPDATE Cypher...', file=sys.stderr)
        run_rmp('update', str(method_update_file))
        print('Running constructor CREATE Cypher...', file=sys.stderr)
        run_rmp('create', str(constructor_create_file))
        print('Running constructor UPDATE Cypher...', file=sys.stderr)
        run_rmp('update', str(constructor_update_file))
        print('Running field CREATE Cypher...', file=sys.stderr)
        run_rmp('create', str(field_create_file))
        print('Running field UPDATE Cypher...', file=sys.stderr)
        run_rmp('update', str(field_update_file))
        print('Enrichment complete.', file=sys.stderr)


if __name__ == '__main__':
    main()
