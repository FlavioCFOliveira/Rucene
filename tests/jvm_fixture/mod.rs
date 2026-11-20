//! Launches the Java oracles of the test suites without paying the Maven and
//! source-launch start-up costs on every call.
//!
//! The tests build their oracle command exactly as before and pass it through
//! [`launch`], which returns an equivalent command:
//!
//! * **`mvn [-q] [-o] compile exec:java -D… -Dexec.mainClass=M [-Dexec.args=A]`**
//!   (run in the harness directory) becomes the JVM the `mvn` launcher would
//!   have started, running `M` directly:
//!   `java --enable-native-access=ALL-UNNAMED $MAVEN_OPTS -D… -cp target/classes:<runtime class path> M <A>`.
//!   `A` is split as `exec:java` splits `exec.args` (plexus
//!   `CommandLineUtils.translateCommandline`), and the class path is the one
//!   `exec:java` puts on its class loader (project classes, then the runtime
//!   dependencies in Maven's resolution order). The harness is compiled
//!   (`mvn compile`) and its class path resolved
//!   (`mvn dependency:build-classpath`) only when `pom.xml` or a file under
//!   `src/` changed since the last preparation, under a cross-process file
//!   lock, so concurrent test processes never race on `target/classes`.
//! * **`java [options] X.java args…`** (a JEP 330/458 source-file launch,
//!   which compiles `X.java` in memory on every run) becomes
//!   `java [options] -cp <compiled X>:<class path> <first top-level class of X> args…`.
//!   `X.java` is compiled once per change of its source tree by the same
//!   `javac` with the options the source launcher uses
//!   (`jdk.compiler/com.sun.tools.javac.launcher.RelevantJavacOptions`), into
//!   a cache directory keyed by the source, the class path, the
//!   compile-relevant options and the JDK.
//!
//! Every other command is returned unchanged. Whenever the rewrite cannot be
//! shown equivalent (an unrecognised argument, quotes in `exec.args`, a
//! `.mvn` directory, `MAVEN_ARGS`) or preparing it fails (Maven or the JDK
//! absent, a compile error), the original command is returned as it is, so
//! the behaviour of a test without Maven or a JDK, or with a broken fixture,
//! stays exactly what it was.
//!
//! Standard I/O is not carried over: callers configure it on the returned
//! command.
#![allow(dead_code)]

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

/// Returns the command to run in place of `command`: the direct JVM launch
/// equivalent to a Maven `exec:java` or a source-file launch, or a copy of
/// `command` itself.
pub fn launch(command: &Command) -> Command {
    let program = Path::new(command.get_program());
    let rewritten = match program.file_name().and_then(OsStr::to_str) {
        Some("mvn") => maven_exec_java(command),
        Some("java") => java_source_file(command),
        _ => None,
    };
    rewritten.unwrap_or_else(|| {
        rebuild(
            command,
            command.get_program(),
            command.get_args().map(OsStr::to_os_string).collect(),
        )
    })
}

/// A new command running `program` with `args` and the environment and
/// working directory of `original`.
fn rebuild(original: &Command, program: &OsStr, args: Vec<OsString>) -> Command {
    let mut command = Command::new(program);
    command.args(args);
    for (key, value) in original.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    if let Some(dir) = original.get_current_dir() {
        command.current_dir(dir);
    }
    command
}

/// The value `key` has in the environment `command` will run with.
fn env_of(command: &Command, key: &str) -> Option<OsString> {
    for (k, v) in command.get_envs() {
        if k == key {
            return v.map(OsStr::to_os_string);
        }
    }
    std::env::var_os(key)
}

// ---------------------------------------------------------------------------
// mvn compile exec:java
// ---------------------------------------------------------------------------

fn maven_exec_java(command: &Command) -> Option<Command> {
    let harness = command.get_current_dir()?.to_path_buf();
    if !harness.join("pom.xml").is_file() || has_mvn_dir(&harness) {
        return None;
    }
    // The `mvn` launcher appends these to the command line.
    for key in ["MAVEN_ARGS", "MAVEN_CONFIG"] {
        if env_of(command, key).is_some_and(|v| !v.is_empty()) {
            return None;
        }
    }

    let mut quiet = false;
    let mut offline = false;
    let mut goals = Vec::new();
    let mut properties = Vec::new();
    let mut main_class = None;
    let mut exec_args = None;
    for arg in command.get_args() {
        let arg = arg.to_str()?;
        match arg {
            "-q" => quiet = true,
            "-o" => offline = true,
            "compile" | "exec:java" => goals.push(arg),
            _ => {
                let property = arg.strip_prefix("-D")?;
                let (key, value) = property.split_once('=')?;
                match key {
                    "exec.mainClass" => main_class = Some(value.to_string()),
                    "exec.args" => exec_args = Some(value.to_string()),
                    _ => {}
                }
                // Maven sets every `-D` user property as a system property of
                // the JVM the main class runs in.
                properties.push(arg.to_string());
            }
        }
    }
    // Without `-q` Maven logs to the standard output the fixture writes to.
    if !quiet || goals != ["compile", "exec:java"] {
        return None;
    }
    let main_class = main_class?;
    let main_args = match exec_args {
        Some(args) => translate_commandline(&args)?,
        None => Vec::new(),
    };

    let class_path = harness_class_path(command, &harness, offline)?;

    // `mvn` runs `$JAVA_HOME/bin/java`, or the `java` on the PATH.
    let java = match env_of(command, "JAVA_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home)
            .join("bin")
            .join("java")
            .into_os_string(),
        None => OsString::from("java"),
    };
    let mut args = Vec::new();
    if java_accepts_native_access(&java) {
        args.push(OsString::from("--enable-native-access=ALL-UNNAMED"));
    }
    if let Some(opts) = env_of(command, "MAVEN_OPTS") {
        args.extend(opts.to_str()?.split_whitespace().map(OsString::from));
    }
    args.extend(properties.into_iter().map(OsString::from));
    args.push(OsString::from("-cp"));
    args.push(class_path);
    args.push(OsString::from(main_class));
    args.extend(main_args.into_iter().map(OsString::from));
    Some(rebuild(command, &java, args))
}

/// Whether a directory from `dir` upwards holds a `.mvn` directory, whose
/// `jvm.config` or `maven.config` the `mvn` launcher would apply.
fn has_mvn_dir(dir: &Path) -> bool {
    dir.ancestors().any(|d| d.join(".mvn").is_dir())
}

/// `org.codehaus.plexus.util.cli.CommandLineUtils.translateCommandline` for
/// text without quotes: split on the space character, empty tokens dropped.
/// Text with quotes is not rewritten.
fn translate_commandline(text: &str) -> Option<Vec<String>> {
    if text.contains(['"', '\'']) {
        return None;
    }
    Some(
        text.split(' ')
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// `mvn` adds `--enable-native-access=ALL-UNNAMED` when its JVM accepts it
/// (`bin/mvn`, MNG-8248).
fn java_accepts_native_access(java: &OsStr) -> bool {
    static PROBES: OnceLock<Mutex<HashMap<OsString, bool>>> = OnceLock::new();
    let probes = PROBES.get_or_init(Default::default);
    if let Some(&known) = lock(probes).get(java) {
        return known;
    }
    let accepted = Command::new(java)
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    lock(probes).insert(java.to_os_string(), accepted);
    accepted
}

/// `target/classes` followed by the runtime class path of the harness,
/// compiling the harness first when its sources changed.
fn harness_class_path(command: &Command, harness: &Path, offline: bool) -> Option<OsString> {
    static PREPARED: OnceLock<Mutex<HashMap<PathBuf, OsString>>> = OnceLock::new();
    let prepared = PREPARED.get_or_init(Default::default);
    if let Some(class_path) = lock(prepared).get(harness) {
        return Some(class_path.clone());
    }

    let target = harness.join("target");
    fs::create_dir_all(&target).ok()?;
    let _lock = FileLock::acquire(&target.join("rucene-harness.lock"))?;

    let classes = target.join("classes");
    let class_path_file = target.join("rucene-harness.classpath");
    let fingerprint_file = target.join("rucene-harness.fingerprint");
    let mut inputs = vec![harness.join("pom.xml")];
    collect_files(&harness.join("src"), &mut |_| true, &mut inputs).ok()?;
    let fingerprint = fingerprint(&inputs, resolve_program(command.get_program()).as_deref())?;

    let up_to_date = classes.is_dir()
        && class_path_file.is_file()
        && fs::read_to_string(&fingerprint_file).is_ok_and(|stored| stored == fingerprint);
    if !up_to_date {
        let _ = fs::remove_file(&fingerprint_file);
        let mut mvn = rebuild(command, command.get_program(), Vec::new());
        mvn.arg("-q");
        if offline {
            mvn.arg("-o");
        }
        let output = mvn
            .arg("compile")
            .arg("dependency:build-classpath")
            .arg(format!("-Dmdep.outputFile={}", class_path_file.display()))
            .arg("-Dmdep.includeScope=runtime")
            .stdin(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() || !classes.is_dir() {
            return None;
        }
        fs::write(&fingerprint_file, &fingerprint).ok()?;
    }

    let dependencies = fs::read_to_string(&class_path_file).ok()?;
    let mut entries = vec![classes];
    entries.extend(
        std::env::split_paths(dependencies.trim_end_matches(['\r', '\n']))
            .filter(|p| !p.as_os_str().is_empty()),
    );
    let class_path = std::env::join_paths(entries).ok()?;
    lock(prepared).insert(harness.to_path_buf(), class_path.clone());
    Some(class_path)
}

// ---------------------------------------------------------------------------
// java X.java
// ---------------------------------------------------------------------------

/// `java` options followed by a separate value (`java --help`, `--help-extra`).
const OPTIONS_WITH_VALUE: [&str; 13] = [
    "-cp",
    "-classpath",
    "--class-path",
    "-p",
    "--module-path",
    "--upgrade-module-path",
    "--add-modules",
    "--add-exports",
    "--add-opens",
    "--add-reads",
    "--limit-modules",
    "--patch-module",
    "--enable-native-access",
];

/// Options the source launcher hands to `javac`
/// (`RelevantJavacOptions`); the class path is handled apart.
const JAVAC_OPTIONS: [&str; 7] = [
    "-p",
    "--module-path",
    "--upgrade-module-path",
    "--add-modules",
    "--add-exports",
    "--limit-modules",
    "--patch-module",
];

/// The fixed `javac` options of the source launcher
/// (`RelevantJavacOptions`), except `-implicit:none`: the classes of the
/// source tree the launcher compiles on demand are written out here
/// (`-implicit:class`) so they can be loaded from the cache.
const LAUNCHER_JAVAC_OPTIONS: [&str; 8] = [
    "-proc:none",
    "-implicit:class",
    "-Xprefer:source",
    "-Xdiags:verbose",
    "-Xlint:deprecation",
    "-Xlint:unchecked",
    "-Xlint:-options",
    "-XDsourceLauncher",
];

fn java_source_file(command: &Command) -> Option<Command> {
    let args: Vec<&OsStr> = command.get_args().collect();
    let mut options: Vec<OsString> = Vec::new();
    let mut javac_options: Vec<OsString> = Vec::new();
    let mut class_path: Option<(usize, OsString)> = None;
    let mut index = 0;
    let source = loop {
        let arg = *args.get(index)?;
        let text = arg.to_str()?;
        if !text.starts_with('-') {
            break PathBuf::from(arg);
        }
        if text == "--source"
            || text.starts_with("--source=")
            || text == "-m"
            || text == "--module"
            || text.starts_with("--module=")
            || text == "-jar"
            || text == "--enable-preview"
            || text.starts_with("-Djdk.internal.javac.source")
            || text.starts_with("-agentlib:jdwp")
            || text.starts_with("-Xrunjdwp")
        {
            return None;
        }
        let (name, inline_value) = match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (text, None),
        };
        let value: Option<OsString> = match inline_value {
            Some(value) => Some(OsString::from(value)),
            None if OPTIONS_WITH_VALUE.contains(&name) => {
                index += 1;
                Some(args.get(index)?.to_os_string())
            }
            None => None,
        };
        if matches!(name, "-cp" | "-classpath" | "--class-path") {
            options.push(OsString::from(name));
            class_path = Some((options.len(), value?));
            options.push(OsString::new());
        } else {
            if JAVAC_OPTIONS.contains(&name) {
                javac_options.push(OsString::from(name));
                javac_options.push(value.clone()?);
            }
            // Every other option is passed on as it was written.
            options.push(arg.to_os_string());
            if inline_value.is_none() {
                if let Some(value) = value {
                    options.push(value);
                }
            }
        }
        index += 1;
    };
    if source.extension() != Some(OsStr::new("java")) {
        return None;
    }
    let rest: Vec<OsString> = args[index + 1..].iter().map(|a| a.to_os_string()).collect();

    // Without `-cp`, the launcher compiles and runs with `CLASSPATH`, or the
    // working directory.
    let original_class_path = match &class_path {
        Some((_, value)) => value.clone(),
        None => env_of(command, "CLASSPATH")
            .filter(|cp| !cp.is_empty())
            .unwrap_or_else(|| OsString::from(".")),
    };
    let source = match command.get_current_dir() {
        Some(dir) if source.is_relative() => dir.join(source),
        _ => source,
    };
    let compiled = compile_source(
        command,
        &source,
        class_path.as_ref().map(|(_, cp)| cp.as_os_str()),
        &javac_options,
    )?;

    let mut entries = vec![compiled.classes.into_os_string()];
    entries.push(original_class_path);
    let run_class_path = join_class_path(&entries);
    match class_path {
        Some((slot, _)) => options[slot] = run_class_path,
        None => {
            options.push(OsString::from("-cp"));
            options.push(run_class_path);
        }
    }
    options.push(OsString::from(compiled.main_class));
    options.extend(rest);
    Some(rebuild(command, command.get_program(), options))
}

/// Joins class-path entries with the platform separator, keeping each entry
/// as given (an entry may itself be a list).
fn join_class_path(entries: &[OsString]) -> OsString {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let mut joined = OsString::new();
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            joined.push(separator);
        }
        joined.push(entry);
    }
    joined
}

struct Compiled {
    classes: PathBuf,
    main_class: String,
}

fn compile_source(
    command: &Command,
    source: &Path,
    class_path: Option<&OsStr>,
    javac_options: &[OsString],
) -> Option<Compiled> {
    static COMPILED: OnceLock<Mutex<HashMap<u64, (PathBuf, String)>>> = OnceLock::new();
    let compiled = COMPILED.get_or_init(Default::default);

    let source = fs::canonicalize(source).ok()?;
    let java = resolve_program(command.get_program())?;
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    class_path.hash(&mut hasher);
    javac_options.hash(&mut hasher);
    java.hash(&mut hasher);
    if class_path.is_none() {
        // The compile class path then comes from the environment.
        env_of(command, "CLASSPATH").hash(&mut hasher);
        command.get_current_dir().hash(&mut hasher);
    }
    let key = hasher.finish();
    if let Some((classes, main_class)) = lock(compiled).get(&key) {
        return Some(Compiled {
            classes: classes.clone(),
            main_class: main_class.clone(),
        });
    }

    let text = fs::read_to_string(&source).ok()?;
    let declaration = first_declaration(&text)?;
    let main_class = match &declaration.package {
        Some(package) => format!("{package}.{}", declaration.class),
        None => declaration.class.clone(),
    };
    // JEP 458: the launcher compiles other classes on demand from the source
    // tree rooted at the file's directory (unnamed package), or at the
    // directory above the package directories when they match the package.
    let root = match &declaration.package {
        None => source.parent().map(Path::to_path_buf),
        Some(package) => (|| {
            let mut dir = source.parent()?;
            for component in package.rsplit('.') {
                if dir.file_name() != Some(OsStr::new(component)) {
                    return None;
                }
                dir = dir.parent()?;
            }
            Some(dir.to_path_buf())
        })(),
    };

    let stem = source.file_stem()?.to_string_lossy().into_owned();
    let dir = cache_root().join(format!("{stem}-{key:016x}"));
    fs::create_dir_all(&dir).ok()?;
    let _lock = FileLock::acquire(&dir.join("lock"))?;

    let mut inputs = Vec::new();
    match &root {
        Some(root) => collect_files(
            root,
            &mut |p| p.extension() == Some(OsStr::new("java")),
            &mut inputs,
        )
        .ok()?,
        None => inputs.push(source.clone()),
    }
    let fingerprint = fingerprint(&inputs, Some(&java))?;
    let fingerprint_file = dir.join("fingerprint");
    let classes = dir.join("classes");
    let class_file = classes.join(format!("{}.class", main_class.replace('.', "/")));
    let up_to_date = class_file.is_file()
        && fs::read_to_string(&fingerprint_file).is_ok_and(|stored| stored == fingerprint);
    if !up_to_date {
        let _ = fs::remove_file(&fingerprint_file);
        let _ = fs::remove_dir_all(&classes);
        let mut javac = rebuild(command, command.get_program(), Vec::new());
        javac
            .args(["-m", "jdk.compiler/com.sun.tools.javac.Main"])
            .args(LAUNCHER_JAVAC_OPTIONS)
            .args(javac_options);
        if let Some(class_path) = class_path {
            javac.arg("-cp").arg(class_path);
        }
        if let Some(root) = &root {
            javac.arg("--source-path").arg(root);
        }
        let output = javac
            .arg("-d")
            .arg(&classes)
            .arg(&source)
            .stdin(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() || !class_file.is_file() {
            return None;
        }
        fs::write(&fingerprint_file, &fingerprint).ok()?;
    }

    lock(compiled).insert(key, (classes.clone(), main_class.clone()));
    Some(Compiled {
        classes,
        main_class,
    })
}

/// Where compiled source files are cached: the integration-test scratch
/// directory of the target directory, or the system temporary directory for
/// unit tests.
fn cache_root() -> PathBuf {
    match option_env!("CARGO_TARGET_TMPDIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::temp_dir(),
    }
    .join("rucene-jvm-fixtures")
}

struct Declaration {
    package: Option<String>,
    class: String,
}

/// The package and the first top-level class, interface, enum or record
/// declared in `text`: the class the source launcher runs.
fn first_declaration(text: &str) -> Option<Declaration> {
    let tokens = java_tokens(text);
    let mut package = None;
    let mut depth = 0usize;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "{" => depth += 1,
            "}" => depth = depth.checked_sub(1)?,
            "package" if depth == 0 && package.is_none() => {
                let mut name = String::new();
                i += 1;
                while i < tokens.len() && tokens[i] != ";" {
                    name.push_str(&tokens[i]);
                    i += 1;
                }
                package = Some(name);
            }
            "class" | "interface" | "enum" | "record" if depth == 0 => {
                // `@interface` declares an annotation type, still a class.
                let name = tokens.get(i + 1)?;
                if name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
                {
                    return Some(Declaration {
                        package,
                        class: name.clone(),
                    });
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Identifiers, `.`, `;`, braces and other punctuation of Java source, with
/// comments and string, text-block and character literals removed.
fn java_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c == '"' && chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"') {
            i += 3;
            while i < chars.len()
                && !(chars[i] == '"'
                    && chars.get(i + 1) == Some(&'"')
                    && chars.get(i + 2) == Some(&'"'))
            {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 3;
        } else if c == '"' || c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            tokens.push(chars[start..i].iter().collect());
        } else {
            if !c.is_whitespace() {
                tokens.push(c.to_string());
            }
            i += 1;
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// Shared pieces
// ---------------------------------------------------------------------------

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// An exclusive advisory lock on a file, held until dropped. It excludes
/// other processes and other threads of this one (each acquisition opens its
/// own file description).
struct FileLock(File);

impl FileLock {
    fn acquire(path: &Path) -> Option<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .ok()?;
        file.lock().ok()?;
        Some(Self(file))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Every file under `dir` accepted by `keep`, in a deterministic order.
fn collect_files(
    dir: &Path,
    keep: &mut dyn FnMut(&Path) -> bool,
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, keep, out)?;
        } else if keep(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// Path, size and modification time of every input, and of the tool that
/// builds from them: a change to any of them invalidates the build.
fn fingerprint(inputs: &[PathBuf], tool: Option<&Path>) -> Option<String> {
    let mut text = String::new();
    for path in inputs.iter().map(PathBuf::as_path).chain(tool) {
        let meta = fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        text.push_str(&format!(
            "{}\t{}\t{}\n",
            path.display(),
            meta.len(),
            modified
        ));
    }
    Some(text)
}

/// The file `program` names, looked up on the `PATH` when it has no
/// directory part, with symbolic links resolved.
fn resolve_program(program: &OsStr) -> Option<PathBuf> {
    let program = Path::new(program);
    if program.components().count() > 1 {
        return fs::canonicalize(program).ok();
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .and_then(|found| fs::canonicalize(found).ok())
}
