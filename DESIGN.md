# DESIGN.md - Lyrixearch

## Context (from discovery)

- Artifact type: desktop data tool (library table + detail inspector + settings), with an embedded browser panel
- Positioning: technical / utilitarian, with the warmth of audio hardware
- Audience: people who curate large local music libraries and care about tags | Primary action: find missing lyrics and embed them
- Adjectives: precise, tactile, nocturnal, dependable
- Visual word translations:
  - precise -> Lucide icons, tabular numerals, hairline edges, 4px grid
  - tactile -> silver "hardware" buttons that press down 1px, perforated-grille texture, silkscreen labels
  - nocturnal -> near-black charcoal base, off-white text, one lit accent instead of many colors
  - dependable -> every state has a lamp AND a word; nothing animates that the user repeats often
- Aesthetic essence (3 words): matte, rack-mounted, lit
- Single-minded proposition: a piece of studio gear that happens to run on your library
- References: admire the app icon supplied by the owner (charcoal faceplate, speaker grille, one green LED); Teenage Engineering and classic hi-fi faceplates for restraint and silkscreen labelling; Raycast for not animating frequent actions. Avoid: indigo-on-dark SaaS dashboards, glass cards.
- Mode: both (dark is the primary; light is a designed sibling) | Density: dense but not cramped
- Constraints: Tauri 2 webview (WKWebView / WebView2 / WebKitGTK), fully offline (fonts and icon bundled), plain HTML/CSS/JS with no build step, WCAG 2.2 AA

## Aesthetic

- Direction: studio rack hardware, derived from the app icon
- Defining trait: the window is one chassis divided by seams, not floating cards. Panels are flush; structure comes from 1px edges.
- Signature move: **lamps**. Every track state is a real LED (off / amber missing / amber blinking / green embedded / blue found / purple NA / red error), and progress is an LED bar-graph. The single green LED in the icon is echoed by the engine lamp in the top bar that lights while searching.
- Graphic device: perforated grille (dot matrix) used on the empty state and inspector placeholder, echoing the icon's speaker grille.

## Typography

- Wordmark: Inter 600 | source: Google Fonts via Fontsource (bundled) | license: OFL
- Display / headings / labels: Sora 600 | source: Google Fonts via Fontsource (bundled) | license: OFL
- Body / supporting: Source Sans 3 400, 500, 600 | source: Google Fonts via Fontsource (bundled) | license: OFL
- Mono: IBM Plex Mono 400, 500 | source: Google Fonts (bundled) | license: OFL
- Scale: ratio 1.25 (major third), base 14px

| step | size | line-height | use |
|------|------|-------------|-----|
| xs | 11.2px | 1.4 | silkscreen labels (uppercase, tracking 0.12em), counters |
| body | 15px | 1.5 | table, controls, lyrics (1.7 in the lyrics pane) |
| md | 17.5px | 1.3 | inspector title, dialog titles |
| lg | 21.9px | 1.25 | empty-state heading |

- Roles: Sora only for UPPERCASE silkscreen labels (state words, tags, section heads); Source Sans 3 for everything read, track titles included; Plex Mono only for numbers (times, counts, paths). Monospace never stands in for "technical".
- Weights: 400 / 500 / 600; weight never changes on hover or selection | Measure: lyrics pane ~55ch, left-aligned | Tracking: labels +0.12em, display 0
- CJK track names fall back to the system CJK face (stack ends in system fonts only as a fallback, never as the primary face).

## Color

- Strategy: audio hardware is neutral metal plus one signal lamp. A cool charcoal neutral (hue 260, chroma under 0.01) carries 90% of the UI; the only chromatic color is LED green, matching the icon. Deliberately nowhere near the indigo/violet band.
- Distribution: 60 neutral / 30 silver (controls, text) / 10 lamp colors
- Palette, dark (role -> OKLCH | hex approx):
  - bg: oklch(0.155 0.006 260) | #0f1012
  - surface (panels): oklch(0.19 0.007 260) | #16171a
  - surface-2 (inputs, raised): oklch(0.23 0.008 260) | #1e2024
  - surface-3 (hover, overlays): oklch(0.275 0.009 260) | #27292e
  - border: oklch(0.32 0.008 260) | #313338
  - border-strong: oklch(0.54 0.01 260) | #6a6d75
  - fg: oklch(0.93 0.004 260) | #e8e9ec
  - muted: oklch(0.72 0.01 260) | #a5a8b0
  - faint: oklch(0.62 0.01 260) | #82858d
  - silver (primary button fill): oklch(0.90 0.005 260) | #dcdde0 with text oklch(0.18 0.006 260)
  - led-green (brand green, the icon's LED): oklch(0.817 0.245 138) | #65E51C, 10.3:1 or better on every dark surface
  - led-amber: oklch(0.82 0.15 78) | #f0b94a
  - led-red: oklch(0.70 0.19 25) | #f2655a
- Palette, light (designed sibling, not inverted): bg oklch(0.955 0.004 260), surface oklch(0.985 0.003 260), surface-2 oklch(0.94 0.005 260), surface-3 oklch(0.90 0.006 260), border oklch(0.86 0.006 260), border-strong oklch(0.58 0.01 260), fg oklch(0.22 0.008 260), muted oklch(0.45 0.01 260), silver becomes a dark fill oklch(0.24 0.008 260) with light text, led-green oklch(0.57 0.18 138) for lamps and the focus ring (3.7:1), oklch(0.48 0.14 138) for green text (5.2:1 or better), call-to-action fill oklch(0.5 0.15 138) with near-white text (5.5:1), red text oklch(0.5 0.2 27), amber text oklch(0.48 0.11 70).
- Rules: no pure black or white; elevation in dark = lightness steps; borders are lighter than their surface; lamp colors are always paired with a text label.

## Spacing, radius, shadow

- Spacing base: 4px, scale 4 / 8 / 12 / 16 / 24 / 32 / 48. Tight inside groups (4-8), generous between regions (24+).
- Radius: 3px (controls, panels, sheets, tags) and pill (LEDs, switches). Nothing else.
- Shadow approach: defined edge only (1px border, plus a 1px inner top highlight on raised hardware controls). No drop shadows, no glow except the lit-LED ring, which is the signature.

## Layout and composition

- Grid: three flush columns (sidebar 224px | table, flexible | inspector 440px) under a 56px faceplate strip; seams are 1px edges, no gaps, no floating cards.
- Spacing rhythm: 4-8px inside controls and rows, 24px between sections.
- Signature layout move: the chassis (seams, not cards) plus the LED bar-graph meter running along the bottom edge of the faceplate strip.
- Density: dense (40px rows) | Scanning: left-to-right F pattern, lamp first.
- Responsive: desktop-first; min window 980px; under 1180px the faceplate buttons drop their text labels.

## Components and states

- Button hierarchy: primary = silver fill (Find missing); secondary = outlined (Add folder, Find & embed all); tertiary = text (Scan library, Clear selection). Hover lightens, active moves 1px down, focus ring, disabled at 45%, busy state through disabled plus the engine lamp and meter.
- Inputs: visible labels in settings; the filter and web fields carry aria-labels and a leading icon; validation inline; focus = ring.
- Tables: text left, numerals right with tabular-nums, 1px row separators, sticky silkscreen header, lamp column first, state also written in words in the Source column.
- Overlays: one sheet at a time (settings, confirm); focus moves in, is trapped, returns to the trigger; Escape and backdrop close; opens with opacity + 2% scale from its top edge in 220ms.
- Empty / loading / error: empty state is a grille panel with a verb-led prompt and the primary action; loading is the lamp plus meter; errors say what happened and keep the work (not found tracks stay in the list with the reasons in Info).
- Queue row: 56px cover thumbnail (grille placeholder until the art loads) with the status lamp top-left and the duration in a solid badge bottom-right, then three lines (file name, artist, title) and, at the right, the state in words plus two icon buttons (search, show in file browser). 72px rows, defined edge between rows, selection by lightness.
- Player: silver icon button, tabular time, accent-colored range, all native controls; the seek bar is disabled until the track is loaded.
- Resizable divider: 6px hit area on a 1px edge, `role="separator"` with `aria-valuenow`, arrow keys (24px, shift 80px), Home and End, double-click to reset; the native browser panel is hidden while dragging.
- Footer status bar: the engine lamp and its state in words, replaced by live progress (words plus the LED bar-graph) during a search; library counts are in the filter tabs only.
- Focus ring: box-shadow 0 0 0 2px bg, 0 0 0 4px led-green (3:1 or better against both surfaces).

## Wording

One term per concept, in the interface and in tooltips:

| concept | word |
|---|---|
| add tracks | Import (Files…, Folder…) |
| search everything listed and write what is found | Fetch |
| search the selection without writing | Search |
| write found lyrics into the files | Embed |
| no lyrics yet | Missing |
| no source had them | Not found (never "NA") |
| take tracks out of the list (files untouched) | Remove from list / Clear embedded from list |
| delete lyrics inside the files | Delete lyrics from file(s)… (always red, always last, after a rule) |
| reveal a file | Show in Finder / Show in Explorer / Show in file manager |

## Toolbars and menus

- Header: brand, filter, Import menu, Library menu, Fetch with its count (the only filled button), theme, settings.
- Status bar: one line. The engine lamp with READY; while a search runs, SEARCHING with done/total, the current track and source, the found count and the LED meter; DONE with the result for a few seconds after. A bot check waiting for the person takes its place with Verify and Skip. Counts live only in the filter tabs, never repeated here.
- List bar: selection on the left; Search, Embed, LRC files, then one overflow (⋯) for rarer list actions.
- Inspector: Copy, Edit, Search again and one overflow (⋯); text size on the far right. Everything that writes a file says so in its label or tooltip.
- Menus are fixed popovers anchored to their button (never full width), destructive item last and red.
- Bot checks never open a window by themselves: sites behind one load in a hidden browser; a check that needs a person appears as a lamp and a sentence in the status bar with Verify and Skip.
- Web tab: Use selection puts the text highlighted on the page into the editor.

## Motion

- Duration scale: instant 80ms / fast 120ms / base 180ms / sheet 260ms
- Easing: --ease-out cubic-bezier(0.23, 1, 0.32, 1); --ease-in-out cubic-bezier(0.77, 0, 0.175, 1)
- What animates: opacity and transform only. Searching lamp blinks (opacity). Meter fill is scaleX. Rows, filters and tabs do not animate (high frequency).
- Reduced motion: lamp blink becomes a steady dim amber; sheet and toast become a plain opacity fade.
- Signature motion: none beyond the lamp; restraint is the point.

## Iconography

- Set: Lucide (inlined SVG paths, no runtime dependency) | grid: 24px | stroke: 2px | caps/joins: round / round
- Brand mark: the approved artwork package v6 (charcoal speaker faceplate, green magnifier over lyric lines, green LED), used as is at 32px in the faceplate and as the app icon (icns, ico, Linux hicolor 16-512).
- Tray: the brand "LS" mark with a status lamp (idle blue, fetching yellow, complete green, stopped red, error red with !), dark or light artwork from the system theme, never template-tinted on macOS; the tooltip says the state in words.

## Imagery and illustration

- Mode: product visuals only (the app icon, grille texture, concentric-ring "cone" drawn in SVG)
- Rules: no stock, no gradients, no blobs
- Avoid: stock people, glossy tech illustrations, corporate-Memphis
- Text-over-image contrast: no text over imagery

## Dark mode

- Base bg: L0.155 near-black | fg: off-white L0.93 | elevation ramp: 0.155 / 0.19 / 0.23 / 0.275
- Accent (dark): brand green oklch(0.817 0.245 138) (#65E51C), used only for lamps, focus ring, switch-on, and the meter | border: lighter than surface
- Theme switch: no transition during the swap

## Accessibility

- Contrast: AA verified in both modes by computing ratios from the tokens (every text pair 4.5:1 or better; control edges, focus ring and lamps 3:1 or better) | Focus: visible ring on every control
- Keyboard: the track list is a listbox (arrows, Cmd/Ctrl-A, Esc); tabs, sheets and switches are operable by keyboard; sheets trap and return focus
- Targets: 24px minimum everywhere, 28px in dense areas, 34px for toolbar buttons | Color independence: every lamp has a text label; UI is readable in grayscale
- Reduced motion: honored | Notes: drag-and-drop of folders always has the Add folder button as the non-drag alternative; source reordering uses up/down buttons

## Tokens (source of truth)

See `crates/gui/ui/style.css` (`:root` and `[data-theme="light"]`), which projects this file.

- Adapter: plain CSS custom properties

## Cards and surfaces

- Surfaces: border only, 3px radius | nesting: never cards in cards; the inspector, table and sidebar are flush regions

## Slop audit

- Date: 2026-10-14 | Result: pass after fixing 9 contrast failures and 4 craft items found during the audit
- Caught and corrected: (1) dark control edges were 2.2:1 against panels, now 3:1+; (2) light-theme red text 4.15:1, amber text 2.24:1, focus ring 2.98:1 on the page background, control edges 2.3:1, all raised to pass; (3) `color-mix` in OKLCH made the "not found" lamp purple, switched to OKLab; (4) reorder arrows were 20px tall, now 24px; (5) web tab hint was clipped; (6) sheets and toasts had no exit animation.
- Palette tells: no indigo/violet, no gradient, no cream; the only chromatic color is LED green. Type: Inter + Sora + Source Sans 3 (owner's choice), ratio 1.25. Surface tells: no side-tab borders, no border-plus-shadow stacking, no blur, radius 3px. Motion: opacity/transform only, reduced-motion handled. Icons: one grid and stroke, aria-labels on every icon-only button. Craft: state matrix present (hover, active, focus, disabled, selected, busy via lamp and meter); empty, loading and error states designed.
- Not verified: screen-reader pass, 200% zoom, color-blindness simulation. Lamp meaning is also written in words in the State column, so grayscale reading was designed in but not simulated.

## Changelog

- 2026-11-26: brand package v6 (icon, tray, brand green #65E51C with recomputed contrast); Import menu replaces Files + Folder; Fetch replaces Find & embed all; the engine lamp and live progress share one status line in the footer (the duplicate counts are gone); "Not found" replaces NA; list and inspector toolbars cut to three labelled actions plus an overflow menu; type roles tightened (Sora labels, Source Sans text, Plex Mono numbers); fixed the full-width menus and the macOS web panel sitting one title bar too high; bot checks moved into a hidden browser with an in-app notice; Use selection in the Web tab.

- 2026-10-14: renamed to Lyrixearch; macOS icon; queue rows with cover art; player; resizable inspector; footer status bar; mobile web view. Layout fix: middle grid row is minmax(0, 1fr) so panels scroll. Re-checked: no new colors, radii or shadows introduced; all new controls have hover, active, focus and disabled states, labels, and 24px or larger targets.
- 2026-10-14: contrast fixes (see slop audit).
- 2026-10-14: first version. Replaced the earlier indigo/blue gradient glass UI with the icon-derived hardware direction.

- 2026-10-14 (3): fonts changed to the owner's set (Inter wordmark, Sora headings, Source Sans 3 body; Plex Mono kept for timecodes and counts) and icons to Lucide, on the owner's request. Status colors: found is blue, NA (not found) is purple; the orange tokens are gone. New tokens verified by computation: dark purple text 8.2:1 and blue 9.4:1 on surface; light purple text 7.5:1 and blue 6.6:1; the green call-to-action in light mode is oklch(0.5 0.15 145) with near-white text so it clears 4.5:1. Filter tabs are six equal columns so none scrolls out of view. Track list gets a select-all checkbox (tri-state), a files menu (format auto/.lrc/.txt, replace existing), a more menu and a drop-embedded button. The top bar has one green Find & embed all button for the tracks currently listed. Browser panel is re-placed after every window resize (macOS pins child webviews to the bottom edge).
