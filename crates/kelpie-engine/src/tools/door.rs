//! The one door a sandboxed agent reaches kelpie through
//!
//! The relay runs inside crabpot, which closes every route out but the ones the
//! policy names. The door is the one it names, and it differs by platform
//! because crabpot's narrow grants do (`docs/design/kelpie-tools.md`, The route
//! out):
//!
//! | Platform | Door | crabpot key |
//! |---|---|---|
//! | macOS | A Unix socket in kelpie's data folder | `allowUnixSockets`, that one path |
//! | Linux | TCP on `127.0.0.1`, a port the OS picks | `allowTcp`, that one address |
//!
//! Linux cannot take the socket, since crabpot's seccomp filter refuses every
//! `AF_UNIX` socket. macOS cannot take `allowTcp`, which Seatbelt cannot narrow
//! to one host. Windows waits with the rest of Windows (ADR 0005).
//!
//! Lifted from SK's bridge (`crates/adapters/src/adapters/bridge/mod.rs` at
//! `1dc51c040`): the socket's name is kept in a file so a later run binds the
//! same path, a socket a killed run left is replaced, and dropping removes it.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::{env, fs};

/// The longest path a socket may be bound to on macOS, in bytes
///
/// `sockaddr_un.sun_path` is 104 bytes there, the last of them the NUL.
#[cfg(any(target_os = "macos", test))]
pub(super) const LONGEST_SOCKET: usize = 103;

/// What a relay dials: a Unix socket's path, or a TCP address
///
/// Its text form is what `KELPIE_TOOLS_DOOR` holds: an absolute path, or
/// `host:port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Door {
    address: Address,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Address {
    Socket(PathBuf),
    Tcp(SocketAddr),
}

impl Door {
    /// The door the text names
    ///
    /// # Errors
    ///
    /// The text is neither an absolute path nor an address with a port.
    pub fn parse(text: &str) -> io::Result<Self> {
        let address = if Path::new(text).is_absolute() {
            Address::Socket(PathBuf::from(text))
        } else {
            Address::Tcp(text.parse().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{text:?} is not a socket path or an address with a port: {error}"),
                )
            })?)
        };
        Ok(Self { address })
    }

    /// The Unix socket this door is, for the policy's `allowUnixSockets`
    #[must_use]
    pub fn socket(&self) -> Option<&Path> {
        match &self.address {
            Address::Socket(path) => Some(path),
            Address::Tcp(_) => None,
        }
    }

    /// The TCP address this door is, for the policy's `allowTcp`
    #[must_use]
    pub fn tcp(&self) -> Option<SocketAddr> {
        match &self.address {
            Address::Socket(_) => None,
            Address::Tcp(address) => Some(*address),
        }
    }

    /// Connect, as a relay does
    pub(super) fn connect(&self) -> io::Result<Stream> {
        match &self.address {
            Address::Tcp(address) => TcpStream::connect(address).map(Stream::Tcp),
            #[cfg(unix)]
            Address::Socket(path) => {
                std::os::unix::net::UnixStream::connect(path).map(Stream::Unix)
            }
            #[cfg(not(unix))]
            Address::Socket(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a Unix socket door waits for Windows (ADR 0005)",
            )),
        }
    }
}

impl fmt::Display for Door {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.address {
            Address::Socket(path) => write!(f, "{}", path.display()),
            Address::Tcp(address) => write!(f, "{address}"),
        }
    }
}

/// A blocking connection to the door
#[derive(Debug)]
pub(super) enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Stream {
    pub(super) fn try_clone(&self) -> io::Result<Self> {
        match self {
            Self::Tcp(stream) => stream.try_clone().map(Self::Tcp),
            #[cfg(unix)]
            Self::Unix(stream) => stream.try_clone().map(Self::Unix),
        }
    }

    /// Tell the other end nothing more is coming
    pub(super) fn finish_writing(&self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.shutdown(Shutdown::Write),
            #[cfg(unix)]
            Self::Unix(stream) => stream.shutdown(Shutdown::Write),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buf),
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buf),
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
        }
    }
}

/// The listening end of the door on macOS: a Unix socket
#[cfg(target_os = "macos")]
pub(super) type Listener = tokio::net::UnixListener;
/// The listening end of the door on Linux: loopback TCP
#[cfg(target_os = "linux")]
pub(super) type Listener = tokio::net::TcpListener;
/// A connection the door accepted
#[cfg(target_os = "macos")]
pub(super) type Accepted = tokio::net::UnixStream;
/// A connection the door accepted
#[cfg(target_os = "linux")]
pub(super) type Accepted = tokio::net::TcpStream;

/// The door opened, and what removes it
///
/// Dropping it removes the socket file on macOS. The listener is not here: the
/// server owns it, on the runtime that accepts on it.
#[derive(Debug)]
pub(super) struct Open {
    pub(super) door: Door,
    socket: Option<PathBuf>,
}

impl Drop for Open {
    fn drop(&mut self) {
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// Open the door of a run that keeps its data in `data_dir`
///
/// macOS binds a socket there, under a name kept in a file so the next run binds
/// the same path, and replaces a socket a killed run left. A path past the
/// 104-byte limit goes under the temporary folder instead. The listener is
/// made without a runtime, and registered with one by the caller.
#[cfg(target_os = "macos")]
pub(super) fn open(data_dir: &Path) -> io::Result<(Open, std::os::unix::net::UnixListener)> {
    let name = kept_name(&data_dir.join("tools-door"))?;
    let mut socket = data_dir.join(format!("{name}.sock"));
    if socket.as_os_str().len() > LONGEST_SOCKET {
        socket = env::temp_dir().join(format!("kelpie-{name}.sock"));
    }
    if socket.as_os_str().len() > LONGEST_SOCKET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is {} bytes, longer than the {LONGEST_SOCKET} a socket's path may be",
                socket.display(),
                socket.as_os_str().len()
            ),
        ));
    }
    let _ = fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let door = Door {
        address: Address::Socket(socket.clone()),
    };
    Ok((
        Open {
            door,
            socket: Some(socket),
        },
        listener,
    ))
}

/// Open the door of a run: a port on `127.0.0.1` the OS picks
#[cfg(target_os = "linux")]
pub(super) fn open(_data_dir: &Path) -> io::Result<(Open, std::net::TcpListener)> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let door = Door {
        address: Address::Tcp(listener.local_addr()?),
    };
    Ok((Open { door, socket: None }, listener))
}

/// The name in `file`, if it is 32 hex characters, and else a new one written there
#[cfg(target_os = "macos")]
fn kept_name(file: &Path) -> io::Result<String> {
    if let Ok(text) = fs::read_to_string(file) {
        let name = text.trim();
        if name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(name.to_owned());
        }
    }
    let name = super::hex128()?;
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    fs::write(file, &name)?;
    Ok(name)
}
