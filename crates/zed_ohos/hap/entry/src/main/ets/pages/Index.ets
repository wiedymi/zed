import { FrameCallback, UIContext } from '@kit.ArkUI';
import {
  AbilityConstant,
  AbilityLifecycleCallback,
  Configuration,
  EnvironmentCallback,
  Permissions,
  UIAbility,
  abilityAccessCtrl,
  common
} from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { picker } from '@kit.CoreFileKit';
import { pointer } from '@kit.InputKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import {
  prepareAuxiliaryWindow,
  setAuxiliaryWindowVisibility
} from '../entryability/AuxiliaryWindow';

const LOG_DOMAIN = 0xD002;
const LOG_TAG = 'ZedOHOS';
const FILE_ACCESS_PERSIST: Permissions = 'ohos.permission.FILE_ACCESS_PERSIST';

interface ZedNativeContext {
  initializeFrameScheduler(
    requestFrame: () => void,
    openExternalUrl: (url: string) => void,
    setCursor: (style: number, visible: boolean) => void,
    setAuxiliaryWindow: (visible: boolean, title: string, width: number, height: number) => void,
    requestPathPrompt: (
      requestId: number,
      files: boolean,
      directories: boolean,
      multiple: boolean,
      save: boolean,
      suggestedName?: string,
      defaultUri?: string
    ) => void,
    requestPrompt: (
      requestId: number,
      level: number,
      message: string,
      detail: string | undefined,
      answers: Array<string>
    ) => void,
    uiContext: UIContext,
    scaleFactor: number,
    filesDir: string,
    cacheDir: string
  ): void;
  setScaleFactor(scaleFactor: number): void;
  dispatchKeyEvent(
    action: number,
    code: number,
    keyText: string,
    unicode: number | undefined,
    modifiers: number
  ): boolean;
  setLifecyclePhase(phase: number): void;
  handleMemoryWarning(): void;
  completePathPrompt(requestId: number, paths: Array<string>, error?: string): void;
  completePrompt(requestId: number, answer?: number, error?: string): void;
  closeAuxiliaryWindow(): void;
  onFrame(): void;
}

class ZedFrameCallback extends FrameCallback {
  private nativeContext: ZedNativeContext;
  private scheduled: boolean = false;

  constructor(nativeContext: ZedNativeContext) {
    super();
    this.nativeContext = nativeContext;
  }

  request(uiContext: UIContext): void {
    if (this.scheduled) {
      return;
    }
    this.scheduled = true;
    uiContext.postFrameCallback(this);
  }

  onFrame(_frameTimeInNano: number): void {
    this.scheduled = false;
    this.nativeContext.onFrame();
  }

  onIdle(_timeLeftInNano: number): void {
  }
}

@Entry
@Component
struct Index {
  private frameCallback: ZedFrameCallback | undefined;
  private nativeContext: ZedNativeContext | undefined;
  private unregisterLifecycle: (() => void) | undefined;
  private unregisterEnvironment: (() => void) | undefined;

  private dispatchNativeKeyEvent(event: KeyEvent): boolean {
    const nativeContext = this.nativeContext;
    if (!nativeContext) {
      return false;
    }
    let modifiers = 0;
    if (event.getModifierKeyState?.(['Shift'])) {
      modifiers |= 1;
    }
    if (event.getModifierKeyState?.(['Ctrl'])) {
      modifiers |= 2;
    }
    if (event.getModifierKeyState?.(['Alt'])) {
      modifiers |= 4;
    }
    return nativeContext.dispatchKeyEvent(
      event.type,
      event.keyCode,
      event.keyText,
      event.unicode,
      modifiers
    );
  }

  aboutToDisappear(): void {
    this.unregisterLifecycle?.();
    this.unregisterLifecycle = undefined;
    this.unregisterEnvironment?.();
    this.unregisterEnvironment = undefined;
    this.nativeContext?.setLifecyclePhase(2);
  }

  build() {
    Stack() {
      XComponent({
        id: 'zed-surface',
        type: XComponentType.SURFACE,
        libraryname: 'zed_ohos'
      })
        .onLoad((nativeContext) => {
          const uiContext = this.getUIContext();
          const hostContext = uiContext.getHostContext();
          if (!hostContext) {
            throw new Error('Zed XComponent has no ArkUI host context');
          }
          const abilityContext = hostContext as common.UIAbilityContext;
          const scaleFactor = 1 / uiContext.px2vp(1);
          const zedContext = nativeContext as ZedNativeContext;
          this.nativeContext = zedContext;
          const frameCallback = new ZedFrameCallback(zedContext);
          this.frameCallback = frameCallback;
          zedContext.initializeFrameScheduler(() => {
            frameCallback.request(uiContext);
          }, (url: string) => {
            abilityContext.startAbility({
              action: 'ohos.want.action.viewData',
              uri: url
            }).catch((error: Error) => {
              hilog.error(LOG_DOMAIN, LOG_TAG, 'Failed to open external URL: %{public}s', JSON.stringify(error));
            });
          }, (style: number, visible: boolean) => {
            const cursorController = uiContext.getCursorController();
            const cursorStyles: Array<pointer.PointerStyle> = [
              pointer.PointerStyle.DEFAULT,
              pointer.PointerStyle.TEXT_CURSOR,
              pointer.PointerStyle.CROSS,
              pointer.PointerStyle.HAND_GRABBING,
              pointer.PointerStyle.HAND_OPEN,
              pointer.PointerStyle.HAND_POINTING,
              pointer.PointerStyle.WEST,
              pointer.PointerStyle.EAST,
              pointer.PointerStyle.WEST_EAST,
              pointer.PointerStyle.NORTH,
              pointer.PointerStyle.SOUTH,
              pointer.PointerStyle.NORTH_SOUTH,
              pointer.PointerStyle.NORTH_WEST_SOUTH_EAST,
              pointer.PointerStyle.NORTH_EAST_SOUTH_WEST,
              pointer.PointerStyle.TEXT_CURSOR,
              pointer.PointerStyle.CURSOR_FORBID,
              pointer.PointerStyle.HAND_POINTING,
              pointer.PointerStyle.CURSOR_COPY,
              pointer.PointerStyle.DEFAULT
            ];
            const cursorStyle = cursorStyles[style] ?? pointer.PointerStyle.DEFAULT;
            cursorController.setCursor(cursorStyle);
            pointer.setPointerVisibleSync(visible);
          }, (visible: boolean, title: string, width: number, height: number) => {
            setAuxiliaryWindowVisibility(visible, title, width, height);
          }, (
            requestId: number,
            files: boolean,
            directories: boolean,
            multiple: boolean,
            save: boolean,
            suggestedName?: string,
            defaultUri?: string
          ) => {
            hilog.info(
              LOG_DOMAIN,
              LOG_TAG,
              'Opening path prompt %{public}d (files=%{public}s, directories=%{public}s, multiple=%{public}s)',
              requestId,
              `${files}`,
              `${directories}`,
              `${multiple}`
            );
            abilityAccessCtrl.createAtManager()
              .requestPermissionsFromUser(abilityContext, [FILE_ACCESS_PERSIST])
              .then((result) => {
                if (result.authResults.length !== 1 || result.authResults[0] !== 0) {
                  zedContext.completePathPrompt(requestId, [], 'Persistent folder access was not granted');
                  return;
                }
                const documentPicker = new picker.DocumentViewPicker(abilityContext);
                let selection: Promise<Array<string>>;
                if (save) {
                  const options = new picker.DocumentSaveOptions();
                  if (suggestedName) {
                    options.newFileNames = [suggestedName];
                  }
                  if (defaultUri) {
                    options.defaultFilePathUri = defaultUri;
                  }
                  hilog.info(LOG_DOMAIN, LOG_TAG, 'Launching DocumentViewPicker.save for request %{public}d', requestId);
                  selection = documentPicker.save(options);
                } else {
                  const options = new picker.DocumentSelectOptions();
                  if (files && directories) {
                    options.selectMode = picker.DocumentSelectMode.MIXED;
                  } else if (directories) {
                    options.selectMode = picker.DocumentSelectMode.FOLDER;
                  } else {
                    options.selectMode = picker.DocumentSelectMode.FILE;
                  }
                  options.maxSelectNumber = multiple ? 500 : 1;
                  hilog.info(LOG_DOMAIN, LOG_TAG, 'Launching DocumentViewPicker.select for request %{public}d', requestId);
                  selection = documentPicker.select(options);
                }
                selection
                  .then((paths: Array<string>) => {
                    hilog.info(
                      LOG_DOMAIN,
                      LOG_TAG,
                      'DocumentViewPicker request %{public}d returned %{public}d path(s)',
                      requestId,
                      paths.length
                    );
                    zedContext.completePathPrompt(requestId, paths);
                  })
                  .catch((error: BusinessError) => {
                    if (error.code === 13900001) {
                      zedContext.completePathPrompt(requestId, []);
                      return;
                    }
                    zedContext.completePathPrompt(
                      requestId,
                      [],
                      `HarmonyOS picker failed (${error.code}): ${error.message}`
                    );
                  });
              })
              .catch((error: Error) => {
                zedContext.completePathPrompt(
                  requestId,
                  [],
                  `Unable to request persistent folder access: ${error.message}`
                );
              });
          }, (
            requestId: number,
            level: number,
            message: string,
            detail: string | undefined,
            answers: Array<string>
          ) => {
            const buttons: Array<AlertDialogButtonOptions> = answers.map(
              (answer: string, index: number): AlertDialogButtonOptions => ({
                value: answer,
                defaultFocus: index === 0,
                primary: index === 0,
                action: () => zedContext.completePrompt(requestId, index)
              })
            );
            try {
              hilog.info(
                LOG_DOMAIN,
                LOG_TAG,
                'Opening GPUI prompt %{public}d (level=%{public}d, answers=%{public}d)',
                requestId,
                level,
                answers.length
              );
              uiContext.showAlertDialog({
                title: detail ? message : undefined,
                message: detail ?? message,
                autoCancel: false,
                buttons: buttons
              });
            } catch (error) {
              zedContext.completePrompt(requestId, undefined, `Unable to open dialog: ${JSON.stringify(error)}`);
            }
          }, uiContext, scaleFactor, abilityContext.filesDir, abilityContext.cacheDir);
          setTimeout((): void => {
            prepareAuxiliaryWindow()
              .then((): void => {
                zedContext.setScaleFactor(scaleFactor);
                zedContext.onFrame();
                frameCallback.request(uiContext);
              })
              .catch((error: Error): void => {
                hilog.error(
                  LOG_DOMAIN,
                  LOG_TAG,
                  'Failed to prepare the Settings window: %{public}s',
                  JSON.stringify(error)
                );
              });
          }, 3000);
          const applicationContext = abilityContext.getApplicationContext();
          const lifecycleCallback: AbilityLifecycleCallback = {
            onAbilityCreate: (_ability: UIAbility): void => {},
            onWindowStageCreate: (_ability: UIAbility): void => {},
            onWindowStageActive: (_ability: UIAbility): void => zedContext.setLifecyclePhase(0),
            onWindowStageInactive: (_ability: UIAbility): void => zedContext.setLifecyclePhase(1),
            onWindowStageDestroy: (_ability: UIAbility): void => zedContext.setLifecyclePhase(2),
            onAbilityDestroy: (_ability: UIAbility): void => zedContext.setLifecyclePhase(2),
            onAbilityForeground: (_ability: UIAbility): void => zedContext.setLifecyclePhase(3),
            onAbilityBackground: (_ability: UIAbility): void => zedContext.setLifecyclePhase(2),
            onAbilityContinue: (_ability: UIAbility): void => {}
          };
          const lifecycleCallbackId = applicationContext.on('abilityLifecycle', lifecycleCallback);
          this.unregisterLifecycle = () => {
            applicationContext.off('abilityLifecycle', lifecycleCallbackId).catch((error: Error) => {
              hilog.error(
                LOG_DOMAIN,
                LOG_TAG,
                'Failed to remove lifecycle callback: %{public}s',
                JSON.stringify(error)
              );
            });
          };
          const environmentCallback: EnvironmentCallback = {
            onConfigurationUpdated: (_configuration: Configuration): void => {},
            onMemoryLevel: (_level: AbilityConstant.MemoryLevel): void => zedContext.handleMemoryWarning()
          };
          const environmentCallbackId = applicationContext.on('environment', environmentCallback);
          this.unregisterEnvironment = () => {
            applicationContext.off('environment', environmentCallbackId).catch((error: Error) => {
              hilog.error(
                LOG_DOMAIN,
                LOG_TAG,
                'Failed to remove environment callback: %{public}s',
                JSON.stringify(error)
              );
            });
          };
          zedContext.setLifecyclePhase(0);
        })
        .onKeyPreIme((event: KeyEvent): boolean => this.dispatchNativeKeyEvent(event))
        .onKeyEvent((event: KeyEvent): boolean => this.dispatchNativeKeyEvent(event))
        .onAreaChange(() => {
          const nativeContext = this.nativeContext;
          if (nativeContext) {
            nativeContext.setScaleFactor(1 / this.getUIContext().px2vp(1));
          }
        })
        .width('100%')
        .height('100%')
        .focusable(true)
        .focusOnTouch(true)
    }
    .width('100%')
    .height('100%')
  }
}
