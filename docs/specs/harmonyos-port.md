# Spec: Porting Zed to HarmonyOS NEXT / OpenHarmony

**Status:** P1 renderer-parity and P4 developer-tooling milestones in progress — native workspace, full Native Drawing styles, stable resize/input, external folders, a separate native Settings window, shell, Toybox, Git, and idle process scheduling verified; the rebuilt Node/LSP package is installed and awaiting on-device language-server validation

**Author:** wiedymi

**Branch:** `harmonyos-port`

**Started:** 2026-08

## 1. Goal

The first milestone is a working, native Zed editor inside the HarmonyOS NEXT PC emulator. It must boot, render its real GPUI interface, accept keyboard and pointer input, and edit and save a file in the application sandbox.

Git, language servers, terminals, tasks, extensions, remote development, and general external-directory support did not block this milestone. Their production UI is enabled where it can run without a child process; executable-backed behavior is added through HNP after the editor shell is reliable in the simulator.

### First usable milestone (achieved)

The milestone is complete when all of the following work in the PC emulator:

- A debug-signed ArkTS HAP loads Zed's Rust library.
- `gpui_ohos` creates and owns a GPUI window hosted by ArkUI XComponent.
- A new Native Drawing renderer draws the real Zed UI.
- The application handles resize, foreground/background, and XComponent surface recreation.
- Hardware keyboard, mouse/pointer, focus, clipboard, and baseline IME composition work.
- Zed opens, edits, saves, closes, and reopens a file in its private sandbox.

This milestone does not require HNP or any external executable.

### Current implementation snapshot (2026-08-10)

The port is past the scaffold stage. The installed HAP currently boots the real
`workspace::MultiWorkspace` and `workspace::Workspace` shell, opens a real
`editor::Editor` over a local `project::Project`, attaches the production Agent,
Project, Outline, Git, and Terminal panels, renders GPUI scenes through Native
Drawing, accepts native XComponent input, and automatically saves the sandbox
document. Agent and Project panels are opened by default so the simulator shows
the product shell rather than an editor-only demo.

| Capability | Current state |
|---|---|
| Rust HAP bootstrap | Working in the API 24 x86_64 PC emulator |
| Real GPUI workspace | Working; tabbed editor, status/dock chrome, Agent, Project, Outline, Git, and Terminal panels use production Zed entities. Every newly created `Workspace` runs the OHOS panel initializer, including workspaces created by Open Folder, Open Recent, and clone flows. A live `test` to `zed` folder switch retained the Agent, Project, and Terminal docks on-device. Dock sizes are clamped only after usable workspace bounds exist, and a resized Project panel reopens at its previous width. |
| Native Drawing | Native quads, per-corner rounding, content clips, solid/dashed borders, gradients, slash/checkerboard patterns, drop/inset shadows, tessellated paths, straight/wavy underlines, sprite transforms, and monochrome/subpixel/polychrome atlas sprites are implemented, cross-compiled, and verified on-device against workspace panels, menus, Settings, notifications, and controls. Native bitmaps are cached per atlas tile revision and color transform instead of being recreated for every sprite draw. Simplified/Traditional Chinese and colored HMOS emoji fallback are verified in a real editor buffer. |
| Frame scheduling | Demand-driven ArkUI `UIContext.postFrameCallback`, requested from Rust through an N-API thread-safe function. ArkTS permits only one outstanding frame callback, and Rust applies only the newest pending XComponent resize at that frame boundary. Release builds compile out per-frame, resize, and input debug logging. After removing an OHOS pipe-read readiness spin, the foreground and GPUI worker threads remain asleep while the app is idle. |
| Scale and resize | Dynamic ArkUI density synchronization and physical-to-logical geometry are verified across maximize, restore, and live resize. A 24-frame baseline capture and a 12-frame capture after resize coalescing contained no blank/black frame; physical and logical bounds changed together while scale remained exactly `1.9`. |
| Input | Hardware keys, mouse buttons/motion, touchscreen taps, touch-to-caret, focus, native IME attachment, and ArkUI mouse-wheel axis scrolling work. `onKeyPreIme` forwards hardware keys before the system IME consumes printable key-down events, while `onKeyEvent` remains the fallback. Typed text comes from ArkUI `keyText`/Unicode rather than a US-layout table; physical Shift+9 produces `(` and physical Space separates terminal arguments on-device. Mouse-originated synthetic touch events are identified with `OH_NativeXComponent_GetTouchEventSourceType` and ignored so one desktop click cannot open and immediately close a menu. |
| Persistence | Real sandbox file save and reopen working through `BufferStore` |
| Clipboard | HarmonyOS system pasteboard read/write working; on-device round trip verified and pasteboard permission handled by `EntryAbility` |
| Credentials | HarmonyOS Asset Store read/write/delete working with persistent, encrypted records; restart survival and deletion verified on-device |
| Agent/network UI | Production Agent panel, language-model/provider initialization, ACP tools, web-search providers, prompt store, and real Reqwest HTTP client enabled; provider use still requires normal account/API configuration |
| App menu/system bridges | In-window Zed/File/Edit/Selection/View/Go/Run/Help menus use production actions for sign-in/out, settings, folder/file opening, save, search, selection, comments, diagnostics, tasks, panels, themes, extensions, command palette, onboarding, and feedback. The menu works on-device. The production Settings view opens in a separate non-modal native `Zed — Settings` subwindow with server decorations, its own XComponent, GPUI window state, surface, atlas, input routing, resize callbacks, and close lifecycle. Settings rendering and keyboard input were verified while the main workspace remained alive behind it. User settings and keymap files are watched and reloaded. URL opening and HarmonyOS document select/save pickers are bridged through ArkTS. |
| External workspaces | `DocumentViewPicker` selection, persistent URI permission, native FileShare URI-to-path resolution, restart restoration, public workspace access, and live folder-to-folder switching with complete workspace docks work on-device. Clone's “Open repo in new project” choice creates a distinct `Workspace` in the existing OHOS `MultiWorkspace` until general workspace-window support is available. |
| Process-backed UI | A private `zedtools` HNP supplies Dash, Toybox 0.8.11, and Git 2.51.0. Terminal pipes, Git init/status/config/add/commit/log, HTTPS `ls-remote`, and shallow clone are verified on-device. PTY creation is denied by the HAP SELinux domain, so terminals intentionally use pipes. OHOS child pipes use blocking worker-backed I/O because both epoll and POSIX poll repeatedly report false readiness for these descriptors; terminal output still flows and the workers sleep at idle. |
| Language servers | Built-in grammars and language adapters are registered. The rebuilt Node.js 22.23.2/npm HNP with the Tailwind Unicode-regexp and ESLint `openharmony` packaging adaptations is built, packaged, and installed; its Node executable is a validated x86_64 OHOS PIE linked only to OHOS libc/libc++. vtsls launched with the earlier package. Tailwind, ESLint, and complete diagnostics are not recorded as fixed until this newly installed package is exercised on-device. |
| Platform work still pending | General workspace/tool multi-window support beyond the main-plus-Settings pair, exact non-sRGB gradient interpolation, active-scene text/path profiling and native-object lifetime instrumentation, lifecycle stress coverage, reveal/open-with, downloaded native executable policy, SSH, and physical-device validation |

Observed emulator contract:

```text
ABI:             x86_64
API:             24
Product:         emulator 6.1.0.125(SP12DEVC00E47R1P3)
OpenHarmony:     OpenHarmony-6.1.1.125
Kernel:          Linux 5.10.210 x86_64
Security patch:  2026/05/01
Display density: 1.9
```

## 2. Decisions

The following decisions are authoritative for the initial port:

1. **Rendering uses HarmonyOS Native Drawing.** `gpui_ohos` translates GPUI `Scene` primitives into `OH_Drawing_*` canvas operations.
2. **The initial renderer does not use `gpui_wgpu`.** It does not initialize wgpu, EGL, OpenGL ES, or Vulkan directly.
3. **HarmonyOS chooses the underlying GPU backend.** The renderer uses `OH_Drawing_GpuContextCreate` and `OH_Drawing_SurfaceCreateOnScreen` with the XComponent `OHNativeWindow`.
4. **Simulator-first is the critical path.** Product tooling is gated out until the real Zed UI works in the PC emulator.
5. **External directories use the system picker and persistent FileShare authorization.** The app stores the authorized URI, reactivates it after restart, and resolves it through the native FileShare API. The private sandbox remains the safe fallback.
6. **Process-backed features use a private HNP.** Packaged, pinned executables are invoked through normal Zed process integrations; arbitrary downloaded native executables are not assumed executable.
7. **Platform detection uses `target_env = "ohos"`.** OHOS must be excluded from desktop-Linux dependency and source gates.
8. **The first package is a locally installed, debug-signed HAP.** Store distribution, OpenHarmony dual packaging, and physical-device certification are later work.
9. **Production UI is enabled before its executable backends.** Agent, Project, Outline, Git, and Terminal panels remain real Zed components. Missing HNP tools produce explicit unavailable/error states rather than substitute implementations.
10. **Credentials use HarmonyOS Asset Store.** Provider secrets are stored as persistent, encrypted Asset Store records. The alias is a SHA-256-derived identifier and the username/password payload is versioned and limited to the platform's 1024-byte secret bound.
11. **No compatibility shims for missing developer tools.** Git, terminals, language servers, tasks, and SSH become functional only through tested HNP executables and the normal Zed integrations.
12. **The initial HNP toolchain is Dash + Toybox + Git + Node/npm.** Dash provides POSIX shell behavior, Toybox supplies standard utilities, patched upstream Git supplies the normal Zed Git backend, and upstream Node 22 supplies JavaScript language-server execution and npm installation.
13. **PTY denial is a platform constraint, not emulated away.** The current HAP SELinux domain cannot open `/dev/ptmx`; terminal sessions therefore use real child processes over stdin/stdout/stderr pipes. Interactive PTY semantics remain a physical-image/policy gate.
14. **Public workspace Git metadata may live privately.** When HarmonyOS prevents atomic `.git` directory operations in an authorized public directory, Zed uses deterministic private metadata and a standard `.git` gitfile pointer while retaining the public worktree.
15. **Settings uses a dedicated native auxiliary window.** `gpui_ohos` currently supports the main workspace plus one non-modal Settings subwindow. Each maps its actual ArkUI XComponent ID to independent GPUI window state, Native Drawing surface, atlas, callbacks, focus, resize, and input routing. ArkTS prewarms the hidden Settings XComponent after the main workspace's initial paint because a never-shown subwindow does not instantiate its XComponent; it then minimizes the subwindow until GPUI requests it. HarmonyOS owns the Settings titlebar, move, maximize, resize, and close controls through server decorations. This is the final Settings architecture, but not yet arbitrary general multi-window support.
16. **OHOS child-process pipes use blocking worker-backed I/O.** The emulator's pipe descriptors repeatedly wake both epoll and POSIX poll before a nonblocking read can make progress, causing `async-io` workers to spin. The OHOS `async-process` adaptation wraps child stdin/stdout/stderr with `blocking::Unblock`, as its Windows backend does. Async sockets and the Unix child reaper remain unchanged.
17. **Workspace initialization follows entity creation, not the first app window.** The OHOS bootstrap observes every production `Workspace` and attaches its real panels and workspace actions there. Folder, recent-project, restore, and clone flows must never depend on one-off initialization of the sandbox workspace. Although `gpui_ohos` now exposes a dedicated Settings subwindow, it does not yet expose arbitrary workspace windows; clone's “Open repo in new project” path therefore uses a separate production `Workspace` inside the existing `MultiWorkspace` until general workspace-window support is implemented.

### Not a current decision

The port does not choose Vulkan versus OpenGL ES. Native Drawing owns that implementation detail. A custom wgpu, Vulkan, or GLES renderer is considered only if profiling later proves Native Drawing inadequate.

## 3. Target and SDK contract

| Dimension | Initial decision |
|---|---|
| Runtime | HarmonyOS NEXT PC emulator |
| SDK baseline | HarmonyOS/OpenHarmony API 24 for initial development |
| Primary Rust target | `x86_64-unknown-linux-ohos`, subject to emulator ABI confirmation |
| Secondary Rust target | `aarch64-unknown-linux-ohos`, compile-only until physical-device work |
| Packaging | ArkTS HAP containing a Rust `cdylib` |
| Installation | DevEco Studio / `hvigor` plus `hdc` |
| CI | Compile and package only until a reliable emulator runner exists |

P0 must record the emulator ABI, API level, and image version using `hdc`. If the PC image does not expose x86_64 to native applications, the primary target must be corrected before platform work proceeds.

### Rust target identity

Rust exposes OHOS as:

```text
target_os = "linux"
target_env = "ohos"
target_family = "unix"
```

HarmonyOS-specific source gates must use:

```rust
#[cfg(target_env = "ohos")]
```

Desktop Linux gates must exclude OHOS:

```rust
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
```

Cargo target tables use the equivalent form:

```toml
[target.'cfg(target_env = "ohos")'.dependencies]

[target.'cfg(all(target_os = "linux", not(target_env = "ohos")))'.dependencies]
```

`gpui_platform` currently selects `gpui_linux` for Linux targets. The OHOS branch must be selected first, and every reachable desktop-Linux dependency must be audited.

## 4. Architecture

```text
ArkTS HAP
  UIAbility and application lifecycle
  ArkUI XComponent
  system dialogs and permission prompts
  N-API bridge
          |
          v
zed_ohos (Rust cdylib)
  product bootstrap
  AppState/project/workspace bootstrap
  sandbox path bootstrap and persistent document
  N-API frame bridge
          |
          v
gpui_ohos
  Platform and PlatformWindow
  foreground/background executors
  ArkUI frame-callback scheduling
  XComponent lifecycle and OHNativeWindow ownership
  keyboard, pointer, IME, clipboard, paths
  Native Drawing renderer and sprite atlas
          |
          v
HarmonyOS Native Drawing
  OH_Drawing_GpuContext
  OH_Drawing_Surface created on OHNativeWindow
  OH_Drawing_Canvas
          |
          v
Selected Zed crates
  GPUI, workspace, editor, production panels, Agent UI/model providers,
  project/fs/worktree/buffer model, HTTP client
```

### Crate layout

- Add `crates/gpui_ohos` for the platform implementation and initial Native Drawing renderer.
- Add `crates/zed_ohos` for the Rust `cdylib`, product bootstrap, ArkTS entry module, and HAP packaging.
- Keep Native Drawing ownership and scene translation isolated in `gpui_ohos::drawing`, with atlas storage in `gpui_ohos::atlas`. Split them into another crate only if reuse or build boundaries later justify that cost.
- Keep the renderer-neutral cosmic-text implementation in `gpui_text`; OHOS must not depend on or initialize `gpui_wgpu` to obtain text shaping.
- Keep raw HarmonyOS pointers and unsafe FFI inside small ownership wrappers. Do not expose raw Native Drawing or XComponent handles to general GPUI or Zed code.

### Binding direction

- Use generated C bindings pinned to the chosen SDK/API level.
- Community `ohos-sys` or `ohos-native-bindings` crates may supply bindings, but the selected versions must be pinned and missing API 24 bindings must be generated reproducibly.
- Use `napi-ohos` only at the ArkTS/Rust module boundary if it fits the lifecycle requirements; Native Drawing and XComponent callbacks stay in the native platform layer.
- Link only the platform libraries actually used by the current milestone. The current runtime uses Native Drawing, NativeWindow, XComponent, ArkUI/N-API, IME, pasteboard, Asset Store, and logging. NativeVSync is not linked by the current design.
- Asset Store queries and deletes must use the same `IS_PERSISTENT` and `REQUIRE_ATTR_ENCRYPTED` attributes as writes. `REQUIRE_ATTR_ENCRYPTED` selects the credential-encrypted database in the platform service; omitting it silently queries a different database.

## 5. Native Drawing renderer

### Surface lifecycle

For every XComponent surface:

1. Receive the `OHNativeWindow` from the XComponent surface-created callback.
2. Set `SET_BUFFER_GEOMETRY` to the exact physical surface size and use scale-to-window behavior during interactive resize.
3. Create an `OH_Drawing_GpuContext` with `OH_Drawing_GpuContextCreate`.
4. Create an on-screen `OH_Drawing_Surface` with `OH_Drawing_SurfaceCreateOnScreen`.
5. Obtain its `OH_Drawing_Canvas`.
6. Translate the current GPUI `Scene` and draw it onto the canvas.
7. Flush the surface with `OH_Drawing_SurfaceFlush`.
8. On resize, store only the newest physical size and request an ArkUI frame. At the frame boundary, destroy the old on-screen Drawing wrapper before creating the replacement for the same `OHNativeWindow`. Overlapping wrappers cause the old wrapper to invalidate the replacement EGL surface when destroyed. Clear only a newly created window surface; while replacing a resize wrapper, retain the compositor's previous buffer until the next complete scene is flushed so interactive resize cannot expose a blank frame.
9. On surface destruction, remove all window-bound state and retain only the event handler needed for a later surface-created callback.

ArkTS recomputes `1 / UIContext.px2vp(1)` on every XComponent area change and
sends it through a dedicated N-API scale bridge. `gpui_ohos` updates the window
scale, logical bounds, input conversion, display bounds, and GPUI resize
callback together. Surface dimensions remain physical pixels; they are never
treated as logical ArkUI vp values.

The API 24 SDK exposes this device-selected GPU path from API 16. The deprecated `OH_Drawing_GpuContextCreateFromGL` path is not used.

Frame production is demand-driven through ArkUI. A Rust dispatcher wake calls an
N-API thread-safe function; ArkTS posts a `FrameCallback` with
`UIContext.postFrameCallback`; that callback re-enters Rust and drains the GPUI
foreground queue before rendering. GPUI invalidation schedules only the next
required frame. The ArkTS callback tracks whether it is already scheduled so
repeated Rust wakes cannot enqueue duplicate callbacks. XComponent resize events
replace one pending physical size, which Rust applies immediately before GPUI's
frame callback; no intermediate resize creates a redundant Drawing surface.
`OH_NativeVSync_RequestFrame` is intentionally not used because GPUI foreground
work and XComponent callbacks must remain coordinated with the ArkUI UI thread.

The renderer records a complete GPUI scene into an
`OH_Drawing_RecordCmdUtils` command list, replays it once to the on-screen
canvas, and then flushes. Emulator profiling measured command recording at
roughly 1–5 ms and replay plus `OH_Drawing_SurfaceFlush` at roughly 31–51 ms
during live resize, so the remaining simulator roughness is primarily the
Native Drawing/emulator presentation path rather than GPUI scene construction.
Release builds do not emit the profiling or per-input logs on these hot paths.

The foreground queue lock must be released before running a task. Opening a GPUI
window synchronously replays the already-created XComponent surface event, which
may re-enter queue draining. Holding the receiver lock across task execution
deadlocks the ArkUI main thread.

### Scene translation order

Renderer coverage is implemented and verified in this order:

1. Clear/background and solid quads.
2. Scissor/clip, rounded rectangles, transforms, and opacity.
3. Lines, underlines, paths, and vector icons.
4. Bitmap upload and the GPUI monochrome/polychrome sprite atlas.
5. Text glyph rendering and emoji/CJK fallback.
6. Shadows, images, embedded surfaces, and remaining blend modes.

The current native translation covers content-mask clipping, independent corner
radii, elliptical inner border corners, solid and dashed border rings, linear
gradients, slash/checkerboard patterns, Gaussian drop shadows, composited inset
shadows, tessellated GPUI paths, straight/wavy underlines, and rounded image
clips, non-solid path fills, and sprite transforms. It does not substitute CSS
or component-specific decoration. `PaintSurface` and the public surface element
carry an image payload only on macOS, so an OHOS `PrimitiveBatch::Surfaces` has
no renderable source today and is intentionally a no-op rather than a fake
surface implementation. On-device integration now verifies menus, the separate
native Settings window, notification cards, panel/control borders, rounded clips,
shadows, maximize/restore repaint, and atlas output. Remaining renderer gates are
exact non-sRGB gradient interpolation and focused performance/lifetime coverage.

GPUI's sprite atlas stores CPU-readable pixels, while Native Drawing requires an
`OH_Drawing_Bitmap` handle. The renderer therefore keeps the atlas pixels in
shared storage and caches the converted native bitmap by atlas tile, tile
revision, and color transform. Reusing an atlas slot increments its revision so
stale native content cannot be drawn, and periodic pruning releases cache entries
that are no longer reachable. This is the Native Drawing implementation of the
existing GPUI atlas contract, not a second rendering backend.

Renderer work is validated directly against the real editor plus focused examples
where a primitive is otherwise difficult to trigger. The real editor is the
authoritative integration gate.

### Text direction

Native Drawing is the raster renderer, but the first milestone does not also replace GPUI text shaping and layout.

- Reuse the existing cosmic-text/swash shaping behavior where practical.
- Extract or expose the minimum shared text-system code if depending on `gpui_wgpu` would otherwise initialize or pull in an unused renderer.
- Feed the shaped glyph/sprite output to the Native Drawing atlas.
- Obtain HarmonyOS system font paths explicitly and include a bundled fallback. Do not rely on desktop Linux fontconfig paths merely because `target_os` is `linux`.

The OHOS platform loads `/system/fonts` directly. The API 24 emulator supplies
HarmonyOS Sans SC/TC, Noto Sans/Serif CJK, and `HMOSColorEmoji*` faces there;
Simplified Chinese, Traditional Chinese, and colored emoji have been rendered
on-device in the production editor. The bundled Zed fonts remain available for
Zed's configured UI/editor families.

Native Drawing text layout can be evaluated later, independently of the renderer decision.

### Renderer acceptance

The renderer gate requires:

- correct scale factor, resizing, clipping, and alpha composition;
- editor text, tabs, project tree, menus, icons, cursors, selections, diagnostics, and scrollbars rendering correctly;
- surface destruction/recreation without stale native handles;
- stable idle and active frame scheduling;
- no per-frame growth in Native Drawing objects, bitmaps, or atlas allocations.

## 6. GPUI platform contract

The initial `gpui_ohos` support matrix is:

| Area | Initial requirement |
|---|---|
| Event loop | Run GPUI callbacks on the required UI thread and wake it from Rust tasks |
| Window | Host the main workspace and one auxiliary Settings GPUI window in distinct XComponents and report independent bounds/scale/focus |
| Lifecycle | Handle foreground/background, focus, resize, surface loss, and memory pressure |
| Rendering | Translate `Scene` to Native Drawing and flush from the ArkUI frame callback |
| Input | Hardware keyboard, modifiers, pointer, buttons, scroll, and basic touch |
| IME | Focus, preedit, commit, selection, caret rectangle, show/hide |
| Clipboard | Plain-text copy and paste |
| Paths | Application files, cache, temporary, preferences, and logs directories |
| Dialogs | Minimum error/prompt behavior needed to boot and edit |
| Credentials | Store provider secrets in persistent, encrypted HarmonyOS Asset Store records |
| Unsupported APIs | Return safe defaults or explicit unsupported errors; never panic |

Accessibility, arbitrary workspace/tool windows, drag and drop, notifications, screen capture, and voice remain unfinished. The main workspace plus a separate native Settings window are implemented; this limited two-window contract must not be described as general multi-window support. URL opening plus document open/folder/save pickers are implemented. Reveal/open-with and richer native prompts remain incomplete and must not be described as supported.

The Settings XComponent is registered and briefly prewarmed only after the main
workspace has painted. `gpui_ohos` routes native callbacks by the XComponent's
actual ArkUI ID, renders each requested window independently during a frame, and
removes the auxiliary GPUI state before asking ArkTS to hide the subwindow. A
native close request first runs GPUI's `should_close`/`close` callbacks so view
state is released rather than leaving an invisible Settings entity behind.

The ArkUI physical-pixel density is process/platform state, not transient window
state. It must be retained even when the frame bridge is configured before the
asynchronously opened GPUI window, then applied to window bounds, input
coordinates, caret bounds, and rendering. The API 24 PC emulator currently
reports 1.9.

Rust panics must never unwind through XComponent's C callbacks. Every native
surface and input callback is a containment boundary, and event handlers are
temporarily removed from `RefCell`-owned surface state before invocation to make
reentrant resize/window events safe.

## 7. Current product profile

The OHOS product uses production Zed components while keeping unsupported native
integrations explicit. A visible panel is not evidence that its executable
backend is available.

| Capability/dependency | Current disposition |
|---|---|
| `gpui_ohos` | Enabled |
| Native Drawing renderer | Enabled |
| `gpui_linux` and Linux desktop integrations | Disabled on `target_env = "ohos"` |
| `gpui_wgpu` renderer | Disabled for OHOS rendering |
| Core GPUI/editor/project model | Enabled |
| Core editor registries | File finder, diagnostics, buffer/project search, LSP locations, tasks, snippets, selectors, Vim, live settings/keymap reload, trusted-worktree policy, tab switcher, prompts, settings profiles, Markdown/image/CSV/SVG previews, onboarding, notifications, JSON schemas, feature flags, and syntax-theme propagation enabled |
| Built-in native grammars | Enabled where target-compatible |
| Sandbox filesystem | Enabled |
| External folder/file picker and persistence | Enabled through ArkTS `DocumentViewPicker`, persistent URI activation, and native FileShare path resolution |
| Agent/AI UI | Enabled with production Agent panel, provider/model registration, ACP tools, web-search providers, prompt store, real HTTP client, and Asset Store credentials |
| Project/Outline panels | Enabled |
| Git UI/backend | Production panel and real packaged Git 2.51.0 backend enabled; local operations and HTTPS transport verified |
| Language servers | Built-in grammars/adapters enabled; the replacement bundled-small-ICU Node 22/npm package and the remaining Tailwind/ESLint fixes are the active runtime gates |
| Terminal and tasks | Dash and Toybox are packaged; terminal child processes work over blocking worker-backed pipes without idle polling spin. PTY semantics are unavailable under the current SELinux domain. Tasks can use the same HNP commands but still require dedicated on-device coverage. |
| Remote SSH/workspaces | Disabled initially |
| Extension host / Wasmtime | Enabled with production extension store plus workspace-backed grammar/language/LSP proxies; representative extension installation still needs on-device coverage |
| LiveKit, voice, screen capture | Disabled |
| Auto-update and crash helper executables | Disabled |

UI visibility, dependency inclusion, command registration, and state initialization must be gated together. Hiding a command while retaining an unbuildable dependency is not sufficient.

## 8. Files and processes after the editor milestone

### External project directories

HarmonyOS PC/2-in-1 persistent FileShare authorization is now the selected workspace model. The port has proved picker selection, permission activation, URI-to-path conversion, recursive file access, edits, and restart restoration. The remaining storage checks are:

- atomic replace/save behavior;
- all `.git` directory operations beyond the private-metadata fallback;
- symlink semantics;
- `inotify` delivery for authorized directories.

The public Documents provider does not expose identical rename semantics to a desktop filesystem. Git therefore keeps repository metadata in private app storage when required and places a normal gitfile pointer in the authorized public worktree. The sandbox remains the fallback for operations that cannot be represented safely.

### HNP process foundation

HNP is the selected direction for packaged native tools. OpenHarmony documents HNP specifically for productivity applications that ship native packages such as Python, Node, and Java, and demonstrates launching their executables with `fork` and `execv`.

The private `zedtools` HNP and process probe are implemented. The following have been validated:

- launch through Rust `std::process::Command`;
- arguments, environment, working directory, stdin/stdout/stderr, and exit status;
- Dash/Toybox terminal execution and Git HTTPS network access inside the application sandbox;
- the current PTY denial: `/dev/ptmx` is blocked by SELinux, while pipe-backed children work;
- terminal input/output after routing OHOS child pipes through `blocking::Unblock`;
- idle worker behavior: the four primary `GPUI-Worker-*` threads and the child-pipe
  workers sleep at `0%` after startup instead of collectively consuming roughly
  `150–220%` CPU while a terminal is open.

The HNP build inputs pin upstream Node.js 22.23.2 and npm. The rebuilt Node uses
bundled small-ICU data rather than `--without-intl`, because current JavaScript
language servers use Unicode-property regular expressions. The build completed,
its OHOS ELF dependencies were validated, and the resulting Node/npm tree is in
the installed HAP. npm installs on OHOS use `--no-bin-links`; Zed launches concrete package
entrypoints instead of npm's Unix `.bin` symlinks. GitHub source archives are
extracted with validated, in-archive link materialization so package links work
without allowing an archive path to escape its destination. vtsls has been
observed running on-device with the earlier package. Tailwind and ESLint now
have replacement-runtime/package adaptations installed, but both remain explicit
acceptance gates until their processes and diagnostics are re-verified on-device.

Kill/restart cleanup, foreground/background resource policy, and PTY availability on other signed images remain explicit runtime gates.

Only packaged HNP executables are assumed. Downloading and executing arbitrary native binaries remains unsupported until separately proven and accepted for distribution.

### Tooling order

The tooling order is now:

1. [x] Package Dash and Toybox and enable the terminal process path.
2. [x] Package Git and reuse Zed's existing Git executable integration.
3. [x] Package upstream Node 22/npm and launch a real Node-based language server (vtsls); finish Tailwind/ESLint acceptance coverage.
4. [ ] Exercise tasks using the same process foundation.
5. [ ] Package OpenSSH for the existing SSH/SCP transport, or choose an in-process transport if packaging proves unsuitable.

Each feature remains independently gated. Failure of one tool must not block the editor or renderer.

## 9. Implementation roadmap

### P0 — Toolchain and HAP proof

- [x] Record emulator ABI, API level, image version, kernel, and density with `hdc`.
- [x] Configure reproducible Rust linker wrappers for `x86_64-unknown-linux-ohos`.
- [x] Build the Rust `cdylib` and load it from ArkTS through N-API.
- [x] Package, debug-sign, install, launch, and log from the HAP.
- [ ] Compile the same shell for aarch64.

**Gate:** Rust code runs inside the actual PC emulator HAP.

### P1 — `gpui_ohos` and Native Drawing

- [x] Add OHOS platform selection before desktop Linux.
- [x] Add `gpui_ohos` with working foreground/background executors, display, and window implementation.
- [x] Register XComponent surface, touch, mouse, axis/scroll, hover, key, focus, and blur callbacks with panic containment.
- [x] Create, draw, flush, resize, and destroy a Native Drawing on-screen surface.
- [ ] Complete Scene translation. Native clips, independent rounded corners, solid/dashed borders, gradients/patterns, drop/inset shadows, solid/gradient/pattern paths, straight/wavy underlines, rounded images, sprite transforms, and all sprite atlas classes are cross-built and visually verified on-device. Atlas bitmaps are cached by tile revision and color transform. Exact non-sRGB interpolation and active-scene performance/lifetime profiling remain. Non-macOS GPUI surfaces currently have no image payload and are an explicit no-op.
- [x] Use ArkUI `UIContext.postFrameCallback` for demand-driven frames.
- [x] Re-verify maximize, restore, and live resize after the incorrect-scaling report. The rebuilt HAP preserves scale `1.9` and correct logical layout as physical bounds change. ArkTS frame requests and Rust surface resize work are coalesced, and both the 24-frame baseline and 12-frame post-coalescing live-resize captures contain no blank/black frame.
- [ ] Complete suspend/resume and same-process surface-loss/recreation stress coverage.

**Gate:** A representative GPUI example renders text and all required primitive classes reliably through Native Drawing.

### P2 — First working Zed in the simulator

- [x] Add the focused OHOS product/dependency profile needed by the real workspace.
- [x] Boot production `MultiWorkspace`/`Workspace` entities with tab/dock/status chrome, a real editor item, command-palette registration, and the production project panel.
- [x] Complete hardware keyboard, pointer buttons/motion, ArkUI axis scrolling, touch-to-caret, focus, and baseline native IME attachment. Hardware keys are intercepted through `onKeyPreIme` before IME consumption, typed characters use ArkUI `keyText`/Unicode, physical Shift+9 produces `(`, physical Space separates terminal arguments, and mouse-wheel scrolling plus mouse-versus-touch source deduplication are verified on-device.
- [x] Replace the process-local clipboard fallback with HarmonyOS system pasteboard integration and verify an on-device round trip.
- [x] Open and edit a real sandbox file/workspace through `RealFs`, `WorktreeStore`, and `BufferStore`.
- [x] Automatically save edits, close/reinstall/restart, and reopen the persisted sandbox document without corruption.
- [x] Load the production Agent, Project, Outline, Git, and Terminal panels; open Agent and Project by default. Prevent transient zero workspace bounds from seeding a tiny dock size and preserve a user-resized Project panel width across close/reopen.
- [x] Initialize panels for every newly created `Workspace`, and verify that a live external-folder switch retains the Agent, Project, and Terminal docks. Route clone's separate-project choice through the existing `MultiWorkspace` while general workspace windows remain unavailable.
- [x] Initialize the production HTTP client, language-model/provider stack, prompt store, and secure Asset Store credential backend.
- [x] Initialize production file finder, diagnostics, search, LSP locations, tasks, snippets, selectors, Vim, tab switcher, prompts, settings profiles, previews, onboarding, notifications, Git-hosting providers, feature flags, ACP/web-search support, and extension language/LSP/theme registration.
- [ ] Make every unavailable process/system-integration command visibly disabled or absent.

**Gate:** Zed's real workspace UI works in the PC simulator and can persistently edit a sandboxed file. This is the first usable milestone.

### P3 — Native OS integration and process proof

- [x] Prove persistent external-directory picker access, native FileShare resolution, and restart reactivation.
- [x] Load HarmonyOS system fonts from `/system/fonts` and verify Simplified/Traditional Chinese plus colored HMOS emoji fallback in the production editor.
- [x] Implement persistent encrypted provider credentials with HarmonyOS Asset Store and validate write/read/restart/delete on-device.
- [x] Package and launch the HNP process probe.
- [x] Validate pipe-backed processes and packaged Git network access; record the current SELinux PTY denial.
- [x] Adapt OHOS child stdin/stdout/stderr to blocking worker-backed I/O and verify terminal output plus sleeping workers at idle.
- [x] Implement ArkTS bridges for document/folder selection, Save As, and URL opening.
- [ ] Implement reveal/open-with and finish lifecycle/background stress coverage.

**Gate:** One storage model and one HNP process model are proven without destabilizing the editor.

### P4 — Developer tooling

- [x] Enable the production Git and Terminal panel UI without fake backends.
- [x] Package Dash, Toybox 0.8.11, and Git 2.51.0; verify shell, local Git, HTTPS query, and shallow clone on-device.
- [ ] Finish representative language-server coverage. The rebuilt Node/npm package is installed and the earlier package launched vtsls; verify Tailwind and ESLint with the replacement runtime on-device.
- [x] Package a shell and enable terminal child processes over pipes.
- [ ] Verify tasks and complete diagnostics from a real Node-based language server using packaged Node 22/npm; the vtsls process launch is proven.
- [ ] Enable SSH/remote workspaces using packaged OpenSSH or a separately justified in-process transport.
- [ ] Reassess extension-host support.

**Gate:** Tooling is enabled feature-by-feature with explicit package, lifecycle, and failure handling.

### P5 — Productization

- [ ] Test physical aarch64 PC/2-in-1 hardware.
- [ ] Validate OpenHarmony runtime compatibility and packaging differences.
- [x] Add the main-plus-Settings multi-window path and open the production Settings view in a separate non-modal native window with server decorations and independent rendering/input.
- [ ] Generalize the platform beyond the dedicated Settings subwindow to arbitrary workspace/tool windows and persist their native bounds/state.
- [ ] Add accessibility, drag and drop, crash reporting, and update strategy.
- [x] Profile idle frame/process scheduling and atlas bitmap creation; cache native atlas bitmaps and eliminate the child-pipe readiness spin.
- [ ] Complete active-scene text/path profiling and Native Drawing object-lifetime instrumentation.
- [ ] Consider an alternate renderer only if measured Native Drawing limitations justify it.

## 10. Risks and gates

| Risk | Mitigation |
|---|---|
| OHOS accidentally compiles desktop Linux paths | Gate with `target_env = "ohos"` and audit the resolved Cargo graph |
| Native Drawing does not map cleanly to a GPUI primitive | Build renderer examples in primitive order before booting full Zed |
| Native handles outlive the XComponent surface | Centralize ownership, destroy the old on-screen wrapper before rebinding the same NativeWindow, and recreate all window-dependent resources on lifecycle callbacks |
| ArkUI/native callbacks re-enter GPUI | Never hold the foreground queue or surface-state borrow while running a task/event handler; contain all panics at C callback boundaries |
| Density arrives before the GPUI window exists | Store scale factor on the platform and apply it when every window is created |
| Text shaping or system fonts differ from Linux | Reuse existing shaping initially and load HarmonyOS font paths explicitly |
| Renderer allocations grow per frame | Add object/atlas lifetime instrumentation to the P1 gate |
| epoll/poll reports OHOS child pipes ready before a nonblocking read can progress | Use blocking worker-backed child-pipe I/O on OHOS; retain async sockets and the Unix child reaper |
| Full Zed dependencies block compilation | Keep an explicit OHOS product profile, enable production components incrementally, and exclude only target-incompatible integrations |
| External directories lack desktop filesystem semantics | Prove them after P2; retain sandbox import/export as the fallback |
| HNP behavior differs on the commercial PC image | Treat HNP as a post-editor runtime proof, not a P0 assumption |
| Arbitrary downloaded executables are prohibited | Support packaged HNP tools only unless policy and runtime tests prove more |
| Public OpenHarmony and HarmonyOS NEXT SDKs diverge | Pin both identities and validate every runtime gate on the actual emulator/device |
| CI lacks an emulator | Label CI compile/package-only and keep runtime gates explicit |

## 11. Deferred/open decisions

- Import/export UX for provider operations that cannot satisfy normal filesystem semantics.
- The representative Node language server acceptance test and eventual OpenSSH build.
- Whether remote SSH ultimately uses packaged OpenSSH or an in-process implementation.
- Extension-host and Wasmtime support.
- Physical-device minimum API and hardware support matrix.
- OpenHarmony runtime and dual-distribution strategy.
- Alternate renderer work, only after Native Drawing profiling.

## 12. Repository evidence to keep current

- `crates/zed_ohos/src/zed_ohos.rs` — HAP bootstrap, ArkUI frame bridge, real editor/worktree/buffer startup, and sandbox autosave.
- `crates/zed_ohos/hap/entry/src/main/ets/pages/Index.ets` — XComponent host, UI context, app sandbox paths, and ArkUI frame callback.
- `crates/gpui_ohos/src/platform.rs` — `Platform`, `PlatformWindow`, scale/input/IME coordination, and scene submission.
- `crates/gpui_ohos/src/dispatcher.rs` — background executors, reentrant foreground queue, timers, and ArkUI wake requests.
- `crates/gpui_ohos/src/xcomponent.rs` — XComponent lifecycle/input registration, surface ownership, and FFI panic containment.
- `crates/gpui_ohos/src/drawing.rs` — NativeWindow geometry, Native Drawing lifecycle, and scene translation.
- `crates/gpui_ohos/src/atlas.rs` — renderer atlas uploads and CPU-readable tile storage.
- `crates/gpui_ohos/src/clipboard.rs` — HarmonyOS system pasteboard text read/write.
- `crates/gpui_ohos/src/credentials.rs` — persistent encrypted HarmonyOS Asset Store provider credentials.
- `crates/gpui_ohos/src/path_prompt.rs` — persistent URI permission activation and native FileShare URI-to-path resolution.
- `crates/gpui_ohos/src/hnp.rs` — versioned lookup for packaged HNP executables.
- `crates/zed_ohos/hap/entry/src/main/module.json5` — runtime permissions for internet, pasteboard, and persistent Asset Store data.
- `crates/zed_ohos/hnp` — pinned shell/tool/Git/Node build inputs and HNP manifest.
- `crates/gpui_text` — renderer-neutral cosmic-text/swash GPUI text system.
- `script/build-ohos.ps1` and `script/ohos-x86_64-clang.cmd` — repeatable Rust/HAP build, signing, install, and launch path.
- `crates/gpui/src/platform.rs` — `Platform`, `PlatformWindow`, text-system, and atlas contracts.
- `crates/gpui_wgpu/src/wgpu_renderer.rs` — reference mapping from GPUI `Scene` primitives to an existing renderer.
- `crates/gpui_macos/src/metal_renderer.rs` — reference native renderer implementation.
- `crates/gpui_platform/src/gpui_platform.rs` — platform dispatch.
- `crates/gpui_platform/Cargo.toml` — platform dependencies.
- `crates/zed/Cargo.toml` — desktop product features and dependencies.
- `crates/remote/src/transport/ssh.rs` — executable-based SSH/SCP transport.
- `crates/git/src/repository.rs` — Git executable integration.
- `crates/languages/src/lib.rs` and `crates/grammars/src/grammars.rs` — built-in grammar registration.

## 13. External references

- [Rust platform support: OpenHarmony](https://doc.rust-lang.org/rustc/platform-support/openharmony.html)
- [OpenHarmony HNP development guide](https://gitcode.com/openharmony/startup_appspawn/tree/master/service/hnp)
- [Node.js OpenHarmony target support](https://github.com/nodejs/node/pull/58350)
- [Node.js 22.23.2 release](https://nodejs.org/id/blog/release/v22.23.2)
- [OpenHarmony persistent file permission](https://gitee.com/openharmony/docs/blob/1120d909a7f369bef6e021a33b289e213324c5e3/en/application-dev/file-management/file-persistPermission.md)
- [HarmonyOS persistent FileShare authorization](https://developer.huawei.com/consumer/cn/doc/HarmonyOS-Guides/native-fileshare-guidelines)
- [OpenHarmony XComponent development](https://gitee.com/openharmony/docs/blob/master/en/application-dev/ui/napi-xcomponent-guidelines.md)
- [OpenHarmony NativeWindow development](https://gitee.com/openharmony/docs/blob/master/en/application-dev/napi/native_window-guidelines.md)
- [OpenHarmony Native Drawing headers](https://gitee.com/openharmony/interface_sdk_c/tree/master/graphic/graphic_2d/native_drawing)
- [OpenHarmony Asset Store](https://gitee.com/openharmony/security_asset)
- [OpenHarmony ArkUI UIContext](https://gitee.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/js-apis-arkui-UIContext.md)
- [openharmony-rs/ohos-sys](https://github.com/openharmony-rs/ohos-sys)
- [ohos-rs/ohos-native-bindings](https://github.com/ohos-rs/ohos-native-bindings)
