use std::{
    ffi::{CStr, CString},
    ptr::NonNull,
};

use anyhow::{Context as _, Result, bail};
use ohos_sys::{
    pasteboard,
    udmf::{data_management_framework as udmf, data_struct as uds},
};
use ohos_sys_opaque_types::{OH_UdmfData, OH_UdmfRecord};

pub(crate) fn read_text() -> Result<Option<String>> {
    let pasteboard = Pasteboard::new()?;
    // SAFETY: `pasteboard` owns a live OH_Pasteboard and the MIME pointer is
    // static for the duration of the call.
    if !unsafe {
        pasteboard::OH_Pasteboard_HasType(
            pasteboard.0.as_ptr(),
            pasteboard::PASTEBOARD_MIMETYPE_TEXT_PLAIN.as_ptr(),
        )
    } {
        return Ok(None);
    }

    let mut status = pasteboard::PASTEBOARD_ErrCode::OK.0 as i32;
    // SAFETY: The pasteboard pointer and status out pointer are both valid.
    let data = unsafe { pasteboard::OH_Pasteboard_GetData(pasteboard.0.as_ptr(), &mut status) };
    ensure_pasteboard_status("reading pasteboard data", status)?;
    let data = UdmfData::from_ptr(data).context("HarmonyOS returned null pasteboard data")?;
    let plain_text = PlainText::new()?;
    // SAFETY: Both UDMF objects are live and owned by these wrappers.
    let status =
        unsafe { udmf::OH_UdmfData_GetPrimaryPlainText(data.0.as_ptr(), plain_text.0.as_ptr()) };
    ensure_udmf_status("extracting plain text from pasteboard data", status)?;
    // SAFETY: The returned string belongs to `plain_text` and is copied before
    // that object is destroyed.
    let content = unsafe { uds::OH_UdsPlainText_GetContent(plain_text.0.as_ptr()) };
    let content =
        NonNull::new(content.cast_mut()).context("HarmonyOS returned null pasteboard text")?;
    // SAFETY: UDMF returns a null-terminated UTF-8 string for plain-text data.
    let content = unsafe { CStr::from_ptr(content.as_ptr()) }
        .to_str()
        .context("HarmonyOS pasteboard text is not valid UTF-8")?
        .to_owned();
    Ok(Some(content))
}

pub(crate) fn write_text(text: &str) -> Result<()> {
    let text = CString::new(text).context("clipboard text contains an interior null byte")?;
    let pasteboard = Pasteboard::new()?;
    let data = UdmfData::new()?;
    let record = UdmfRecord::new()?;
    let plain_text = PlainText::new()?;

    // SAFETY: `plain_text` is live and `text` remains valid for the call.
    let status = unsafe { uds::OH_UdsPlainText_SetContent(plain_text.0.as_ptr(), text.as_ptr()) };
    ensure_udmf_status("setting UDMF plain-text content", status)?;
    // SAFETY: Both objects are live for the call and until SetData completes.
    let status =
        unsafe { udmf::OH_UdmfRecord_AddPlainText(record.0.as_ptr(), plain_text.0.as_ptr()) };
    ensure_udmf_status("adding plain text to a UDMF record", status)?;
    // SAFETY: Both objects are live for the call and until SetData completes.
    let status = unsafe { udmf::OH_UdmfData_AddRecord(data.0.as_ptr(), record.0.as_ptr()) };
    ensure_udmf_status("adding a record to UDMF data", status)?;
    // SAFETY: The pasteboard and UDMF data remain live for the call. The
    // pasteboard service copies the supplied data before returning.
    let status =
        unsafe { pasteboard::OH_Pasteboard_SetData(pasteboard.0.as_ptr(), data.0.as_ptr()) };
    ensure_pasteboard_status("writing pasteboard data", status)
}

fn ensure_pasteboard_status(operation: &str, status: i32) -> Result<()> {
    if status != pasteboard::PASTEBOARD_ErrCode::OK.0 as i32 {
        bail!("{operation} failed with pasteboard status {status}");
    }
    Ok(())
}

fn ensure_udmf_status(operation: &str, status: i32) -> Result<()> {
    if status != ohos_sys::udmf::Udmf_ErrCode::E_OK.0 as i32 {
        bail!("{operation} failed with UDMF status {status}");
    }
    Ok(())
}

struct Pasteboard(NonNull<pasteboard::OH_Pasteboard>);

impl Pasteboard {
    fn new() -> Result<Self> {
        // SAFETY: Creates a new pasteboard handle owned by the caller.
        let pasteboard = unsafe { pasteboard::OH_Pasteboard_Create() };
        Ok(Self(NonNull::new(pasteboard).context(
            "HarmonyOS could not create a pasteboard handle",
        )?))
    }
}

impl Drop for Pasteboard {
    fn drop(&mut self) {
        // SAFETY: This wrapper uniquely owns the live pasteboard handle.
        unsafe { pasteboard::OH_Pasteboard_Destroy(self.0.as_ptr()) };
    }
}

struct UdmfData(NonNull<OH_UdmfData>);

impl UdmfData {
    fn new() -> Result<Self> {
        // SAFETY: Creates a new UDMF data object owned by the caller.
        let data = unsafe { udmf::OH_UdmfData_Create() };
        Self::from_ptr(data).context("HarmonyOS could not create UDMF data")
    }

    fn from_ptr(data: *mut OH_UdmfData) -> Option<Self> {
        NonNull::new(data).map(Self)
    }
}

impl Drop for UdmfData {
    fn drop(&mut self) {
        // SAFETY: This wrapper uniquely owns the live UDMF data object.
        unsafe { udmf::OH_UdmfData_Destroy(self.0.as_ptr()) };
    }
}

struct UdmfRecord(NonNull<OH_UdmfRecord>);

impl UdmfRecord {
    fn new() -> Result<Self> {
        // SAFETY: Creates a new UDMF record owned by the caller.
        let record = unsafe { udmf::OH_UdmfRecord_Create() };
        Ok(Self(
            NonNull::new(record).context("HarmonyOS could not create a UDMF record")?,
        ))
    }
}

impl Drop for UdmfRecord {
    fn drop(&mut self) {
        // SAFETY: This wrapper uniquely owns the live UDMF record.
        unsafe { udmf::OH_UdmfRecord_Destroy(self.0.as_ptr()) };
    }
}

struct PlainText(NonNull<uds::OH_UdsPlainText>);

impl PlainText {
    fn new() -> Result<Self> {
        // SAFETY: Creates a new UDS plain-text object owned by the caller.
        let plain_text = unsafe { uds::OH_UdsPlainText_Create() };
        Ok(Self(NonNull::new(plain_text).context(
            "HarmonyOS could not create a UDS plain-text object",
        )?))
    }
}

impl Drop for PlainText {
    fn drop(&mut self) {
        // SAFETY: This wrapper uniquely owns the live UDS plain-text object.
        unsafe { uds::OH_UdsPlainText_Destroy(self.0.as_ptr()) };
    }
}
