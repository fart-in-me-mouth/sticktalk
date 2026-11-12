# Asset provenance

No Discord logos, names, or artwork are included. StickTalk uses an original neutral layout and wordmark composed in the app.

- **Inter**, Rasmus Andersson: [rsms/inter](https://github.com/rsms/inter), SIL Open Font License 1.1, copied at `assets/fonts/Inter-OFL.txt`. Source `InterVariable.ttf`; `tools/subset_font.py` produces Latin/punctuation static 400/500/600 faces, renamed **StickTalk UI** to avoid the reserved font name. The three original compact weights total ~387 KiB; the review adds the italic faces listed below. Variable sources are not linked.
- **Lucide** SVG icon subset: [lucide-icons/lucide](https://github.com/lucide-icons/lucide), ISC, copied at `assets/icons/LICENSE`. A consistent subset is embedded and recolored through theme tokens. Aliases are mapped in `icons.rs`.
- **Twemoji** emoji PNG subset: Twitter and contributors, [jdecked/twemoji](https://github.com/jdecked/twemoji). Graphics are [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), license at `assets/emoji/LICENSE-CC-BY-4.0`. Common reaction/picker artwork is rendered as images; other typed emoji uses Windows fallback. Attribution also appears in Appearance/About. The subset is not a claim of complete Unicode emoji coverage.
- **Mock reading-corner photograph**, PNW Production: [Lamp, Laptop and Plant on Table](https://www.pexels.com/photo/lamp-laptop-and-plant-on-table-8250980/), [Pexels License](https://www.pexels.com/license/). The 256×170 JPEG at `assets/mock-workspace.jpg` is used only for mock screenshots/test data. No external image request is required at runtime in mock mode. The thumbnail download used `images.pexels.com/photos/8250980/pexels-photo-8250980.jpeg?auto=compress&cs=tinysrgb&w=256&h=170&fit=crop`.


## Added in lead-review revision

- **JetBrains Mono Regular**, JetBrains: [upstream](https://github.com/JetBrains/JetBrainsMono), SIL OFL 1.1 (`assets/fonts/JetBrainsMono-OFL.txt`). Latin/punctuation subset, renamed **StickTalk Mono**; used only for code. Inter italic and semibold italic subsets are **StickTalk UI** under the existing Inter OFL. Variable sources are not embedded.
- **Google multicolor G**: official [pre-approved sign-in assets](https://developers.google.com/static/identity/images/signin-assets.zip), used following [Google's branding guidelines](https://developers.google.com/identity/branding-guidelines). The approved gradient icon is kept at its original aspect/colors on white, inside `assets/google-g.svg`. That SVG embeds the official PNG fallback because this native SVG engine cannot faithfully render the download's `foreignObject`/conic-gradient SVG. It is not a recolored Lucide icon or a hand-drawn replacement. StickTalk UI's bundled Inter is retained for button labels.
- **Upload circle**: original SVG dashed outline plus the Lucide camera geometry (ISC attribution above); its Upload label is native Inter, not an SVG fallback font.
