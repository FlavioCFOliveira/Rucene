//! A [`LockFactory`] that verifies lock correctness against an external server.
//!
//! Ported from `org.apache.lucene.store.VerifyingLockFactory`.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use super::{Directory, Lock, LockFactory};
use crate::error::{LuceneError, Result};

/// Protocol byte sent to [`LockVerifyServer`](super::LockVerifyServer) when a
/// lock is released.
///
/// Equivalent to `VerifyingLockFactory.MSG_LOCK_RELEASED`.
pub const MSG_LOCK_RELEASED: u8 = 0;

/// Protocol byte sent to [`LockVerifyServer`](super::LockVerifyServer) when a
/// lock is acquired.
///
/// Equivalent to `VerifyingLockFactory.MSG_LOCK_ACQUIRED`.
pub const MSG_LOCK_ACQUIRED: u8 = 1;

/// The socket streams shared by a [`VerifyingLockFactory`] and every lock it
/// hands out.
///
/// Java passes the socket's `InputStream` and `OutputStream` around as bare
/// references, shared by the factory and its `CheckedLock` inner class. Rust
/// requires an owner, so the two halves live here and are shared behind an
/// [`Arc<Mutex<…>>`]. The mutex also makes [`VerifyingLockFactory`] `Sync`,
/// which [`LockFactory`] requires; Java's class is used single-threaded per
/// process by [`LockStressTest`](super::LockStressTest), so serialising the
/// exchange changes no observable behaviour.
struct VerifyChannel {
    input: Box<dyn Read + Send>,
    output: Box<dyn Write + Send>,
}

impl VerifyChannel {
    /// Sends `message` to the verification server and requires it to echo the
    /// same byte back.
    ///
    /// Equivalent to `VerifyingLockFactory.CheckedLock.verify(byte)`.
    fn verify(&mut self, message: u8) -> Result<()> {
        self.output.write_all(&[message])?;
        self.output.flush()?;
        let mut byte = [0u8; 1];
        let read = self.input.read(&mut byte)?;
        if read == 0 {
            // Java detects this as `in.read() < 0`, i.e. end of stream.
            return Err(LuceneError::IllegalState(
                "Lock server died because of locking error.".to_string(),
            ));
        }
        if byte[0] != message {
            return Err(LuceneError::Io(std::io::Error::other(
                "Protocol violation.",
            )));
        }
        Ok(())
    }
}

/// A [`Lock`] that reports every acquire and release to the verification
/// server.
///
/// Equivalent to the private inner class `VerifyingLockFactory.CheckedLock`.
struct CheckedLock {
    /// `None` once the lock has been closed, so that closing twice is a no-op
    /// the way Java's try-with-resources idiom makes it.
    lock: Option<Box<dyn Lock>>,
    channel: Arc<Mutex<VerifyChannel>>,
}

impl CheckedLock {
    /// Wraps `lock` and announces the acquisition to the server.
    ///
    /// # Errors
    ///
    /// Propagates any failure of the exchange with the server. As in Java, the
    /// underlying lock stays held when the announcement fails: the constructor
    /// throws before the wrapper takes responsibility for releasing it.
    fn new(lock: Box<dyn Lock>, channel: Arc<Mutex<VerifyChannel>>) -> Result<Self> {
        channel
            .lock()
            .map_err(|_| LuceneError::IllegalState("verify channel mutex poisoned".to_string()))?
            .verify(MSG_LOCK_ACQUIRED)?;
        Ok(Self {
            lock: Some(lock),
            channel,
        })
    }
}

impl Lock for CheckedLock {
    fn ensure_valid(&self) -> Result<()> {
        match &self.lock {
            Some(lock) => lock.ensure_valid(),
            None => Err(LuceneError::AlreadyClosed(
                "Lock instance already released".to_string(),
            )),
        }
    }

    fn close(&mut self) -> Result<()> {
        let Some(mut lock) = self.lock.take() else {
            return Ok(());
        };
        // Java: `try (Lock l = lock) { l.ensureValid(); verify(MSG_LOCK_RELEASED); }`
        // — the body runs first, then the resource is closed, and a failure in
        // the body wins over a failure in `close`.
        let body = (|| {
            lock.ensure_valid()?;
            self.channel
                .lock()
                .map_err(|_| {
                    LuceneError::IllegalState("verify channel mutex poisoned".to_string())
                })?
                .verify(MSG_LOCK_RELEASED)
        })();
        let closed = lock.close();
        body.and(closed)
    }
}

/// A [`LockFactory`] that wraps another and verifies that every lock
/// obtain/release is correct — that two processes never hold the lock at the
/// same time.
///
/// Equivalent to `org.apache.lucene.store.VerifyingLockFactory`. It does this
/// by talking to an external [`LockVerifyServer`](super::LockVerifyServer),
/// which asserts that at most one process holds the lock at any moment. Run
/// that server on the host and port whose socket streams are handed to
/// [`VerifyingLockFactory::new`].
///
/// See also [`LockStressTest`](super::LockStressTest), the client that drives
/// this factory in a loop.
pub struct VerifyingLockFactory {
    lf: Box<dyn LockFactory>,
    channel: Arc<Mutex<VerifyChannel>>,
}

impl VerifyingLockFactory {
    /// Creates a verifying factory over `lf`, talking to the server through
    /// `input` and `output`.
    ///
    /// Equivalent to
    /// `VerifyingLockFactory(LockFactory, InputStream, OutputStream)`. For a
    /// TCP connection, pass two independent handles obtained from
    /// [`std::net::TcpStream::try_clone`].
    pub fn new(
        lf: Box<dyn LockFactory>,
        input: Box<dyn Read + Send>,
        output: Box<dyn Write + Send>,
    ) -> Self {
        Self {
            lf,
            channel: Arc::new(Mutex::new(VerifyChannel { input, output })),
        }
    }

    /// Returns the factory being verified.
    pub fn get_delegate(&self) -> &dyn LockFactory {
        self.lf.as_ref()
    }
}

impl LockFactory for VerifyingLockFactory {
    fn obtain_lock(&self, dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>> {
        let lock = self.lf.obtain_lock(dir, lock_name)?;
        Ok(Box::new(CheckedLock::new(lock, Arc::clone(&self.channel))?))
    }

    fn directory_type_name(&self) -> &'static str {
        "VerifyingLockFactory"
    }
}

impl std::fmt::Debug for VerifyingLockFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifyingLockFactory")
            .field("lf", &self.lf.directory_type_name())
            .finish()
    }
}
