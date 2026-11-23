# Chat fonts

Fonts a person can choose for messages and the composer (Settings >
Appearance > Chat Font). Each file is the Latin subset published by
Fontsource (`@fontsource/<name>@5.3.0`, `files/<name>-latin-<weight>-normal.woff2`),
unmodified. They are declared in `styles/themes.css`; the browser fetches one
only when that font is chosen and text in it is on screen. The service worker
never precaches them.

Both are licensed under the SIL Open Font License 1.1; the full license text
with each copyright notice is in the `*-OFL.txt` file beside each font.

| Choice | Font | Weights | Copyright |
|---|---|---|---|
| Dyslexia-friendly | OpenDyslexic | 400, 700 | 2019 Abbie Gonzalez (Reserved Font Name "OpenDyslexic") |
| Hyperlegible | Atkinson Hyperlegible | 400, 700 | 2020 Braille Institute of America, Inc. |

The other choices use fonts already bundled (Inter, JetBrains Mono) or the
device's own (system sans-serif, serif).
