//! Fixed workers, bounded admission and cooperative, bounded shutdown.
use super::{
    Connection,
    http::{self, Request, RequestPolicy},
    transport::{Address, Listener, Peer, SocketRef, Stream, WakeAddress},
};
use std::{
    collections::VecDeque,
    fmt,
    io::{self, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct CancelState {
    stopped: AtomicBool,
    mutex: Mutex<()>,
    wake: Condvar,
}

/// Shared cancellation with an optional request deadline. The host and upstream
/// transport must honor it; Rust cannot safely kill an arbitrary user handler.
#[derive(Clone)]
pub struct Cancellation {
    state: Arc<CancelState>,
    deadline: Option<Instant>,
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancellation {
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancelState {
                stopped: AtomicBool::new(false),
                mutex: Mutex::new(()),
                wake: Condvar::new(),
            }),
            deadline: None,
        }
    }

    pub fn with_timeout(&self, duration: Duration) -> Self {
        let candidate = Instant::now().checked_add(duration);
        Self {
            state: self.state.clone(),
            deadline: match (self.deadline, candidate) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (left, right) => left.or(right),
            },
        }
    }

    pub fn cancel(&self) {
        let _guard = self
            .state
            .mutex
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.state.stopped.store(true, Ordering::Release);
        self.state.wake.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.stopped.load(Ordering::Acquire)
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    /// Wait without polling. Returns true if cancellation/deadline is observed.
    pub fn wait_timeout(&self, duration: Duration) -> bool {
        let guard = self
            .state
            .mutex
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.is_cancelled() {
            return true;
        }
        let duration = self.remaining().map_or(duration, |left| left.min(duration));
        let _ = self
            .state
            .wake
            .wait_timeout_while(guard, duration, |_| !self.is_cancelled());
        self.is_cancelled()
    }

    pub(crate) fn clamp_timeout(&self, requested: Option<Duration>) -> Option<Duration> {
        match (requested, self.remaining()) {
            (Some(left), Some(right)) => Some(left.min(right).max(Duration::from_millis(1))),
            (None, Some(right)) => Some(right.max(Duration::from_millis(1))),
            (left, None) => left,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub workers: usize,
    pub queue: usize,
    pub max_body_bytes: usize,
    pub max_response_bytes: usize,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
    pub request_timeout: Duration,
    pub request_policy: RequestPolicy,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            workers: 4,
            queue: 16,
            max_body_bytes: 4 * 1024 * 1024,
            max_response_bytes: 16 * 1024 * 1024,
            read_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(300),
            request_policy: RequestPolicy::default(),
        }
    }
}

impl ServerConfig {
    fn validate(&self) -> io::Result<()> {
        if !(1..=256).contains(&self.workers)
            || !(1..=65536).contains(&self.queue)
            || self.max_body_bytes == 0
            || self.max_response_bytes == 0
            || self.read_timeout.is_zero()
            || self.write_timeout.is_zero()
            || self.request_timeout.is_zero()
            || Instant::now().checked_add(self.request_timeout).is_none()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid server bounds",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct HandlerError {
    pub status: u16,
    pub message: String,
}

impl HandlerError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl fmt::Display for HandlerError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(&self.message)
    }
}
impl std::error::Error for HandlerError {}
impl From<io::Error> for HandlerError {
    fn from(error: io::Error) -> Self {
        Self::new(502, error.to_string())
    }
}

/// A path/protocol extension point. The server installs no management routes.
/// The handler owns application authentication and policy; Gateway adds the
/// registered AI adapters and caller-key authentication as a separate handler.
pub trait Handler: Send + Sync + 'static {
    fn handle(
        &self,
        request: Request,
        peer: &Peer,
        response: &mut Response<'_>,
        cancellation: &Cancellation,
    ) -> Result<(), HandlerError>;
}

impl<F> Handler for F
where
    F: for<'a> Fn(Request, &Peer, &mut Response<'a>, &Cancellation) -> Result<(), HandlerError>
        + Send
        + Sync
        + 'static,
{
    fn handle(
        &self,
        request: Request,
        peer: &Peer,
        response: &mut Response<'_>,
        cancellation: &Cancellation,
    ) -> Result<(), HandlerError> {
        self(request, peer, response, cancellation)
    }
}

/// Host hook for peer credentials, including SO_PEERCRED. It runs before
/// admission and must be bounded. No native unsafe code is duplicated here.
pub trait PeerInspector: Send + Sync + 'static {
    fn inspect(&self, socket: SocketRef<'_>, peer: &mut Peer) -> io::Result<()>;
}

/// One response, with bounded writes and explicit HEAD handling.
pub struct Response<'a> {
    socket: &'a mut dyn Connection,
    maximum: usize,
    head_only: bool,
    started: bool,
}

impl Response<'_> {
    pub fn has_started(&self) -> bool {
        self.started
    }

    pub fn send(
        &mut self,
        status: u16,
        content_type: &str,
        body: &[u8],
    ) -> Result<(), HandlerError> {
        self.send_with_headers(status, content_type, &[], body)
    }

    pub fn send_with_headers(
        &mut self,
        status: u16,
        content_type: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<(), HandlerError> {
        if self.started {
            return Err(HandlerError::new(500, "response has already started"));
        }
        if body.len() > self.maximum {
            return Err(HandlerError::new(
                502,
                "response exceeds max_response_bytes",
            ));
        }
        http::validate_response_head(status, content_type, headers)?;
        if matches!(status, 204 | 304) && !body.is_empty() {
            return Err(HandlerError::new(502, "body forbidden for response status"));
        }
        self.started = true;
        http::respond_with_headers(
            self.socket,
            status,
            content_type,
            headers,
            body,
            self.head_only,
        )?;
        Ok(())
    }

    pub fn stream(
        &mut self,
        status: u16,
        content_type: &str,
        headers: &[(String, String)],
        run: impl FnOnce(&mut dyn Write) -> Result<(), HandlerError>,
    ) -> Result<(), HandlerError> {
        if self.started {
            return Err(HandlerError::new(500, "response has already started"));
        }
        http::validate_response_head(status, content_type, headers)?;
        if self.head_only {
            self.started = true;
            http::respond_head(self.socket, status, content_type, headers, None)?;
            return Ok(());
        }
        let mut writer = http::Chunked {
            socket: self.socket,
            started: false,
            status,
            content_type: content_type.to_owned(),
            extra: headers.to_vec(),
            bytes: 0,
            maximum: self.maximum,
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            run(&mut writer).and_then(|()| writer.finish().map_err(HandlerError::from))
        }));
        // Preserve wire state even when a custom callback panics after writing.
        self.started = writer.started;
        match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn fail(&mut self, error: &HandlerError) {
        if self.started {
            return;
        }
        let status = if (400..=599).contains(&error.status) {
            error.status
        } else {
            500
        };
        let body = format!(
            "{{\"error\":{{\"message\":{},\"type\":{},\"code\":{status}}}}}",
            http::quote(&error.message),
            http::quote(self.socket.error_type())
        );
        let _ = self.send(status, "application/json", body.as_bytes());
    }
}

struct ManagedConnection {
    inner: Box<dyn Connection + Send>,
    cancellation: Cancellation,
}
impl io::Read for ManagedConnection {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.inner.read(bytes)
    }
}
impl Write for ManagedConnection {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Connection for ManagedConnection {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner
            .set_read_timeout(self.cancellation.clamp_timeout(timeout))
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner
            .set_write_timeout(self.cancellation.clamp_timeout(timeout))
    }
    fn shutdown_write(&self) -> io::Result<()> {
        self.inner.shutdown_write()
    }
    fn cancelled(&self) -> bool {
        self.cancellation.is_cancelled() || self.inner.cancelled()
    }
    fn error_type(&self) -> &'static str {
        self.inner.error_type()
    }
}
struct Accepted {
    stream: ManagedConnection,
    peer: Peer,
}

struct QueueState {
    waiting: VecDeque<Accepted>,
    closed: bool,
}

struct Pool {
    config: ServerConfig,
    handler: Arc<dyn Handler>,
    inspector: Option<Arc<dyn PeerInspector>>,
    decoder: Option<Arc<dyn http::BodyDecoder + Send + Sync>>,
    cancellation: Cancellation,
    queue: Mutex<QueueState>,
    available: Condvar,
    accepted: AtomicUsize,
    rejected: AtomicUsize,
    active: AtomicUsize,
    listener_failures: AtomicUsize,
}

impl Pool {
    fn submit_native(&self, stream: Stream, mut peer: Peer) -> Result<(), SubmitError> {
        if let Some(inspector) = &self.inspector
            && let Err(error) = inspector.inspect(stream.socket(), &mut peer)
        {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(SubmitError {
                kind: SubmitFailure::PeerRejected,
                connection: Box::new(stream),
                peer,
                message: error.to_string(),
            });
        }
        self.submit_boxed(Box::new(stream), peer)
    }

    fn submit_boxed(
        &self,
        connection: Box<dyn Connection + Send>,
        peer: Peer,
    ) -> Result<(), SubmitError> {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        let failure = if queue.closed {
            Some(SubmitFailure::Stopped)
        } else if queue.waiting.len() >= self.config.queue {
            Some(SubmitFailure::Full)
        } else {
            None
        };
        if let Some(kind) = failure {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(SubmitError {
                kind,
                connection,
                peer,
                message: "server admission rejected".into(),
            });
        }
        let stream = ManagedConnection {
            inner: connection,
            cancellation: self.cancellation.with_timeout(self.config.request_timeout),
        };
        queue.waiting.push_back(Accepted { stream, peer });
        self.accepted.fetch_add(1, Ordering::Relaxed);
        self.available.notify_one();
        Ok(())
    }

    fn next(&self) -> Option<Accepted> {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if queue.closed {
                let discarded = std::mem::take(&mut queue.waiting);
                drop(queue);
                // User-supplied Connection::Drop may block. Keep destruction
                // on a tracked worker, never on the shutdown caller or lock.
                drop(discarded);
                return None;
            }
            if let Some(accepted) = queue.waiting.pop_front() {
                return Some(accepted);
            }
            queue = self
                .available
                .wait(queue)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    fn close(&self) {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        queue.closed = true;
        self.available.notify_all();
    }

    fn work(&self) {
        while let Some(mut accepted) = self.next() {
            self.active.fetch_add(1, Ordering::Relaxed);
            let _ = catch_unwind(AssertUnwindSafe(|| self.process(&mut accepted)));
            self.active.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn process(&self, accepted: &mut Accepted) {
        let cancel = accepted.stream.cancellation.clone();
        if accepted
            .stream
            .set_write_timeout(Some(self.config.write_timeout))
            .is_err()
        {
            return;
        }
        let request = match http::read_request_with_decoder(
            &mut accepted.stream,
            self.config.max_body_bytes,
            self.config.read_timeout,
            self.config.request_policy,
            self.decoder
                .as_deref()
                .map(|decoder| decoder as &dyn http::BodyDecoder),
        ) {
            Ok(request) => request,
            Err(error) => {
                http::reject_and_close(&mut accepted.stream, error.status, error.message);
                return;
            }
        };
        let mut response = Response {
            socket: &mut accepted.stream,
            maximum: self.config.max_response_bytes,
            head_only: request.method == "HEAD",
            started: false,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.handler
                .handle(request, &accepted.peer, &mut response, &cancel)
        }));
        match result {
            Ok(Ok(())) if !response.has_started() => {
                response.fail(&HandlerError::new(
                    500,
                    "handler returned without a response",
                ));
            }
            Ok(Err(error)) => response.fail(&error),
            Err(_) => response.fail(&HandlerError::new(500, "handler panicked")),
            _ => {}
        }
        let _ = accepted.stream.shutdown_write();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitFailure {
    Full,
    Stopped,
    PeerRejected,
}

/// Admission failure returns ownership so a native host can send its own
/// rejection or use reject(), which writes a bounded 503/403 and half-closes.
pub struct SubmitError {
    pub kind: SubmitFailure,
    pub connection: Box<dyn Connection + Send>,
    pub peer: Peer,
    pub message: String,
}

impl SubmitError {
    pub fn reject(mut self) {
        let _ = self
            .connection
            .set_write_timeout(Some(Duration::from_millis(250)));
        let status = if self.kind == SubmitFailure::PeerRejected {
            403
        } else {
            503
        };
        http::reject_and_close(
            self.connection.as_mut(),
            status,
            if status == 403 {
                "peer is not permitted"
            } else {
                "server is busy or stopping"
            },
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerStats {
    pub accepted: usize,
    pub rejected: usize,
    pub active: usize,
    pub queued: usize,
    pub listener_failures: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShutdownReport {
    pub workers_remaining: usize,
    pub acceptors_remaining: usize,
    pub wakers_remaining: usize,
}

impl ShutdownReport {
    pub fn complete(&self) -> bool {
        self.workers_remaining + self.acceptors_remaining + self.wakers_remaining == 0
    }
}

/// Bounded worker pool with optional blocking acceptors. Idle workers/acceptors
/// sleep in the OS; there is no timer-based listener polling.
///
/// A host with an existing native readiness loop can omit listen() and call
/// submit() instead. Explicit shutdown reports uncooperative handlers; dropping
/// a handle never claims those arbitrary user threads were forcibly killed.
pub struct Server {
    pool: Arc<Pool>,
    workers: Vec<JoinHandle<()>>,
    acceptors: Vec<JoinHandle<()>>,
    wakers: Vec<JoinHandle<()>>,
    wake_addresses: Vec<WakeAddress>,
    addresses: Vec<Address>,
    stopping: bool,
}

impl Server {
    pub fn new(config: ServerConfig, handler: impl Handler) -> io::Result<Self> {
        Self::with_peer_inspector(config, handler, None)
    }

    pub fn with_peer_inspector(
        config: ServerConfig,
        handler: impl Handler,
        inspector: Option<Arc<dyn PeerInspector>>,
    ) -> io::Result<Self> {
        Self::with_hooks(config, handler, inspector, None)
    }

    /// Host peer verification and request decoding remain explicit opt-ins.
    pub fn with_hooks(
        config: ServerConfig,
        handler: impl Handler,
        inspector: Option<Arc<dyn PeerInspector>>,
        decoder: Option<Arc<dyn http::BodyDecoder + Send + Sync>>,
    ) -> io::Result<Self> {
        config.validate()?;
        let pool = Arc::new(Pool {
            config,
            handler: Arc::new(handler),
            inspector,
            decoder,
            cancellation: Cancellation::new(),
            queue: Mutex::new(QueueState {
                waiting: VecDeque::new(),
                closed: false,
            }),
            available: Condvar::new(),
            accepted: AtomicUsize::new(0),
            rejected: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            listener_failures: AtomicUsize::new(0),
        });
        let mut server = Self {
            pool,
            workers: Vec::new(),
            acceptors: Vec::new(),
            wakers: Vec::new(),
            wake_addresses: Vec::new(),
            addresses: Vec::new(),
            stopping: false,
        };
        for index in 0..server.pool.config.workers {
            let pool = server.pool.clone();
            server.workers.push(
                thread::Builder::new()
                    .name(format!("zcall-worker-{index}"))
                    .spawn(move || pool.work())?,
            );
        }
        Ok(server)
    }

    /// Adds a listener before or while serving. At most 16 listeners are owned.
    /// The listener must be blocking; this method restores that mode.
    pub fn listen(&mut self, listener: Listener) -> io::Result<Address> {
        if self.stopping || self.acceptors.len() >= 16 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "server is stopped or listener limit reached",
            ));
        }
        listener.set_nonblocking(false)?;
        let address = listener.address()?;
        let wake = listener.wake_address()?;
        let pool = self.pool.clone();
        let index = self.acceptors.len();
        let acceptor = thread::Builder::new()
            .name(format!("zcall-accept-{index}"))
            .stack_size(256 * 1024)
            .spawn(move || {
                while !pool.cancellation.is_cancelled() {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            if pool.cancellation.is_cancelled() {
                                break;
                            }
                            if let Err(error) = pool.submit_native(stream, peer) {
                                error.reject();
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => {
                            pool.listener_failures.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            })?;
        self.acceptors.push(acceptor);
        self.wake_addresses.push(wake);
        self.addresses.push(address.clone());
        Ok(address)
    }

    pub fn addresses(&self) -> &[Address] {
        &self.addresses
    }

    /// Submit a host-owned connection with host-verified peer metadata.
    /// PeerInspector applies to owned native acceptors, not this host boundary.
    pub fn submit(
        &self,
        connection: impl Connection + Send + 'static,
        peer: Peer,
    ) -> Result<(), SubmitError> {
        self.submit_boxed(Box::new(connection), peer)
    }

    pub fn submit_boxed(
        &self,
        connection: Box<dyn Connection + Send>,
        peer: Peer,
    ) -> Result<(), SubmitError> {
        self.pool.submit_boxed(connection, peer)
    }

    /// Apply the optional native peer inspector before admission.
    pub fn submit_stream(&self, stream: Stream, peer: Peer) -> Result<(), SubmitError> {
        self.pool.submit_native(stream, peer)
    }

    pub fn cancellation(&self) -> Cancellation {
        self.pool.cancellation.clone()
    }

    pub fn stats(&self) -> ServerStats {
        let queued = self
            .pool
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .waiting
            .len();
        ServerStats {
            accepted: self.pool.accepted.load(Ordering::Relaxed),
            rejected: self.pool.rejected.load(Ordering::Relaxed),
            active: self.pool.active.load(Ordering::Relaxed),
            queued,
            listener_failures: self.pool.listener_failures.load(Ordering::Relaxed),
        }
    }

    /// Stop admission, discard queued work, cancel in-flight handlers and wait
    /// no longer than this budget for cooperative threads. All waits share one
    /// deadline. A nonzero report is explicit, and another call can reap later.
    pub fn shutdown(&mut self, timeout: Duration) -> ShutdownReport {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        if !self.stopping {
            self.stopping = true;
            self.pool.cancellation.cancel();
            self.pool.close();
            for (index, address) in self.wake_addresses.drain(..).enumerate() {
                if let Ok(waker) = thread::Builder::new()
                    .name(format!("zcall-wake-{index}"))
                    .stack_size(128 * 1024)
                    .spawn(move || address.wake())
                {
                    self.wakers.push(waker);
                }
            }
        }
        loop {
            reap(&mut self.workers);
            reap(&mut self.acceptors);
            reap(&mut self.wakers);
            let report = ShutdownReport {
                workers_remaining: self.workers.len(),
                acceptors_remaining: self.acceptors.len(),
                wakers_remaining: self.wakers.len(),
            };
            if report.complete() || Instant::now() >= deadline {
                return report;
            }
            thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(2)),
            );
        }
    }
}

fn reap(handles: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < handles.len() {
        if handles[index].is_finished() {
            let _ = handles.swap_remove(index).join();
        } else {
            index += 1;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown(Duration::ZERO);
    }
}
