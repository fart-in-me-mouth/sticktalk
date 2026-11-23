// Finds colors written into the web app's sources instead of taken from theme
// tokens (docs/features/themes.md, "Theme-correct surfaces"). Run after every
// merge:
//
//   node crates/web/scripts/theme-lint.mjs          # exit 1 on a new hard-coded color
//   node crates/web/scripts/theme-lint.mjs --all    # also list the allowed ones
//
// A color is allowed only where it is not UI chrome, each for a stated reason:
// - token definitions (custom properties) in styles/app.css;
// - a fallback inside var(--token, #...) (before the token sheet loads);
// - mask images (#000 there means "opaque", not a color you see);
// - black shadows (a shadow is dark in every theme);
// - artwork between `theme-lint: artwork` and `theme-lint: end` comments
//   (cosmetics, confetti: content, the same in every theme);
// - scrims and media letterboxes: bg-black/NN, and bg-black or text-white on
//   video and image overlays (a photo is not themed);
// - the entries in theme-lint-allow.json, each with its reason.
import { readFile, readdir } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

const web = resolve(new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1"));
const all = process.argv.includes("--all");
const allow = JSON.parse(await readFile(join(web, "scripts/theme-lint-allow.json"), "utf8"));

async function walk(dir, test) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await walk(path, test)));
    else if (test(entry.name)) out.push(path);
  }
  return out;
}
const files = [
  ...(await walk(join(web, "styles"), (n) => n.endsWith(".css"))),
  ...(await walk(join(web, "src/live"), (n) => n.endsWith(".rs"))),
  ...(await walk(join(web, "public"), (n) => n.endsWith(".js") && !/^(mock-|platform\.js)/.test(n)))
    // Activity frames (public/activities/) are sandboxed content apps, themed
    // through the tokens the SDK hands them, with their own artwork colors.
    .filter((p) => !/[\\/](vendor|runtime|cosmetics|stickers|emoji|activities)[\\/]/.test(p)),
  join(web, "index.html"),
];

const palette = "slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose";
const patterns = [
  // Hex colors (#rgb, #rrggbb, #rrggbbaa); a Rust format string like "#{:06x}" is not one.
  { kind: "hex", re: /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![0-9a-zA-Z_-])/g },
  // Literal rgb()/hsl()/oklch() (relative colors "from var(...)" are fine).
  { kind: "function", re: /\b(?:rgba?|hsla?|oklch|oklab|lab|lch|hwb)\((?!from )[^)]*\)/g },
  // Tailwind color utilities that are not tokens.
  { kind: "utility", re: new RegExp(`\\b(?:bg|text|border|ring|outline|fill|stroke|from|via|to|decoration|shadow|accent|caret|divide|placeholder)-(?:white|black|(?:${palette})-\\d{2,3}|\\[(?:#|rgb|hsl)[^\\]]*\\])(?:/\\d+)?`, "g") },
  // Named colors as CSS values.
  { kind: "named", re: /(?:^|[\s:,(])(?:white|black|red|green|blue|yellow|orange|purple|gray|grey|pink)(?=\s*[;,)!}]|\s*$)/g },
];

const findings = [];
for (const file of files) {
  const rel = relative(web, file).replaceAll("\\", "/");
  const text = await readFile(file, "utf8");
  let artwork = false;
  text.split(/\r?\n/).forEach((line, i) => {
    if (/theme-lint: artwork/.test(line)) artwork = true;
    if (/theme-lint: end/.test(line)) { artwork = false; return; }
    // Comments in CSS/JS/Rust are prose.
    const code = line.replace(/\/\*.*?\*\/|^\s*\/\/.*$|^\s*\*.*$|\s\/\/\s.*$/g, "");
    for (const { kind, re } of patterns) {
      for (const m of code.matchAll(re)) {
        const match = m[0].trim();
        if (kind === "named" && rel.endsWith(".rs")) continue; // prose in Rust strings
        if (kind === "hex" && /^#[0-9]{3,6}$/.test(match) && !/color|background|fill|stroke|border|shadow/i.test(code)) continue; // "#123" issue numbers
        let reason = null;
        if (artwork) reason = "artwork region";
        else if (rel === "styles/app.css" && /^\s*--[\w-]+\s*:/.test(code)) reason = "token definition";
        else if (new RegExp(`var\\(--[\\w-]+,\\s*${match.replace(/[()[\]\\/.*+?^$|]/g, "\\$&")}`).test(code)) reason = "var() fallback";
        else if (/mask/i.test(code)) reason = "mask image";
        else if (/shadow-\[[^\]]*$/.test(code.slice(0, m.index)) || /(box|text)-shadow/.test(code) && /^#0{3,4}|^#0{6}/.test(match)) reason = "shadow (dark in every theme)";
        else if (/^bg-black\/\d+$/.test(match)) reason = "scrim";
        else if (/^(bg-black|text-white)$/.test(match) && /video|img|image|viewer|media|tile|gif|camera|thumbnail|poster|preview/i.test(code)) reason = "media overlay";
        else {
          const entry = allow.find((a) => a.file === rel && code.includes(a.contains) && (!a.match || a.match === match));
          if (entry) reason = entry.reason;
        }
        findings.push({ file: rel, line: i + 1, kind, match, reason });
      }
    }
  });
}
const bad = findings.filter((f) => !f.reason);
for (const f of all ? findings : bad)
  console.log(`${f.reason ? "allowed" : "ERROR  "} ${f.file}:${f.line}  ${f.kind}  ${f.match}${f.reason ? `  (${f.reason})` : ""}`);
console.log(`${files.length} files, ${findings.length} colors, ${findings.length - bad.length} allowed, ${bad.length} hard-coded.`);
if (bad.length) {
  console.log("Use a theme token (var(--...), bg-background, text-foreground, text-primary, ...) instead; see docs/features/themes.md.");
  process.exit(1);
}
