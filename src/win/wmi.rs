//! Minimal WMI client for the built-in display (`root\WMI` brightness classes).
//! Built-in panels do not speak DDC/CI; Windows exposes them through
//! `WmiMonitorBrightness` (read) and `WmiMonitorBrightnessMethods.WmiSetBrightness` (write).

use super::util::log;
use crate::display_id::wql_escape;
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::{E_UNEXPECTED, ERROR_NOT_FOUND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoInitializeSecurity, CoSetProxyBlanket, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0_0, VT_BSTR, VT_I4, VT_UI1, VariantClear,
};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemContext, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE, WbemLocator,
};
use windows::core::{BSTR, Error, PCWSTR, Result, w};

/// Process-wide COM security with default authentication and impersonation, which some
/// systems require before WMI calls succeed. Call once, right after `CoInitializeEx`;
/// `RPC_E_TOO_LATE` (already set) and any other failure are ignored.
pub fn init_security() {
    // SAFETY: all pointer arguments are None; no security descriptor is supplied.
    let _ = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        )
    };
}

/// Log a failed WMI step with its HRESULT and turn the result into an `Option`.
fn logged<T>(what: &str, r: Result<T>) -> Option<T> {
    r.map_err(|e| log(&format!("wmi {what} failed: {:#010X}", e.code().0)))
        .ok()
}

fn not_found() -> Error {
    ERROR_NOT_FOUND.to_hresult().into()
}

/// A connection to the `ROOT\WMI` namespace. Use it only on the thread that created it.
pub struct Wmi {
    services: IWbemServices,
    /// WMI `InstanceName` of the panel to control; `None`: the first active one.
    instance: Option<String>,
}

impl Wmi {
    /// Connect to `ROOT\WMI`, bound to `instance` if given. COM must already be initialized
    /// on this thread. Failures are logged.
    pub fn connect(instance: Option<String>) -> Option<Wmi> {
        logged("connect", Self::try_connect(instance))
    }

    fn try_connect(instance: Option<String>) -> Result<Wmi> {
        // SAFETY: plain COM calls; every pointer argument is valid or None for the call.
        unsafe {
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
            let services = locator.ConnectServer(
                &BSTR::from(r"ROOT\WMI"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None::<&IWbemContext>,
            )?;
            CoSetProxyBlanket(
                &services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )?;
            Ok(Wmi { services, instance })
        }
    }

    /// `InstanceName` of every active brightness-capable panel (one local WMI query,
    /// roughly 10-50 ms).
    pub fn instances(&self) -> Vec<String> {
        let q = "SELECT InstanceName FROM WmiMonitorBrightness WHERE Active=TRUE";
        logged("instances query", self.objects(q))
            .unwrap_or_default()
            .iter()
            .filter_map(|o| get_string(o, w!("InstanceName")))
            .map(|b| b.to_string())
            .collect()
    }

    /// `WHERE` clause selecting the bound panel, or the first active one.
    fn filter(&self) -> String {
        match &self.instance {
            Some(n) => format!(" WHERE Active=TRUE AND InstanceName='{}'", wql_escape(n)),
            None => " WHERE Active=TRUE".to_string(),
        }
    }

    /// Current built-in brightness in percent, if the panel reports one.
    pub fn read(&self) -> Option<u8> {
        logged("read", self.try_read())
    }

    fn try_read(&self) -> Result<u8> {
        let q = format!(
            "SELECT CurrentBrightness FROM WmiMonitorBrightness{}",
            self.filter()
        );
        let obj = self.objects(&q)?.into_iter().next().ok_or_else(not_found)?;
        let mut v = VARIANT::default();
        // SAFETY: `v` is a valid out-variant; it is read by its type tag, then cleared.
        unsafe {
            obj.Get(w!("CurrentBrightness"), 0, &mut v, None, None)?;
            let inner = &v.Anonymous.Anonymous;
            let out = match inner.vt {
                VT_UI1 => Some(inner.Anonymous.bVal),
                VT_I4 => u8::try_from(inner.Anonymous.lVal).ok(),
                _ => None,
            };
            let _ = VariantClear(&mut v);
            out.map(|p| p.min(100)).ok_or_else(|| E_UNEXPECTED.into())
        }
    }

    /// Set the built-in brightness in percent. Returns whether WMI accepted the call.
    pub fn set(&self, percent: u8) -> bool {
        logged("set", self.try_set(percent)).is_some()
    }

    fn try_set(&self, percent: u8) -> Result<()> {
        // The method must be executed on an instance, addressed by its `__PATH`.
        let q = format!(
            "SELECT __PATH FROM WmiMonitorBrightnessMethods{}",
            self.filter()
        );
        let target = self.objects(&q)?.into_iter().next().ok_or_else(not_found)?;
        let path = get_string(&target, w!("__PATH")).ok_or_else(not_found)?;
        // SAFETY: COM calls on live interfaces; variants hold no heap data.
        unsafe {
            // In-parameters come from the class definition, not from the instance.
            let mut class = None;
            self.services.GetObject(
                &BSTR::from("WmiMonitorBrightnessMethods"),
                WBEM_GENERIC_FLAG_TYPE(0),
                None::<&IWbemContext>,
                Some(&mut class),
                None,
            )?;
            let mut in_sig: Option<IWbemClassObject> = None;
            class.ok_or_else(not_found)?.GetMethod(
                w!("WmiSetBrightness"),
                0,
                &mut in_sig,
                std::ptr::null_mut(),
            )?;
            let params = in_sig.ok_or_else(not_found)?.SpawnInstance(0)?;
            // Timeout is a uint32 (VT_I4 is coerced), Brightness a uint8 (VT_UI1).
            params.Put(w!("Timeout"), 0, &scalar(VT_I4, |v| v.lVal = 0), 0)?;
            params.Put(
                w!("Brightness"),
                0,
                &scalar(VT_UI1, |v| v.bVal = percent.min(100)),
                0,
            )?;
            self.services.ExecMethod(
                &path,
                &BSTR::from("WmiSetBrightness"),
                WBEM_GENERIC_FLAG_TYPE(0),
                None::<&IWbemContext>,
                &params,
                None,
                None,
            )
        }
    }

    /// All objects of a WQL query.
    fn objects(&self, wql: &str) -> Result<Vec<IWbemClassObject>> {
        let mut out = Vec::new();
        // SAFETY: the enumerator and the one-element out-array are valid for each call.
        unsafe {
            let en = self.services.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(wql),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None::<&IWbemContext>,
            )?;
            loop {
                let mut objs = [None];
                let mut n = 0u32;
                en.Next(WBEM_INFINITE, &mut objs, &mut n).ok()?;
                let [obj] = objs;
                match obj {
                    Some(o) if n == 1 => out.push(o),
                    _ => break,
                }
            }
        }
        Ok(out)
    }
}

/// A string property of `obj` (`None` if missing or not a BSTR).
fn get_string(obj: &IWbemClassObject, name: PCWSTR) -> Option<BSTR> {
    let mut v = VARIANT::default();
    // SAFETY: `v` is a valid out-variant; the BSTR is copied out before the variant is cleared.
    unsafe {
        obj.Get(name, 0, &mut v, None, None).ok()?;
        let s = (v.Anonymous.Anonymous.vt == VT_BSTR)
            .then(|| (*v.Anonymous.Anonymous.Anonymous.bstrVal).clone());
        let _ = VariantClear(&mut v);
        s
    }
}

/// A heap-free VARIANT of type `vt` whose payload `fill` sets.
fn scalar(
    vt: VARENUM,
    fill: impl FnOnce(&mut windows::Win32::System::Variant::VARIANT_0_0_0),
) -> VARIANT {
    let mut inner = VARIANT_0_0 {
        vt,
        ..Default::default()
    };
    fill(&mut inner.Anonymous);
    let mut v = VARIANT::default();
    v.Anonymous.Anonymous = ManuallyDrop::new(inner);
    v
}
