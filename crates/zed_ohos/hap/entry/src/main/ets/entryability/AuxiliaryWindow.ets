import { window } from '@kit.ArkUI';
import { hilog } from '@kit.PerformanceAnalysisKit';

const LOG_DOMAIN = 0xD002;
const LOG_TAG = 'ZedOHOS';

enum WindowCommand {
  OPEN = 0,
  CLOSE = 1,
  SHOW = 2,
  MINIMIZE = 3,
  TOGGLE_MAXIMIZE = 4,
  TOGGLE_FULLSCREEN = 5,
  RESIZE = 6,
  SET_TITLE = 7
}

let windowStage: window.WindowStage | undefined;
let mainWindow: window.Window | undefined;
const auxiliaryWindows: Map<number, window.Window> = new Map();
const auxiliaryCloseHandlers: Map<number, () => void> = new Map();
const observedWindowIds: Set<number> = new Set();
let windowOperation: Promise<void> = Promise.resolve();
let windowStateHandler: ((
  windowId: number,
  physicalLeft: number,
  physicalTop: number,
  maximized: boolean,
  fullscreen: boolean
) => void) | undefined;

class AuxiliaryPageStorage {
  windowId: number;
  componentId: string;

  constructor(windowId: number) {
    this.windowId = windowId;
    this.componentId = `zed-auxiliary-surface-${windowId}`;
  }
}

export function configureWindowStage(stage: window.WindowStage): void {
  windowStage = stage;
  stage.getMainWindow().then((resolvedWindow: window.Window): void => {
    mainWindow = resolvedWindow;
    observeWindow(0, resolvedWindow);
  }).catch((error: Error): void => {
    hilog.error(LOG_DOMAIN, LOG_TAG, 'Failed to obtain the main window: %{public}s', JSON.stringify(error));
  });
}

function publishWindowState(windowId: number, targetWindow: window.Window): void {
  const handler = windowStateHandler;
  if (!handler) {
    return;
  }
  try {
    const rect = targetWindow.getWindowProperties().windowRect;
    let maximized = false;
    let fullscreen = false;
    try {
      const status = targetWindow.getWindowStatus();
      maximized = status === window.WindowStatusType.MAXIMIZE;
      fullscreen = status === window.WindowStatusType.FULL_SCREEN;
    } catch (_) {
      // A newly created main window can report no status before its first show.
      // Its rectangle is still valid and is needed to center auxiliary windows.
    }
    handler(
      windowId,
      rect.left,
      rect.top,
      maximized,
      fullscreen
    );
  } catch (error) {
    hilog.error(
      LOG_DOMAIN,
      LOG_TAG,
      'Failed to publish state for window %{public}d: %{public}s',
      windowId,
      JSON.stringify(error)
    );
  }
}

function observeWindow(windowId: number, targetWindow: window.Window): void {
  if (observedWindowIds.has(windowId)) {
    publishWindowState(windowId, targetWindow);
    return;
  }
  try {
    targetWindow.on('windowRectChange', (): void => {
      publishWindowState(windowId, targetWindow);
    });
    targetWindow.on('windowStatusChange', (): void => {
      publishWindowState(windowId, targetWindow);
    });
    observedWindowIds.add(windowId);
    publishWindowState(windowId, targetWindow);
  } catch (error) {
    hilog.error(
      LOG_DOMAIN,
      LOG_TAG,
      'Failed to observe state for window %{public}d: %{public}s',
      windowId,
      JSON.stringify(error)
    );
  }
}

function stopObservingWindow(windowId: number, targetWindow: window.Window): void {
  if (!observedWindowIds.delete(windowId)) {
    return;
  }
  try {
    targetWindow.off('windowRectChange');
    targetWindow.off('windowStatusChange');
  } catch (error) {
    hilog.error(
      LOG_DOMAIN,
      LOG_TAG,
      'Failed to stop observing window %{public}d: %{public}s',
      windowId,
      JSON.stringify(error)
    );
  }
}

function observeAuxiliaryClose(windowId: number, targetWindow: window.Window): void {
  try {
    targetWindow.on('windowWillClose', (): Promise<boolean> => {
      const handler = auxiliaryCloseHandlers.get(windowId);
      if (!handler) {
        hilog.warn(
          LOG_DOMAIN,
          LOG_TAG,
          'Ignored a close request while auxiliary window %{public}d was not ready',
          windowId
        );
        return Promise.resolve(true);
      }
      hilog.info(LOG_DOMAIN, LOG_TAG, 'Sending close request for auxiliary window %{public}d to GPUI', windowId);
      try {
        handler();
      } catch (error) {
        hilog.error(
          LOG_DOMAIN,
          LOG_TAG,
          'Failed to request GPUI close for auxiliary window %{public}d: %{public}s',
          windowId,
          JSON.stringify(error)
        );
      }
      return Promise.resolve(true);
    });
  } catch (error) {
    throw new Error(
      `Unable to observe close for HarmonyOS window ${windowId}: ${JSON.stringify(error)}`
    );
  }
}

function stopObservingAuxiliaryClose(windowId: number, targetWindow: window.Window): void {
  auxiliaryCloseHandlers.delete(windowId);
  try {
    targetWindow.off('windowWillClose');
  } catch (error) {
    hilog.error(
      LOG_DOMAIN,
      LOG_TAG,
      'Failed to stop observing close for auxiliary window %{public}d: %{public}s',
      windowId,
      JSON.stringify(error)
    );
  }
}

export function setAuxiliaryCloseHandler(
  windowId: number,
  handler: (() => void) | undefined
): void {
  if (handler) {
    auxiliaryCloseHandlers.set(windowId, handler);
  } else {
    auxiliaryCloseHandlers.delete(windowId);
  }
}

export function setWindowStateHandler(handler: ((
  windowId: number,
  physicalLeft: number,
  physicalTop: number,
  maximized: boolean,
  fullscreen: boolean
) => void) | undefined): void {
  windowStateHandler = handler;
  if (!handler) {
    return;
  }
  if (mainWindow) {
    observeWindow(0, mainWindow);
  }
  auxiliaryWindows.forEach((targetWindow: window.Window, windowId: number): void => {
    observeWindow(windowId, targetWindow);
  });
}

async function createAuxiliaryWindow(
  windowId: number,
  title: string,
  left: number,
  top: number,
  width: number,
  height: number
): Promise<window.Window> {
  const existing = auxiliaryWindows.get(windowId);
  if (existing) {
    return existing;
  }
  const stage = windowStage;
  if (!stage) {
    throw new Error('HarmonyOS window stage is not configured');
  }
  let createdWindow: window.Window;
  try {
    createdWindow = await stage.createSubWindowWithOptions(`zed-window-${windowId}`, {
      title: title,
      decorEnabled: true,
      isModal: false,
      maximizeSupported: true,
      windowRect: {
        left: left,
        top: top,
        width: width,
        height: height
      }
    });
  } catch (error) {
    throw new Error(`Unable to allocate HarmonyOS window ${windowId}: ${JSON.stringify(error)}`);
  }
  auxiliaryWindows.set(windowId, createdWindow);
  try {
    observeAuxiliaryClose(windowId, createdWindow);
    const storage = new LocalStorage(new AuxiliaryPageStorage(windowId));
    await createdWindow.loadContent('pages/Auxiliary', storage);
    await createdWindow.setWindowTitle(title);
    await createdWindow.resize(width, height);
    await createdWindow.moveWindowTo(left, top);
    await createdWindow.showWindow();
    observeWindow(windowId, createdWindow);
    return createdWindow;
  } catch (error) {
    auxiliaryWindows.delete(windowId);
    stopObservingAuxiliaryClose(windowId, createdWindow);
    stopObservingWindow(windowId, createdWindow);
    await createdWindow.destroyWindow().catch((): void => {});
    throw new Error(`Unable to create HarmonyOS window ${windowId}: ${JSON.stringify(error)}`);
  }
}

async function resolveWindow(windowId: number): Promise<window.Window> {
  if (windowId !== 0) {
    const auxiliaryWindow = auxiliaryWindows.get(windowId);
    if (!auxiliaryWindow) {
      throw new Error(`HarmonyOS auxiliary window ${windowId} does not exist`);
    }
    return auxiliaryWindow;
  }
  if (mainWindow) {
    return mainWindow;
  }
  const stage = windowStage;
  if (!stage) {
    throw new Error('HarmonyOS window stage is not configured');
  }
  try {
    mainWindow = await stage.getMainWindow();
  } catch (error) {
    throw new Error(`Unable to obtain the HarmonyOS main window: ${JSON.stringify(error)}`);
  }
  return mainWindow;
}

async function applyWindowCommand(
  windowId: number,
  command: WindowCommand,
  title: string,
  left: number,
  top: number,
  width: number,
  height: number
): Promise<void> {
  if (command === WindowCommand.OPEN) {
    await createAuxiliaryWindow(windowId, title, left, top, width, height);
    return;
  }
  if (command === WindowCommand.CLOSE) {
    if (windowId === 0) {
      return;
    }
    const auxiliaryWindow = auxiliaryWindows.get(windowId);
    if (!auxiliaryWindow) {
      return;
    }
    auxiliaryWindows.delete(windowId);
    stopObservingAuxiliaryClose(windowId, auxiliaryWindow);
    stopObservingWindow(windowId, auxiliaryWindow);
    try {
      await auxiliaryWindow.destroyWindow();
    } catch (error) {
      throw new Error(`Unable to close HarmonyOS window ${windowId}: ${JSON.stringify(error)}`);
    }
    return;
  }

  const targetWindow = await resolveWindow(windowId);
  try {
    switch (command) {
      case WindowCommand.SHOW:
        await targetWindow.showWindow();
        break;
      case WindowCommand.MINIMIZE:
        await targetWindow.minimize();
        break;
      case WindowCommand.TOGGLE_MAXIMIZE:
        if (targetWindow.getWindowStatus() === window.WindowStatusType.MAXIMIZE) {
          await targetWindow.recover();
        } else {
          await targetWindow.maximize(window.MaximizePresentation.EXIT_IMMERSIVE);
        }
        break;
      case WindowCommand.TOGGLE_FULLSCREEN:
        if (targetWindow.getWindowStatus() === window.WindowStatusType.FULL_SCREEN) {
          await targetWindow.recover();
        } else {
          await targetWindow.maximize(window.MaximizePresentation.ENTER_IMMERSIVE_DISABLE_TITLE_AND_DOCK_HOVER);
        }
        break;
      case WindowCommand.RESIZE:
        await targetWindow.resize(width, height);
        break;
      case WindowCommand.SET_TITLE:
        await targetWindow.setWindowTitle(title);
        break;
      default:
        throw new Error(`Unknown HarmonyOS window command ${command}`);
    }
  } catch (error) {
    throw new Error(
      `HarmonyOS window command ${command} failed for window ${windowId}: ${JSON.stringify(error)}`
    );
  }
}

export function controlWindow(
  windowId: number,
  command: number,
  title: string,
  left: number,
  top: number,
  width: number,
  height: number
): void {
  windowOperation = windowOperation.then((): Promise<void> => {
    return applyWindowCommand(windowId, command as WindowCommand, title, left, top, width, height);
  }).catch((error: Error): void => {
    hilog.error(
      LOG_DOMAIN,
      LOG_TAG,
      'Failed to apply window command %{public}d to window %{public}d: %{public}s',
      command,
      windowId,
      JSON.stringify(error)
    );
  });
}
