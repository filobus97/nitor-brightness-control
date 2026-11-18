//! Monitor discovery and stable identity (spec §8).

use std::collections::HashMap;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DestroyPhysicalMonitor, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RegGetValueW};
use windows::core::{BOOL, PCWSTR, w};

/// Owned dxva2 physical-monitor handle, destroyed on drop (`DestroyPhysicalMonitor`).
pub struct PhysicalHandle(HANDLE);

// The handle is only ever used by one thread at a time (its DDC worker).
unsafe impl Send for PhysicalHandle {}

impl PhysicalHandle {
    /// # Safety
    /// `h` must be a handle from `GetPhysicalMonitorsFromHMONITOR` that nobody else destroys.
    pub unsafe fn from_raw(h: HANDLE) -> Self {
        PhysicalHandle(h)
    }

    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for PhysicalHandle {
    fn drop(&mut self) {
        // SAFETY: we own the handle (see `from_raw`); it is destroyed exactly once.
        unsafe {
            let _ = DestroyPhysicalMonitor(self.0);
        }
    }
}

/// One physical monitor. Cloned displays yield several entries for one `hmonitor`.
pub struct FoundMonitor {
    /// EDID manufacturer + product code + serial, e.g. `DEL41D9-ABC123`; device-path based fallback.
    /// Clones of an identical model get a `#2`, `#3`… suffix.
    pub stable_id: String,
    /// e.g. `DELL U2723QE`.
    pub friendly_name: String,
    pub serial: Option<String>,
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub device_name: String,
    pub hmonitor: HMONITOR,
    pub rect: RECT,
    /// `None` if Windows gave no physical handle (not controllable).
    pub handle: Option<PhysicalHandle>,
}

/// What DisplayConfig knows about one active path's target.
struct Target {
    source_gdi_name: String,
    friendly_name: String,
    device_path: String,
    edid_mfg: u16,
    edid_prod: u16,
}

fn from_wide(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Query all active display paths with their source/target names.
fn query_targets() -> Vec<Target> {
    let mut out = Vec::new();
    let (mut np, mut nm) = (0u32, 0u32);
    // SAFETY: out-pointers are valid locals.
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) }
        != ERROR_SUCCESS
    {
        return out;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
    // SAFETY: the arrays have exactly the element counts passed in `np`/`nm`.
    let r = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        )
    };
    if r != ERROR_SUCCESS {
        return out;
    }
    paths.truncate(np as usize);
    for p in &paths {
        let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        src.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        src.header.adapterId = p.sourceInfo.adapterId;
        src.header.id = p.sourceInfo.id;
        // SAFETY: header.size matches the struct that follows the header.
        if unsafe { DisplayConfigGetDeviceInfo(&mut src.header) } != 0 {
            continue;
        }
        let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        tgt.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        tgt.header.adapterId = p.targetInfo.adapterId;
        tgt.header.id = p.targetInfo.id;
        // SAFETY: header.size matches the struct that follows the header.
        if unsafe { DisplayConfigGetDeviceInfo(&mut tgt.header) } != 0 {
            continue;
        }
        out.push(Target {
            source_gdi_name: from_wide(&src.viewGdiDeviceName),
            friendly_name: from_wide(&tgt.monitorFriendlyDeviceName),
            device_path: from_wide(&tgt.monitorDevicePath),
            edid_mfg: tgt.edidManufactureId,
            edid_prod: tgt.edidProductCodeId,
        });
    }
    out
}

/// Split `\\?\DISPLAY#DEL41D9#5&2a3c1b&0&UID4352#{guid}` into (model, instance).
fn split_device_path(path: &str) -> Option<(&str, &str)> {
    let mut it = path.trim_start_matches(r"\\?\").split('#');
    let class = it.next()?;
    let model = it.next()?;
    let inst = it.next()?;
    (class.eq_ignore_ascii_case("DISPLAY") && !model.is_empty() && !inst.is_empty())
        .then_some((model, inst))
}

/// Read the EDID blob from `HKLM\SYSTEM\CurrentControlSet\Enum\DISPLAY\...\Device Parameters`.
fn read_edid(device_path: &str) -> Option<Vec<u8>> {
    let (model, inst) = split_device_path(device_path)?;
    let key = to_wide(&format!(
        r"SYSTEM\CurrentControlSet\Enum\DISPLAY\{model}\{inst}\Device Parameters"
    ));
    let mut buf = vec![0u8; 1024];
    let mut size = buf.len() as u32;
    // SAFETY: `key` is NUL-terminated; `buf` is valid for `size` bytes.
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    buf.truncate(size as usize);
    Some(buf)
}

fn clean_serial(s: &str) -> Option<String> {
    let c: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!c.is_empty()).then_some(c)
}

/// Parse an EDID block into (`{MFG}{PROD}`, serial).
fn parse_edid(e: &[u8]) -> Option<(String, Option<String>)> {
    if e.len() < 128 || e[0..8] != [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] {
        return None;
    }
    let m = u16::from_be_bytes([e[8], e[9]]);
    let letter = |v: u16| -> Option<char> {
        let v = (v & 0x1F) as u8;
        (1..=26).contains(&v).then(|| (b'A' + v - 1) as char)
    };
    let mfg: String = [letter(m >> 10)?, letter(m >> 5)?, letter(m)?]
        .iter()
        .collect();
    let prod = u16::from_le_bytes([e[10], e[11]]);
    let mut serial = None;
    for off in [54, 72, 90, 108] {
        let d = &e[off..off + 18];
        if d[0] == 0 && d[1] == 0 && d[2] == 0 && d[3] == 0xFF {
            let text: String = d[5..18]
                .iter()
                .take_while(|&&b| b != 0x0A)
                .map(|&b| b as char)
                .collect();
            serial = clean_serial(text.trim());
            break;
        }
    }
    if serial.is_none() {
        let n = u32::from_le_bytes([e[12], e[13], e[14], e[15]]);
        if n != 0 {
            serial = Some(n.to_string());
        }
    }
    Some((format!("{mfg}{prod:04X}"), serial))
}

fn make_id(base: &str, serial: &Option<String>) -> String {
    match serial {
        Some(s) => format!("{base}-{s}"),
        None => base.to_string(),
    }
}

/// Stable ID and serial for a target: EDID, else DisplayConfig ids, else device path.
fn identify(t: Option<&Target>) -> (Option<String>, Option<String>) {
    let Some(t) = t else { return (None, None) };
    if let Some((base, serial)) = read_edid(&t.device_path).as_deref().and_then(parse_edid) {
        return (Some(make_id(&base, &serial)), serial);
    }
    if t.edid_mfg != 0 || t.edid_prod != 0 {
        return (Some(format!("{:04X}{:04X}", t.edid_mfg, t.edid_prod)), None);
    }
    if let Some((model, inst)) = split_device_path(&t.device_path) {
        let s: String = format!("DISPLAY#{model}#{inst}")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '#' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        return (Some(s), None);
    }
    (None, None)
}

unsafe extern "system" fn enum_proc(h: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` is the `&mut Vec<HMONITOR>` passed by `enumerate`.
    let v = unsafe { &mut *(lp.0 as *mut Vec<HMONITOR>) };
    v.push(h);
    BOOL(1)
}

/// Enumerate all monitors. Never requests the capabilities string and never does DDC I/O.
pub fn enumerate() -> Vec<FoundMonitor> {
    let mut hmons: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only dereferences the Vec pointer, which outlives the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut hmons as *mut Vec<HMONITOR> as isize),
        );
    }
    let targets = query_targets();
    let mut out: Vec<FoundMonitor> = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();

    for hmon in hmons {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: cbSize says the buffer is a MONITORINFOEXW, which starts with a MONITORINFO.
        let ok = unsafe {
            GetMonitorInfoW(hmon, (&mut mi as *mut MONITORINFOEXW).cast::<MONITORINFO>())
        };
        if !ok.as_bool() {
            continue;
        }
        let device_name = from_wide(&mi.szDevice);
        let rect = mi.monitorInfo.rcMonitor;
        let my_targets: Vec<&Target> = targets
            .iter()
            .filter(|t| t.source_gdi_name == device_name)
            .collect();

        // Physical monitor handles (best effort).
        let mut phys: Vec<PHYSICAL_MONITOR> = Vec::new();
        let mut n = 0u32;
        // SAFETY: out-pointer is a valid local.
        if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut n) }.is_ok() && n > 0 {
            let mut arr = vec![PHYSICAL_MONITOR::default(); n as usize];
            // SAFETY: the slice length equals the count Windows reported.
            if unsafe { GetPhysicalMonitorsFromHMONITOR(hmon, &mut arr) }.is_ok() {
                phys = arr;
            }
        }

        // One entry per physical handle, or a single non-controllable entry.
        let count = phys.len().max(1);
        let matched = phys.len() == my_targets.len();
        for i in 0..count {
            let (handle, desc) = match phys.get(i) {
                Some(pm) => {
                    let raw = pm.hPhysicalMonitor;
                    let d = pm.szPhysicalMonitorDescription;
                    let handle = (!raw.is_invalid() && raw.0 as usize != 0)
                        // SAFETY: handle came from GetPhysicalMonitorsFromHMONITOR; we are its only owner.
                        .then(|| unsafe { PhysicalHandle::from_raw(raw) });
                    (handle, from_wide(&d))
                }
                None => (None, String::new()),
            };
            let target = my_targets.get(i).copied();
            let (id, serial) = identify(target);
            let n_out = out.len() + 1;
            let friendly_name = target
                .map(|t| t.friendly_name.clone())
                .filter(|s| matched && !s.is_empty())
                .or_else(|| Some(desc).filter(|s| !s.is_empty()))
                .or_else(|| {
                    target
                        .map(|t| t.friendly_name.clone())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| format!("Monitor {n_out}"));
            let mut stable_id = id.unwrap_or_else(|| {
                let d: String = device_name
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect();
                format!("{d}-{i}")
            });
            let c = seen.entry(stable_id.clone()).or_insert(0);
            *c += 1;
            if *c > 1 {
                stable_id = format!("{stable_id}#{c}");
            }
            out.push(FoundMonitor {
                stable_id,
                friendly_name,
                serial,
                device_name: device_name.clone(),
                hmonitor: hmon,
                rect,
                handle,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_edid() {
        let mut e = [0u8; 128];
        e[0..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
        // "DEL": D=4, E=5, L=12 -> 00100 00101 01100
        let m: u16 = (4 << 10) | (5 << 5) | 12;
        e[8..10].copy_from_slice(&m.to_be_bytes());
        e[10..12].copy_from_slice(&0x41D9u16.to_le_bytes());
        e[12..16].copy_from_slice(&1234u32.to_le_bytes());
        assert_eq!(
            parse_edid(&e),
            Some(("DEL41D9".into(), Some("1234".into())))
        );
        e[72..76].copy_from_slice(&[0, 0, 0, 0xFF]);
        e[77..77 + 7].copy_from_slice(b"ABC123\n");
        assert_eq!(
            parse_edid(&e),
            Some(("DEL41D9".into(), Some("ABC123".into())))
        );
        e[0] = 1;
        assert_eq!(parse_edid(&e), None);
    }

    #[test]
    fn splits_path() {
        let p = r"\\?\DISPLAY#DEL41D9#5&2a3c1b&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        assert_eq!(
            split_device_path(p),
            Some(("DEL41D9", "5&2a3c1b&0&UID4352"))
        );
    }
}
