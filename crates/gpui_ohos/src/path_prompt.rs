use std::{
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    ptr,
};

use anyhow::{Context as _, Result, bail, ensure};

#[repr(C)]
struct FileSharePolicyInfo {
    uri: *mut c_char,
    length: u32,
    operation_mode: u32,
}

#[repr(C)]
struct FileSharePolicyErrorResult {
    uri: *mut c_char,
    code: u32,
    message: *mut c_char,
}

#[link(name = "ohfileuri")]
unsafe extern "C" {
    fn OH_FileUri_GetPathFromUri(uri: *const c_char, length: u32, result: *mut *mut c_char) -> i32;
}

#[link(name = "ohfileshare")]
unsafe extern "C" {
    fn OH_FileShare_PersistPermission(
        policies: *const FileSharePolicyInfo,
        policy_count: u32,
        errors: *mut *mut FileSharePolicyErrorResult,
        error_count: *mut u32,
    ) -> i32;
    fn OH_FileShare_ActivatePermission(
        policies: *const FileSharePolicyInfo,
        policy_count: u32,
        errors: *mut *mut FileSharePolicyErrorResult,
        error_count: *mut u32,
    ) -> i32;
    fn OH_FileShare_ReleasePolicyErrorResult(
        errors: *mut FileSharePolicyErrorResult,
        error_count: u32,
    );
}

const READ_WRITE_MODE: u32 = 3;

pub(crate) fn persist_and_resolve_uris(uris: &[String]) -> Result<Vec<(String, PathBuf)>> {
    persist_permissions(uris)?;
    uris.iter()
        .map(|uri| Ok((uri.clone(), resolve_uri(uri)?)))
        .collect()
}

pub fn activate_persistent_uri(uri: &str) -> Result<PathBuf> {
    let uris = [uri.to_owned()];
    let (uri_strings, policies) = build_policies(&uris)?;
    let _uri_strings = uri_strings;
    apply_permissions(
        &policies,
        OH_FileShare_ActivatePermission,
        "activating persistent path access",
    )?;
    resolve_uri(uri)
}

pub fn resolve_incoming_uri(uri: &str) -> Result<PathBuf> {
    resolve_uri(uri)
}

fn persist_permissions(uris: &[String]) -> Result<()> {
    let (uri_strings, policies) = build_policies(uris)?;
    let _uri_strings = uri_strings;
    apply_permissions(
        &policies,
        OH_FileShare_PersistPermission,
        "persisting selected path access",
    )
}

fn build_policies(uris: &[String]) -> Result<(Vec<CString>, Vec<FileSharePolicyInfo>)> {
    let uri_strings = uris
        .iter()
        .map(|uri| CString::new(uri.as_str()).context("selected URI contains an interior NUL"))
        .collect::<Result<Vec<_>>>()?;
    let policies = uri_strings
        .iter()
        .map(|uri| {
            Ok(FileSharePolicyInfo {
                uri: uri.as_ptr().cast_mut(),
                length: u32::try_from(uri.as_bytes().len()).context("selected URI is too long")?,
                operation_mode: READ_WRITE_MODE,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((uri_strings, policies))
}

fn apply_permissions(
    policies: &[FileSharePolicyInfo],
    apply: unsafe extern "C" fn(
        *const FileSharePolicyInfo,
        u32,
        *mut *mut FileSharePolicyErrorResult,
        *mut u32,
    ) -> i32,
    operation: &str,
) -> Result<()> {
    let mut errors = ptr::null_mut();
    let mut error_count = 0;
    // SAFETY: `policies` and its backing C strings remain alive for the call,
    // and both output pointers refer to initialized writable storage.
    let status = unsafe {
        apply(
            policies.as_ptr(),
            u32::try_from(policies.len()).context("too many selected paths")?,
            &mut errors,
            &mut error_count,
        )
    };
    let policy_errors = format_policy_errors(errors, error_count);
    if !errors.is_null() {
        // SAFETY: The FileShare API owns this result and requires this matching
        // release function with the returned item count.
        unsafe { OH_FileShare_ReleasePolicyErrorResult(errors, error_count) };
    }
    ensure!(
        status == 0,
        "{operation} failed with HarmonyOS status {status}"
    );
    ensure!(
        policy_errors.is_empty(),
        "HarmonyOS rejected {operation}: {}",
        policy_errors.join("; ")
    );
    Ok(())
}

fn format_policy_errors(errors: *mut FileSharePolicyErrorResult, error_count: u32) -> Vec<String> {
    if errors.is_null() || error_count == 0 {
        return Vec::new();
    }
    let Ok(error_count) = usize::try_from(error_count) else {
        return vec!["invalid error count returned by HarmonyOS".to_owned()];
    };
    // SAFETY: HarmonyOS returned this pointer with `error_count` initialized
    // entries, and it remains valid until the caller releases it.
    let errors = unsafe { std::slice::from_raw_parts(errors, error_count) };
    errors
        .iter()
        .map(|error| {
            let uri = c_string_lossy(error.uri);
            let message = c_string_lossy(error.message);
            format!("{uri}: {message} (policy code {})", error.code)
        })
        .collect()
}

fn c_string_lossy(value: *const c_char) -> String {
    if value.is_null() {
        return "<missing>".to_owned();
    }
    // SAFETY: FileShare returns NUL-terminated strings for non-null URI and
    // message pointers within each live error result.
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn resolve_uri(uri: &str) -> Result<PathBuf> {
    if !uri.contains("://") {
        let path = PathBuf::from(uri);
        ensure!(path.is_absolute(), "picker returned relative path {uri:?}");
        return Ok(path);
    }

    let uri = CString::new(uri).context("selected URI contains an interior NUL")?;
    let length = u32::try_from(uri.as_bytes().len()).context("selected URI is too long")?;
    let mut path = ptr::null_mut();
    // SAFETY: `uri` is a live NUL-terminated string, and `path` is a valid out
    // pointer. The API allocates a C string that must be released with free().
    let status = unsafe { OH_FileUri_GetPathFromUri(uri.as_ptr(), length, &mut path) };
    if status != 0 {
        bail!("converting selected URI to a path failed with HarmonyOS status {status}");
    }
    ensure!(!path.is_null(), "HarmonyOS returned a null selected path");
    // SAFETY: A successful conversion returns a NUL-terminated allocation.
    let path_string = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: OH_FileUri_GetPathFromUri documents that its successful result
    // is allocated with malloc and must be released using free().
    unsafe { libc::free(path.cast()) };
    let path = PathBuf::from(path_string);
    ensure!(path.is_absolute(), "picker returned relative path {path:?}");
    Ok(path)
}
