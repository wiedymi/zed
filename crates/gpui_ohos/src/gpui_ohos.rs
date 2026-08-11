#![cfg(target_env = "ohos")]

mod accessibility;
mod atlas;
mod clipboard;
mod credentials;
mod dispatcher;
mod drawing;
mod hnp;
mod ime;
mod logging;
mod path_prompt;
mod platform;
mod xcomponent;

pub use drawing::NativeDrawingSurface;
pub use hnp::packaged_executable;
pub use logging::{LogLevel, log_message};
pub use path_prompt::{activate_persistent_uri, resolve_incoming_uri};
pub use platform::{
    attach_auxiliary_window, close_auxiliary_window, complete_path_prompt, complete_prompt,
    configure_frame_scheduler, current_platform, dispatch_arkui_file_drop_event,
    dispatch_arkui_key_event, handle_memory_warning, handle_open_urls,
    handle_system_notification_response, on_arkui_frame, persistent_uri_for_path,
    set_accessibility_enabled, set_lifecycle_phase, set_scale_factor, set_system_appearance,
    set_thermal_level, update_arkui_window_activation, update_arkui_window_state,
};
pub use xcomponent::{
    NativeEvent, NativeMouseEvent, NativeScrollEvent, NativeSurfaceEvent, NativeTouchEvent,
    XComponentHandle, current_xcomponent, register_xcomponent,
};
