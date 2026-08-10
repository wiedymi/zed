export type NotificationResponseHandler = (tag: string, actionId?: string) => void;

class PendingNotificationResponse {
  tag: string;
  actionId: string | undefined;

  constructor(tag: string, actionId?: string) {
    this.tag = tag;
    this.actionId = actionId;
  }
}

let notificationResponseHandler: NotificationResponseHandler | undefined;
const pendingNotificationResponses: Array<PendingNotificationResponse> = [];

export function setNotificationResponseHandler(handler: NotificationResponseHandler | undefined): void {
  notificationResponseHandler = handler;
  if (!handler) {
    return;
  }
  for (const response of pendingNotificationResponses.splice(0)) {
    handler(response.tag, response.actionId);
  }
}

export function deliverNotificationResponse(tag: string, actionId?: string): void {
  const handler = notificationResponseHandler;
  if (handler) {
    handler(tag, actionId);
  } else {
    pendingNotificationResponses.push(new PendingNotificationResponse(tag, actionId));
  }
}
