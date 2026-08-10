import { unifiedDataChannel, uniformTypeDescriptor } from '@kit.ArkData';
import { hilog } from '@kit.PerformanceAnalysisKit';

const LOG_DOMAIN = 0xD002;
const LOG_TAG = 'ZedOHOS';

export interface FileDropNativeContext {
  dispatchFileDropEvent(
    windowId: number,
    kind: number,
    x: number,
    y: number,
    uris: Array<string>
  ): void;
}

function fileUris(event: DragEvent): Array<string> {
  const uris: Array<string> = [];
  try {
    const records = event.getData().getRecords();
    for (const record of records) {
      const uri = (record as unifiedDataChannel.File).uri;
      if (uri && !uris.includes(uri)) {
        uris.push(uri);
      }
    }
  } catch (error) {
    hilog.warn(
      LOG_DOMAIN,
      LOG_TAG,
      'Unable to read UDMF file records from drag event: %{public}s',
      JSON.stringify(error)
    );
  }
  return uris;
}

export function dispatchFileDrag(
  nativeContext: FileDropNativeContext | undefined,
  windowId: number,
  kind: number,
  event: DragEvent
): void {
  if (!nativeContext) {
    return;
  }
  const uris = kind === 0 || kind === 3 ? fileUris(event) : [];
  const effectiveKind = kind === 0 && uris.length === 0 ? 1 : kind;
  try {
    nativeContext.dispatchFileDropEvent(
      windowId,
      effectiveKind,
      event.getWindowX(),
      event.getWindowY(),
      uris
    );
  } catch (error) {
    hilog.error(
      LOG_DOMAIN,
      LOG_TAG,
      'Failed to dispatch file drag event %{public}d: %{public}s',
      kind,
      JSON.stringify(error)
    );
  }
}

export const fileDropTypes: Array<uniformTypeDescriptor.UniformDataType> = [
  uniformTypeDescriptor.UniformDataType.FILE,
  uniformTypeDescriptor.UniformDataType.FILE_URI,
  uniformTypeDescriptor.UniformDataType.DIRECTORY,
  uniformTypeDescriptor.UniformDataType.FOLDER
];
