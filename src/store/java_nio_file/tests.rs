//! Unit tests of the default filesystem against the values OpenJDK 25
//! reports for the same operations (the probes are listed with each test;
//! the oracle grid is `crates/rucene-test-framework/tests/fixtures/nio_default.tsv`).

use super::*;

fn path(s: &str) -> Path {
    Path::new(default_file_system(), s)
}

fn temp_dir() -> (tempfile::TempDir, Path) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = path(tmp.path().to_str().unwrap())
        .to_real_path(&[])
        .unwrap();
    (tmp, dir)
}

/// `Paths.get(...)` operations measured on OpenJDK 25.
#[test]
fn unix_path_operations_match_the_jdk() {
    assert_eq!(path("abc").hash_code(), 96354);
    assert_eq!(path("").hash_code(), 0);
    assert_eq!(path("ä").hash_code(), 6209);
    assert_eq!(path("").get_file_name().unwrap().to_string(), "");
    assert!(path("/").get_file_name().is_none());
    assert!(path("a").get_parent().is_none());
    assert_eq!(path("a/./b/../c/").normalize().to_string(), "a/c");
    assert_eq!(path("../a/..").normalize().to_string(), "..");
    assert_eq!(
        path("/a/b").relativize(&path("/a/c/d")).to_string(),
        "../c/d"
    );
    assert_eq!(path("/a/b").relativize(&path("/a/b")).to_string(), "");
    assert_eq!(path("a").resolve_str("/x").to_string(), "/x");
    assert_eq!(path("a").resolve_str("").to_string(), "a");
    assert_eq!(path("").resolve_str("x").to_string(), "x");
    assert_eq!(path("a//b/").to_string(), "a/b");
    assert_eq!(path("a").compare_to(&path("b")), -1);
    assert!(path("/a/b").starts_with(&path("/a")));
    assert_eq!(
        default_file_system()
            .get_path("a", &["", "b", "c"])
            .to_string(),
        "a/b/c"
    );
    let nul = std::panic::catch_unwind(|| path("a\0b")).unwrap_err();
    let message = nul.downcast_ref::<String>().unwrap();
    assert!(message.starts_with("java.nio.file.InvalidPathException: Nul character not allowed"));
}

/// The exceptions of the default provider, measured on OpenJDK 25.
#[test]
fn default_provider_errors_match_the_jdk() {
    let (_tmp, d) = temp_dir();
    let class_and_message = |e: io::Error| (java_class_of(&e).to_string(), java_message_of(&e));

    let nope = d.resolve_str("nope");
    assert_eq!(
        class_and_message(Files::delete(&nope).unwrap_err()),
        (NO_SUCH_FILE_EXCEPTION.to_string(), Some(nope.to_string()))
    );
    assert!(!Files::delete_if_exists(&nope).unwrap());
    let sub = d.resolve_str("sub");
    Files::create_directory(&sub).unwrap();
    Files::write(&sub.resolve_str("f"), &[1], &[]).unwrap();
    assert_eq!(
        class_and_message(Files::delete(&sub).unwrap_err()),
        (
            DIRECTORY_NOT_EMPTY_EXCEPTION.to_string(),
            Some(sub.to_string())
        )
    );
    assert_eq!(
        class_and_message(Files::create_directory(&sub).unwrap_err()),
        (
            FILE_ALREADY_EXISTS_EXCEPTION.to_string(),
            Some(sub.to_string())
        )
    );
    let f = sub.resolve_str("f");
    assert_eq!(
        class_and_message(Files::create_file(&f).unwrap_err()),
        (
            FILE_ALREADY_EXISTS_EXCEPTION.to_string(),
            Some(f.to_string())
        )
    );
    assert_eq!(
        class_and_message(
            <dyn FileChannel>::open(&sub, &[OpenOption::Write])
                .err()
                .unwrap()
        ),
        (
            FILE_SYSTEM_EXCEPTION.to_string(),
            Some(format!("{sub}: Is a directory"))
        )
    );
    let nope2 = d.resolve_str("nope2");
    assert_eq!(
        class_and_message(Files::r#move(&nope, &nope2, &[CopyOption::AtomicMove]).unwrap_err()),
        (
            NO_SUCH_FILE_EXCEPTION.to_string(),
            Some(format!("{nope} -> {nope2}"))
        )
    );
    assert_eq!(
        class_and_message(Files::new_directory_stream(&f).err().unwrap()),
        (NOT_DIRECTORY_EXCEPTION.to_string(), Some(f.to_string()))
    );
    let x = f.resolve_str("x");
    assert_eq!(
        class_and_message(Files::create_directories(&x).unwrap_err()),
        (
            FILE_SYSTEM_EXCEPTION.to_string(),
            Some(format!("{x}: Not a directory"))
        )
    );
}

/// `FileChannel`, `FileLock` and the streams, measured on OpenJDK 25.
#[test]
fn default_channels_and_streams_match_the_jdk() {
    let (_tmp, d) = temp_dir();
    let f = d.resolve_str("f");
    Files::write(&f, &[1, 2, 3], &[]).unwrap();
    let fc = <dyn FileChannel>::open(&f, &[OpenOption::Read, OpenOption::Write]).unwrap();
    let lock = fc.try_lock_whole().unwrap().unwrap();
    assert_eq!(
        lock.lock_to_string(),
        "sun.nio.ch.FileLockImpl[0:9223372036854775807 exclusive valid]"
    );
    fc.close().unwrap();
    assert!(!lock.is_valid());
    assert_eq!(
        java_to_string(&fc.size().unwrap_err()),
        "java.nio.channels.ClosedChannelException"
    );
    assert_eq!(
        java_class_of(&lock.release().unwrap_err()),
        CLOSED_CHANNEL_EXCEPTION
    );
    let mut input = Files::new_input_stream(&f, &[]).unwrap();
    assert_eq!(input.available().unwrap(), 3);
    assert_eq!(input.skip(10).unwrap(), 3);
    assert_eq!(input.read_byte().unwrap(), -1);
    assert!(!input.mark_supported());
    assert_eq!(
        java_to_string(&input.reset().unwrap_err()),
        "java.io.IOException: mark/reset not supported"
    );
    input.close().unwrap();
    assert_eq!(
        java_class_of(&input.read_byte().unwrap_err()),
        CLOSED_CHANNEL_EXCEPTION
    );
    assert_eq!(
        Files::read_attributes_by_name(&f, "basic:size,lastModifiedTime", &[])
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect::<Vec<_>>(),
        ["size", "lastModifiedTime"]
    );
}

/// `toUri()` and `getPath(URI)`, measured on OpenJDK 25.
#[test]
fn uri_round_trip_matches_the_jdk() {
    let (_tmp, d) = temp_dir();
    let p = d.resolve_str("äÄöÖüÜß 中国 a b%");
    let uri = p.to_uri();
    assert!(uri.starts_with("file:///"));
    assert!(uri
        .ends_with("/%C3%A4%C3%84%C3%B6%C3%96%C3%BC%C3%9C%C3%9F%20%E4%B8%AD%E5%9B%BD%20a%20b%25"));
    assert_eq!(default_file_system().provider().get_path(&uri), p);
    assert!(d.to_uri().ends_with('/'));
}

#[test]
fn glob_matcher_matches_names() {
    let fs = default_file_system();
    let m = fs.get_path_matcher("glob:f*");
    assert!(m.matches(&path("foo")));
    assert!(!m.matches(&path("bar")));
    assert!(!m.matches(&path("f/o")));
    let m = fs.get_path_matcher("glob:*.{java,class}");
    assert!(m.matches(&path("A.java")));
    assert!(m.matches(&path("A.class")));
    assert!(!m.matches(&path("A.txt")));
    let m = fs.get_path_matcher("glob:[a-c]?");
    assert!(m.matches(&path("bx")));
    assert!(!m.matches(&path("dx")));
}
