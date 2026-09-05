//! Late-bound IDispatch helpers. Windows only. No PowerShell.

use anyhow::{anyhow, bail, Result};
use windows::core::{Interface, BSTR, GUID, IUnknown, PCWSTR, VARIANT};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CLSIDFromProgID, COINIT_APARTMENTTHREADED, DISPATCH_FLAGS, DISPATCH_METHOD,
    DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, EXCEPINFO, IDispatch,
};
use windows::Win32::System::Ole::DISPID_PROPERTYPUT;

pub struct ComInit;

impl ComInit {
    pub fn new() -> Result<Self> {
        let hr = unsafe { windows::Win32::System::Com::CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        hr.ok()?;
        Ok(Self)
    }
}

impl Drop for ComInit {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::Com::CoUninitialize();
        }
    }
}

pub fn create_progid(progid: &str) -> Result<IDispatch> {
    let wide: Vec<u16> = progid.encode_utf16().chain(std::iter::once(0)).collect();
    let clsid = unsafe { CLSIDFromProgID(PCWSTR(wide.as_ptr())) }?;
    let disp: IDispatch = unsafe { windows::Win32::System::Com::CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER) }?;
    Ok(disp)
}

fn dispid(obj: &IDispatch, name: &str) -> Result<i32> {
    let mut wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let ptr = PCWSTR(wide.as_mut_ptr());
    let mut id = 0i32;
    unsafe {
        obj.GetIDsOfNames(&GUID::zeroed(), &ptr, 1, 0, &mut id)?;
    }
    Ok(id)
}

fn invoke(obj: &IDispatch, name: &str, flags: DISPATCH_FLAGS, args: &[VARIANT]) -> Result<VARIANT> {
    let id = dispid(obj, name)?;
    let mut args_rev: Vec<VARIANT> = args.iter().rev().cloned().collect();
    let mut named = DISPID_PROPERTYPUT;
    let mut params = DISPPARAMS {
        rgvarg: if args_rev.is_empty() {
            std::ptr::null_mut()
        } else {
            args_rev.as_mut_ptr()
        },
        rgdispidNamedArgs: std::ptr::null_mut(),
        cArgs: args_rev.len() as u32,
        cNamedArgs: 0,
    };
    if flags == DISPATCH_PROPERTYPUT {
        params.rgdispidNamedArgs = &mut named;
        params.cNamedArgs = 1;
    }
    let mut result = VARIANT::new();
    let mut excep = EXCEPINFO::default();
    let mut argerr = 0u32;
    unsafe {
        obj.Invoke(
            id,
            &GUID::zeroed(),
            0,
            flags,
            &params,
            Some(&mut result),
            Some(&mut excep),
            Some(&mut argerr),
        )?;
    }
    Ok(result)
}

pub fn get(obj: &IDispatch, name: &str) -> Result<VARIANT> {
    invoke(obj, name, DISPATCH_PROPERTYGET, &[])
}

pub fn call(obj: &IDispatch, name: &str, args: &[VARIANT]) -> Result<VARIANT> {
    invoke(obj, name, DISPATCH_METHOD, args)
}

pub fn put(obj: &IDispatch, name: &str, value: VARIANT) -> Result<()> {
    let _ = invoke(obj, name, DISPATCH_PROPERTYPUT, &[value])?;
    Ok(())
}

pub fn as_dispatch(v: &VARIANT) -> Result<IDispatch> {
    unsafe {
        let raw = v.as_raw();
        let vt = raw.Anonymous.Anonymous.vt;
        let p = if vt == 9 || vt == 13 {
            // VT_DISPATCH or VT_UNKNOWN
            raw.Anonymous.Anonymous.Anonymous.pdispVal
        } else {
            bail!("VARIANT is not IDispatch (vt={vt})");
        };
        if p.is_null() {
            bail!("null IDispatch");
        }
        let unk = IUnknown::from_raw_borrowed(&p).ok_or_else(|| anyhow!("not IUnknown"))?;
        unk.cast::<IDispatch>().map_err(|e| anyhow!("{e}"))
    }
}

pub fn as_i32(v: &VARIANT) -> Result<i32> {
    i32::try_from(v).map_err(|e| anyhow!("{e}"))
}

pub fn as_bool(v: &VARIANT) -> Result<bool> {
    bool::try_from(v).map_err(|e| anyhow!("{e}"))
}

pub fn as_f64(v: &VARIANT) -> Result<f64> {
    f64::try_from(v).map_err(|e| anyhow!("{e}"))
}

pub fn as_string(v: &VARIANT) -> Result<String> {
    let b = BSTR::try_from(v).map_err(|e| anyhow!("{e}"))?;
    Ok(b.to_string())
}

pub fn var_i32(n: i32) -> VARIANT {
    VARIANT::from(n)
}

pub fn var_bool(b: bool) -> VARIANT {
    VARIANT::from(b)
}

pub fn var_bstr(s: &str) -> VARIANT {
    VARIANT::from(s)
}

/// Outlook DATE (days since 1899-12-30) to local DateTime.
pub fn date_serial_to_local(serial: f64) -> chrono::DateTime<chrono::Local> {
    use chrono::{Duration, Local, NaiveDate, TimeZone};
    let days = serial.floor() as i64;
    let frac = serial - serial.floor();
    let base = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap();
    let date = base + Duration::days(days);
    let secs = (frac * 86400.0).round() as i64;
    let ndt = date.and_hms_opt(0, 0, 0).unwrap() + Duration::seconds(secs);
    Local.from_local_datetime(&ndt).single().unwrap_or_else(|| {
        chrono::DateTime::from_naive_utc_and_offset(ndt, *Local::now().offset())
    })
}


