//! Self-update over WinHTTP: look up the latest GitHub release, download and verify the
//! exe for this architecture, swap it in place (spec §10 "Updates"). Everything here
//! blocks; the UI thread must call it from a worker thread only.

use crate::update::{self, Release};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::core::PCWSTR;

use super::util::wide;

/// Size caps for downloads (the exe is well under 2 MB).
const MAX_API: usize = 1 << 20;
const MAX_SUMS: usize = 1 << 16;
const MAX_EXE: usize = 32 << 20;
/// How long a freshly started exe waits for its predecessor to exit.
const WAIT_PREDECESSOR_MS: u32 = 10_000;
const SUMS_NAME: &str = "SHA256SUMS.txt";

pub enum InstallError {
    /// The exe's folder rejects writes: the user has to download the release by hand.
    NotWritable,
    Failed(String),
}

/// Closes a WinHTTP handle on drop.
struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> Result<Handle, String> {
        if raw.is_null() {
            Err(windows::core::Error::from_thread().message())
        } else {
            Ok(Handle(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful WinHTTP call and is closed once.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

fn win_err(e: windows::core::Error) -> String {
    e.message()
}

/// HTTPS GET following redirects; the body, if the status is 200 and it fits in `max` bytes.
/// Sends only `User-Agent: Nitor/<version>` and, if given, an `Accept` header.
fn get(url: &str, accept: Option<&str>, max: usize) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("not an https URL")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (agent, host, path) = (
        wide(&format!("Nitor/{}", update::current_version())),
        wide(host),
        wide(&format!("/{path}")),
    );
    let headers: Option<Vec<u16>> =
        accept.map(|a| format!("Accept: {a}\r\n").encode_utf16().collect());
    // SAFETY: every buffer passed below outlives the call it is passed to; the handles
    // are closed by `Handle::drop` in reverse order of creation.
    unsafe {
        let session = Handle::new(WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 30_000).map_err(win_err)?;
        let conn = Handle::new(WinHttpConnect(
            session.0,
            PCWSTR(host.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ))?;
        let req = Handle::new(WinHttpOpenRequest(
            conn.0,
            windows::core::w!("GET"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSendRequest(req.0, headers.as_deref(), None, 0, 0, 0).map_err(win_err)?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(win_err)?;

        let mut status = 0u32;
        let mut len = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(std::ptr::addr_of_mut!(status).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(win_err)?;
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }

        let mut body = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                req.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .map_err(win_err)?;
            if read == 0 {
                return Ok(body);
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > max {
                return Err("download too large".into());
            }
        }
    }
}

/// SHA-256 through Windows CNG.
fn sha256(data: &[u8]) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];
    // SAFETY: plain call with valid input and output slices; the algorithm handle is a
    // pseudo-handle that needs no opening or closing.
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    if status.is_ok() {
        Ok(out)
    } else {
        Err(format!("SHA-256 failed (0x{:08X})", status.0))
    }
}

/// The latest published (non-prerelease) release.
pub fn check() -> Result<Release, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        update::repo_slug()
    );
    let body = get(&url, Some("application/vnd.github+json"), MAX_API)?;
    update::parse_release(&String::from_utf8_lossy(&body))
        .ok_or_else(|| "unexpected answer from GitHub".to_string())
}

/// `<exe>` + `.new` / `.old`, next to the exe.
fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Download and verify the release's exe, then swap it in for the running one.
/// Returns the path to start the new version from (not `current_exe`, which may follow
/// the rename).
pub fn install(release: &Release) -> Result<PathBuf, InstallError> {
    let failed = |e: String| InstallError::Failed(e);
    let asset_name = update::asset_name();
    let asset = asset_name.as_str();
    let url_of = |name: &str| {
        release
            .asset_url(name)
            .ok_or_else(|| failed(format!("{name} is not part of this release")))
    };
    let sums = get(url_of(SUMS_NAME)?, None, MAX_SUMS).map_err(failed)?;
    let expected = update::parse_sha256sums(&String::from_utf8_lossy(&sums), asset)
        .ok_or_else(|| failed(format!("no checksum for {asset}")))?;
    let exe = get(url_of(asset)?, None, MAX_EXE).map_err(failed)?;
    if sha256(&exe).map_err(failed)? != expected {
        return Err(failed("checksum mismatch".into()));
    }

    let current = super::util::exe_path();
    let (new, old) = (sibling(&current, ".new"), sibling(&current, ".old"));
    if let Err(e) = std::fs::write(&new, &exe) {
        let _ = std::fs::remove_file(&new);
        return Err(if e.kind() == std::io::ErrorKind::PermissionDenied {
            InstallError::NotWritable
        } else {
            failed(e.to_string())
        });
    }
    // Windows lets a running exe be renamed, not overwritten.
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(&current, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(failed(e.to_string()));
    }
    if let Err(e) = std::fs::rename(&new, &current) {
        let _ = std::fs::rename(&old, &current);
        let _ = std::fs::remove_file(&new);
        return Err(failed(e.to_string()));
    }
    Ok(current)
}

/// Start `exe` (the new version); it waits for this process to exit before taking over.
pub fn restart(exe: &Path) -> Result<(), String> {
    Command::new(exe)
        .arg("--after-update")
        .arg(std::process::id().to_string())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// In the new exe: wait (up to 10 s) for the process that started the update to exit,
/// so the single-instance mutex is free.
pub fn wait_for_exit(pid: u32) {
    // SAFETY: plain calls; the handle is closed after the wait.
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, WAIT_PREDECESSOR_MS);
            let _: Result<(), _> = CloseHandle(HANDLE(h.0));
        }
    }
}

/// Best effort: remove the previous version left behind by an update.
pub fn remove_old_exe() {
    let _ = std::fs::remove_file(sibling(&super::util::exe_path(), ".old"));
}
