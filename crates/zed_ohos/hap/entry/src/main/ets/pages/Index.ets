import { FrameCallback, UIContext } from '@kit.ArkUI';
import {
  AbilityConstant,
  AbilityLifecycleCallback,
  Configuration,
  EnvironmentCallback,
  Permissions,
  UIAbility,
  Want,
  WantAgent,
  abilityAccessCtrl,
  common,
  wantAgent
} from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { fileUri, picker } from '@kit.CoreFileKit';
import { pointer } from '@kit.InputKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { notificationManager } from '@kit.NotificationKit';
import { controlWindow, setWindowStateHandler } from '../entryability/AuxiliaryWindow';
import { setNotificationResponseHandler } from '../entryability/NativeBridge';
import { dispatchFileDrag, FileDropNativeContext, fileDropTypes } from '../entryability/FileDrop';

const LOG_DOMAIN = 0xD002;
const LOG_TAG = 'ZedOHOS';
const FILE_ACCESS_PERSIST: Permissions = 'ohos.permission.FILE_ACCESS_PERSIST';

enum ApplicationCommand {
  ACTIVATE = 0,
  BACKGROUND = 1,
  RESTART = 2
}

interface ZedNativeContext extends FileDropNativeContext {
  initializeFrameScheduler(
    requestFrame: () => void,
    openExternalUrl: (url: string) => void,
    openExternalPath: (path: string, reveal: boolean) => void,
    setCursor: (style: number, visible: boolean) => void,
    controlWindow: (
      windowId: number,
      command: number,
      title: string,
      left: number,
      top: number,
      width: number,
      height: number
    ) => void,
    controlApplication: (command: number) => void,
    showSystemNotification: (
      id: number,
      tag: string,
      title: string,
      body: string,
      actionIds: Array<string>,
      actionLabels: Array<string>
    ) => void,
    dismissSystemNotification: (id: number, tag: string) => void,
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
    windowId: number,
    action: number,
    code: number,
    keyText: string,
    unicode: number | undefined,
    modifiers: number
  ): boolean;
  setLifecyclePhase(phase: number): void;
  handleMemoryWarning(): void;
  updateWindowState(
    windowId: number,
    physicalLeft: number,
    physicalTop: number,
    maximized: boolean,
    fullscreen: boolean
  ): void;
  completePathPrompt(requestId: number, paths: Array<string>, error?: string): void;
  completePrompt(requestId: number, answer?: number, error?: string): void;
  handleSystemNotificationResponse(tag: string, actionId?: string): void;
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
      0,
      event.type,
      event.keyCode,
      event.keyText,
      event.unicode,
      modifiers
    );
  }

  private createNotificationWantAgent(
    requestCode: number,
    tag: string,
    actionId: string
  ): Promise<WantAgent> {
    const targetWant: Want = {
      bundleName: 'dev.zed.Zed',
      abilityName: 'EntryAbility',
      parameters: {
        'zed.notification.tag': tag,
        'zed.notification.action': actionId
      }
    };
    const info: wantAgent.WantAgentInfo = {
      wants: [targetWant],
      actionType: wantAgent.OperationType.START_ABILITY,
      requestCode: requestCode,
      actionFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG]
    };
    return wantAgent.getWantAgent(info);
  }

  private async publishSystemNotification(
    abilityContext: common.UIAbilityContext,
    id: number,
    tag: string,
    title: string,
    body: string,
    actionIds: Array<string>,
    actionLabels: Array<string>
  ): Promise<void> {
    if (actionIds.length !== actionLabels.length) {
      throw new Error('System notification action IDs and labels have different lengths');
    }
    if (!await notificationManager.isNotificationEnabled()) {
      await notificationManager.requestEnableNotification(abilityContext);
    }
    const bodyWantAgent = await this.createNotificationWantAgent(id * 64, tag, '');
    const actionButtons: Array<notificationManager.NotificationActionButton> = [];
    for (let index = 0; index < actionIds.length; index += 1) {
      const actionId = actionIds[index];
      const actionLabel = actionLabels[index];
      const actionWantAgent = await this.createNotificationWantAgent(
        id * 64 + index + 1,
        tag,
        actionId
      );
      actionButtons.push({
        title: actionLabel,
        wantAgent: actionWantAgent
      });
    }
    const request: notificationManager.NotificationRequest = {
      id: id,
      label: tag,
      notificationSlotType: notificationManager.SlotType.OTHER_TYPES,
      content: {
        notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
        normal: {
          title: title,
          text: body
        }
      },
      wantAgent: bodyWantAgent,
      actionButtons: actionButtons,
      tapDismissed: true
    };
    await notificationManager.publish(request);
  }

  private controlApplication(
    abilityContext: common.UIAbilityContext,
    command: number
  ): void {
    let operation: Promise<void>;
    switch (command as ApplicationCommand) {
      case ApplicationCommand.ACTIVATE:
        controlWindow(0, 2, '', 0, 0, 0, 0);
        operation = abilityContext.startAbility({
          bundleName: 'dev.zed.Zed',
          abilityName: 'EntryAbility'
        });
        break;
      case ApplicationCommand.BACKGROUND:
        operation = abilityContext.moveAbilityToBackground();
        break;
      case ApplicationCommand.RESTART:
        operation = abilityContext.restartApp({
          bundleName: 'dev.zed.Zed',
          abilityName: 'EntryAbility'
        });
        break;
      default:
        hilog.error(
          LOG_DOMAIN,
          LOG_TAG,
          'Unknown application command %{public}d',
          command
        );
        return;
    }
    operation.catch((error: Error): void => {
      hilog.error(
        LOG_DOMAIN,
        LOG_TAG,
        'Failed to apply application command %{public}d: %{public}s',
        command,
        JSON.stringify(error)
      );
    });
  }

  aboutToDisappear(): void {
    this.unregisterLifecycle?.();
    this.unregisterLifecycle = undefined;
    this.unregisterEnvironment?.();
    this.unregisterEnvironment = undefined;
    setNotificationResponseHandler(undefined);
    setWindowStateHandler(undefined);
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
          }, (path: string, reveal: boolean) => {
            try {
              const uri = fileUri.getUriFromPath(path);
              abilityContext.startAbility({
                action: reveal ? 'ohos.want.action.select' : 'ohos.want.action.viewData',
                uri: uri
              }).catch((error: Error) => {
                hilog.error(
                  LOG_DOMAIN,
                  LOG_TAG,
                  'Failed to %{public}s external path %{public}s: %{public}s',
                  reveal ? 'reveal' : 'open',
                  path,
                  JSON.stringify(error)
                );
              });
            } catch (error) {
              hilog.error(
                LOG_DOMAIN,
                LOG_TAG,
                'Failed to convert external path %{public}s to a URI: %{public}s',
                path,
                JSON.stringify(error)
              );
            }
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
          }, (
            windowId: number,
            command: number,
            title: string,
            left: number,
            top: number,
            width: number,
            height: number
          ) => {
            controlWindow(windowId, command, title, left, top, width, height);
          }, (command: number) => {
            this.controlApplication(abilityContext, command);
          }, (
            id: number,
            tag: string,
            title: string,
            body: string,
            actionIds: Array<string>,
            actionLabels: Array<string>
          ) => {
            this.publishSystemNotification(
              abilityContext,
              id,
              tag,
              title,
              body,
              actionIds,
              actionLabels
            ).catch((error: Error): void => {
              hilog.error(
                LOG_DOMAIN,
                LOG_TAG,
                'Failed to publish system notification %{public}s: %{public}s',
                tag,
                JSON.stringify(error)
              );
            });
          }, (id: number, tag: string) => {
            notificationManager.cancel(id, tag).catch((error: Error): void => {
              hilog.error(
                LOG_DOMAIN,
                LOG_TAG,
                'Failed to dismiss system notification %{public}s: %{public}s',
                tag,
                JSON.stringify(error)
              );
            });
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
          setWindowStateHandler((
            windowId: number,
            physicalLeft: number,
            physicalTop: number,
            maximized: boolean,
            fullscreen: boolean
          ): void => {
            zedContext.updateWindowState(
              windowId,
              physicalLeft,
              physicalTop,
              maximized,
              fullscreen
            );
          });
          setNotificationResponseHandler((tag: string, actionId?: string): void => {
            zedContext.handleSystemNotificationResponse(tag, actionId);
          });
          zedContext.setScaleFactor(scaleFactor);
          zedContext.onFrame();
          frameCallback.request(uiContext);
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
        .onDragEnter((event: DragEvent): void => dispatchFileDrag(this.nativeContext, 0, 0, event))
        .onDragMove((event: DragEvent): void => dispatchFileDrag(this.nativeContext, 0, 1, event))
        .onDragLeave((event: DragEvent): void => dispatchFileDrag(this.nativeContext, 0, 2, event))
        .onDrop((event: DragEvent): void => dispatchFileDrag(this.nativeContext, 0, 3, event))
        .allowDrop(fileDropTypes)
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
