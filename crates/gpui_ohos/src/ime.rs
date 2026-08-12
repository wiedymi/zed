use std::{
    cmp,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    slice,
};

use anyhow::{Result, anyhow};
use ohos_sys::inputmethod::{
    attach_options::{
        InputMethod_AttachOptions, OH_AttachOptions_CreateWithRequestKeyboardReason,
        OH_AttachOptions_Destroy,
    },
    controller::{
        OH_InputMethodController_Attach, OH_InputMethodController_AttachWithUIContext,
        OH_InputMethodController_Detach,
    },
    cursor_info::OH_CursorInfo_SetRect,
    inputmethod_proxy::{
        InputMethod_InputMethodProxy, OH_InputMethodProxy_HideKeyboard,
        OH_InputMethodProxy_ShowTextInput,
    },
    text_config::{
        InputMethod_TextConfig, OH_TextConfig_GetCursorInfo, OH_TextConfig_SetEnterKeyType,
        OH_TextConfig_SetInputType, OH_TextConfig_SetPreviewTextSupport,
        OH_TextConfig_SetSelection,
    },
    text_editor_proxy::{
        InputMethod_TextEditorProxy, OH_TextEditorProxy_Create, OH_TextEditorProxy_Destroy,
        OH_TextEditorProxy_SetCallbackInMainThread, OH_TextEditorProxy_SetDeleteBackwardFunc,
        OH_TextEditorProxy_SetDeleteForwardFunc, OH_TextEditorProxy_SetFinishTextPreviewFunc,
        OH_TextEditorProxy_SetGetLeftTextOfCursorFunc,
        OH_TextEditorProxy_SetGetRightTextOfCursorFunc, OH_TextEditorProxy_SetGetTextConfigFunc,
        OH_TextEditorProxy_SetGetTextIndexAtCursorFunc,
        OH_TextEditorProxy_SetHandleExtendActionFunc, OH_TextEditorProxy_SetHandleSetSelectionFunc,
        OH_TextEditorProxy_SetInsertTextFunc, OH_TextEditorProxy_SetMoveCursorFunc,
        OH_TextEditorProxy_SetReceivePrivateCommandFunc, OH_TextEditorProxy_SetSendEnterKeyFunc,
        OH_TextEditorProxy_SetSendKeyboardStatusFunc, OH_TextEditorProxy_SetSetPreviewTextFunc,
    },
    types::{
        InputMethod_Direction, InputMethod_EnterKeyType, InputMethod_ExtendAction,
        InputMethod_KeyboardStatus, InputMethod_RequestKeyboardReason, InputMethod_TextInputType,
        InputMethodResult,
    },
};

use crate::{
    LogLevel, log_message,
    platform::{
        ime_candidate_rect, ime_delete, ime_finish_composition, ime_insert_text, ime_move_cursor,
        ime_perform_action, ime_selection, ime_set_composition, ime_set_selection,
        ime_text_for_range, ime_text_length,
    },
};

/// Owns the HarmonyOS input-method connection for the GPUI text input handler.
pub(crate) struct NativeIme {
    editor_proxy: NonNull<InputMethod_TextEditorProxy>,
    input_method_proxy: Option<NonNull<InputMethod_InputMethodProxy>>,
    ui_context: NonNull<c_void>,
}

impl NativeIme {
    pub(crate) fn new(ui_context: NonNull<c_void>) -> Result<Self> {
        // SAFETY: The function allocates an opaque proxy or returns null.
        let editor_proxy = NonNull::new(unsafe { OH_TextEditorProxy_Create() })
            .ok_or_else(|| anyhow!("OH_TextEditorProxy_Create returned null"))?;

        let result = (|| {
            register_callback(
                "OH_TextEditorProxy_SetGetTextConfigFunc",
                // SAFETY: `editor_proxy` is live and the callback has static lifetime.
                unsafe {
                    OH_TextEditorProxy_SetGetTextConfigFunc(
                        editor_proxy.as_ptr(),
                        Some(get_text_config),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetInsertTextFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetInsertTextFunc(editor_proxy.as_ptr(), Some(insert_text))
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetDeleteForwardFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetDeleteForwardFunc(
                        editor_proxy.as_ptr(),
                        Some(delete_forward),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetDeleteBackwardFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetDeleteBackwardFunc(
                        editor_proxy.as_ptr(),
                        Some(delete_backward),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetSendKeyboardStatusFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetSendKeyboardStatusFunc(
                        editor_proxy.as_ptr(),
                        Some(send_keyboard_status),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetSendEnterKeyFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetSendEnterKeyFunc(
                        editor_proxy.as_ptr(),
                        Some(send_enter_key),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetMoveCursorFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetMoveCursorFunc(editor_proxy.as_ptr(), Some(move_cursor))
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetHandleSetSelectionFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetHandleSetSelectionFunc(
                        editor_proxy.as_ptr(),
                        Some(handle_set_selection),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetHandleExtendActionFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetHandleExtendActionFunc(
                        editor_proxy.as_ptr(),
                        Some(handle_extend_action),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetGetLeftTextOfCursorFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetGetLeftTextOfCursorFunc(
                        editor_proxy.as_ptr(),
                        Some(get_left_text),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetGetRightTextOfCursorFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetGetRightTextOfCursorFunc(
                        editor_proxy.as_ptr(),
                        Some(get_right_text),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetGetTextIndexAtCursorFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetGetTextIndexAtCursorFunc(
                        editor_proxy.as_ptr(),
                        Some(get_text_index),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetReceivePrivateCommandFunc",
                // SAFETY: Same proxy and callback lifetime contract. Zed does
                // not currently define vendor-private IME commands, but the
                // controller requires every proxy callback to be installed.
                unsafe {
                    OH_TextEditorProxy_SetReceivePrivateCommandFunc(
                        editor_proxy.as_ptr(),
                        Some(receive_private_command),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetSetPreviewTextFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetSetPreviewTextFunc(
                        editor_proxy.as_ptr(),
                        Some(set_preview_text),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetFinishTextPreviewFunc",
                // SAFETY: Same proxy and callback lifetime contract.
                unsafe {
                    OH_TextEditorProxy_SetFinishTextPreviewFunc(
                        editor_proxy.as_ptr(),
                        Some(finish_text_preview),
                    )
                },
            )?;
            register_callback(
                "OH_TextEditorProxy_SetCallbackInMainThread",
                // SAFETY: The proxy is live. Main-thread callbacks are required
                // because GPUI's platform and window state are UI-thread owned.
                unsafe { OH_TextEditorProxy_SetCallbackInMainThread(editor_proxy.as_ptr(), true) },
            )?;

            Ok(Self {
                editor_proxy,
                input_method_proxy: None,
                ui_context,
            })
        })();

        if result.is_err() {
            // SAFETY: The proxy was allocated above and ownership was not
            // transferred to the returned Rust object on the error path.
            unsafe { OH_TextEditorProxy_Destroy(editor_proxy.as_ptr()) };
        }
        result
    }

    pub(crate) fn show_for_touch(&mut self) -> Result<()> {
        if self.input_method_proxy.is_none() {
            self.attach(
                true,
                InputMethod_RequestKeyboardReason::IME_REQUEST_REASON_TOUCH,
            )?;
            return Ok(());
        }
        let input_method_proxy = self
            .input_method_proxy
            .ok_or_else(|| anyhow!("input method is not attached"))?;
        let options = AttachOptions::with_reason(
            true,
            InputMethod_RequestKeyboardReason::IME_REQUEST_REASON_TOUCH,
        )?;
        register_callback(
            "OH_InputMethodProxy_ShowTextInput",
            // SAFETY: Both opaque handles remain live for `self` and `options`.
            unsafe {
                OH_InputMethodProxy_ShowTextInput(input_method_proxy.as_ptr(), options.as_ptr())
            },
        )
    }

    pub(crate) fn hide(&self) {
        let Some(input_method_proxy) = self.input_method_proxy else {
            return;
        };
        // SAFETY: The proxy remains live until detach in `Drop`.
        if let Err(error) = unsafe { OH_InputMethodProxy_HideKeyboard(input_method_proxy.as_ptr()) }
        {
            log_message(
                LogLevel::Warning,
                format!("OH_InputMethodProxy_HideKeyboard failed: {error:?}"),
            );
        }
    }

    fn attach(
        &mut self,
        show_keyboard: bool,
        reason: InputMethod_RequestKeyboardReason,
    ) -> Result<()> {
        let options = AttachOptions::with_reason(show_keyboard, reason)?;
        let mut input_method_proxy = std::ptr::null_mut();
        // API 23's context-aware path is preferred for multi-container
        // applications. Some emulator images reject it while the input method
        // service is starting; a later touch can safely retry this operation.
        // SAFETY: ArkUI returned `ui_context` from the active page, the editor
        // proxy is live, and the out pointer is valid.
        let context_attach = unsafe {
            OH_InputMethodController_AttachWithUIContext(
                self.ui_context.as_ptr().cast(),
                self.editor_proxy.as_ptr(),
                options.as_ptr(),
                &mut input_method_proxy,
            )
        };
        if let Err(error) = context_attach {
            log_message(
                LogLevel::Warning,
                format!(
                    "context-aware IME attach failed ({error:?}); using the single-window attach API"
                ),
            );
            input_method_proxy = std::ptr::null_mut();
            // SAFETY: This process owns one ArkUI window, and all remaining
            // opaque handles have the same validity contract as above.
            register_callback("OH_InputMethodController_Attach", unsafe {
                OH_InputMethodController_Attach(
                    self.editor_proxy.as_ptr(),
                    options.as_ptr(),
                    &mut input_method_proxy,
                )
            })?;
        }
        self.input_method_proxy = Some(
            NonNull::new(input_method_proxy)
                .ok_or_else(|| anyhow!("input method attach returned a null proxy"))?,
        );
        Ok(())
    }
}

impl Drop for NativeIme {
    fn drop(&mut self) {
        if let Some(input_method_proxy) = self.input_method_proxy {
            // SAFETY: The controller owns the returned proxy until this detach.
            if let Err(error) =
                unsafe { OH_InputMethodController_Detach(input_method_proxy.as_ptr()) }
            {
                log_message(
                    LogLevel::Warning,
                    format!("OH_InputMethodController_Detach failed: {error:?}"),
                );
            }
        }
        // SAFETY: Detach has ended all callbacks and this object uniquely owns
        // the editor proxy allocation.
        unsafe { OH_TextEditorProxy_Destroy(self.editor_proxy.as_ptr()) };
    }
}

struct AttachOptions(NonNull<InputMethod_AttachOptions>);

impl AttachOptions {
    fn with_reason(show_keyboard: bool, reason: InputMethod_RequestKeyboardReason) -> Result<Self> {
        // SAFETY: The function allocates an opaque options value or returns null.
        NonNull::new(unsafe {
            OH_AttachOptions_CreateWithRequestKeyboardReason(show_keyboard, reason)
        })
        .map(Self)
        .ok_or_else(|| anyhow!("OH_AttachOptions_CreateWithRequestKeyboardReason returned null"))
    }

    fn as_ptr(&self) -> *mut InputMethod_AttachOptions {
        self.0.as_ptr()
    }
}

impl Drop for AttachOptions {
    fn drop(&mut self) {
        // SAFETY: This object uniquely owns the options allocation.
        unsafe { OH_AttachOptions_Destroy(self.0.as_ptr()) };
    }
}

fn register_callback(operation: &str, result: InputMethodResult) -> Result<()> {
    result.map_err(|error| anyhow!("{operation} failed: {error:?}"))
}

fn guarded<R>(name: &str, failure: R, callback: impl FnOnce() -> Result<R>) -> R {
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            log_message(
                LogLevel::Error,
                format!("HarmonyOS IME callback {name} failed: {error:#}"),
            );
            failure
        }
        Err(_) => {
            log_message(
                LogLevel::Error,
                format!("HarmonyOS IME callback {name} panicked"),
            );
            failure
        }
    }
}

unsafe extern "C" fn get_text_config(
    _proxy: *mut InputMethod_TextEditorProxy,
    config: *mut InputMethod_TextConfig,
) {
    guarded("get_text_config", (), || {
        let config = NonNull::new(config).ok_or_else(|| anyhow!("null text config"))?;
        register_callback(
            "OH_TextConfig_SetInputType",
            // SAFETY: The config is borrowed for this callback.
            unsafe {
                OH_TextConfig_SetInputType(
                    config.as_ptr(),
                    InputMethod_TextInputType::IME_TEXT_INPUT_TYPE_MULTILINE,
                )
            },
        )?;
        register_callback(
            "OH_TextConfig_SetEnterKeyType",
            // SAFETY: The config is borrowed for this callback.
            unsafe {
                OH_TextConfig_SetEnterKeyType(
                    config.as_ptr(),
                    InputMethod_EnterKeyType::IME_ENTER_KEY_NEWLINE,
                )
            },
        )?;
        register_callback(
            "OH_TextConfig_SetPreviewTextSupport",
            // SAFETY: The config is borrowed for this callback.
            unsafe { OH_TextConfig_SetPreviewTextSupport(config.as_ptr(), true) },
        )?;

        if let Some(selection) = ime_selection() {
            let start = to_i32(selection.range.start);
            let end = to_i32(selection.range.end);
            register_callback(
                "OH_TextConfig_SetSelection",
                // SAFETY: The config is borrowed for this callback.
                unsafe { OH_TextConfig_SetSelection(config.as_ptr(), start, end) },
            )?;
        }

        if let Some((left, top, width, height)) = ime_candidate_rect() {
            let mut cursor = std::ptr::null_mut();
            register_callback(
                "OH_TextConfig_GetCursorInfo",
                // SAFETY: The config is borrowed and the out pointer is valid.
                unsafe { OH_TextConfig_GetCursorInfo(config.as_ptr(), &mut cursor) },
            )?;
            if let Some(cursor) = NonNull::new(cursor) {
                register_callback(
                    "OH_CursorInfo_SetRect",
                    // SAFETY: Cursor info is owned by the borrowed text config.
                    unsafe { OH_CursorInfo_SetRect(cursor.as_ptr(), left, top, width, height) },
                )?;
            }
        }
        Ok(())
    });
}

unsafe extern "C" fn insert_text(
    _proxy: *mut InputMethod_TextEditorProxy,
    text: *const u16,
    length: usize,
) {
    guarded("insert_text", (), || {
        let text = unsafe { decode_utf16(text, length) }?;
        ime_insert_text(&text);
        Ok(())
    });
}

unsafe extern "C" fn delete_forward(_proxy: *mut InputMethod_TextEditorProxy, length: i32) {
    guarded("delete_forward", (), || {
        ime_delete(true, nonnegative_usize(length));
        Ok(())
    });
}

unsafe extern "C" fn delete_backward(_proxy: *mut InputMethod_TextEditorProxy, length: i32) {
    guarded("delete_backward", (), || {
        ime_delete(false, nonnegative_usize(length));
        Ok(())
    });
}

unsafe extern "C" fn send_keyboard_status(
    _proxy: *mut InputMethod_TextEditorProxy,
    status: InputMethod_KeyboardStatus,
) {
    log_message(
        LogLevel::Debug,
        format!("HarmonyOS keyboard status changed to {}", status.0),
    );
}

unsafe extern "C" fn send_enter_key(
    _proxy: *mut InputMethod_TextEditorProxy,
    _enter_key_type: InputMethod_EnterKeyType,
) {
    guarded("send_enter_key", (), || {
        ime_insert_text("\n");
        Ok(())
    });
}

unsafe extern "C" fn move_cursor(
    _proxy: *mut InputMethod_TextEditorProxy,
    direction: InputMethod_Direction,
) {
    guarded("move_cursor", (), || {
        ime_move_cursor(direction.0);
        Ok(())
    });
}

unsafe extern "C" fn handle_set_selection(
    _proxy: *mut InputMethod_TextEditorProxy,
    start: i32,
    end: i32,
) {
    guarded("handle_set_selection", (), || {
        ime_set_selection(nonnegative_usize(start)..nonnegative_usize(end));
        Ok(())
    });
}

unsafe extern "C" fn handle_extend_action(
    _proxy: *mut InputMethod_TextEditorProxy,
    action: InputMethod_ExtendAction,
) {
    guarded("handle_extend_action", (), || {
        ime_perform_action(action.0);
        Ok(())
    });
}

unsafe extern "C" fn get_left_text(
    _proxy: *mut InputMethod_TextEditorProxy,
    number: i32,
    text: *mut u16,
    length: *mut usize,
) {
    guarded("get_left_text", (), || {
        let count = nonnegative_usize(number);
        let cursor = ime_selection().map_or(0, |selection| {
            if selection.reversed {
                selection.range.start
            } else {
                selection.range.end
            }
        });
        let start = cursor.saturating_sub(count);
        let value = ime_text_for_range(start..cursor).unwrap_or_default();
        // SAFETY: HarmonyOS supplies a buffer with capacity `number` and a
        // writable length pointer for the duration of this callback.
        unsafe { write_utf16_output(text, length, count, &value) };
        Ok(())
    });
}

unsafe extern "C" fn get_right_text(
    _proxy: *mut InputMethod_TextEditorProxy,
    number: i32,
    text: *mut u16,
    length: *mut usize,
) {
    guarded("get_right_text", (), || {
        let count = nonnegative_usize(number);
        let cursor = ime_selection().map_or(0, |selection| {
            if selection.reversed {
                selection.range.start
            } else {
                selection.range.end
            }
        });
        let end = cursor
            .saturating_add(count)
            .min(ime_text_length().unwrap_or(cursor));
        let value = ime_text_for_range(cursor..end).unwrap_or_default();
        // SAFETY: Same output-buffer contract as `get_left_text`.
        unsafe { write_utf16_output(text, length, count, &value) };
        Ok(())
    });
}

unsafe extern "C" fn get_text_index(_proxy: *mut InputMethod_TextEditorProxy) -> i32 {
    guarded("get_text_index", 0, || {
        let cursor = ime_selection().map_or(0, |selection| {
            if selection.reversed {
                selection.range.start
            } else {
                selection.range.end
            }
        });
        Ok(to_i32(cursor))
    })
}

unsafe extern "C" fn receive_private_command(
    _proxy: *mut InputMethod_TextEditorProxy,
    _commands: *mut *mut ohos_sys::inputmethod::private_command::InputMethod_PrivateCommand,
    _size: usize,
) -> i32 {
    // No vendor-private commands are defined for the Zed editor. Returning
    // zero reports that the request was safely ignored.
    0
}

unsafe extern "C" fn set_preview_text(
    _proxy: *mut InputMethod_TextEditorProxy,
    text: *const u16,
    length: usize,
    start: i32,
    end: i32,
) -> i32 {
    guarded("set_preview_text", -1, || {
        let text = unsafe { decode_utf16(text, length) }?;
        let replacement = if start >= 0 && end >= 0 {
            Some(nonnegative_usize(start)..nonnegative_usize(end))
        } else {
            None
        };
        ime_set_composition(replacement, &text);
        Ok(0)
    })
}

unsafe extern "C" fn finish_text_preview(_proxy: *mut InputMethod_TextEditorProxy) {
    guarded("finish_text_preview", (), || {
        ime_finish_composition();
        Ok(())
    });
}

unsafe fn decode_utf16(text: *const u16, length: usize) -> Result<String> {
    if length == 0 {
        return Ok(String::new());
    }
    let text = NonNull::new(text.cast_mut()).ok_or_else(|| anyhow!("null UTF-16 text"))?;
    // SAFETY: The IME callback guarantees `length` readable UTF-16 units for
    // the callback's duration, and the pointer was checked above.
    let units = unsafe { slice::from_raw_parts(text.as_ptr(), length) };
    String::from_utf16(units).map_err(|error| anyhow!("invalid UTF-16 from input method: {error}"))
}

unsafe fn write_utf16_output(
    output: *mut u16,
    output_length: *mut usize,
    capacity: usize,
    value: &str,
) {
    if output_length.is_null() {
        return;
    }
    // SAFETY: The IME contract guarantees a writable length pointer.
    unsafe { *output_length = 0 };
    if capacity == 0 || output.is_null() {
        return;
    }
    let encoded = value.encode_utf16().collect::<Vec<_>>();
    let copied = cmp::min(capacity, encoded.len());
    // SAFETY: HarmonyOS allocates `capacity` output units and `copied` is
    // bounded by that capacity. The source vector is live for the copy.
    unsafe { std::ptr::copy_nonoverlapping(encoded.as_ptr(), output, copied) };
    // SAFETY: Same valid length pointer as above.
    unsafe { *output_length = copied };
}

fn to_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn nonnegative_usize(value: i32) -> usize {
    usize::try_from(value.max(0)).unwrap_or_default()
}
