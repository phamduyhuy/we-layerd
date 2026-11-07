use std::{
    fs,
    io::{self, ErrorKind, Read, Write},
    net::Shutdown,
    os::fd::AsRawFd,
    os::unix::net::{SocketAddr, UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use std::os::linux::net::SocketAddrExt;

use anyhow::{anyhow, Context, Result};
use we_core::config::OutputBinding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
    Stop,
    Pause,
    Resume,
    Reload,
    Reconfigure,
}

impl ControlCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Reload => "reload",
            Self::Reconfigure => "reconfigure",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "stop" => Some(Self::Stop),
            "pause" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "reload" => Some(Self::Reload),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistCommand {
    Play(String),
    Next,
    Previous,
    Stop,
    Output { output: String, action: OutputPlaylistAction },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputPlaylistAction {
    Play(String),
    Next,
    Previous,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileCommand {
    Apply(String),
}

impl ProfileCommand {
    fn request(&self) -> String {
        match self {
            Self::Apply(name) => format!("profile apply {name}"),
        }
    }
}

#[derive(Debug)]
pub struct OutputPlaylistRequest {
    pub output: String,
    pub action: OutputPlaylistAction,
    pub reply: std::sync::mpsc::Sender<std::result::Result<Option<OutputBinding>, String>>,
}

impl PlaylistCommand {
    fn request(&self) -> String {
        match self {
            Self::Play(name) => format!("playlist play {name}"),
            Self::Next => "playlist next".to_string(),
            Self::Previous => "playlist previous".to_string(),
            Self::Stop => "playlist stop".to_string(),
            Self::Output { output, action } => match action {
                OutputPlaylistAction::Play(name) => {
                    format!("playlist output {output} play {name}")
                }
                OutputPlaylistAction::Next => format!("playlist output {output} next"),
                OutputPlaylistAction::Previous => {
                    format!("playlist output {output} previous")
                }
                OutputPlaylistAction::Stop => format!("playlist output {output} stop"),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLoopExit {
    Stop,
    RestartCurrent,
    Reconfigure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ControlRequest {
    Command(ControlCommand),
    Status,
    SwitchConfig(PathBuf),
    ReloadSceneConfig(PathBuf),
    Playlist(PlaylistCommand),
    Profile(ProfileCommand),
}

impl ControlRequest {
    fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("switch-config ") {
            let path = rest.trim();
            if path.is_empty() {
                return None;
            }
            return Some(Self::SwitchConfig(PathBuf::from(path)));
        }
        if let Some(rest) = trimmed.strip_prefix("reload-scene-config ") {
            let path = rest.trim();
            if path.is_empty() {
                return None;
            }
            return Some(Self::ReloadSceneConfig(PathBuf::from(path)));
        }
        if let Some(name) = trimmed.strip_prefix("playlist play ") {
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            return Some(Self::Playlist(PlaylistCommand::Play(name.to_string())));
        }
        if let Some(name) = trimmed.strip_prefix("profile apply ") {
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            return Some(Self::Profile(ProfileCommand::Apply(name.to_string())));
        }
        if let Some(rest) = trimmed.strip_prefix("playlist output ") {
            let (output, action) = rest.split_once(' ')?;
            let output = output.trim();
            let action = action.trim();
            if output.is_empty() || action.is_empty() {
                return None;
            }
            let action = if let Some(name) = action.strip_prefix("play ") {
                let name = name.trim();
                if name.is_empty() {
                    return None;
                }
                OutputPlaylistAction::Play(name.to_string())
            } else {
                match action.to_ascii_lowercase().as_str() {
                    "next" => OutputPlaylistAction::Next,
                    "previous" | "prev" => OutputPlaylistAction::Previous,
                    "stop" => OutputPlaylistAction::Stop,
                    _ => return None,
                }
            };
            return Some(Self::Playlist(PlaylistCommand::Output {
                output: output.to_string(),
                action,
            }));
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "playlist next" => return Some(Self::Playlist(PlaylistCommand::Next)),
            "playlist previous" | "playlist prev" => {
                return Some(Self::Playlist(PlaylistCommand::Previous))
            }
            "playlist stop" => return Some(Self::Playlist(PlaylistCommand::Stop)),
            _ => {}
        }
        let normalized = trimmed.to_ascii_lowercase();
        if normalized == "status" {
            return Some(Self::Status);
        }
        ControlCommand::parse(&normalized).map(Self::Command)
    }
}

const IPC_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLIENTS: usize = 16;

fn peer_uid_allowed(peer: u32, owner: u32) -> bool {
    peer == owner
}

fn authenticate_peer(stream: &UnixStream) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut credentials as *mut _ as *mut libc::c_void,
                &mut length,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if length as usize != std::mem::size_of_val(&credentials)
            || !peer_uid_allowed(credentials.uid, unsafe { libc::geteuid() })
        {
            return Err(io::Error::new(ErrorKind::PermissionDenied, "IPC peer UID mismatch"));
        }
    }
    Ok(())
}

fn read_message(stream: &mut UnixStream, limit: usize, timeout: Duration) -> io::Result<String> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::new(ErrorKind::TimedOut, "IPC read deadline exceeded"))?;
        stream.set_read_timeout(Some(remaining))?;
        let mut buffer = [0; 4096];
        match stream.read(&mut buffer) {
            Ok(0) => {
                return String::from_utf8(bytes)
                    .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
            }
            Ok(count) => {
                if bytes.len() + count > limit {
                    return Err(io::Error::new(ErrorKind::InvalidData, "IPC message too large"));
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

struct ClientSlot(Arc<AtomicUsize>);
impl Drop for ClientSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct ControlServer {
    socket_path: Option<PathBuf>,
    _instance_lock: fs::File,
}

impl ControlServer {
    pub fn start<F, H, S, P, R>(
        tx: Sender<ControlCommand>,
        status_provider: F,
        command_handler: H,
        switch_config_handler: S,
        playlist_handler: P,
        profile_handler: R,
    ) -> Result<Self>
    where
        F: Fn() -> String + Send + Sync + 'static,
        H: Fn(ControlCommand) -> Result<bool> + Send + Sync + 'static,
        S: Fn(&Path, bool) -> Result<()> + Send + Sync + 'static,
        P: Fn(PlaylistCommand) -> Result<()> + Send + Sync + 'static,
        R: Fn(ProfileCommand) -> Result<()> + Send + Sync + 'static,
    {
        let instance_lock = acquire_instance_lock()?;
        let endpoint = default_endpoint()?;
        let listener = bind_listener(&endpoint)?;
        let socket_path = endpoint.socket_path();
        spawn_listener(
            listener,
            tx,
            status_provider,
            command_handler,
            switch_config_handler,
            playlist_handler,
            profile_handler,
        );

        Ok(Self { socket_path, _instance_lock: instance_lock })
    }
}

fn spawn_listener<F, H, S, P, R>(
    listener: UnixListener,
    tx: Sender<ControlCommand>,
    status_provider: F,
    command_handler: H,
    switch_config_handler: S,
    playlist_handler: P,
    profile_handler: R,
) where
    F: Fn() -> String + Send + Sync + 'static,
    H: Fn(ControlCommand) -> Result<bool> + Send + Sync + 'static,
    S: Fn(&Path, bool) -> Result<()> + Send + Sync + 'static,
    P: Fn(PlaylistCommand) -> Result<()> + Send + Sync + 'static,
    R: Fn(ProfileCommand) -> Result<()> + Send + Sync + 'static,
{
    let status_provider = std::sync::Arc::new(status_provider);
    let command_handler = std::sync::Arc::new(command_handler);
    let switch_config_handler = std::sync::Arc::new(switch_config_handler);
    let playlist_handler = std::sync::Arc::new(playlist_handler);
    let profile_handler = std::sync::Arc::new(profile_handler);
    let clients = Arc::new(AtomicUsize::new(0));
    // Keep mutations ordered while slow request/response I/O stays outside this lock.
    let dispatch = Arc::new(Mutex::new(()));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            if authenticate_peer(&stream).is_err() {
                continue;
            }
            if clients
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    (count < MAX_CLIENTS).then_some(count + 1)
                })
                .is_err()
            {
                continue;
            }
            let slot = ClientSlot(clients.clone());
            let status_provider = status_provider.clone();
            let command_handler = command_handler.clone();
            let switch_config_handler = switch_config_handler.clone();
            let playlist_handler = playlist_handler.clone();
            let profile_handler = profile_handler.clone();
            let tx = tx.clone();
            let dispatch = dispatch.clone();
            let _ = thread::Builder::new().name("we-layerd-ipc".into()).spawn(move || {
                let _slot = slot;
                let _ = stream.set_write_timeout(Some(IPC_TIMEOUT));
                let buf = match read_message(&mut stream, MAX_REQUEST_BYTES, IPC_TIMEOUT) {
                    Ok(buf) => buf,
                    Err(_) => {
                        let _ = stream.write_all(b"ERR invalid or timed out request\n");
                        return;
                    }
                };
                let Some(request) = ControlRequest::parse(&buf) else {
                    let _ = stream.write_all(b"ERR unknown command\n");
                    return;
                };
                let response = {
                    let Ok(_dispatch) = dispatch.lock() else {
                        return;
                    };
                    let result = match request {
                        ControlRequest::Status => None,
                        ControlRequest::SwitchConfig(ref path) => {
                            Some(switch_config_handler(path, false))
                        }
                        ControlRequest::ReloadSceneConfig(ref path) => {
                            Some(switch_config_handler(path, true))
                        }
                        ControlRequest::Playlist(ref command) => {
                            Some(playlist_handler(command.clone()))
                        }
                        ControlRequest::Profile(ref command) => {
                            Some(profile_handler(command.clone()))
                        }
                        ControlRequest::Command(command) => {
                            Some(command_handler(command).and_then(|handled| {
                                if handled {
                                    Ok(())
                                } else {
                                    tx.send(command).context("daemon not running")
                                }
                            }))
                        }
                    };
                    match result {
                        None => status_provider(),
                        Some(Ok(())) => "OK\n".to_string(),
                        Some(Err(error)) => format!("ERR {error}\n"),
                    }
                };
                if response.len() <= MAX_RESPONSE_BYTES {
                    let _ = stream.write_all(response.as_bytes());
                } else {
                    let _ = stream.write_all(b"ERR response too large\n");
                }
            });
        }
    });
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        if let Some(socket_path) = &self.socket_path {
            let _ = fs::remove_file(socket_path);
        }
    }
}

pub fn send_command(command: ControlCommand) -> Result<()> {
    let response = send_request(command.as_str())?;
    if response.trim_start().starts_with("ERR") {
        return Err(anyhow!(response.trim().to_string()));
    }
    Ok(())
}

pub fn request_running_config() -> Result<String> {
    let response = send_request("status")?;
    if response.trim_start().starts_with("ERR") {
        return Err(anyhow!(response.trim().to_string()));
    }
    Ok(response)
}

pub fn send_switch_config(config_path: &Path) -> Result<()> {
    let response = send_request(&format!("switch-config {}", config_path.display()))?;
    if response.trim_start().starts_with("ERR") {
        return Err(anyhow!(response.trim().to_string()));
    }
    Ok(())
}

pub fn send_reload_scene_config(config_path: &Path) -> Result<()> {
    let response = send_request(&format!("reload-scene-config {}", config_path.display()))?;
    if response.trim_start().starts_with("ERR") {
        return Err(anyhow!(response.trim().to_string()));
    }
    Ok(())
}

pub fn send_playlist_command(command: &PlaylistCommand) -> Result<()> {
    let response = send_request(&command.request())?;
    if response.trim_start().starts_with("ERR") {
        return Err(anyhow!(response.trim().to_string()));
    }
    Ok(())
}

pub fn send_profile_command(command: &ProfileCommand) -> Result<()> {
    let response = send_request(&command.request())?;
    if response.trim_start().starts_with("ERR") {
        return Err(anyhow!(response.trim().to_string()));
    }
    Ok(())
}

fn send_request(request: &str) -> Result<String> {
    let mut last_error: Option<anyhow::Error> = None;
    for endpoint in control_endpoints() {
        match connect_stream(&endpoint) {
            Ok(mut stream) => {
                authenticate_peer(&stream).context("untrusted IPC server")?;
                stream.set_write_timeout(Some(IPC_TIMEOUT))?;
                if request.len() > MAX_REQUEST_BYTES {
                    return Err(anyhow!("IPC request too large"));
                }
                stream
                    .write_all(request.as_bytes())
                    .with_context(|| format!("failed to send IPC request '{request}'"))?;
                let _ = stream.shutdown(Shutdown::Write);
                let response = read_message(&mut stream, MAX_RESPONSE_BYTES, IPC_TIMEOUT)
                    .context("failed to read IPC response")?;
                if response.trim().is_empty() {
                    return Err(anyhow!("IPC server closed without a response"));
                }
                return Ok(response);
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    Err(anyhow!("failed to reach we-layerd control endpoint (daemon may not be running)"))
        .context(last_error.unwrap_or_else(|| anyhow!("no endpoint available")))
}

#[derive(Debug, Clone)]
enum Endpoint {
    Path(PathBuf),
    #[cfg(target_os = "linux")]
    Abstract(Vec<u8>),
}

impl Endpoint {
    fn socket_path(&self) -> Option<PathBuf> {
        match self {
            Self::Path(path) => Some(path.clone()),
            #[cfg(target_os = "linux")]
            Self::Abstract(_) => None,
        }
    }
}

fn default_endpoint() -> Result<Endpoint> {
    #[cfg(target_os = "linux")]
    {
        Ok(Endpoint::Abstract(abstract_socket_name()))
    }

    #[cfg(not(target_os = "linux"))]
    {
        Ok(Endpoint::Path(default_socket_path()?))
    }
}

fn control_endpoints() -> Vec<Endpoint> {
    #[cfg(target_os = "linux")]
    {
        vec![
            Endpoint::Abstract(abstract_socket_name()),
            Endpoint::Path(
                default_socket_path()
                    .unwrap_or_else(|_| PathBuf::from("/tmp/we-layerd-control.sock")),
            ),
        ]
    }

    #[cfg(not(target_os = "linux"))]
    {
        vec![Endpoint::Path(
            default_socket_path().unwrap_or_else(|_| PathBuf::from("/tmp/we-layerd-control.sock")),
        )]
    }
}

fn bind_listener(endpoint: &Endpoint) -> Result<UnixListener> {
    match endpoint {
        Endpoint::Path(socket_path) => bind_file_listener(socket_path),
        #[cfg(target_os = "linux")]
        Endpoint::Abstract(name) => {
            let addr = SocketAddr::from_abstract_name(name)
                .context("failed to build abstract IPC socket")?;
            UnixListener::bind_addr(&addr)
                .context("failed to bind abstract IPC socket for we-layerd")
        }
    }
}

fn bind_file_listener(socket_path: &Path) -> Result<UnixListener> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if socket_path.exists() {
        if UnixStream::connect(socket_path).is_ok() {
            return Err(anyhow!("we-layerd is already running"));
        }
        let _ = fs::remove_file(socket_path);
    }

    UnixListener::bind(socket_path)
        .with_context(|| format!("failed to bind IPC socket {}", socket_path.display()))
}

fn connect_stream(endpoint: &Endpoint) -> Result<UnixStream> {
    match endpoint {
        Endpoint::Path(path) => UnixStream::connect(path)
            .with_context(|| format!("failed to connect IPC socket {}", path.display())),
        #[cfg(target_os = "linux")]
        Endpoint::Abstract(name) => {
            let addr = SocketAddr::from_abstract_name(name)
                .context("failed to build abstract IPC socket")?;
            UnixStream::connect_addr(&addr).context("failed to connect abstract IPC socket")
        }
    }
}

fn default_socket_path() -> Result<PathBuf> {
    Ok(ipc_runtime_dir()?.join("control.sock"))
}

fn instance_lock_path() -> Result<PathBuf> {
    Ok(ipc_runtime_dir()?.join("instance.lock"))
}

fn ipc_runtime_dir() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime_dir).join("we-layerd"));
    }

    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".config/we-layerd"))
}

fn acquire_instance_lock() -> Result<fs::File> {
    let lock_path = instance_lock_path()?;
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;

    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Err(anyhow!("we-layerd is already running"));
        }
        return Err(err).with_context(|| format!("failed to lock {}", lock_path.display()));
    }

    Ok(file)
}

#[cfg(target_os = "linux")]
fn abstract_socket_name() -> Vec<u8> {
    let uid = unsafe { libc::geteuid() };
    format!("we-layerd.control.{uid}").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{ControlRequest, OutputPlaylistAction, PlaylistCommand, ProfileCommand};

    #[test]
    fn scene_only_reload_does_not_alias_manual_config_switch() {
        assert_eq!(
            ControlRequest::parse("reload-scene-config /tmp/my settings/config.toml"),
            Some(ControlRequest::ReloadSceneConfig("/tmp/my settings/config.toml".into()))
        );
        assert_eq!(ControlRequest::parse("reload-scene-config   "), None);
        assert_eq!(
            ControlRequest::parse("switch-config /tmp/config.toml"),
            Some(ControlRequest::SwitchConfig("/tmp/config.toml".into()))
        );
    }

    #[test]
    fn playlist_ipc_preserves_named_playlists_and_rejects_empty_names() {
        assert_eq!(
            ControlRequest::parse("playlist play Focus Session"),
            Some(ControlRequest::Playlist(PlaylistCommand::Play("Focus Session".to_string())))
        );
        assert_eq!(ControlRequest::parse("playlist play    "), None);
        assert_eq!(
            ControlRequest::parse("playlist next"),
            Some(ControlRequest::Playlist(PlaylistCommand::Next))
        );
        assert_eq!(
            ControlRequest::parse("playlist previous"),
            Some(ControlRequest::Playlist(PlaylistCommand::Previous))
        );
        assert_eq!(
            ControlRequest::parse("playlist stop"),
            Some(ControlRequest::Playlist(PlaylistCommand::Stop))
        );
    }

    #[test]
    fn playlist_ipc_routes_output_scoped_actions_without_changing_global_commands() {
        assert_eq!(
            ControlRequest::parse("playlist output DP-1 play Focus Session"),
            Some(ControlRequest::Playlist(PlaylistCommand::Output {
                output: "DP-1".to_string(),
                action: OutputPlaylistAction::Play("Focus Session".to_string()),
            }))
        );
        assert_eq!(
            ControlRequest::parse("playlist output HDMI-A-1 next"),
            Some(ControlRequest::Playlist(PlaylistCommand::Output {
                output: "HDMI-A-1".to_string(),
                action: OutputPlaylistAction::Next,
            }))
        );
        assert_eq!(
            ControlRequest::parse("playlist output HDMI-A-1 stop"),
            Some(ControlRequest::Playlist(PlaylistCommand::Output {
                output: "HDMI-A-1".to_string(),
                action: OutputPlaylistAction::Stop,
            }))
        );
        assert_eq!(ControlRequest::parse("playlist output DP-1 play    "), None);
        assert_eq!(ControlRequest::parse("playlist output    next"), None);
        assert_eq!(
            ControlRequest::parse("playlist next"),
            Some(ControlRequest::Playlist(PlaylistCommand::Next))
        );
    }

    #[test]
    fn profile_ipc_preserves_named_profile_and_rejects_empty_names() {
        assert_eq!(
            ControlRequest::parse("profile apply Desk Setup"),
            Some(ControlRequest::Profile(ProfileCommand::Apply("Desk Setup".to_string())))
        );
        assert_eq!(ControlRequest::parse("profile apply    "), None);
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn uid_authentication_rejects_other_users_and_accepts_socket_peer() {
        assert!(peer_uid_allowed(1000, 1000));
        assert!(!peer_uid_allowed(1001, 1000));
        let (peer, _) = UnixStream::pair().unwrap();
        authenticate_peer(&peer).unwrap();
    }

    #[test]
    fn read_has_size_and_total_time_limits() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"too long").unwrap();
        assert_eq!(
            read_message(&mut reader, 4, Duration::from_secs(1)).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        let (mut reader, _writer) = UnixStream::pair().unwrap();
        let started = Instant::now();
        assert!(read_message(&mut reader, 4, Duration::from_millis(30)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn idle_client_does_not_block_status_or_commands() {
        use std::sync::atomic::AtomicU64;
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "we-layerd-test-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let addr = SocketAddr::from_abstract_name(name).unwrap();
        let listener = UnixListener::bind_addr(&addr).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_listener(
            listener,
            tx,
            || "test-status".into(),
            |_| Ok(false),
            |_, _| Ok(()),
            |_| Ok(()),
            |_| Ok(()),
        );
        let _idle = UnixStream::connect_addr(&addr).unwrap();
        let request = |text: &[u8]| {
            let mut stream = UnixStream::connect_addr(&addr).unwrap();
            stream.write_all(text).unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            read_message(&mut stream, MAX_RESPONSE_BYTES, Duration::from_secs(1)).unwrap()
        };
        assert_eq!(request(b"status"), "test-status");
        assert_eq!(request(b"pause"), "OK\n");
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), ControlCommand::Pause);
    }
}
