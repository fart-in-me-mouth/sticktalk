// The built-in "connect to a server" page. It talks only to the app itself:
// `state` and `connect` are answered by the shell, never by the network.
(() => {
  const form = document.getElementById("form");
  const address = document.getElementById("address");
  const error = document.getElementById("error");
  const submit = document.getElementById("submit");
  const failed = "Something went wrong. Try again.";
  let token = "";
  let label = "Connect";

  const show = (message) => {
    error.textContent = message || "";
    error.hidden = !message;
    address.setAttribute("aria-invalid", message ? "true" : "false");
  };
  const busy = (on) => {
    submit.disabled = on;
    address.readOnly = on;
    submit.textContent = on ? "Connecting…" : label;
  };

  fetch("state", { cache: "no-store" })
    .then((response) => response.json())
    .then((state) => {
      token = state.token;
      address.value = state.address || "";
      document.getElementById("version").textContent = "StickTalk " + state.version;
      if (state.error) {
        document.getElementById("title").textContent = "Can't reach your server";
        document.getElementById("subtitle").textContent =
          "Try again, or enter a different address.";
        label = "Retry";
        show(state.error);
      }
      busy(false);
      address.focus();
      address.select();
    })
    .catch(() => show("This page could not start. Close StickTalk and open it again."));

  address.addEventListener("input", () => {
    if (label !== "Connect") {
      label = "Connect";
      submit.textContent = label;
    }
    show("");
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (submit.disabled) return;
    show("");
    busy(true);
    try {
      const response = await fetch("connect", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ token, address: address.value }),
      });
      const result = await response.json();
      if (result.ok) {
        // Stay "Connecting…" until the server's page replaces this one.
        location.replace(result.url);
        return;
      }
      show(result.error || failed);
    } catch {
      show(failed);
    }
    busy(false);
    address.focus();
  });
})();
