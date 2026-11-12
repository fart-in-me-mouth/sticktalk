# GIF picker (owner requirement): works like Discord's

## UX
- A **GIF button** in the composer, between the attach and emoji buttons. It opens a popover above the composer: about 420×460, bg.elevated, radius 8.
- **Tabs:** GIFs, Favorites (starred), and later Stickers/Emoji.
- **Default view:** a grid of trending categories, each tile a looping preview with its label.
- **Search box:** debounced by 250 ms, results in a 2-column masonry grid with infinite scroll.
- **Hovering a GIF** shows a star icon to add or remove it from Favorites. Favorites are stored per account on the server.
- **Clicking a GIF sends it right away** as its own message. The message body is the GIF URL, rendered inline as the GIF with no link text, the way Discord does it.
- **Attribution:** a "Powered by <provider>" label in the picker footer, as the provider's terms require.
- **Keyboard:** Esc closes the picker. Arrow keys and Enter pick a result.

## Provider and privacy
- The **server proxies all GIF search requests.** The API key lives only on the server, and users' IPs are never sent to the GIF provider.
- The server caches trending results and searches (in memory with a TTL, e.g. 10 minutes) and rate-limits per user.
- Use a provider abstraction (`GifProvider` trait). The implementer must check the **current** status and terms of the candidate providers (Tenor, GIPHY, KLIPY) before choosing one. Pick one that is free, still available, and allows this use. Document the choice and how to get a free API key in `deploy/README.md`.
- If no key is configured, the server reports `gifs: false` in `/api/meta` and the client hides the GIF button.

## Playback must stay light (top product priority)
- Ask the provider for **small MP4/WebM or animated WebP preview renditions** in the picker, never full-size GIFs. In chat, use the medium rendition.
- Animate only what is **visible on screen**, and only while the window is focused. Pause animations that are offscreen or when the window is minimized or unfocused. Stop all timers when nothing is animating.
- Add a setting, "Play GIFs automatically": Always / On hover / Never. Default is Always.
- Decode off the UI thread. Cap decoded frame memory per GIF. Use an LRU cache for decoded frames and a disk cache for the files.
- Uploaded animated GIF/WebP attachments use the same playback rules.

## Proto / server
- New REST endpoints, additive only:
  - `GET /api/gifs/trending`
  - `GET /api/gifs/search?q=&cursor=`
  - `GET /api/gifs/categories`
  - favorites CRUD
- Responses contain only the fields the client needs: id, preview URL and dimensions, send URL and dimensions, and the title for accessibility.
