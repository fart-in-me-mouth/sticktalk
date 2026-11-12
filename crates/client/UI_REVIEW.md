# UI_SPEC §8 self-review — lead revision

48 states at 1280×800 and 1920×1080: **96 PNGs**. All were opened in paired review sheets; key auth, markdown, settings, modal and message-action states were also inspected at source resolution. These are the actual iced view tree, rendered with native tiny-skia **offscreen**, not browser mockups or OS screenshots. The final capture pass uses `sticktalk --mock --screenshots <dir>`; it never constructs an OS window. Dimensions/hashes are recorded in [manifest.json](screenshots/manifest.json).

## Checklist / fixes

- §1–§2: neutral palette, bundled Inter weights/italics, Lucide icons, narrow initial avatars, unchanged rail/sidebar/chat/member proportions. Full-screen settings use a right-aligned 218px nav block, bounded content, circular close and ESC.
- §3–§4: 2px mention inset and following-text spacing; attachment continuation no longer reserves an empty paragraph; 8px text-to-attachment spacing; 32px action toolbar floats 16px above group heads and fully above tight continuations, without covering reply/text; More and own Edit are present. Beginning header is visible in `channel-top` and short-channel markdown captures.
- §5–§6: shared styles/focus rings, terse empty/loading/disconnected states, #000 at 70% modal backdrop, right-aligned form footers, dashed upload circle and prefilled server name. Settings remain full-screen behind their subordinate modals/menus.
- §7–§7b: user-facing server wording, no marketing/tagline/security-store prose, no manual prose line breaks. Login primary labels centered/Semibold, link prefix muted at the field edge, blank password placeholder and register-only helper. The remaining Appearance attribution section is labeled Credits.
- Google enabled/disabled login, Register, username selection, linked/unlinked/Google-only methods and password editing inspected. Official multicolor mark is preserved on white. Masked links show the real URL in confirmation; spoilers have hidden/revealed captures. Fenced code is unwrapped in a horizontal scroller, with a separate monospace font.
- Each size checked for clipped/overflowing controls, name ellipsis, footer placement, title/baseline alignment, backdrop consistency, image aspect ratio and readable wrapping. Normal chat captures are at the present; a partial top row is expected scrolling, not an overflowing row.

- Scrolling additions: `scroll-5000` and `scroll-skeleton` checked at both sizes. Missing/fast-scroll placeholders are static circles plus two bars; visible rows preserve their reserved height, with no shimmer or idle redraw loop. All 24 latest paired review sheets were inspected, and markdown, own context menu and skeleton states were also opened at source resolution.

## Remaining caveats

- The context-menu + opens a separate picker, but it currently has only nine bundled emoji choices, not a full Unicode catalog/search. This part of A4 remains incomplete.

- Mark Unread is local to this client session: v1 only defines forward MarkRead, so there is no server-side backward/synced read mutation within the permitted scope.
- The Google SVG asset embeds the official PNG fallback because the native SVG engine does not support the approved source's conic-gradient/foreignObject construct; shape/colors/aspect are not recreated or recolored. Inter labels follow the app's authoritative typography rule.
- Optional category tracking is unsupported by iced's public Text API. Ellipsis remains character-budget based. Arbitrary Unicode emoji/CJK/RTL coverage is not exhaustively certified; bundled common Twemoji reactions are inspected.
- This is view-tree/layout QA plus unit/mock-flow tests, not a claim of exhaustive OS mouse, file-picker/drag-drop, assistive-technology, live Google, or real-server two-peer end-to-end certification. No real browser was opened by the revised tests. Native performance uses only the guarded non-activating secondary-monitor mode.
- Legacy screenshot scenario filenames use `guild` and `community-picker` for compatibility; the displayed UI says server.

## Screenshot matrix (both inspected)

| State | 1280×800 | 1920×1080 |
|---|---|---|
| add-friend | [PNG](screenshots/add-friend-1280x800.png) | [PNG](screenshots/add-friend-1920x1080.png) |
| change-server | [PNG](screenshots/change-server-1280x800.png) | [PNG](screenshots/change-server-1920x1080.png) |
| channel-top | [PNG](screenshots/channel-top-1280x800.png) | [PNG](screenshots/channel-top-1920x1080.png) |
| community-picker | [PNG](screenshots/community-picker-1280x800.png) | [PNG](screenshots/community-picker-1920x1080.png) |
| context | [PNG](screenshots/context-1280x800.png) | [PNG](screenshots/context-1920x1080.png) |
| context-own-message | [PNG](screenshots/context-own-message-1280x800.png) | [PNG](screenshots/context-own-message-1920x1080.png) |
| create-guild-form | [PNG](screenshots/create-guild-form-1280x800.png) | [PNG](screenshots/create-guild-form-1920x1080.png) |
| disconnected | [PNG](screenshots/disconnected-1280x800.png) | [PNG](screenshots/disconnected-1920x1080.png) |
| dm-chat | [PNG](screenshots/dm-chat-1280x800.png) | [PNG](screenshots/dm-chat-1920x1080.png) |
| edit-message | [PNG](screenshots/edit-message-1280x800.png) | [PNG](screenshots/edit-message-1920x1080.png) |
| empty-channel | [PNG](screenshots/empty-channel-1280x800.png) | [PNG](screenshots/empty-channel-1920x1080.png) |
| empty-home | [PNG](screenshots/empty-home-1280x800.png) | [PNG](screenshots/empty-home-1920x1080.png) |
| google-login | [PNG](screenshots/google-login-1280x800.png) | [PNG](screenshots/google-login-1920x1080.png) |
| group-chat | [PNG](screenshots/group-chat-1280x800.png) | [PNG](screenshots/group-chat-1920x1080.png) |
| guild-chat | [PNG](screenshots/guild-chat-1280x800.png) | [PNG](screenshots/guild-chat-1920x1080.png) |
| guild-settings-bans | [PNG](screenshots/guild-settings-bans-1280x800.png) | [PNG](screenshots/guild-settings-bans-1920x1080.png) |
| guild-settings-channels | [PNG](screenshots/guild-settings-channels-1280x800.png) | [PNG](screenshots/guild-settings-channels-1920x1080.png) |
| guild-settings-invites | [PNG](screenshots/guild-settings-invites-1280x800.png) | [PNG](screenshots/guild-settings-invites-1920x1080.png) |
| guild-settings-members | [PNG](screenshots/guild-settings-members-1280x800.png) | [PNG](screenshots/guild-settings-members-1920x1080.png) |
| guild-settings-overview | [PNG](screenshots/guild-settings-overview-1280x800.png) | [PNG](screenshots/guild-settings-overview-1920x1080.png) |
| home-all | [PNG](screenshots/home-all-1280x800.png) | [PNG](screenshots/home-all-1920x1080.png) |
| home-blocked | [PNG](screenshots/home-blocked-1280x800.png) | [PNG](screenshots/home-blocked-1920x1080.png) |
| home-online | [PNG](screenshots/home-online-1280x800.png) | [PNG](screenshots/home-online-1920x1080.png) |
| home-pending | [PNG](screenshots/home-pending-1280x800.png) | [PNG](screenshots/home-pending-1920x1080.png) |
| invite | [PNG](screenshots/invite-1280x800.png) | [PNG](screenshots/invite-1920x1080.png) |
| join-guild | [PNG](screenshots/join-guild-1280x800.png) | [PNG](screenshots/join-guild-1920x1080.png) |
| loading | [PNG](screenshots/loading-1280x800.png) | [PNG](screenshots/loading-1920x1080.png) |
| login | [PNG](screenshots/login-1280x800.png) | [PNG](screenshots/login-1920x1080.png) |
| markdown | [PNG](screenshots/markdown-1280x800.png) | [PNG](screenshots/markdown-1920x1080.png) |
| markdown-revealed | [PNG](screenshots/markdown-revealed-1280x800.png) | [PNG](screenshots/markdown-revealed-1920x1080.png) |
| masked-link-confirm | [PNG](screenshots/masked-link-confirm-1280x800.png) | [PNG](screenshots/masked-link-confirm-1920x1080.png) |
| member-menu | [PNG](screenshots/member-menu-1280x800.png) | [PNG](screenshots/member-menu-1920x1080.png) |
| mention-autocomplete | [PNG](screenshots/mention-autocomplete-1280x800.png) | [PNG](screenshots/mention-autocomplete-1920x1080.png) |
| message-hover | [PNG](screenshots/message-hover-1280x800.png) | [PNG](screenshots/message-hover-1920x1080.png) |
| message-hover-grouped | [PNG](screenshots/message-hover-grouped-1280x800.png) | [PNG](screenshots/message-hover-grouped-1920x1080.png) |
| message-hover-own | [PNG](screenshots/message-hover-own-1280x800.png) | [PNG](screenshots/message-hover-own-1920x1080.png) |
| password-settings | [PNG](screenshots/password-settings-1280x800.png) | [PNG](screenshots/password-settings-1920x1080.png) |
| pick-username | [PNG](screenshots/pick-username-1280x800.png) | [PNG](screenshots/pick-username-1920x1080.png) |
| register | [PNG](screenshots/register-1280x800.png) | [PNG](screenshots/register-1920x1080.png) |
| scroll-5000 | [PNG](screenshots/scroll-5000-1280x800.png) | [PNG](screenshots/scroll-5000-1920x1080.png) |
| scroll-skeleton | [PNG](screenshots/scroll-skeleton-1280x800.png) | [PNG](screenshots/scroll-skeleton-1920x1080.png) |
| sign-in-methods | [PNG](screenshots/sign-in-methods-1280x800.png) | [PNG](screenshots/sign-in-methods-1920x1080.png) |
| sign-in-methods-google-only | [PNG](screenshots/sign-in-methods-google-only-1280x800.png) | [PNG](screenshots/sign-in-methods-google-only-1920x1080.png) |
| sign-in-methods-unlinked | [PNG](screenshots/sign-in-methods-unlinked-1280x800.png) | [PNG](screenshots/sign-in-methods-unlinked-1920x1080.png) |
| toast | [PNG](screenshots/toast-1280x800.png) | [PNG](screenshots/toast-1920x1080.png) |
| user-settings-account | [PNG](screenshots/user-settings-account-1280x800.png) | [PNG](screenshots/user-settings-account-1920x1080.png) |
| user-settings-appearance | [PNG](screenshots/user-settings-appearance-1280x800.png) | [PNG](screenshots/user-settings-appearance-1920x1080.png) |
| user-settings-profile | [PNG](screenshots/user-settings-profile-1280x800.png) | [PNG](screenshots/user-settings-profile-1920x1080.png) |
