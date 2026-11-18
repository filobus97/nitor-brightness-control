//! Minimal WMI client for the built-in display (`root\WMI` brightness classes).
//! Built-in panels do not speak DDC/CI; Windows exposes them through
//! `WmiMonitorBrightness` (read) and `WmiMonitorBrightnessMethods.WmiSetBrightness` (write).

use std::mem::ManuallyDrop;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0_0, VT_BSTR, VT_I4, VT_UI1, VariantClear,
};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemContext, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE, WbemLocator,
};
use windows::core::{BSTR, PCWSTR, w};

/// A connection to the `ROOT\WMI` namespace. Use it only on the thread that created it.
pub struct Wmi {
    services: IWbemServices,
}

impl Wmi {
    /// Connect to `ROOT\WMI`. COM must already be initialized on this thread.
    pub fn connect() -> windows::core::Result<Wmi> {
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
            Ok(Wmi { services })
        }
    }

    /// Current built-in brightness in percent, if an active panel reports one.
    pub fn read(&self) -> Option<u8> {
        let obj =
            self.first("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE")?;
        let mut v = VARIANT::default();
        // SAFETY: `v` is a valid out-variant; it is read by its type tag, then cleared.
        unsafe {
            obj.Get(w!("CurrentBrightness"), 0, &mut v, None, None)
                .ok()?;
            let inner = &v.Anonymous.Anonymous;
            let out = match inner.vt {
                VT_UI1 => Some(inner.Anonymous.bVal),
                VT_I4 => u8::try_from(inner.Anonymous.lVal).ok(),
                _ => None,
            };
            let _ = VariantClear(&mut v);
            out.map(|p| p.min(100))
        }
    }

    /// Set the built-in brightness in percent. Returns whether WMI accepted the call.
    pub fn set(&self, percent: u8) -> bool {
        self.try_set(percent).is_some()
    }

    fn try_set(&self, percent: u8) -> Option<()> {
        // The method must be executed on an instance, addressed by its `__PATH`.
        let target =
            self.first("SELECT __PATH FROM WmiMonitorBrightnessMethods WHERE Active=TRUE")?;
        let mut path = VARIANT::default();
        // SAFETY: COM calls on live interfaces; variants are cleared or hold no heap data.
        unsafe {
            target.Get(w!("__PATH"), 0, &mut path, None, None).ok()?;
            // Copy the BSTR out so clearing the variant frees the original.
            let path_str = (path.Anonymous.Anonymous.vt == VT_BSTR)
                .then(|| (*path.Anonymous.Anonymous.Anonymous.bstrVal).clone());
            let _ = VariantClear(&mut path);
            let path_str = path_str?;

            // In-parameters come from the class definition, not from the instance.
            let mut class = None;
            self.services
                .GetObject(
                    &BSTR::from("WmiMonitorBrightnessMethods"),
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None::<&IWbemContext>,
                    Some(&mut class),
                    None,
                )
                .ok()?;
            let mut in_sig: Option<IWbemClassObject> = None;
            class?
                .GetMethod(w!("WmiSetBrightness"), 0, &mut in_sig, std::ptr::null_mut())
                .ok()?;
            let params = in_sig?.SpawnInstance(0).ok()?;
            // Timeout is a uint32 (VT_I4 is coerced), Brightness a uint8 (VT_UI1).
            params
                .Put(w!("Timeout"), 0, &scalar(VT_I4, |v| v.lVal = 0), 0)
                .ok()?;
            params
                .Put(
                    w!("Brightness"),
                    0,
                    &scalar(VT_UI1, |v| v.bVal = percent.min(100)),
                    0,
                )
                .ok()?;
            self.services
                .ExecMethod(
                    &path_str,
                    &BSTR::from("WmiSetBrightness"),
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None::<&IWbemContext>,
                    &params,
                    None,
                    None,
                )
                .ok()
        }
    }

    /// First object of a WQL query.
    fn first(&self, wql: &str) -> Option<IWbemClassObject> {
        // SAFETY: the enumerator and the one-element out-array are valid for the call.
        unsafe {
            let en = self
                .services
                .ExecQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from(wql),
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None::<&IWbemContext>,
                )
                .ok()?;
            let mut objs = [None];
            let mut n = 0u32;
            en.Next(WBEM_INFINITE, &mut objs, &mut n).ok().ok()?;
            let [obj] = objs;
            if n == 1 { obj } else { None }
        }
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
