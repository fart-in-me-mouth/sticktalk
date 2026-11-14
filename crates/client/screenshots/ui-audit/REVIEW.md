# Desktop visual evidence

Branch: `t3/desktop-ui-polish-audit`

## Coverage and inspection

- 190 distinct in-scope screen/state fixtures × 940×600, 1280×800, and 1920×1080: **570 offscreen PNGs**.
- 16 representative fixtures rendered at 150% application scale (1280×800 logical → 1920×1200 physical).
- Two real wgpu secondary-window captures at 100% and 150% application scale. Both windows were non-activating and closed after capture. Windows display settings were not changed.
- The initial matrix had 508 PNGs. Minimum-window autocomplete crashed before it could produce its baseline PNG; this is now covered by layout regressions.
- Every matrix image was inspected in contact sheets, with full-resolution checks for dense chrome, long names/messages, narrow settings, popouts, minimum-height login/notifications, wrapping reactions, picker/search errors, and image-viewer menus. New and revised states were inspected again after rendering.

The full local `baseline/`, `final/`, `scale-150/` matrices and contact sheets are ignored to avoid committing hundreds of redundant assets. Their final dimensions and PNG-byte SHA-256 hashes are recorded in `capture-inventory.json`. The ten selected full-resolution comparisons below **are committed**.

## Before / after pairs

Each filename is identical under `before/` and `after/`:

1. `01-audit-long-names-940x600.png` — measured ellipsis and bounded chrome.
2. `02-audit-many-reactions-940x600.png` — wrapping reaction rows.
3. `03-audit-quick-long-940x600.png` — fixed search field and two-line results.
4. `04-emoji-picker-940x600.png` — bounded picker with visible controls.
5. `05-change-server-940x600.png` — configuration inside the login card.
6. `06-guild-settings-role-display-940x600.png` — role navigation and single-line tabs.
7. `07-audit-image-error-1280x800.png` — centered failure/retry state.
8. `08-audit-search-empty-1280x800.png` — scope/count and empty-state recovery.
9. `09-notification-settings-muted-940x600.png` — complete duration row and fixed Done footer.
10. `10-audit-many-servers-1280x800.png` — fixed Home/Add Server, scrolling server icons.

## Reproduce

See `docs/UI_AUDIT_REPORT.md` for the Rust commands. Generate inspection sheets with:

```powershell
python crates/client/tools/ui_audit_contact_sheets.py <captures> <sheets> --pattern '*940x600.png'
python crates/client/tools/ui_audit_evidence.py
```

The evidence indexer requires the baseline/final/scaled/native PNGs at the paths above; it checks complete three-size coverage and retains the comparison pairs. It never edits source images.

## Native test limitation

The approved native input helper failed to connect (`failed to connect native pipe`, OS error 2), including the documented retry/reinitialization. Therefore real pointer hover, drag-resize, right-click, physical keyboard traversal, and OS-DPI switching are **not verified**. Native captures, event/layout regression tests, and application-scale captures are evidence of rendering, not a substitute claim of those interactions.

`native-idle-100.json` records a 60-second debug mock idle sample: **0.078% of one CPU core**, **140.8 MiB working set**, **165.6 MiB private bytes**. RAM exceeds the ~100 MB goal; no RAM improvement is claimed.
