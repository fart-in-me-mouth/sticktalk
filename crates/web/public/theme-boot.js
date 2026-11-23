// Runs in <head>, before the first paint (a classic, blocking script): puts
// back the theme, accessibility attributes and app icon that preferences.js
// last applied, so the login page and the skeleton never flash the default
// theme. Reads only the nonsecret snapshot preferences.js writes.
(() => {
  let boot = null;
  try {
    const raw = localStorage.getItem('sticktalk-boot');
    if (raw && raw.length <= 24576) boot = JSON.parse(raw);
  } catch { return; }
  if (!boot || typeof boot !== 'object') return;
  const root = document.documentElement;
  const safe = /^[A-Za-z0-9 #%().,_-]{0,64}$/;
  if (boot.attrs && typeof boot.attrs === 'object')
    for (const [k, v] of Object.entries(boot.attrs))
      if (/^[a-zA-Z]{1,32}$/.test(k) && typeof v === 'string' && safe.test(v)) root.dataset[k] = v;
  if (boot.vars && typeof boot.vars === 'object')
    for (const [k, v] of Object.entries(boot.vars))
      if (/^--[a-z-]{1,32}$/.test(k) && typeof v === 'string' && safe.test(v)) root.style.setProperty(k, v);
  if (typeof boot.zoom === 'number' && boot.zoom !== 1 && boot.zoom >= 0.5 && boot.zoom <= 2) root.style.zoom = String(boot.zoom);
  // The derived theme: custom properties only, in one constructed sheet that
  // preferences.js keeps using. A value that could close the rule is refused.
  if (typeof boot.css === 'string' && boot.css && !/[{}<@\\]/.test(boot.css)) {
    try {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(`:root:root:root{${boot.css}}`);
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
      globalThis.__sticktalkThemeSheet = sheet;
    } catch {}
  }
  const color = typeof boot.color === 'string' && /^#[0-9a-fA-F]{6}$/.test(boot.color) ? boot.color : null;
  if (color) document.querySelector('meta[name="theme-color"]')?.setAttribute('content', color);
  const icon = typeof boot.icon === 'string' && /^[a-z]{2,16}$/.test(boot.icon) && boot.icon !== 'mint' ? boot.icon : null;
  if (icon) {
    const href = `/public/app-icons/${icon}-192.png`;
    for (const link of document.querySelectorAll('link[rel~="icon"],link[rel="apple-touch-icon"]')) link.href = href;
    document.querySelector('link[rel="manifest"]')?.setAttribute('href', `/public/app-icons/${icon}.webmanifest`);
    // The login page's logo is parsed after this script.
    document.addEventListener('DOMContentLoaded', () => {
      for (const img of document.querySelectorAll('img[data-app-logo]')) img.src = href;
    }, { once: true });
  }
})();
