//! Standalone tool that repeatedly acquires and releases a lock, checking every
//! step against a [`LockVerifyServer`](super::LockVerifyServer).
//!
//! Ported from `org.apache.lucene.store.LockStressTest`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::exceptions::LockObtainFailedException;
use super::lock_verify_server::START_GUN_SIGNAL;
use super::verifying_lock_factory::VerifyingLockFactory;
use super::{LockFactory, NIOFSDirectory, NativeFSLockFactory, NoLockFactory, SimpleFSLockFactory};
use crate::error::{LuceneError, Result};

/// Name of the lock file the stress test contends on.
///
/// Equivalent to `LockStressTest.LOCK_FILE_NAME`.
pub const LOCK_FILE_NAME: &str = "test.lock";

/// How long the client waits to connect to the verification server.
///
/// Equivalent to the `socket.connect(addr, 3000)` timeout in Lucene.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// A `SplitMix64` generator standing in for Java's `java.util.Random`.
///
/// Only two decisions in the stress test are random — whether to attempt a
/// double obtain (one chance in ten) and whether to rebuild the factory first
/// (a coin flip) — so any uniform source serves. `java.util.Random` cannot be
/// reproduced without porting its exact linear congruential generator, and the
/// test's meaning does not depend on the sequence, only on the two
/// probabilities.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Seeds the generator from the wall clock, as `new Random()` does.
    fn from_clock() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self {
            state: seed | 1, // never seed with zero
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a uniformly distributed value in `[0, bound)`.
    ///
    /// Equivalent to `java.util.Random.nextInt(int)`. Rejection sampling keeps
    /// the distribution exactly uniform, as Java's own implementation does.
    fn next_int(&mut self, bound: u32) -> u32 {
        debug_assert!(bound > 0, "bound must be positive");
        let bound = u64::from(bound);
        let limit = (u64::MAX / bound) * bound;
        loop {
            let value = self.next_u64();
            if value < limit {
                return (value % bound) as u32;
            }
        }
    }

    /// Equivalent to `java.util.Random.nextBoolean()`.
    fn next_bool(&mut self) -> bool {
        self.next_u64() >> 63 == 1
    }
}

/// A simple standalone tool that forever acquires and releases a lock using a
/// specific [`LockFactory`].
///
/// Equivalent to `org.apache.lucene.store.LockStressTest`. Run several
/// instances, each with its own unique id and each pointing at the same lock
/// directory, to verify that locking works correctly. A
/// [`LockVerifyServer`](super::LockVerifyServer) must already be running.
///
/// See also [`VerifyingLockFactory`], which reports each acquire and release to
/// that server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LockStressTest;

impl LockStressTest {
    /// Name of the lock file the stress test contends on.
    ///
    /// Equivalent to `LockStressTest.LOCK_FILE_NAME`.
    pub const LOCK_FILE_NAME: &'static str = LOCK_FILE_NAME;

    /// Command-line entry point.
    ///
    /// `args` are the seven arguments Java's `main` expects: `myID`,
    /// `verifierHost`, `verifierPort`, `lockFactoryClassName`, `lockDirName`,
    /// `sleepTimeMS` and `count`. Returns the process exit code instead of
    /// calling `System.exit`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if a numeric argument cannot be
    /// parsed, and propagates any failure from [`run`](Self::run).
    pub fn main(args: &[String]) -> Result<i32> {
        if args.len() != 7 {
            log::info!(
                "Usage: LockStressTest myID verifierHost verifierPort lockFactoryClassName lockDirName sleepTimeMS count\n\
                 \n\
                 \x20 myID = int from 0 .. 255 (should be unique for test process)\n\
                 \x20 verifierHost = hostname that LockVerifyServer is listening on\n\
                 \x20 verifierPort = port that LockVerifyServer is listening on\n\
                 \x20 lockFactoryClassName = primary FSLockFactory class that we will use\n\
                 \x20 lockDirName = path to the lock directory\n\
                 \x20 sleepTimeMS = milliseconds to pause betweeen each lock obtain/release\n\
                 \x20 count = number of locking tries\n\
                 \n\
                 You should run multiple instances of this process, each with its own\n\
                 unique ID, and each pointing to the same lock directory, to verify\n\
                 that locking is working correctly.\n\
                 \n\
                 Make sure you are first running LockVerifyServer."
            );
            return Ok(1);
        }

        let my_id = Self::parse(&args[0], "myID")?;
        let verifier_host = &args[1];
        let verifier_port: u16 = args[2].parse().map_err(|_| {
            LuceneError::IllegalArgument(format!("verifierPort is not a port: {}", args[2]))
        })?;
        let lock_factory_class_name = &args[3];
        let lock_dir_path = Path::new(&args[4]);
        let sleep_time_ms = Self::parse(&args[5], "sleepTimeMS")?;
        let count = Self::parse(&args[6], "count")?;

        Self::run(
            my_id,
            verifier_host,
            verifier_port,
            lock_factory_class_name,
            lock_dir_path,
            sleep_time_ms,
            count,
        )
    }

    fn parse(value: &str, name: &str) -> Result<i64> {
        value
            .parse()
            .map_err(|_| LuceneError::IllegalArgument(format!("{name} is not a number: {value}")))
    }

    /// Runs the stress loop: connects to the verification server, waits for the
    /// starting gun, and then obtains and releases the lock `count` times.
    ///
    /// Returns the process exit code — `1` when `my_id` is outside `0..=255`,
    /// `0` on success.
    ///
    /// Equivalent to the private
    /// `LockStressTest.run(int, String, int, String, Path, int, int)`, made
    /// public here so the tool can be driven from library code and from tests
    /// without spawning a process.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the connection cannot be made, if the
    /// server does not fire the starting gun, or if a lock is obtained twice
    /// (`Double obtain`), and propagates any failure from the lock factory.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        my_id: i64,
        verifier_host: &str,
        verifier_port: u16,
        lock_factory_class_name: &str,
        lock_dir_path: &Path,
        sleep_time_ms: i64,
        count: i64,
    ) -> Result<i32> {
        if !(0..=255).contains(&my_id) {
            log::info!("myID must be a unique int 0..255");
            return Ok(1);
        }
        let sleep_time = Duration::from_millis(sleep_time_ms.max(0) as u64);

        let lock_factory = Self::get_new_lock_factory(lock_factory_class_name)?;
        // We test the lock factory directly, so we don't need it on the
        // directory itself (the directory is just for testing).
        let lock_dir =
            NIOFSDirectory::with_lock_factory(lock_dir_path, Some(Box::new(NoLockFactory)))?;

        let addr = (verifier_host, verifier_port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "cannot resolve verifier address {verifier_host}:{verifier_port}"
                ))
            })?;
        log::info!("Connecting to server {addr} and registering as client {my_id}...");

        // Wait at most 3 seconds to successfully connect, else fail.
        let socket = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        let mut control_input = socket.try_clone()?;
        let mut control_output = socket.try_clone()?;

        control_output.write_all(&[my_id as u8])?;
        control_output.flush()?;
        let mut verify_lf = VerifyingLockFactory::new(
            lock_factory,
            Box::new(socket.try_clone()?),
            Box::new(socket.try_clone()?),
        );
        let mut rnd = SplitMix64::from_clock();

        // Wait for the starting gun.
        let mut byte = [0u8; 1];
        if control_input.read(&mut byte)? == 0 || byte[0] != START_GUN_SIGNAL {
            return Err(LuceneError::Io(std::io::Error::other("Protocol violation")));
        }

        for i in 0..count {
            match verify_lf.obtain_lock(&lock_dir, LOCK_FILE_NAME) {
                Ok(mut held) => {
                    let outcome = Self::hold_lock(
                        &mut verify_lf,
                        &lock_dir,
                        lock_factory_class_name,
                        &socket,
                        &mut rnd,
                        sleep_time,
                    );
                    // Java closes the lock through try-with-resources, so a
                    // failure in the body wins over a failure in `close`.
                    let closed = held.close();
                    let combined = outcome.and(closed);
                    if let Err(error) = combined {
                        if !LockObtainFailedException::is(&error) {
                            return Err(error);
                        }
                        // Obtain failed; Java's outer `catch` swallows it.
                    }
                }
                Err(error) if LockObtainFailedException::is(&error) => {
                    // Obtain failed.
                }
                Err(error) => return Err(error),
            }

            if i % 500 == 0 {
                log::info!("{}% done.", (i as f64) * 100.0 / (count as f64));
            }

            thread::sleep(sleep_time);
        }

        log::info!("Finished {count} tries.");
        Ok(0)
    }

    /// The body Java runs while holding the lock: occasionally proves that a
    /// second obtain fails, then sleeps.
    fn hold_lock(
        verify_lf: &mut VerifyingLockFactory,
        lock_dir: &NIOFSDirectory,
        lock_factory_class_name: &str,
        socket: &TcpStream,
        rnd: &mut SplitMix64,
        sleep_time: Duration,
    ) -> Result<()> {
        if rnd.next_int(10) == 0 {
            if rnd.next_bool() {
                *verify_lf = VerifyingLockFactory::new(
                    Self::get_new_lock_factory(lock_factory_class_name)?,
                    Box::new(socket.try_clone()?),
                    Box::new(socket.try_clone()?),
                );
            }
            match verify_lf.obtain_lock(lock_dir, LOCK_FILE_NAME) {
                Ok(mut second_lock) => {
                    // Java: `try (Lock secondLock = …) { throw new
                    // IOException("Double obtain"); }` — the resource is closed
                    // and the exception from the body propagates.
                    let _ = second_lock.close();
                    return Err(LuceneError::Io(std::io::Error::other("Double obtain")));
                }
                Err(error) if LockObtainFailedException::is(&error) => {
                    // Pass: obtaining the lock twice must fail.
                }
                Err(error) => return Err(error),
            }
        }
        thread::sleep(sleep_time);
        Ok(())
    }

    /// Resolves the lock factory named by `lock_factory_class_name`.
    ///
    /// Equivalent to `LockStressTest.getNewLockFactory(String)`.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java resolves the name reflectively: it looks for a static `INSTANCE`
    /// field, then for a no-argument constructor, on any class named on the
    /// command line. Rust has no reflection, so this port resolves the name
    /// against the filesystem lock factories the crate provides —
    /// [`NativeFSLockFactory`] and [`SimpleFSLockFactory`] — accepting both the
    /// simple and the fully qualified Java class name. The returned value is
    /// typed as [`LockFactory`] rather than as `FSLockFactory` because Rust
    /// gained trait-object upcasting only in 1.86, after this crate's minimum
    /// supported version; the factories returned are filesystem factories all
    /// the same.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] for an unknown name, exactly as Java throws
    /// `IOException("Cannot get lock factory singleton of …")`.
    pub fn get_new_lock_factory(lock_factory_class_name: &str) -> Result<Box<dyn LockFactory>> {
        let simple_name = lock_factory_class_name
            .rsplit('.')
            .next()
            .unwrap_or(lock_factory_class_name);
        match simple_name {
            "NativeFSLockFactory" => Ok(Box::new(NativeFSLockFactory)),
            "SimpleFSLockFactory" => Ok(Box::new(SimpleFSLockFactory)),
            _ => Err(LuceneError::Io(std::io::Error::other(format!(
                "Cannot get lock factory singleton of {lock_factory_class_name}"
            )))),
        }
    }
}
