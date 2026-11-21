// Progressive, genuinely interactive login. Download/compile the application
// WASM only for a recovered session, Google completion, or successful sign-in.
let register = false,
  loading = false;
const root = document.getElementById("boot");
const form = root.querySelector("form"),
  status = document.getElementById("boot-status");
const $ = (id) => document.getElementById(id);
// Login is usable before WASM, including when the iOS keyboard shrinks/pans
// only the visual viewport. The live host takes over these listeners on mount.
const viewport=()=>{
  document.documentElement.style.setProperty('--viewport-height',`${window.visualViewport?.height??innerHeight}px`);
  document.documentElement.style.setProperty('--viewport-offset',`${window.visualViewport?.offsetTop??0}px`);
};
window.visualViewport?.addEventListener('resize',viewport);
window.visualViewport?.addEventListener('scroll',viewport);
viewport();
// Never allow the fallback HTML form to serialize a password into a URL.
form.addEventListener("submit", (event) => event.preventDefault());
const hasCsrf = document.cookie
  .split(";")
  .some((v) => v.trim().startsWith("__Host-sticktalk_csrf="));
const query = new URLSearchParams(location.search);
const signup = query.get("google") === "signup";
// "Forgot your password?": the person who runs the server issues a reset code.
let reset = query.has("reset");
// The server answers with a code; say what it means here.
function explain(code) {
  if (reset)
    return {
      invalid_credentials: "That reset code isn’t valid for this username, or it has expired. Ask for a new one.",
      invalid_request: "Enter your username, the reset code exactly as you were given it, and a new password.",
      rate_limited: "Too many attempts. Wait a minute and try again.",
    }[code] ?? "Could not reset the password. Try again.";
  return {
    invalid_credentials: "Incorrect username or password.",
    username_taken: "That username is taken. Choose another.",
    invalid_request: register
      ? "Usernames are 3 to 32 letters, numbers or underscores, and a password is required."
      : "Incorrect username or password.",
    rate_limited: "Too many attempts. Wait a minute and try again.",
    unavailable: "The server is busy. Try again shortly.",
  }[code] ?? (register ? "Could not create the account. Try again." : "Could not log in. Try again.");
}
function say(text, error = false) {
  status.textContent = text;
  status.classList.toggle("text-destructive", error);
  status.classList.toggle("text-muted-foreground", !error);
  status.setAttribute("role", error ? "alert" : "status");
}
// One form, three purposes: log in, register, or set a new password with a code.
function show() {
  $("boot-title").textContent = reset ? "Reset your password" : register ? "Create an account" : "Welcome back!";
  $("boot-subtitle").textContent = reset
    ? "Ask the person who runs this server for a reset code. It works for 30 minutes."
    : register ? "Create your StickTalk account." : "Log in to catch up with your friends.";
  $("boot-submit").textContent = reset ? "Set New Password" : register ? "Create Account" : "Log In";
  $("auth-password").autocomplete = reset || register ? "new-password" : "current-password";
  $("boot-password-label").textContent = reset ? "New Password" : "Password";
  $("boot-code").hidden = !reset;
  $("boot-username-help").hidden = !register;
  $("boot-forgot").hidden = reset || register;
  $("boot-switch-copy").textContent = reset ? "Remembered it? " : register ? "Already have an account? " : "Need an account? ";
  root.querySelector("[data-live=register]").textContent = reset || register ? "Log in" : "Register";
  if (!reset) $("boot-google").hidden = !google;
  else $("boot-google").hidden = true;
}
let google = false;
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
    window.visualViewport?.removeEventListener('resize',viewport);
    window.visualViewport?.removeEventListener('scroll',viewport);
  } catch {
    loading = false;
    root.replaceChildren(form);
    form.querySelectorAll('input,button').forEach(element=>element.disabled=false);
    say("Could not load the app. Check your connection and try again.", true);
  }
}
// Install the shell worker after the core has mounted (live.js). Installing
// here would race its precache against the first compileStreaming download.
if (!reset && (query.has("mock") || hasCsrf || signup)) {
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
      google = !!c?.google_redirect;
      show();
    })
    .catch(() => {
      say("Can’t reach the server. Check your connection.", true);
    });
  show();
  root.querySelector("[data-live=register]").onclick = () => {
    if (reset) {
      reset = false;
      history.replaceState(history.state, "", location.pathname);
    } else register = !register;
    // A message about the other form no longer applies.
    say("");
    show();
  };
  $("boot-forgot").onclick = (e) => {
    e.preventDefault();
    reset = true;
    history.replaceState(history.state, "", "/?reset");
    say("");
    show();
  };
  form.onsubmit = async (e) => {
    e.preventDefault();
    const submit = $("boot-submit");
    submit.disabled = true;
    say(reset ? "Setting your new password…" : "Signing in…");
    try {
      const username = $("auth-username").value.trim(),
        password = $("auth-password").value;
      const [path, body] = reset
        ? ["auth/reset-password", { username, code: $("auth-code").value.trim(), new_password: password }]
        : register
          ? ["browser/session/register", { username, password, display_name: null }]
          : ["browser/session/login", { username, password }];
      let response;
      try {
        response = await fetch(`/api/v1/${path}`, {
          method: "POST",
          credentials: "same-origin",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        });
      } catch {
        throw Error("Can’t reach the server. Check your connection and try again.");
      }
      if (!response.ok) {
        const error = await response.json().catch(() => ({}));
        throw Error(explain(error.code));
      }
      if (reset) {
        reset = false;
        history.replaceState(history.state, "", location.pathname);
        $("auth-password").value = "";
        $("auth-code").value = "";
        show();
        say("Your password was changed. Log in with the new one.");
        return;
      }
      // Never expose/persist a bearer credential. Rust recovers cookie session DTOs.
      await loadApp();
    } catch (error) {
      say(error.message, true);
    } finally {
      submit.disabled = false;
    }
  };
}
