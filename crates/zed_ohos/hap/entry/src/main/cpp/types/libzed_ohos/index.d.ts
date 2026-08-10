export const nativeVersion: () => string;
export const initializeFrameScheduler: (
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
  uiContext: object,
  scaleFactor: number,
  filesDir: string,
  cacheDir: string
) => void;
export const setScaleFactor: (scaleFactor: number) => void;
export const dispatchKeyEvent: (
  action: number,
  code: number,
  keyText: string,
  unicode: number | undefined,
  modifiers: number
) => boolean;
export const closeAuxiliaryWindow: () => void;
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
