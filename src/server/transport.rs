//! Safe standard-library TCP/Unix connections and owned Unix socket paths.
use super::{Cancellation, Connection};
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    time::Duration,
};

#[cfg(unix)]
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::Arc,
};

/// Transport location. A local socket is not evidence of an OS peer identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Address {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

/// Metadata established by the listener and an optional host peer inspector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Peer {
    pub address: Address,
    pub loopback: bool,
    pub uid: Option<u32>,
    pub pid: Option<u32>,
}

/// Borrow the native stream for a host-owned peer-credential hook.
pub enum SocketRef<'a> {
    Tcp(&'a TcpStream),
    #[cfg(unix)]
    Unix(&'a UnixStream),
}

enum StreamKind {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix {
        socket: UnixStream,
        binding: Option<Arc<UnixBinding>>,
    },
}

/// An accepted stream. Clones retain the Unix ownership guard and cancellation.
pub struct Stream {
    inner: StreamKind,
    cancellation: Cancellation,
}

impl Stream {
    pub fn from_tcp(socket: TcpStream) -> Self {
        Self {
            inner: StreamKind::Tcp(socket),
            cancellation: Cancellation::new(),
        }
    }

    #[cfg(unix)]
    pub fn from_unix(socket: UnixStream) -> Self {
        Self {
            inner: StreamKind::Unix {
                socket,
                binding: None,
            },
            cancellation: Cancellation::new(),
        }
    }

    pub fn socket(&self) -> SocketRef<'_> {
        match &self.inner {
            StreamKind::Tcp(socket) => SocketRef::Tcp(socket),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => SocketRef::Unix(socket),
        }
    }

    pub fn set_cancellation(&mut self, cancellation: Cancellation) {
        self.cancellation = cancellation;
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        let inner = match &self.inner {
            StreamKind::Tcp(socket) => StreamKind::Tcp(socket.try_clone()?),
            #[cfg(unix)]
            StreamKind::Unix { socket, binding } => StreamKind::Unix {
                socket: socket.try_clone()?,
                binding: binding.clone(),
            },
        };
        Ok(Self {
            inner,
            cancellation: self.cancellation.clone(),
        })
    }

    pub fn shutdown(&self) -> io::Result<()> {
        match &self.inner {
            StreamKind::Tcp(socket) => socket.shutdown(Shutdown::Both),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => socket.shutdown(Shutdown::Both),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            StreamKind::Tcp(socket) => socket.read(bytes),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => socket.read(bytes),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            StreamKind::Tcp(socket) => socket.write(bytes),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => socket.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            StreamKind::Tcp(socket) => socket.flush(),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => socket.flush(),
        }
    }
}

impl Connection for Stream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        let timeout = self.cancellation.clamp_timeout(timeout);
        match &self.inner {
            StreamKind::Tcp(socket) => socket.set_read_timeout(timeout),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => socket.set_read_timeout(timeout),
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        let timeout = self.cancellation.clamp_timeout(timeout);
        match &self.inner {
            StreamKind::Tcp(socket) => socket.set_write_timeout(timeout),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => socket.set_write_timeout(timeout),
        }
    }

    fn shutdown_write(&self) -> io::Result<()> {
        match &self.inner {
            StreamKind::Tcp(socket) => socket.shutdown(Shutdown::Write),
            #[cfg(unix)]
            StreamKind::Unix { socket, .. } => socket.shutdown(Shutdown::Write),
        }
    }

    fn cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

/// A blocking listener. Hosts with native readiness APIs may use nonblocking
/// mode and call accept themselves before submitting to the worker pool.
pub enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix {
        listener: UnixListener,
        binding: Arc<UnixBinding>,
    },
}

impl Listener {
    pub fn bind_tcp(address: impl std::net::ToSocketAddrs) -> io::Result<Self> {
        Ok(Self::Tcp(TcpListener::bind(address)?))
    }

    pub fn from_tcp(listener: TcpListener) -> Self {
        Self::Tcp(listener)
    }

    pub fn address(&self) -> io::Result<Address> {
        match self {
            Self::Tcp(listener) => Ok(Address::Tcp(listener.local_addr()?)),
            #[cfg(unix)]
            Self::Unix { binding, .. } => Ok(Address::Unix(binding.path.clone())),
        }
    }

    pub fn set_nonblocking(&self, value: bool) -> io::Result<()> {
        match self {
            Self::Tcp(listener) => listener.set_nonblocking(value),
            #[cfg(unix)]
            Self::Unix { listener, .. } => listener.set_nonblocking(value),
        }
    }

    pub fn accept(&self) -> io::Result<(Stream, Peer)> {
        match self {
            Self::Tcp(listener) => {
                let (socket, address) = listener.accept()?;
                socket.set_nodelay(true)?;
                Ok((
                    Stream::from_tcp(socket),
                    Peer {
                        address: Address::Tcp(address),
                        loopback: address.ip().is_loopback(),
                        uid: None,
                        pid: None,
                    },
                ))
            }
            #[cfg(unix)]
            Self::Unix { listener, binding } => {
                let (socket, _) = listener.accept()?;
                Ok((
                    Stream {
                        inner: StreamKind::Unix {
                            socket,
                            binding: Some(binding.clone()),
                        },
                        cancellation: Cancellation::new(),
                    },
                    Peer {
                        address: Address::Unix(binding.path.clone()),
                        loopback: true,
                        uid: None,
                        pid: None,
                    },
                ))
            }
        }
    }

    /// Bind in an existing private directory owned by owner_uid. No parent is
    /// created and no existing non-socket node is ever replaced.
    #[cfg(unix)]
    pub fn bind_unix(path: impl AsRef<Path>, owner_uid: u32) -> io::Result<Self> {
        Self::bind_unix_with_probe(path, owner_uid, |_| {
            Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "existing Unix socket requires an explicit bounded liveness probe",
            ))
        })
    }

    /// An embedding host may supply its bounded native liveness probe to
    /// recover a stale owned socket. false means confirmed stale, not unknown.
    /// The default bind method never removes an existing socket.
    #[cfg(unix)]
    pub fn bind_unix_with_probe(
        path: impl AsRef<Path>,
        owner_uid: u32,
        live: impl FnOnce(&Path) -> io::Result<bool>,
    ) -> io::Result<Self> {
        let path = path.as_ref();
        validate_parent(path, owner_uid)?;
        let mut lock_name = path
            .file_name()
            .ok_or_else(|| invalid("Unix socket needs a file name"))?
            .to_os_string();
        lock_name.push(".lock");
        let lock_path = path.with_file_name(lock_name);
        let lock = open_lock(&lock_path, owner_uid)?;
        lock.try_lock().map_err(|error| {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("Unix socket lock: {error}"),
            )
        })?;
        // Recheck the name after obtaining the lock, so a renamed lock inode
        // cannot authorize a different path.
        let current_lock = fs::symlink_metadata(&lock_path)?;
        if !same_file(&current_lock, &lock.metadata()?) {
            return Err(invalid("Unix socket lock identity changed"));
        }
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() || metadata.uid() != owner_uid {
                    return Err(invalid("existing Unix socket path is not an owned socket"));
                }
                if live(path)? {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "Unix socket is live",
                    ));
                }
                let now = fs::symlink_metadata(path)?;
                if !same_file(&metadata, &now) {
                    return Err(invalid("Unix socket changed during stale check"));
                }
                fs::remove_file(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(path)?;
        let metadata = fs::symlink_metadata(path)?;
        let binding = Arc::new(UnixBinding {
            path: path.to_owned(),
            identity: (metadata.dev(), metadata.ino()),
            owner_uid,
            _lock: lock,
        });
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Self::Unix { listener, binding })
    }

    pub(crate) fn wake_address(&self) -> io::Result<WakeAddress> {
        Ok(WakeAddress {
            address: self.address()?,
            #[cfg(unix)]
            binding: match self {
                Self::Unix { binding, .. } => Some(binding.clone()),
                _ => None,
            },
        })
    }
}

#[cfg(unix)]
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(unix)]
fn validate_parent(path: &Path, owner_uid: u32) -> io::Result<()> {
    if !path.is_absolute()
        || path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err(invalid(
            "Unix socket path must be absolute without traversal",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Unix socket needs a parent"))?;
    for ancestor in parent.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid("Unix socket ancestors must be real directories"));
        }
    }
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.uid() != owner_uid || metadata.mode() & 0o077 != 0 {
        return Err(invalid(
            "Unix socket parent must be private and owned by owner_uid",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn open_lock(path: &Path, owner_uid: u32) -> io::Result<File> {
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let before = fs::symlink_metadata(path)?;
            if !before.is_file() || before.file_type().is_symlink() {
                return Err(invalid("Unix socket lock must be a regular file"));
            }
            let file = OpenOptions::new().read(true).write(true).open(path)?;
            if !same_file(&before, &file.metadata()?) {
                return Err(invalid("Unix socket lock changed during open"));
            }
            file
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner_uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(invalid(
            "Unix socket lock must be private, owned and singly linked",
        ));
    }
    Ok(file)
}

/// Public only because it is retained by the Unix Listener enum variant.
/// Its fields and construction are private.
#[cfg(unix)]
pub struct UnixBinding {
    path: PathBuf,
    identity: (u64, u64),
    owner_uid: u32,
    _lock: File,
}

#[cfg(unix)]
impl Drop for UnixBinding {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && metadata.uid() == self.owner_uid
            && (metadata.dev(), metadata.ino()) == self.identity
        {
            let _ = fs::remove_file(&self.path);
        }
        // Keep the stable lock file. Unlinking it could split old waiters from
        // a new process using a replacement inode.
    }
}

pub(crate) struct WakeAddress {
    address: Address,
    #[cfg(unix)]
    binding: Option<Arc<UnixBinding>>,
}

impl WakeAddress {
    pub(crate) fn wake(&self) {
        match &self.address {
            Address::Tcp(address) => {
                let mut address = *address;
                if address.ip().is_unspecified() {
                    address.set_ip(if address.is_ipv4() {
                        std::net::Ipv4Addr::LOCALHOST.into()
                    } else {
                        std::net::Ipv6Addr::LOCALHOST.into()
                    });
                }
                let _ = TcpStream::connect_timeout(&address, Duration::from_millis(100));
            }
            #[cfg(unix)]
            Address::Unix(path) => {
                // Called on a bounded-shutdown helper thread, not the caller.
                // The acceptor owns the only listener descriptor, so closing
                // it also releases any connect waiting for backlog capacity.
                if let Some(binding) = &self.binding
                    && let Ok(metadata) = fs::symlink_metadata(path)
                    && (metadata.dev(), metadata.ino()) == binding.identity
                {
                    let _ = UnixStream::connect(path);
                }
            }
        }
    }
}
