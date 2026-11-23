// Text to speech (docs/features/themes.md) with the browser's own speech
// synthesis. Nothing is created until something is spoken. Rust decides what
// to say and when (never in Do Not Disturb or while deafened in a call).
// Tests replace the speaker with `globalThis.__sticktalkSpeech = (text, rate) => ...`.
const MAX_QUEUE = 4;
export function speak(text, rate) {
  const said = String(text).slice(0, 400);
  if (!said.trim()) return;
  if (typeof globalThis.__sticktalkSpeech === 'function') { globalThis.__sticktalkSpeech(said, rate); return; }
  const synth = globalThis.speechSynthesis;
  if (!synth || typeof SpeechSynthesisUtterance !== 'function') return;
  // A backlog is dropped rather than read out minutes late.
  if (synth.pending && pendingCount >= MAX_QUEUE) synth.cancel();
  const u = new SpeechSynthesisUtterance(said);
  u.rate = Math.min(2, Math.max(0.5, Number(rate) || 1));
  pendingCount++;
  u.onend = u.onerror = () => { pendingCount = Math.max(0, pendingCount - 1); };
  synth.speak(u);
}
let pendingCount = 0;
export function stopSpeaking() {
  pendingCount = 0;
  if (typeof globalThis.__sticktalkSpeech === 'function') return;
  globalThis.speechSynthesis?.cancel();
}
/// Deafened in a call (media.js keeps this current).
export function deafened() {
  return globalThis.__sticktalkDeafened === true;
}
