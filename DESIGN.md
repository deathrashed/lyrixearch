# DESIGN.md - Lyrixearch

## Direction

- Artifact: desktop data tool. Track list with cover art, an inspector (Lyrics, Edit, Web, Info), a status line, and a separate Settings window.
- Audience: people who curate large local music libraries and care about their tags. Primary action: fetch missing lyrics and embed them.
- Character: native and quiet, like a well-made macOS utility (reference: the owner's FluxDown app). The brand shows in the details: the green LED from the icon is the only accent, and the status lamps are coloured.
- Mode: Operate. Scanability, consistency and native expectations outrank expression.
- Constraints: Tauri 2 webview (WKWebView / WebView2 / WebKitGTK), works offline, plain HTML/CSS/JS with no build step, WCAG 2.2 AA.

## Typography

- One family: the platform's interface font (`-apple-system`, Segoe UI, Cantarell / Noto Sans, then CJK fallbacks). Nothing is bundled.
- Monospace (`ui-monospace`) only for the session log and code paths.
- Sentence case everywhere. No uppercase labels, no letter-spacing tricks.
- Sizes: 11.5px (secondary, counts, tags), 13px (body, controls, rows), 15px (panel and window titles), 18px (empty-state headings). Lyrics: 14px by default, 11-28px in Appearance.
- Numbers use tabular figures.

## Color

Neutral greys carry the interface. One accent marks the main action, the selection and focus. Status lamps carry meaning.

| role | dark | light |
|---|---|---|
| bg (list area, sidebars) | #1c1c1e | #f8f9fa |
| surface (panels, header, footer) | #242426 | #ffffff |
| surface-2 (controls, cards, menus) | #2c2c2e | #f1f3f5 |
| surface-3 (hover, selected tab) | #3a3a3c | #e7e9ec |
| border | #353537 | #e4e4e7 |
| border-strong | #48484a | #c9c9cf |
| text | #f5f5f7 | #09090b |
| muted text | #a1a1a6 | #5f5f68 |
| faint text | #8e8e93 | #6b6b74 |

- Accent: brand green #65E51C by default. In dark mode it is oklch(0.817 0.245 138). In light mode lamps use oklch(0.57 0.18 138) and text uses oklch(0.48 0.14 138). Appearance also offers blue, purple, amber and pink. The accent is used only for the Fetch button, the selection tint (16%), the focus outline, links, switches and sliders.
- Status colours are fixed and never follow the accent: embedded / ready is green (`--ok`, the brand green), missing is amber, found is blue, not found is purple, error is red. Every lamp has a word next to it.
- Contrast: body and secondary text reach at least 4.5:1 on their surfaces in both themes. Lamps, focus outlines and control edges reach at least 3:1.

## Shape, space, depth

- Radius: 6px for controls and panels; 8px for cards, menus and toasts; 10px for sheets. Lamps, counts and switches are pills.
- Spacing: 4px base (4 / 8 / 12 / 16 / 24 / 32).
- Depth: flat surfaces separated by 1px borders. Only floating elements (menus, toasts, sheets) get a soft shadow.

## Layout

- Main window: header (52px), then the list and inspector split by a resizable divider, then the status line (30px).
- Header: brand, filter, Import menu, Library menu, Fetch with its count (the only filled button), theme toggle, Settings.
- Filter bar: pills with a lamp, a label and a count (All, Missing, Embedded, Found, Not found, Error). The selected pill gets an accent tint, like a selected sidebar item. The list handles one task at a time, so there is no sidebar.
- List bar: selection on the left; Search, Embed, LRC files, then one overflow menu.
- Rows: 72px tall with a 56px cover (duration badge bottom-right, lamp top-left), then file name, artist and title. The state is written out at the right, with two quiet icon buttons. A selected row gets the accent tint.
- Inspector: title with status and format tags, the player, then pill tabs Lyrics / Edit / Web / Info. Info is label / value rows, with the folder in the accent colour. Toolbar: Copy, Edit, Search again and an overflow menu, with text size on the right.
- Status line: a lamp and "Ready" when idle. While searching it shows "Searching" with done/total, the current track and source, the found count and a thin rounded progress bar. When the search ends it shows "Done" with the result for a few seconds. A bot check waiting for the person also shows here, with Verify and Skip. Counts appear only in the filter bar.
- Settings window (880x620): a section sidebar (General, Sources, Files, Appearance, Plugins, About), a title and subtitle, then cards of rows. Each row has a title, a one-line description and a control on the right. Changes save at once; there is no Save button.

## Components

- Buttons: quiet grey fill (surface-2) with a 1px border; hover is one step lighter. The primary button in sheets has an accent tint with accent text. Fetch is solid accent. Ghost buttons are text only. Heights: 30px in the header, 26px in toolbars.
- Menus: anchored to their button, never full width. Rows are 30px with an accent-tint hover. The destructive item comes last, in red, after a divider line.
- Segmented controls: a small track with the selected option raised.
- Switches and sliders use the accent.
- Empty states: a quiet icon, a heading and one sentence, then the action, centred.
- Focus: 2px accent outline with a 1px offset, on every control.

## Appearance settings

- Theme: System (default), Dark or Light. Applies to both windows; on System it follows the OS.
- Accent colour: green (default), blue, purple, amber or pink. Status lamps keep their own colours.
- Interface scale: 90 / 100 / 110 / 125% for the main window. The web panel's position scales with it.
- Lyrics text size: 11-28px.
- Stored in the webview's local storage, which both windows share. A change in one window reaches the other through the storage event.

## Menu bar / tray

- One icon: the symbolic "LS" mark from the brand package. On macOS it is a template image, tinted by the system. Following Bjango's "Designing menu bar extras", it has no padding and about the weight of a 16 pt circle: letters 10 pt tall, about 20 pt wide. Elsewhere it is the dark-panel or light-panel PNG, matching the system theme.
- The search state is not drawn in the icon. It is the first, disabled line of the menu (Ready / Fetching 3 of 20 · 1 found / Done · …) and the tooltip.
- Menu: status, Show Lyrixearch, Fetch (brings the window forward and asks first, like the button), Stop (enabled while fetching), Quit.

## Wording

One term per concept:

| concept | word |
|---|---|
| add tracks | Import (Files…, Folder…) |
| search everything listed and write what is found | Fetch |
| search the selection without writing | Search |
| write found lyrics into the files | Embed |
| no lyrics yet | Missing |
| no source had them | Not found |
| take tracks out of the list (files untouched) | Remove from list / Clear embedded from list |
| delete lyrics inside the files | Delete lyrics from file(s)… |
| reveal a file | Show in Finder / Show in Explorer / Show in file manager |

## Motion

- 120-260ms, ease-out, opacity and transform only. Menus and sheets fade in and scale up by 2%; toasts fade.
- The searching lamp pulses. Rows, filters and tabs never animate.
- With reduced motion: no pulse, plain fades only.

## Iconography

- Lucide icons (inline SVG, 24px grid, 2px stroke, round caps), shared by both windows through `ui/shared.js`.
- App icon: the approved artwork package v6 (charcoal speaker faceplate, green magnifier over lyric lines, green LED), used for the bundle, the window, the header and the About section.

## Tokens

`crates/gui/ui/style.css` (`:root` and `[data-theme="light"]`). `ui/shared.js` sets the accent at runtime from the Appearance setting.

## Changelog

- 2026-11-26: native direction, after the owner's FluxDown reference. System font, neutral greys, sentence case, pill filters and tabs, label/value Info, Settings in its own window with live saving, Appearance (theme, accent, scale, lyrics size), symbolic menu bar icon, and status colours that no longer follow the accent. Replaces the earlier studio-rack direction (Sora uppercase labels, grille texture, bundled fonts).
- 2026-11-26: Import menu, Fetch, Not found, one status line in the footer, overflow menus, hidden-browser bot checks, Use selection in the Web tab, brand package v6 and tray icon.
- 2026-10-14: first version (studio-rack direction).
