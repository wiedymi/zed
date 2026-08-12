use ohos_sys::hilog;

const LOG_DOMAIN: u32 = 0xD002;
const LOG_TAG: &[u8] = b"ZedOHOS";

#[derive(Clone, Copy, Debug)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    fn native(self) -> hilog::LogLevel {
        match self {
            Self::Debug => hilog::LogLevel::LOG_DEBUG,
            Self::Info => hilog::LogLevel::LOG_INFO,
            Self::Warning => hilog::LogLevel::LOG_WARN,
            Self::Error => hilog::LogLevel::LOG_ERROR,
        }
    }
}

pub fn log_message(level: LogLevel, message: impl AsRef<str>) {
    let message = message.as_ref().as_bytes();
    // SAFETY: `OH_LOG_PrintMsgByLen` receives explicit byte lengths, so neither
    // the tag nor message needs to be NUL terminated. Both slices remain alive
    // for the duration of the call.
    let result = unsafe {
        hilog::OH_LOG_PrintMsgByLen(
            hilog::LogType::LOG_APP,
            level.native(),
            LOG_DOMAIN,
            LOG_TAG.as_ptr().cast(),
            LOG_TAG.len(),
            message.as_ptr().cast(),
            message.len(),
        )
    };
    if result < 0 {
        eprintln!("ZedOHOS logging failed with status {result}");
    }
}
