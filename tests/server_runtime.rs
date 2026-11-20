#![cfg(feature = "server")]

use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant},
};
use zcall::server::{
    Cancellation, HandlerError, PeerInspector, Response, Server, ServerConfig,
    http::{Request, RequestPolicy},
    transport::{Address, Listener, Peer, SocketRef},
};

fn echo(
    request: Request,
    _: &Peer,
    response: &mut Response<'_>,
    _: &Cancellation,
) -> Result<(), HandlerError> {
    if request.path != "/custom" {
        return Err(HandlerError::new(404, "custom route not found"));
    }
    response.send(200, "text/plain", b"native")
}

fn tcp_address(address: Address) -> std::net::SocketAddr {
    match address {
        Address::Tcp(address) => address,
        #[cfg(unix)]
        _ => panic!("TCP address expected"),
    }
}

fn connect(address: std::net::SocketAddr, request: &[u8]) -> TcpStream {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(request).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    stream
}

fn read_reply(mut stream: TcpStream) -> String {
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

const GET: &[u8] = b"GET /custom HTTP/1.1\r\nHost: localhost\r\n\r\n";

#[test]
fn custom_routes_media_origin_and_head_are_host_owned() {
    let config = ServerConfig {
        request_policy: RequestPolicy {
            allow_origin: true,
            require_json_post: false,
            ..RequestPolicy::default()
        },
        ..ServerConfig::default()
    };
    let mut server = Server::new(config, echo).unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let reply = read_reply(connect(address, b"POST /custom HTTP/1.1\r\nHost: localhost\r\nOrigin: https://example.invalid\r\nContent-Type: text/plain\r\nContent-Length: 3\r\n\r\nabc"));
    assert!(reply.starts_with("HTTP/1.1 200"));
    assert!(reply.ends_with("native"));
    let reply = read_reply(connect(
        address,
        b"HEAD /custom HTTP/1.1\r\nHost: localhost\r\n\r\n",
    ));
    assert!(reply.contains("Content-Length: 6\r\n"));
    assert_eq!(reply.split_once("\r\n\r\n").unwrap().1, "");
    let missing = read_reply(connect(
        address,
        b"HEAD /missing HTTP/1.1\r\nHost: localhost\r\n\r\n",
    ));
    assert!(missing.starts_with("HTTP/1.1 404"));
    assert_eq!(missing.split_once("\r\n\r\n").unwrap().1, "");
    assert!(server.shutdown(Duration::from_secs(2)).complete());
}

#[test]
fn queue_saturation_has_an_explicit_bounded_503() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let handler_gate = gate.clone();
    let (entered, observed) = mpsc::channel();
    let config = ServerConfig {
        workers: 1,
        queue: 1,
        ..ServerConfig::default()
    };
    let mut server = Server::new(
        config,
        move |_: Request, _: &Peer, response: &mut Response<'_>, _: &Cancellation| {
            let _ = entered.send(());
            let (lock, wake) = &*handler_gate;
            let guard = lock.lock().unwrap();
            drop(wake.wait_while(guard, |ready| !*ready).unwrap());
            response.send(200, "text/plain", b"done")
        },
    )
    .unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let first = connect(address, GET);
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let second = connect(address, GET);
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.stats().queued != 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(server.stats().queued, 1);
    let start = Instant::now();
    let rejection = read_reply(connect(address, GET));
    assert!(rejection.starts_with("HTTP/1.1 503"));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(server.stats().rejected, 1);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(read_reply(first).ends_with("done"));
    assert!(read_reply(second).ends_with("done"));
    assert!(server.shutdown(Duration::from_secs(2)).complete());
}

#[test]
fn idle_and_active_shutdown_share_a_single_bounded_deadline() {
    let (entered, observed) = mpsc::channel();
    let mut server = Server::new(
        ServerConfig::default(),
        move |_: Request, _: &Peer, _: &mut Response<'_>, cancel: &Cancellation| {
            entered.send(()).unwrap();
            assert!(cancel.wait_timeout(Duration::from_secs(30)));
            Err(HandlerError::new(503, "cancelled"))
        },
    )
    .unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let _request = connect(address, GET);
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let start = Instant::now();
    assert!(server.shutdown(Duration::from_secs(1)).complete());
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(server.shutdown(Duration::ZERO).complete());

    let mut idle = Server::new(ServerConfig::default(), echo).unwrap();
    idle.listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
        .unwrap();
    assert!(idle.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn uncooperative_handler_is_reported_without_an_unbounded_join() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let handler_gate = gate.clone();
    let (entered, observed) = mpsc::channel();
    let mut server = Server::new(
        ServerConfig {
            workers: 1,
            ..ServerConfig::default()
        },
        move |_: Request, _: &Peer, _: &mut Response<'_>, _: &Cancellation| {
            entered.send(()).unwrap();
            let (lock, wake) = &*handler_gate;
            let guard = lock.lock().unwrap();
            drop(wake.wait_while(guard, |ready| !*ready).unwrap());
            Ok(())
        },
    )
    .unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let _request = connect(address, GET);
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let start = Instant::now();
    let report = server.shutdown(Duration::from_millis(20));
    assert_eq!(report.workers_remaining, 1);
    assert!(!report.complete());
    assert!(start.elapsed() < Duration::from_millis(300));
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

struct Credentials;
impl PeerInspector for Credentials {
    fn inspect(&self, _: SocketRef<'_>, peer: &mut Peer) -> std::io::Result<()> {
        peer.uid = Some(1379);
        peer.pid = Some(2468);
        Ok(())
    }
}

#[test]
fn native_peer_metadata_comes_from_the_hook_not_request_headers() {
    let mut server = Server::with_peer_inspector(
        ServerConfig::default(),
        |_: Request, peer: &Peer, response: &mut Response<'_>, _: &Cancellation| {
            assert_eq!(peer.uid, Some(1379));
            assert_eq!(peer.pid, Some(2468));
            response.send(200, "text/plain", b"verified")
        },
        Some(Arc::new(Credentials)),
    )
    .unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let reply = read_reply(connect(
        address,
        b"GET /custom HTTP/1.1\r\nHost: localhost\r\nX-Peer-Uid: 0\r\n\r\n",
    ));
    assert!(reply.ends_with("verified"));
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn request_deadline_notifies_a_handler_without_a_polling_timer() {
    let config = ServerConfig {
        request_timeout: Duration::from_millis(30),
        ..ServerConfig::default()
    };
    let (elapsed, observed) = mpsc::channel();
    let mut server = Server::new(
        config,
        move |_: Request, _: &Peer, _: &mut Response<'_>, cancel: &Cancellation| {
            let start = Instant::now();
            assert!(cancel.wait_timeout(Duration::from_secs(30)));
            elapsed.send(start.elapsed()).unwrap();
            Err(HandlerError::new(504, "deadline"))
        },
    )
    .unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let _request = connect(address, GET);
    assert!(observed.recv_timeout(Duration::from_secs(1)).unwrap() < Duration::from_millis(500));
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs::{self, DirBuilder},
        os::unix::{
            fs::{DirBuilderExt, MetadataExt, PermissionsExt, symlink},
            net::{UnixListener, UnixStream},
        },
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static IDS: AtomicU64 = AtomicU64::new(0);

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = fs::canonicalize("/tmp").unwrap().join(format!(
                "zcall-server-{}-{}",
                std::process::id(),
                IDS.fetch_add(1, Ordering::Relaxed)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
        fn uid(&self) -> u32 {
            fs::metadata(&self.0).unwrap().uid()
        }
        fn socket(&self) -> PathBuf {
            self.0.join("api.sock")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn socket_guard_and_lock_live_until_the_last_accepted_clone() {
        let directory = Directory::new();
        let path = directory.socket();
        let listener = Listener::bind_unix(&path, directory.uid()).unwrap();
        let _client = UnixStream::connect(&path).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let duplicate = stream.try_clone().unwrap();
        let lock_path = path.with_file_name("api.sock.lock");
        let inode = fs::metadata(&lock_path).unwrap().ino();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        drop(listener);
        drop(stream);
        assert!(path.exists());
        assert!(Listener::bind_unix(&path, directory.uid()).is_err());
        drop(duplicate);
        assert!(!path.exists());
        let rebound = Listener::bind_unix(&path, directory.uid()).unwrap();
        assert_eq!(fs::metadata(&lock_path).unwrap().ino(), inode);
        drop(rebound);
        assert!(!path.exists());
        assert_eq!(fs::metadata(&lock_path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn unsafe_nodes_owners_and_public_parents_are_preserved() {
        let directory = Directory::new();
        let path = directory.socket();
        fs::write(&path, b"preserve").unwrap();
        assert!(Listener::bind_unix(&path, directory.uid()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"preserve");
        fs::remove_file(&path).unwrap();
        let victim = directory.0.join("victim");
        fs::write(&victim, b"preserve").unwrap();
        symlink(&victim, &path).unwrap();
        assert!(Listener::bind_unix(&path, directory.uid()).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"preserve");
        fs::remove_file(&path).unwrap();
        assert!(Listener::bind_unix(&path, directory.uid().wrapping_add(1)).is_err());
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Listener::bind_unix(&path, directory.uid()).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn existing_socket_requires_explicit_confirmed_stale_evidence() {
        let directory = Directory::new();
        let path = directory.socket();
        let original = UnixListener::bind(&path).unwrap();
        assert!(Listener::bind_unix(&path, directory.uid()).is_err());
        assert!(Listener::bind_unix_with_probe(&path, directory.uid(), |_| Ok(true)).is_err());
        assert!(path.exists());
        drop(original);
        let rebound =
            Listener::bind_unix_with_probe(
                &path,
                directory.uid(),
                |path| match UnixStream::connect(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                        Ok(false)
                    }
                    Ok(_) => Ok(true),
                    Err(error) => Err(error),
                },
            )
            .unwrap();
        drop(rebound);
        assert!(!path.exists());
    }

    #[test]
    fn cleanup_never_deletes_a_replacement_node() {
        let directory = Directory::new();
        let path = directory.socket();
        let listener = Listener::bind_unix(&path, directory.uid()).unwrap();
        fs::rename(&path, directory.0.join("old.sock")).unwrap();
        fs::write(&path, b"replacement").unwrap();
        drop(listener);
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
    }

    #[test]
    fn one_pool_serves_tcp_and_unix_and_closes_both_acceptors() {
        let directory = Directory::new();
        let path = directory.socket();
        let mut server = Server::new(ServerConfig::default(), echo).unwrap();
        let tcp = tcp_address(
            server
                .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
                .unwrap(),
        );
        server
            .listen(Listener::bind_unix(&path, directory.uid()).unwrap())
            .unwrap();
        assert!(read_reply(connect(tcp, GET)).ends_with("native"));
        let mut unix = UnixStream::connect(&path).unwrap();
        unix.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        unix.write_all(GET).unwrap();
        unix.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        unix.read_to_string(&mut response).unwrap();
        assert!(response.ends_with("native"));
        assert_eq!(server.addresses().len(), 2);
        assert!(server.shutdown(Duration::from_secs(1)).complete());
        assert!(!path.exists());
    }
}

#[test]
fn stream_panic_cannot_append_a_second_http_response() {
    let mut server = Server::new(
        ServerConfig {
            workers: 1,
            ..ServerConfig::default()
        },
        |request: Request, _: &Peer, response: &mut Response<'_>, _: &Cancellation| {
            if request.path == "/panic" {
                response.stream(200, "text/event-stream", &[], |writer| {
                    writer.write_all(b"data: hello\n\n")?;
                    panic!("intentional local fixture panic");
                })
            } else {
                response.send(200, "text/plain", b"still alive")
            }
        },
    )
    .unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let broken = read_reply(connect(
        address,
        b"GET /panic HTTP/1.1\r\nHost: localhost\r\n\r\n",
    ));
    assert_eq!(broken.matches("HTTP/1.1").count(), 1);
    assert!(broken.starts_with("HTTP/1.1 200"));
    assert!(!broken.ends_with("0\r\n\r\n"));
    assert!(read_reply(connect(address, GET)).ends_with("still alive"));
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn a_host_connection_can_use_the_pool_without_library_acceptors() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client = connect(listener.local_addr().unwrap(), GET);
    let (accepted, address) = listener.accept().unwrap();
    let peer = Peer {
        address: Address::Tcp(address),
        loopback: true,
        uid: None,
        pid: None,
    };
    let mut server = Server::new(ServerConfig::default(), echo).unwrap();
    assert!(server.submit(accepted, peer).is_ok());
    assert!(server.addresses().is_empty());
    assert!(read_reply(client).ends_with("native"));
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn a_blocking_queued_connection_drop_does_not_block_shutdown_or_queue_lock() {
    struct BlockingDrop {
        stream: TcpStream,
        gate: Arc<(Mutex<bool>, Condvar)>,
        entered: mpsc::Sender<()>,
    }
    impl Read for BlockingDrop {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.stream.read(bytes)
        }
    }
    impl Write for BlockingDrop {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.stream.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.stream.flush()
        }
    }
    impl zcall::server::Connection for BlockingDrop {
        fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
            self.stream.set_read_timeout(timeout)
        }
        fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
            self.stream.set_write_timeout(timeout)
        }
        fn shutdown_write(&self) -> std::io::Result<()> {
            self.stream.shutdown(Shutdown::Write)
        }
    }
    impl Drop for BlockingDrop {
        fn drop(&mut self) {
            let _ = self.entered.send(());
            let lock = self.gate.0.lock().unwrap();
            drop(self.gate.1.wait_while(lock, |ready| !*ready).unwrap());
        }
    }
    let (handler_entered, handler_observed) = mpsc::channel();
    let mut server = Server::new(
        ServerConfig {
            workers: 1,
            queue: 1,
            ..ServerConfig::default()
        },
        move |_: Request, _: &Peer, _: &mut Response<'_>, cancel: &Cancellation| {
            handler_entered.send(()).unwrap();
            cancel.wait_timeout(Duration::from_secs(3));
            Ok(())
        },
    )
    .unwrap();
    let address = tcp_address(
        server
            .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
            .unwrap(),
    );
    let _first = connect(address, GET);
    handler_observed
        .recv_timeout(Duration::from_secs(2))
        .unwrap();

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let _second = connect(listener.local_addr().unwrap(), GET);
    let (stream, address) = listener.accept().unwrap();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (entered, observed) = mpsc::channel();
    let peer = Peer {
        address: Address::Tcp(address),
        loopback: true,
        uid: None,
        pid: None,
    };
    assert!(
        server
            .submit(
                BlockingDrop {
                    stream,
                    gate: gate.clone(),
                    entered
                },
                peer
            )
            .is_ok()
    );
    assert_eq!(server.stats().queued, 1);

    // This fallback releases a broken implementation too, turning a regression
    // into a bounded failing test rather than hanging the test runner.
    let fallback_gate = gate.clone();
    let fallback = std::thread::spawn(move || {
        let ready = fallback_gate.0.lock().unwrap();
        let (mut ready, _) = fallback_gate
            .1
            .wait_timeout_while(ready, Duration::from_millis(500), |ready| !*ready)
            .unwrap();
        *ready = true;
        fallback_gate.1.notify_all();
    });
    let start = Instant::now();
    let report = server.shutdown(Duration::from_millis(10));
    let elapsed = start.elapsed();
    observed.recv_timeout(Duration::from_secs(1)).unwrap();
    let queued = server.stats().queued;
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    fallback.join().unwrap();
    assert!(elapsed < Duration::from_millis(200), "{elapsed:?}");
    assert_eq!(report.workers_remaining, 1);
    assert_eq!(queued, 0);
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}
