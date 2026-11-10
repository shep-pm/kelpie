use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use semver::Version;

use super::*;
use crate::run::RunId;

/// The bridge every test installs
const ID: &str = "claude-acp";
const VERSION: &str = "0.101.1";

/// The command the fixtures hold, and the only one kelpie unpacks
const CMD: &str = "claude-code-acp";

/// The fixture's bridge that is run with a subcommand, as the live index's goose entry is
const GOOSE: &str = "goose";
const GOOSE_VERSION: &str = "1.2.0";
const GOOSE_CMD: &str = "goose";
const ACP: &str = "acp";
const GOOSE_PROVIDER: &str = "GOOSE_PROVIDER";
const OLLAMA: &str = "ollama";

/// How long a fake bridge is given to answer the probe
const ANSWER: Duration = Duration::from_secs(5);

/// What the fake bridge says about why it never answered
const NO_ARG: &str = "the bridge was started without its argument";

/// What the command says, which is how a test tells a whole file from half one
const COMMAND_TEXT: &str = "#!/bin/sh\necho kelpie bridge fixture\n";

/// Every platform kelpie ships on
const PLATFORMS: [Platform; 5] = [
    Platform::LinuxX64,
    Platform::LinuxArm64,
    Platform::MacOsX64,
    Platform::MacOsArm64,
    Platform::WindowsX64,
];

/// The archives the server answers with
const TAR: &str = "bridge.tar.gz";
const ZIP: &str = "bridge.zip";
const GOOSE_TAR: &str = "goose.tar.gz";
const GOOSE_ZIP: &str = "goose.zip";

/// The registry's index, which the server answers with `{{base}}` made its own address
const REGISTRY: &str = "registry.json";

/// A local HTTP server that answers `GET /<name>` with the fixture of that name
///
/// It listens on a port the OS picks, so tests that run side by side never
/// share one, and it counts what it served. Anything else it answers 404.
struct Server {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    /// Start a server that waits `delay` before it answers each request
    fn start(delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (hits, stop) = (hits.clone(), stop.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let hits = hits.clone();
                    thread::spawn(move || answer(stream, addr, delay, &hits));
                }
            })
        };
        Self {
            addr,
            hits,
            stop,
            thread: Some(thread),
        }
    }

    /// The url `name` is served at
    fn url(&self, name: &str) -> String {
        format!("http://{}/{name}", self.addr)
    }

    /// How many fixtures it has sent
    fn served(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read one request from `stream` and answer it
fn answer(mut stream: TcpStream, addr: SocketAddr, delay: Duration, hits: &AtomicUsize) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(&chunk[..read]),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("/");
    thread::sleep(delay);
    let name = path.trim_start_matches('/');
    let reply = match name {
        TAR | ZIP | GOOSE_TAR | GOOSE_ZIP => {
            hits.fetch_add(1, Ordering::SeqCst);
            let body = fs::read(fixture_path(name)).unwrap();
            let mut reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            reply.extend_from_slice(&body);
            reply
        }
        REGISTRY => {
            let body = fs::read_to_string(fixture_path(name))
                .unwrap()
                .replace("{{base}}", &format!("http://{addr}"));
            let mut reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            reply.extend_from_slice(body.as_bytes());
            reply
        }
        _ => b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
    };
    let _ = stream.write_all(&reply);
}

/// A data dir of its own for one test, never written over by another
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-bridge-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn data(&self) -> &Path {
        &self.0
    }

    /// The folder one pinned version installs into
    fn version(&self) -> PathBuf {
        self.0.join("bridges").join(ID).join(VERSION)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A fixture's path in the crate
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The archive `name` the server serves, with the sha256 the fixture really has
fn download(server: &Server, name: &str) -> Download {
    Download {
        url: server.url(name),
        sha256: hash_file(&fixture_path(name)).unwrap(),
        cmd: CMD.to_string(),
        args: Vec::new(),
        env: BTreeMap::new(),
    }
}

/// A download for a test that never fetches it, so it needs no server
fn unfetched(name: &str) -> Download {
    Download {
        url: format!("http://127.0.0.1:1/{name}"),
        sha256: hash_file(&fixture_path(name)).unwrap(),
        cmd: CMD.to_string(),
        args: Vec::new(),
        env: BTreeMap::new(),
    }
}

/// The one platform this test runs on, with the fixture `name` as its archive
fn platforms_for(server: &Server, name: &str) -> [(Platform, Download); 1] {
    [(this_platform(), download(server, name))]
}

/// The platform these tests run on
fn this_platform() -> Platform {
    Platform::current().expect("a platform kelpie ships on")
}

/// A pin over the given platforms
fn pin(platforms: &[(Platform, Download)]) -> Pin<'_> {
    Pin {
        id: ID,
        registry_id: ID,
        version: VERSION,
        platforms,
    }
}

/// The names inside a folder, sorted
fn entries(dir: &Path) -> Vec<String> {
    let mut names = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<String>>();
    names.sort();
    names
}

/// A version that is not kelpie's pin
fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

mod index;
mod install;
mod probing;
mod updates;
mod versions;
