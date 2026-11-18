//! Monitor discovery and stable identity (spec §8).

use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Gdi::HMONITOR;

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
        todo!()
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

/// Enumerate all monitors. Never requests the capabilities string and never does DDC I/O.
pub fn enumerate() -> Vec<FoundMonitor> {
    todo!()
}
