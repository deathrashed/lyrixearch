// Shared by the main window and the settings window: icons, small helpers and appearance.
// Appearance lives in localStorage, which both windows share, so a change in one reaches the other
// through the "storage" event.
(() => {
  'use strict';
  // ───────────────────────── icons: Lucide (24 grid, 2px stroke, round caps), inlined so the app works offline ─────────────────────────
  const ICONS = {
    gear: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" /> <circle cx="12" cy="12" r="3" />',
    palette: '<circle cx="13.5" cy="6.5" r=".5" fill="currentColor" /> <circle cx="17.5" cy="10.5" r=".5" fill="currentColor" /> <circle cx="8.5" cy="7.5" r=".5" fill="currentColor" /> <circle cx="6.5" cy="12.5" r=".5" fill="currentColor" /> <path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z" />',
    info: '<circle cx="12" cy="12" r="10" /> <path d="M12 16v-4" /> <path d="M12 8h.01" />',
    database: '<ellipse cx="12" cy="5" rx="9" ry="3" /> <path d="M3 5V19A9 3 0 0 0 21 19V5" /> <path d="M3 12A9 3 0 0 0 21 12" />',
    search: '<circle cx="11" cy="11" r="8" /> <path d="m21 21-4.3-4.3" />',
    plus: '<path d="M5 12h14" /> <path d="M12 5v14" />',
    refresh: '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" /> <path d="M21 3v5h-5" /> <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" /> <path d="M8 16H3v5" />',
    download: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /> <polyline points="7 10 12 15 17 10" /> <line x1="12" x2="12" y1="15" y2="3" />',
    export: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /> <polyline points="17 8 12 3 7 8" /> <line x1="12" x2="12" y1="3" y2="15" />',
    stop: '<rect width="18" height="18" x="3" y="3" rx="2" />',
    moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />',
    sun: '<circle cx="12" cy="12" r="4" /> <path d="M12 2v2" /> <path d="M12 20v2" /> <path d="m4.93 4.93 1.41 1.41" /> <path d="m17.66 17.66 1.41 1.41" /> <path d="M2 12h2" /> <path d="M20 12h2" /> <path d="m6.34 17.66-1.41 1.41" /> <path d="m19.07 4.93-1.41 1.41" />',
    sliders: '<line x1="21" x2="14" y1="4" y2="4" /> <line x1="10" x2="3" y1="4" y2="4" /> <line x1="21" x2="12" y1="12" y2="12" /> <line x1="8" x2="3" y1="12" y2="12" /> <line x1="21" x2="16" y1="20" y2="20" /> <line x1="12" x2="3" y1="20" y2="20" /> <line x1="14" x2="14" y1="2" y2="6" /> <line x1="8" x2="8" y1="10" y2="14" /> <line x1="16" x2="16" y1="18" y2="22" />',
    folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />',
    library: '<path d="m16 6 4 14" /> <path d="M12 6v14" /> <path d="M8 8v12" /> <path d="M4 4v16" />',
    reveal: '<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2" />',
    link: '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /> <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />',
    trash: '<path d="M3 6h18" /> <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" /> <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" /> <line x1="10" x2="10" y1="11" y2="17" /> <line x1="14" x2="14" y1="11" y2="17" />',
    globe: '<circle cx="12" cy="12" r="10" /> <path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20" /> <path d="M2 12h20" />',
    check: '<path d="M20 6 9 17l-5-5" />',
    x: '<path d="M18 6 6 18" /> <path d="m6 6 12 12" />',
    left: '<path d="m15 18-6-6 6-6" />',
    right: '<path d="m9 18 6-6-6-6" />',
    plug: '<path d="M12 22v-5" /> <path d="M9 8V2" /> <path d="M15 8V2" /> <path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8Z" />',
    up: '<path d="m18 15-6-6-6 6" />',
    down: '<path d="m6 9 6 6 6-6" />',
    play: '<polygon points="6 3 20 12 6 21 6 3" />',
    pause: '<rect x="14" y="4" width="4" height="16" rx="1" /> <rect x="6" y="4" width="4" height="16" rx="1" />',
    copy: '<rect width="14" height="14" x="8" y="8" rx="2" ry="2" /> <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />',
    paste: '<path d="M15 2H9a1 1 0 0 0-1 1v2c0 .6.4 1 1 1h6c.6 0 1-.4 1-1V3c0-.6-.4-1-1-1Z" /> <path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2M16 4h2a2 2 0 0 1 2 2v2M11 14h10" /> <path d="m17 10 4 4-4 4" />',
    fileText: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" /> <path d="M14 2v4a2 2 0 0 0 2 2h4" /> <path d="M10 9H8" /> <path d="M16 13H8" /> <path d="M16 17H8" />',
    filePlus: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" /> <path d="M14 2v4a2 2 0 0 0 2 2h4" /> <path d="M9 15h6" /> <path d="M12 18v-6" />',
    edit: '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" /> <path d="m15 5 4 4" />',
    external: '<path d="M15 3h6v6" /> <path d="M10 14 21 3" /> <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />',
    user: '<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2" /> <circle cx="12" cy="7" r="4" />',
    note: '<path d="M9 18V5l12-2v13" /> <circle cx="6" cy="18" r="3" /> <circle cx="18" cy="16" r="3" />',
    phone: '<rect width="14" height="20" x="5" y="2" rx="2" ry="2" /> <path d="M12 18h.01" />',
    rows: '<rect width="18" height="18" x="3" y="3" rx="2" /> <path d="M21 9H3" /> <path d="M21 15H3" />',
    grid: '<rect width="7" height="7" x="3" y="3" rx="1" /> <rect width="7" height="7" x="14" y="3" rx="1" /> <rect width="7" height="7" x="14" y="14" rx="1" /> <rect width="7" height="7" x="3" y="14" rx="1" />',
    type: '<polyline points="4 7 4 4 20 4 20 7" /> <line x1="9" x2="15" y1="20" y2="20" /> <line x1="12" x2="12" y1="4" y2="20" />',
    bolt: '<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z" />',
    more: '<circle cx="12" cy="12" r="1" /> <circle cx="19" cy="12" r="1" /> <circle cx="5" cy="12" r="1" />',
    listx: '<path d="M11 12H3" /> <path d="M16 6H3" /> <path d="M16 18H3" /> <path d="m19 10-4 4" /> <path d="m15 10 4 4" />',
  };
  const icon = (n) => `<svg class="i" viewBox="0 0 24 24" aria-hidden="true">${ICONS[n] || ''}</svg>`;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const errText = (e) => (typeof e === 'string' ? e : e?.message || JSON.stringify(e));
  const store = { get: (k) => { try { return localStorage.getItem(k); } catch { return null; } }, set: (k, v) => { try { localStorage.setItem(k, v); } catch { /* storage may be unavailable */ } } };

  // accent colours offered in Appearance; green is the brand (the icon's LED)
  const ACCENTS = {
    green: { name: 'Lyrixearch green', dark: 'oklch(0.817 0.245 138)', light: 'oklch(0.57 0.18 138)', lightText: 'oklch(0.48 0.14 138)', cta: 'oklch(0.5 0.15 138)', fgDark: 'oklch(0.18 0.02 138)' },
    blue: { name: 'Blue', dark: 'oklch(0.72 0.15 250)', light: 'oklch(0.55 0.19 258)', lightText: 'oklch(0.48 0.18 258)', cta: 'oklch(0.52 0.19 258)', fgDark: 'oklch(0.16 0.03 250)' },
    purple: { name: 'Purple', dark: 'oklch(0.72 0.16 300)', light: 'oklch(0.55 0.2 300)', lightText: 'oklch(0.48 0.19 300)', cta: 'oklch(0.5 0.2 300)', fgDark: 'oklch(0.16 0.03 300)' },
    amber: { name: 'Amber', dark: 'oklch(0.82 0.15 78)', light: 'oklch(0.62 0.15 65)', lightText: 'oklch(0.5 0.12 60)', cta: 'oklch(0.55 0.14 60)', fgDark: 'oklch(0.18 0.03 78)' },
    pink: { name: 'Pink', dark: 'oklch(0.74 0.17 355)', light: 'oklch(0.56 0.2 0)', lightText: 'oklch(0.5 0.19 0)', cta: 'oklch(0.52 0.2 0)', fgDark: 'oklch(0.16 0.03 355)' },
  };

  const systemDark = () => matchMedia('(prefers-color-scheme: dark)').matches;
  /** Theme setting: 'system', 'dark' or 'light'. */
  const themeSetting = () => { const t = store.get('lf-theme'); return t === 'dark' || t === 'light' ? t : 'system'; };
  const resolvedTheme = () => { const t = themeSetting(); return t === 'system' ? (systemDark() ? 'dark' : 'light') : t; };

  /** Apply theme, accent and lyrics size to this document. */
  function applyAppearance() {
    const root = document.documentElement;
    const theme = resolvedTheme();
    root.classList.add('theme-switching');
    root.dataset.theme = theme;
    const a = ACCENTS[store.get('lf-accent')] || ACCENTS.green;
    const p = root.style;
    if (theme === 'dark') {
      p.setProperty('--accent', a.dark); p.setProperty('--accent-text', a.dark); p.setProperty('--accent-fg', a.fgDark); p.setProperty('--cta', a.dark); p.setProperty('--cta-fg', a.fgDark);
    } else {
      p.setProperty('--accent', a.light); p.setProperty('--accent-text', a.lightText); p.setProperty('--accent-fg', 'oklch(0.98 0.01 138)'); p.setProperty('--cta', a.cta); p.setProperty('--cta-fg', 'oklch(0.99 0.005 138)');
    }
    const size = Number(store.get('lf-lyrics-size')) || 14;
    p.setProperty('--lyrics-size', `${Math.min(28, Math.max(11, size))}px`);
    requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove('theme-switching')));
    return theme;
  }

  window.LX = { ICONS, icon, esc, errText, store, ACCENTS, themeSetting, resolvedTheme, applyAppearance };
})();
