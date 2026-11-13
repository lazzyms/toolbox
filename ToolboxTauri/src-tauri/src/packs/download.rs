//! Installing a pack from the release server. The only pack code that uses the network.
//!
//! Where a pack is downloaded from comes only from the built-in catalog: the payload's `path`
//! joined to the release server set at build time. Requests use HTTPS and follow redirects only to
//! that server's own download hosts. Before anything is downloaded, the disk holding app data must
//! have room for the download and the installed pack together. The file is streamed into
//! `pack-downloads/` in app data and then installed by [`install::install`], the same
//! verify-then-install path as a pack file the person chose. The download is deleted either way.
//!
//! Installs run one at a time, in the order they were asked for. A queued install can be
//! cancelled without touching the others. There are no automatic retries: a failure comes back as
//! a typed error, and trying again starts from nothing.

use super::install::{self, remove_tree, unavailable, InstallContext, PackInstallError, PackInstallErrorKind, PackLimit};
use super::{list_packs_in, load_catalog, offered_payload, CatalogPack, PackList, Payload};
use crate::kit::common::batch_runner::RunCancel;
use crate::kit::common::limits::CancelFlag;
use futures_util::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::Url;
use serde::Serialize;
use std::future::Future;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

pub const PROGRESS_EVENT: &str = "pack-progress";
/// Private folder in app data for the file being downloaded. Separate from install staging,
/// which the install clears before it starts.
const DOWNLOADS_DIR: &str = "pack-downloads";
/// The release server. Catalog paths are relative to it.
const RELEASE_BASE: &str = "https://github.com/lazzyms/toolbox/releases/download/";
/// Hosts the release server hands its downloads to.
const RELEASE_REDIRECT_HOSTS: &[&str] = &["objects.githubusercontent.com", "release-assets.githubusercontent.com"];
/// Development override: the server to download packs from instead, for a local stand-in. Release
/// builds ignore it.
#[cfg(debug_assertions)]
const SERVER_OVERRIDE_ENV: &str = "TOOLBOX_PACK_SERVER";
/// How often a wait looks at its cancel flag.
const TICK: Duration = Duration::from_millis(50);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const MAX_REDIRECTS: usize = 10;

type Result<T> = std::result::Result<T, PackInstallError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Waiting for the installs asked for earlier.
    Queued,
    Downloading,
    /// Downloaded. Checking and installing it.
    Verifying,
}

/// The `pack-progress` event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress<'a> {
    pub run_id: &'a str,
    pub pack_id: &'a str,
    pub phase: Phase,
    pub received_bytes: u64,
    pub total_bytes: u64,
}

/// Where packs come from and how the network and disk are reached.
pub struct Transport {
    /// Ends in `/`. Catalog paths are joined to it.
    pub base: Url,
    /// Hosts a redirect may lead to, beside the base's own.
    pub redirect_hosts: Vec<String>,
    pub resolver: Arc<dyn Resolve>,
    /// How long the server has to start answering.
    pub answer_timeout: Duration,
    /// How long a started download may go without receiving anything.
    pub stall_timeout: Duration,
    pub free_space: fn(&Path) -> io::Result<u64>,
}

impl Transport {
    fn release() -> std::result::Result<Self, String> {
        #[allow(unused_mut)]
        let (mut base, mut redirect_hosts) = (RELEASE_BASE.to_string(), RELEASE_REDIRECT_HOSTS.iter().map(|host| host.to_string()).collect());
        #[cfg(debug_assertions)]
        if let Ok(server) = std::env::var(SERVER_OVERRIDE_ENV) {
            (base, redirect_hosts) = (server, Vec::new());
        }
        let base = Url::parse(&base).map_err(|error| format!("The pack server address is not valid: {error}"))?;
        Ok(Self { base, redirect_hosts, resolver: Arc::new(SystemResolver), answer_timeout: Duration::from_secs(30), stall_timeout: Duration::from_secs(30), free_space })
    }
}

/// The install queue and the client it downloads with.
pub struct Downloads {
    base: Url,
    client: reqwest::Client,
    answer_timeout: Duration,
    stall_timeout: Duration,
    free_space: fn(&Path) -> io::Result<u64>,
    /// Held for a whole install. Tokio's mutex hands out turns in the order they were asked for.
    turn: tokio::sync::Mutex<()>,
}

static DOWNLOADS: LazyLock<std::result::Result<Downloads, String>> = LazyLock::new(|| Downloads::new(Transport::release()?));

/// Downloads and installs a catalog pack and returns the refreshed pack list. `cancel_run` with
/// the same run id cancels it, whether it is queued, downloading or being checked.
#[tauri::command]
pub async fn install_pack(app: tauri::AppHandle, pack_id: String, run_id: String) -> Result<PackList> {
    let app_data = super::app_data(&app).map_err(unavailable)?;
    let app_version = app.package_info().version.to_string();
    let cancel = RunCancel::register(run_id.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let downloads = DOWNLOADS.as_ref().map_err(unavailable)?;
        let catalog = load_catalog().map_err(unavailable)?;
        let public_key = install::trusted_public_key().map_err(unavailable)?;
        let target = super::current_target();
        let changed = |pack_id: &str| install::emit_changed(&app, pack_id);
        let context = InstallContext { catalog: &catalog, app_data: &app_data, target: &target, app_version: &app_version, public_key: &public_key, limits: install::PackLimits::DEFAULT, cancel: Some(cancel.flag()), changed: Some(&changed) };
        downloads.install(&pack_id, &context, &|phase, received_bytes, total_bytes| {
            use tauri::Emitter;
            let _ = app.emit(PROGRESS_EVENT, Progress { run_id: &run_id, pack_id: &pack_id, phase, received_bytes, total_bytes });
        })?;
        Ok(list_packs_in(&catalog, &app_data, &target, &app_version))
    }).await.map_err(unavailable)?
}

impl Downloads {
    pub fn new(transport: Transport) -> std::result::Result<Self, String> {
        let Transport { base, redirect_hosts, resolver, answer_timeout, stall_timeout, free_space } = transport;
        if !base.path().ends_with('/') || base.query().is_some() {
            return Err(format!("The pack server address must end in /: {base}"));
        }
        let host = base.host_str().ok_or("The pack server address has no host")?.to_string();
        if cfg!(not(debug_assertions)) && base.scheme() != "https" {
            return Err("The pack server must use HTTPS".into());
        }
        // The same TLS provider the updater installs; whichever runs first sets it.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let scheme = base.scheme().to_string();
        let hosts: Vec<String> = std::iter::once(host.clone()).chain(redirect_hosts).collect();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            let allowed = attempt.url().scheme() == scheme && attempt.url().host_str().is_some_and(|host| hosts.iter().any(|allowed| allowed == host));
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if allowed {
                attempt.follow()
            } else {
                let refused = format!("a redirect to {} is not a Toolbox download host", attempt.url());
                attempt.error(refused)
            }
        });
        let mut builder = reqwest::Client::builder()
            .https_only(base.scheme() == "https")
            .redirect(policy)
            .dns_resolver(resolver)
            .connect_timeout(answer_timeout);
        // A local stand-in server is never reached through a proxy.
        if host.parse::<std::net::IpAddr>().is_ok_and(|address| address.is_loopback()) {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|error| error.to_string())?;
        Ok(Self { base, client, answer_timeout, stall_timeout, free_space, turn: tokio::sync::Mutex::new(()) })
    }

    /// Waits for its turn, checks free space, downloads the pack the catalog lists for this target
    /// and installs it through [`install::install`]. Blocking: call it off the async runtime.
    pub fn install(&self, pack_id: &str, context: &InstallContext, progress: &(dyn Fn(Phase, u64, u64) + Sync)) -> Result<()> {
        let never = CancelFlag::default();
        let cancel = context.cancel.unwrap_or(&never);
        let (pack, payload) = offered(context, pack_id)?;
        let url = self.base.join(&payload.path).map_err(|error| PackInstallError::new(PackInstallErrorKind::NotInCatalog, error.to_string()))?;
        let _turn = tauri::async_runtime::block_on(async {
            match self.turn.try_lock() {
                Ok(turn) => Ok(turn),
                Err(_) => {
                    progress(Phase::Queued, 0, payload.download_bytes);
                    match wait(self.turn.lock(), cancel, Duration::MAX).await {
                        Waited::Done(turn) => Ok(turn),
                        _ => Err(cancelled()),
                    }
                }
            }
        })?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }

        let folder = context.app_data.join(DOWNLOADS_DIR);
        // A crash during an earlier download can leave a file behind.
        remove_tree(&folder).and_then(|()| std::fs::create_dir_all(&folder)).map_err(unavailable)?;
        let needed_bytes = payload.download_bytes.saturating_add(payload.installed_bytes);
        let result = match (self.free_space)(context.app_data) {
            Err(error) => Err(unavailable(error)),
            Ok(free_bytes) if free_bytes < needed_bytes => Err(PackInstallError::new(
                PackInstallErrorKind::NotEnoughSpace { needed_bytes, free_bytes },
                format!("{} needs {needed_bytes} bytes free and the disk has {free_bytes}", pack.id),
            )),
            Ok(_) => {
                let file = folder.join(format!("{}-{}.toolboxpack", pack.id, pack.version));
                let limits = &context.limits;
                let cap = limits.archive_bytes.saturating_add(limits.metadata_bytes.saturating_mul(3));
                tauri::async_runtime::block_on(self.download(url, &file, payload.download_bytes, cap, cancel, progress)).and_then(|received| {
                    progress(Phase::Verifying, received, received);
                    install::install(&file, context)
                })
            }
        };
        let cleaned = remove_tree(&folder);
        result?;
        cleaned.map_err(unavailable)
    }

    /// Streams `url` into `file` and returns how many bytes it received.
    async fn download(&self, url: Url, file: &Path, expected: u64, cap: u64, cancel: &CancelFlag, progress: &(dyn Fn(Phase, u64, u64) + Sync)) -> Result<u64> {
        let response = match wait(self.client.get(url).send(), cancel, self.answer_timeout).await {
            Waited::Cancelled => return Err(cancelled()),
            Waited::TimedOut => return Err(PackInstallError::new(PackInstallErrorKind::ServerBlocked, "The release server did not answer")),
            Waited::Done(Err(error)) => return Err(network_error(&error, false)),
            Waited::Done(Ok(response)) => response,
        };
        if !response.status().is_success() {
            return Err(PackInstallError::new(PackInstallErrorKind::ServerBlocked, format!("The release server answered {}", response.status())));
        }
        let total = response.content_length().unwrap_or(expected);
        if total > cap {
            return Err(over_cap());
        }
        let mut output = std::fs::File::create_new(file).map_err(unavailable)?;
        let mut stream = response.bytes_stream();
        let mut received = 0u64;
        let mut reported = Instant::now();
        progress(Phase::Downloading, 0, total);
        loop {
            let chunk = match wait(stream.next(), cancel, self.stall_timeout).await {
                Waited::Cancelled => return Err(cancelled()),
                Waited::TimedOut => return Err(PackInstallError::new(PackInstallErrorKind::Interrupted, "The download stalled")),
                Waited::Done(None) => break,
                Waited::Done(Some(Err(error))) => return Err(network_error(&error, true)),
                Waited::Done(Some(Ok(chunk))) => chunk,
            };
            received += chunk.len() as u64;
            if received > cap {
                return Err(over_cap());
            }
            output.write_all(&chunk).map_err(unavailable)?;
            if reported.elapsed() >= PROGRESS_INTERVAL {
                reported = Instant::now();
                progress(Phase::Downloading, received, total.max(received));
            }
        }
        if received < total {
            return Err(PackInstallError::new(PackInstallErrorKind::Interrupted, format!("The download ended after {received} of {total} bytes")));
        }
        output.sync_all().map_err(unavailable)?;
        progress(Phase::Downloading, received, received);
        Ok(received)
    }
}

/// The catalog pack and payload this app offers for `pack_id`: the listing's own choice.
fn offered<'a>(context: &InstallContext<'a>, pack_id: &str) -> Result<(&'a CatalogPack, &'a Payload)> {
    context.catalog.packs.iter().find(|pack| pack.id == pack_id)
        .and_then(|pack| Some((pack, offered_payload(pack, context.target, context.app_version)?)))
        .ok_or_else(|| PackInstallError::new(PackInstallErrorKind::NotInCatalog, format!("The catalog offers no pack {pack_id:?} for this app and target")))
}

fn cancelled() -> PackInstallError {
    PackInstallError::new(PackInstallErrorKind::Cancelled, "The install was cancelled")
}

fn over_cap() -> PackInstallError {
    PackInstallError::new(PackInstallErrorKind::OverLimit { limit: PackLimit::ArchiveSize }, "The download is larger than a pack file can be")
}

/// Maps a client error to the person's situation: no network, a server that cannot be used, or a
/// download that broke off after it started.
fn network_error(error: &reqwest::Error, downloading: bool) -> PackInstallError {
    let mut causes = std::iter::successors(Some(error as &(dyn std::error::Error + 'static)), |cause| cause.source());
    let offline = causes.any(|cause| cause.is::<NameNotFound>() || cause.downcast_ref::<io::Error>().is_some_and(|error| {
        matches!(error.kind(), io::ErrorKind::NetworkUnreachable | io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkDown)
    }));
    let kind = if offline {
        PackInstallErrorKind::Offline
    } else if downloading {
        PackInstallErrorKind::Interrupted
    } else {
        PackInstallErrorKind::ServerBlocked
    };
    PackInstallError::new(kind, format!("{error:#}"))
}

enum Waited<T> {
    Done(T),
    Cancelled,
    TimedOut,
}

/// Runs `future` until it finishes, `cancel` is set, or `limit` passes. The future stays alive
/// between checks, so a queued turn keeps its place.
async fn wait<F: Future>(future: F, cancel: &CancelFlag, limit: Duration) -> Waited<F::Output> {
    let mut future = std::pin::pin!(future);
    let started = Instant::now();
    loop {
        if cancel.is_cancelled() {
            return Waited::Cancelled;
        }
        match tokio::time::timeout(TICK, future.as_mut()).await {
            Ok(output) => return Waited::Done(output),
            Err(_) if started.elapsed() >= limit => return Waited::TimedOut,
            Err(_) => {}
        }
    }
}

/// The name lookup failed, which is how being offline usually shows.
#[derive(Debug)]
pub struct NameNotFound(pub String);

impl std::fmt::Display for NameNotFound {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "{} could not be looked up", self.0)
    }
}

impl std::error::Error for NameNotFound {}

/// The operating system's resolver, with failures marked so they read as offline.
struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let lookup = host.clone();
            let found = tauri::async_runtime::spawn_blocking(move || std::net::ToSocketAddrs::to_socket_addrs(&(lookup.as_str(), 0)).map(Iterator::collect::<Vec<_>>)).await;
            match found {
                Ok(Ok(addresses)) if !addresses.is_empty() => Ok(Box::new(addresses.into_iter()) as Addrs),
                _ => Err(Box::new(NameNotFound(host)) as Box<dyn std::error::Error + Send + Sync>),
            }
        })
    }
}

/// Bytes the current user can still write on the disk holding `path`.
#[cfg(unix)]
fn free_space(path: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: statvfs only writes into the zeroed struct it is given.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[allow(clippy::unnecessary_cast)]
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(windows)]
fn free_space(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: the path is NUL-terminated and the totals it does not need are null.
    let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(available)
}

#[cfg(test)]
mod tests {
    use super::super::install::tests::{sandbox, Fixture, TestKey};
    use super::super::install::{digest_hex, PackLimits};
    use super::super::{current_target, parse_catalog, Catalog, PackStatus, PACKS_DIR};
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Mutex};

    const APP: &str = "1.0.14";

    /// What the stand-in server answers for one path.
    #[derive(Clone)]
    enum Reply {
        File(Vec<u8>),
        /// Sends the headers and half the body, then waits until the gate opens before the rest.
        Gated(Vec<u8>, Arc<AtomicBool>),
        /// Promises the whole file and closes the connection halfway.
        CutShort(Vec<u8>),
        Status(u16),
        Redirect(String),
    }

    /// A local HTTP server standing in for the release server. It records the paths asked for.
    struct Server {
        address: SocketAddr,
        routes: Arc<Mutex<HashMap<String, Reply>>>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let routes: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
            let requests: Arc<Mutex<Vec<String>>> = Arc::default();
            let (shared_routes, shared_requests) = (routes.clone(), requests.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (routes, requests) = (shared_routes.clone(), shared_requests.clone());
                    std::thread::spawn(move || {
                        let _ = Self::answer(stream, &routes, &requests);
                    });
                }
            });
            Self { address, routes, requests }
        }

        fn answer(mut stream: TcpStream, routes: &Mutex<HashMap<String, Reply>>, requests: &Mutex<Vec<String>>) -> io::Result<()> {
            let mut reader = BufReader::new(stream.try_clone()?);
            let mut request_line = String::new();
            reader.read_line(&mut request_line)?;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line)? <= 2 {
                    break;
                }
            }
            let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();
            requests.lock().unwrap().push(path.clone());
            let reply = routes.lock().unwrap().get(&path).cloned().unwrap_or(Reply::Status(404));
            let head = |status: &str, length: usize| format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
            match reply {
                Reply::File(bytes) => {
                    stream.write_all(head("200 OK", bytes.len()).as_bytes())?;
                    stream.write_all(&bytes)?;
                }
                Reply::Gated(bytes, gate) => {
                    stream.write_all(head("200 OK", bytes.len()).as_bytes())?;
                    let half = bytes.len() / 2;
                    stream.write_all(&bytes[..half])?;
                    stream.flush()?;
                    while !gate.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    stream.write_all(&bytes[half..])?;
                }
                Reply::CutShort(bytes) => {
                    stream.write_all(head("200 OK", bytes.len()).as_bytes())?;
                    stream.write_all(&bytes[..bytes.len() / 2])?;
                }
                Reply::Status(code) => stream.write_all(head(&format!("{code} Refused"), 0).as_bytes())?,
                Reply::Redirect(location) => stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())?,
            }
            stream.flush()
        }

        fn route(&self, path: &str, reply: Reply) {
            self.routes.lock().unwrap().insert(path.to_string(), reply);
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        fn base(&self) -> Url {
            Url::parse(&format!("http://{}/releases/", self.address)).unwrap()
        }
    }

    fn plenty(_: &Path) -> io::Result<u64> {
        Ok(u64::MAX)
    }

    fn transport(base: Url) -> Transport {
        Transport { base, redirect_hosts: Vec::new(), resolver: Arc::new(SystemResolver), answer_timeout: Duration::from_secs(5), stall_timeout: Duration::from_secs(5), free_space: plenty }
    }

    fn pack_path(id: &str) -> String {
        format!("packs/{id}-2.0/{id}-2.0.toolboxpack")
    }

    /// Packs built in code, a catalog that offers them, and an app-data folder to install into.
    struct Setup {
        app_data: PathBuf,
        catalog: Catalog,
        key: TestKey,
        files: HashMap<String, Vec<u8>>,
        /// The pack ids installs reported as changed, in order.
        changed: std::sync::Mutex<Vec<String>>,
    }

    impl Setup {
        fn new(name: &str, ids: &[&str]) -> Self {
            let key = TestKey::new(7);
            let mut files = HashMap::new();
            let mut packs = Vec::new();
            for id in ids {
                let mut fixture = Fixture::new();
                fixture.id = id.to_string();
                let file = fixture.file(&key);
                packs.push(serde_json::json!({
                    "id": id, "name": format!("Pack {id}"), "description": "d", "toolIds": ["pdf-ocr"], "license": "Apache-2.0",
                    "version": fixture.version, "minPackFormat": 1, "appVersions": { "from": "1.0.0" },
                    "payloads": [{ "target": current_target(), "downloadBytes": file.len(), "installedBytes": 4096, "sha256": digest_hex(&fixture.payload()), "path": pack_path(id) }]
                }));
                files.insert(id.to_string(), file);
            }
            let catalog = parse_catalog(&serde_json::json!({ "formatVersion": 1, "packs": packs }).to_string()).unwrap();
            Self { app_data: sandbox(&format!("download-{name}")).join("app-data"), catalog, key, files, changed: Default::default() }
        }

        fn file(&self, id: &str) -> Vec<u8> {
            self.files[id].clone()
        }

        fn serve(&self, server: &Server) {
            for (id, bytes) in &self.files {
                server.route(&format!("/releases/{}", pack_path(id)), Reply::File(bytes.clone()));
            }
        }

        /// Installs `id` and records every progress event it sends.
        fn install(&self, downloads: &Downloads, id: &str, cancel: &CancelFlag, events: &(dyn Fn(Phase, u64, u64) + Sync)) -> Result<()> {
            let target = current_target();
            let public_key = self.key.public_key();
            let changed = |pack_id: &str| self.changed.lock().unwrap().push(pack_id.to_string());
            let context = InstallContext { catalog: &self.catalog, app_data: &self.app_data, target: &target, app_version: APP, public_key: &public_key, limits: PackLimits::DEFAULT, cancel: Some(cancel), changed: Some(&changed) };
            downloads.install(id, &context, events)
        }

        /// How many times installs reported `id` as changed.
        fn changes(&self, id: &str) -> usize {
            self.changed.lock().unwrap().iter().filter(|changed| *changed == id).count()
        }

        fn status(&self, id: &str) -> PackStatus {
            list_packs_in(&self.catalog, &self.app_data, &current_target(), APP).packs.into_iter().find(|pack| pack.id == id).unwrap().status
        }

        /// Installs version 1.0 of `id` from a pack file, so the catalog's 2.0 is an update.
        fn install_old(&self, id: &str) {
            let mut old = Fixture::new();
            old.id = id.to_string();
            old.version = "1.0".into();
            let file = self.app_data.with_file_name(format!("{id}-1.0.toolboxpack"));
            std::fs::write(&file, old.file(&self.key)).unwrap();
            let (catalog, target, public_key) = (old.catalog(), current_target(), self.key.public_key());
            install::install(&file, &InstallContext { catalog: &catalog, app_data: &self.app_data, target: &target, app_version: APP, public_key: &public_key, limits: PackLimits::DEFAULT, cancel: None, changed: None }).unwrap();
        }

        fn installed_version(&self, id: &str) -> Option<String> {
            list_packs_in(&self.catalog, &self.app_data, &current_target(), APP).packs.into_iter().find(|pack| pack.id == id).unwrap().installed_version
        }

        /// The version folders in the pack's folder.
        fn version_folders(&self, id: &str) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.app_data.join(PACKS_DIR).join(id)).unwrap().flatten()
                .filter(|entry| entry.file_type().unwrap().is_dir())
                .map(|entry| entry.file_name().into_string().unwrap()).collect();
            names.sort();
            names
        }

        /// Nothing in install staging and no downloaded file left.
        fn assert_no_staging(&self, case: &str) {
            assert!(!self.app_data.join("pack-staging").exists(), "{case}: staging was left");
            assert!(!self.app_data.join(DOWNLOADS_DIR).exists(), "{case}: the download was left");
        }

        /// No visible pack, nothing in install staging and no downloaded file left.
        fn assert_clean(&self, id: &str, case: &str) {
            assert!(!self.app_data.join(PACKS_DIR).join(id).exists(), "{case}: a pack folder was left");
            assert!(!self.app_data.join("pack-staging").exists(), "{case}: staging was left");
            assert!(!self.app_data.join(DOWNLOADS_DIR).exists(), "{case}: the download was left");
            assert_eq!(self.status(id), PackStatus::NotInstalled, "{case}");
        }
    }

    type Events = Mutex<Vec<(Phase, u64, u64)>>;

    fn recorder(events: &Events) -> impl Fn(Phase, u64, u64) + Sync + '_ {
        move |phase, received, total| events.lock().unwrap().push((phase, received, total))
    }

    fn kind(result: Result<()>) -> PackInstallErrorKind {
        result.expect_err("the install should fail").kind
    }

    #[test]
    fn a_download_from_a_stand_in_server_installs_through_the_verified_path_with_well_formed_progress() {
        let setup = Setup::new("installs", &["ocr-hindi"]);
        let server = Server::start();
        setup.serve(&server);
        let downloads = Downloads::new(transport(server.base())).unwrap();
        let events = Events::default();
        setup.install(&downloads, "ocr-hindi", &CancelFlag::default(), &recorder(&events)).unwrap();

        assert_eq!(server.requests(), [format!("/releases/{}", pack_path("ocr-hindi"))]);
        assert_eq!(setup.status("ocr-hindi"), PackStatus::Installed);
        assert!(!setup.app_data.join(DOWNLOADS_DIR).exists() && !setup.app_data.join("pack-staging").exists());
        let events = events.into_inner().unwrap();
        let size = setup.file("ocr-hindi").len() as u64;
        assert_eq!(events.first(), Some(&(Phase::Downloading, 0, size)));
        assert_eq!(events.last(), Some(&(Phase::Verifying, size, size)));
        assert_eq!(events.iter().filter(|(phase, ..)| *phase == Phase::Verifying).count(), 1);
        assert!(events.iter().all(|(_, received, total)| received <= total && *total == size));
        assert!(events.windows(2).all(|pair| pair[0].1 <= pair[1].1), "progress only moves forward: {events:?}");
        assert!(events.contains(&(Phase::Downloading, size, size)));
        // The event payload the UI reads, tied to one run id.
        let payload = serde_json::to_value(Progress { run_id: "run-1", pack_id: "ocr-hindi", phase: Phase::Downloading, received_bytes: 5, total_bytes: size }).unwrap();
        assert_eq!(payload, serde_json::json!({ "runId": "run-1", "packId": "ocr-hindi", "phase": "downloading", "receivedBytes": 5, "totalBytes": size }));
    }

    #[test]
    fn repair_downloads_the_pack_again_and_a_failed_repair_leaves_it_damaged_with_nothing_left_over() {
        use super::super::uninstall::{resolve_pack_resource, PackHolds, RequiredPack};
        const MODEL: &str = "tessdata/hin.traineddata";
        let setup = Setup::new("repair", &["ocr-hindi"]);
        let server = Server::start();
        setup.serve(&server);
        let downloads = Downloads::new(transport(server.base())).unwrap();
        let attempt = || setup.install(&downloads, "ocr-hindi", &CancelFlag::default(), &|_, _, _| {});
        attempt().unwrap();
        assert_eq!(setup.changes("ocr-hindi"), 1, "the install reports the pack changed");
        let model = setup.app_data.join(PACKS_DIR).join("ocr-hindi/2.0").join(MODEL);
        let original = std::fs::read(&model).unwrap();
        let mut changed = original.clone();
        changed[0] ^= 1;
        std::fs::write(&model, changed).unwrap();
        let resolve = || resolve_pack_resource(MODEL, &setup.app_data, RequiredPack { id: "ocr-hindi", name: "Hindi" }, APP, None, None, &mut PackHolds::default());
        assert!(resolve().is_err());
        assert_eq!(setup.status("ocr-hindi"), PackStatus::Damaged);

        let path = format!("/releases/{}", pack_path("ocr-hindi"));
        let mut corrupt = setup.file("ocr-hindi");
        let middle = corrupt.len() / 2;
        corrupt[middle] ^= 0xff;
        for (case, reply) in [("server error", Reply::Status(500)), ("corrupt download", Reply::File(corrupt)), ("cut short", Reply::CutShort(setup.file("ocr-hindi")))] {
            server.route(&path, reply);
            assert!(attempt().is_err(), "{case}");
            assert_eq!(setup.status("ocr-hindi"), PackStatus::Damaged, "{case}");
            assert!(resolve().is_err(), "{case}");
            assert!(!setup.app_data.join("pack-staging").exists() && !setup.app_data.join(DOWNLOADS_DIR).exists(), "{case}: files were left");
            assert_eq!(setup.changes("ocr-hindi"), 1, "{case}: a failed repair changes nothing");
        }

        setup.serve(&server);
        attempt().unwrap();
        assert_eq!(setup.changes("ocr-hindi"), 2, "the repair reports the pack changed");
        assert_eq!(setup.status("ocr-hindi"), PackStatus::Installed);
        assert!(!setup.app_data.join("pack-staging").exists() && !setup.app_data.join(DOWNLOADS_DIR).exists());
        let resolved = resolve().unwrap();
        assert_eq!(std::fs::read(resolved.path).unwrap(), original);
    }

    const MODEL: &str = "tessdata/hin.traineddata";

    fn resolve(setup: &Setup, run: &mut super::super::uninstall::PackHolds) -> std::result::Result<PathBuf, crate::kit::contracts::ToolError> {
        super::super::uninstall::resolve_pack_resource(MODEL, &setup.app_data, super::super::uninstall::RequiredPack { id: "ocr-hindi", name: "Hindi" }, APP, None, None, run).map(|resolved| resolved.path)
    }

    #[test]
    fn a_newer_catalog_version_is_an_update_that_downloads_only_when_asked_and_replaces_the_old_version() {
        let setup = Setup::new("update", &["ocr-hindi"]);
        let server = Server::start();
        setup.serve(&server);
        setup.install_old("ocr-hindi");

        assert_eq!((setup.status("ocr-hindi"), setup.installed_version("ocr-hindi").as_deref()), (PackStatus::UpdateAvailable, Some("1.0")));
        assert!(resolve(&setup, &mut Default::default()).unwrap().starts_with(setup.app_data.join("packs/ocr-hindi/1.0")));
        assert!(server.requests().is_empty(), "nothing is downloaded before Update");

        let events = Events::default();
        setup.install(&Downloads::new(transport(server.base())).unwrap(), "ocr-hindi", &CancelFlag::default(), &recorder(&events)).unwrap();
        assert_eq!(server.requests(), [format!("/releases/{}", pack_path("ocr-hindi"))]);
        assert!(!events.into_inner().unwrap().is_empty(), "the update reports its progress");
        assert_eq!(setup.changes("ocr-hindi"), 1, "the update reports the pack changed");
        assert_eq!((setup.status("ocr-hindi"), setup.installed_version("ocr-hindi").as_deref()), (PackStatus::Installed, Some("2.0")));
        assert_eq!(setup.version_folders("ocr-hindi"), ["2.0"], "the old version is removed");
        assert!(resolve(&setup, &mut Default::default()).unwrap().starts_with(setup.app_data.join("packs/ocr-hindi/2.0")));
        setup.assert_no_staging("updated");
    }

    #[test]
    fn a_failed_or_cancelled_update_keeps_the_old_version_usable_and_leaves_nothing_behind() {
        let setup = Setup::new("update-fails", &["ocr-hindi"]);
        let server = Server::start();
        let path = format!("/releases/{}", pack_path("ocr-hindi"));
        let downloads = Downloads::new(transport(server.base())).unwrap();
        setup.install_old("ocr-hindi");
        let old = setup.app_data.join("packs/ocr-hindi/1.0");
        let assert_old_kept = |case: &str| {
            assert_eq!((setup.status("ocr-hindi"), setup.installed_version("ocr-hindi").as_deref()), (PackStatus::UpdateAvailable, Some("1.0")), "{case}");
            assert_eq!(setup.version_folders("ocr-hindi"), ["1.0"], "{case}: no half-installed version");
            let resolved = resolve(&setup, &mut Default::default()).unwrap_or_else(|error| panic!("{case}: {error:?}"));
            assert!(resolved.starts_with(&old), "{case}");
            setup.assert_no_staging(case);
        };

        let mut corrupt = setup.file("ocr-hindi");
        let middle = corrupt.len() / 2;
        corrupt[middle] ^= 0xff;
        for (case, reply) in [("server error", Reply::Status(500)), ("corrupt download", Reply::File(corrupt)), ("cut short", Reply::CutShort(setup.file("ocr-hindi")))] {
            server.route(&path, reply);
            assert!(setup.install(&downloads, "ocr-hindi", &CancelFlag::default(), &|_, _, _| {}).is_err(), "{case}");
            assert_old_kept(case);
        }

        let gate = Arc::new(AtomicBool::new(false));
        server.route(&path, Reply::Gated(setup.file("ocr-hindi"), gate.clone()));
        let cancel = CancelFlag::default();
        let during_download = setup.install(&downloads, "ocr-hindi", &cancel, &|phase, _, _| if phase == Phase::Downloading { cancel.cancel() });
        gate.store(true, Ordering::Release);
        assert_eq!(kind(during_download), PackInstallErrorKind::Cancelled);
        assert_old_kept("cancelled while downloading");

        server.route(&path, Reply::File(setup.file("ocr-hindi")));
        let cancel = CancelFlag::default();
        let during_check = setup.install(&downloads, "ocr-hindi", &cancel, &|phase, _, _| if phase == Phase::Verifying { cancel.cancel() });
        assert_eq!(kind(during_check), PackInstallErrorKind::Cancelled);
        assert_old_kept("cancelled while checking");
        assert_eq!(setup.changes("ocr-hindi"), 0, "failed and cancelled updates change nothing");

        setup.install(&downloads, "ocr-hindi", &CancelFlag::default(), &|_, _, _| {}).unwrap();
        assert_eq!(setup.changes("ocr-hindi"), 1);
        assert_eq!(setup.installed_version("ocr-hindi").as_deref(), Some("2.0"));
    }

    #[test]
    fn a_run_using_the_old_version_keeps_it_through_the_update_and_it_is_removed_when_the_run_ends() {
        let setup = Setup::new("update-held", &["ocr-hindi"]);
        let server = Server::start();
        setup.serve(&server);
        setup.install_old("ocr-hindi");
        let mut running = Default::default();
        let old_model = resolve(&setup, &mut running).unwrap();
        assert!(old_model.starts_with(setup.app_data.join("packs/ocr-hindi/1.0")));

        setup.install(&Downloads::new(transport(server.base())).unwrap(), "ocr-hindi", &CancelFlag::default(), &|_, _, _| {}).unwrap();
        assert_eq!((setup.status("ocr-hindi"), setup.installed_version("ocr-hindi").as_deref()), (PackStatus::Installed, Some("2.0")));
        // The running run still reads the version it started with; a new run gets the new one.
        assert_eq!(setup.version_folders("ocr-hindi"), ["1.0", "2.0"]);
        assert!(std::fs::read(&old_model).is_ok());
        let mut later = Default::default();
        assert!(resolve(&setup, &mut later).unwrap().starts_with(setup.app_data.join("packs/ocr-hindi/2.0")));
        drop(later);
        assert_eq!(setup.version_folders("ocr-hindi"), ["1.0", "2.0"], "the first run still holds the pack");

        drop(running);
        assert_eq!(setup.version_folders("ocr-hindi"), ["2.0"]);
        assert!(resolve(&setup, &mut Default::default()).unwrap().starts_with(setup.app_data.join("packs/ocr-hindi/2.0")));
        setup.assert_no_staging("held update");
    }

    #[test]
    fn the_production_command_path_runs_off_the_async_runtime() {
        let setup = Setup::new("blocking", &["ocr-hindi"]);
        let server = Server::start();
        setup.serve(&server);
        let downloads = Arc::new(Downloads::new(transport(server.base())).unwrap());
        let setup = Arc::new(setup);
        let result = tauri::async_runtime::block_on(tauri::async_runtime::spawn_blocking({
            let (downloads, setup) = (downloads.clone(), setup.clone());
            move || setup.install(&downloads, "ocr-hindi", &CancelFlag::default(), &|_, _, _| {})
        })).unwrap();
        result.unwrap();
        assert_eq!(setup.status("ocr-hindi"), PackStatus::Installed);
    }

    #[test]
    fn not_enough_free_space_is_refused_before_anything_is_downloaded() {
        let setup = Setup::new("space", &["ocr-hindi"]);
        let server = Server::start();
        setup.serve(&server);
        let downloads = Downloads::new(Transport { free_space: |_| Ok(1000), ..transport(server.base()) }).unwrap();
        let needed = setup.file("ocr-hindi").len() as u64 + 4096;
        let events = Events::default();
        assert_eq!(kind(setup.install(&downloads, "ocr-hindi", &CancelFlag::default(), &recorder(&events))), PackInstallErrorKind::NotEnoughSpace { needed_bytes: needed, free_bytes: 1000 });
        assert!(server.requests().is_empty());
        assert!(events.lock().unwrap().is_empty());
        setup.assert_clean("ocr-hindi", "not enough space");
    }

    #[test]
    fn cancelling_during_the_download_or_the_check_leaves_nothing_and_a_later_install_works() {
        let setup = Setup::new("cancel", &["ocr-hindi"]);
        let server = Server::start();
        let path = format!("/releases/{}", pack_path("ocr-hindi"));
        let downloads = Downloads::new(transport(server.base())).unwrap();

        // During the download: the server holds the second half back until after the cancel.
        let gate = Arc::new(AtomicBool::new(false));
        server.route(&path, Reply::Gated(setup.file("ocr-hindi"), gate.clone()));
        let cancel = CancelFlag::default();
        let during_download = setup.install(&downloads, "ocr-hindi", &cancel, &|phase, _, _| if phase == Phase::Downloading { cancel.cancel() });
        gate.store(true, Ordering::Release);
        assert_eq!(kind(during_download), PackInstallErrorKind::Cancelled);
        setup.assert_clean("ocr-hindi", "cancelled while downloading");

        // During the check: cancelled as soon as the download is complete.
        server.route(&path, Reply::File(setup.file("ocr-hindi")));
        let cancel = CancelFlag::default();
        let during_check = setup.install(&downloads, "ocr-hindi", &cancel, &|phase, _, _| if phase == Phase::Verifying { cancel.cancel() });
        assert_eq!(kind(during_check), PackInstallErrorKind::Cancelled);
        setup.assert_clean("ocr-hindi", "cancelled while checking");

        setup.install(&downloads, "ocr-hindi", &CancelFlag::default(), &|_, _, _| {}).unwrap();
        assert_eq!(setup.status("ocr-hindi"), PackStatus::Installed);
    }

    /// A resolver that finds nothing, as when there is no network.
    struct NoNetwork;

    impl Resolve for NoNetwork {
        fn resolve(&self, name: Name) -> Resolving {
            let host = name.as_str().to_string();
            Box::pin(async move { Err(Box::new(NameNotFound(host)) as Box<dyn std::error::Error + Send + Sync>) })
        }
    }

    #[test]
    fn offline_blocked_and_interrupted_are_typed_never_retried_and_a_later_install_starts_clean() {
        let setup = Setup::new("network", &["ocr-hindi"]);
        let server = Server::start();
        let path = format!("/releases/{}", pack_path("ocr-hindi"));
        let file = setup.file("ocr-hindi");
        let attempt = |downloads: &Downloads| setup.install(downloads, "ocr-hindi", &CancelFlag::default(), &|_, _, _| {});
        let downloads = Downloads::new(transport(server.base())).unwrap();

        let offline = Downloads::new(Transport { resolver: Arc::new(NoNetwork), ..transport(Url::parse("http://packs.toolbox.test/releases/").unwrap()) }).unwrap();
        assert_eq!(kind(attempt(&offline)), PackInstallErrorKind::Offline);
        setup.assert_clean("ocr-hindi", "offline");

        let refused_port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let refused = Downloads::new(transport(Url::parse(&format!("http://{refused_port}/releases/")).unwrap())).unwrap();
        // A TLS client against a server that does not speak TLS fails its handshake.
        let tls = Downloads::new(transport(Url::parse(&format!("https://{}/releases/", server.address)).unwrap())).unwrap();
        let blocked: Vec<(&str, Option<Reply>, &Downloads)> = vec![
            ("connection refused", None, &refused),
            ("TLS handshake", None, &tls),
            ("HTTP 403", Some(Reply::Status(403)), &downloads),
            ("redirect to another host", Some(Reply::Redirect(format!("http://elsewhere.toolbox.test:{}/x.toolboxpack", server.address.port()))), &downloads),
        ];
        for (case, reply, downloads) in blocked {
            if let Some(reply) = reply {
                server.route(&path, reply);
            }
            let before = server.requests().len();
            assert_eq!(kind(attempt(downloads)), PackInstallErrorKind::ServerBlocked, "{case}");
            assert!(server.requests().len() <= before + 1, "{case}: retried");
            setup.assert_clean("ocr-hindi", case);
        }

        server.route(&path, Reply::CutShort(file.clone()));
        let before = server.requests().len();
        assert_eq!(kind(attempt(&downloads)), PackInstallErrorKind::Interrupted);
        assert_eq!(server.requests().len(), before + 1, "an interrupted download is not retried");
        setup.assert_clean("ocr-hindi", "cut short");

        let gate = Arc::new(AtomicBool::new(false));
        server.route(&path, Reply::Gated(file.clone(), gate.clone()));
        let stalls = Downloads::new(Transport { stall_timeout: Duration::from_millis(300), ..transport(server.base()) }).unwrap();
        assert_eq!(kind(attempt(&stalls)), PackInstallErrorKind::Interrupted);
        gate.store(true, Ordering::Release);
        setup.assert_clean("ocr-hindi", "stalled");

        // Trying again starts from nothing and works, through a redirect on the server's own host.
        server.route(&path, Reply::Redirect(format!("{}moved.toolboxpack", server.base())));
        server.route("/releases/moved.toolboxpack", Reply::File(file));
        attempt(&downloads).unwrap();
        assert_eq!(setup.status("ocr-hindi"), PackStatus::Installed);
    }

    #[test]
    fn queued_installs_run_one_at_a_time_in_order_and_cancelling_one_leaves_the_others() {
        let setup = Arc::new(Setup::new("queue", &["pack-a", "pack-b", "pack-c"]));
        let server = Server::start();
        setup.serve(&server);
        let gate = Arc::new(AtomicBool::new(false));
        server.route(&format!("/releases/{}", pack_path("pack-a")), Reply::Gated(setup.file("pack-a"), gate.clone()));
        let downloads = Arc::new(Downloads::new(transport(server.base())).unwrap());
        let started: Arc<Mutex<Vec<String>>> = Arc::default();

        let start = |id: &'static str, cancel: CancelFlag| {
            let (setup, downloads, started) = (setup.clone(), downloads.clone(), started.clone());
            let (sender, receiver) = mpsc::channel();
            let worker = std::thread::spawn(move || setup.install(&downloads, id, &cancel, &|phase, _, _| {
                if phase == Phase::Downloading && !started.lock().unwrap().iter().any(|seen| seen == id) {
                    started.lock().unwrap().push(id.to_string());
                }
                let _ = sender.send(phase);
            }));
            (worker, receiver)
        };
        let wait_for = |receiver: &mpsc::Receiver<Phase>, phase: Phase| {
            while receiver.recv_timeout(Duration::from_secs(10)).expect("an event") != phase {}
        };

        let (a, a_events) = start("pack-a", CancelFlag::default());
        wait_for(&a_events, Phase::Downloading);
        let b_cancel = CancelFlag::default();
        let (b, b_events) = start("pack-b", b_cancel.clone());
        wait_for(&b_events, Phase::Queued);
        let (c, c_events) = start("pack-c", CancelFlag::default());
        wait_for(&c_events, Phase::Queued);

        b_cancel.cancel();
        assert_eq!(kind(b.join().unwrap()), PackInstallErrorKind::Cancelled);
        assert_eq!(*started.lock().unwrap(), ["pack-a"], "C waits while A is still downloading");
        gate.store(true, Ordering::Release);
        a.join().unwrap().unwrap();
        c.join().unwrap().unwrap();

        assert_eq!(*started.lock().unwrap(), ["pack-a", "pack-c"]);
        assert_eq!(server.requests(), [format!("/releases/{}", pack_path("pack-a")), format!("/releases/{}", pack_path("pack-c"))]);
        assert_eq!((setup.status("pack-a"), setup.status("pack-b"), setup.status("pack-c")), (PackStatus::Installed, PackStatus::NotInstalled, PackStatus::Installed));
        assert!(!setup.app_data.join(DOWNLOADS_DIR).exists());
        let _ = c_events;
    }

    #[test]
    fn a_pack_the_catalog_does_not_offer_is_refused_without_a_request() {
        let setup = Setup::new("unknown", &["ocr-hindi"]);
        let server = Server::start();
        let downloads = Downloads::new(transport(server.base())).unwrap();
        assert_eq!(kind(setup.install(&downloads, "../../elsewhere", &CancelFlag::default(), &|_, _, _| {})), PackInstallErrorKind::NotInCatalog);
        assert!(server.requests().is_empty());
    }

    #[test]
    fn the_release_server_is_https_and_redirects_only_to_its_download_hosts() {
        let transport = Transport::release().unwrap();
        assert_eq!(transport.base.as_str(), RELEASE_BASE);
        assert_eq!(transport.base.scheme(), "https");
        assert_eq!(transport.redirect_hosts, RELEASE_REDIRECT_HOSTS);
        assert!(Downloads::new(Transport { base: Url::parse("https://github.com/no-trailing-slash").unwrap(), ..transport }).is_err());
    }

    #[test]
    fn free_space_reads_the_disk() {
        assert!(free_space(&std::env::temp_dir()).unwrap() > 0);
    }
}
