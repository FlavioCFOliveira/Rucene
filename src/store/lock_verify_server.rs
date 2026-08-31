//! Standalone server that verifies at most one process holds a lock at a time.
//!
//! Ported from `org.apache.lucene.store.LockVerifyServer`.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::verifying_lock_factory::{MSG_LOCK_ACQUIRED, MSG_LOCK_RELEASED};
use crate::error::{LuceneError, Result};

/// Byte the server writes to every client once all of them have connected, to
/// release them simultaneously.
///
/// Equivalent to `LockVerifyServer.START_GUN_SIGNAL`.
pub const START_GUN_SIGNAL: u8 = 43;

/// How long clients are given to connect, matching the 30 second
/// `ServerSocket.setSoTimeout` Lucene sets.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the accept loop sleeps between polls while waiting for a client.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Sentinel stored in the shared lock holder slot when no client holds the
/// lock. Matches Java's `lockedID[0] = -1`.
const NO_HOLDER: i32 = -1;

/// Sentinel stored in the shared lock holder slot once a violation has been
/// detected, so that the remaining client threads exit. Matches Java's
/// `lockedID[0] = -2`.
const VIOLATION: i32 = -2;

/// A simple standalone server that must be running when
/// [`VerifyingLockFactory`](super::VerifyingLockFactory) is used.
///
/// Equivalent to `org.apache.lucene.store.LockVerifyServer`. The server
/// verifies that at most one process holds the lock at a time: each client
/// announces every acquire and release, and the server rejects an acquire while
/// another client holds the lock, or a release by a client that does not.
///
/// See also [`LockStressTest`](super::LockStressTest), the client side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LockVerifyServer;

/// The shared state guarded by the server's monitor: which client currently
/// holds the lock, or one of the [`NO_HOLDER`] / [`VIOLATION`] sentinels.
struct LockedId(Mutex<i32>);

/// The starting gun: a one-shot condition every client thread waits on.
///
/// This is the [`std::sync::Condvar`] equivalent of Java's
/// `CountDownLatch(1)`.
struct StartingGun {
    fired: Mutex<bool>,
    condition: Condvar,
}

impl StartingGun {
    fn new() -> Self {
        Self {
            fired: Mutex::new(false),
            condition: Condvar::new(),
        }
    }

    /// Blocks until [`fire`](Self::fire) is called.
    fn wait(&self) -> Result<()> {
        let mut fired = self
            .fired
            .lock()
            .map_err(|_| LuceneError::IllegalState("starting gun mutex poisoned".to_string()))?;
        while !*fired {
            fired = self.condition.wait(fired).map_err(|_| {
                LuceneError::IllegalState("starting gun mutex poisoned".to_string())
            })?;
        }
        Ok(())
    }

    /// Releases every waiting client thread.
    fn fire(&self) -> Result<()> {
        let mut fired = self
            .fired
            .lock()
            .map_err(|_| LuceneError::IllegalState("starting gun mutex poisoned".to_string()))?;
        *fired = true;
        self.condition.notify_all();
        Ok(())
    }
}

impl LockVerifyServer {
    /// Byte the server writes to every client once all of them have connected.
    ///
    /// Equivalent to `LockVerifyServer.START_GUN_SIGNAL`.
    pub const START_GUN_SIGNAL: u8 = START_GUN_SIGNAL;

    /// Binds an ephemeral port on `hostname`, waits for `max_clients` clients,
    /// releases them all at once, and then verifies their lock traffic until
    /// every client disconnects.
    ///
    /// `start_clients` is invoked with the bound address once the listener is
    /// ready; Lucene uses that callback so tests can launch the client
    /// processes against the port the operating system chose.
    ///
    /// Equivalent to
    /// `LockVerifyServer.run(String, int, Consumer<InetSocketAddress>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the listener cannot be bound or a client
    /// does not connect within 30 seconds, and [`LuceneError::IllegalState`]
    /// when a client acquires a lock another client already holds, or releases
    /// one it does not hold.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java lets a violation escape as an uncaught exception on the client
    /// thread — `run` itself completes normally and prints `Server terminated.`
    /// — which is only observable on the console. Here the violation is
    /// returned to the caller, because a library function whose whole purpose
    /// is to detect a violation has to be able to report it. Java's progress
    /// messages go to `System.out`; this port emits them through the `log`
    /// crate. Java bounds `accept` with `ServerSocket.setSoTimeout`, which the
    /// Rust standard library does not expose, so the listener is polled in
    /// non-blocking mode against the same 30 second deadline.
    pub fn run<F>(hostname: &str, max_clients: usize, start_clients: F) -> Result<()>
    where
        F: FnOnce(SocketAddr),
    {
        let listener = TcpListener::bind((hostname, 0))?;
        let local_addr = listener.local_addr()?;
        log::info!("Listening on {local_addr}...");

        // Callback only for the test to start the clients:
        start_clients(local_addr);

        let locked_id = Arc::new(LockedId(Mutex::new(NO_HOLDER)));
        let starting_gun = Arc::new(StartingGun::new());
        let mut threads = Vec::with_capacity(max_clients);

        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + ACCEPT_TIMEOUT;
        for _ in 0..max_clients {
            let client = Self::accept_before(&listener, deadline)?;
            client.set_nonblocking(false)?;
            let locked_id = Arc::clone(&locked_id);
            let starting_gun = Arc::clone(&starting_gun);
            threads.push(thread::spawn(move || {
                Self::serve_client(client, &locked_id, &starting_gun)
            }));
        }

        // Start.
        log::info!("All clients started, fire gun...");
        starting_gun.fire()?;

        // Wait for all threads to finish.
        let mut first_failure: Option<LuceneError> = None;
        for thread in threads {
            match thread.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if first_failure.is_none() {
                        first_failure = Some(error);
                    }
                }
                Err(_) => {
                    if first_failure.is_none() {
                        first_failure = Some(LuceneError::IllegalState(
                            "lock verify client thread panicked".to_string(),
                        ));
                    }
                }
            }
        }

        log::info!("Server terminated.");
        match first_failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Command-line entry point.
    ///
    /// `args` are the two arguments Java's `main` expects: the address to bind
    /// to and the number of clients. Returns the process exit code, which is
    /// `1` when the arguments are wrong and `0` otherwise, instead of calling
    /// `System.exit` the way Java does.
    ///
    /// # Errors
    ///
    /// Propagates any failure from [`run`](Self::run).
    pub fn main(args: &[String]) -> Result<i32> {
        if args.len() != 2 {
            log::info!("Usage: LockVerifyServer bindToIp clients");
            return Ok(1);
        }
        let clients: usize = args[1].parse().map_err(|_| {
            LuceneError::IllegalArgument(format!("clients is not a number: {}", args[1]))
        })?;
        Self::run(&args[0], clients, |_| {})?;
        Ok(0)
    }

    /// Accepts one connection, giving up once `deadline` has passed.
    fn accept_before(listener: &TcpListener, deadline: Instant) -> Result<TcpStream> {
        loop {
            match listener.accept() {
                Ok((client, _)) => return Ok(client),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(LuceneError::Io(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Timed out waiting for clients to connect.",
                        )));
                    }
                    thread::sleep(ACCEPT_POLL_INTERVAL);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Serves one client for its whole lifetime: reads its id, waits for the
    /// starting gun, and then validates every lock command it sends.
    fn serve_client(
        client: TcpStream,
        locked_id: &LockedId,
        starting_gun: &StartingGun,
    ) -> Result<()> {
        let mut input = client.try_clone()?;
        let mut output = client;

        let mut byte = [0u8; 1];
        if input.read(&mut byte)? == 0 {
            return Err(LuceneError::Io(std::io::Error::other(
                "Client closed connection before communication started.",
            )));
        }
        let id = i32::from(byte[0]);

        starting_gun.wait()?;
        output.write_all(&[START_GUN_SIGNAL])?;
        output.flush()?;

        loop {
            if input.read(&mut byte)? == 0 {
                return Ok(()); // closed
            }
            let command = byte[0];

            let mut holder = locked_id
                .0
                .lock()
                .map_err(|_| LuceneError::IllegalState("locked id mutex poisoned".to_string()))?;
            let current_lock = *holder;
            if current_lock == VIOLATION {
                return Ok(()); // another thread got an error, so we exit too
            }
            match command {
                MSG_LOCK_ACQUIRED => {
                    if current_lock != NO_HOLDER {
                        *holder = VIOLATION;
                        return Err(LuceneError::IllegalState(format!(
                            "id {id} got lock, but {current_lock} already holds the lock"
                        )));
                    }
                    *holder = id;
                }
                MSG_LOCK_RELEASED => {
                    if current_lock != id {
                        *holder = VIOLATION;
                        return Err(LuceneError::IllegalState(format!(
                            "id {id} released the lock, but {current_lock} is the one holding the lock"
                        )));
                    }
                    *holder = NO_HOLDER;
                }
                other => {
                    return Err(LuceneError::Other(format!("Unrecognized command: {other}")));
                }
            }
            output.write_all(&[command])?;
            output.flush()?;
        }
    }
}
