#![cfg(target_env = "ohos")]

use std::{
    any::Any,
    borrow::Cow,
    cell::RefCell,
    ffi::c_void,
    fs as std_fs,
    io::Write as _,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Once},
};

use agent_ui::AgentPanel;
use anyhow::{Context as _, Result, anyhow};
use assets::Assets;
use client::{Client, user::UserStore};
use clock::RealSystemClock;
use db::{AppDatabase, kvp::KeyValueStore};
use editor::Editor;
use fs::{Fs, RealFs};
use futures::{Future, StreamExt as _};
use git::GitHostingProviderRegistry;
use git_ui::git_panel::GitPanel;
use gpui::{
    App, AppContext as _, Application, ApplicationHandle, AsyncWindowContext, Context, Entity,
    Focusable as _, Menu, MenuItem, OsAction, PathPromptOptions, Subscription, TaskExt as _,
    UpdateGlobal as _, WeakEntity, WindowOptions,
};
use http_client::HttpClientWithUrl;
use language::{Buffer, BufferEvent, LanguageRegistry};
use napi_derive_ohos::napi;
use napi_ohos::{
    JsValue as _, Status,
    bindgen_prelude::{Env, FnArgs, Function, Object, Unknown},
    threadsafe_function::ThreadsafeFunctionPriority,
};
use node_runtime::{NodeBinaryOptions, NodeRuntime};
use outline_panel::OutlinePanel;
use project::{Project, ProjectPath, buffer_store::BufferStore};
use project_panel::ProjectPanel;
use prompt_store::PromptBuilder;
use release_channel::AppVersion;
use reqwest_client::ReqwestClient;
use session::{AppSession, Session};
use terminal_view::terminal_panel::TerminalPanel;
use theme::ActiveTheme as _;
use util::rel_path::RelPath;
use uuid::Uuid;
use workspace::{AppState, MultiWorkspace, OpenMode, Workspace, WorkspaceStore};

gpui::actions!(zed_ohos, [OpenFolder]);

const NATIVE_XCOMPONENT_OBJECT: &str = "__NATIVE_XCOMPONENT_OBJ__";
const SANDBOX_DOCUMENT_NAME: &str = "welcome.md";
const LAST_WORKSPACE_URI_KEY: &str = "ohos.last_workspace_uri";
const INITIAL_DOCUMENT: &str = "# Zed on HarmonyOS\n\nThis is a real Zed buffer stored in the app sandbox.\n\nEdit this text, close Zed, and reopen it: your changes are saved automatically.\n";
static INSTALL_PANIC_HOOK: Once = Once::new();

thread_local! {
    /// Retains GPUI for the lifetime of the ArkUI process. HarmonyOS owns the
    /// outer event loop, so dropping this handle would tear down all entities.
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
    static RUNTIME: RefCell<Option<OhosEditorRuntime>> = const { RefCell::new(None) };
}

struct OhosEditorRuntime {
    _editor: Entity<Editor>,
    _workspace: Entity<Workspace>,
    _multi_workspace: Entity<MultiWorkspace>,
    _project: Entity<Project>,
    _document: Entity<SandboxDocument>,
}

struct SandboxPaths {
    workspace_dir: PathBuf,
    document_path: PathBuf,
}

struct SandboxDocument {
    buffer_store: Entity<BufferStore>,
    buffer: Entity<Buffer>,
    _project: Entity<Project>,
    _buffer_subscription: Subscription,
    saving: bool,
    save_again: bool,
}

impl SandboxDocument {
    fn new(
        buffer_store: Entity<BufferStore>,
        project: Entity<Project>,
        buffer: Entity<Buffer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let buffer_subscription = cx.subscribe(&buffer, |this, _, event, cx| {
            if matches!(event, BufferEvent::Edited { .. }) {
                this.request_save(cx);
            }
        });
        Self {
            buffer_store,
            buffer,
            _project: project,
            _buffer_subscription: buffer_subscription,
            saving: false,
            save_again: false,
        }
    }

    fn request_save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            self.save_again = true;
            return;
        }

        self.saving = true;
        let buffer = self.buffer.clone();
        let save = self
            .buffer_store
            .update(cx, |store, cx| store.save_buffer(buffer, cx));
        cx.spawn(async move |this, cx| {
            let result = save.await;
            if let Err(error) = this.update(cx, |this, cx| this.save_finished(result, cx)) {
                gpui_ohos::log_message(
                    gpui_ohos::LogLevel::Error,
                    format!("sandbox save completion lost its document: {error}"),
                );
            }
        })
        .detach();
    }

    fn save_finished(&mut self, result: Result<()>, cx: &mut Context<Self>) {
        self.saving = false;
        if let Err(error) = result {
            gpui_ohos::log_message(
                gpui_ohos::LogLevel::Error,
                format!("failed to save the sandbox document: {error:#}"),
            );
        }

        if std::mem::take(&mut self.save_again) {
            self.request_save(cx);
        }
    }
}

#[napi]
pub fn native_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[napi]
pub fn initialize_frame_scheduler(
    env: Env,
    request_frame: Function<'_, (), ()>,
    open_external_url: Function<'_, String, ()>,
    set_cursor: Function<'_, (u32, bool), ()>,
    set_auxiliary_window: Function<'_, (bool, String, u32, u32), ()>,
    request_path_prompt: Function<
        '_,
        (u32, bool, bool, bool, bool, Option<String>, Option<String>),
        (),
    >,
    request_prompt: Function<'_, (u32, u32, String, Option<String>, Vec<String>), ()>,
    ui_context: Unknown<'_>,
    scale_factor: f64,
    files_dir: String,
    cache_dir: String,
) -> napi_ohos::Result<()> {
    let sandbox_paths = configure_sandbox_paths(&files_dir, &cache_dir)
        .map_err(|error| napi_ohos::Error::from_reason(format!("{error:#}")))?;
    let scale_factor = scale_factor as f32;
    // ArkUI creates the XComponent surface before this callback. Seed GPUI's
    // scale before opening the embedded window so its first real surface bounds
    // are logical pixels, rather than briefly laying the workspace out at the
    // physical size and correcting it afterwards.
    gpui_ohos::current_platform(false);
    gpui_ohos::set_scale_factor(scale_factor)
        .map_err(|error| napi_ohos::Error::from_reason(format!("{error:#}")))?;
    catch_unwind(AssertUnwindSafe(|| start_zed(sandbox_paths)))
        .map_err(|panic| {
            napi_ohos::Error::from_reason(format!(
                "Zed panicked during startup: {}",
                panic_message(panic.as_ref())
            ))
        })?
        .map_err(|error| napi_ohos::Error::from_reason(format!("{error:#}")))?;

    let request_frame = request_frame
        .build_threadsafe_function::<()>()
        .callee_handled::<false>()
        .build()?;
    let schedule_frame = Arc::new(move || {
        let status = request_frame.call_with_priority((), ThreadsafeFunctionPriority::Immediate);
        if status == Status::Ok {
            Ok(())
        } else {
            Err(anyhow!("N-API frame callback returned {status}"))
        }
    });
    let open_external_url = open_external_url
        .build_threadsafe_function::<String>()
        .callee_handled::<false>()
        .build()?;
    let open_external_url = Arc::new(move |url: String| {
        let status =
            open_external_url.call_with_priority(url, ThreadsafeFunctionPriority::Immediate);
        if status == Status::Ok {
            Ok(())
        } else {
            Err(anyhow!("N-API external URL callback returned {status}"))
        }
    });
    let set_cursor = set_cursor
        .build_threadsafe_function::<(u32, bool)>()
        .callee_handled::<false>()
        .build_callback(|context| Ok(FnArgs::from(context.value)))?;
    let set_cursor = Arc::new(move |style: u32, visible: bool| {
        let status =
            set_cursor.call_with_priority((style, visible), ThreadsafeFunctionPriority::Immediate);
        if status == Status::Ok {
            Ok(())
        } else {
            Err(anyhow!("N-API cursor callback returned {status}"))
        }
    });
    let set_auxiliary_window = set_auxiliary_window
        .build_threadsafe_function::<(bool, String, u32, u32)>()
        .callee_handled::<false>()
        .build_callback(|context| Ok(FnArgs::from(context.value)))?;
    let set_auxiliary_window = Arc::new(
        move |visible: bool, title: String, width: u32, height: u32| {
            let status = set_auxiliary_window.call_with_priority(
                (visible, title, width, height),
                ThreadsafeFunctionPriority::Immediate,
            );
            if status == Status::Ok {
                Ok(())
            } else {
                Err(anyhow!("N-API auxiliary-window callback returned {status}"))
            }
        },
    );
    let request_path_prompt = request_path_prompt
        .build_threadsafe_function::<(u32, bool, bool, bool, bool, Option<String>, Option<String>)>(
        )
        .callee_handled::<false>()
        .build_callback(|context| Ok(FnArgs::from(context.value)))?;
    let request_path_prompt = Arc::new(
        move |request_id: u32,
              files: bool,
              directories: bool,
              multiple: bool,
              save: bool,
              suggested_name: Option<String>,
              default_uri: Option<String>| {
            let status = request_path_prompt.call_with_priority(
                (
                    request_id,
                    files,
                    directories,
                    multiple,
                    save,
                    suggested_name,
                    default_uri,
                ),
                ThreadsafeFunctionPriority::Immediate,
            );
            if status == Status::Ok {
                Ok(())
            } else {
                Err(anyhow!("N-API path picker callback returned {status}"))
            }
        },
    );
    let request_prompt = request_prompt
        .build_threadsafe_function::<(u32, u32, String, Option<String>, Vec<String>)>()
        .callee_handled::<false>()
        .build_callback(|context| Ok(FnArgs::from(context.value)))?;
    let request_prompt = Arc::new(
        move |request_id: u32,
              level: u32,
              message: String,
              detail: Option<String>,
              answers: Vec<String>| {
            let status = request_prompt.call_with_priority(
                (request_id, level, message, detail, answers),
                ThreadsafeFunctionPriority::Immediate,
            );
            if status == Status::Ok {
                Ok(())
            } else {
                Err(anyhow!("N-API prompt callback returned {status}"))
            }
        },
    );
    let mut ui_context_handle = std::ptr::null_mut();
    // SAFETY: Both N-API handles belong to this active call, and the out
    // pointer remains valid for the duration of the conversion.
    let status = unsafe {
        ohos_sys::arkui::native_node_napi::OH_ArkUI_GetContextFromNapiValue(
            env.raw().cast(),
            ui_context.raw().cast(),
            &mut ui_context_handle,
        )
    };
    if status != 0 {
        return Err(napi_ohos::Error::from_reason(format!(
            "OH_ArkUI_GetContextFromNapiValue failed with status {status}"
        )));
    }
    let ui_context_handle = std::ptr::NonNull::new(ui_context_handle.cast::<c_void>())
        .ok_or_else(|| napi_ohos::Error::from_reason("ArkUI returned a null UIContext handle"))?;
    gpui_ohos::configure_frame_scheduler(
        schedule_frame,
        open_external_url,
        set_cursor,
        set_auxiliary_window,
        request_path_prompt,
        request_prompt,
        scale_factor,
        ui_context_handle,
    )
    .map_err(|error| napi_ohos::Error::from_reason(format!("{error:#}")))
}

#[napi]
pub fn close_auxiliary_window() -> napi_ohos::Result<()> {
    catch_unwind(AssertUnwindSafe(gpui_ohos::close_auxiliary_window)).map_err(|panic| {
        napi_ohos::Error::from_reason(format!(
            "Zed panicked while closing an auxiliary window: {}",
            panic_message(panic.as_ref())
        ))
    })
}

#[napi]
pub fn set_scale_factor(scale_factor: f64) -> napi_ohos::Result<()> {
    catch_unwind(AssertUnwindSafe(|| {
        gpui_ohos::set_scale_factor(scale_factor as f32)
    }))
    .map_err(|panic| {
        napi_ohos::Error::from_reason(format!(
            "Zed panicked while updating display scale: {}",
            panic_message(panic.as_ref())
        ))
    })?
    .map_err(|error| napi_ohos::Error::from_reason(format!("{error:#}")))
}

#[napi]
pub fn dispatch_key_event(
    action: u32,
    code: i32,
    key_text: String,
    unicode: Option<u32>,
    modifiers: u32,
) -> napi_ohos::Result<bool> {
    catch_unwind(AssertUnwindSafe(|| {
        gpui_ohos::dispatch_arkui_key_event(action, code, key_text, unicode, modifiers)
    }))
    .map_err(|panic| {
        napi_ohos::Error::from_reason(format!(
            "Zed panicked while handling a key event: {}",
            panic_message(panic.as_ref())
        ))
    })
}

#[napi]
pub fn set_lifecycle_phase(phase: u32) -> napi_ohos::Result<()> {
    catch_unwind(AssertUnwindSafe(|| gpui_ohos::set_lifecycle_phase(phase)))
        .map_err(|panic| {
            napi_ohos::Error::from_reason(format!(
                "Zed panicked while updating application lifecycle: {}",
                panic_message(panic.as_ref())
            ))
        })?
        .map_err(|error| napi_ohos::Error::from_reason(format!("{error:#}")))
}

#[napi]
pub fn handle_memory_warning() -> napi_ohos::Result<()> {
    catch_unwind(AssertUnwindSafe(gpui_ohos::handle_memory_warning)).map_err(|panic| {
        napi_ohos::Error::from_reason(format!(
            "Zed panicked while handling memory pressure: {}",
            panic_message(panic.as_ref())
        ))
    })
}

#[napi]
pub fn complete_path_prompt(
    request_id: u32,
    paths: Vec<String>,
    error: Option<String>,
) -> napi_ohos::Result<()> {
    catch_unwind(AssertUnwindSafe(|| {
        gpui_ohos::complete_path_prompt(request_id, paths, error)
    }))
    .map_err(|panic| {
        napi_ohos::Error::from_reason(format!(
            "Zed panicked while completing a path prompt: {}",
            panic_message(panic.as_ref())
        ))
    })
}

#[napi]
pub fn complete_prompt(
    request_id: u32,
    answer: Option<u32>,
    error: Option<String>,
) -> napi_ohos::Result<()> {
    catch_unwind(AssertUnwindSafe(|| {
        gpui_ohos::complete_prompt(request_id, answer, error)
    }))
    .map_err(|panic| {
        napi_ohos::Error::from_reason(format!(
            "Zed panicked while completing a prompt: {}",
            panic_message(panic.as_ref())
        ))
    })
}

#[napi]
pub fn on_frame() -> napi_ohos::Result<()> {
    catch_unwind(AssertUnwindSafe(gpui_ohos::on_arkui_frame)).map_err(|panic| {
        napi_ohos::Error::from_reason(format!(
            "Zed panicked while rendering a frame: {}",
            panic_message(panic.as_ref())
        ))
    })
}

#[napi(module_exports)]
fn initialize_native_surface(env: Env, exports: Object) -> napi_ohos::Result<()> {
    install_panic_hook();
    let native_object = exports
        .get::<Unknown>(NATIVE_XCOMPONENT_OBJECT)?
        .ok_or_else(|| {
            napi_ohos::Error::from_reason(format!(
                "native module export {NATIVE_XCOMPONENT_OBJECT} is missing"
            ))
        })?;
    let mut component = std::ptr::null_mut::<c_void>();
    napi_ohos::check_status!(
        // SAFETY: `env` and `native_object` are provided by the active N-API
        // module initialization call. `component` is a valid out pointer.
        unsafe { napi_ohos::sys::napi_unwrap(env.raw(), native_object.raw(), &mut component) },
        "failed to unwrap OH_NativeXComponent"
    )?;

    // SAFETY: A successful `napi_unwrap` of the XComponent-reserved export
    // yields the live `OH_NativeXComponent` pointer owned by ArkUI.
    unsafe { gpui_ohos::register_xcomponent(component) }
        .map_err(|error| napi_ohos::Error::from_reason(format!("{error:#}")))?;

    Ok(())
}

fn install_panic_hook() {
    INSTALL_PANIC_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let location = info
                .location()
                .map(|location| {
                    format!(
                        "{}:{}:{}",
                        location.file(),
                        location.line(),
                        location.column()
                    )
                })
                .unwrap_or_else(|| "unknown location".to_owned());
            gpui_ohos::log_message(
                gpui_ohos::LogLevel::Error,
                format!("panic at {location}: {}", panic_message(info.payload())),
            );
        }));
    });
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

fn configure_sandbox_paths(files_dir: &str, cache_dir: &str) -> Result<SandboxPaths> {
    let files_dir = require_absolute_directory(files_dir, "filesDir")?;
    let cache_dir = require_absolute_directory(cache_dir, "cacheDir")?;
    let data_dir = files_dir.join("zed-data");
    paths::try_set_custom_data_dir(&data_dir)
        .with_context(|| format!("configuring Zed data at {}", data_dir.display()))?;
    paths::try_set_custom_temp_dir(&cache_dir)
        .with_context(|| format!("configuring Zed cache at {}", cache_dir.display()))?;
    let git_metadata_dir = data_dir.join("git");
    std_fs::create_dir_all(&git_metadata_dir).with_context(|| {
        format!(
            "creating private HarmonyOS Git metadata root at {}",
            git_metadata_dir.display()
        )
    })?;

    let workspace_dir = files_dir.join("workspace");
    std_fs::create_dir_all(&workspace_dir).with_context(|| {
        format!(
            "creating the HarmonyOS sandbox workspace at {}",
            workspace_dir.display()
        )
    })?;
    let document_path = workspace_dir.join(SANDBOX_DOCUMENT_NAME);
    match std_fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&document_path)
    {
        Ok(mut file) => file
            .write_all(INITIAL_DOCUMENT.as_bytes())
            .with_context(|| format!("seeding {}", document_path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error).with_context(|| format!("creating {}", document_path.display()));
        }
    }

    Ok(SandboxPaths {
        workspace_dir,
        document_path,
    })
}

fn require_absolute_directory(path: &str, source: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    anyhow::ensure!(
        !path.as_os_str().is_empty(),
        "ArkUI supplied an empty {source}"
    );
    anyhow::ensure!(
        path.is_absolute(),
        "ArkUI supplied a relative {source}: {path:?}"
    );
    Ok(path.to_path_buf())
}

fn start_zed(sandbox_paths: SandboxPaths) -> Result<()> {
    if APPLICATION.with_borrow(Option::is_some) {
        return Ok(());
    }

    let launch_result = Rc::new(RefCell::new(None));
    let launch_result_for_callback = launch_result.clone();

    let application = Application::new_inaccessible(gpui_ohos::current_platform(false))
        .with_assets(Assets)
        .run_embedded(move |cx| {
            if let Err(error) = initialize_zed(cx, sandbox_paths) {
                launch_result_for_callback.replace(Some(error));
            }
        });

    if let Some(error) = launch_result.borrow_mut().take() {
        return Err(error);
    }
    APPLICATION.with_borrow_mut(|slot| *slot = Some(application));
    gpui_ohos::log_message(
        gpui_ohos::LogLevel::Info,
        "Zed runtime initialized on HarmonyOS",
    );
    Ok(())
}

fn initialize_zed(cx: &mut App, sandbox_paths: SandboxPaths) -> Result<()> {
    load_embedded_fonts(cx)?;
    release_channel::init(AppVersion::load(env!("CARGO_PKG_VERSION"), None, None), cx);
    gpui_tokio::init(cx);
    settings::init(cx);
    theme_settings::init(theme::LoadThemes::All(Box::new(Assets)), cx);
    menu::init();
    zed_actions::init();
    editor::init(cx);
    command_palette::init(cx);
    language_model::init(cx);
    git_ui::init(cx);
    project_panel::init(cx);
    outline_panel::init(cx);
    terminal_view::init(cx);

    let app_db = AppDatabase::new();
    let session_kvp = KeyValueStore::from_app_db(&app_db);
    let restored_workspace_path = session_kvp
        .read_kvp(LAST_WORKSPACE_URI_KEY)
        .context("reading the last HarmonyOS workspace URI")?
        .and_then(|uri| match gpui_ohos::activate_persistent_uri(&uri) {
            Ok(path) if path.is_dir() => Some(path),
            Ok(path) => {
                gpui_ohos::log_message(
                    gpui_ohos::LogLevel::Warning,
                    format!(
                        "the last HarmonyOS workspace is no longer a directory: {}",
                        path.display()
                    ),
                );
                None
            }
            Err(error) => {
                gpui_ohos::log_message(
                    gpui_ohos::LogLevel::Warning,
                    format!("failed to reactivate the last HarmonyOS workspace: {error:#}"),
                );
                None
            }
        });
    cx.set_global(app_db);
    let trusted_paths = match workspace::WorkspaceDb::global(cx).fetch_trusted_worktrees() {
        Ok(paths) => paths,
        Err(error) => {
            gpui_ohos::log_message(
                gpui_ohos::LogLevel::Error,
                format!("failed to read trusted HarmonyOS worktrees: {error:#}"),
            );
            Default::default()
        }
    };
    project::trusted_worktrees::init(trusted_paths, cx);

    let git_binary = gpui_ohos::packaged_executable("git")
        .context("locating the packaged HarmonyOS Git executable")?;
    let fs: Arc<dyn Fs> = Arc::new(RealFs::new(
        Some(git_binary),
        cx.background_executor().clone(),
    ));
    <dyn Fs>::set_global(fs.clone(), cx);
    settings::SettingsStore::update_global(cx, {
        let fs = fs.clone();
        move |store, cx| {
            store.watch_settings_files(fs, cx, |settings_file, result, _cx| {
                if let Err(error) = result.result() {
                    gpui_ohos::log_message(
                        gpui_ohos::LogLevel::Error,
                        format!("failed to load HarmonyOS {settings_file:?} settings: {error:#}"),
                    );
                }
            });
        }
    });
    initialize_keymaps(fs.clone(), cx);
    GitHostingProviderRegistry::set_global(Arc::new(GitHostingProviderRegistry::new()), cx);
    git_hosting_providers::init(cx);
    extension::init(cx);
    let user_agent = format!(
        "Zed/{} (HarmonyOS; {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH
    );
    let network_http = Arc::new(
        ReqwestClient::user_agent(&user_agent).context("initializing the HarmonyOS HTTP client")?,
    );
    cx.set_http_client(network_http.clone());
    let http = Arc::new(HttpClientWithUrl::new(
        network_http,
        "https://zed.dev",
        None,
    ));
    let client = Client::new(Arc::new(RealSystemClock), http, cx);
    Client::set_global(client.clone(), cx);
    client::init(&client, cx);
    Project::init(&client, cx);
    feature_flags::FeatureFlagStore::init(cx);

    let mut languages = LanguageRegistry::new(cx.background_executor().clone());
    languages.set_language_server_download_dir(paths::languages_dir().clone());
    let languages = Arc::new(languages);
    let node_binary = gpui_ohos::packaged_executable("node")
        .context("locating the packaged HarmonyOS Node.js executable")?;
    let npm_binary = gpui_ohos::packaged_executable("npm")
        .context("locating the packaged HarmonyOS npm executable")?;
    let (_, node_options) = watch::channel(Some(NodeBinaryOptions {
        allow_path_lookup: false,
        allow_binary_download: false,
        use_paths: Some((node_binary, npm_binary)),
    }));
    let node_runtime = NodeRuntime::new(client.http_client(), None, node_options);
    languages::init(languages.clone(), fs.clone(), node_runtime.clone(), cx);
    let mut language_server_statuses = languages.language_server_binary_statuses();
    cx.spawn(async move |_cx| {
        while let Some((server, status)) = language_server_statuses.next().await {
            gpui_ohos::log_message(
                match status {
                    language::BinaryStatus::Failed { .. } => gpui_ohos::LogLevel::Error,
                    _ => gpui_ohos::LogLevel::Info,
                },
                format!("language server {server}: {status:?}"),
            );
        }
    })
    .detach();
    let user_store = cx.new(|cx| UserStore::new(client.clone(), cx));
    let workspace_store = cx.new(|cx| WorkspaceStore::new(client.clone(), cx));
    let project = Project::local(
        client.clone(),
        node_runtime.clone(),
        user_store.clone(),
        languages.clone(),
        fs.clone(),
        None,
        Default::default(),
        cx,
    );
    let workspace_dir = sandbox_paths.workspace_dir;
    let document_path = sandbox_paths.document_path;
    let worktree = project.update(cx, |project, cx| {
        project.find_or_create_worktree(&workspace_dir, true, cx)
    });
    let session = cx
        .background_executor()
        .spawn(Session::new(Uuid::new_v4().to_string(), session_kvp));

    cx.spawn(async move |cx| {
        let result: Result<()> = async {
            let session = session.await;
            let app_state = cx.update(|cx| {
                let session = cx.new(|cx| AppSession::new(session, cx));
                let app_state = Arc::new(AppState {
                    languages,
                    client,
                    user_store,
                    workspace_store,
                    fs,
                    build_window_options: |_, _| WindowOptions::default(),
                    node_runtime,
                    session,
                });
                AppState::set_global(app_state.clone(), cx);
                client::RefreshLlmTokenListener::register(
                    app_state.client.clone(),
                    app_state.user_store.clone(),
                    cx,
                );
                let extension_host_proxy = extension::ExtensionHostProxy::global(cx);
                language_extension::init(
                    language_extension::LspAccess::ViaWorkspaces({
                        let workspace_store = app_state.workspace_store.clone();
                        Arc::new(move |cx: &mut App| {
                            workspace_store.update(cx, |workspace_store, cx| {
                                Ok(workspace_store
                                    .workspaces()
                                    .filter_map(|workspace| workspace.upgrade())
                                    .map(|workspace| {
                                        workspace.read(cx).project().read(cx).lsp_store()
                                    })
                                    .collect())
                            })
                        })
                    }),
                    extension_host_proxy.clone(),
                    app_state.languages.clone(),
                );
                extension_host::init(
                    extension_host_proxy.clone(),
                    app_state.fs.clone(),
                    app_state.client.clone(),
                    app_state.node_runtime.clone(),
                    cx,
                );
                theme_extension::init(
                    extension_host_proxy,
                    theme::ThemeRegistry::global(cx),
                    cx.background_executor().clone(),
                );
                language_models::init(app_state.user_store.clone(), app_state.client.clone(), cx);
                acp_tools::init(cx);
                web_search::init(cx);
                web_search_providers::init(
                    app_state.client.clone(),
                    app_state.user_store.clone(),
                    cx,
                );
                let prompt_builder = PromptBuilder::load(app_state.fs.clone(), false, cx);
                project::AgentRegistryStore::init_global(
                    cx,
                    app_state.fs.clone(),
                    app_state.client.http_client(),
                );
                agent_ui::init(
                    app_state.fs.clone(),
                    prompt_builder,
                    app_state.languages.clone(),
                    false,
                    false,
                    cx,
                );
                snippet_provider::init(cx);
                image_viewer::init(cx);
                diagnostics::init(cx);
                workspace::init(app_state.clone(), cx);
                initialize_workspaces(cx);
                ui_prompt::init(cx);
                go_to_line::init(cx);
                file_finder::init(cx);
                tab_switcher::init(cx);
                outline::init(cx);
                project_symbols::init(cx);
                tasks_ui::init(cx);
                snippets_ui::init(cx);
                search::init(cx);
                lsp_locations::init(cx);
                cx.set_global(workspace::PaneSearchBarCallbacks {
                    setup_search_bar: |languages, toolbar, window, cx| {
                        let search_bar =
                            cx.new(|cx| search::BufferSearchBar::new(languages, window, cx));
                        toolbar.update(cx, |toolbar, cx| {
                            toolbar.add_item(search_bar, window, cx);
                        });
                    },
                    wrap_div_with_search_actions:
                        search::buffer_search::register_pane_search_actions,
                });
                vim::init(cx);
                encoding_selector::init(cx);
                language_selector::init(cx);
                line_ending_selector::init(cx);
                toolchain_selector::init(cx);
                settings_profile_selector::init(cx);
                language_tools::init(cx);
                channel::init(&app_state.client.clone(), app_state.user_store.clone(), cx);
                notifications::init(app_state.client.clone(), app_state.user_store.clone(), cx);
                feedback::init(cx);
                markdown_preview::init(cx);
                csv_preview::init(cx);
                svg_preview::init(cx);
                onboarding::init(cx);
                json_schema_store::init(cx);
                recent_projects::init(cx);
                theme_selector::init(cx);
                settings_ui::init(cx);
                keymap_editor::init(cx);
                extensions_ui::init(cx);
                which_key::init(cx);
                title_bar::init(cx);
                app_state.languages.set_theme(cx.theme().clone());
                cx.observe_global::<theme::GlobalTheme>({
                    let languages = app_state.languages.clone();
                    move |cx| languages.set_theme(cx.theme().clone())
                })
                .detach();
                cx.set_menus(ohos_app_menus());
                cx.on_action(|_: &OpenFolder, cx| open_folder(cx));
                cx.on_action(|_: &zed_actions::Quit, cx| cx.quit());
                app_state
            });

            let (worktree, _) = worktree.await.context("opening the sandbox worktree")?;
            let relative_path = RelPath::from_unix_str(SANDBOX_DOCUMENT_NAME)
                .context("constructing the sandbox document path")?
                .into_arc();
            let project_path = ProjectPath {
                worktree_id: worktree.read_with(cx, |worktree, _| worktree.id()),
                path: relative_path,
            };
            let open_buffer =
                project.update(cx, |project, cx| project.open_buffer(project_path, cx));
            let buffer = open_buffer
                .await
                .with_context(|| format!("opening {}", document_path.display()))?;

            let window = cx.update(|cx| {
                let workspace_slot = Rc::new(RefCell::new(None));
                let workspace_slot_for_window = workspace_slot.clone();
                let editor_slot = Rc::new(RefCell::new(None));
                let editor_slot_for_window = editor_slot.clone();
                let project_for_window = project.clone();
                let buffer_for_window = buffer.clone();
                let app_state_for_window = app_state.clone();
                let window = cx
                    .open_window(WindowOptions::default(), move |window, cx| {
                        let workspace = cx.new(|cx| {
                            Workspace::new(
                                None,
                                project_for_window.clone(),
                                app_state_for_window.clone(),
                                window,
                                cx,
                            )
                        });
                        let editor = cx.new(|cx| {
                            Editor::for_buffer(
                                buffer_for_window,
                                Some(project_for_window),
                                window,
                                cx,
                            )
                        });
                        workspace.update(cx, |workspace, cx| {
                            workspace.add_item_to_active_pane(
                                Box::new(editor.clone()),
                                None,
                                true,
                                window,
                                cx,
                            );
                        });
                        window.focus(&editor.focus_handle(cx), cx);
                        workspace_slot_for_window.replace(Some(workspace.clone()));
                        editor_slot_for_window.replace(Some(editor));
                        cx.new(|cx| MultiWorkspace::new(workspace, window, cx))
                    })
                    .context("opening the HarmonyOS workspace window")?;
                let multi_workspace = window
                    .entity(cx)
                    .context("retaining the HarmonyOS multi-workspace entity")?;
                let workspace = workspace_slot
                    .borrow_mut()
                    .take()
                    .context("retaining the HarmonyOS workspace entity")?;
                let editor = editor_slot
                    .borrow_mut()
                    .take()
                    .context("retaining the HarmonyOS editor entity")?;
                let buffer_store = project.read(cx).buffer_store().clone();
                let document =
                    cx.new(|cx| SandboxDocument::new(buffer_store, project.clone(), buffer, cx));
                RUNTIME.with_borrow_mut(|slot| {
                    *slot = Some(OhosEditorRuntime {
                        _editor: editor,
                        _workspace: workspace,
                        _multi_workspace: multi_workspace,
                        _project: project,
                        _document: document,
                    });
                });
                Result::<_, anyhow::Error>::Ok(window)
            })?;

            if let Some(path) = restored_workspace_path {
                let display_path = path.display().to_string();
                let open = window.update(cx, |multi_workspace, window, cx| {
                    multi_workspace.open_project(vec![path], OpenMode::Activate, window, cx)
                })?;
                open.await?;
                gpui_ohos::log_message(
                    gpui_ohos::LogLevel::Info,
                    format!("restored HarmonyOS workspace at {display_path}"),
                );
            }

            gpui_ohos::log_message(
                gpui_ohos::LogLevel::Info,
                format!(
                    "opened persistent sandbox workspace at {}",
                    document_path.display()
                ),
            );
            Ok(())
        }
        .await;

        if let Err(error) = result {
            gpui_ohos::log_message(
                gpui_ohos::LogLevel::Error,
                format!("failed to open the HarmonyOS sandbox workspace: {error:#}"),
            );
        }
    })
    .detach();

    Ok(())
}

fn initialize_keymaps(fs: Arc<dyn Fs>, cx: &mut App) {
    enum KeymapUpdate {
        File(String),
        Settings,
    }

    if let Err(error) = reload_keymaps(Vec::new(), cx) {
        gpui_ohos::log_message(
            gpui_ohos::LogLevel::Error,
            format!("failed to load the default HarmonyOS keymap: {error:#}"),
        );
    }

    let (keymap_file_rx, keymap_file_watcher) =
        settings::watch_config_file(cx.background_executor(), fs, paths::keymap_file().clone());
    let (settings_tx, settings_rx) = futures::channel::mpsc::unbounded();
    cx.observe_global::<settings::SettingsStore>(move |_cx| {
        let _ = settings_tx.unbounded_send(());
    })
    .detach();

    let mut updates = futures::stream::select(
        keymap_file_rx.map(KeymapUpdate::File),
        settings_rx.map(|()| KeymapUpdate::Settings),
    );
    cx.spawn(async move |cx| {
        let _keymap_file_watcher = keymap_file_watcher;
        let mut user_bindings = Vec::new();
        while let Some(update) = updates.next().await {
            cx.update(|cx| {
                if let KeymapUpdate::File(content) = update {
                    match settings::KeymapFile::load(&content, cx) {
                        settings::KeymapFileLoadResult::Success { key_bindings } => {
                            user_bindings = key_bindings;
                        }
                        settings::KeymapFileLoadResult::SomeFailedToLoad {
                            key_bindings,
                            error_message,
                        } => {
                            if !key_bindings.is_empty() {
                                user_bindings = key_bindings;
                            }
                            gpui_ohos::log_message(
                                gpui_ohos::LogLevel::Error,
                                format!(
                                    "some HarmonyOS key bindings failed to load: {error_message:?}"
                                ),
                            );
                        }
                        settings::KeymapFileLoadResult::JsonParseFailure { error } => {
                            gpui_ohos::log_message(
                                gpui_ohos::LogLevel::Error,
                                format!("failed to parse the HarmonyOS keymap: {error:#}"),
                            );
                            return;
                        }
                    }
                }

                if let Err(error) = reload_keymaps(user_bindings.clone(), cx) {
                    gpui_ohos::log_message(
                        gpui_ohos::LogLevel::Error,
                        format!("failed to reload HarmonyOS key bindings: {error:#}"),
                    );
                }
            });
        }
    })
    .detach();
}

fn reload_keymaps(mut user_bindings: Vec<gpui::KeyBinding>, cx: &mut App) -> Result<()> {
    use settings::{KeybindSource, Settings as _};

    cx.clear_key_bindings();
    let base_keymap = *settings::BaseKeymap::get_global(cx);
    if base_keymap != settings::BaseKeymap::None {
        cx.bind_keys(load_builtin_keymap(
            settings::DEFAULT_KEYMAP_PATH,
            KeybindSource::Default,
            cx,
        )?);
        if let Some(asset_path) = base_keymap.asset_path() {
            cx.bind_keys(load_builtin_keymap(asset_path, KeybindSource::Base, cx)?);
        }
        if vim_mode_setting::VimModeSetting::get_global(cx).0
            || vim_mode_setting::HelixModeSetting::get_global(cx).0
        {
            cx.bind_keys(load_builtin_keymap(
                settings::VIM_KEYMAP_PATH,
                KeybindSource::Vim,
                cx,
            )?);
        }
        cx.bind_keys(load_builtin_keymap(
            settings::SPECIFIC_OVERRIDES_KEYMAP_PATH,
            KeybindSource::Default,
            cx,
        )?);
    }

    for binding in &mut user_bindings {
        binding.set_meta(KeybindSource::User.meta());
    }
    cx.bind_keys(user_bindings);
    cx.set_menus(ohos_app_menus());
    keymap_editor::KeymapEventChannel::trigger_keymap_changed(cx);
    Ok(())
}

fn load_builtin_keymap(
    asset_path: &str,
    source: settings::KeybindSource,
    cx: &App,
) -> Result<Vec<gpui::KeyBinding>> {
    // The shared desktop keymaps contain bindings for optional subsystems. On
    // HarmonyOS those subsystems become available incrementally, so retain
    // every binding whose real action is registered instead of discarding the
    // entire production keymap when one optional action is absent.
    let mut bindings = settings::KeymapFile::load_asset_allow_partial_failure(asset_path, cx)?;
    for binding in &mut bindings {
        binding.set_meta(source.meta());
    }
    Ok(bindings)
}

fn initialize_workspaces(cx: &mut App) {
    cx.observe_new(
        |workspace: &mut Workspace, window, cx: &mut Context<Workspace>| {
            workspace.register_action(open_project_tasks_file);

            let Some(window) = window else {
                return;
            };
            let panel_task = initialize_panels(window, cx);
            workspace.set_panels_task(panel_task);
        },
    )
    .detach();
}

fn open_project_tasks_file(
    workspace: &mut Workspace,
    _: &zed_actions::OpenProjectTasks,
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) {
    let Some(open) = open_local_workspace_file(
        workspace,
        paths::local_tasks_file_relative_path(),
        settings::initial_tasks_content(),
        window,
        cx,
    ) else {
        gpui_ohos::log_message(
            gpui_ohos::LogLevel::Warning,
            "cannot open .zed/tasks.json because this workspace has no visible folder",
        );
        return;
    };
    open.detach_and_log_err(cx);
}

fn open_local_workspace_file(
    workspace: &mut Workspace,
    relative_path: &'static RelPath,
    initial_contents: Cow<'static, str>,
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) -> Option<gpui::Task<Result<Entity<Editor>>>> {
    let project = workspace.project().clone();
    let worktree = project
        .read(cx)
        .visible_worktrees(cx)
        .find_map(|worktree| worktree.read(cx).root_entry()?.is_dir().then_some(worktree));
    let worktree = worktree?;
    let worktree_id = worktree.read(cx).id();

    Some(cx.spawn_in(window, async move |workspace, cx| {
        let absolute_path = worktree.read_with(cx, |worktree, _| {
            worktree.abs_path().join(relative_path.as_std_path())
        });
        let fs = project.read_with(cx, |project, _| project.fs().clone());
        let file_exists = fs
            .metadata(&absolute_path)
            .await
            .ok()
            .flatten()
            .is_some_and(|metadata| !metadata.is_dir && !metadata.is_fifo);

        if !file_exists {
            if let Some(parent) = relative_path.parent()
                && worktree.read_with(cx, |worktree, _| worktree.entry_for_path(parent).is_none())
            {
                project
                    .update(cx, |project, cx| {
                        project.create_entry((worktree_id, parent), true, cx)
                    })
                    .await
                    .context("worktree disappeared while creating .zed")?;
            }
            if worktree.read_with(cx, |worktree, _| {
                worktree.entry_for_path(relative_path).is_none()
            }) {
                project
                    .update(cx, |project, cx| {
                        project.create_entry((worktree_id, relative_path), false, cx)
                    })
                    .await
                    .context("worktree disappeared while creating tasks.json")?;
            }
        }

        let editor = workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_path((worktree_id, relative_path), None, true, window, cx)
            })?
            .await?
            .downcast::<Editor>()
            .context("expected tasks.json to open in an editor")?;
        editor.update(cx, |editor, cx| {
            if let Some(buffer) = editor.buffer().read(cx).as_singleton()
                && buffer.read(cx).is_empty()
            {
                buffer.update(cx, |buffer, cx| {
                    buffer.edit([(0..0, initial_contents)], None, cx)
                });
            }
        });
        Ok(editor)
    }))
}

fn open_folder(cx: &mut App) {
    gpui_ohos::log_message(
        gpui_ohos::LogLevel::Info,
        "dispatching the HarmonyOS Open Folder action",
    );
    let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<MultiWorkspace>())
    else {
        gpui_ohos::log_message(
            gpui_ohos::LogLevel::Error,
            "Open Folder could not find the HarmonyOS workspace window",
        );
        return;
    };
    let prompt = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: None,
    });

    cx.spawn(async move |cx| {
        let paths = match prompt.await {
            Ok(Ok(Some(paths))) => paths,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(error)) => {
                gpui_ohos::log_message(
                    gpui_ohos::LogLevel::Error,
                    format!("HarmonyOS Open Folder failed: {error:#}"),
                );
                return Ok(());
            }
            Err(_) => {
                gpui_ohos::log_message(
                    gpui_ohos::LogLevel::Error,
                    "HarmonyOS Open Folder channel closed before completion",
                );
                return Ok(());
            }
        };

        let selected_paths = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let persistent_uri = paths
            .first()
            .and_then(|path| gpui_ohos::persistent_uri_for_path(path));
        let open = window.update(cx, |multi_workspace, window, cx| {
            multi_workspace.open_project(paths, OpenMode::Activate, window, cx)
        })?;
        open.await?;
        if let Some(uri) = persistent_uri {
            let key_value_store = cx.update(|cx| KeyValueStore::global(cx));
            cx.background_spawn(async move {
                key_value_store
                    .write_kvp(LAST_WORKSPACE_URI_KEY.to_owned(), uri)
                    .await
            })
            .await
            .context("remembering the last HarmonyOS workspace")?;
        }
        gpui_ohos::log_message(
            gpui_ohos::LogLevel::Info,
            format!("opened HarmonyOS workspace at {selected_paths}"),
        );
        Ok::<(), anyhow::Error>(())
    })
    .detach_and_log_err(cx);
}

/// Zed's application menu rendered by the production cross-platform title bar.
/// HarmonyOS has no process-wide desktop menu, so the platform exposes these
/// entries through the in-window hamburger/menu bar used on Linux and Windows.
fn ohos_app_menus() -> Vec<Menu> {
    vec![
        Menu::new("Zed").items([
            MenuItem::action("Sign In", client::SignIn),
            MenuItem::action("Sign Out", client::SignOut),
            MenuItem::separator(),
            // Keep the primary file actions in the touch-accessible root menu
            // as well as the full File menu. HarmonyOS pointer/touch input has
            // no reliable hover gesture for switching an already-open desktop
            // menu bar between top-level menus.
            MenuItem::action("Open Folder…", OpenFolder),
            MenuItem::action("Open File…", workspace::OpenFiles),
            MenuItem::action("New File", workspace::NewFile),
            MenuItem::action("Save", workspace::Save { save_intent: None }),
            MenuItem::action("Save As…", workspace::SaveAs),
            MenuItem::action("Save All", workspace::SaveAll { save_intent: None }),
            MenuItem::separator(),
            MenuItem::action("Settings", zed_actions::OpenSettings),
            MenuItem::action("Keymap", zed_actions::OpenKeymap),
            MenuItem::action(
                "Select Theme…",
                zed_actions::theme_selector::Toggle::default(),
            ),
            MenuItem::action("Extensions", zed_actions::Extensions::default()),
            MenuItem::separator(),
            MenuItem::action("Command Palette…", zed_actions::command_palette::Toggle),
            MenuItem::separator(),
            MenuItem::action("Quit Zed", zed_actions::Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New File", workspace::NewFile),
            MenuItem::action("Open File…", workspace::OpenFiles),
            MenuItem::action("Open Folder…", OpenFolder),
            MenuItem::action("Open Recent…", zed_actions::OpenRecent::default()),
            MenuItem::action("Add Folder to Project…", workspace::AddFolderToProject),
            MenuItem::separator(),
            MenuItem::action("Save", workspace::Save { save_intent: None }),
            MenuItem::action("Save As…", workspace::SaveAs),
            MenuItem::action("Save All", workspace::SaveAll { save_intent: None }),
            MenuItem::separator(),
            MenuItem::action(
                "Close Editor",
                workspace::CloseActiveItem {
                    save_intent: None,
                    close_pinned: true,
                },
            ),
            MenuItem::action("Close Project", workspace::CloseProject),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", editor::actions::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", editor::actions::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", editor::actions::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", editor::actions::Copy, OsAction::Copy),
            MenuItem::action("Copy and Trim", editor::actions::CopyAndTrim),
            MenuItem::os_action("Paste", editor::actions::Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::action("Find", search::buffer_search::Deploy::find()),
            MenuItem::action("Find in Project", workspace::DeploySearch::default()),
            MenuItem::separator(),
            MenuItem::action(
                "Toggle Line Comment",
                editor::actions::ToggleComments::default(),
            ),
            MenuItem::os_action(
                "Select All",
                editor::actions::SelectAll,
                OsAction::SelectAll,
            ),
        ]),
        Menu::new("Selection").items([
            MenuItem::os_action(
                "Select All",
                editor::actions::SelectAll,
                OsAction::SelectAll,
            ),
            MenuItem::action("Expand Selection", editor::actions::SelectLargerSyntaxNode),
            MenuItem::action("Shrink Selection", editor::actions::SelectSmallerSyntaxNode),
            MenuItem::action("Select Next Sibling", editor::actions::SelectNextSyntaxNode),
            MenuItem::action(
                "Select Previous Sibling",
                editor::actions::SelectPreviousSyntaxNode,
            ),
            MenuItem::separator(),
            MenuItem::action(
                "Add Cursor Above",
                editor::actions::AddSelectionAbove {
                    skip_soft_wrap: true,
                },
            ),
            MenuItem::action(
                "Add Cursor Below",
                editor::actions::AddSelectionBelow {
                    skip_soft_wrap: true,
                },
            ),
            MenuItem::action(
                "Select Next Occurrence",
                editor::actions::SelectNext {
                    replace_newest: false,
                },
            ),
            MenuItem::action(
                "Select Previous Occurrence",
                editor::actions::SelectPrevious {
                    replace_newest: false,
                },
            ),
            MenuItem::action("Select All Occurrences", editor::actions::SelectAllMatches),
            MenuItem::separator(),
            MenuItem::action("Move Line Up", editor::actions::MoveLineUp),
            MenuItem::action("Move Line Down", editor::actions::MoveLineDown),
            MenuItem::action("Duplicate Selection", editor::actions::DuplicateLineDown),
        ]),
        Menu::new("View").items([
            MenuItem::action(
                "Zoom In",
                zed_actions::IncreaseBufferFontSize { persist: false },
            ),
            MenuItem::action(
                "Zoom Out",
                zed_actions::DecreaseBufferFontSize { persist: false },
            ),
            MenuItem::action(
                "Reset Zoom",
                zed_actions::ResetBufferFontSize { persist: false },
            ),
            MenuItem::action(
                "Reset All Zoom",
                zed_actions::ResetAllZoom { persist: false },
            ),
            MenuItem::separator(),
            MenuItem::action("Toggle Left Dock", workspace::ToggleLeftDock),
            MenuItem::action("Toggle Right Dock", workspace::ToggleRightDock),
            MenuItem::action("Toggle Bottom Dock", workspace::ToggleBottomDock),
            MenuItem::action("Toggle All Docks", workspace::ToggleAllDocks),
            MenuItem::submenu(Menu::new("Editor Layout").items([
                MenuItem::action("Split Up", workspace::SplitUp::default()),
                MenuItem::action("Split Down", workspace::SplitDown::default()),
                MenuItem::action("Split Left", workspace::SplitLeft::default()),
                MenuItem::action("Split Right", workspace::SplitRight::default()),
            ])),
            MenuItem::separator(),
            MenuItem::action("Project Panel", zed_actions::project_panel::ToggleFocus),
            MenuItem::action("Outline Panel", outline_panel::ToggleFocus),
            MenuItem::action("Terminal Panel", terminal_view::terminal_panel::Toggle),
            MenuItem::action("Agent Panel", zed_actions::assistant::ToggleFocus),
            MenuItem::action("Git Panel", zed_actions::git_panel::ToggleFocus),
            MenuItem::separator(),
            MenuItem::action("Diagnostics", diagnostics::Deploy),
        ]),
        Menu::new("Go").items([
            MenuItem::action("Back", workspace::GoBack),
            MenuItem::action("Forward", workspace::GoForward),
            MenuItem::separator(),
            MenuItem::action("Command Palette…", zed_actions::command_palette::Toggle),
            MenuItem::action("Go to File…", workspace::ToggleFileFinder::default()),
            MenuItem::action("Go to Symbol…", zed_actions::outline::ToggleOutline),
            MenuItem::action("Go to Line/Column…", editor::actions::ToggleGoToLine),
            MenuItem::separator(),
            MenuItem::action(
                "Go to Definition",
                editor::actions::GoToDefinition::default(),
            ),
            MenuItem::action("Go to Declaration", editor::actions::GoToDeclaration),
            MenuItem::action("Go to Type Definition", editor::actions::GoToTypeDefinition),
            MenuItem::action(
                "Find All References",
                editor::actions::FindAllReferences::default(),
            ),
            MenuItem::separator(),
            MenuItem::action("Next Problem", editor::actions::GoToDiagnostic::default()),
            MenuItem::action(
                "Previous Problem",
                editor::actions::GoToPreviousDiagnostic::default(),
            ),
        ]),
        Menu::new("Run").items([
            MenuItem::action(
                "Spawn Task",
                zed_actions::Spawn::ViaModal {
                    reveal_target: None,
                },
            ),
            MenuItem::separator(),
            MenuItem::action("Edit tasks.json…", zed_actions::OpenProjectTasks),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Show Welcome", onboarding::ShowWelcome),
            MenuItem::separator(),
            MenuItem::action("File Bug Report…", zed_actions::feedback::FileBugReport),
            MenuItem::action("Request Feature…", zed_actions::feedback::RequestFeature),
            MenuItem::action("Email Zed…", zed_actions::feedback::EmailZed),
            MenuItem::separator(),
            MenuItem::action("Zed Repository", feedback::OpenZedRepo),
        ]),
    ]
}

fn initialize_panels(
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) -> gpui::Task<Result<()>> {
    cx.spawn_in(window, async move |workspace, cx| {
        let project_panel = ProjectPanel::load(workspace.clone(), cx.clone());
        let outline_panel = OutlinePanel::load(workspace.clone(), cx.clone());
        let terminal_panel = TerminalPanel::load(workspace.clone(), cx.clone());
        let git_panel = GitPanel::load(workspace.clone(), cx.clone());
        let agent_panel = AgentPanel::load(workspace.clone(), cx.clone());

        futures::join!(
            add_panel_when_ready("project", project_panel, workspace.clone(), cx.clone()),
            add_panel_when_ready("outline", outline_panel, workspace.clone(), cx.clone()),
            add_panel_when_ready("terminal", terminal_panel, workspace.clone(), cx.clone()),
            add_panel_when_ready("git", git_panel, workspace.clone(), cx.clone()),
            add_panel_when_ready("agent", agent_panel, workspace.clone(), cx.clone()),
        );

        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_panel::<AgentPanel>(window, cx);
            workspace.open_panel::<ProjectPanel>(window, cx);
            workspace.open_panel::<TerminalPanel>(window, cx);
        })?;
        Ok(())
    })
}

async fn add_panel_when_ready<P>(
    name: &'static str,
    panel_task: impl Future<Output = Result<Entity<P>>> + 'static,
    workspace: WeakEntity<Workspace>,
    mut cx: AsyncWindowContext,
) where
    P: workspace::Panel + 'static,
{
    match panel_task.await {
        Ok(panel) => {
            if let Err(error) = workspace.update_in(&mut cx, |workspace, window, cx| {
                workspace.add_panel(panel, window, cx);
            }) {
                gpui_ohos::log_message(
                    gpui_ohos::LogLevel::Error,
                    format!("failed to attach the HarmonyOS {name} panel: {error:#}"),
                );
            }
        }
        Err(error) => gpui_ohos::log_message(
            gpui_ohos::LogLevel::Error,
            format!("failed to load the HarmonyOS {name} panel: {error:#}"),
        ),
    }
}

fn load_embedded_fonts(cx: &App) -> Result<()> {
    let mut embedded_fonts = Vec::new();
    for font_path in cx.asset_source().list("fonts")? {
        if !font_path.ends_with(".ttf") {
            continue;
        }
        let bytes = cx
            .asset_source()
            .load(&font_path)?
            .with_context(|| format!("embedded font {font_path} is missing"))?;
        embedded_fonts.push(bytes);
    }

    cx.text_system()
        .add_fonts(embedded_fonts)
        .context("loading Zed's embedded fonts")
}
