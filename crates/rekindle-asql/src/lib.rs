//! Asynchronous handle for rusqlite library.
//!
//! VENDORED VERBATIM from `tokio-rusqlite` 0.7.0 (MIT, © 2022 Eray Karatay;
//! see the crate `LICENSE`). The only change vs upstream is the dependency
//! pin (`rusqlite 0.37` → `0.39`) in `Cargo.toml` — upstream is unmaintained
//! and its `rusqlite ^0.37` pin conflicts with veilid-core 0.5.8's sqlite
//! chain (`keyvaluedb-sqlite` 0.2.6 → `async-sqlite` 0.5.7 → `rusqlite 0.39`). The source below keeps upstream's battle-tested
//! dedicated-thread + channel + close semantics; its two additions are
//! [`Connection::call_detached`] and the call watch (below), neither of which
//! changes how a call runs. One fix: a successful close disconnects the queue
//! before it resolves, so every later call sees the closed connection
//! (finding AQ1; upstream 0.7.0 resolved first). Another: dropping the last
//! handle waits for the thread to close the database (finding DB1; upstream
//! left it detached, racing SQLCipher's exit handler). Our
//! crates depend on it under the name `tokio_rusqlite` via a Cargo
//! `package`-rename, so no call-sites change.
//!
//! # Guide
//!
//! This library provides [`Connection`] struct. [`Connection`] struct is a handle
//! to call functions in background thread and can be cloned cheaply.
//! [`Connection::call`] method calls provided function in the background thread
//! and returns its result asynchronously.
//!
//! # Design
//!
//! A thread is spawned for each opened connection handle. When `call` method
//! is called: provided function is boxed, sent to the thread through mpsc
//! channel and executed. Return value is then sent by oneshot channel from
//! the thread and then returned from function.
//!
//! # Call watch
//!
//! The one addition to upstream's API: [`watch_calls`] installs a process-wide
//! [`CallWatch`] told of every call queued on a connection's thread (open,
//! call, detached call, close), and again once the call is queued. Production
//! installs none, so a call costs two unset `OnceLock` reads. The network
//! simulator installs one that blocks its runtime thread until the call has
//! run, so its tasks resume in one order whatever the database thread's
//! timing (plan E2.B S8, F11).
//!
//! # Example
//!
//! ```rust,no_run
//! // (consumers import this crate under the `tokio_rusqlite` alias via a
//! // Cargo package-rename; inside the crate itself it's `rekindle_asql`.)
//! use rekindle_asql::{params, Connection, Result};
//!
//! #[derive(Debug)]
//! struct Person {
//!     id: i32,
//!     name: String,
//!     data: Option<Vec<u8>>,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let conn = Connection::open_in_memory().await?;
//!
//!     let people = conn
//!         .call(|conn| {
//!             conn.execute(
//!                 "CREATE TABLE person (
//!                     id    INTEGER PRIMARY KEY,
//!                     name  TEXT NOT NULL,
//!                     data  BLOB
//!                 )",
//!                 [],
//!             )?;
//!
//!             let steven = Person {
//!                 id: 1,
//!                 name: "Steven".to_string(),
//!                 data: None,
//!             };
//!
//!             conn.execute(
//!                 "INSERT INTO person (name, data) VALUES (?1, ?2)",
//!                 params![steven.name, steven.data],
//!             )?;
//!
//!             let mut stmt = conn.prepare("SELECT id, name, data FROM person")?;
//!             let people = stmt
//!                 .query_map([], |row| {
//!                     Ok(Person {
//!                         id: row.get(0)?,
//!                         name: row.get(1)?,
//!                         data: row.get(2)?,
//!                     })
//!                 })?
//!                 .collect::<std::result::Result<Vec<Person>, rusqlite::Error>>()?;
//!
//!             Ok(people)
//!         })
//!         .await?;
//!
//!     for person in people {
//!         println!("Found person {:?}", person);
//!     }
//!
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]
// Upstream's header additionally had `clippy::cargo_common_metadata`. It's
// dropped here (only this one lint): it is a crates.io-PUBLISHING completeness
// check and is workspace-wide — enabling it demands repository/readme/keywords/
// categories/license on all ~35 of our (deliberately unpublished) workspace
// crates, which was never part of our lint standard. Every code-quality lint
// below is kept.
#![warn(
    clippy::await_holding_lock,
    clippy::dbg_macro,
    clippy::empty_enum,
    clippy::enum_glob_use,
    clippy::inefficient_to_string,
    clippy::mem_forget,
    clippy::mutex_integer,
    clippy::needless_continue,
    clippy::todo,
    clippy::unimplemented,
    clippy::wildcard_imports,
    future_incompatible,
    missing_docs,
    missing_debug_implementations,
    unreachable_pub
)]

#[cfg(test)]
mod tests;
mod worker;

use crossbeam_channel::{Receiver, Sender};
use std::{
    fmt::{self, Debug, Display},
    path::Path,
    sync::{Arc, OnceLock},
    thread,
};
use tokio::sync::oneshot::{self};
use worker::Worker;

pub use rusqlite::{self, *};

const BUG_TEXT: &str = "bug in tokio-rusqlite, please report";
const CLOSED_TEXT: &str = "database connection should be open";

/// Told of every call queued on a connection's background thread.
pub trait CallWatch: Send + Sync {
    /// A call is being queued. The token returned is dropped on the
    /// background thread once the call has run and its result (if any) is
    /// sent, or wherever the call is dropped unrun.
    fn begin(&self) -> CallToken;

    /// The call just begun is queued. Called on the queuing thread; a watch
    /// may block it here until the call's token drops, so the caller resumes
    /// with the result already sent. By default it returns at once.
    fn queued(&self) {}
}

/// What a [`CallWatch`] holds for one call; dropping it ends the call.
pub type CallToken = Option<Box<dyn Send>>;

static CALL_WATCH: OnceLock<&'static dyn CallWatch> = OnceLock::new();

/// Install the process-wide [`CallWatch`]. Returns `false` if one is
/// already installed (the first stays).
pub fn watch_calls(watch: &'static dyn CallWatch) -> bool {
    CALL_WATCH.set(watch).is_ok()
}

fn begin() -> CallToken {
    CALL_WATCH.get().and_then(|watch| watch.begin())
}

fn queued() {
    if let Some(watch) = CALL_WATCH.get() {
        watch.queued();
    }
}

#[derive(Debug)]
/// Represents the errors specific for this library.
#[allow(
    clippy::enum_variant_names,
    reason = "the `Error(E)` variant repeats the enum name, but it is part of the vendored `tokio_rusqlite` public API (consumers and the `From<rusqlite::Error>` impl below depend on it), so it can't be renamed — scoped to this item, not the crate"
)]
#[non_exhaustive]
pub enum Error<E = rusqlite::Error> {
    /// The connection to the SQLite has been closed and cannot be queried any more.
    ConnectionClosed,

    /// An error occured while closing the SQLite connection.
    /// This `Error` variant contains the [`Connection`], which can be used to retry the close operation
    /// and the underlying [`rusqlite::Error`] that made it impossile to close the database.
    Close((Connection, rusqlite::Error)),

    /// An application-specific error occured.
    Error(E),
}

impl<E: Display> Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ConnectionClosed => write!(f, "ConnectionClosed"),
            Error::Close((_, e)) => write!(f, "Close((Connection, \"{e}\"))"),
            Error::Error(e) => write!(f, "Error(\"{e}\")"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ConnectionClosed => None,
            Error::Close((_, e)) => Some(e),
            Error::Error(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Error::Error(value)
    }
}

/// The result returned on method calls in this crate.
pub type Result<T> = std::result::Result<T, Error>;

type CallFn = Box<dyn FnOnce(&mut rusqlite::Connection) + Send + 'static>;

enum Message {
    Execute(CallFn),
    Close(
        oneshot::Sender<std::result::Result<(), rusqlite::Error>>,
        CallToken,
    ),
}

/// A handle to call functions in background thread.
///
/// Dropping the last handle (finding DB1) waits for the background thread
/// to finish the calls queued before it and close the SQLite connection,
/// as dropping a `rusqlite::Connection` closes it before returning.
/// Upstream left the thread detached, so a process could exit while it
/// was still closing: SQLCipher's exit handler (`sqlcipher_extra_shutdown`,
/// run by `atexit` and the image's terminator) frees its mutexes, private
/// heap and crypto provider under a close still using them, and the
/// process died after its last test had passed.
#[derive(Clone)]
pub struct Connection {
    // Declared before `_worker` (held only for its drop), so the last handle disconnects the queue
    // before it waits for the thread the disconnect stops.
    sender: Sender<Message>,
    _worker: Arc<Worker>,
}

impl Connection {
    /// Open a new connection to a SQLite database.
    ///
    /// `Connection::open(path)` is equivalent to
    /// `Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE |
    /// OpenFlags::SQLITE_OPEN_CREATE)`.
    ///
    /// # Failure
    ///
    /// Will return `Err` if `path` cannot be converted to a C-compatible
    /// string or if the underlying SQLite open call fails.
    pub async fn open<P: AsRef<Path>>(path: P) -> std::result::Result<Self, rusqlite::Error> {
        let path = path.as_ref().to_owned();
        start(move || rusqlite::Connection::open(path)).await
    }

    /// Open a new connection to an in-memory SQLite database.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite open call fails.
    pub async fn open_in_memory() -> std::result::Result<Self, rusqlite::Error> {
        start(rusqlite::Connection::open_in_memory).await
    }

    /// Open a new connection to a SQLite database.
    ///
    /// [Database Connection](http://www.sqlite.org/c3ref/open.html) for a
    /// description of valid flag combinations.
    ///
    /// # Failure
    ///
    /// Will return `Err` if `path` cannot be converted to a C-compatible
    /// string or if the underlying SQLite open call fails.
    pub async fn open_with_flags<P: AsRef<Path>>(
        path: P,
        flags: OpenFlags,
    ) -> std::result::Result<Self, rusqlite::Error> {
        let path = path.as_ref().to_owned();
        start(move || rusqlite::Connection::open_with_flags(path, flags)).await
    }

    /// Open a new connection to a SQLite database using the specific flags
    /// and vfs name.
    ///
    /// [Database Connection](http://www.sqlite.org/c3ref/open.html) for a
    /// description of valid flag combinations.
    ///
    /// # Failure
    ///
    /// Will return `Err` if either `path` or `vfs` cannot be converted to a
    /// C-compatible string or if the underlying SQLite open call fails.
    pub async fn open_with_flags_and_vfs<P: AsRef<Path>>(
        path: P,
        flags: OpenFlags,
        vfs: &str,
    ) -> std::result::Result<Self, rusqlite::Error> {
        let path = path.as_ref().to_owned();
        let vfs = vfs.to_owned();
        start(move || rusqlite::Connection::open_with_flags_and_vfs(path, flags, &*vfs)).await
    }

    /// Open a new connection to an in-memory SQLite database.
    ///
    /// [Database Connection](http://www.sqlite.org/c3ref/open.html) for a
    /// description of valid flag combinations.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite open call fails.
    pub async fn open_in_memory_with_flags(
        flags: OpenFlags,
    ) -> std::result::Result<Self, rusqlite::Error> {
        start(move || rusqlite::Connection::open_in_memory_with_flags(flags)).await
    }

    /// Open a new connection to an in-memory SQLite database using the
    /// specific flags and vfs name.
    ///
    /// [Database Connection](http://www.sqlite.org/c3ref/open.html) for a
    /// description of valid flag combinations.
    ///
    /// # Failure
    ///
    /// Will return `Err` if `vfs` cannot be converted to a C-compatible
    /// string or if the underlying SQLite open call fails.
    pub async fn open_in_memory_with_flags_and_vfs(
        flags: OpenFlags,
        vfs: &str,
    ) -> std::result::Result<Self, rusqlite::Error> {
        let vfs = vfs.to_owned();
        start(move || rusqlite::Connection::open_in_memory_with_flags_and_vfs(flags, &*vfs)).await
    }

    /// Call a function in background thread and get the result
    /// asynchronously.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the database connection has been closed.
    /// Will return `Error::Error` wrapping the inner error if `function` failed.
    pub async fn call<F, R, E>(&self, function: F) -> std::result::Result<R, Error<E>>
    where
        F: FnOnce(&mut rusqlite::Connection) -> std::result::Result<R, E> + 'static + Send,
        R: Send + 'static,
        E: Send + 'static,
    {
        self.call_raw(function)
            .await
            .map_err(|_| Error::ConnectionClosed)
            .and_then(|result| result.map_err(Error::Error))
    }

    /// Call a function in background thread and get the result
    /// asynchronously.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the database connection has been closed.
    pub async fn call_raw<F, R>(&self, function: F) -> Result<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> R + 'static + Send,
        R: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel::<R>();
        let token = begin();

        self.sender
            .send(Message::Execute(Box::new(move |conn| {
                let value = function(conn);
                let _ = sender.send(value);
                drop(token);
            })))
            .map_err(|_| Error::ConnectionClosed)?;
        queued();

        receiver.await.map_err(|_| Error::ConnectionClosed)
    }

    /// Queue a function on the background thread without waiting for it.
    /// Calls run in the order they were queued, so a later [`Connection::call`]
    /// sees this one's effects. No task is spawned: the caller needs no
    /// runtime and nothing outlives the connection.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the database connection has been closed.
    pub fn call_detached<F>(&self, function: F) -> Result<()>
    where
        F: FnOnce(&mut rusqlite::Connection) + Send + 'static,
    {
        let token = begin();
        self.sender
            .send(Message::Execute(Box::new(move |conn| {
                function(conn);
                drop(token);
            })))
            .map_err(|_| Error::ConnectionClosed)?;
        queued();
        Ok(())
    }

    /// Call a function in background thread and get the result
    /// asynchronously.
    ///
    /// This method can cause a `panic` if the underlying database connection is closed.
    /// it is a more user-friendly alternative to the [`Connection::call`] method.
    /// It should be safe if the connection is never explicitly closed (using the [`Connection::close`] call).
    ///
    /// Calling this on a closed connection, or queuing it behind a `close`
    /// that succeeds, will cause a `panic`.
    pub async fn call_unwrap<F, R>(&self, function: F) -> R
    where
        F: FnOnce(&mut rusqlite::Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel::<R>();
        let token = begin();

        self.sender
            .send(Message::Execute(Box::new(move |conn| {
                let value = function(conn);
                let _ = sender.send(value);
                drop(token);
            })))
            .expect(CLOSED_TEXT);
        queued();

        // The reply sender is dropped unsent only when the call is discarded
        // unrun: it was queued behind a `close` that succeeded.
        receiver.await.expect(CLOSED_TEXT)
    }

    /// Close the database connection.
    ///
    /// This is functionally equivalent to the `Drop` implementation for
    /// `Connection`. It consumes the `Connection`, but on error returns it
    /// to the caller for retry purposes.
    ///
    /// If successful, any following `close` operations performed
    /// on `Connection` copies will succeed immediately.
    ///
    /// On the other hand, any calls to [`Connection::call`] (or
    /// [`Connection::call_detached`]) will return a [`Error::ConnectionClosed`],
    /// and any calls to [`Connection::call_unwrap`] will cause a `panic`. This
    /// holds from the moment `close` resolves, and for every call queued
    /// behind the close while it ran.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite close call fails.
    pub async fn close(self) -> Result<()> {
        let (sender, receiver) = oneshot::channel::<std::result::Result<(), rusqlite::Error>>();

        if let Err(crossbeam_channel::SendError(_)) =
            self.sender.send(Message::Close(sender, begin()))
        {
            // If the channel is closed on the other side, it means the connection closed successfully
            // This is a safeguard against calling close on a `Copy` of the connection
            return Ok(());
        }
        queued();

        let result = receiver.await;

        if result.is_err() {
            // If we get a RecvError at this point, it also means the channel closed in the meantime
            // we can assume the connection is closed
            return Ok(());
        }

        result.unwrap().map_err(|e| Error::Close((self, e)))
    }
}

impl Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection").finish()
    }
}

impl From<rusqlite::Connection> for Connection {
    fn from(conn: rusqlite::Connection) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded::<Message>();
        let worker = Worker::new(thread::spawn(move || event_loop(conn, receiver)));

        Self {
            sender,
            _worker: worker,
        }
    }
}

async fn start<F>(open: F) -> rusqlite::Result<Connection>
where
    F: FnOnce() -> rusqlite::Result<rusqlite::Connection> + Send + 'static,
{
    let (sender, receiver) = crossbeam_channel::unbounded::<Message>();
    let (result_sender, result_receiver) = oneshot::channel();
    let token = begin();

    let thread = thread::spawn(move || {
        let conn = match open() {
            Ok(c) => c,
            Err(e) => {
                let _ = result_sender.send(Err(e));
                drop(token);
                return;
            }
        };

        let sent = result_sender.send(Ok(()));
        drop(token);
        if let Err(_e) = sent {
            return;
        }

        event_loop(conn, receiver);
    });
    let worker = Worker::new(thread);
    queued();

    result_receiver.await.expect(BUG_TEXT).map(|()| Connection {
        sender,
        _worker: worker,
    })
}

// The receiver is owned for the whole thread lifetime, not borrowed:
// `event_loop` runs on its own thread and outlives every caller frame.
#[allow(
    clippy::needless_pass_by_value,
    reason = "owned for the spawned thread's lifetime; a borrow would tie it to the caller's frame"
)]
fn event_loop(mut conn: rusqlite::Connection, receiver: Receiver<Message>) {
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Execute(f) => f(&mut conn),
            Message::Close(s, token) => {
                let result = conn.close();

                match result {
                    Ok(v) => {
                        // Disconnect the queue BEFORE the closer learns of
                        // the close (finding AQ1). Upstream sent first and
                        // dropped the receiver only on return, so a call
                        // made right after `close` resolved could still be
                        // queued, then discarded unrun: `call_detached`
                        // returned `Ok`, `call_unwrap` panicked with
                        // `BUG_TEXT`. Dropping it here also discards every
                        // call queued behind the close, each of which then
                        // sees its reply sender dropped.
                        drop(receiver);
                        s.send(Ok(v)).expect(BUG_TEXT);
                        drop(token);
                        return;
                    }
                    Err((c, e)) => {
                        conn = c;
                        s.send(Err(e)).expect(BUG_TEXT);
                        drop(token);
                    }
                }
            }
        }
    }
}
