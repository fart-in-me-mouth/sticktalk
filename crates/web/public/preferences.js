// Nonsecret device preferences only. No account, message, draft or token storage.
const defaults = { density: 'cozy', fontScale: 1, underlineLinks: false, linkPreviews: true, plainNames: false, reduceCosmetics: false };
export function normalizePreferences(value = {}) {
  return { density: value?.density === 'compact' ? 'compact' : 'cozy',
    fontScale: [0.8,0.9,1,1.1,1.25,1.5].includes(Number(value?.fontScale)) ? Number(value.fontScale) : 1,
    underlineLinks: value?.underlineLinks === true,
    linkPreviews: value?.linkPreviews !== false,
    // Profiles (styles/profiles.css): other people's name styles and cosmetics.
    plainNames: value?.plainNames === true,
    reduceCosmetics: value?.reduceCosmetics === true };
}
let preferences = { ...defaults };
export function loadPreferences() {
  try {
    const raw = localStorage.getItem('sticktalk-appearance');
    if(raw && raw.length <= 1024) preferences = normalizePreferences(JSON.parse(raw));
  } catch { /* Storage may be disabled in a private browser. */ }
  applyPreferences();
}
export function applyPreferences() {
  const root=document.documentElement;
  root.dataset.density=preferences.density;
  root.dataset.underlineLinks=String(preferences.underlineLinks);
  root.dataset.linkPreviews=String(preferences.linkPreviews);
  root.dataset.plainNames=String(preferences.plainNames);
  root.dataset.reduceCosmetics=String(preferences.reduceCosmetics);
  root.style.setProperty('--chat-font-size',`${15*preferences.fontScale}px`);
  document.querySelectorAll('[data-preference]').forEach(field=>{
    const value=preferences[field.dataset.preference];
    if(field.type==='checkbox')field.checked=value; else field.value=String(value);
  });
}
export function setPreference(field) {
  const key=field.dataset.preference;
  if(!Object.hasOwn(defaults,key))return;
  preferences=normalizePreferences({...preferences,[key]:field.type==='checkbox'?field.checked:field.value});
  try { localStorage.setItem('sticktalk-appearance',JSON.stringify(preferences)); } catch {}
  applyPreferences();
}
