# Display-name fonts

Fonts a person can choose for their display name (`NameFont` in
`crates/proto/src/profiles.rs`). Each file is the Latin subset published by
Fontsource (`@fontsource/<name>@5`, `files/<name>-latin-<weight>-normal.woff2`).
The four fonts without a Reserved Font Name were subset again with fontTools
to U+0020-007E, U+00A0-00FF, U+2018-201F and U+2026 (`kern`, `liga`, `calt`
kept). Press Start 2P and Playfair Display have Reserved Font Names, so their
files are Fontsource's, unmodified. They are declared in
`styles/profiles.css` and are fetched by the browser only when a name in that
font is on screen. The service worker never precaches them.

All are licensed under the SIL Open Font License 1.1; the full license text
with each copyright notice is in the `*-LICENSE.txt` file beside each font.

| Style | Font | Weight | File | Copyright |
|---|---|---|---|---|
| Script | Pacifico | 400 | `pacifico-latin-400.woff2` | 2018 The Pacifico Project Authors |
| Pixel | Press Start 2P | 400 | `press-start-2p-latin-400.woff2` | 2012 The Press Start 2P Project Authors (Reserved Font Name "Press Start 2P") |
| Future | Orbitron | 700 | `orbitron-latin-700.woff2` | 2018 The Orbitron Project Authors |
| Serif | Playfair Display | 700 | `playfair-display-latin-700.woff2` | 2017 The Playfair Display Project Authors (Reserved Font Name "Playfair Display") |
| Handwritten | Caveat | 700 | `caveat-latin-700.woff2` | 2014 The Caveat Project Authors |
| Display | Bungee | 400 | `bungee-latin-400.woff2` | 2023 The Bungee Project Authors |

The CSS family names (`"ST Script"`, `"ST Pixel"`, ...) are local aliases
used only to select the files; the fonts' own names are unchanged.
