//! DDC/CI worker threads (spec §7). One worker per physical monitor; the UI
//! thread only posts requests and receives `DdcEvent`s through `notify`.

use super::monitors::PhysicalHandle;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdcEvent {
    /// Successful read of VCP 0x10, in percent.
    Value(u8),
    /// The last requested value was written, in percent.
    Written(u8),
    /// Read or write failed twice (one retry after 100 ms).
    Unresponsive,
    /// Reads work but writes are rejected (game/HDR preset, driver lock).
    ReadOnly,
}

/// Called from the worker thread with `(key, event)`. Must be cheap and non-blocking.
pub type Notify = Arc<dyn Fn(u64, DdcEvent) + Send + Sync>;

pub struct DdcWorker {
    // implementation-defined
}

impl DdcWorker {
    /// Start a worker that owns `handle`. `key` is echoed in every event so the
    /// UI can ignore events from workers of a previous scan.
    pub fn spawn(key: u64, handle: PhysicalHandle, write_interval: Duration, notify: Notify) -> DdcWorker {
        let _ = (key, handle, write_interval, notify);
        todo!()
    }

    /// Queue a read (coalesced with other pending reads).
    pub fn read(&self) {
        todo!()
    }

    /// Queue a write in percent; replaces any pending write (latest value wins).
    pub fn write(&self, percent: u8) {
        let _ = percent;
        todo!()
    }

    pub fn set_write_interval(&self, interval: Duration) {
        let _ = interval;
        todo!()
    }

    /// While paused (display off), writes stay pending and are not sent.
    pub fn set_paused(&self, paused: bool) {
        let _ = paused;
        todo!()
    }
}

impl Drop for DdcWorker {
    /// Signal the thread to stop after its current operation; does not join.
    fn drop(&mut self) {
        todo!()
    }
}

/// Blocking single read with one retry, for standalone `list`/`get`. Returns percent.
pub fn read_once(handle: &PhysicalHandle) -> Option<u8> {
    let _ = handle;
    todo!()
}
