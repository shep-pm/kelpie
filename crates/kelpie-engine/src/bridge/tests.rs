use std::fs;
use std::io::Read as _;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use super::*;
use crate::run::RunId;

/// The bridge every test installs
const ID: &str = "claude-acp";
const VERSION: &str = "0.101.1";

/// The command the fixtures hold, and the only one kelpie unpacks
const CMD: &str = "claude-code-acp";

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

/// The two archives the server answers with
const TAR: &str = "bridge.tar.gz";
const ZIP: &str = "bridge.zip";

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
        TAR | ZIP => {
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
    }
}

/// A download for a test that never fetches it, so it needs no server
fn unfetched(name: &str) -> Download {
    Download {
        url: format!("http://127.0.0.1:1/{name}"),
        sha256: hash_file(&fixture_path(name)).unwrap(),
        cmd: CMD.to_string(),
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

#[test]
fn a_bridge_installs_once_and_is_reused() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);

    let first = ensure(dir.data(), &pin(&platforms)).unwrap();
    let second = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(first, second);
    assert_eq!(server.served(), 1, "the second call fetched again");
    assert_eq!(first, dir.version().join(CMD));
    assert_eq!(fs::read_to_string(&first).unwrap(), COMMAND_TEXT);
    assert_eq!(
        fs::read_to_string(record_path(&first)).unwrap().trim(),
        hash_file(&first).unwrap()
    );

    // A pin whose archive would now hash wrong still reuses what is installed,
    // which is the same call answering without fetching anything
    let mut changed = download(&server, TAR);
    changed.sha256 = "0".repeat(64);
    let reused = ensure(dir.data(), &pin(&[(this_platform(), changed)])).unwrap();
    assert_eq!(reused, first);
    assert_eq!(server.served(), 1);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&first).unwrap().permissions().mode();
        assert_ne!(mode & 0o111, 0, "{first:?} is not runnable");
    }
}

#[test]
fn a_zip_archive_installs_too() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, ZIP);

    let installed = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(installed, dir.version().join(CMD));
    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
}

#[test]
fn a_checksum_mismatch_installs_nothing() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let mut archive = download(&server, TAR);
    archive.sha256 = "0".repeat(64);

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(
        matches!(error, BridgeError::ChecksumMismatch { .. }),
        "{error}"
    );
    let text = error.to_string();
    assert!(text.contains(&"0".repeat(64)), "no expected sum in {text}");
    assert!(
        text.contains(&hash_file(&fixture_path(TAR)).unwrap()),
        "no actual sum in {text}"
    );
    assert_eq!(
        entries(&dir.version()),
        Vec::<String>::new(),
        "it installed anyway"
    );
}

#[test]
fn only_the_command_is_unpacked() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);

    let installed = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
    assert_eq!(
        entries(&dir.version()),
        [CMD.to_string(), format!("{CMD}.sha256")]
    );
}

#[test]
fn an_interrupted_install_is_not_used() {
    let dir = Dir::new();
    let version = dir.version();
    fs::create_dir_all(&version).unwrap();
    // What a run that died mid-install leaves behind: half a command, and the
    // folder it was being staged in
    fs::write(version.join(CMD), "#!/bin/sh\necho hal").unwrap();
    fs::create_dir_all(version.join(".install-stale")).unwrap();
    fs::write(version.join(".install-stale").join(CMD), "half an archive").unwrap();

    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);
    let installed = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(installed, version.join(CMD));
    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
    assert_eq!(
        entries(&version),
        [CMD.to_string(), format!("{CMD}.sha256")],
        "the staged folder was left beside it"
    );
}

#[test]
fn installs_that_overlap_fetch_once_and_both_succeed() {
    let dir = Dir::new();
    // Slow enough that the second call starts while the first is downloading
    let server = Server::start(Duration::from_millis(300));
    let platforms = platforms_for(&server, TAR);

    let results = thread::scope(|scope| {
        let first = scope.spawn(|| ensure(dir.data(), &pin(&platforms)));
        thread::sleep(Duration::from_millis(100));
        let second = scope.spawn(|| ensure(dir.data(), &pin(&platforms)));
        [first.join().unwrap(), second.join().unwrap()]
    });

    for result in results {
        let installed = result.unwrap();
        assert_eq!(fs::read_to_string(installed).unwrap(), COMMAND_TEXT);
    }
    assert_eq!(server.served(), 1, "the overlapping call fetched again");
    assert_eq!(
        entries(&dir.version()),
        [CMD.to_string(), format!("{CMD}.sha256")]
    );
}

#[test]
fn a_platform_with_no_download_says_so() {
    let dir = Dir::new();
    let other = PLATFORMS
        .into_iter()
        .find(|platform| *platform != this_platform())
        .unwrap();

    let error = ensure(dir.data(), &pin(&[(other, unfetched(TAR))])).unwrap_err();

    assert!(matches!(error, BridgeError::NoDownload { .. }), "{error}");
    assert!(
        error.to_string().contains(&format!("{ID} {VERSION}")),
        "{error}"
    );
    assert!(!dir.data().join("bridges").exists());
}

#[test]
fn an_archive_format_kelpie_does_not_unpack_says_so() {
    let dir = Dir::new();
    let mut archive = unfetched(TAR);
    archive.url = "http://127.0.0.1:1/bridge.tar.xz".into();
    archive.sha256 = "0".repeat(64);

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(
        matches!(error, BridgeError::ArchiveFormat { .. }),
        "{error}"
    );
    assert!(error.to_string().contains(".tar.xz"), "{error}");
}

#[test]
fn an_archive_without_the_command_says_so() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let mut archive = download(&server, TAR);
    archive.cmd = "not-in-the-archive".into();

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(
        matches!(error, BridgeError::MissingCommand { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("not-in-the-archive"), "{error}");
    assert_eq!(entries(&dir.version()), Vec::<String>::new());
}

#[test]
fn a_pin_that_names_a_path_outside_the_bridges_folder_is_refused() {
    let dir = Dir::new();
    let mut archive = unfetched(TAR);
    archive.cmd = "../escape".into();

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(matches!(error, BridgeError::PinNotUsable { .. }), "{error}");
    assert!(error.to_string().contains("../escape"), "{error}");
    assert!(!dir.data().join("bridges").exists());
}

#[test]
fn the_index_gives_a_pin_for_the_stable_version() {
    let server = Server::start(Duration::ZERO);

    let index = registry::fetch(&server.url(REGISTRY)).unwrap();

    let pin = index.bridge(ID).expect("claude-acp is in the index");
    assert_eq!((pin.id, pin.registry_id, pin.version), (ID, ID, "0.102.0"));
    assert_eq!(pin.platforms.len(), PLATFORMS.len());
    let (_, arm_mac) = pin
        .platforms
        .iter()
        .find(|(platform, _)| *platform == Platform::MacOsArm64)
        .expect("the index has macOS on arm64");
    assert_eq!(
        *arm_mac,
        Download {
            url: server.url(ZIP),
            sha256: hash_file(&fixture_path(ZIP)).unwrap(),
            cmd: CMD.to_string(),
        },
        "the command is the file's name, without the registry's ./"
    );
    assert!(
        index.bridge("codex-acp").is_none(),
        "an agent with no binary distribution has no pin"
    );
}
