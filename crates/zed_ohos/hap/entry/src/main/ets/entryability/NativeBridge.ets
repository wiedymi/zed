export type NotificationResponseHandler = (tag: string, actionId?: string) => void;
export type OpenUrlsHandler = (urls: Array<string>) => void;

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
let openUrlsHandler: OpenUrlsHandler | undefined;
const pendingOpenUrls: Array<Array<string>> = [];

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

export function setOpenUrlsHandler(handler: OpenUrlsHandler | undefined): void {
  openUrlsHandler = handler;
  if (!handler) {
    return;
  }
  for (const urls of pendingOpenUrls.splice(0)) {
    handler(urls);
  }
}

export function deliverOpenUrls(urls: Array<string>): void {
  if (urls.length === 0) {
    return;
  }
  const handler = openUrlsHandler;
  if (handler) {
    handler(urls);
  } else {
    pendingOpenUrls.push(urls);
  }
}
