// JavaAst — the Java side of the Rucene knowledge graph, from the javac AST.
//
// Reads, on standard input, one file per line: "<module>\t<absolute path>\t<path
// relative to its project root>\t<src|test>". Parses every file with the JDK's
// own parser (com.sun.source; parse only, no attribution), in parallel on all
// cores, and writes one JSON object per file on standard output.
//
// Declaration texts (parameter lists, return types, modifiers) are taken from
// the source as written, every whitespace run collapsed to one space and
// annotations removed, so that member keys are stable across syncs.
//
// Run: java tools/kgsync/java/JavaAst.java < files.tsv

import com.sun.source.tree.*;
import com.sun.source.util.*;
import javax.lang.model.element.Modifier;
import javax.tools.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;

public class JavaAst {
    record In(String module, String abs, String rel, String role) {}

    static final Set<String> MODS = Set.of("public", "protected", "private", "abstract", "static", "final",
            "transient", "volatile", "synchronized", "native", "strictfp", "default", "sealed", "non-sealed");

    public static void main(String[] args) throws Exception {
        List<In> ins = new ArrayList<>();
        try (BufferedReader r = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
            String l;
            while ((l = r.readLine()) != null) {
                String[] p = l.split("\t");
                if (p.length == 4) ins.add(new In(p[0], p[1], p[2], p[3]));
            }
        }
        int n = Runtime.getRuntime().availableProcessors();
        int chunk = Math.max(1, (ins.size() + n * 4 - 1) / (n * 4));
        ExecutorService ex = Executors.newFixedThreadPool(n);
        List<Future<List<String>>> fs = new ArrayList<>();
        for (int i = 0; i < ins.size(); i += chunk) {
            List<In> part = ins.subList(i, Math.min(ins.size(), i + chunk));
            fs.add(ex.submit(() -> parse(part)));
        }
        PrintStream out = new PrintStream(new BufferedOutputStream(System.out, 1 << 20), false, StandardCharsets.UTF_8);
        try {
            for (Future<List<String>> f : fs) for (String s : f.get()) out.println(s);
        } catch (Exception e) {
            e.printStackTrace();
            System.exit(1);
        }
        out.flush();
        ex.shutdownNow();
        System.exit(0);
    }

    static List<String> parse(List<In> part) throws IOException {
        JavaCompiler jc = ToolProvider.getSystemJavaCompiler();
        StandardJavaFileManager fm = jc.getStandardFileManager(null, Locale.ROOT, StandardCharsets.UTF_8);
        Map<String, In> byPath = new HashMap<>();
        List<Path> paths = new ArrayList<>();
        for (In in : part) {
            Path p = Path.of(in.abs).toAbsolutePath().normalize();
            byPath.put(p.toString(), in);
            paths.add(p);
        }
        // One task per chunk: parse only, every file of the chunk at once.
        JavacTask task = (JavacTask) jc.getTask(new PrintWriter(Writer.nullWriter()), fm, d -> {}, List.of("-proc:none"), null, fm.getJavaFileObjectsFromPaths(paths));
        Iterable<? extends CompilationUnitTree> units = task.parse();
        SourcePositions sp = Trees.instance(task).getSourcePositions();
        List<String> res = new ArrayList<>();
        for (CompilationUnitTree cu : units) {
            String key = Path.of(cu.getSourceFile().toUri()).toAbsolutePath().normalize().toString();
            In in = byPath.get(key);
            if (in == null) continue;
            String src = cu.getSourceFile().getCharContent(true).toString();
            res.add(new FileWalker(in, src, cu, sp).json());
        }
        return res;
    }

    static final class FileWalker {
        final In in;
        final String src;
        final CompilationUnitTree cu;
        final SourcePositions sp;
        final StringBuilder types = new StringBuilder();
        final StringBuilder members = new StringBuilder();
        final Set<String> idents = new TreeSet<>();
        String pkg = "";

        FileWalker(In in, String src, CompilationUnitTree cu, SourcePositions sp) {
            this.in = in; this.src = src; this.cu = cu; this.sp = sp;
        }

        long s(Tree t) { return sp.getStartPosition(cu, t); }
        long e(Tree t) { return sp.getEndPosition(cu, t); }

        String text(long a, long b) {
            if (a < 0 || b < 0 || b < a || b > src.length()) return "";
            return src.substring((int) a, (int) b);
        }

        static String ws(String x) { return x.replaceAll("\\s+", " "); }

        static String stripComments(String x) {
            return x.replaceAll("/\\*(?s:.*?)\\*/", " ").replaceAll("//[^\\n]*", " ");
        }

        /** Source text of [a, b) with the given annotation spans removed. */
        String textWithout(long a, long b, List<? extends Tree> annots) {
            StringBuilder sb = new StringBuilder();
            long cur = a;
            List<long[]> cuts = new ArrayList<>();
            for (Tree t : annots) cuts.add(new long[] {s(t), e(t)});
            cuts.sort(Comparator.comparingLong(x -> x[0]));
            for (long[] c : cuts) {
                if (c[0] >= cur && c[1] <= b) { sb.append(text(cur, c[0])); cur = c[1]; }
            }
            sb.append(text(cur, b));
            return sb.toString();
        }

        String modifiers(ModifiersTree m) {
            String t = textWithout(s(m), e(m), m.getAnnotations());
            StringBuilder sb = new StringBuilder();
            for (String w : t.trim().split("\\s+")) {
                if (MODS.contains(w)) { if (sb.length() > 0) sb.append(' '); sb.append(w); }
            }
            return sb.toString();
        }

        String visibility(ModifiersTree m, boolean inInterface) {
            Set<Modifier> f = m.getFlags();
            if (f.contains(Modifier.PUBLIC) || inInterface) return "public";
            if (f.contains(Modifier.PROTECTED)) return "protected";
            if (f.contains(Modifier.PRIVATE)) return "private";
            return "package";
        }

        String json() {
            if (cu.getPackageName() != null) pkg = cu.getPackageName().toString();
            StringBuilder imports = new StringBuilder();
            for (ImportTree it : cu.getImports()) {
                if (imports.length() > 0) imports.append(',');
                str(imports, (it.isStatic() ? "static " : "") + it.getQualifiedIdentifier().toString());
            }
            for (Tree t : cu.getTypeDecls()) {
                if (t instanceof ClassTree ct) type(ct, null, pkg.isEmpty() ? "" : pkg + ".", false);
            }
            // Simple type names the file mentions (same-package dependencies).
            new TreeScanner<Void, Void>() {
                @Override public Void visitIdentifier(IdentifierTree node, Void v) {
                    String n = node.getName().toString();
                    if (!n.isEmpty() && Character.isUpperCase(n.charAt(0))) idents.add(n);
                    return null;
                }
            }.scan(cu, null);
            StringBuilder sb = new StringBuilder(4096);
            sb.append("{\"rel\":"); str(sb, in.rel);
            sb.append(",\"module\":"); str(sb, in.module);
            sb.append(",\"role\":"); str(sb, in.role);
            sb.append(",\"pkg\":"); str(sb, pkg);
            sb.append(",\"loc\":").append(src.isEmpty() ? 0 : src.split("\n", -1).length - (src.endsWith("\n") ? 1 : 0));
            sb.append(",\"imports\":[").append(imports).append(']');
            sb.append(",\"idents\":[");
            boolean first = true;
            for (String i : idents) { if (!first) sb.append(','); first = false; str(sb, i); }
            sb.append("],\"types\":[").append(types).append("],\"members\":[").append(members).append("]}");
            return sb.toString();
        }

        void type(ClassTree ct, String parentQn, String prefix, boolean parentIsInterface) {
            String name = ct.getSimpleName().toString();
            if (name.isEmpty()) return;
            String qn = prefix + name;
            Tree.Kind k = ct.getKind();
            String kind = switch (k) {
                case INTERFACE -> "interface";
                case ENUM -> "enum";
                case RECORD -> "record";
                case ANNOTATION_TYPE -> "annotation";
                default -> "class";
            };
            boolean isInterface = k == Tree.Kind.INTERFACE || k == Tree.Kind.ANNOTATION_TYPE;
            if (types.length() > 0) types.append(',');
            types.append("{\"qn\":"); str(types, qn);
            types.append(",\"name\":"); str(types, name);
            types.append(",\"kind\":"); str(types, kind);
            types.append(",\"parent\":"); if (parentQn == null) types.append("null"); else str(types, parentQn);
            types.append(",\"vis\":"); str(types, visibility(ct.getModifiers(), parentIsInterface));
            types.append(",\"abstract\":").append(ct.getModifiers().getFlags().contains(Modifier.ABSTRACT));
            types.append(",\"line\":").append(cu.getLineMap().getLineNumber(s(ct)));
            types.append(",\"extends\":[");
            if (ct.getExtendsClause() != null) str(types, ws(typeText(ct.getExtendsClause())));
            types.append("],\"implements\":[");
            boolean f = true;
            for (Tree t : ct.getImplementsClause()) { if (!f) types.append(','); f = false; str(types, ws(typeText(t))); }
            types.append("]}");
            for (Tree m : ct.getMembers()) {
                if (m instanceof ClassTree inner) {
                    type(inner, qn, qn + "$", isInterface);
                } else if (m instanceof MethodTree mt) {
                    method(mt, qn, name, isInterface);
                    localTypes(mt.getBody(), qn);
                } else if (m instanceof VariableTree vt) {
                    field(vt, qn, k == Tree.Kind.ENUM, isInterface, name);
                    localTypes(vt.getInitializer(), qn);
                } else if (m instanceof BlockTree bt) {
                    localTypes(bt, qn);
                }
            }
        }

        /** Named local types declared in a body (anonymous classes are skipped). */
        void localTypes(Tree body, String owner) {
            if (body == null) return;
            new TreeScanner<Void, Void>() {
                @Override public Void visitClass(ClassTree c, Void v) {
                    if (!c.getSimpleName().toString().isEmpty()) {
                        type(c, owner, owner + "$", false);
                        return null; // its own members are walked by type()
                    }
                    return super.visitClass(c, v);
                }
                @Override public Void visitNewClass(NewClassTree n, Void v) {
                    // Anonymous class bodies may declare named local types too.
                    return super.visitNewClass(n, v);
                }
            }.scan(body, null);
        }

        /** A type's text without annotations and without type arguments' spacing changes. */
        String typeText(Tree t) {
            List<Tree> annots = new ArrayList<>();
            new TreeScanner<Void, Void>() {
                @Override public Void visitAnnotation(AnnotationTree a, Void v) { annots.add(a); return null; }
            }.scan(t, null);
            return textWithout(s(t), e(t), annots).trim();
        }

        void method(MethodTree mt, String owner, String ownerName, boolean inInterface) {
            boolean ctor = mt.getReturnType() == null;
            String name = ctor ? ownerName : mt.getName().toString();
            if (name.equals("<init>")) name = ownerName;
            // Parameter list as written: from '(' after the name to its matching ')'.
            long nameEnd = ctor ? s(mt) : e(mt.getReturnType());
            int open = src.indexOf('(', (int) Math.max(nameEnd, s(mt)));
            int close = matching(open);
            List<Tree> annots = new ArrayList<>();
            for (VariableTree p : mt.getParameters()) annots.addAll(p.getModifiers().getAnnotations());
            if (mt.getReceiverParameter() != null) annots.addAll(mt.getReceiverParameter().getModifiers().getAnnotations());
            String params = open >= 0 && close > open ? ws(stripComments(textWithout(open + 1, close, annots))) : "";
            String typeParams = "";
            if (!mt.getTypeParameters().isEmpty()) {
                Tree a = mt.getTypeParameters().get(0);
                Tree b = mt.getTypeParameters().get(mt.getTypeParameters().size() - 1);
                int lt = src.lastIndexOf('<', (int) s(a));
                int gt = src.indexOf('>', (int) e(b));
                if (lt >= 0 && gt > lt) typeParams = ws(src.substring(lt, gt + 1));
            }
            String ret = ctor ? "" : ws(typeText(mt.getReturnType()));
            String retFull = ctor ? "" : (typeParams.isEmpty() ? ret : typeParams + " " + ret);
            String sig = (ctor ? (typeParams.isEmpty() ? "" : typeParams + " ") : retFull + " ") + name + "(" + params + ")";
            List<String> an = new ArrayList<>();
            for (AnnotationTree a : mt.getModifiers().getAnnotations()) {
                String t = a.getAnnotationType().toString();
                an.add(t.substring(t.lastIndexOf('.') + 1));
            }
            member(owner + "#" + name + "(" + params + ")", name, ctor ? "constructor" : "method",
                    modifiers(mt.getModifiers()), owner, retFull, sig, cu.getLineMap().getLineNumber(s(mt)), an);
        }

        void field(VariableTree vt, String owner, boolean inEnum, boolean inInterface, String ownerName) {
            String name = vt.getName().toString();
            // An enum constant's declaration starts with its own name (no type).
            boolean constant = inEnum && text(s(vt), e(vt)).replaceAll("^(\\s*@\\w+(\\([^)]*\\))?)*\\s*", "").startsWith(name);
            String type = constant ? ownerName : ws(typeText(vt.getType()));
            if (!constant && (type.contains(" " + name) || type.contains(",") || type.contains("/*"))) {
                // C-style array declarators (`int a[], b[]`) or comments in the type
                // text: javac's own rendering of the declared type.
                type = vt.getType().toString();
            }
            String sig = type + " " + name;
            if (constant) {
                sig = name;
                int p = (int) s(vt) + text(s(vt), e(vt)).indexOf(name) + name.length();
                int q = p;
                while (q < src.length() && Character.isWhitespace(src.charAt(q))) q++;
                if (q < src.length() && src.charAt(q) == '(') {
                    int c = matching(q);
                    if (c > q) sig = name + ws(stripComments(src.substring(q, c + 1)));
                }
            }
            String mods = constant ? "public static final" : modifiers(vt.getModifiers());
            member(owner + "#" + name, name, constant ? "enumConstant" : "field", mods,
                    owner, type, sig, cu.getLineMap().getLineNumber(s(vt)), List.of());
        }

        int matching(int open) {
            if (open < 0) return -1;
            int depth = 0;
            for (int i = open; i < src.length(); i++) {
                char c = src.charAt(i);
                if (c == '(') depth++;
                else if (c == ')') { depth--; if (depth == 0) return i; }
            }
            return -1;
        }

        void member(String qn, String name, String kind, String mods, String owner, String ret, String sig, long line, List<String> annots) {
            if (members.length() > 0) members.append(',');
            members.append("{\"qn\":"); str(members, qn);
            members.append(",\"name\":"); str(members, name);
            members.append(",\"kind\":"); str(members, kind);
            members.append(",\"mods\":"); str(members, mods);
            members.append(",\"owner\":"); str(members, owner);
            members.append(",\"ret\":"); str(members, ret);
            members.append(",\"sig\":"); str(members, sig);
            members.append(",\"line\":").append(line);
            members.append(",\"annots\":[");
            for (int i = 0; i < annots.size(); i++) { if (i > 0) members.append(','); str(members, annots.get(i)); }
            members.append("]}");
        }
    }

    static void str(StringBuilder sb, String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> sb.append("\\\"");
                case '\\' -> sb.append("\\\\");
                case '\n' -> sb.append("\\n");
                case '\r' -> sb.append("\\r");
                case '\t' -> sb.append("\\t");
                default -> { if (c < 0x20) sb.append(String.format("\\u%04x", (int) c)); else sb.append(c); }
            }
        }
        sb.append('"');
    }
}
