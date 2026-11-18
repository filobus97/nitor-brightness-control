//! Blocking HTTPS GET over WinHTTP, shared by the update check and the place search.
//! Call from a worker thread only.

use crate::update;
use std::ffi::c_void;
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::PCWSTR;

use super::util::wide;

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
pub fn get(url: &str, accept: Option<&str>, max: usize) -> Result<Vec<u8>, String> {
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
