import {
  AbilityConstant,
  ConfigurationConstant,
  Permissions,
  UIAbility,
  Want,
  abilityAccessCtrl
} from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';
import { configureWindowStage } from './AuxiliaryWindow';
import { deliverNotificationResponse } from './NativeBridge';

const LOG_DOMAIN = 0xD002;
const LOG_TAG = 'ZedOHOS';
const READ_PASTEBOARD: Permissions = 'ohos.permission.READ_PASTEBOARD';

export default class EntryAbility extends UIAbility {
  private handleNotificationWant(want: Want): void {
    const tag = want.parameters?.['zed.notification.tag'];
    if (typeof tag !== 'string') {
      return;
    }
    const action = want.parameters?.['zed.notification.action'];
    deliverNotificationResponse(tag, typeof action === 'string' && action.length > 0 ? action : undefined);
  }

  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    try {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET);
    } catch (error) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'Failed to apply system color mode: %{public}s', JSON.stringify(error));
    }
    abilityAccessCtrl.createAtManager()
      .requestPermissionsFromUser(this.context, [READ_PASTEBOARD])
      .then((result) => {
        if (result.authResults.length !== 1) {
          hilog.error(LOG_DOMAIN, LOG_TAG, 'Pasteboard permission request returned an unexpected result');
          return;
        }
        const grantStatus = result.authResults[0];
        if (grantStatus === 0) {
          hilog.info(LOG_DOMAIN, LOG_TAG, 'Pasteboard read permission granted');
        } else {
          hilog.warn(LOG_DOMAIN, LOG_TAG, 'Pasteboard read permission denied: %{public}d', grantStatus);
        }
      })
      .catch((error: Error) => {
        hilog.error(LOG_DOMAIN, LOG_TAG, 'Failed to request pasteboard permission: %{public}s', JSON.stringify(error));
      });
    hilog.info(LOG_DOMAIN, LOG_TAG, 'Zed ability created');
    this.handleNotificationWant(want);
  }

  onNewWant(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    this.handleNotificationWant(want);
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    configureWindowStage(windowStage);
    windowStage.loadContent('pages/Index', (error) => {
      if (error.code) {
        hilog.error(LOG_DOMAIN, LOG_TAG, 'Failed to load Zed surface: %{public}s', JSON.stringify(error));
        return;
      }
      hilog.info(LOG_DOMAIN, LOG_TAG, 'Zed ArkUI surface loaded');
    });
  }

  onForeground(): void {
    hilog.info(LOG_DOMAIN, LOG_TAG, 'Zed entered foreground');
  }

  onBackground(): void {
    hilog.info(LOG_DOMAIN, LOG_TAG, 'Zed entered background');
  }

  onDestroy(): void {
    hilog.info(LOG_DOMAIN, LOG_TAG, 'Zed ability destroyed');
  }
}
