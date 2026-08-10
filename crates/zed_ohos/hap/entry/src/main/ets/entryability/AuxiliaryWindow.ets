import { window } from '@kit.ArkUI';
import { hilog } from '@kit.PerformanceAnalysisKit';

const LOG_DOMAIN = 0xD002;
const LOG_TAG = 'ZedOHOS';
const SETTINGS_WINDOW_NAME = 'zed-settings';

let windowStage: window.WindowStage | undefined;
let settingsWindow: window.Window | undefined;
let settingsWindowReady: Promise<window.Window> | undefined;
let windowOperation: Promise<void> = Promise.resolve();

export function configureAuxiliaryWindowStage(stage: window.WindowStage): void {
  windowStage = stage;
}

export function prepareAuxiliaryWindow(): Promise<window.Window> {
  if (settingsWindow) {
    return Promise.resolve(settingsWindow);
  }
  if (settingsWindowReady) {
    return settingsWindowReady;
  }
  const stage = windowStage;
  if (!stage) {
    return Promise.reject(new Error('HarmonyOS window stage is not configured'));
  }
  const ready = stage.createSubWindowWithOptions(SETTINGS_WINDOW_NAME, {
    title: 'Zed — Settings',
    decorEnabled: true,
    isModal: false,
    maximizeSupported: true,
    windowRect: {
      left: 0,
      top: 0,
      width: 128,
      height: 128
    }
  }).then((createdWindow: window.Window): Promise<window.Window> => {
    settingsWindow = createdWindow;
    return createdWindow.loadContent('pages/Auxiliary')
      .then(() => createdWindow.showWindow({ focusOnShow: false }))
      .then(() => createdWindow.minimize())
      .then(() => createdWindow);
  }).catch((error: Error): never => {
    settingsWindow = undefined;
    settingsWindowReady = undefined;
    throw error;
  });
  settingsWindowReady = ready;
  return ready;
}

export function setAuxiliaryWindowVisibility(
  visible: boolean,
  _title: string,
  width: number,
  height: number
): void {
  windowOperation = windowOperation.then(async (): Promise<void> => {
    const auxiliaryWindow = await prepareAuxiliaryWindow();
    if (visible) {
      await auxiliaryWindow.resize(width, height);
      await auxiliaryWindow.showWindow();
    } else {
      await auxiliaryWindow.minimize();
    }
  }).catch((error: Error): void => {
    hilog.error(
      LOG_DOMAIN,
      LOG_TAG,
      'Failed to update the Settings window: %{public}s',
      JSON.stringify(error)
    );
  });
}
