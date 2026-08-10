import { dispatchFileDrag, FileDropNativeContext, fileDropTypes } from '../entryability/FileDrop';

interface ZedAuxiliaryNativeContext extends FileDropNativeContext {
  setScaleFactor(scaleFactor: number): void;
  dispatchKeyEvent(
    windowId: number,
    action: number,
    code: number,
    keyText: string,
    unicode: number | undefined,
    modifiers: number
  ): boolean;
  attachAuxiliaryWindow(windowId: number): void;
  closeAuxiliaryWindow(windowId: number): void;
}

@Entry({ useSharedStorage: true })
@Component
struct Auxiliary {
  @LocalStorageProp('windowId') private windowId: number = 0;
  @LocalStorageProp('componentId') private componentId: string = '';
  private nativeContext: ZedAuxiliaryNativeContext | undefined;

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
      this.windowId,
      event.type,
      event.keyCode,
      event.keyText,
      event.unicode,
      modifiers
    );
  }

  aboutToDisappear(): void {
    this.nativeContext?.closeAuxiliaryWindow(this.windowId);
    this.nativeContext = undefined;
  }

  build() {
    Stack() {
      XComponent({
        id: this.componentId,
        type: XComponentType.SURFACE,
        libraryname: 'zed_ohos'
      })
        .onLoad((nativeContext) => {
          const zedContext = nativeContext as ZedAuxiliaryNativeContext;
          this.nativeContext = zedContext;
          zedContext.attachAuxiliaryWindow(this.windowId);
          zedContext.setScaleFactor(1 / this.getUIContext().px2vp(1));
        })
        .onKeyPreIme((event: KeyEvent): boolean => this.dispatchNativeKeyEvent(event))
        .onKeyEvent((event: KeyEvent): boolean => this.dispatchNativeKeyEvent(event))
        .onDragEnter((event: DragEvent): void =>
          dispatchFileDrag(this.nativeContext, this.windowId, 0, event))
        .onDragMove((event: DragEvent): void =>
          dispatchFileDrag(this.nativeContext, this.windowId, 1, event))
        .onDragLeave((event: DragEvent): void =>
          dispatchFileDrag(this.nativeContext, this.windowId, 2, event))
        .onDrop((event: DragEvent): void =>
          dispatchFileDrag(this.nativeContext, this.windowId, 3, event))
        .allowDrop(fileDropTypes)
        .onAreaChange(() => {
          this.nativeContext?.setScaleFactor(1 / this.getUIContext().px2vp(1));
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
