//! Task #339 oracle parity: every `mockfile` filesystem driven through the
//! operation sequences of `MockfileOracle.java` (Lucene 10.5.0's
//! `lucene-test-framework` jar on JDK 25) must give the same results,
//! exception classes and messages (`tests/fixtures/mockfile.tsv`).

mod oracle_support;

use std::collections::BTreeMap;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use oracle_support::{assert_same, io_error, labelled, mask, panic_error, TempDir};
use rucene::store::java_nio_file::{
    default_file_system, java_to_string, AsynchronousFileChannel, AttributeValue, CopyOption,
    DirectoryStream, FileChannel, FileSystem, FileSystemProvider, Files, OpenOption, Path,
};
use rucene::util::{IOUtils, InfoStream};
use rucene_test_framework::mockfile::{
    DisableFsyncFS, ExtrasFS, FilterFileSystem, FilterPath, HandleLimitFS, LeakFS, MaxOpenHandles,
    ShuffleFS, VerboseFS, VirusCheckingFS, WindowsFS,
};

struct Script {
    rows: BTreeMap<String, String>,
    dir: String,
    rel: Option<String>,
}

impl Script {
    fn m(&self, s: &str) -> String {
        mask(s, &self.dir, self.rel.as_deref())
    }

    fn row(&mut self, label: &str, op: impl FnOnce() -> io::Result<String>) {
        let line = match catch_unwind(AssertUnwindSafe(op)) {
            Ok(Ok(r)) => format!("OK\t{}", self.m(&r)),
            Ok(Err(e)) => {
                let (c, m) = io_error(&e);
                format!("{c}\t{}", self.m(&m))
            }
            Err(p) => {
                let t = randomizedtesting::jdk::exception::Throwable::from_panic(p);
                let cause = t
                    .cause()
                    .map(|c| {
                        format!(
                            " <- {}: {}",
                            c.class_name(),
                            self.m(c.message().unwrap_or("null"))
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "{}\t{}{cause}",
                    t.class_name(),
                    self.m(t.message().unwrap_or("<null>"))
                )
            }
        };
        self.rows.insert(label.to_string(), line);
    }
}

fn fresh(base: &str, name: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(base).join(name);
    std::fs::create_dir(&p).unwrap();
    p
}

fn filter_path(p: &std::path::Path, fs: &Arc<dyn FileSystem>) -> Path {
    let filter = Arc::clone(fs)
        .as_any()
        .downcast_ref::<FilterFileSystem>()
        .map(|_| ())
        .is_some();
    assert!(filter);
    Path::new(default_file_system(), p.as_os_str()).with_file_system(Arc::clone(fs))
}

fn names(mut s: Box<dyn DirectoryStream>) -> io::Result<Vec<String>> {
    let mut l = Vec::new();
    for p in s.iterator() {
        l.push(p?.get_file_name().unwrap().to_string());
    }
    s.close()?;
    Ok(l)
}

fn list(v: &[String]) -> String {
    format!("[{}]", v.join(", "))
}

/// A handle of each kind the oracle opens, as something closeable.
enum Handle {
    In(Box<dyn rucene::store::java_nio_file::InputStream>),
    Out(Box<dyn rucene::store::java_nio_file::OutputStream>),
    Channel(Arc<dyn FileChannel>),
    Byte(Box<dyn rucene::store::java_nio_file::SeekableByteChannel>),
    Dir(Box<dyn DirectoryStream>),
    Async(Arc<dyn AsynchronousFileChannel>),
}

impl Handle {
    fn to_java_string(&self) -> String {
        match self {
            Handle::In(s) => s.stream_to_string(),
            Handle::Out(s) => s.stream_to_string(),
            Handle::Channel(c) => c.channel_to_string(),
            Handle::Byte(c) => c.channel_to_string(),
            Handle::Dir(s) => s.stream_to_string(),
            Handle::Async(c) => c.channel_to_string(),
        }
    }

    fn close(&mut self) -> io::Result<()> {
        match self {
            Handle::In(s) => s.close(),
            Handle::Out(s) => s.close(),
            Handle::Channel(c) => c.close(),
            Handle::Byte(c) => c.close(),
            Handle::Dir(s) => s.close(),
            Handle::Async(c) => c.close(),
        }
    }
}

/// The oracle's capturing `InfoStream`.
struct Capture(Arc<Mutex<Vec<String>>>);

impl InfoStream for Capture {
    fn java_class_name(&self) -> &'static str {
        "MockfileOracle$1"
    }
    fn message(&self, component: &str, message: &str) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{component}: {message}"));
    }
    fn is_enabled(&self, _component: &str) -> bool {
        true
    }
    fn close(&self) {}
}

#[test]
fn mockfile_filesystems_match_lucene() {
    let tmp = TempDir::new("mockfile-oracle");
    let base_dir = tmp.str();
    let mut s = Script {
        rows: BTreeMap::new(),
        dir: base_dir.clone(),
        rel: None,
    };
    let def = default_file_system();

    // LeakFS: every handle kind, leaked and closed.
    for kind in [
        "InputStream",
        "OutputStream",
        "FileChannel",
        "SeekableByteChannel",
        "DirectoryStream",
        "AsynchronousFileChannel",
    ] {
        let base = fresh(&base_dir, &format!("leak-{kind}"));
        std::fs::write(base.join("f"), [1u8]).unwrap();
        let fs = LeakFS::new(Arc::clone(&def)).get_file_system();
        let f = filter_path(&base.join("f"), &fs);
        let dir = filter_path(&base, &fs);
        let mut c = match kind {
            "InputStream" => Handle::In(Files::new_input_stream(&f, &[]).unwrap()),
            "OutputStream" => Handle::Out(Files::new_output_stream(&f, &[]).unwrap()),
            "FileChannel" => {
                Handle::Channel(<dyn FileChannel>::open(&f, &[OpenOption::Read]).unwrap())
            }
            "SeekableByteChannel" => Handle::Byte(Files::new_byte_channel(&f, &[]).unwrap()),
            "DirectoryStream" => Handle::Dir(Files::new_directory_stream(&dir).unwrap()),
            _ => Handle::Async(
                f.get_file_system()
                    .provider()
                    .new_asynchronous_file_channel(&f, &[OpenOption::Read])
                    .unwrap(),
            ),
        };
        let text = c.to_java_string();
        s.row(&format!("LeakFS {kind} toString"), || Ok(text));
        s.row(&format!("LeakFS {kind} leaked close"), || {
            fs.close().map(|_| "-".into())
        });
        c.close().unwrap();
        c.close().unwrap();
        s.row(&format!("LeakFS {kind} closed close"), || {
            fs.close().map(|_| "-".into())
        });
    }
    s.row("LeakFS provider toString", || {
        Ok(LeakFS::new(Arc::clone(&def)).provider_to_string())
    });
    s.row("LeakFS scheme", || {
        Ok(FileSystemProvider::get_scheme(&*LeakFS::new(Arc::clone(
            &def,
        ))))
    });

    // HandleLimitFS.
    {
        let base = fresh(&base_dir, "limit");
        std::fs::write(base.join("f"), [1u8]).unwrap();
        let provider = HandleLimitFS::new(Arc::clone(&def), 2);
        let fs = FileSystemProvider::get_file_system(&*provider);
        let f = filter_path(&base.join("f"), &fs);
        let mut a = Files::new_input_stream(&f, &[]).unwrap();
        let mut b = Files::new_input_stream(&f, &[]).unwrap();
        s.row("HandleLimitFS count 2", || Ok(provider.count().to_string()));
        s.row("HandleLimitFS third", || {
            Files::new_input_stream(&f, &[]).map(|x| x.stream_to_string())
        });
        s.row("HandleLimitFS count after refusal", || {
            Ok(provider.count().to_string())
        });
        a.close().unwrap();
        s.row("HandleLimitFS count after close", || {
            Ok(provider.count().to_string())
        });
        s.row("HandleLimitFS reopen", || {
            Files::new_input_stream(&f, &[])?.close()?;
            Ok(provider.count().to_string())
        });
        b.close().unwrap();
        s.row("HandleLimitFS count end", || {
            Ok(provider.count().to_string())
        });
        s.row("HandleLimitFS MAX_OPEN_FILES", || {
            Ok(MaxOpenHandles::MAX_OPEN_FILES.to_string())
        });
    }

    // WindowsFS.
    {
        let base = fresh(&base_dir, "windows");
        let fs = WindowsFS::new(Arc::clone(&def)).get_file_system();
        let dir = Arc::clone(&fs).get_path(&base.to_string_lossy(), &[]);
        let f = dir.resolve_str("f");
        Files::write(&f, &[1], &[]).unwrap();
        let mut o = Files::new_output_stream(&f, &[OpenOption::Append]).unwrap();
        s.row("WindowsFS delete open", || {
            Files::delete(&f).map(|_| "-".into())
        });
        s.row("WindowsFS deleteIfExists open", || {
            Files::delete_if_exists(&f).map(|b| b.to_string())
        });
        s.row("WindowsFS move open source", || {
            Files::r#move(&f, &dir.resolve_str("g"), &[]).map(|p| p.to_string())
        });
        Files::write(&dir.resolve_str("h"), &[2], &[]).unwrap();
        s.row("WindowsFS move onto open target", || {
            Files::r#move(&dir.resolve_str("h"), &f, &[CopyOption::ReplaceExisting])
                .map(|p| p.to_string())
        });
        s.row("WindowsFS close after transfer", || {
            o.close().map(|_| "-".into())
        });
        s.row("WindowsFS delete closed", || {
            Files::delete(&f)?;
            Ok(Files::exists(&f, &[]).to_string())
        });
        s.row("WindowsFS delete missing", || {
            Files::delete(&f).map(|_| "-".into())
        });
        s.row("WindowsFS deleteIfExists missing", || {
            Files::delete_if_exists(&f).map(|b| b.to_string())
        });
        for name in [
            "CON", "aux", "LPT1", "a:b", "a<b", "q?", "x*", "ok-name", "COM10",
        ] {
            s.row(&format!("WindowsFS resolve {name}"), || {
                Ok(dir.resolve_str(name).get_file_name().unwrap().to_string())
            });
        }
        s.row("WindowsFS resolve root", || {
            Ok(dir.resolve(&Arc::clone(&fs).get_path("/", &[])).to_string())
        });
        let g = dir.resolve_str("g2");
        Files::write(&g, &[1], &[]).unwrap();
        let ch = <dyn FileChannel>::open(&g, &[OpenOption::Read]).unwrap();
        s.row("WindowsFS delete open channel", || {
            Files::delete(&g).map(|_| "-".into())
        });
        ch.close().unwrap();
        s.row("WindowsFS delete after channel", || {
            Files::delete(&g).map(|_| "-".into())
        });
        s.row("WindowsFS mismatch", || {
            Ok(dir.resolve(&Path::new(Arc::clone(&def), "x")).to_string())
        });
        let leak = LeakFS::new(Arc::clone(&def)).get_file_system();
        s.row("WindowsFS mismatch filter", || {
            Ok(dir
                .resolve(&Arc::clone(&leak).get_path("x", &[]))
                .to_string())
        });
    }

    // VirusCheckingFS.
    for salt in [0i64, 1, 42, -7, 0x5DEECE66D] {
        let base = fresh(&base_dir, &format!("virus{salt}"));
        let provider = VirusCheckingFS::new(Arc::clone(&def), salt);
        let fs = FileSystemProvider::get_file_system(&*provider);
        let dir = filter_path(&base, &fs);
        let mut sb = Vec::new();
        for i in 0..20 {
            let p = dir.resolve_str(&if i == 7 {
                "write.lock".to_string()
            } else {
                format!("f{i}")
            });
            Files::write(&p, &[1], &[]).unwrap();
            match Files::delete(&p) {
                Ok(()) => sb.push(format!("{i}:ok")),
                Err(e)
                    if rucene::store::java_nio_file::java_class_of(&e)
                        == "java.nio.file.AccessDeniedException" =>
                {
                    sb.push(format!("{i}:denied"))
                }
                Err(e) => panic!("{e}"),
            }
        }
        s.row(&format!("VirusCheckingFS salt={salt}"), || Ok(sb.join(" ")));
        s.row(
            &format!("VirusCheckingFS salt={salt} refusal message"),
            || {
                for i in 0..40 {
                    let p = dir.resolve_str(&format!("g{i}"));
                    Files::write(&p, &[1], &[])?;
                    if let Err(e) = Files::delete(&p) {
                        return Ok(java_to_string(&e));
                    }
                }
                Ok("none".into())
            },
        );
        s.row(&format!("VirusCheckingFS salt={salt} missing"), || {
            Files::delete(&dir.resolve_str("missing")).map(|_| "-".into())
        });
        provider.disable();
        s.row(&format!("VirusCheckingFS salt={salt} disabled"), || {
            let mut n = 0;
            for i in 0..20 {
                let p = dir.resolve_str(&format!("h{i}"));
                Files::write(&p, &[1], &[])?;
                Files::delete(&p)?;
                n += 1;
            }
            Ok(format!("{n} {}", provider.is_enabled()))
        });
    }

    // ShuffleFS.
    for seed in [0i64, 1, -1, 42, i64::MIN, 0xDEADBEEF] {
        let base = fresh(&base_dir, &format!("shuffle{seed}"));
        for n in [
            "b",
            "a",
            "_0.cfs",
            "segments_1",
            "write.lock",
            "c",
            "Z",
            "\u{e9}",
            "10",
            "2",
        ] {
            std::fs::write(base.join(n), [1u8]).unwrap();
        }
        let fs = ShuffleFS::new(Arc::clone(&def), seed).get_file_system();
        let dir = filter_path(&base, &fs);
        s.row(&format!("ShuffleFS seed={seed}"), || {
            Ok(list(&names(Files::new_directory_stream(&dir)?)?))
        });
        s.row(&format!("ShuffleFS seed={seed} glob"), || {
            Ok(list(&names(Files::new_directory_stream_glob(
                &dir, "[a-c]*",
            )?)?))
        });
    }

    // ExtrasFS.
    for (active, create_directory) in [(true, false), (true, true), (false, true)] {
        let base = fresh(&base_dir, &format!("extras{active}{create_directory}"));
        let fs = ExtrasFS::new(Arc::clone(&def), active, create_directory).get_file_system();
        let dir = filter_path(&base, &fs);
        s.row(
            &format!("ExtrasFS active={active} dir={create_directory}"),
            || {
                let sub = dir.resolve_str("sub");
                Files::create_directory(&sub)?;
                let l = names(Files::new_directory_stream(&sub)?)?;
                let is_dir = if l.is_empty() {
                    String::new()
                } else {
                    Files::is_directory(&sub.resolve_str(&l[0]), &[]).to_string()
                };
                Ok(format!("{} {is_dir}", list(&l)))
            },
        );
        s.row(
            &format!("ExtrasFS active={active} dir={create_directory} createDirectories"),
            || {
                let q = dir.resolve_str("p/q");
                Files::create_directories(&q)?;
                let mut l = names(Files::new_directory_stream(&dir.resolve_str("p"))?)?;
                l.sort();
                Ok(list(&l))
            },
        );
    }
    s.row("ExtrasFS isExtra", || {
        Ok(format!(
            "{} {}",
            ExtrasFS::is_extra("extra0"),
            ExtrasFS::is_extra("extra1")
        ))
    });

    // DisableFsyncFS.
    {
        let base = fresh(&base_dir, "fsync");
        let fs = DisableFsyncFS::new(Arc::clone(&def)).get_file_system();
        let f = filter_path(&base.join("f"), &fs);
        s.row("DisableFsyncFS force", || {
            let c = <dyn FileChannel>::open(&f, &[OpenOption::Create, OpenOption::Write])?;
            c.write(&[1, 2])?;
            c.force(true)?;
            let size = c.size()?;
            c.close()?;
            Ok(size.to_string())
        });
        s.row("DisableFsyncFS force closed", || {
            let c = <dyn FileChannel>::open(&f, &[OpenOption::Write])?;
            c.close()?;
            c.force(true)?;
            Ok("no-op".into())
        });
        s.row("DisableFsyncFS IOUtils.fsync", || {
            IOUtils::fsync(&f, false).map_err(|e| io::Error::other(e.to_string()))?;
            Ok("-".into())
        });
    }

    // VerboseFS.
    {
        let base = fresh(&base_dir, "verbose");
        let log = Arc::new(Mutex::new(Vec::new()));
        let fs =
            VerboseFS::new(Arc::clone(&def), Arc::new(Capture(Arc::clone(&log)))).get_file_system();
        let dir = filter_path(&base, &fs);
        let cwd = Path::new(Arc::clone(&def), ".")
            .to_absolute_path()
            .normalize();
        s.rel = Some(
            cwd.relativize(&Path::new(Arc::clone(&def), base.as_os_str()))
                .to_string(),
        );
        type Op<'a> = Box<dyn FnOnce() -> io::Result<()> + 'a>;
        let d = &dir;
        let ops: Vec<Op> = vec![
            Box::new(|| Files::create_directory(&d.resolve_str("d")).map(|_| ())),
            Box::new(|| Files::create_directory(&d.resolve_str("d")).map(|_| ())),
            Box::new(|| Files::new_output_stream(&d.resolve_str("a"), &[])?.close()),
            Box::new(|| {
                Files::new_output_stream(
                    &d.resolve_str("b"),
                    &[OpenOption::CreateNew, OpenOption::Write],
                )?
                .close()
            }),
            Box::new(|| {
                <dyn FileChannel>::open(&d.resolve_str("a"), &[OpenOption::Write])?.close()
            }),
            Box::new(|| <dyn FileChannel>::open(&d.resolve_str("a"), &[OpenOption::Read])?.close()),
            Box::new(|| {
                <dyn FileChannel>::open(&d.resolve_str("nope"), &[OpenOption::Read]).map(|_| ())
            }),
            // `EnumSet.of(CREATE, APPEND)`: the declaration order.
            Box::new(|| {
                Files::new_byte_channel(
                    &d.resolve_str("c"),
                    &[OpenOption::Append, OpenOption::Create],
                )?
                .close()
            }),
            Box::new(|| Files::copy(&d.resolve_str("a"), &d.resolve_str("e"), &[]).map(|_| ())),
            Box::new(|| {
                Files::r#move(
                    &d.resolve_str("e"),
                    &d.resolve_str("g"),
                    &[CopyOption::AtomicMove],
                )
                .map(|_| ())
            }),
            Box::new(|| Files::create_link(&d.resolve_str("l"), &d.resolve_str("a")).map(|_| ())),
            Box::new(|| {
                Files::create_symbolic_link(&d.resolve_str("s"), &d.resolve_str("a")).map(|_| ())
            }),
            Box::new(|| Files::delete(&d.resolve_str("g"))),
            Box::new(|| Files::delete(&d.resolve_str("g"))),
            Box::new(|| Files::delete_if_exists(&d.resolve_str("g")).map(|_| ())),
            Box::new(|| Files::delete_if_exists(&d.resolve_str("a")).map(|_| ())),
            Box::new(|| {
                Files::set_attribute(
                    &d.resolve_str("b"),
                    "basic:bogus",
                    &AttributeValue::Long(1),
                    &[],
                )
                .map(|_| ())
            }),
        ];
        for (i, op) in ops.into_iter().enumerate() {
            log.lock().unwrap().clear();
            let label = format!("VerboseFS op{i}");
            let outcome = catch_unwind(AssertUnwindSafe(op));
            let logged = s.m(&log.lock().unwrap().join(" | "));
            let line = match outcome {
                Ok(Ok(())) => format!("OK\t{logged}"),
                Ok(Err(e)) => {
                    let (c, m) = io_error(&e);
                    format!("{c}\t{}\t{logged}", s.m(&m))
                }
                Err(p) => {
                    let (c, m) = panic_error(p);
                    format!("{c}\t{}\t{logged}", s.m(&m))
                }
            };
            s.rows.insert(label, line);
        }
        s.rel = None;
    }

    // FilterPath and FilterFileSystem.
    {
        let base = fresh(&base_dir, "filter");
        let provider = LeakFS::new(Arc::clone(&def));
        let fs = FileSystemProvider::get_file_system(&*provider);
        let dir = filter_path(&base, &fs);
        s.row("FilterPath toString", || {
            Ok(dir.resolve_str("x").to_string())
        });
        s.row("FilterPath equals", || {
            let x = dir.resolve_str("x");
            let same = x == Arc::clone(&fs).get_path(&base_dir, &["x"]);
            let hash = x.hash_code()
                == Path::new(Arc::clone(&def), base.join("x").as_os_str()).hash_code();
            Ok(format!("{same} {hash}"))
        });
        s.row("FilterPath unwrap", || {
            let u = FilterPath::unwrap(&dir.resolve_str("x"));
            Ok(
                rucene::store::java_nio_file::same_file_system(u.get_file_system(), &def)
                    .to_string(),
            )
        });
        s.row("FilterPath uri", || {
            let p = dir.resolve_str("\u{e4}\u{4e2d}");
            Ok((FileSystemProvider::get_path(&*provider, &p.to_uri()) == p).to_string())
        });
        s.row("FilterFileSystem separator", || Ok(fs.get_separator()));
        s.row("FilterFileSystem roots", || {
            Ok(Arc::clone(&fs).get_root_directories()[0].to_string())
        });
        s.row("FilterFileSystem views", || {
            let mut v = fs.supported_file_attribute_views();
            v.sort();
            Ok(list(&v))
        });
        s.row("FilterFileSystem isOpen", || {
            Ok(format!("{} {}", fs.is_open(), fs.is_read_only()))
        });
        s.row("FilterFileSystem matcher", || {
            let pm = fs.get_path_matcher("glob:*.txt");
            Ok(format!(
                "{} {}",
                pm.matches(&Arc::clone(&fs).get_path("a.txt", &[])),
                pm.matches(&Path::new(Arc::clone(&def), "a.txt"))
            ))
        });
        s.row("FilterFileStore", || {
            let st = Files::get_file_store(&dir)?;
            Ok(format!("{} {}", st.type_(), st.name()))
        });
        s.row("readSymbolicLink unwrapped", || {
            Files::create_symbolic_link(&dir.resolve_str("s"), &dir.resolve_str("t"))?;
            let p = Files::read_symbolic_link(&dir.resolve_str("s"))?;
            Ok(
                rucene::store::java_nio_file::same_file_system(p.get_file_system(), &def)
                    .to_string(),
            )
        });
        s.row("newFileSystem", || {
            let n = rucene_test_framework::mockfile::FilterFileSystemProvider::new_file_system(
                &*provider,
            )?;
            Ok(rucene::store::java_nio_file::same_file_system(&n, &fs).to_string())
        });
    }

    assert_same("mockfile", &labelled("mockfile.tsv"), &s.rows);
}
