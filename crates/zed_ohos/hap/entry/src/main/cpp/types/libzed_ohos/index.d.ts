export const nativeVersion: () => string;
export const initializeFrameScheduler: (
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
  uiContext: object,
  scaleFactor: number,
  filesDir: string,
  cacheDir: string
) => void;
export const setScaleFactor: (scaleFactor: number) => void;
export const dispatchKeyEvent: (
  windowId: number,
  action: number,
  code: number,
  keyText: string,
  unicode: number | undefined,
  modifiers: number
) => boolean;
export const dispatchFileDropEvent: (
  windowId: number,
  kind: number,
  x: number,
  y: number,
  uris: Array<string>
) => void;
export const attachAuxiliaryWindow: (windowId: number) => void;
export const closeAuxiliaryWindow: (windowId: number) => void;
export const updateWindowState: (
  windowId: number,
  physicalLeft: number,
  physicalTop: number,
  maximized: boolean,
  fullscreen: boolean
) => void;
export const updateWindowActivation: (windowId: number, active: boolean) => void;
export const handleSystemNotificationResponse: (tag: string, actionId?: string) => void;
export const setLifecyclePhase: (phase: number) => void;
export const handleMemoryWarning: () => void;
export const completePathPrompt: (
  requestId: number,
  paths: Array<string>,
  error?: string
) => void;
export const completePrompt: (
  requestId: number,
  answer?: number,
  error?: string
) => void;
export const onFrame: () => void;
