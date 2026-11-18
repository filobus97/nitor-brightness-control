//! Ambient light sensor (WinRT `LightSensor`). The reading event arrives on a sensor
//! thread: it only stores the lux and posts a message; all state lives on the UI thread.

use crate::win::util::log;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use windows::Devices::Sensors::{LightSensor, LightSensorReadingChangedEventArgs};
use windows::Foundation::TypedEventHandler;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::Ref;

/// Shortest report interval we ask for (ms); the sensor's own minimum if larger.
const REPORT_INTERVAL_MS: u32 = 1000;
/// Report only readings that moved by this much (percent / lux), where supported.
const THRESHOLD_PERCENT: f32 = 10.0;
const THRESHOLD_LUX: f32 = 5.0;

/// Latest lux as `f32` bits, and whether a message for it is already on its way.
static LUX: AtomicU32 = AtomicU32::new(0);
static PENDING: AtomicBool = AtomicBool::new(false);

/// Whether this PC has an ambient light sensor (looked up once).
pub fn sensor_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| LightSensor::GetDefault().is_ok())
}

/// The latest reading posted by the sensor thread, clearing the pending flag.
pub fn take_lux() -> f32 {
    PENDING.store(false, Ordering::Release);
    f32::from_bits(LUX.load(Ordering::Acquire))
}

/// An active `ReadingChanged` subscription; dropping it unsubscribes.
pub struct Subscription {
    sensor: LightSensor,
    token: i64,
}

impl Subscription {
    /// Subscribe and return the subscription with the current reading (lux), if any.
    pub fn start(hwnd: isize) -> Option<(Subscription, Option<f32>)> {
        let sensor = LightSensor::GetDefault().ok()?;
        let interval = sensor
            .MinimumReportInterval()
            .unwrap_or(0)
            .max(REPORT_INTERVAL_MS);
        let _ = sensor.SetReportInterval(interval);
        // Windows 10 2004+: let the driver suppress small changes (best effort).
        if let Ok(t) = sensor.ReportThreshold() {
            let _ = t.SetLuxPercentage(THRESHOLD_PERCENT);
            let _ = t.SetAbsoluteLux(THRESHOLD_LUX);
        }
        let handler =
            TypedEventHandler::new(move |_, args: Ref<LightSensorReadingChangedEventArgs>| {
                let lux = args.ok()?.Reading()?.IlluminanceInLux()?;
                LUX.store(lux.to_bits(), Ordering::Release);
                if !PENDING.swap(true, Ordering::AcqRel) {
                    // SAFETY: posting to a window handle is safe even if the window is gone.
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(hwnd as *mut _)),
                            super::WM_APP_AUTO_SENSOR,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
                Ok(())
            });
        let token = match sensor.ReadingChanged(&handler) {
            Ok(t) => t,
            Err(e) => {
                log(&format!("auto sensor: subscribe failed: {}", e.message()));
                return None;
            }
        };
        let now = sensor
            .GetCurrentReading()
            .and_then(|r| r.IlluminanceInLux())
            .ok();
        log(&format!("auto sensor: subscribed, interval {interval} ms"));
        Some((Subscription { sensor, token }, now))
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.sensor.RemoveReadingChanged(self.token);
        // 0 returns the sensor to its default (idle) reporting.
        let _ = self.sensor.SetReportInterval(0);
        log("auto sensor: unsubscribed");
    }
}
