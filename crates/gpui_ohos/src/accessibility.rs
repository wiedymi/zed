use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_void},
    ptr::{self, NonNull},
    sync::{Arc, OnceLock, Weak},
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{A11yCallbacks, Pixels, Point, accesskit};
use parking_lot::Mutex;

use crate::xcomponent::XComponentHandle;

const SUCCESS: i32 = 0;
const FAILED: i32 = -1;
const BAD_PARAMETER: i32 = -2;
const SEARCH_PREDECESSORS: i32 = 1 << 0;
const SEARCH_SIBLINGS: i32 = 1 << 1;
const SEARCH_CHILDREN: i32 = 1 << 2;
const SEARCH_RECURSIVE_CHILDREN: i32 = 1 << 3;
const DIRECTION_FORWARD: i32 = 0x10;
const DIRECTION_BACKWARD: i32 = 0x20;
const DIRECTION_UP: i32 = 0x01;
const DIRECTION_DOWN: i32 = 0x02;
const DIRECTION_LEFT: i32 = 0x04;
const DIRECTION_RIGHT: i32 = 0x08;
const ACTION_CLICK: i32 = 0x10;
const ACTION_LONG_CLICK: i32 = 0x20;
const ACTION_GAIN_FOCUS: i32 = 0x40;
const ACTION_CLEAR_FOCUS: i32 = 0x80;
const ACTION_SCROLL_FORWARD: i32 = 0x100;
const ACTION_SCROLL_BACKWARD: i32 = 0x200;
const ACTION_SELECT_TEXT: i32 = 0x2000;
const ACTION_SET_TEXT: i32 = 0x4000;
const EVENT_PAGE_OPEN: i32 = 0x2000_0000;
const EVENT_PAGE_CONTENT_UPDATE: i32 = 0x0000_0800;
const EVENT_FOCUS_NODE_UPDATE: i32 = 0x1000_0001;

#[repr(C)]
struct NativeProvider([u8; 0]);
#[repr(C)]
struct NativeInfo([u8; 0]);
#[repr(C)]
struct NativeInfoList([u8; 0]);
#[repr(C)]
struct NativeActionArguments([u8; 0]);
#[repr(C)]
struct NativeEvent([u8; 0]);

#[repr(C)]
struct NativeAction {
    action_type: i32,
    description: *const c_char,
}

#[repr(C)]
struct NativeRect {
    left_top_x: i32,
    left_top_y: i32,
    right_bottom_x: i32,
    right_bottom_y: i32,
}

#[repr(C)]
struct NativeRange {
    min: f64,
    max: f64,
    current: f64,
}

#[repr(C)]
struct NativeCallbacks {
    find_by_id:
        Option<unsafe extern "C" fn(*const c_char, i64, i32, i32, *mut NativeInfoList) -> i32>,
    find_by_text: Option<
        unsafe extern "C" fn(*const c_char, i64, *const c_char, i32, *mut NativeInfoList) -> i32,
    >,
    find_focused:
        Option<unsafe extern "C" fn(*const c_char, i64, i32, i32, *mut NativeInfo) -> i32>,
    find_next_focus:
        Option<unsafe extern "C" fn(*const c_char, i64, i32, i32, *mut NativeInfo) -> i32>,
    execute_action: Option<
        unsafe extern "C" fn(*const c_char, i64, i32, *mut NativeActionArguments, i32) -> i32,
    >,
    clear_focus: Option<unsafe extern "C" fn(*const c_char) -> i32>,
    cursor_position: Option<unsafe extern "C" fn(*const c_char, i64, i32, *mut i32) -> i32>,
}

#[link(name = "ace_ndk.z")]
unsafe extern "C" {
    fn OH_NativeXComponent_GetNativeAccessibilityProvider(
        component: *mut c_void,
        provider: *mut *mut NativeProvider,
    ) -> i32;
    fn OH_ArkUI_AccessibilityProviderRegisterCallbackWithInstance(
        instance_id: *const c_char,
        provider: *mut NativeProvider,
        callbacks: *mut NativeCallbacks,
    ) -> i32;
    fn OH_ArkUI_AddAndGetAccessibilityElementInfo(list: *mut NativeInfoList) -> *mut NativeInfo;
    fn OH_ArkUI_AccessibilityElementInfoSetElementId(info: *mut NativeInfo, id: i32) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetParentId(info: *mut NativeInfo, id: i32) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetComponentType(
        info: *mut NativeInfo,
        value: *const c_char,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetContents(
        info: *mut NativeInfo,
        value: *const c_char,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetHintText(
        info: *mut NativeInfo,
        value: *const c_char,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetAccessibilityDescription(
        info: *mut NativeInfo,
        value: *const c_char,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetChildNodeIds(
        info: *mut NativeInfo,
        count: i32,
        ids: *mut i64,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetOperationActions(
        info: *mut NativeInfo,
        count: i32,
        actions: *mut NativeAction,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetScreenRect(
        info: *mut NativeInfo,
        rect: *mut NativeRect,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetCheckable(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetChecked(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetFocusable(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetFocused(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetVisible(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetSelected(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetClickable(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetLongClickable(info: *mut NativeInfo, value: bool)
    -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetEnabled(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetIsPassword(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetScrollable(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetEditable(info: *mut NativeInfo, value: bool) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetRangeInfo(
        info: *mut NativeInfo,
        range: *mut NativeRange,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetSelectedTextStart(
        info: *mut NativeInfo,
        index: i32,
    ) -> i32;
    fn OH_ArkUI_AccessibilityElementInfoSetSelectedTextEnd(
        info: *mut NativeInfo,
        index: i32,
    ) -> i32;
    fn OH_ArkUI_FindAccessibilityActionArgumentByKey(
        arguments: *mut NativeActionArguments,
        key: *const c_char,
        value: *mut *mut c_char,
    ) -> i32;
    fn OH_ArkUI_CreateAccessibilityEventInfo() -> *mut NativeEvent;
    fn OH_ArkUI_DestoryAccessibilityEventInfo(event: *mut NativeEvent);
    fn OH_ArkUI_AccessibilityEventSetEventType(event: *mut NativeEvent, event_type: i32) -> i32;
    fn OH_ArkUI_SendAccessibilityAsyncEvent(
        provider: *mut NativeProvider,
        event: *mut NativeEvent,
        callback: Option<unsafe extern "C" fn(i32)>,
    );
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TreeChange {
    content: bool,
    focus: bool,
}

struct State {
    callbacks: A11yCallbacks,
    active: bool,
    nodes: HashMap<accesskit::NodeId, accesskit::Node>,
    root: Option<accesskit::NodeId>,
    focus: Option<accesskit::NodeId>,
    ids: HashMap<accesskit::NodeId, i32>,
    reverse_ids: HashMap<i32, accesskit::NodeId>,
    next_id: i32,
    scale_factor: f32,
    screen_origin: Point<Pixels>,
}

impl State {
    fn update(&mut self, update: accesskit::TreeUpdate) -> Result<TreeChange> {
        let previous_root = self.root;
        let previous_focus = self.focus;
        let mut content_changed = false;
        if let Some(tree) = update.tree {
            content_changed |= self.root != Some(tree.root);
            self.root = Some(tree.root);
        }
        self.focus = Some(update.focus);
        for (id, node) in update.nodes {
            content_changed |= self.nodes.get(&id) != Some(&node);
            self.ensure_id(id)?;
            self.nodes.insert(id, node);
        }
        self.remove_unreachable();
        Ok(TreeChange {
            content: content_changed || previous_root != self.root,
            focus: previous_focus != self.focus,
        })
    }

    fn ensure_id(&mut self, id: accesskit::NodeId) -> Result<i32> {
        if let Some(native_id) = self.ids.get(&id) {
            return Ok(*native_id);
        }
        let native_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .context("accessibility element ID space is exhausted")?;
        self.ids.insert(id, native_id);
        self.reverse_ids.insert(native_id, id);
        Ok(native_id)
    }

    fn remove_unreachable(&mut self) {
        let Some(root) = self.root else { return };
        let mut reachable = HashSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if reachable.insert(id)
                && let Some(node) = self.nodes.get(&id)
            {
                pending.extend(node.children());
            }
        }
        self.nodes.retain(|id, _| reachable.contains(id));
        self.ids.retain(|id, _| reachable.contains(id));
        self.reverse_ids.retain(|_, id| reachable.contains(id));
        self.focus = self.focus.filter(|id| reachable.contains(id));
    }

    fn node_id(&self, native_id: i64) -> Option<accesskit::NodeId> {
        if native_id < 0 {
            self.root
        } else {
            i32::try_from(native_id)
                .ok()
                .and_then(|id| self.reverse_ids.get(&id).copied())
        }
    }

    fn native_id(&self, id: accesskit::NodeId) -> Option<i32> {
        self.ids.get(&id).copied()
    }

    fn parent_id(&self, child: accesskit::NodeId) -> Option<accesskit::NodeId> {
        self.nodes
            .iter()
            .find_map(|(id, node)| node.children().contains(&child).then_some(*id))
    }

    fn ordered_nodes(&self) -> Vec<accesskit::NodeId> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let mut ordered = Vec::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            if !node.is_hidden() {
                ordered.push(id);
                pending.extend(node.children().iter().rev());
            }
        }
        ordered
    }

    fn subtree_nodes(&self, root: accesskit::NodeId) -> Vec<accesskit::NodeId> {
        let mut ordered = Vec::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            if !node.is_hidden() {
                ordered.push(id);
                pending.extend(node.children().iter().rev());
            }
        }
        ordered
    }
}

type SharedState = Arc<Mutex<State>>;

fn registry() -> &'static Mutex<HashMap<String, Weak<Mutex<State>>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Weak<Mutex<State>>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone, Copy)]
struct NativeCallbacksPointer(NonNull<NativeCallbacks>);

// HarmonyOS retains this pointer and does not expose an unregister function.
// The callbacks contain only function pointers and select the per-window state
// through the instance ID, so one process-lifetime table serves every window.
unsafe impl Send for NativeCallbacksPointer {}
unsafe impl Sync for NativeCallbacksPointer {}

fn native_callbacks() -> NativeCallbacksPointer {
    static CALLBACKS: OnceLock<NativeCallbacksPointer> = OnceLock::new();
    *CALLBACKS.get_or_init(|| {
        let callbacks = Box::new(NativeCallbacks {
            find_by_id: Some(find_by_id),
            find_by_text: Some(find_by_text),
            find_focused: Some(find_focused),
            find_next_focus: Some(find_next_focus),
            execute_action: Some(execute_action),
            clear_focus: Some(clear_focus),
            cursor_position: Some(cursor_position),
        });
        NativeCallbacksPointer(NonNull::from(Box::leak(callbacks)))
    })
}

#[derive(Clone, Copy)]
struct NativeEventPointer(NonNull<NativeEvent>);

// HarmonyOS does not return the event pointer to its completion callback. Keep
// one process-wide event in flight so the callback always releases that event.
// Tree updates that arrive while it is pending are coalesced.
unsafe impl Send for NativeEventPointer {}
unsafe impl Sync for NativeEventPointer {}

fn pending_event() -> &'static Mutex<Option<NativeEventPointer>> {
    static EVENT: OnceLock<Mutex<Option<NativeEventPointer>>> = OnceLock::new();
    EVENT.get_or_init(|| Mutex::new(None))
}

unsafe extern "C" fn accessibility_event_sent(error_code: i32) {
    if error_code != SUCCESS {
        crate::log_message(
            crate::LogLevel::Error,
            format!("HarmonyOS accessibility event failed with result {error_code}"),
        );
    }
    let Some(event) = pending_event().lock().take() else {
        crate::log_message(
            crate::LogLevel::Error,
            "HarmonyOS completed an accessibility event that Zed did not own",
        );
        return;
    };
    unsafe { OH_ArkUI_DestoryAccessibilityEventInfo(event.0.as_ptr()) };
}

pub(crate) struct Adapter {
    instance_id: CString,
    provider: NonNull<NativeProvider>,
    state: SharedState,
}

impl Adapter {
    pub(crate) fn new(
        component: XComponentHandle,
        instance_id: &str,
        callbacks: A11yCallbacks,
        scale_factor: f32,
        screen_origin: Point<Pixels>,
        active: bool,
    ) -> Result<Self> {
        let instance_id = CString::new(instance_id).context("XComponent ID contains a NUL byte")?;
        let state = State {
            callbacks,
            active: false,
            nodes: HashMap::new(),
            root: None,
            focus: None,
            ids: HashMap::new(),
            reverse_ids: HashMap::new(),
            next_id: 1,
            scale_factor,
            screen_origin,
        };
        let state = Arc::new(Mutex::new(state));
        let mut provider = ptr::null_mut();
        ensure_success("get XComponent accessibility provider", unsafe {
            OH_NativeXComponent_GetNativeAccessibilityProvider(
                component.native().as_ptr().cast(),
                &mut provider,
            )
        })?;
        let provider =
            NonNull::new(provider).context("HarmonyOS returned a null accessibility provider")?;
        let callbacks = native_callbacks();
        let registry_key = instance_id.to_string_lossy().into_owned();
        registry()
            .lock()
            .insert(registry_key.clone(), Arc::downgrade(&state));
        if let Err(error) = ensure_success("register XComponent accessibility callbacks", unsafe {
            OH_ArkUI_AccessibilityProviderRegisterCallbackWithInstance(
                instance_id.as_ptr(),
                provider.as_ptr(),
                callbacks.0.as_ptr(),
            )
        }) {
            registry().lock().remove(&registry_key);
            return Err(error);
        }
        let adapter = Self {
            instance_id,
            provider,
            state,
        };
        adapter.set_active(active)?;
        Ok(adapter)
    }

    pub(crate) fn update(&self, update: accesskit::TreeUpdate) -> Result<()> {
        let mut state = self.state.lock();
        if !state.active {
            return Ok(());
        }
        let change = state.update(update)?;
        drop(state);
        if change.content {
            self.send_event(EVENT_PAGE_CONTENT_UPDATE)?;
        }
        if change.focus {
            self.send_event(EVENT_FOCUS_NODE_UPDATE)?;
        }
        Ok(())
    }

    pub(crate) fn set_active(&self, active: bool) -> Result<()> {
        let mut state = self.state.lock();
        if state.active == active {
            return Ok(());
        }
        if active {
            state.active = true;
            let result = (state.callbacks.activation)()
                .context("GPUI did not provide an initial accessibility tree")
                .and_then(|update| state.update(update).map(|_| ()));
            if let Err(error) = result {
                (state.callbacks.deactivation)();
                state.active = false;
                return Err(error);
            }
        } else {
            (state.callbacks.deactivation)();
            state.active = false;
            state.nodes.clear();
            state.root = None;
            state.focus = None;
            state.ids.clear();
            state.reverse_ids.clear();
            state.next_id = 1;
        }
        drop(state);
        if active {
            self.send_event(EVENT_PAGE_OPEN)?;
        }
        Ok(())
    }

    pub(crate) fn update_geometry(&self, scale_factor: f32, screen_origin: Point<Pixels>) {
        let mut state = self.state.lock();
        let changed = state.scale_factor != scale_factor || state.screen_origin != screen_origin;
        state.scale_factor = scale_factor;
        state.screen_origin = screen_origin;
        let notify = changed && state.active && state.focus.is_some();
        drop(state);
        if notify && let Err(error) = self.send_event(EVENT_FOCUS_NODE_UPDATE) {
            crate::log_message(
                crate::LogLevel::Error,
                format!("failed to send HarmonyOS accessibility bounds change: {error:#}"),
            );
        }
    }

    fn send_event(&self, event_type: i32) -> Result<()> {
        let mut pending = pending_event().lock();
        if pending.is_some() {
            return Ok(());
        }
        let event = NonNull::new(unsafe { OH_ArkUI_CreateAccessibilityEventInfo() })
            .context("HarmonyOS could not allocate an accessibility event")?;
        if let Err(error) = ensure_success("set HarmonyOS accessibility event type", unsafe {
            OH_ArkUI_AccessibilityEventSetEventType(event.as_ptr(), event_type)
        }) {
            unsafe { OH_ArkUI_DestoryAccessibilityEventInfo(event.as_ptr()) };
            return Err(error);
        }
        *pending = Some(NativeEventPointer(event));
        drop(pending);
        unsafe {
            OH_ArkUI_SendAccessibilityAsyncEvent(
                self.provider.as_ptr(),
                event.as_ptr(),
                Some(accessibility_event_sent),
            )
        };
        Ok(())
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        registry()
            .lock()
            .remove(self.instance_id.to_string_lossy().as_ref());
        let state = self.state.lock();
        if state.active {
            (state.callbacks.deactivation)();
        }
    }
}

fn state_for(instance_id: *const c_char) -> Option<SharedState> {
    let id = NonNull::new(instance_id.cast_mut())?;
    let id = unsafe { CStr::from_ptr(id.as_ptr()) }.to_str().ok()?;
    registry().lock().get(id)?.upgrade()
}

unsafe extern "C" fn find_by_id(
    instance_id: *const c_char,
    element_id: i64,
    mode: i32,
    _request_id: i32,
    list: *mut NativeInfoList,
) -> i32 {
    let (Some(list), Some(state)) = (NonNull::new(list), state_for(instance_id)) else {
        return BAD_PARAMETER;
    };
    let state = state.lock();
    let Some(id) = state.node_id(element_id) else {
        return FAILED;
    };
    let mut ids = vec![id];
    if mode & SEARCH_PREDECESSORS != 0 {
        let mut parent = state.parent_id(id);
        while let Some(id) = parent {
            ids.push(id);
            parent = state.parent_id(id);
        }
    }
    if mode & SEARCH_SIBLINGS != 0
        && let Some(parent) = state.parent_id(id).and_then(|id| state.nodes.get(&id))
    {
        ids.extend(parent.children().iter().copied());
    }
    if mode & (SEARCH_CHILDREN | SEARCH_RECURSIVE_CHILDREN) != 0
        && let Some(node) = state.nodes.get(&id)
    {
        let mut pending = node.children().to_vec();
        while let Some(id) = pending.pop() {
            ids.push(id);
            if mode & SEARCH_RECURSIVE_CHILDREN != 0
                && let Some(node) = state.nodes.get(&id)
            {
                pending.extend(node.children());
            }
        }
    }
    ids.sort_unstable_by_key(|id| state.native_id(*id));
    ids.dedup();
    for id in ids {
        let info = unsafe { OH_ArkUI_AddAndGetAccessibilityElementInfo(list.as_ptr()) };
        if fill_info(&state, id, info).is_err() {
            return FAILED;
        }
    }
    SUCCESS
}

unsafe extern "C" fn find_by_text(
    instance_id: *const c_char,
    element_id: i64,
    text: *const c_char,
    _request_id: i32,
    list: *mut NativeInfoList,
) -> i32 {
    let (Some(text), Some(list), Some(state)) = (
        NonNull::new(text.cast_mut()),
        NonNull::new(list),
        state_for(instance_id),
    ) else {
        return BAD_PARAMETER;
    };
    let text = unsafe { CStr::from_ptr(text.as_ptr()) }.to_string_lossy();
    let state = state.lock();
    let Some(root) = state.node_id(element_id) else {
        return FAILED;
    };
    for id in state.subtree_nodes(root) {
        let Some(node) = state.nodes.get(&id) else {
            continue;
        };
        let matches = node
            .label()
            .is_some_and(|value| value.contains(text.as_ref()))
            || node
                .value()
                .is_some_and(|value| value.contains(text.as_ref()))
            || node
                .description()
                .is_some_and(|value| value.contains(text.as_ref()));
        if matches {
            let info = unsafe { OH_ArkUI_AddAndGetAccessibilityElementInfo(list.as_ptr()) };
            if fill_info(&state, id, info).is_err() {
                return FAILED;
            }
        }
    }
    SUCCESS
}

unsafe extern "C" fn find_focused(
    instance_id: *const c_char,
    _element_id: i64,
    _focus_type: i32,
    _request_id: i32,
    info: *mut NativeInfo,
) -> i32 {
    let (Some(info), Some(state)) = (NonNull::new(info), state_for(instance_id)) else {
        return BAD_PARAMETER;
    };
    let state = state.lock();
    state
        .focus
        .and_then(|id| fill_info(&state, id, info.as_ptr()).ok())
        .map_or(FAILED, |()| SUCCESS)
}

unsafe extern "C" fn find_next_focus(
    instance_id: *const c_char,
    element_id: i64,
    direction: i32,
    _request_id: i32,
    info: *mut NativeInfo,
) -> i32 {
    let (Some(info), Some(state)) = (NonNull::new(info), state_for(instance_id)) else {
        return BAD_PARAMETER;
    };
    let state = state.lock();
    let ordered = state.ordered_nodes();
    let Some(index) = state
        .node_id(element_id)
        .and_then(|id| ordered.iter().position(|candidate| *candidate == id))
    else {
        return FAILED;
    };
    let next = match direction {
        DIRECTION_FORWARD => index.checked_add(1).and_then(|index| ordered.get(index)),
        DIRECTION_BACKWARD => index.checked_sub(1).and_then(|index| ordered.get(index)),
        DIRECTION_UP | DIRECTION_DOWN | DIRECTION_LEFT | DIRECTION_RIGHT => {
            spatial_neighbor(&state, ordered[index], &ordered, direction)
        }
        _ => None,
    };
    next.and_then(|id| fill_info(&state, *id, info.as_ptr()).ok())
        .map_or(FAILED, |()| SUCCESS)
}

fn spatial_neighbor<'a>(
    state: &State,
    source: accesskit::NodeId,
    candidates: &'a [accesskit::NodeId],
    direction: i32,
) -> Option<&'a accesskit::NodeId> {
    let source_bounds = state.nodes.get(&source)?.bounds()?;
    let source_x = (source_bounds.x0 + source_bounds.x1) / 2.0;
    let source_y = (source_bounds.y0 + source_bounds.y1) / 2.0;
    candidates
        .iter()
        .filter(|candidate| **candidate != source)
        .filter_map(|candidate| {
            let bounds = state.nodes.get(candidate)?.bounds()?;
            let x = (bounds.x0 + bounds.x1) / 2.0;
            let y = (bounds.y0 + bounds.y1) / 2.0;
            let (primary, secondary) = match direction {
                DIRECTION_UP if y < source_y => (source_y - y, (source_x - x).abs()),
                DIRECTION_DOWN if y > source_y => (y - source_y, (source_x - x).abs()),
                DIRECTION_LEFT if x < source_x => (source_x - x, (source_y - y).abs()),
                DIRECTION_RIGHT if x > source_x => (x - source_x, (source_y - y).abs()),
                _ => return None,
            };
            let score = primary.mul_add(primary * 4.0, secondary * secondary);
            score.is_finite().then_some((candidate, score))
        })
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(candidate, _)| candidate)
}

unsafe extern "C" fn execute_action(
    instance_id: *const c_char,
    element_id: i64,
    action: i32,
    arguments: *mut NativeActionArguments,
    _request_id: i32,
) -> i32 {
    let Some(state) = state_for(instance_id) else {
        return FAILED;
    };
    let state = state.lock();
    let Some(target_node) = state.node_id(element_id) else {
        return FAILED;
    };
    let Some(node) = state.nodes.get(&target_node) else {
        return FAILED;
    };
    let (action, data) = match action {
        ACTION_CLICK => (accesskit::Action::Click, None),
        ACTION_LONG_CLICK => (accesskit::Action::ShowContextMenu, None),
        ACTION_GAIN_FOCUS => (accesskit::Action::Focus, None),
        ACTION_CLEAR_FOCUS => (accesskit::Action::Blur, None),
        ACTION_SCROLL_FORWARD => (
            if node.supports_action(accesskit::Action::ScrollDown) {
                accesskit::Action::ScrollDown
            } else {
                accesskit::Action::ScrollRight
            },
            Some(accesskit::ActionData::ScrollUnit(
                accesskit::ScrollUnit::Page,
            )),
        ),
        ACTION_SCROLL_BACKWARD => (
            if node.supports_action(accesskit::Action::ScrollUp) {
                accesskit::Action::ScrollUp
            } else {
                accesskit::Action::ScrollLeft
            },
            Some(accesskit::ActionData::ScrollUnit(
                accesskit::ScrollUnit::Page,
            )),
        ),
        ACTION_SET_TEXT => {
            let Some(value) = action_argument(arguments, "ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE")
                .or_else(|| action_argument(arguments, "text"))
            else {
                return BAD_PARAMETER;
            };
            (
                if node.supports_action(accesskit::Action::SetValue) {
                    accesskit::Action::SetValue
                } else {
                    accesskit::Action::ReplaceSelectedText
                },
                Some(accesskit::ActionData::Value(value.into())),
            )
        }
        ACTION_SELECT_TEXT => {
            let Some(selection) = text_selection(&state, target_node, arguments) else {
                return BAD_PARAMETER;
            };
            (
                accesskit::Action::SetTextSelection,
                Some(accesskit::ActionData::SetTextSelection(selection)),
            )
        }
        _ => return BAD_PARAMETER,
    };
    (state.callbacks.action)(accesskit::ActionRequest {
        action,
        target_tree: accesskit::TreeId::ROOT,
        target_node,
        data,
    });
    SUCCESS
}

unsafe extern "C" fn clear_focus(instance_id: *const c_char) -> i32 {
    let Some(state) = state_for(instance_id) else {
        return FAILED;
    };
    let state = state.lock();
    if let Some(target_node) = state.focus {
        (state.callbacks.action)(accesskit::ActionRequest {
            action: accesskit::Action::Blur,
            target_tree: accesskit::TreeId::ROOT,
            target_node,
            data: None,
        });
    }
    SUCCESS
}

unsafe extern "C" fn cursor_position(
    instance_id: *const c_char,
    element_id: i64,
    _request_id: i32,
    index: *mut i32,
) -> i32 {
    let (Some(index), Some(state)) = (NonNull::new(index), state_for(instance_id)) else {
        return BAD_PARAMETER;
    };
    let state = state.lock();
    let Some(selection) = state
        .node_id(element_id)
        .and_then(|id| state.nodes.get(&id))
        .and_then(|node| node.text_selection())
    else {
        return FAILED;
    };
    let Ok(position) = i32::try_from(selection.focus.character_index) else {
        return FAILED;
    };
    unsafe { *index.as_ptr() = position };
    SUCCESS
}

fn fill_info(state: &State, id: accesskit::NodeId, info: *mut NativeInfo) -> Result<()> {
    let info = NonNull::new(info).context("HarmonyOS returned a null accessibility element")?;
    let node = state
        .nodes
        .get(&id)
        .context("accessibility node is missing")?;
    let native_id = state
        .native_id(id)
        .context("accessibility node ID is missing")?;
    check(unsafe { OH_ArkUI_AccessibilityElementInfoSetElementId(info.as_ptr(), native_id) })?;
    let parent_id = state
        .parent_id(id)
        .and_then(|id| state.native_id(id))
        .unwrap_or(-1);
    check(unsafe { OH_ArkUI_AccessibilityElementInfoSetParentId(info.as_ptr(), parent_id) })?;
    set_string(
        info,
        format!("{:?}", node.role()),
        OH_ArkUI_AccessibilityElementInfoSetComponentType,
    )?;
    if let Some(value) = node.label().or_else(|| node.value()) {
        set_string(info, value, OH_ArkUI_AccessibilityElementInfoSetContents)?;
    }
    if let Some(value) = node.placeholder() {
        set_string(info, value, OH_ArkUI_AccessibilityElementInfoSetHintText)?;
    }
    if let Some(value) = node.description() {
        set_string(
            info,
            value,
            OH_ArkUI_AccessibilityElementInfoSetAccessibilityDescription,
        )?;
    }
    let mut child_ids = node
        .children()
        .iter()
        .filter_map(|id| state.native_id(*id).map(i64::from))
        .collect::<Vec<_>>();
    let child_count = i32::try_from(child_ids.len()).context("too many accessibility children")?;
    check(unsafe {
        OH_ArkUI_AccessibilityElementInfoSetChildNodeIds(
            info.as_ptr(),
            child_count,
            child_ids.as_mut_ptr(),
        )
    })?;
    let mut actions = native_actions(node);
    let action_count = i32::try_from(actions.len()).context("too many accessibility actions")?;
    check(unsafe {
        OH_ArkUI_AccessibilityElementInfoSetOperationActions(
            info.as_ptr(),
            action_count,
            actions.as_mut_ptr(),
        )
    })?;
    if let Some(bounds) = node.bounds() {
        let mut rect = native_rect(bounds, state.scale_factor, state.screen_origin)?;
        check(unsafe { OH_ArkUI_AccessibilityElementInfoSetScreenRect(info.as_ptr(), &mut rect) })?;
    }
    if let (Some(min), Some(max), Some(current)) = (
        node.min_numeric_value(),
        node.max_numeric_value(),
        node.numeric_value(),
    ) && min.is_finite()
        && max.is_finite()
        && current.is_finite()
        && min <= max
    {
        let mut range = NativeRange { min, max, current };
        check(unsafe { OH_ArkUI_AccessibilityElementInfoSetRangeInfo(info.as_ptr(), &mut range) })?;
    }
    if let Some(selection) = node.text_selection() {
        let start = selection
            .anchor
            .character_index
            .min(selection.focus.character_index);
        let end = selection
            .anchor
            .character_index
            .max(selection.focus.character_index);
        let start = i32::try_from(start).context("accessibility selection start is too large")?;
        let end = i32::try_from(end).context("accessibility selection end is too large")?;
        check(unsafe {
            OH_ArkUI_AccessibilityElementInfoSetSelectedTextStart(info.as_ptr(), start)
        })?;
        check(unsafe { OH_ArkUI_AccessibilityElementInfoSetSelectedTextEnd(info.as_ptr(), end) })?;
    }
    let checkable = node.toggled().is_some();
    let checked = !matches!(node.toggled(), None | Some(accesskit::Toggled::False));
    let focusable = node.supports_action(accesskit::Action::Focus) || state.focus == Some(id);
    let clickable = node.supports_action(accesskit::Action::Click);
    let long_clickable = node.supports_action(accesskit::Action::ShowContextMenu);
    let scrollable = [
        accesskit::Action::ScrollDown,
        accesskit::Action::ScrollUp,
        accesskit::Action::ScrollLeft,
        accesskit::Action::ScrollRight,
    ]
    .into_iter()
    .any(|action| node.supports_action(action));
    let editable = node.supports_action(accesskit::Action::SetValue)
        || node.supports_action(accesskit::Action::ReplaceSelectedText);
    for result in [
        unsafe { OH_ArkUI_AccessibilityElementInfoSetCheckable(info.as_ptr(), checkable) },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetChecked(info.as_ptr(), checked) },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetFocusable(info.as_ptr(), focusable) },
        unsafe {
            OH_ArkUI_AccessibilityElementInfoSetFocused(info.as_ptr(), state.focus == Some(id))
        },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetVisible(info.as_ptr(), !node.is_hidden()) },
        unsafe {
            OH_ArkUI_AccessibilityElementInfoSetSelected(
                info.as_ptr(),
                node.is_selected().unwrap_or(false),
            )
        },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetClickable(info.as_ptr(), clickable) },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetLongClickable(info.as_ptr(), long_clickable) },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetEnabled(info.as_ptr(), !node.is_disabled()) },
        unsafe {
            OH_ArkUI_AccessibilityElementInfoSetIsPassword(
                info.as_ptr(),
                node.role() == accesskit::Role::PasswordInput,
            )
        },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetScrollable(info.as_ptr(), scrollable) },
        unsafe { OH_ArkUI_AccessibilityElementInfoSetEditable(info.as_ptr(), editable) },
    ] {
        check(result)?;
    }
    Ok(())
}

fn native_actions(node: &accesskit::Node) -> Vec<NativeAction> {
    let mut actions = Vec::new();
    push_action(&mut actions, node, ACTION_CLICK, accesskit::Action::Click);
    push_action(
        &mut actions,
        node,
        ACTION_LONG_CLICK,
        accesskit::Action::ShowContextMenu,
    );
    push_action(
        &mut actions,
        node,
        ACTION_GAIN_FOCUS,
        accesskit::Action::Focus,
    );
    push_action(
        &mut actions,
        node,
        ACTION_CLEAR_FOCUS,
        accesskit::Action::Blur,
    );
    if node.supports_action(accesskit::Action::ScrollDown)
        || node.supports_action(accesskit::Action::ScrollRight)
    {
        actions.push(NativeAction {
            action_type: ACTION_SCROLL_FORWARD,
            description: ptr::null(),
        });
    }
    if node.supports_action(accesskit::Action::ScrollUp)
        || node.supports_action(accesskit::Action::ScrollLeft)
    {
        actions.push(NativeAction {
            action_type: ACTION_SCROLL_BACKWARD,
            description: ptr::null(),
        });
    }
    if node.supports_action(accesskit::Action::SetValue)
        || node.supports_action(accesskit::Action::ReplaceSelectedText)
    {
        actions.push(NativeAction {
            action_type: ACTION_SET_TEXT,
            description: ptr::null(),
        });
    }
    push_action(
        &mut actions,
        node,
        ACTION_SELECT_TEXT,
        accesskit::Action::SetTextSelection,
    );
    actions
}

fn push_action(
    actions: &mut Vec<NativeAction>,
    node: &accesskit::Node,
    native: i32,
    action: accesskit::Action,
) {
    if node.supports_action(action) {
        actions.push(NativeAction {
            action_type: native,
            description: ptr::null(),
        });
    }
}

fn native_rect(bounds: accesskit::Rect, scale: f32, origin: Point<Pixels>) -> Result<NativeRect> {
    let scale = f64::from(scale);
    let origin_x = f64::from(origin.x);
    let origin_y = f64::from(origin.y);
    Ok(NativeRect {
        left_top_x: checked_coordinate(bounds.x0 * scale + origin_x)?,
        left_top_y: checked_coordinate(bounds.y0 * scale + origin_y)?,
        right_bottom_x: checked_coordinate(bounds.x1 * scale + origin_x)?,
        right_bottom_y: checked_coordinate(bounds.y1 * scale + origin_y)?,
    })
}

fn checked_coordinate(value: f64) -> Result<i32> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        bail!("accessibility coordinate {value} is outside i32")
    }
    Ok(value.round() as i32)
}

fn set_string(
    info: NonNull<NativeInfo>,
    value: impl AsRef<str>,
    setter: unsafe extern "C" fn(*mut NativeInfo, *const c_char) -> i32,
) -> Result<()> {
    let value = CString::new(value.as_ref()).context("accessibility text contains a NUL byte")?;
    check(unsafe { setter(info.as_ptr(), value.as_ptr()) })
}

fn action_argument(arguments: *mut NativeActionArguments, key: &str) -> Option<String> {
    let arguments = NonNull::new(arguments)?;
    let key = CString::new(key).ok()?;
    let mut value = ptr::null_mut();
    if unsafe {
        OH_ArkUI_FindAccessibilityActionArgumentByKey(arguments.as_ptr(), key.as_ptr(), &mut value)
    } != SUCCESS
    {
        return None;
    }
    let value = NonNull::new(value)?;
    Some(
        unsafe { CStr::from_ptr(value.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn text_selection(
    state: &State,
    target: accesskit::NodeId,
    arguments: *mut NativeActionArguments,
) -> Option<accesskit::TextSelection> {
    let current = state.nodes.get(&target)?.text_selection()?;
    let begin = action_argument(arguments, "selectTextBegin")
        .or_else(|| action_argument(arguments, "TextBegin"))?
        .parse::<usize>()
        .ok()?;
    let end = action_argument(arguments, "selectTextEnd")
        .or_else(|| action_argument(arguments, "TextEnd"))?
        .parse::<usize>()
        .ok()?;
    let forward = action_argument(arguments, "selectTextInForward")
        .or_else(|| action_argument(arguments, "TextInForward"))
        .is_none_or(|value| value != "false" && value != "0");
    let begin = accesskit::TextPosition {
        node: current.anchor.node,
        character_index: begin,
    };
    let end = accesskit::TextPosition {
        node: current.focus.node,
        character_index: end,
    };
    Some(if forward {
        accesskit::TextSelection {
            anchor: begin,
            focus: end,
        }
    } else {
        accesskit::TextSelection {
            anchor: end,
            focus: begin,
        }
    })
}

fn ensure_success(operation: &str, result: i32) -> Result<()> {
    if result == SUCCESS {
        Ok(())
    } else {
        Err(anyhow!("{operation} failed with HarmonyOS result {result}"))
    }
}

fn check(result: i32) -> Result<()> {
    ensure_success("update HarmonyOS accessibility element", result)
}
