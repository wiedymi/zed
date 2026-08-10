#![cfg(target_env = "ohos")]

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
pub use path_prompt::activate_persistent_uri;
pub use platform::{
    close_auxiliary_window, complete_path_prompt, complete_prompt, configure_frame_scheduler,
    current_platform, dispatch_arkui_key_event, handle_memory_warning, on_arkui_frame,
    persistent_uri_for_path, set_lifecycle_phase, set_scale_factor,
};
pub use xcomponent::{
    NativeEvent, NativeMouseEvent, NativeScrollEvent, NativeSurfaceEvent, NativeTouchEvent,
    XComponentHandle, current_xcomponent, register_xcomponent,
};
