// Notification URLs are provided as decimal strings by the server. Never parse
// channel/message IDs through JavaScript Number (Snowflakes exceed 2^53).
self.addEventListener("push", (event) => {
  event.waitUntil(
    (async () => {
      let payload;
      try {
        payload = event.data.json();
      } catch {
        return;
      }
      let url;
      try {
        url = new URL(payload.url, self.location.origin);
      } catch {
        return;
      }
      if (url.origin !== self.location.origin) return;
      await self.registration.showNotification("StickTalk", {
        body: "You have unread messages.",
        icon: "/public/icons/icon-192.png",
        tag: url.pathname + url.search,
        data: { url: url.href },
      });
    })(),
  );
});
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    (async () => {
      const url = new URL(
        event.notification.data?.url ?? "/",
        self.location.origin,
      );
      if (url.origin !== self.location.origin) return;
      const windows = await self.clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      });
      const existing = windows.find(
        (c) => new URL(c.url).origin === url.origin,
      );
      if (existing) {
        await existing.navigate(url.href);
        await existing.focus();
      } else await self.clients.openWindow(url.href);
    })(),
  );
});
