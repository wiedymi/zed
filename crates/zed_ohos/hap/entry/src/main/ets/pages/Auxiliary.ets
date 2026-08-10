interface ZedAuxiliaryNativeContext {
  setScaleFactor(scaleFactor: number): void;
  dispatchKeyEvent(
    action: number,
    code: number,
    keyText: string,
    unicode: number | undefined,
    modifiers: number
  ): boolean;
  closeAuxiliaryWindow(): void;
}

@Entry
@Component
struct Auxiliary {
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
      event.type,
      event.keyCode,
      event.keyText,
      event.unicode,
      modifiers
    );
  }

  aboutToDisappear(): void {
    this.nativeContext?.closeAuxiliaryWindow();
    this.nativeContext = undefined;
  }

  build() {
    Stack() {
      XComponent({
        id: 'zed-settings-surface',
        type: XComponentType.SURFACE,
        libraryname: 'zed_ohos'
      })
        .onLoad((nativeContext) => {
          const zedContext = nativeContext as ZedAuxiliaryNativeContext;
          this.nativeContext = zedContext;
          zedContext.setScaleFactor(1 / this.getUIContext().px2vp(1));
        })
        .onKeyPreIme((event: KeyEvent): boolean => this.dispatchNativeKeyEvent(event))
        .onKeyEvent((event: KeyEvent): boolean => this.dispatchNativeKeyEvent(event))
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
