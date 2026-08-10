use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ffi::c_void,
    ops::Range,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    path::{Path, PathBuf},
    ptr::NonNull,
    rc::{Rc, Weak},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use futures::channel::oneshot;
use gpui::{
    Action, AnyWindowHandle, AppLifecyclePhase, BackgroundExecutor, Bounds, Capslock,
    ClipboardItem, CursorStyle, DispatchEventResult, DisplayId, DummyKeyboardMapper,
    ForegroundExecutor, GpuSpecs, KeyDownEvent, KeyUpEvent, Keymap, Keystroke, Menu, MenuItem,
    Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    NavigationDirection, OwnedMenu, PathPromptOptions, Pixels, Platform, PlatformAtlas,
    PlatformDisplay, PlatformInput, PlatformInputHandler, PlatformKeyboardLayout,
    PlatformKeyboardMapper, PlatformTextSystem, PlatformWindow, Point, PromptButton, PromptLevel,
    RequestFrameOptions, Scene, ScrollDelta, ScrollWheelEvent, Size, Task, ThermalState,
    TouchEvent, TouchId, TouchPhase, UTF16Selection, WindowAppearance, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowParams, point, px,
};
use gpui_text::CosmicTextSystem;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use uuid::Uuid;

use crate::{
    LogLevel, NativeEvent, NativeMouseEvent, NativeScrollEvent, NativeSurfaceEvent,
    NativeTouchEvent, XComponentHandle,
    atlas::OhosAtlas,
    current_xcomponent,
    dispatcher::OhosDispatcher,
    ime::NativeIme,
    log_message,
    xcomponent::{
        apply_pending_surface_resize, clear_event_handler, set_event_handler, with_surface,
        xcomponent_by_id,
    },
};

thread_local! {
    static CURRENT_PLATFORM: RefCell<Option<Rc<OhosPlatform>>> = const { RefCell::new(None) };
}

pub fn current_platform(_headless: bool) -> Rc<dyn Platform> {
    CURRENT_PLATFORM.with_borrow_mut(|current| {
        if let Some(platform) = current.as_ref() {
            return platform.clone();
        }
        let platform = match OhosPlatform::new() {
            Ok(platform) => platform,
            Err(error) => {
                log_message(
                    LogLevel::Error,
                    format!("failed to initialize GPUI: {error:#}"),
                );
                // The Platform factory cannot return an error and GPUI cannot
                // run without its dispatcher, VSync connection, and surface.
                std::process::abort();
            }
        };
        *current = Some(platform.clone());
        platform
    })
}

pub fn on_arkui_frame() {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            log_message(
                LogLevel::Error,
                "ArkUI frame arrived without a GPUI platform",
            );
            return;
        };
        platform.handle_frame();
    });
}

/// Installs the thread-safe ArkTS wake callback and configures the
/// physical-pixel to logical-pixel scale.
pub fn configure_frame_scheduler(
    callback: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    open_external_url: Arc<dyn Fn(String) -> Result<()> + Send + Sync>,
    set_cursor: Arc<dyn Fn(u32, bool) -> Result<()> + Send + Sync>,
    set_auxiliary_window: Arc<dyn Fn(bool, String, u32, u32) -> Result<()> + Send + Sync>,
    request_path_prompt: Arc<
        dyn Fn(u32, bool, bool, bool, bool, Option<String>, Option<String>) -> Result<()>
            + Send
            + Sync,
    >,
    request_prompt: Arc<
        dyn Fn(u32, u32, String, Option<String>, Vec<String>) -> Result<()> + Send + Sync,
    >,
    scale_factor: f32,
    ui_context: NonNull<c_void>,
) -> Result<()> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        bail!("ArkUI supplied an invalid scale factor {scale_factor}");
    }

    CURRENT_PLATFORM.with_borrow(|current| {
        let platform = current
            .as_ref()
            .context("GPUI has not initialized before ArkUI configuration")?;
        platform.configure_frame_scheduler(
            callback,
            open_external_url,
            set_cursor,
            set_auxiliary_window,
            request_path_prompt,
            request_prompt,
            scale_factor,
            ui_context,
        )
    })
}

/// Updates the physical-pixel to logical-pixel scale after an ArkUI area or
/// density change.
pub fn set_scale_factor(scale_factor: f32) -> Result<()> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        bail!("ArkUI supplied an invalid scale factor {scale_factor}");
    }

    CURRENT_PLATFORM.with_borrow(|current| {
        let platform = current
            .as_ref()
            .context("GPUI has not initialized before the scale-factor update")?;
        platform.set_scale_factor(scale_factor)
    })
}

pub fn complete_path_prompt(request_id: u32, uris: Vec<String>, error: Option<String>) {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            log_message(
                LogLevel::Error,
                format!("path prompt {request_id} completed before GPUI initialized"),
            );
            return;
        };
        platform.complete_path_prompt(request_id, uris, error);
    });
}

pub fn complete_prompt(request_id: u32, answer: Option<u32>, error: Option<String>) {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            log_message(
                LogLevel::Error,
                format!("prompt {request_id} completed before GPUI initialized"),
            );
            return;
        };
        platform.complete_prompt(request_id, answer, error);
    });
}

pub fn set_lifecycle_phase(phase: u32) -> Result<()> {
    let phase = match phase {
        0 => AppLifecyclePhase::Active,
        1 => AppLifecyclePhase::Inactive,
        2 => AppLifecyclePhase::Background,
        3 => AppLifecyclePhase::Foreground,
        _ => bail!("ArkUI supplied an invalid lifecycle phase {phase}"),
    };
    CURRENT_PLATFORM.with_borrow(|current| {
        let platform = current
            .as_ref()
            .context("GPUI has not initialized before the lifecycle update")?;
        platform.handle_lifecycle_phase(phase);
        Ok(())
    })
}

pub fn handle_memory_warning() {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            log_message(
                LogLevel::Error,
                "memory warning arrived before GPUI initialized",
            );
            return;
        };
        platform.handle_memory_warning();
    });
}

pub fn close_auxiliary_window() {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            log_message(
                LogLevel::Error,
                "auxiliary window closed before GPUI initialized",
            );
            return;
        };
        platform.close_auxiliary_window();
    });
}

pub fn dispatch_arkui_key_event(
    action: u32,
    code: i32,
    key_text: String,
    unicode: Option<u32>,
    modifiers: u32,
) -> bool {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            log_message(
                LogLevel::Error,
                "ArkUI key event arrived before GPUI initialized",
            );
            return false;
        };
        platform.dispatch_arkui_key_event(ArkUiKeyEvent {
            action,
            code,
            key_text,
            unicode,
            modifiers,
        })
    })
}

pub fn persistent_uri_for_path(path: &Path) -> Option<String> {
    CURRENT_PLATFORM.with_borrow(|current| {
        current
            .as_ref()
            .and_then(|platform| platform.persistent_uris.borrow().get(path).cloned())
    })
}

#[derive(Default)]
struct PlatformCallbacks {
    quit: Option<Box<dyn FnMut()>>,
    reopen: Option<Box<dyn FnMut()>>,
    system_wake: Option<Box<dyn FnMut()>>,
    lifecycle: Option<Box<dyn FnMut(AppLifecyclePhase)>>,
    memory_warning: Option<Box<dyn FnMut()>>,
    app_menu_action: Option<Box<dyn FnMut(&dyn Action)>>,
    will_open_app_menu: Option<Box<dyn FnMut()>>,
    validate_app_menu_command: Option<Box<dyn FnMut(&dyn Action) -> bool>>,
}

struct PendingPrompt {
    sender: oneshot::Sender<usize>,
    answer_count: usize,
    fallback_answer: usize,
}

struct ArkUiKeyEvent {
    action: u32,
    code: i32,
    key_text: String,
    unicode: Option<u32>,
    modifiers: u32,
}

type AuxiliaryWindowCallback = Arc<dyn Fn(bool, String, u32, u32) -> Result<()> + Send + Sync>;

struct OhosPlatform {
    component: XComponentHandle,
    dispatcher: Arc<OhosDispatcher>,
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    text_system: Arc<dyn PlatformTextSystem>,
    display: Rc<OhosDisplay>,
    scale_factor: Cell<f32>,
    windows: RefCell<Vec<Rc<RefCell<WindowState>>>>,
    ime: RefCell<Option<NativeIme>>,
    menus: RefCell<Vec<OwnedMenu>>,
    open_external_url: RefCell<Option<Arc<dyn Fn(String) -> Result<()> + Send + Sync>>>,
    set_cursor: RefCell<Option<Arc<dyn Fn(u32, bool) -> Result<()> + Send + Sync>>>,
    set_auxiliary_window: RefCell<Option<AuxiliaryWindowCallback>>,
    request_path_prompt: RefCell<
        Option<
            Arc<
                dyn Fn(u32, bool, bool, bool, bool, Option<String>, Option<String>) -> Result<()>
                    + Send
                    + Sync,
            >,
        >,
    >,
    request_prompt: RefCell<
        Option<
            Arc<dyn Fn(u32, u32, String, Option<String>, Vec<String>) -> Result<()> + Send + Sync>,
        >,
    >,
    next_path_prompt_id: Cell<u32>,
    pending_path_prompts: RefCell<HashMap<u32, oneshot::Sender<Result<Option<Vec<PathBuf>>>>>>,
    pending_new_path_prompts: RefCell<HashMap<u32, oneshot::Sender<Result<Option<PathBuf>>>>>,
    next_prompt_id: Cell<u32>,
    pending_prompts: RefCell<HashMap<u32, PendingPrompt>>,
    persistent_uris: RefCell<HashMap<PathBuf, String>>,
    cursor_style: Cell<CursorStyle>,
    cursor_visible: Cell<bool>,
    callbacks: RefCell<PlatformCallbacks>,
}

impl OhosPlatform {
    fn new() -> Result<Rc<Self>> {
        let component = current_xcomponent()?;
        let dispatcher = OhosDispatcher::new()?;
        let background_executor = BackgroundExecutor::new(dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(dispatcher.clone());
        let system_font_dir = Path::new("/system/fonts");
        let text_system = if system_font_dir.is_dir() {
            CosmicTextSystem::new_with_font_dir("HarmonyOS Sans", system_font_dir)
        } else {
            log_message(
                LogLevel::Warning,
                "HarmonyOS system font directory is unavailable; using bundled fonts only",
            );
            CosmicTextSystem::new_without_system_fonts("Zed Sans")
        };
        Ok(Rc::new(Self {
            component,
            dispatcher,
            background_executor,
            foreground_executor,
            text_system: Arc::new(text_system),
            display: Rc::new(OhosDisplay::default()),
            scale_factor: Cell::new(1.0),
            windows: RefCell::new(Vec::new()),
            ime: RefCell::new(None),
            menus: RefCell::new(Vec::new()),
            open_external_url: RefCell::new(None),
            set_cursor: RefCell::new(None),
            set_auxiliary_window: RefCell::new(None),
            request_path_prompt: RefCell::new(None),
            request_prompt: RefCell::new(None),
            next_path_prompt_id: Cell::new(1),
            pending_path_prompts: RefCell::new(HashMap::new()),
            pending_new_path_prompts: RefCell::new(HashMap::new()),
            next_prompt_id: Cell::new(1),
            pending_prompts: RefCell::new(HashMap::new()),
            persistent_uris: RefCell::new(HashMap::new()),
            cursor_style: Cell::new(CursorStyle::Arrow),
            cursor_visible: Cell::new(true),
            callbacks: RefCell::new(PlatformCallbacks::default()),
        }))
    }

    fn request_frame(&self) {
        for window in self.windows.borrow().iter() {
            window.borrow_mut().frame_requested = true;
        }
        self.dispatcher.request_main_wake();
    }

    fn active_window_state(&self) -> Option<Rc<RefCell<WindowState>>> {
        self.windows
            .borrow()
            .iter()
            .rev()
            .find(|window| window.borrow().active)
            .cloned()
            .or_else(|| self.windows.borrow().first().cloned())
    }

    fn handle_lifecycle_phase(&self, phase: AppLifecyclePhase) {
        log_message(
            LogLevel::Info,
            format!("HarmonyOS application lifecycle changed to {phase:?}"),
        );
        let mut callback = self.callbacks.borrow_mut().lifecycle.take();
        if let Some(callback) = callback.as_mut() {
            callback(phase);
        }
        if let Some(callback) = callback {
            self.callbacks.borrow_mut().lifecycle = Some(callback);
        }
        if matches!(
            phase,
            AppLifecyclePhase::Active | AppLifecyclePhase::Foreground
        ) {
            self.request_frame();
        }
    }

    fn handle_memory_warning(&self) {
        log_message(LogLevel::Warning, "HarmonyOS reported memory pressure");
        let mut callback = self.callbacks.borrow_mut().memory_warning.take();
        if let Some(callback) = callback.as_mut() {
            callback();
        }
        if let Some(callback) = callback {
            self.callbacks.borrow_mut().memory_warning = Some(callback);
        }
    }

    fn handle_frame(&self) {
        self.dispatcher.begin_frame();
        self.dispatcher.drain_main_queue();

        let windows = self.windows.borrow().clone();
        for window in windows {
            let component = window.borrow().component;
            match apply_pending_surface_resize(component) {
                Ok(Some((width, height))) => {
                    self.handle_surface_event(
                        &window,
                        NativeSurfaceEvent::Resized { width, height },
                    );
                    #[cfg(debug_assertions)]
                    log_message(
                        LogLevel::Debug,
                        format!("Native Drawing surface resized: {width}x{height}"),
                    );
                }
                Ok(None) => {}
                Err(error) => {
                    log_message(
                        LogLevel::Error,
                        format!("failed to apply the pending surface resize: {error:#}"),
                    );
                }
            }
            let mut callback = {
                let mut state = window.borrow_mut();
                if !state.frame_requested || !state.surface_available {
                    continue;
                }
                state.frame_requested = false;
                state.callbacks.request_frame.take()
            };
            if let Some(callback) = callback.as_mut() {
                callback(RequestFrameOptions {
                    require_presentation: true,
                    force_render: false,
                });
            }
            if let Some(callback) = callback {
                window.borrow_mut().callbacks.request_frame = Some(callback);
            }
        }
    }

    fn configure_frame_scheduler(
        &self,
        callback: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        open_external_url: Arc<dyn Fn(String) -> Result<()> + Send + Sync>,
        set_cursor: Arc<dyn Fn(u32, bool) -> Result<()> + Send + Sync>,
        set_auxiliary_window: AuxiliaryWindowCallback,
        request_path_prompt: Arc<
            dyn Fn(u32, bool, bool, bool, bool, Option<String>, Option<String>) -> Result<()>
                + Send
                + Sync,
        >,
        request_prompt: Arc<
            dyn Fn(u32, u32, String, Option<String>, Vec<String>) -> Result<()> + Send + Sync,
        >,
        scale_factor: f32,
        ui_context: NonNull<c_void>,
    ) -> Result<()> {
        self.dispatcher.install_wake_callback(callback);
        self.open_external_url.replace(Some(open_external_url));
        self.set_cursor.replace(Some(set_cursor));
        self.set_auxiliary_window
            .replace(Some(set_auxiliary_window));
        self.notify_cursor();
        self.request_path_prompt.replace(Some(request_path_prompt));
        self.request_prompt.replace(Some(request_prompt));
        let ime = NativeIme::new(ui_context).context("attaching the HarmonyOS input method")?;
        self.ime.replace(Some(ime));
        self.set_scale_factor(scale_factor)
    }

    fn set_scale_factor(&self, scale_factor: f32) -> Result<()> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            bail!("ArkUI supplied an invalid scale factor {scale_factor}");
        }
        if self.scale_factor.get() == scale_factor {
            self.request_frame();
            return Ok(());
        }
        self.scale_factor.set(scale_factor);
        log_message(
            LogLevel::Info,
            format!("updated ArkUI display scale factor: {scale_factor:.3}"),
        );
        let windows = self.windows.borrow().clone();
        if windows.is_empty() {
            self.request_frame();
            return Ok(());
        }

        for window in windows {
            let resize = {
                let mut state = window.borrow_mut();
                state.scale_factor = scale_factor;
                state.surface_size.map(|(width, height)| {
                    let size = Size::new(
                        px(width as f32 / scale_factor),
                        px(height as f32 / scale_factor),
                    );
                    state.bounds = Bounds::new(Point::default(), size);
                    (size, state.primary, state.callbacks.resize.take())
                })
            };
            if let Some((size, primary, mut callback)) = resize {
                if primary {
                    self.display
                        .bounds
                        .replace(Bounds::new(Point::default(), size));
                }
                if let Some(callback) = callback.as_mut() {
                    callback(size, scale_factor);
                }
                if let Some(callback) = callback {
                    window.borrow_mut().callbacks.resize = Some(callback);
                }
            }
        }
        self.request_frame();
        Ok(())
    }

    fn complete_path_prompt(&self, request_id: u32, uris: Vec<String>, error: Option<String>) {
        log_message(
            LogLevel::Info,
            format!(
                "HarmonyOS path prompt {request_id} completed with {} selection(s){}",
                uris.len(),
                error
                    .as_deref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            ),
        );
        let path_sender = self.pending_path_prompts.borrow_mut().remove(&request_id);
        let new_path_sender = self
            .pending_new_path_prompts
            .borrow_mut()
            .remove(&request_id);
        if path_sender.is_none() && new_path_sender.is_none() {
            log_message(
                LogLevel::Warning,
                format!("HarmonyOS completed unknown path prompt {request_id}"),
            );
            return;
        }
        let result = if let Some(error) = error {
            Err(anyhow::anyhow!(error))
        } else if uris.is_empty() {
            Ok(None)
        } else {
            crate::path_prompt::persist_and_resolve_uris(&uris).map(|selections| {
                let mut persistent_uris = self.persistent_uris.borrow_mut();
                Some(
                    selections
                        .into_iter()
                        .map(|(uri, path)| {
                            persistent_uris.insert(path.clone(), uri);
                            path
                        })
                        .collect(),
                )
            })
        };
        let receiver_dropped = if let Some(sender) = path_sender {
            sender.send(result).is_err()
        } else if let Some(sender) = new_path_sender {
            sender
                .send(result.map(|paths| paths.and_then(|paths| paths.into_iter().next())))
                .is_err()
        } else {
            false
        };
        if receiver_dropped {
            log_message(
                LogLevel::Warning,
                format!("receiver for HarmonyOS path prompt {request_id} was dropped"),
            );
        }
    }

    fn complete_prompt(&self, request_id: u32, answer: Option<u32>, error: Option<String>) {
        let Some(pending) = self.pending_prompts.borrow_mut().remove(&request_id) else {
            log_message(
                LogLevel::Warning,
                format!("HarmonyOS completed unknown prompt {request_id}"),
            );
            return;
        };
        if let Some(error) = error {
            log_message(
                LogLevel::Error,
                format!("HarmonyOS prompt {request_id} failed: {error}"),
            );
        }
        let answer = answer
            .and_then(|answer| usize::try_from(answer).ok())
            .filter(|answer| *answer < pending.answer_count)
            .unwrap_or(pending.fallback_answer);
        if pending.sender.send(answer).is_err() {
            log_message(
                LogLevel::Warning,
                format!("receiver for HarmonyOS prompt {request_id} was dropped"),
            );
        }
    }

    fn request_prompt(
        &self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        let request_prompt = self.request_prompt.borrow().as_ref().cloned()?;
        if answers.is_empty() {
            log_message(
                LogLevel::Warning,
                "refusing to open a prompt without answers",
            );
            return None;
        }
        let answer_count = answers.len();
        let fallback_answer = answers
            .iter()
            .position(PromptButton::is_cancel)
            .unwrap_or_default();
        let labels = answers
            .iter()
            .map(|answer| answer.label().to_string())
            .collect::<Vec<_>>();
        let level = match level {
            PromptLevel::Info => 0,
            PromptLevel::Warning => 1,
            PromptLevel::Critical => 2,
        };
        let request_id = self.next_prompt_id.get();
        self.next_prompt_id.set(request_id.wrapping_add(1).max(1));
        let (sender, receiver) = oneshot::channel();
        self.pending_prompts.borrow_mut().insert(
            request_id,
            PendingPrompt {
                sender,
                answer_count,
                fallback_answer,
            },
        );
        if let Err(error) = request_prompt(
            request_id,
            level,
            message.to_owned(),
            detail.map(ToOwned::to_owned),
            labels,
        ) {
            self.pending_prompts.borrow_mut().remove(&request_id);
            log_message(
                LogLevel::Error,
                format!("failed to ask HarmonyOS to open prompt {request_id}: {error:#}"),
            );
            return None;
        }
        Some(receiver)
    }

    fn update_cursor(&self, style: CursorStyle, visible: bool) {
        if self.cursor_style.get() == style && self.cursor_visible.get() == visible {
            return;
        }
        self.cursor_style.set(style);
        self.cursor_visible.set(visible);
        self.notify_cursor();
    }

    fn notify_cursor(&self) {
        let Some(set_cursor) = self.set_cursor.borrow().as_ref().cloned() else {
            return;
        };
        if let Err(error) = set_cursor(
            cursor_style_code(self.cursor_style.get()),
            self.cursor_visible.get(),
        ) {
            log_message(
                LogLevel::Error,
                format!("failed to update the HarmonyOS cursor: {error:#}"),
            );
        }
    }

    fn handle_native_event(&self, window: &Rc<RefCell<WindowState>>, event: NativeEvent) {
        self.dispatcher.drain_main_queue();
        match event {
            NativeEvent::Surface(event) => self.handle_surface_event(window, event),
            NativeEvent::Touch(event) => dispatch_touch(window, event),
            NativeEvent::Mouse(event) => {
                if !self.cursor_visible.get() {
                    self.update_cursor(self.cursor_style.get(), true);
                }
                dispatch_mouse(window, event);
            }
            NativeEvent::Scroll(event) => dispatch_scroll(window, event),
            NativeEvent::Hover(hovered) => update_hover(window, hovered),
            NativeEvent::Focused(active) => {
                if active {
                    let windows = self.windows.borrow().clone();
                    for other in windows {
                        if !Rc::ptr_eq(&other, window) {
                            update_active(&other, false);
                        }
                    }
                }
                update_active(window, active);
            }
        }
    }

    fn dispatch_arkui_key_event(&self, event: ArkUiKeyEvent) -> bool {
        self.dispatcher.drain_main_queue();
        let Some(window) = self.active_window_state() else {
            return false;
        };
        dispatch_key(&window, event)
    }

    fn close_auxiliary_window(&self) {
        let window = self
            .windows
            .borrow()
            .iter()
            .find(|window| !window.borrow().primary)
            .cloned();
        let Some(window) = window else {
            return;
        };
        let mut should_close = window.borrow_mut().callbacks.should_close.take();
        let allowed = should_close.as_mut().is_none_or(|callback| callback());
        if let Some(callback) = should_close {
            window.borrow_mut().callbacks.should_close = Some(callback);
        }
        if allowed && let Some(callback) = window.borrow_mut().callbacks.close.take() {
            callback();
        }
    }

    fn release_window(&self, window: &Rc<RefCell<WindowState>>) {
        let (component, primary, title) = {
            let state = window.borrow();
            (state.component, state.primary, state.title.clone())
        };
        self.windows
            .borrow_mut()
            .retain(|candidate| !Rc::ptr_eq(candidate, window));
        clear_event_handler(component);
        if !primary
            && let Some(set_auxiliary_window) = self.set_auxiliary_window.borrow().as_ref().cloned()
            && let Err(error) = set_auxiliary_window(false, title, 0, 0)
        {
            log_message(
                LogLevel::Error,
                format!("failed to hide the HarmonyOS auxiliary window: {error:#}"),
            );
        }
    }

    fn handle_surface_event(&self, window: &Rc<RefCell<WindowState>>, event: NativeSurfaceEvent) {
        match event {
            NativeSurfaceEvent::ResizePending => self.request_frame(),
            NativeSurfaceEvent::Created { width, height }
            | NativeSurfaceEvent::Resized { width, height } => {
                let (size, scale_factor, primary, mut callback) = {
                    let mut state = window.borrow_mut();
                    state.surface_available = true;
                    state.surface_size = Some((width, height));
                    let size = Size::new(
                        px(width as f32 / state.scale_factor),
                        px(height as f32 / state.scale_factor),
                    );
                    state.bounds = Bounds::new(Point::default(), size);
                    state.frame_requested = true;
                    let callback = state.callbacks.resize.take();
                    (size, state.scale_factor, state.primary, callback)
                };
                if primary {
                    self.display
                        .bounds
                        .replace(Bounds::new(Point::default(), size));
                }
                #[cfg(debug_assertions)]
                log_message(
                    LogLevel::Debug,
                    format!(
                        "GPUI surface bounds: {width}x{height} physical, {:.1}x{:.1} logical, scale {scale_factor:.3}",
                        f32::from(size.width),
                        f32::from(size.height),
                    ),
                );
                if let Some(callback) = callback.as_mut() {
                    callback(size, scale_factor);
                }
                if let Some(callback) = callback {
                    window.borrow_mut().callbacks.resize = Some(callback);
                }
                self.request_frame();
            }
            NativeSurfaceEvent::Destroyed => {
                let mut state = window.borrow_mut();
                state.surface_available = false;
                state.surface_size = None;
            }
        }
    }
}

impl Drop for OhosPlatform {
    fn drop(&mut self) {
        self.dispatcher.clear_wake_callback();
    }
}

impl Platform for OhosPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.text_system.clone()
    }

    fn run(&self, on_finish_launching: Box<dyn FnOnce()>) {
        on_finish_launching();
        self.request_frame();
    }

    fn quit(&self) {
        if let Some(callback) = self.callbacks.borrow_mut().quit.as_mut() {
            callback();
        }
    }

    fn restart(&self, _binary_path: Option<PathBuf>) {
        log_message(LogLevel::Warning, "process restart is owned by HarmonyOS");
    }

    fn activate(&self, _ignoring_other_apps: bool) {}
    fn hide(&self) {}
    fn hide_other_apps(&self) {}
    fn unhide_other_apps(&self) {}

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        vec![self.display.clone()]
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(self.display.clone())
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        self.active_window_state()
            .map(|window| window.borrow().handle)
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowParams,
    ) -> Result<Box<dyn PlatformWindow>> {
        let primary = self.windows.borrow().is_empty();
        if !primary && self.windows.borrow().len() >= 2 {
            bail!("HarmonyOS currently exposes one workspace and one auxiliary window");
        }
        let component = if primary {
            self.component
        } else {
            xcomponent_by_id("zed-settings-surface")?
        };
        if self
            .windows
            .borrow()
            .iter()
            .any(|window| window.borrow().component == component)
        {
            bail!("HarmonyOS XComponent is already attached to a GPUI window");
        }
        let title = options
            .titlebar
            .as_ref()
            .and_then(|titlebar| titlebar.title.as_ref())
            .map(ToString::to_string)
            .unwrap_or_else(|| "Zed".to_owned());
        let auxiliary_window = if primary {
            None
        } else {
            let width = physical_window_dimension(
                options.bounds.size.width,
                self.scale_factor.get(),
                "width",
            )?;
            let height = physical_window_dimension(
                options.bounds.size.height,
                self.scale_factor.get(),
                "height",
            )?;
            let callback = self
                .set_auxiliary_window
                .borrow()
                .as_ref()
                .cloned()
                .context("HarmonyOS auxiliary-window bridge is not configured")?;
            Some((callback, width, height))
        };
        let atlas = OhosAtlas::new();
        let state = Rc::new(RefCell::new(WindowState {
            handle,
            component,
            primary,
            bounds: options.bounds,
            scale_factor: self.scale_factor.get(),
            mouse_position: Point::default(),
            modifiers: Modifiers::default(),
            capslock: Capslock::default(),
            pressed_keys: HashSet::new(),
            pressed_mouse_button: None,
            touch_mouse_id: None,
            input_handler: None,
            title: title.clone(),
            active: true,
            hovered: false,
            fullscreen: primary,
            surface_available: false,
            surface_size: None,
            screen_origin: Point::default(),
            ime_candidate_bounds: None,
            frame_requested: true,
            background: WindowBackgroundAppearance::Opaque,
            callbacks: WindowCallbacks::default(),
            atlas,
        }));
        if !primary {
            for window in self.windows.borrow().iter() {
                window.borrow_mut().active = false;
            }
        }
        self.windows.borrow_mut().push(state.clone());
        let weak_state = Rc::downgrade(&state);
        if let Err(error) = set_event_handler(
            component,
            Box::new(move |event| {
                let Some(state) = weak_state.upgrade() else {
                    return;
                };
                CURRENT_PLATFORM.with_borrow(|current| {
                    if let Some(platform) = current.as_ref() {
                        platform.handle_native_event(&state, event);
                    }
                });
            }),
        ) {
            self.windows
                .borrow_mut()
                .retain(|window| !Rc::ptr_eq(window, &state));
            return Err(error);
        }
        if primary {
            self.display.bounds.replace(options.bounds);
        } else if let Some((set_auxiliary_window, width, height)) = auxiliary_window {
            if let Err(error) = set_auxiliary_window(true, title, width, height) {
                self.windows
                    .borrow_mut()
                    .retain(|window| !Rc::ptr_eq(window, &state));
                clear_event_handler(component);
                return Err(error);
            }
        }
        self.request_frame();
        Ok(Box::new(OhosWindow(state)))
    }

    fn window_appearance(&self) -> WindowAppearance {
        WindowAppearance::Dark
    }

    fn open_url(&self, url: &str) {
        let Some(open_external_url) = self.open_external_url.borrow().as_ref().cloned() else {
            log_message(
                LogLevel::Error,
                format!("cannot open external URL before ArkUI is configured: {url}"),
            );
            return;
        };
        if let Err(error) = open_external_url(url.to_owned()) {
            log_message(
                LogLevel::Error,
                format!("failed to ask HarmonyOS to open {url}: {error:#}"),
            );
        }
    }

    fn on_open_urls(&self, _callback: Box<dyn FnMut(Vec<String>)>) {}

    fn register_url_scheme(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        let (sender, receiver) = oneshot::channel();
        let Some(request_path_prompt) = self.request_path_prompt.borrow().as_ref().cloned() else {
            if sender
                .send(Err(anyhow::anyhow!("ArkUI path picker is not configured")))
                .is_err()
            {
                log_message(
                    LogLevel::Warning,
                    "path picker receiver dropped before configuration error could be sent",
                );
            }
            return receiver;
        };
        let request_id = self.next_path_prompt_id.get();
        self.next_path_prompt_id
            .set(request_id.wrapping_add(1).max(1));
        self.pending_path_prompts
            .borrow_mut()
            .insert(request_id, sender);
        log_message(
            LogLevel::Info,
            format!(
                "requesting HarmonyOS path prompt {request_id} (files={}, directories={}, multiple={})",
                options.files, options.directories, options.multiple
            ),
        );
        if let Err(error) = request_path_prompt(
            request_id,
            options.files,
            options.directories,
            options.multiple,
            false,
            None,
            None,
        ) {
            self.complete_path_prompt(
                request_id,
                Vec::new(),
                Some(format!("failed to open HarmonyOS path picker: {error:#}")),
            );
        }
        receiver
    }

    fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        let (sender, receiver) = oneshot::channel();
        let Some(request_path_prompt) = self.request_path_prompt.borrow().as_ref().cloned() else {
            if sender
                .send(Err(anyhow::anyhow!("ArkUI path picker is not configured")))
                .is_err()
            {
                log_message(
                    LogLevel::Warning,
                    "new path picker receiver dropped before configuration error could be sent",
                );
            }
            return receiver;
        };
        let request_id = self.next_path_prompt_id.get();
        self.next_path_prompt_id
            .set(request_id.wrapping_add(1).max(1));
        self.pending_new_path_prompts
            .borrow_mut()
            .insert(request_id, sender);
        let default_uri = self.persistent_uris.borrow().get(directory).cloned();
        log_message(
            LogLevel::Info,
            format!(
                "requesting HarmonyOS new path prompt {request_id} in {}",
                directory.display()
            ),
        );
        if let Err(error) = request_path_prompt(
            request_id,
            true,
            false,
            false,
            true,
            suggested_name.map(ToOwned::to_owned),
            default_uri,
        ) {
            self.complete_path_prompt(
                request_id,
                Vec::new(),
                Some(format!(
                    "failed to open HarmonyOS new path picker: {error:#}"
                )),
            );
        }
        receiver
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        true
    }

    fn reveal_path(&self, path: &Path) {
        log_message(
            LogLevel::Info,
            format!("requested reveal for {}", path.display()),
        );
    }

    fn open_with_system(&self, path: &Path) {
        log_message(
            LogLevel::Info,
            format!("requested system open for {}", path.display()),
        );
    }

    fn on_quit(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().quit = Some(callback);
    }

    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().reopen = Some(callback);
    }

    fn on_system_wake(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().system_wake = Some(callback);
    }

    fn on_app_lifecycle(&self, callback: Box<dyn FnMut(AppLifecyclePhase)>) {
        self.callbacks.borrow_mut().lifecycle = Some(callback);
    }

    fn on_memory_warning(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().memory_warning = Some(callback);
    }

    fn set_menus(&self, menus: Vec<Menu>, _keymap: &Keymap) {
        self.menus
            .replace(menus.into_iter().map(Menu::owned).collect());
        self.request_frame();
    }
    fn set_dock_menu(&self, _menu: Vec<MenuItem>, _keymap: &Keymap) {}
    fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        Some(self.menus.borrow().clone())
    }
    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
        self.callbacks.borrow_mut().app_menu_action = Some(callback);
    }
    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().will_open_app_menu = Some(callback);
    }
    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
        self.callbacks.borrow_mut().validate_app_menu_command = Some(callback);
    }

    fn thermal_state(&self) -> ThermalState {
        ThermalState::Nominal
    }
    fn on_thermal_state_change(&self, _callback: Box<dyn FnMut()>) {}

    fn app_path(&self) -> Result<PathBuf> {
        std::env::current_exe().context("failed to obtain HarmonyOS application path")
    }

    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        crate::hnp::packaged_executable(name)
    }

    fn set_cursor_style(&self, style: CursorStyle) {
        self.update_cursor(style, true);
    }
    fn hide_cursor_until_mouse_moves(&self) {
        self.update_cursor(self.cursor_style.get(), false);
    }
    fn is_cursor_visible(&self) -> bool {
        self.cursor_visible.get()
    }
    fn should_auto_hide_scrollbars(&self) -> bool {
        true
    }

    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        match crate::clipboard::read_text() {
            Ok(Some(text)) => Some(ClipboardItem::new_string(text)),
            Ok(None) => None,
            Err(error) => {
                log_message(
                    LogLevel::Error,
                    format!("failed to read the HarmonyOS pasteboard: {error:#}"),
                );
                None
            }
        }
    }

    fn write_to_clipboard(&self, item: ClipboardItem) {
        let Some(text) = item.text() else {
            log_message(
                LogLevel::Warning,
                "HarmonyOS pasteboard currently supports plain-text GPUI entries",
            );
            return;
        };
        if let Err(error) = crate::clipboard::write_text(&text) {
            log_message(
                LogLevel::Error,
                format!("failed to write the HarmonyOS pasteboard: {error:#}"),
            );
        }
    }

    fn read_from_primary(&self) -> Option<ClipboardItem> {
        self.read_from_clipboard()
    }

    fn write_to_primary(&self, item: ClipboardItem) {
        self.write_to_clipboard(item);
    }

    fn write_credentials(&self, url: &str, username: &str, password: &[u8]) -> Task<Result<()>> {
        let url = url.to_owned();
        let username = username.to_owned();
        let password = password.to_vec();
        self.background_executor()
            .spawn(async move { crate::credentials::write(&url, &username, &password) })
    }

    fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        let url = url.to_owned();
        self.background_executor()
            .spawn(async move { crate::credentials::read(&url) })
    }

    fn delete_credentials(&self, url: &str) -> Task<Result<()>> {
        let url = url.to_owned();
        self.background_executor()
            .spawn(async move { crate::credentials::delete(&url) })
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(OhosKeyboardLayout)
    }

    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        Rc::new(DummyKeyboardMapper)
    }

    fn on_keyboard_layout_change(&self, _callback: Box<dyn FnMut()>) {}
}

#[derive(Default, Debug)]
struct OhosDisplay {
    bounds: RefCell<Bounds<Pixels>>,
}

impl PlatformDisplay for OhosDisplay {
    fn id(&self) -> DisplayId {
        DisplayId::new(0)
    }

    fn uuid(&self) -> Result<Uuid> {
        Ok(Uuid::nil())
    }

    fn bounds(&self) -> Bounds<Pixels> {
        *self.bounds.borrow()
    }
}

struct OhosKeyboardLayout;

impl PlatformKeyboardLayout for OhosKeyboardLayout {
    fn id(&self) -> &str {
        "harmonyos.current"
    }

    fn name(&self) -> &str {
        "HarmonyOS Current Keyboard"
    }
}

#[derive(Default)]
struct WindowCallbacks {
    request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    input: Option<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>,
    active: Option<Box<dyn FnMut(bool)>>,
    hover: Option<Box<dyn FnMut(bool)>>,
    resize: Option<Box<dyn FnMut(Size<Pixels>, f32)>>,
    moved: Option<Box<dyn FnMut()>>,
    should_close: Option<Box<dyn FnMut() -> bool>>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
    hit_test: Option<Box<dyn FnMut() -> Option<WindowControlArea>>>,
}

struct WindowState {
    handle: AnyWindowHandle,
    component: XComponentHandle,
    primary: bool,
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    mouse_position: Point<Pixels>,
    modifiers: Modifiers,
    capslock: Capslock,
    pressed_keys: HashSet<i32>,
    pressed_mouse_button: Option<MouseButton>,
    touch_mouse_id: Option<u64>,
    input_handler: Option<PlatformInputHandler>,
    title: String,
    active: bool,
    hovered: bool,
    fullscreen: bool,
    surface_available: bool,
    surface_size: Option<(u32, u32)>,
    screen_origin: Point<Pixels>,
    ime_candidate_bounds: Option<Bounds<Pixels>>,
    frame_requested: bool,
    background: WindowBackgroundAppearance,
    callbacks: WindowCallbacks,
    atlas: Arc<OhosAtlas>,
}

struct OhosWindow(Rc<RefCell<WindowState>>);

impl Drop for OhosWindow {
    fn drop(&mut self) {
        CURRENT_PLATFORM.with_borrow(|current| {
            if let Some(platform) = current.as_ref() {
                platform.release_window(&self.0);
            }
        });
    }
}

impl HasWindowHandle for OhosWindow {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        Err(raw_window_handle::HandleError::NotSupported)
    }
}

impl HasDisplayHandle for OhosWindow {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError>
    {
        Err(raw_window_handle::HandleError::NotSupported)
    }
}

impl PlatformWindow for OhosWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().bounds
    }
    fn is_maximized(&self) -> bool {
        self.0.borrow().primary
    }
    fn window_bounds(&self) -> WindowBounds {
        if self.0.borrow().primary {
            WindowBounds::Fullscreen(self.bounds())
        } else {
            WindowBounds::Windowed(self.bounds())
        }
    }
    fn content_size(&self) -> Size<Pixels> {
        self.bounds().size
    }
    fn resize(&mut self, size: Size<Pixels>) {
        let (primary, title, scale_factor) = {
            let mut state = self.0.borrow_mut();
            state.bounds.size = size;
            (state.primary, state.title.clone(), state.scale_factor)
        };
        if primary {
            return;
        }
        let result = (|| -> Result<()> {
            let width = physical_window_dimension(size.width, scale_factor, "width")?;
            let height = physical_window_dimension(size.height, scale_factor, "height")?;
            CURRENT_PLATFORM.with_borrow(|current| {
                let platform = current
                    .as_ref()
                    .context("GPUI platform is unavailable while resizing a window")?;
                let callback = platform
                    .set_auxiliary_window
                    .borrow()
                    .as_ref()
                    .cloned()
                    .context("HarmonyOS auxiliary-window bridge is not configured")?;
                callback(true, title, width, height)
            })
        })();
        if let Err(error) = result {
            log_message(
                LogLevel::Error,
                format!("failed to resize the HarmonyOS auxiliary window: {error:#}"),
            );
        }
    }
    fn scale_factor(&self) -> f32 {
        self.0.borrow().scale_factor
    }
    fn appearance(&self) -> WindowAppearance {
        WindowAppearance::Dark
    }
    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        CURRENT_PLATFORM.with_borrow(|current| {
            current
                .as_ref()
                .map(|platform| platform.display.clone() as Rc<dyn PlatformDisplay>)
        })
    }
    fn mouse_position(&self) -> Point<Pixels> {
        self.0.borrow().mouse_position
    }
    fn modifiers(&self) -> Modifiers {
        self.0.borrow().modifiers
    }
    fn capslock(&self) -> Capslock {
        self.0.borrow().capslock
    }
    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.0.borrow_mut().input_handler = Some(input_handler);
    }
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.borrow_mut().input_handler.take()
    }
    fn prompt(
        &self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        CURRENT_PLATFORM.with_borrow(|current| {
            current
                .as_ref()
                .and_then(|platform| platform.request_prompt(level, message, detail, answers))
        })
    }
    fn activate(&self) {
        CURRENT_PLATFORM.with_borrow(|current| {
            let Some(platform) = current.as_ref() else {
                return;
            };
            for window in platform.windows.borrow().iter() {
                window.borrow_mut().active = Rc::ptr_eq(window, &self.0);
            }
            if !self.0.borrow().primary {
                let (title, size, scale_factor) = {
                    let state = self.0.borrow();
                    (state.title.clone(), state.bounds.size, state.scale_factor)
                };
                let result = (|| -> Result<()> {
                    let width = physical_window_dimension(size.width, scale_factor, "width")?;
                    let height = physical_window_dimension(size.height, scale_factor, "height")?;
                    let callback = platform
                        .set_auxiliary_window
                        .borrow()
                        .as_ref()
                        .cloned()
                        .context("HarmonyOS auxiliary-window bridge is not configured")?;
                    callback(true, title, width, height)
                })();
                if let Err(error) = result {
                    log_message(
                        LogLevel::Error,
                        format!("failed to activate the HarmonyOS auxiliary window: {error:#}"),
                    );
                }
            }
        });
    }
    fn is_active(&self) -> bool {
        self.0.borrow().active
    }
    fn is_hovered(&self) -> bool {
        self.0.borrow().hovered
    }
    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.0.borrow().background
    }
    fn set_title(&mut self, title: &str) {
        self.0.borrow_mut().title = title.to_owned();
    }
    fn get_title(&self) -> String {
        self.0.borrow().title.clone()
    }
    fn set_background_appearance(&self, appearance: WindowBackgroundAppearance) {
        self.0.borrow_mut().background = appearance;
    }
    fn minimize(&self) {}
    fn zoom(&self) {}
    fn toggle_fullscreen(&self) {
        let fullscreen = self.0.borrow().fullscreen;
        self.0.borrow_mut().fullscreen = !fullscreen;
    }
    fn is_fullscreen(&self) -> bool {
        self.0.borrow().fullscreen
    }
    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        let window = Rc::downgrade(&self.0);
        Some(Rc::new(move || request_window_frame(&window)))
    }
    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.borrow_mut().callbacks.request_frame = Some(callback);
        request_window_frame(&Rc::downgrade(&self.0));
    }
    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.0.borrow_mut().callbacks.input = Some(callback);
    }
    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.borrow_mut().callbacks.active = Some(callback);
    }
    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.borrow_mut().callbacks.hover = Some(callback);
    }
    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.borrow_mut().callbacks.resize = Some(callback);
    }
    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.0.borrow_mut().callbacks.moved = Some(callback);
    }
    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.borrow_mut().callbacks.should_close = Some(callback);
    }
    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.0.borrow_mut().callbacks.hit_test = Some(callback);
    }
    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.borrow_mut().callbacks.close = Some(callback);
    }
    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.borrow_mut().callbacks.appearance_changed = Some(callback);
    }
    fn draw(&self, scene: &Scene) {
        #[cfg(debug_assertions)]
        log_message(
            LogLevel::Debug,
            format!(
                "drawing scene: quads={} underlines={} monochrome={} subpixel={} polychrome={} shadows={} paths={} surfaces={}",
                scene.quads.len(),
                scene.underlines.len(),
                scene.monochrome_sprites.len(),
                scene.subpixel_sprites.len(),
                scene.polychrome_sprites.len(),
                scene.shadows.len(),
                scene.paths.len(),
                scene.surfaces.len(),
            ),
        );
        let (component, atlas) = {
            let state = self.0.borrow();
            (state.component, state.atlas.clone())
        };
        if let Err(error) = with_surface(component, |surface| surface.draw_scene(scene, &atlas)) {
            log_message(
                LogLevel::Error,
                format!("failed to draw GPUI scene: {error:#}"),
            );
        }
    }
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.borrow().atlas.clone()
    }
    fn is_subpixel_rendering_supported(&self) -> bool {
        false
    }
    fn gpu_specs(&self) -> Option<GpuSpecs> {
        None
    }
    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        self.0.borrow_mut().ime_candidate_bounds = Some(bounds);
    }
}

fn request_window_frame(window: &Weak<RefCell<WindowState>>) {
    if let Some(window) = window.upgrade() {
        window.borrow_mut().frame_requested = true;
        CURRENT_PLATFORM.with_borrow(|current| {
            if let Some(platform) = current.as_ref() {
                platform.dispatcher.request_main_wake();
            }
        });
    }
}

fn physical_window_dimension(dimension: Pixels, scale_factor: f32, name: &str) -> Result<u32> {
    let physical = f32::from(dimension) * scale_factor;
    if !physical.is_finite() || physical <= 0.0 || physical > u32::MAX as f32 {
        bail!("HarmonyOS window {name} {physical} is outside the supported physical range");
    }
    Ok(physical.round() as u32)
}

fn cursor_style_code(style: CursorStyle) -> u32 {
    match style {
        CursorStyle::Arrow => 0,
        CursorStyle::IBeam => 1,
        CursorStyle::Crosshair => 2,
        CursorStyle::ClosedHand => 3,
        CursorStyle::OpenHand => 4,
        CursorStyle::PointingHand => 5,
        CursorStyle::ResizeLeft => 6,
        CursorStyle::ResizeRight => 7,
        CursorStyle::ResizeLeftRight | CursorStyle::ResizeColumn => 8,
        CursorStyle::ResizeUp => 9,
        CursorStyle::ResizeDown => 10,
        CursorStyle::ResizeUpDown | CursorStyle::ResizeRow => 11,
        CursorStyle::ResizeUpLeftDownRight => 12,
        CursorStyle::ResizeUpRightDownLeft => 13,
        CursorStyle::IBeamCursorForVerticalLayout => 14,
        CursorStyle::OperationNotAllowed => 15,
        CursorStyle::DragLink => 16,
        CursorStyle::DragCopy => 17,
        CursorStyle::ContextualMenu => 18,
    }
}

fn dispatch_input(window: &Rc<RefCell<WindowState>>, input: PlatformInput) -> DispatchEventResult {
    let mut callback = window.borrow_mut().callbacks.input.take();
    let mut result = DispatchEventResult {
        propagate: true,
        default_prevented: false,
    };
    if let Some(callback) = callback.as_mut() {
        result = callback(input);
    }
    if let Some(callback) = callback {
        window.borrow_mut().callbacks.input = Some(callback);
    }
    result
}

fn dispatch_text(window: &Rc<RefCell<WindowState>>, text: &str) {
    let mut input_handler = window.borrow_mut().input_handler.take();
    if let Some(handler) = input_handler.as_mut() {
        if handler.query_accepts_text_input() {
            log_message(LogLevel::Info, format!("committing text input {text:?}"));
            handler.replace_text_in_range(None, text);
        } else {
            log_message(LogLevel::Warning, "focused handler rejected text input");
        }
    } else {
        log_message(
            LogLevel::Warning,
            "text input arrived without a focused handler",
        );
    }
    if let Some(input_handler) = input_handler {
        window.borrow_mut().input_handler = Some(input_handler);
    }
}

fn with_input_handler<R>(callback: impl FnOnce(&mut PlatformInputHandler) -> R) -> Option<R> {
    CURRENT_PLATFORM.with_borrow(|current| {
        let platform = current.as_ref()?;
        let window = platform.active_window_state()?;
        let mut handler = window.borrow_mut().input_handler.take()?;
        let result = catch_unwind(AssertUnwindSafe(|| callback(&mut handler)));
        window.borrow_mut().input_handler = Some(handler);
        match result {
            Ok(value) => Some(value),
            Err(panic) => resume_unwind(panic),
        }
    })
}

pub(crate) fn ime_selection() -> Option<UTF16Selection> {
    with_input_handler(|handler| handler.selected_text_range(true)).flatten()
}

pub(crate) fn ime_text_length() -> Option<usize> {
    with_input_handler(PlatformInputHandler::text_length_utf16).flatten()
}

pub(crate) fn ime_text_for_range(range: Range<usize>) -> Option<String> {
    with_input_handler(|handler| {
        let mut adjusted = None;
        handler.text_for_range(range, &mut adjusted)
    })
    .flatten()
}

pub(crate) fn ime_insert_text(text: &str) {
    let _ = with_input_handler(|handler| {
        if handler.query_accepts_text_input() {
            handler.replace_text_in_range(None, text);
        }
    });
}

pub(crate) fn ime_delete(forward: bool, length: usize) {
    if length == 0 {
        return;
    }
    let _ = with_input_handler(|handler| {
        let Some(selection) = handler.selected_text_range(true) else {
            return;
        };
        let range = if selection.range.is_empty() {
            if forward {
                let end = selection
                    .range
                    .end
                    .saturating_add(length)
                    .min(handler.text_length_utf16().unwrap_or(selection.range.end));
                selection.range.end..end
            } else {
                selection.range.start.saturating_sub(length)..selection.range.start
            }
        } else {
            selection.range
        };
        if !range.is_empty() {
            handler.replace_text_in_range(Some(range), "");
        }
    });
}

pub(crate) fn ime_set_selection(range: Range<usize>) {
    let _ = with_input_handler(|handler| {
        let length = handler.text_length_utf16().unwrap_or(range.end);
        let start = range.start.min(length);
        let end = range.end.min(length);
        handler.set_selected_text_range(start.min(end)..start.max(end));
    });
}

pub(crate) fn ime_set_composition(replacement: Option<Range<usize>>, text: &str) {
    let _ = with_input_handler(|handler| {
        handler.replace_and_mark_text_in_range(replacement, text, None);
    });
}

pub(crate) fn ime_finish_composition() {
    let _ = with_input_handler(PlatformInputHandler::unmark_text);
}

pub(crate) fn ime_move_cursor(direction: u32) {
    let key = match direction {
        1 => "up",
        2 => "down",
        3 => "left",
        4 => "right",
        _ => return,
    };
    dispatch_ime_keystroke(key, Modifiers::default());
}

pub(crate) fn ime_perform_action(action: u32) {
    let key = match action {
        0 => "a",
        3 => "x",
        4 => "c",
        5 => "v",
        _ => return,
    };
    dispatch_ime_keystroke(
        key,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
}

fn dispatch_ime_keystroke(key: &str, modifiers: Modifiers) {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            return;
        };
        let Some(window) = platform.active_window_state() else {
            return;
        };
        let keystroke = Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: None,
        };
        let _ = dispatch_input(
            &window,
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: true,
            }),
        );
        let _ = dispatch_input(&window, PlatformInput::KeyUp(KeyUpEvent { keystroke }));
    });
}

pub(crate) fn ime_candidate_rect() -> Option<(f64, f64, f64, f64)> {
    let computed = with_input_handler(PlatformInputHandler::ime_candidate_bounds).flatten();
    CURRENT_PLATFORM.with_borrow(|current| {
        let platform = current.as_ref()?;
        let window = platform.active_window_state()?;
        let state = window.borrow();
        let bounds = computed.or(state.ime_candidate_bounds)?;
        let scale = f64::from(state.scale_factor);
        let origin_x = f64::from(f32::from(state.screen_origin.x));
        let origin_y = f64::from(f32::from(state.screen_origin.y));
        Some((
            origin_x + f64::from(f32::from(bounds.origin.x)) * scale,
            origin_y + f64::from(f32::from(bounds.origin.y)) * scale,
            f64::from(f32::from(bounds.size.width)) * scale,
            f64::from(f32::from(bounds.size.height)) * scale,
        ))
    })
}

fn dispatch_touch(window: &Rc<RefCell<WindowState>>, event: NativeTouchEvent) {
    const MOUSE_SOURCE: u32 = 1;
    if event.source == MOUSE_SOURCE {
        return;
    }

    let (scale, position, modifiers) = {
        let mut state = window.borrow_mut();
        let position = point(
            px(event.x / state.scale_factor),
            px(event.y / state.scale_factor),
        );
        state.mouse_position = position;
        state.screen_origin = point(px(event.screen_x - event.x), px(event.screen_y - event.y));
        (state.scale_factor, position, state.modifiers)
    };
    let phase = match event.kind {
        0 => TouchPhase::Started,
        1 => TouchPhase::Ended,
        2 => TouchPhase::Moved,
        3 => TouchPhase::Cancelled,
        _ => return,
    };
    let _ = dispatch_input(
        window,
        PlatformInput::Touch(TouchEvent {
            id: TouchId(u64::try_from(event.id).unwrap_or_default()),
            phase,
            position: point(px(event.x / scale), px(event.y / scale)),
            force: (event.pressure > 0.0).then_some(event.pressure.clamp(0.0, 1.0)),
        }),
    );

    let touch_id = u64::try_from(event.id).unwrap_or_default();
    let tracked_touch = window.borrow().touch_mouse_id;
    match phase {
        TouchPhase::Started if tracked_touch.is_none() => {
            {
                let mut state = window.borrow_mut();
                state.touch_mouse_id = Some(touch_id);
                state.pressed_mouse_button = Some(MouseButton::Left);
            }
            let first_mouse = !window.borrow().active;
            let _ = dispatch_input(
                window,
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse,
                }),
            );
            show_ime_for_touch();
        }
        TouchPhase::Moved if tracked_touch == Some(touch_id) => {
            let _ = dispatch_input(
                window,
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: Some(MouseButton::Left),
                    modifiers,
                }),
            );
        }
        TouchPhase::Ended | TouchPhase::Cancelled if tracked_touch == Some(touch_id) => {
            {
                let mut state = window.borrow_mut();
                state.touch_mouse_id = None;
                state.pressed_mouse_button = None;
            }
            let _ = dispatch_input(
                window,
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                }),
            );
        }
        _ => {}
    }
}

fn dispatch_mouse(window: &Rc<RefCell<WindowState>>, event: NativeMouseEvent) {
    let (position, modifiers) = {
        let mut state = window.borrow_mut();
        let position = point(
            px(event.x / state.scale_factor),
            px(event.y / state.scale_factor),
        );
        state.mouse_position = position;
        state.screen_origin = point(px(event.screen_x - event.x), px(event.screen_y - event.y));
        (position, state.modifiers)
    };
    let button = mouse_button(event.button);
    let input = match event.action {
        1 => {
            let button = button.unwrap_or(MouseButton::Left);
            window.borrow_mut().pressed_mouse_button = Some(button);
            PlatformInput::MouseDown(MouseDownEvent {
                button,
                position,
                modifiers,
                click_count: 1,
                first_mouse: !window.borrow().active,
            })
        }
        2 => {
            let button = button
                .or(window.borrow().pressed_mouse_button)
                .unwrap_or(MouseButton::Left);
            window.borrow_mut().pressed_mouse_button = None;
            PlatformInput::MouseUp(MouseUpEvent {
                button,
                position,
                modifiers,
                click_count: 1,
            })
        }
        3 => PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: window.borrow().pressed_mouse_button,
            modifiers,
        }),
        _ => return,
    };
    let _ = dispatch_input(window, input);
}

fn dispatch_scroll(window: &Rc<RefCell<WindowState>>, event: NativeScrollEvent) {
    let (position, modifiers, scale_factor) = {
        let mut state = window.borrow_mut();
        let position = point(
            px(event.x / state.scale_factor),
            px(event.y / state.scale_factor),
        );
        state.mouse_position = position;
        (position, state.modifiers, state.scale_factor)
    };

    let delta = if event.precise {
        ScrollDelta::Pixels(point(
            px(event.delta_x / scale_factor),
            px(event.delta_y / scale_factor),
        ))
    } else {
        // HarmonyOS reports a mouse-wheel delta in degrees. One conventional
        // 15-degree detent scrolls three GPUI lines.
        const LINES_PER_DEGREE: f32 = 3.0 / 15.0;
        ScrollDelta::Lines(point(
            event.delta_x * LINES_PER_DEGREE,
            event.delta_y * LINES_PER_DEGREE,
        ))
    };
    let touch_phase = match event.action {
        1 => TouchPhase::Started,
        3 => TouchPhase::Ended,
        4 => TouchPhase::Cancelled,
        _ => TouchPhase::Moved,
    };
    let _dispatch_result = dispatch_input(
        window,
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta,
            modifiers,
            touch_phase,
        }),
    );
    #[cfg(debug_assertions)]
    log_message(
        LogLevel::Debug,
        format!(
            "scroll dispatched at ({:.1},{:.1}); propagate={}, default_prevented={}",
            f32::from(position.x),
            f32::from(position.y),
            _dispatch_result.propagate,
            _dispatch_result.default_prevented,
        ),
    );
}

fn show_ime_for_touch() {
    CURRENT_PLATFORM.with_borrow(|current| {
        let Some(platform) = current.as_ref() else {
            return;
        };
        let mut ime = platform.ime.borrow_mut();
        let Some(ime) = ime.as_mut() else {
            return;
        };
        if let Err(error) = ime.show_for_touch() {
            log_message(
                LogLevel::Error,
                format!("failed to show the HarmonyOS input method: {error:#}"),
            );
        }
    });
}

fn mouse_button(button: u32) -> Option<MouseButton> {
    match button {
        0x01 => Some(MouseButton::Left),
        0x02 => Some(MouseButton::Right),
        0x04 => Some(MouseButton::Middle),
        0x08 => Some(MouseButton::Navigate(NavigationDirection::Back)),
        0x10 => Some(MouseButton::Navigate(NavigationDirection::Forward)),
        _ => None,
    }
}

fn dispatch_key(window: &Rc<RefCell<WindowState>>, event: ArkUiKeyEvent) -> bool {
    #[cfg(debug_assertions)]
    log_message(
        LogLevel::Debug,
        format!(
            "key event action={} code={} text={:?} unicode={:?} modifiers={:#x}",
            event.action, event.code, event.key_text, event.unicode, event.modifiers
        ),
    );
    let is_down = event.action == 0;
    {
        let mut state = window.borrow_mut();
        state.modifiers.shift = event.modifiers & 1 != 0;
        state.modifiers.control = event.modifiers & 2 != 0;
        state.modifiers.alt = event.modifiers & 4 != 0;
    }
    if update_modifier(window, event.code, is_down) {
        let (modifiers, capslock) = {
            let state = window.borrow();
            (state.modifiers, state.capslock)
        };
        let _ = dispatch_input(
            window,
            PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers,
                capslock,
            }),
        );
        return true;
    }

    let modifiers = window.borrow().modifiers;
    let Some(key) = key_for_code(event.code) else {
        return false;
    };
    let key_char = system_key_char(&event);
    let keystroke = Keystroke {
        modifiers,
        key,
        key_char,
    };
    if is_down {
        let is_held = !window.borrow_mut().pressed_keys.insert(event.code);
        let text = keystroke.key_char.clone();
        let result = dispatch_input(
            window,
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke,
                is_held,
                prefer_character_input: false,
            }),
        );
        if result.propagate
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.platform
            && !modifiers.function
            && let Some(text) = text
        {
            dispatch_text(window, &text);
        }
    } else if event.action == 1 {
        window.borrow_mut().pressed_keys.remove(&event.code);
        let _ = dispatch_input(window, PlatformInput::KeyUp(KeyUpEvent { keystroke }));
    } else {
        return false;
    }
    true
}

fn update_modifier(window: &Rc<RefCell<WindowState>>, code: i32, is_down: bool) -> bool {
    let mut state = window.borrow_mut();
    match code {
        2045 | 2046 => state.modifiers.alt = is_down,
        2047 | 2048 => state.modifiers.shift = is_down,
        2072 | 2073 => state.modifiers.control = is_down,
        2076 | 2077 => state.modifiers.platform = is_down,
        0 | 2078 => state.modifiers.function = is_down,
        2074 => {
            if is_down {
                state.capslock.on = !state.capslock.on;
            }
        }
        _ => return false,
    }
    true
}

fn key_for_code(code: i32) -> Option<String> {
    if (2017..=2042).contains(&code) {
        let byte = b'a' + u8::try_from(code - 2017).ok()?;
        return Some(char::from(byte).to_string());
    }
    if (2000..=2009).contains(&code) {
        let character = char::from(b'0' + u8::try_from(code - 2000).ok()?);
        return Some(character.to_string());
    }
    if (2103..=2112).contains(&code) {
        let character = char::from(b'0' + u8::try_from(code - 2103).ok()?);
        return Some(character.to_string());
    }
    let key = match code {
        2010 => "*",
        2011 => "#",
        2012 => "up",
        2013 => "down",
        2014 => "left",
        2015 => "right",
        2043 => ",",
        2044 => ".",
        2049 => "tab",
        2050 => "space",
        2054 => "enter",
        2055 => "backspace",
        2056 => "`",
        2057 => "-",
        2058 => "=",
        2059 => "[",
        2060 => "]",
        2061 => "\\",
        2062 => ";",
        2063 => "'",
        2064 => "/",
        2065 => "@",
        2066 => "+",
        2068 => "pageup",
        2069 => "pagedown",
        2070 => "escape",
        2071 => "delete",
        2081 => "home",
        2082 => "end",
        2083 => "insert",
        2090..=2101 => return Some(format!("f{}", code - 2089)),
        2113 => "/",
        2114 => "*",
        2115 => "-",
        2116 => "+",
        2117 => ".",
        2118 => ",",
        2119 => "enter",
        2120 => "=",
        2121 => "(",
        2122 => ")",
        _ => return None,
    };
    Some(key.to_owned())
}

fn system_key_char(event: &ArkUiKeyEvent) -> Option<String> {
    if let Some(character) = event.unicode.and_then(char::from_u32)
        && !character.is_control()
    {
        return Some(character.to_string());
    }
    if event.code == 2050 {
        return Some(" ".to_owned());
    }
    let mut characters = event.key_text.chars();
    let character = characters.next()?;
    if characters.next().is_none() && !character.is_control() {
        Some(character.to_string())
    } else {
        None
    }
}

fn update_hover(window: &Rc<RefCell<WindowState>>, hovered: bool) {
    let mut callback = {
        let mut state = window.borrow_mut();
        state.hovered = hovered;
        state.callbacks.hover.take()
    };
    if let Some(callback) = callback.as_mut() {
        callback(hovered);
    }
    if let Some(callback) = callback {
        window.borrow_mut().callbacks.hover = Some(callback);
    }
}

fn update_active(window: &Rc<RefCell<WindowState>>, active: bool) {
    let mut callback = {
        let mut state = window.borrow_mut();
        state.active = active;
        state.callbacks.active.take()
    };
    if let Some(callback) = callback.as_mut() {
        callback(active);
    }
    if let Some(callback) = callback {
        window.borrow_mut().callbacks.active = Some(callback);
    }
    if !active {
        let capslock = {
            let mut state = window.borrow_mut();
            state.modifiers = Modifiers::default();
            state.pressed_keys.clear();
            state.capslock
        };
        let _ = dispatch_input(
            window,
            PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers: Modifiers::default(),
                capslock,
            }),
        );
        CURRENT_PLATFORM.with_borrow(|current| {
            if let Some(platform) = current.as_ref()
                && let Some(ime) = platform.ime.borrow().as_ref()
            {
                ime.hide();
            }
        });
    }
}
