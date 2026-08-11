use std::{
    ffi::{CStr, c_char},
    ptr::NonNull,
    sync::OnceLock,
};

use anyhow::{Context as _, Result, bail};

const SUCCESS: i32 = 0;

#[repr(C)]
struct AppEventInfo {
    domain: *const c_char,
    name: *const c_char,
    event_type: i32,
    params: *const c_char,
}

#[repr(C)]
struct AppEventGroup {
    name: *const c_char,
    event_infos: *const AppEventInfo,
    info_len: u32,
}

#[repr(C)]
struct NativeWatcher([u8; 0]);

#[link(name = "hiappevent_ndk.z")]
unsafe extern "C" {
    fn OH_HiAppEvent_CreateWatcher(name: *const c_char) -> *mut NativeWatcher;
    fn OH_HiAppEvent_DestroyWatcher(watcher: *mut NativeWatcher);
    fn OH_HiAppEvent_SetAppEventFilter(
        watcher: *mut NativeWatcher,
        domain: *const c_char,
        event_types: u8,
        names: *const *const c_char,
        names_len: i32,
    ) -> i32;
    fn OH_HiAppEvent_SetWatcherOnReceive(
        watcher: *mut NativeWatcher,
        callback: Option<unsafe extern "C" fn(*const c_char, *const AppEventGroup, u32)>,
    ) -> i32;
    fn OH_HiAppEvent_AddWatcher(watcher: *mut NativeWatcher) -> i32;
}

struct CrashWatcher {
    _native: NonNull<NativeWatcher>,
}

// The system can call the process-lifetime watcher from a DFX worker thread.
unsafe impl Send for CrashWatcher {}
unsafe impl Sync for CrashWatcher {}

pub(crate) fn install() -> Result<()> {
    static WATCHER: OnceLock<Result<CrashWatcher, String>> = OnceLock::new();
    match WATCHER.get_or_init(|| create_watcher().map_err(|error| format!("{error:#}"))) {
        Ok(_) => Ok(()),
        Err(error) => bail!("{error}"),
    }
}

fn create_watcher() -> Result<CrashWatcher> {
    // SAFETY: The returned watcher is owned by this process-lifetime wrapper.
    let watcher = NonNull::new(unsafe { OH_HiAppEvent_CreateWatcher(c"ZedCrashWatcher".as_ptr()) })
        .context("HiAppEvent could not create the crash watcher")?;
    let event_names = [c"APP_CRASH".as_ptr()];

    // SAFETY: All pointers are valid for the calls. HiAppEvent copies the
    // filter strings and retains only the static callback function pointer.
    let filter_status = unsafe {
        OH_HiAppEvent_SetAppEventFilter(
            watcher.as_ptr(),
            c"OS".as_ptr(),
            0x01,
            event_names.as_ptr(),
            1,
        )
    };
    if filter_status != SUCCESS {
        // SAFETY: The watcher has not been added and is uniquely owned here.
        unsafe { OH_HiAppEvent_DestroyWatcher(watcher.as_ptr()) };
        bail!("setting the HiAppEvent crash filter failed with status {filter_status}");
    }
    let callback_status =
        unsafe { OH_HiAppEvent_SetWatcherOnReceive(watcher.as_ptr(), Some(receive_crash_events)) };
    if callback_status != SUCCESS {
        // SAFETY: The watcher has not been added and is uniquely owned here.
        unsafe { OH_HiAppEvent_DestroyWatcher(watcher.as_ptr()) };
        bail!("setting the HiAppEvent crash callback failed with status {callback_status}");
    }
    let add_status = unsafe { OH_HiAppEvent_AddWatcher(watcher.as_ptr()) };
    if add_status != SUCCESS {
        // SAFETY: AddWatcher failed, so the watcher is still uniquely owned.
        unsafe { OH_HiAppEvent_DestroyWatcher(watcher.as_ptr()) };
        bail!("adding the HiAppEvent crash watcher failed with status {add_status}");
    }

    Ok(CrashWatcher { _native: watcher })
}

unsafe extern "C" fn receive_crash_events(
    domain: *const c_char,
    groups: *const AppEventGroup,
    group_len: u32,
) {
    let Some(domain) = c_string(domain) else {
        return;
    };
    let Ok(group_len) = usize::try_from(group_len) else {
        return;
    };
    let Some(groups) = NonNull::new(groups.cast_mut()) else {
        return;
    };
    // SAFETY: HiAppEvent guarantees `group_len` live entries for this callback.
    for group in unsafe { std::slice::from_raw_parts(groups.as_ptr(), group_len) } {
        let Ok(info_len) = usize::try_from(group.info_len) else {
            continue;
        };
        let Some(infos) = NonNull::new(group.event_infos.cast_mut()) else {
            continue;
        };
        // SAFETY: Each group guarantees `info_len` live entries for this callback.
        for info in unsafe { std::slice::from_raw_parts(infos.as_ptr(), info_len) } {
            let Some(name) = c_string(info.name) else {
                continue;
            };
            gpui_ohos::log_message(
                gpui_ohos::LogLevel::Error,
                format!(
                    "HarmonyOS FaultLogger reported {domain}/{name} (type {})",
                    info.event_type
                ),
            );
        }
    }
}

fn c_string(value: *const c_char) -> Option<String> {
    let value = NonNull::new(value.cast_mut())?;
    // SAFETY: HiAppEvent exposes null-terminated strings during the callback.
    Some(
        unsafe { CStr::from_ptr(value.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}
