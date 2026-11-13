// Progressive, genuinely interactive login. Download/compile the application
// WASM only for a recovered session, Google completion, or successful sign-in.
let register = false,
  loading = false;
const root = document.getElementById("boot");
const form = root.querySelector("form"),
  status = document.getElementById("boot-status");
// Never allow the fallback HTML form to serialize a password into a URL.
form.addEventListener("submit", (event) => event.preventDefault());
const hasCsrf = document.cookie
  .split(";")
  .some((v) => v.trim().startsWith("__Host-sticktalk_csrf="));
const signup = new URLSearchParams(location.search).get("google") === "signup";
async function loadApp() {
  if (loading) return;
  loading = true;
  const shell = document.getElementById('chat-skeleton');
  if (shell) root.replaceChildren(shell.content.cloneNode(true));
  // Recover while the core downloads/compiles, not after it. This transient
  // promise contains only the browser DTO (never a bearer credential).
  globalThis.__sticktalkSession = fetch('/api/v1/browser/session', {credentials:'same-origin', cache:'no-store'})
    .then(r => r.ok ? r.text() : '').catch(() => '');
  try {
    __WASM_BOOTSTRAP__;
  } catch {
    loading = false;
    root.replaceChildren(form);
    status.textContent =
      "Could not load the app. Check your connection and try again.";
  }
}
// Install the shell worker after the core has mounted (live.js). Installing
// here would race its precache against the first compileStreaming download.
if (new URLSearchParams(location.search).has("mock") || hasCsrf || signup) {
  form
    .querySelectorAll("input,button")
    .forEach((element) => (element.disabled = true));
  loadApp();
} else {
  fetch("/api/v1/browser/capabilities", {
    credentials: "same-origin",
    cache: "no-store",
  })
    .then((r) => (r.ok ? r.json() : null))
    .then((c) => {
      if (c?.google_redirect)
        document.getElementById("boot-google").hidden = false;
    })
    .catch(() => {
      status.textContent = "Server unavailable. Check your connection.";
    });
  root.querySelector("[data-live=register]").onclick = (e) => {
    register = !register;
    document.getElementById("boot-title").textContent = register
      ? "Create an account"
      : "Log in";
    document.getElementById("boot-submit").textContent = register
      ? "Create Account"
      : "Log In";
    document.getElementById("auth-password").autocomplete = register
      ? "new-password"
      : "current-password";
    e.target.textContent = register
      ? "Already have an account? Log in"
      : "Create an account";
  };
  form.onsubmit = async (e) => {
    e.preventDefault();
    const submit = document.getElementById("boot-submit");
    submit.disabled = true;
    status.textContent = "Signing in…";
    try {
      const username = document.getElementById("auth-username").value,
        password = document.getElementById("auth-password").value;
      const body = register
        ? { username, password, display_name: null }
        : { username, password };
      const response = await fetch(
        `/api/v1/browser/session/${register ? "register" : "login"}`,
        {
          method: "POST",
          credentials: "same-origin",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        },
      );
      if (!response.ok) {
        const error = await response.json();
        throw Error(error.message ?? "Could not sign in.");
      }
      // Never expose/persist a bearer credential. Rust recovers cookie session DTOs.
      await loadApp();
    } catch (error) {
      status.textContent = error.message;
    } finally {
      submit.disabled = false;
    }
  };
}
