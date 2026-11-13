export async function subscribe(permission) {
  if ((await permission) !== "granted")
    throw Error(
      "Notifications were not enabled. You can enable them later in browser settings.",
    );
  const registration = await navigator.serviceWorker.ready;
  const response = await fetch("/api/v1/push/key", {
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) throw Error("Push is unavailable on this server.");
  const { public_key } = await response.json();
  const key = Uint8Array.from(
    atob(public_key.replaceAll("-", "+").replaceAll("_", "/")),
    (c) => c.charCodeAt(0),
  );
  const subscription =
    (await registration.pushManager.getSubscription()) ??
    (await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: key,
    }));
  const { endpoint, keys } = subscription.toJSON();
  const csrf = document.cookie
    .split(";")
    .map((v) => v.trim())
    .find((v) => v.startsWith("__Host-sticktalk_csrf="))
    ?.split("=")
    .slice(1)
    .join("=");
  const saved = await fetch("/api/v1/push/subscriptions", {
    method: "POST",
    credentials: "same-origin",
    headers: {
      "Content-Type": "application/json",
      "X-StickTalk-CSRF": csrf ?? "",
    },
    body: JSON.stringify({ endpoint, p256dh: keys.p256dh, auth: keys.auth }),
  });
  if (!saved.ok) throw Error("Could not save notification subscription.");
}
