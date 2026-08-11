#![allow(clippy::missing_const_for_thread_local)]

use std::{
    any::Any,
    cell::RefCell,
    collections::HashMap,
    ffi::c_void,
    mem::MaybeUninit,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    sync::{Mutex, OnceLock},
};

use anyhow::{Context as _, Result, anyhow, bail};
use ohos_sys::arkui::ui_input_event::{
    ArkUI_UIInputEvent, ArkUI_UIInputEvent_Type, OH_ArkUI_AxisEvent_GetAxisAction,
    OH_ArkUI_AxisEvent_GetHorizontalAxisValue, OH_ArkUI_AxisEvent_GetVerticalAxisValue,
    OH_ArkUI_PointerEvent_GetX, OH_ArkUI_PointerEvent_GetY, OH_ArkUI_UIInputEvent_GetEventTime,
    OH_ArkUI_UIInputEvent_GetToolType,
};
use ohos_sys::xcomponent::{
    OH_NativeXComponent, OH_NativeXComponent_Callback, OH_NativeXComponent_EventSourceType,
    OH_NativeXComponent_GetMouseEvent, OH_NativeXComponent_GetTouchEvent,
    OH_NativeXComponent_GetTouchEventSourceType, OH_NativeXComponent_GetXComponentSize,
    OH_NativeXComponent_MouseEvent, OH_NativeXComponent_MouseEvent_Callback,
    OH_NativeXComponent_RegisterBlurEventCallback, OH_NativeXComponent_RegisterCallback,
    OH_NativeXComponent_RegisterFocusEventCallback, OH_NativeXComponent_RegisterMouseEventCallback,
    OH_NativeXComponent_TouchEvent,
};

use crate::{LogLevel, NativeDrawingSurface, log_message};

const SUCCESS: i32 = 0;

unsafe extern "C" {
    fn OH_NativeXComponent_GetXComponentId(
        component: *mut OH_NativeXComponent,
        id: *mut std::ffi::c_char,
        size: *mut u64,
    ) -> i32;

    fn OH_NativeXComponent_RegisterUIInputEventCallback(
        component: *mut OH_NativeXComponent,
        callback: Option<
            unsafe extern "C" fn(
                component: *mut OH_NativeXComponent,
                event: *mut ArkUI_UIInputEvent,
                type_: ArkUI_UIInputEvent_Type,
            ),
        >,
        type_: ArkUI_UIInputEvent_Type,
    ) -> i32;
}

#[derive(Clone, Copy, Debug)]
pub struct NativeTouchEvent {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub screen_x: f32,
    pub screen_y: f32,
    pub kind: u32,
    pub source: u32,
    pub pressure: f32,
    pub timestamp: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeMouseEvent {
    pub x: f32,
    pub y: f32,
    pub screen_x: f32,
    pub screen_y: f32,
    pub action: u32,
    pub button: u32,
    pub timestamp: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeScrollEvent {
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub action: i32,
    pub precise: bool,
    pub timestamp: i64,
}

#[derive(Clone, Copy, Debug)]
pub enum NativeSurfaceEvent {
    Created { width: u32, height: u32 },
    ResizePending,
    Resized { width: u32, height: u32 },
    Destroyed,
}

#[derive(Clone, Copy, Debug)]
pub enum NativeEvent {
    Surface(NativeSurfaceEvent),
    Touch(NativeTouchEvent),
    Mouse(NativeMouseEvent),
    Scroll(NativeScrollEvent),
    Hover(bool),
    Focused(bool),
}

struct RegisteredCallbacks {
    lifecycle: Box<OH_NativeXComponent_Callback>,
    mouse: Box<OH_NativeXComponent_MouseEvent_Callback>,
}

struct ComponentState {
    surface: NativeDrawingSurface,
    pending_resize: Option<(u32, u32)>,
    event_handler: Option<Box<dyn FnMut(NativeEvent)>>,
}

static REGISTERED_COMPONENTS: OnceLock<Mutex<HashMap<usize, RegisteredCallbacks>>> =
    OnceLock::new();

thread_local! {
    static COMPONENT_STATES: RefCell<HashMap<usize, ComponentState>> = RefCell::new(HashMap::new());
    static PENDING_EVENT_HANDLERS: RefCell<HashMap<usize, Box<dyn FnMut(NativeEvent)>>> = RefCell::new(HashMap::new());
    static COMPONENTS_BY_ID: RefCell<HashMap<String, XComponentHandle>> = RefCell::new(HashMap::new());
    static CURRENT_COMPONENT: RefCell<Option<XComponentHandle>> = const { RefCell::new(None) };
}

/// A main-thread handle to the ArkUI XComponent that hosts GPUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XComponentHandle(NonNull<OH_NativeXComponent>);

impl XComponentHandle {
    pub(crate) fn native(self) -> NonNull<OH_NativeXComponent> {
        self.0
    }
}

/// Registers the complete native surface and input callback set for an ArkUI
/// XComponent.
///
/// # Safety
///
/// `component` must be the `OH_NativeXComponent` pointer obtained by unwrapping
/// the `OH_NATIVE_XCOMPONENT_OBJ` N-API export. ArkUI must keep it alive for the
/// lifetime of the native module.
pub unsafe fn register_xcomponent(component: *mut c_void) -> Result<()> {
    let component = NonNull::new(component.cast::<OH_NativeXComponent>())
        .ok_or_else(|| anyhow!("ArkUI supplied a null OH_NativeXComponent"))?;
    let component_id = xcomponent_id(component)?;
    let component_handle = XComponentHandle(component);
    COMPONENTS_BY_ID.with_borrow_mut(|components| {
        components.insert(component_id.clone(), component_handle);
    });
    CURRENT_COMPONENT.with_borrow_mut(|current| {
        if component_id == "zed-surface" || current.is_none() {
            *current = Some(component_handle);
        }
    });
    let key = component.as_ptr() as usize;
    let components = REGISTERED_COMPONENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut components = components
        .lock()
        .map_err(|_| anyhow!("XComponent callback registry lock was poisoned"))?;
    if components.contains_key(&key) {
        return Ok(());
    }

    let mut callbacks = RegisteredCallbacks {
        lifecycle: Box::new(OH_NativeXComponent_Callback {
            OnSurfaceCreated: Some(on_surface_created),
            OnSurfaceChanged: Some(on_surface_changed),
            OnSurfaceDestroyed: Some(on_surface_destroyed),
            DispatchTouchEvent: Some(on_touch_event),
        }),
        mouse: Box::new(OH_NativeXComponent_MouseEvent_Callback {
            DispatchMouseEvent: Some(on_mouse_event),
            DispatchHoverEvent: Some(on_hover_event),
        }),
    };

    // SAFETY: The callback boxes are inserted into the process-lifetime
    // registry below, so the pointers passed to ArkUI remain stable.
    ensure_success("OH_NativeXComponent_RegisterCallback", unsafe {
        OH_NativeXComponent_RegisterCallback(component.as_ptr(), callbacks.lifecycle.as_mut())
    })?;
    // SAFETY: Same lifetime argument as above.
    ensure_success("OH_NativeXComponent_RegisterMouseEventCallback", unsafe {
        OH_NativeXComponent_RegisterMouseEventCallback(component.as_ptr(), callbacks.mouse.as_mut())
    })?;
    // SAFETY: `on_axis_event` has static lifetime and ArkUI only invokes it
    // with a borrowed event for the requested axis-event type.
    ensure_success("OH_NativeXComponent_RegisterUIInputEventCallback", unsafe {
        OH_NativeXComponent_RegisterUIInputEventCallback(
            component.as_ptr(),
            Some(on_axis_event),
            ArkUI_UIInputEvent_Type::ARKUI_UIINPUTEVENT_TYPE_AXIS,
        )
    })?;
    // SAFETY: The function pointers have static lifetime.
    ensure_success("OH_NativeXComponent_RegisterFocusEventCallback", unsafe {
        OH_NativeXComponent_RegisterFocusEventCallback(component.as_ptr(), Some(on_focus_event))
    })?;
    // SAFETY: The function pointers have static lifetime.
    ensure_success("OH_NativeXComponent_RegisterBlurEventCallback", unsafe {
        OH_NativeXComponent_RegisterBlurEventCallback(component.as_ptr(), Some(on_blur_event))
    })?;

    components.insert(key, callbacks);
    log_message(
        LogLevel::Info,
        format!("registered ArkUI XComponent callbacks for {component_id}"),
    );
    Ok(())
}

pub(crate) fn xcomponent_id(component: NonNull<OH_NativeXComponent>) -> Result<String> {
    const MAXIMUM_ID_LENGTH: usize = 128;
    let mut id = [0_u8; MAXIMUM_ID_LENGTH + 1];
    let mut length = u64::try_from(id.len()).context("XComponent ID buffer length exceeds u64")?;
    // SAFETY: `component` is live and the fixed buffer and length output remain
    // valid for the duration of the native call.
    ensure_success("OH_NativeXComponent_GetXComponentId", unsafe {
        OH_NativeXComponent_GetXComponentId(component.as_ptr(), id.as_mut_ptr().cast(), &mut length)
    })?;
    let length = usize::try_from(length).context("XComponent ID length exceeds usize")?;
    if length > id.len() {
        bail!(
            "ArkUI XComponent ID requires {length} bytes, but the buffer holds {}",
            id.len()
        );
    }
    let bytes = id
        .get(..length)
        .context("XComponent ID length is outside its buffer")?;
    let bytes = bytes
        .split(|byte| *byte == 0)
        .next()
        .context("XComponent ID buffer is empty")?;
    let id = std::str::from_utf8(bytes).context("XComponent ID is not UTF-8")?;
    if id.is_empty() {
        bail!("ArkUI returned an empty XComponent ID");
    }
    Ok(id.to_owned())
}

/// Returns the XComponent registered by the current ArkUI page.
pub fn current_xcomponent() -> Result<XComponentHandle> {
    CURRENT_COMPONENT.with_borrow(|component| {
        component.ok_or_else(|| anyhow!("ArkUI has not registered its XComponent yet"))
    })
}

pub(crate) fn xcomponent_by_id(id: &str) -> Result<XComponentHandle> {
    COMPONENTS_BY_ID.with_borrow(|components| {
        components
            .get(id)
            .copied()
            .with_context(|| format!("ArkUI XComponent {id:?} is not registered"))
    })
}

pub(crate) fn set_event_handler(
    component: XComponentHandle,
    handler: Box<dyn FnMut(NativeEvent)>,
) -> Result<()> {
    let key = component.native().as_ptr() as usize;
    let mut handler = Some(handler);
    let ready_size = COMPONENT_STATES.with_borrow_mut(|states| {
        if let Some(state) = states.get_mut(&key) {
            state.event_handler = handler.take();
            Some(state.surface.size())
        } else {
            None
        }
    });
    if let Some(handler) = handler {
        PENDING_EVENT_HANDLERS.with_borrow_mut(|handlers| {
            handlers.insert(key, handler);
        });
    }
    if let Some((width, height)) = ready_size {
        emit(
            component.native(),
            NativeEvent::Surface(NativeSurfaceEvent::Created { width, height }),
        )?;
    }
    Ok(())
}

pub(crate) fn clear_event_handler(component: XComponentHandle) {
    let key = component.native().as_ptr() as usize;
    COMPONENT_STATES.with_borrow_mut(|states| {
        if let Some(state) = states.get_mut(&key) {
            state.event_handler = None;
        }
    });
    PENDING_EVENT_HANDLERS.with_borrow_mut(|handlers| {
        handlers.remove(&key);
    });
}

pub(crate) fn with_surface<R>(
    component: XComponentHandle,
    callback: impl FnOnce(&mut NativeDrawingSurface) -> Result<R>,
) -> Result<R> {
    COMPONENT_STATES.with_borrow_mut(|states| {
        let state = states
            .get_mut(&(component.native().as_ptr() as usize))
            .ok_or_else(|| anyhow!("XComponent surface is not ready"))?;
        callback(&mut state.surface)
    })
}

pub(crate) fn apply_pending_surface_resize(
    component: XComponentHandle,
) -> Result<Option<(u32, u32)>> {
    COMPONENT_STATES.with_borrow_mut(|states| {
        let Some(state) = states.get_mut(&(component.native().as_ptr() as usize)) else {
            return Ok(None);
        };
        let Some((width, height)) = state.pending_resize.take() else {
            return Ok(None);
        };
        if state.surface.size() == (width, height) {
            return Ok(None);
        }
        state.surface.resize(width, height)?;
        Ok(Some((width, height)))
    })
}

unsafe extern "C" fn on_surface_created(component: *mut OH_NativeXComponent, window: *mut c_void) {
    guard_native_callback("surface creation", || {
        if let Err(error) = create_surface(component, window, false) {
            log_message(
                LogLevel::Error,
                format!("surface creation failed: {error:#}"),
            );
        }
    });
}

unsafe extern "C" fn on_surface_changed(component: *mut OH_NativeXComponent, window: *mut c_void) {
    guard_native_callback("surface resize", || {
        if let Err(error) = create_surface(component, window, true) {
            log_message(LogLevel::Error, format!("surface resize failed: {error:#}"));
        }
    });
}

unsafe extern "C" fn on_surface_destroyed(
    component: *mut OH_NativeXComponent,
    _window: *mut c_void,
) {
    guard_native_callback("surface destruction", || {
        let Some(component) = NonNull::new(component) else {
            log_message(
                LogLevel::Error,
                "surface destruction supplied a null component",
            );
            return;
        };
        COMPONENT_STATES.with_borrow_mut(|states| {
            if let Some(mut state) = states.remove(&(component.as_ptr() as usize)) {
                if let Some(handler) = state.event_handler.take() {
                    PENDING_EVENT_HANDLERS.with_borrow_mut(|handlers| {
                        handlers.insert(component.as_ptr() as usize, handler);
                    });
                }
            }
        });
        if let Err(error) = emit_pending(
            component.as_ptr() as usize,
            NativeEvent::Surface(NativeSurfaceEvent::Destroyed),
        ) {
            log_message(
                LogLevel::Error,
                format!("surface destruction dispatch failed: {error:#}"),
            );
        }
        log_message(LogLevel::Info, "destroyed Native Drawing surface");
    });
}

unsafe extern "C" fn on_touch_event(component: *mut OH_NativeXComponent, window: *mut c_void) {
    guard_native_callback("touch dispatch", || {
        let result = (|| -> Result<()> {
            let component = NonNull::new(component).ok_or_else(|| anyhow!("null component"))?;
            let window = NonNull::new(window).ok_or_else(|| anyhow!("null window"))?;
            let mut native_event = MaybeUninit::<OH_NativeXComponent_TouchEvent>::uninit();
            // SAFETY: ArkUI initializes `native_event` on success; both handles
            // originate from the active XComponent callback.
            ensure_success("OH_NativeXComponent_GetTouchEvent", unsafe {
                OH_NativeXComponent_GetTouchEvent(
                    component.as_ptr(),
                    window.as_ptr(),
                    native_event.as_mut_ptr(),
                )
            })?;
            // SAFETY: The successful native call initialized the complete C struct.
            let native_event = unsafe { native_event.assume_init() };
            let mut source = MaybeUninit::<OH_NativeXComponent_EventSourceType>::uninit();
            // SAFETY: The component is active and `native_event.id` identifies the
            // touch point delivered by the callback. The output is initialized on success.
            ensure_success("OH_NativeXComponent_GetTouchEventSourceType", unsafe {
                OH_NativeXComponent_GetTouchEventSourceType(
                    component.as_ptr(),
                    native_event.id,
                    source.as_mut_ptr(),
                )
            })?;
            // SAFETY: The successful native call initialized the source type.
            let source = unsafe { source.assume_init() }.0;
            #[cfg(debug_assertions)]
            log_message(
                LogLevel::Debug,
                format!(
                    "touch event: local=({:.1},{:.1}) screen=({:.1},{:.1}) kind={} source={} id={}",
                    native_event.x,
                    native_event.y,
                    native_event.screenX,
                    native_event.screenY,
                    native_event.type_.0,
                    source,
                    native_event.id,
                ),
            );
            emit(
                component,
                NativeEvent::Touch(NativeTouchEvent {
                    id: native_event.id,
                    x: native_event.x,
                    y: native_event.y,
                    screen_x: native_event.screenX,
                    screen_y: native_event.screenY,
                    kind: native_event.type_.0,
                    source,
                    pressure: native_event.force,
                    timestamp: native_event.timeStamp,
                }),
            )
        })();
        if let Err(error) = result {
            log_message(LogLevel::Error, format!("touch dispatch failed: {error:#}"));
        }
    });
}

unsafe extern "C" fn on_mouse_event(component: *mut OH_NativeXComponent, window: *mut c_void) {
    guard_native_callback("mouse dispatch", || {
        let result = (|| -> Result<()> {
            let component = NonNull::new(component).ok_or_else(|| anyhow!("null component"))?;
            let window = NonNull::new(window).ok_or_else(|| anyhow!("null window"))?;
            let mut native_event = MaybeUninit::<OH_NativeXComponent_MouseEvent>::uninit();
            // SAFETY: ArkUI initializes `native_event` on success.
            ensure_success("OH_NativeXComponent_GetMouseEvent", unsafe {
                OH_NativeXComponent_GetMouseEvent(
                    component.as_ptr(),
                    window.as_ptr(),
                    native_event.as_mut_ptr(),
                )
            })?;
            // SAFETY: The successful native call initialized the complete C struct.
            let native_event = unsafe { native_event.assume_init() };
            #[cfg(debug_assertions)]
            log_message(
                LogLevel::Debug,
                format!(
                    "mouse event: local=({:.1},{:.1}) screen=({:.1},{:.1}) action={} button={}",
                    native_event.x,
                    native_event.y,
                    native_event.screenX,
                    native_event.screenY,
                    native_event.action.0,
                    native_event.button.0,
                ),
            );
            emit(
                component,
                NativeEvent::Mouse(NativeMouseEvent {
                    x: native_event.x,
                    y: native_event.y,
                    screen_x: native_event.screenX,
                    screen_y: native_event.screenY,
                    action: native_event.action.0,
                    button: native_event.button.0,
                    timestamp: native_event.timestamp,
                }),
            )
        })();
        if let Err(error) = result {
            log_message(LogLevel::Error, format!("mouse dispatch failed: {error:#}"));
        }
    });
}

unsafe extern "C" fn on_hover_event(component: *mut OH_NativeXComponent, is_hovering: bool) {
    guard_native_callback("hover dispatch", || {
        if let Some(component) = NonNull::new(component)
            && let Err(error) = emit(component, NativeEvent::Hover(is_hovering))
        {
            log_message(LogLevel::Error, format!("hover dispatch failed: {error:#}"));
        }
    });
}

unsafe extern "C" fn on_axis_event(
    component: *mut OH_NativeXComponent,
    event: *mut ArkUI_UIInputEvent,
    type_: ArkUI_UIInputEvent_Type,
) {
    guard_native_callback("axis dispatch", || {
        let result = (|| -> Result<()> {
            let component = NonNull::new(component).ok_or_else(|| anyhow!("null component"))?;
            let event = NonNull::new(event).ok_or_else(|| anyhow!("null axis event"))?;
            if type_ != ArkUI_UIInputEvent_Type::ARKUI_UIINPUTEVENT_TYPE_AXIS {
                bail!("unexpected UI input event type {}", type_.0);
            }

            // SAFETY: ArkUI owns `event` and guarantees that it remains valid
            // for the duration of this callback. The registered type is AXIS.
            let native_event = unsafe {
                NativeScrollEvent {
                    x: OH_ArkUI_PointerEvent_GetX(event.as_ptr()),
                    y: OH_ArkUI_PointerEvent_GetY(event.as_ptr()),
                    delta_x: OH_ArkUI_AxisEvent_GetHorizontalAxisValue(event.as_ptr()) as f32,
                    delta_y: OH_ArkUI_AxisEvent_GetVerticalAxisValue(event.as_ptr()) as f32,
                    action: OH_ArkUI_AxisEvent_GetAxisAction(event.as_ptr()),
                    precise: OH_ArkUI_UIInputEvent_GetToolType(event.as_ptr()) == 4,
                    timestamp: OH_ArkUI_UIInputEvent_GetEventTime(event.as_ptr()),
                }
            };
            #[cfg(debug_assertions)]
            log_message(
                LogLevel::Debug,
                format!(
                    "axis event: position=({:.1},{:.1}) delta=({:.2},{:.2}) action={} precise={}",
                    native_event.x,
                    native_event.y,
                    native_event.delta_x,
                    native_event.delta_y,
                    native_event.action,
                    native_event.precise,
                ),
            );
            emit(component, NativeEvent::Scroll(native_event))
        })();
        if let Err(error) = result {
            log_message(LogLevel::Error, format!("axis dispatch failed: {error:#}"));
        }
    });
}

unsafe extern "C" fn on_focus_event(component: *mut OH_NativeXComponent, _window: *mut c_void) {
    guard_native_callback("focus dispatch", || emit_focus(component, true));
}

unsafe extern "C" fn on_blur_event(component: *mut OH_NativeXComponent, _window: *mut c_void) {
    guard_native_callback("blur dispatch", || emit_focus(component, false));
}

fn create_surface(
    component: *mut OH_NativeXComponent,
    window: *mut c_void,
    resized: bool,
) -> Result<()> {
    let component = NonNull::new(component).ok_or_else(|| anyhow!("null component"))?;
    let window = NonNull::new(window).ok_or_else(|| anyhow!("null native window"))?;
    let mut width = 0;
    let mut height = 0;
    // SAFETY: The handles originate from the active XComponent surface
    // callback and the output pointers are valid.
    ensure_success("OH_NativeXComponent_GetXComponentSize", unsafe {
        OH_NativeXComponent_GetXComponentSize(
            component.as_ptr(),
            window.as_ptr(),
            &mut width,
            &mut height,
        )
    })?;
    let width = u32::try_from(width).context("XComponent width exceeds u32")?;
    let height = u32::try_from(height).context("XComponent height exceeds u32")?;
    let key = component.as_ptr() as usize;

    if resized {
        let queued = COMPONENT_STATES.with_borrow_mut(|states| -> Result<bool> {
            let Some(state) = states.get_mut(&key) else {
                return Ok(false);
            };
            if state.surface.window() != window {
                return Ok(false);
            }
            state.pending_resize = Some((width, height));
            Ok(true)
        })?;
        if queued {
            emit(
                component,
                NativeEvent::Surface(NativeSurfaceEvent::ResizePending),
            )?;
            return Ok(());
        }
    }

    // SAFETY: The native window is valid until the matching destruction
    // callback, where the state is removed and dropped.
    let mut surface = unsafe { NativeDrawingSurface::new(window, width, height) }?;
    surface.clear(0xff18181b)?;

    COMPONENT_STATES.with_borrow_mut(|states| {
        let previous_handler = states.remove(&key).and_then(|state| state.event_handler);
        let event_handler = previous_handler
            .or_else(|| PENDING_EVENT_HANDLERS.with_borrow_mut(|handlers| handlers.remove(&key)));
        let state = ComponentState {
            surface,
            pending_resize: None,
            event_handler,
        };
        states.insert(key, state);
    });
    let event = if resized {
        NativeSurfaceEvent::Resized { width, height }
    } else {
        NativeSurfaceEvent::Created { width, height }
    };
    emit(component, NativeEvent::Surface(event))?;
    log_message(
        LogLevel::Info,
        format!("Native Drawing surface ready: {width}x{height}"),
    );
    Ok(())
}

fn emit_focus(component: *mut OH_NativeXComponent, focused: bool) {
    let Some(component) = NonNull::new(component) else {
        log_message(LogLevel::Error, "focus event supplied a null component");
        return;
    };
    if let Err(error) = emit(component, NativeEvent::Focused(focused)) {
        log_message(LogLevel::Error, format!("focus dispatch failed: {error:#}"));
    }
}

fn emit(component: NonNull<OH_NativeXComponent>, event: NativeEvent) -> Result<()> {
    let key = component.as_ptr() as usize;
    let handler = COMPONENT_STATES.with_borrow_mut(|states| {
        states
            .get_mut(&key)
            .ok_or_else(|| anyhow!("event arrived before the XComponent surface was ready"))
            .map(|state| state.event_handler.take())
    })?;
    if let Some(handler) = handler {
        dispatch_outside_surface_borrow(key, handler, event);
    }
    Ok(())
}

fn emit_pending(key: usize, event: NativeEvent) -> Result<()> {
    let handler = PENDING_EVENT_HANDLERS
        .with_borrow_mut(|handlers| handlers.remove(&key))
        .ok_or_else(|| anyhow!("XComponent event handler is not installed"))?;
    dispatch_outside_surface_borrow(key, handler, event);
    Ok(())
}

fn dispatch_outside_surface_borrow(
    key: usize,
    mut handler: Box<dyn FnMut(NativeEvent)>,
    event: NativeEvent,
) {
    let panic = catch_unwind(AssertUnwindSafe(|| handler(event))).err();
    let mut handler = Some(handler);
    COMPONENT_STATES.with_borrow_mut(|states| {
        if let Some(state) = states.get_mut(&key)
            && state.event_handler.is_none()
        {
            state.event_handler = handler.take();
        }
    });
    if let Some(handler) = handler {
        PENDING_EVENT_HANDLERS.with_borrow_mut(|handlers| {
            handlers.insert(key, handler);
        });
    }
    if let Some(panic) = panic {
        log_callback_panic("native event handler", panic.as_ref());
    }
}

fn guard_native_callback(name: &'static str, callback: impl FnOnce()) {
    if let Err(panic) = catch_unwind(AssertUnwindSafe(callback)) {
        log_callback_panic(name, panic.as_ref());
    }
}

fn log_callback_panic(name: &str, payload: &(dyn Any + Send)) {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload");
    log_message(
        LogLevel::Error,
        format!("panic contained at the XComponent {name} boundary: {message}"),
    );
}

fn ensure_success(operation: &str, status: i32) -> Result<()> {
    if status == SUCCESS {
        Ok(())
    } else {
        bail!("{operation} failed with status {status}")
    }
}
