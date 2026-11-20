//! Opt-in, synchronous HTTP gateway primitives.
//!
//! This module requires the server feature. It owns bounded buffers and may
//! perform local socket/file I/O; the default kernel remains Sans-I/O and
//! allocation-free. Applications keep their routing, lifecycle, upstream
//! transport and extension policy.

use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    time::Duration,
};

pub mod auth;
pub mod http;
pub mod protocol;

/// A connection supplied by a server or embedding application.
///
/// The host controls cancellation and error naming. Timeout methods affect
/// subsequent reads/writes; shutdown_write preserves the readable half.
pub trait Connection: Read + Write {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn shutdown_write(&self) -> io::Result<()>;
    fn cancelled(&self) -> bool {
        false
    }
    fn error_type(&self) -> &'static str {
        "zcall_error"
    }
}

impl Connection for TcpStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }
    fn shutdown_write(&self) -> io::Result<()> {
        self.shutdown(Shutdown::Write)
    }
}

#[cfg(unix)]
impl Connection for std::os::unix::net::UnixStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        Self::set_read_timeout(self, timeout)
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        Self::set_write_timeout(self, timeout)
    }
    fn shutdown_write(&self) -> io::Result<()> {
        self.shutdown(Shutdown::Write)
    }
}
