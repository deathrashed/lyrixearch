(() => {
  'use strict';
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args || {});
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];
  const ROW_H = 72;
  const AUDIO_EXT = ['mp3', 'flac', 'm4a', 'mp4', 'aac', 'ogg', 'oga', 'opus', 'wma', 'wav', 'aif', 'aiff', 'ape', 'wv'];

  // ───────────────────────── icons: Lucide (24 grid, 2px stroke, round caps), inlined so the app works offline ─────────────────────────
  const ICONS = {
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
  const paintIcons = (root = document) => $$('[data-icon]', root).forEach((el) => { el.innerHTML = icon(el.dataset.icon); });

  // ───────────────────────── state ─────────────────────────
  const S = {
    tracks: new Map(), view: [], filter: 'all', query: '',
    sel: new Set(), anchor: null, active: null,
    settings: null, progress: null, progressAt: 0, current: '', tab: 'lyrics', detail: null,
    modalOpen: false, popOpen: false, webUrl: null, dragging: false,
    covers: new Map(), coverReq: new Set(),
    player: null, seeking: false,
    lyricsSize: 14, mobile: true, tbMode: 'both',
    kind: 'auto', replace: false, pop: null, fileScope: 'sel',
  };

  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const errText = (e) => (typeof e === 'string' ? e : e?.message || JSON.stringify(e));
  const fmtTime = (s) => { s = Math.max(0, Math.round(s || 0)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
  const store = { get: (k) => { try { return localStorage.getItem(k); } catch { return null; } }, set: (k, v) => { try { localStorage.setItem(k, v); } catch { /* storage may be unavailable */ } } };

  const logLines = [];
  let logFrame = 0;

  function renderLog() {
    const pre = $('#log-lines');
    const follow = pre.scrollHeight - pre.scrollTop - pre.clientHeight <= 2;
    pre.textContent = logLines.join('\n') || 'No events in this session.';
    $('#log-copy').disabled = $('#log-clear').disabled = !logLines.length;
    if (follow) pre.scrollTop = pre.scrollHeight;
  }

  function logEvent(message) {
    const time = new Date().toLocaleTimeString();
    const key = S.settings?.config?.lastfm_api_key?.trim();
    const text = key ? String(message).split(key).join('[redacted]') : String(message);
    for (const line of text.split(/\r?\n/)) logLines.push(`[${time}] ${line}`);
    if (logLines.length > 2000) logLines.splice(0, logLines.length - 2000);
    if (!$('#session-log').hidden && !logFrame) logFrame = requestAnimationFrame(() => { logFrame = 0; renderLog(); });
  }

  function wireLog() {
    const dialog = $('#session-log');
    const close = () => closeSheet(dialog);
    $('#btn-log').onclick = () => {
      openSheet(dialog, '#log-close');
      renderLog();
      $('#log-lines').scrollTop = $('#log-lines').scrollHeight;
    };
    $('#log-close').onclick = close;
    dialog.addEventListener('click', (e) => { if (e.target === dialog) close(); });
    dialog.addEventListener('keydown', (e) => { if (e.key === 'Escape') close(); });
    $('#log-copy').onclick = async () => {
      try { await clip.write(logLines.join('\n')); toast('Log copied to the clipboard', 'good'); }
      catch { toast('Clipboard unavailable', 'bad'); }
    };
    $('#log-clear').onclick = () => { logLines.length = 0; renderLog(); $('#log-close').focus(); };
  }

  function toast(msg, kind = '', ms = 4200) {
    const el = document.createElement('div');
    el.className = 'toast';
    const lamp = kind === 'good' ? 'on' : kind === 'bad' ? 'error' : 'queued';
    el.innerHTML = `<i class="led ${lamp}" aria-hidden="true"></i><span>${esc(msg)}</span>`;
    $('#toasts').appendChild(el);
    setTimeout(() => { el.classList.add('out'); setTimeout(() => el.remove(), 130); }, ms);
  }

  const clip = {
    read: () => (T.clipboardManager ? T.clipboardManager.readText() : navigator.clipboard.readText()),
    write: (s) => (T.clipboardManager ? T.clipboardManager.writeText(s) : navigator.clipboard.writeText(s)),
  };

  // ───────────────────────── states: a lamp color, and the same meaning in words ─────────────────────────
  //   embedded green · missing amber · found blue · embedded just now green ring · not found purple · error red
  // display name of a source ("lrclib" -> "LRCLIB"), from the catalog the backend sends
  const pname = (id) => (id && S.settings?.providers?.find((p) => p.id === id)?.name) || id || '';
  function lampClass(t) { return t.status === 'idle' ? (t.has_lyrics ? 'has' : 'missing') : t.status; }
  function stateLabel(t) {
    switch (t.status) {
      case 'queued': return 'queued';
      case 'searching': return t.provider ? `searching · ${pname(t.provider)}` : 'searching';
      case 'found': return `found · ${pname(t.provider)}${t.synced ? ' · synced' : ''}`;
      case 'written': return t.provider ? `embedded · ${pname(t.provider)}` : 'embedded';
      case 'notfound': return 'not found';
      case 'error': return 'error';
      default: return t.has_lyrics ? 'embedded' : 'missing';
    }
  }
  function stateTone(t) {
    const l = lampClass(t);
    if (l === 'has' || l === 'written') return 'lit';
    if (l === 'found') return 'blue';
    if (l === 'missing' || l === 'queued' || l === 'searching') return 'warn';
    if (l === 'notfound') return 'purple';
    return 'bad';
  }

  // ───────────────────────── track queue ─────────────────────────
  const predicates = {
    all: () => true,
    missing: (t) => !t.has_lyrics,
    has: (t) => t.has_lyrics,
    found: (t) => t.status === 'found',
    notfound: (t) => t.status === 'notfound',
    error: (t) => t.status === 'error',
  };

  function setTracks(list) {
    S.tracks = new Map(list.map((t) => [t.id, t]));
    for (const id of [...S.sel]) if (!S.tracks.has(id)) S.sel.delete(id);
    if (S.active && !S.tracks.has(S.active)) setActive(null);
    refresh();
  }

  function recount() {
    const all = [...S.tracks.values()];
    const n = Object.fromEntries(Object.entries(predicates).map(([k, p]) => [k, all.filter(p).length]));
    for (const k of Object.keys(n)) $(`#c-${k}`).textContent = n[k];
  }

  // Tracks the big button searches: everything in the list below it, minus the ones that already
  // have lyrics unless replacing is on.
  const runIds = () => S.view.filter((t) => S.replace || !t.has_lyrics).map((t) => t.id);

  function computeView() {
    const q = S.query.trim().toLowerCase();
    const p = predicates[S.filter];
    S.view = [...S.tracks.values()].filter((t) => p(t) && (!q || `${t.artist} ${t.title} ${t.album || ''} ${t.name}`.toLowerCase().includes(q)));
    S.view.sort((a, b) => a.artist.localeCompare(b.artist) || (a.album || '').localeCompare(b.album || '') || a.name.localeCompare(b.name, undefined, { numeric: true }));
  }

  function setFilter(f, focus = false) {
    S.filter = f;
    $$('#viewtabs [role=tab]').forEach((x) => { const on = x.dataset.filter === f; x.setAttribute('aria-selected', on); x.tabIndex = on ? 0 : -1; if (on && focus) x.focus(); });
    $('#table').scrollTop = 0;
    refresh();
  }

  function refresh() {
    computeView();
    recount();
    $('#spacer').style.height = `${S.view.length * ROW_H}px`;
    $('#empty').hidden = S.tracks.size > 0;
    renderRows();
    renderSelbar();
    renderTop();
  }

  function rowHtml(t) {
    const cls = ['row', S.sel.has(t.id) ? 'sel' : '', S.active === t.id ? 'active' : ''].join(' ');
    const cover = S.covers.get(t.id);
    return `<div class="${cls}" role="option" id="row-${t.id}" aria-selected="${S.sel.has(t.id) || S.active === t.id}" data-id="${t.id}" style="height:${ROW_H}px">
      <div class="thumb">${cover ? `<img alt="" src="${cover}">` : ''}<i class="led ${lampClass(t)}" aria-hidden="true"></i>${t.duration ? `<span class="dur">${fmtTime(t.duration)}</span>` : ''}</div>
      <div class="lines">
        <span class="fname"><span title="${esc(t.path)}">${esc(t.name)}</span></span>
        <span class="meta1">${icon('user')}<span title="${esc(t.artist)}">${esc(t.artist) || '&nbsp;'}</span></span>
        <span class="meta2">${icon('note')}<span title="${esc(t.title)}">${esc(t.title)}</span></span>
      </div>
      <div class="side">
        <span class="src ${stateTone(t)}">${esc(stateLabel(t))}</span>
        <span class="btns">
          <button data-act="search" title="Search lyrics for this track" aria-label="Search lyrics for ${esc(t.title)}">${icon('search')}</button>
          <button data-act="reveal" title="Show in file browser" aria-label="Show ${esc(t.name)} in the file browser">${icon('reveal')}</button>
        </span>
      </div></div>`;
  }

  function renderRows() {
    const box = $('#table');
    const top = box.scrollTop;
    const h = box.clientHeight || 600;
    const start = Math.max(0, Math.floor(top / ROW_H) - 4);
    const end = Math.min(S.view.length, Math.ceil((top + h) / ROW_H) + 4);
    const rows = $('#rows');
    rows.style.transform = `translateY(${start * ROW_H}px)`;
    const slice = S.view.slice(start, end);
    rows.innerHTML = slice.map(rowHtml).join('');
    if (S.active != null) box.setAttribute('aria-activedescendant', `row-${S.active}`); else box.removeAttribute('aria-activedescendant');
    wantCovers(slice);
  }

  // cover thumbnails: fetched lazily for the rows on screen, a few at a time
  const coverQueue = [];
  let coverBusy = 0;
  function wantCovers(slice) {
    for (const t of slice) {
      if (S.covers.has(t.id) || S.coverReq.has(t.id)) continue;
      S.coverReq.add(t.id);
      coverQueue.push(t.id);
    }
    pumpCovers();
  }
  function pumpCovers() {
    while (coverBusy < 3 && coverQueue.length) {
      const id = coverQueue.shift();
      if (!S.tracks.has(id)) { S.coverReq.delete(id); continue; }
      coverBusy++;
      invoke('get_cover', { id })
        .then((url) => {
          S.covers.set(id, url || null);
          const el = $(`#row-${id} .thumb`);
          if (el && url && !el.querySelector('img')) el.insertAdjacentHTML('afterbegin', `<img alt="" src="${url}">`);
        })
        .catch(() => S.covers.set(id, null))
        .finally(() => { coverBusy--; S.coverReq.delete(id); pumpCovers(); });
    }
  }

  function selectOnly(id) { S.sel = new Set([id]); S.anchor = id; }

  function onRowClick(ev) {
    const row = ev.target.closest('.row');
    if (!row) return;
    const id = Number(row.dataset.id);
    const act = ev.target.closest('[data-act]')?.dataset.act;
    if (act === 'search') { startSearch([id], false); return; }
    if (act === 'reveal') { invoke('reveal_track', { id }).catch((e) => toast(errText(e), 'bad')); return; }
    if (ev.shiftKey && S.anchor != null) {
      const a = S.view.findIndex((t) => t.id === S.anchor);
      const b = S.view.findIndex((t) => t.id === id);
      if (a >= 0 && b >= 0) {
        if (!(ev.metaKey || ev.ctrlKey)) S.sel.clear();
        for (let i = Math.min(a, b); i <= Math.max(a, b); i++) S.sel.add(S.view[i].id);
      }
    } else if (ev.metaKey || ev.ctrlKey) {
      S.sel.has(id) ? S.sel.delete(id) : S.sel.add(id);
      S.anchor = id;
    } else selectOnly(id);
    setActive(id);
    $('#table').focus({ preventScroll: true });
    renderRows(); renderSelbar(); renderTop();
  }

  function renderSelbar() {
    const n = S.sel.size;
    const shown = S.view.length;
    const inView = S.view.filter((t) => S.sel.has(t.id)).length;
    const all = shown > 0 && inView === shown;
    const box = $('#sel-all');
    box.setAttribute('aria-checked', all ? 'true' : inView > 0 ? 'mixed' : 'false');
    box.disabled = shown === 0;
    box.title = all ? 'Deselect every track shown' : 'Select every track shown';
    $('#sel-all-label').textContent = all ? 'Deselect all' : 'Select all';
    $('#sel-count').textContent = n ? `${n} of ${S.tracks.size}` : `${shown} shown`;
    for (const id of ['sel-search', 'sel-embed', 'sel-files', 'mn-drop', 'mn-remove']) $(`#${id}`).disabled = n === 0;
    $('#mn-clear-embedded').disabled = ![...S.tracks.values()].some((t) => t.has_lyrics);
  }

  function toggleSelectAll() {
    const all = S.view.length > 0 && S.view.every((t) => S.sel.has(t.id));
    if (all) { for (const t of S.view) S.sel.delete(t.id); } else { for (const t of S.view) S.sel.add(t.id); S.anchor = S.view[0]?.id ?? null; }
    renderRows(); renderSelbar(); renderTop();
  }

  // top strip and status bar: engine lamp, progress with words and a bar
  function renderTop() {
    const pr = S.progress;
    const running = !!pr?.running;
    const run = $('#btn-run');
    const ids = running ? [] : runIds();
    run.classList.toggle('stop', running);
    run.disabled = !running && ids.length === 0;
    run.querySelector('[data-icon]').innerHTML = icon(running ? 'stop' : 'bolt');
    $('#btn-run-label').textContent = running ? 'Stop' : 'Fetch';
    const cnt = $('#btn-run-count');
    cnt.hidden = running || ids.length === 0;
    cnt.textContent = ids.length;
    run.title = running ? 'Stop searching' : ids.length ? `Search ${ids.length} listed track${ids.length === 1 ? '' : 's'} and embed the lyrics found${S.replace ? ', replacing existing lyrics' : ''}` : 'Every listed track has lyrics';
    // status line: Ready, or the live search, or the result for a few seconds after it ends
    const recentlyDone = pr && !running && Date.now() - S.progressAt < 9000;
    $('#engine-led').className = `led ${running ? 'searching' : 'on'}`;
    $('#engine-text').textContent = running ? 'Searching' : recentlyDone ? 'Done' : 'Ready';
    $('#meter').hidden = !running;
    let detail = '';
    if (running) {
      $('#meter-fill').style.setProperty('--p', pr.total ? pr.done / pr.total : 0);
      detail = `${pr.done}/${pr.total}${S.current ? ' · ' + S.current : ''} · ${pr.found} found`;
    } else if (recentlyDone) {
      detail = `${pr.found} found${pr.written ? ' · ' + pr.written + ' embedded' : ''} · ${pr.missed} not found`;
      setTimeout(renderTop, 9100);
    }
    $('#progress-text').textContent = detail;
  }

  // ───────────────────────── player ─────────────────────────
  function renderPlayer() {
    const t = activeTrack();
    if (!t) return;
    const p = S.player;
    const mine = !!p && p.id === t.id && p.state !== 'stopped' && p.state !== 'error';
    const playing = mine && p.state === 'playing';
    const btn = $('#pl-btn');
    btn.innerHTML = icon(playing ? 'pause' : 'play');
    btn.title = playing ? 'Pause' : 'Play';
    btn.setAttribute('aria-label', btn.title);
    const dur = (mine && p.duration) || t.duration || 0;
    const pos = mine ? p.pos : 0;
    const seek = $('#pl-seek');
    seek.disabled = !(mine && dur > 0);
    if (!S.seeking) seek.value = dur ? Math.min(1000, (pos / dur) * 1000) : 0;
    $('#pl-pos').textContent = fmtTime(S.seeking ? (seek.value / 1000) * dur : pos);
    $('#pl-dur').textContent = fmtTime(dur);
  }

  function togglePlay() {
    const t = activeTrack();
    if (!t) return;
    const p = S.player;
    if (!p || p.id !== t.id || p.state === 'stopped' || p.state === 'error') invoke('player_load', { id: t.id, play: true }).catch((e) => toast(errText(e), 'bad'));
    else invoke('player_toggle');
  }

  // ───────────────────────── inspector ─────────────────────────
  async function setActive(id) {
    if (S.player && S.player.id !== id && (S.player.state === 'playing' || S.player.state === 'paused')) invoke('player_stop');
    S.active = id;
    S.detail = null;
    if (id == null) return renderDetail();
    try { S.detail = await invoke('get_lyrics', { id }); } catch (e) { S.detail = null; }
    if (S.active === id) renderDetail();
  }

  const activeTrack = () => S.tracks.get(S.active);
  const shownLyrics = () => S.detail?.found?.plain || S.detail?.found?.synced || S.detail?.embedded || '';

  function renderDetail() {
    const t = activeTrack();
    $('#detail-empty').hidden = !!t;
    $('#detail-body').hidden = !t;
    if (!t) return syncBrowser();

    $('#d-title').textContent = t.name;
    $('#d-title').title = t.path;
    $('#d-led').className = `led ${lampClass(t)}`;
    const tags = [`<span class="tag ${stateTone(t)}">${esc(stateLabel(t))}</span>`, `<span class="tag">${esc(t.format)}</span>`];
    if (t.status === 'found' || t.status === 'written') tags.push(`<span class="tag">${Math.round((t.score || 0) * 100)}%</span>`);
    if (t.synced) tags.push('<span class="tag lit">synced</span>');
    $('#d-tags').innerHTML = tags.join('');
    $('#d-meta').innerHTML = `<b>${esc(t.artist) || 'Unknown artist'}</b> — ${esc(t.title)}${t.album ? ` · ${esc(t.album)}` : ''}`;
    $('#d-meta').title = `${t.artist} — ${t.title}${t.album ? ' · ' + t.album : ''}`;
    renderPlayer();

    const d = S.detail || {};
    const pre = $('#lyrics-text');
    const text = shownLyrics();
    if (d.found) { pre.className = 'lyrics'; pre.textContent = text; }
    else if (d.embedded) { pre.className = 'lyrics'; pre.textContent = d.embedded; }
    else {
      pre.className = 'lyrics placeholder';
      pre.textContent = t.status === 'notfound' ? 'No source had these lyrics. Try the Web tab, paste them in, or embed a matching .lrc or .txt file. Info lists what each source said.' : 'No lyrics yet.';
    }
    $('#act-embed').hidden = !d.found;
    $('#act-copy').disabled = !text;
    $('#tm-remove').hidden = !t.has_lyrics;

    const ta = $('#edit-text');
    ta.dataset.id = t.id;
    ta.value = d.found?.plain || d.embedded || '';
    ta.dataset.orig = ta.value;

    const rows = [['File', t.name], ['Folder', t.folder], ['Format', t.format], ['Duration', t.duration ? fmtTime(t.duration) : 'unknown'], ['Artist', t.artist], ['Title', t.title], ['Album', t.album || 'none'], ['State', stateLabel(t)]];
    if (d.found?.url) rows.push(['Source page', d.found.url]);
    if (t.note) rows.push(['Sources', t.note]);
    $('#info').innerHTML = rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v).replace(/\n/g, '<br>')}</dd>`).join('');

    const q = $('#web-query');
    if (q.dataset.for !== String(t.id)) {
      q.value = `${t.artist} - ${t.title} lyrics`;
      q.dataset.for = String(t.id);
      S.webUrl = null;
    }
    // a new track loads its own search instead of keeping the last track's page
    syncBrowser(S.webUrl === null);
  }

  function setTab(tab, focus = false) {
    S.tab = tab;
    $$('#tabs [role=tab]').forEach((b) => { const on = b.dataset.tab === tab; b.setAttribute('aria-selected', on); b.tabIndex = on ? 0 : -1; if (on && focus) b.focus(); });
    ['lyrics', 'edit', 'web', 'info'].forEach((n) => { $(`#tab-${n}`).hidden = n !== tab; });
    syncBrowser(true);
  }

  // ───────────────────────── toolbar: icons, text, or both ─────────────────────────
  const TB_MODES = [
    { id: 'both', label: 'icon and text', icon: 'rows' },
    { id: 'icons', label: 'icons only', icon: 'grid' },
    { id: 'text', label: 'text only', icon: 'type' },
  ];
  function applyToolbarMode() {
    const m = TB_MODES.find((x) => x.id === S.tbMode) || TB_MODES[0];
    const tb = $('#lyrics-toolbar');
    tb.dataset.mode = m.id;
    $('#listbar').dataset.mode = m.id;
    $$('.btn[data-label]').forEach((b) => { b.title = b.dataset.label; b.setAttribute('aria-label', b.dataset.label); });
  }

  // ───────────────────────── embedded browser ─────────────────────────
  // Query goes in as "artist - title lyrics", plus-encoded. Google AI Mode and Brave AI take the question in
  // the URL (the pattern comes from the wprompt project).
  const plus = (q) => encodeURIComponent(q).replace(/%20/g, '+');
  const ENGINES = {
    google: (q) => `https://www.google.com/search?q=${plus(q)}`,
    google_ai: (q) => `https://www.google.com/search?udm=50&source=searchlabs&q=${plus(q)}`,
    brave_ai: (q) => `https://search.brave.com/ask?q=${plus(q)}`,
    bing: (q) => `https://www.bing.com/search?q=${plus(q)}`,
    duckduckgo: (q) => `https://duckduckgo.com/?q=${plus(q)}`,
    yahoo: (q) => `https://search.yahoo.com/search?p=${plus(q)}`,
    yandex: (q) => `https://yandex.com/search/?text=${plus(q)}`,
  };
  const ENGINE_LABELS = { google: 'Google', google_ai: 'Google AI Mode (browser)', brave_ai: 'Brave AI', bing: 'Bing', duckduckgo: 'DuckDuckGo', yahoo: 'Yahoo', yandex: 'Yandex' };
  const engineOptions = (ids, current) => ids.map((e) => `<option value="${e}" ${e === current ? 'selected' : ''}>${esc(ENGINE_LABELS[e] || e)}</option>`).join('');
  // Google AI Mode shows plain results inside an embedded view (it needs your signed-in browser), so it opens there.
  const EXTERNAL_ENGINES = new Set(['google_ai']);

  // vh lets the backend line the page up with this viewport (the window can be taller, e.g. a macOS title bar)
  const slotRect = () => { const r = $('#webslot').getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height, vh: innerHeight }; };
  // false on Linux, where pages open in the person's browser instead of inside the window
  const canEmbed = () => S.settings?.embedded_browser !== false;
  const browserWanted = () => canEmbed() && S.tab === 'web' && !!activeTrack() && !S.modalOpen && !S.popOpen && !S.dragging;
  let browserOpen = false;
  let lastRect = '';
  let rectRaf = 0;

  // The native page sits above the UI, so its position must always equal the slot's. Rather than rely on
  // resize events (a moved slot with an unchanged size fires none), compare every frame while it is shown.
  function watchRect() {
    cancelAnimationFrame(rectRaf);
    const tick = () => {
      if (!browserWanted() || !browserOpen) { rectRaf = 0; return; }
      const r = slotRect();
      const key = [r.x, r.y, r.w, r.h, innerWidth, innerHeight].map((v) => Math.round(v * 2) / 2).join(',');
      if (key !== lastRect) { lastRect = key; invoke('browser_show', { url: null, rect: r }).catch(() => {}); }
      rectRaf = requestAnimationFrame(tick);
    };
    rectRaf = requestAnimationFrame(tick);
  }

  function syncBrowser(navigate = false) {
    if (!browserWanted()) { if (browserOpen) invoke('browser_hide').catch(() => {}); lastRect = ''; return; }
    const engine = $('#web-engine').value;
    const url = navigate && !S.webUrl && !EXTERNAL_ENGINES.has(engine) ? urlFor($('#web-query').value) : null;
    if (url) S.webUrl = url;
    browserOpen = true;
    const r = slotRect();
    lastRect = [r.x, r.y, r.w, r.h, innerWidth, innerHeight].map((v) => Math.round(v * 2) / 2).join(',');
    invoke('browser_show', { url, rect: r }).catch((e) => toast(`Browser: ${errText(e)}`, 'bad'));
    watchRect();
  }

  function urlFor(text, engine) {
    const v = text.trim();
    if (!v) return null;
    if (/^https?:\/\//i.test(v)) return v;
    return (ENGINES[engine || $('#web-engine').value] || ENGINES.google)(v);
  }

  function goWeb() {
    const engine = $('#web-engine').value;
    const url = urlFor($('#web-query').value);
    if (!url) return;
    if (!canEmbed()) {
      S.webUrl = url;
      invoke('open_external', { url }).then(() => toast('Opened in your browser', 'good')).catch((e) => toast(errText(e), 'bad'));
      return;
    }
    if (EXTERNAL_ENGINES.has(engine) && !/^https?:\/\//i.test($('#web-query').value.trim())) {
      invoke('open_external', { url }).then(() => toast('Opened in your browser, where you are signed in to Google', 'good')).catch((e) => toast(errText(e), 'bad'));
      return;
    }
    S.webUrl = url;
    browserOpen = true;
    invoke('browser_show', { url, rect: slotRect() }).catch((e) => toast(`Browser: ${errText(e)}`, 'bad'));
    watchRect();
  }

  // ───────────────────────── sheets: focus moves in, is trapped, returns on close ─────────────────────────
  const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
  const sheetStack = [];

  function openSheet(modal, focusSel) {
    sheetStack.push({ modal, from: document.activeElement });
    S.modalOpen = true;
    syncBrowser();
    modal.hidden = false;
    (focusSel ? $(focusSel, modal) : $$(FOCUSABLE, modal)[0])?.focus();
  }

  function closeSheet(modal) {
    modal.classList.add('closing');
    setTimeout(() => { modal.classList.remove('closing'); modal.hidden = true; }, 130);
    const i = sheetStack.findIndex((s) => s.modal === modal);
    const entry = i >= 0 ? sheetStack.splice(i, 1)[0] : null;
    S.modalOpen = sheetStack.length > 0;
    syncBrowser();
    entry?.from?.focus?.();
  }

  document.addEventListener('keydown', (e) => {
    const top = sheetStack[sheetStack.length - 1];
    if (!top || e.key !== 'Tab') return;
    const items = $$(FOCUSABLE, top.modal).filter((el) => el.offsetParent !== null);
    if (!items.length) return;
    const first = items[0], last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  });

  function confirmBox(title, text, okLabel, withReplace = false) {
    return new Promise((resolve) => {
      const m = document.createElement('div');
      m.className = 'modal';
      const body = () => (typeof text === 'function' ? text() : text);
      const sw = withReplace ? `<label class="field inline" for="cb-replace"><span class="fl">Replace existing<small>Also search tracks that already have lyrics</small></span><span class="switch"><input id="cb-replace" type="checkbox" ${S.replace ? 'checked' : ''}><i></i></span></label>` : '';
      m.innerHTML = `<div class="sheet compact" role="alertdialog" aria-modal="true" aria-labelledby="cb-t" aria-describedby="cb-d"><header><h2 id="cb-t">${esc(title)}</h2></header>
        <div class="sbody"><p id="cb-d" style="margin:0">${esc(body())}</p>${sw}</div>
        <footer><span></span><span style="display:flex;gap:8px"><button class="btn" data-r="0">Cancel</button><button class="btn primary" data-r="1">${esc(okLabel)}</button></span></footer></div>`;
      document.body.appendChild(m);
      const done = (v) => { closeSheet(m); setTimeout(() => m.remove(), 200); resolve(v); };
      m.addEventListener('click', (e) => { const r = e.target.closest('[data-r]'); if (r) done(r.dataset.r === '1'); else if (e.target === m) done(false); });
      m.addEventListener('change', (e) => {
        if (e.target.id !== 'cb-replace') return;
        setReplace(e.target.checked);
        $('#cb-d', m).textContent = body();
      });
      m.addEventListener('keydown', (e) => { if (e.key === 'Escape') done(false); });
      openSheet(m, '[data-r="0"]');
    });
  }

  // ───────────────────────── floating popovers: lyric files, more actions ─────────────────────────
  function closeFloating(sync = true) {
    if (!S.pop) return;
    S.pop.el.hidden = true;
    S.pop.anchor.setAttribute('aria-expanded', 'false');
    S.pop = null;
    S.popOpen = !!S.libOpen;
    if (sync) syncBrowser();
  }

  function openFloating(el, anchor, focusSel) {
    if (S.pop && S.pop.el === el) { closeFloating(); return; }
    closeFloating(false);
    if (S.libOpen) toggleLibrary(false);
    el.hidden = false;
    const a = anchor.getBoundingClientRect();
    const w = el.offsetWidth, h = el.offsetHeight;
    const left = Math.max(8, Math.min(a.right - w, innerWidth - w - 8));
    const below = a.bottom + 6;
    const top = below + h > innerHeight - 36 ? Math.max(8, a.top - h - 6) : below;
    el.style.left = `${Math.round(left)}px`;
    el.style.top = `${Math.round(top)}px`;
    anchor.setAttribute('aria-expanded', 'true');
    S.pop = { el, anchor };
    S.popOpen = true;
    syncBrowser();
    (focusSel ? $(focusSel, el) : $$('button, input', el)[0])?.focus();
  }

  const scopeIds = () => (S.fileScope === 'active' ? (S.active != null ? [S.active] : []) : [...S.sel]);
  const kindArg = () => (S.kind === 'auto' ? null : S.kind);
  const kindName = () => (S.kind === 'auto' ? '.lrc / .txt' : `.${S.kind}`);

  function paintFileOptions() {
    $$('#fp-kind [data-kind]').forEach((b) => b.setAttribute('aria-checked', b.dataset.kind === S.kind));
    $$('[data-replace]').forEach((i) => { i.checked = S.replace; });
    const n = scopeIds().length;
    $('#fp-scope').textContent = S.fileScope === 'active' ? 'Applies to this track.' : `Applies to the ${n} selected track${n === 1 ? '' : 's'}.`;
  }

  function openFileOptions(anchor, scope) {
    S.fileScope = scope;
    paintFileOptions();
    openFloating($('#file-pop'), anchor, '#fp-embed');
  }

  async function setReplace(v, persist = true) {
    S.replace = !!v;
    $$('[data-replace]').forEach((i) => { i.checked = S.replace; });
    renderTop();
    if (persist && S.settings) {
      try { S.settings = await invoke('set_settings', { config: { ...S.settings.config, overwrite: S.replace } }); }
      catch (e) { toast(errText(e), 'bad'); }
    }
  }

  // ───────────────────────── library menu ─────────────────────────
  function toggleLibrary(open) {
    const pop = $('#library-pop');
    const will = open ?? pop.hidden;
    if (will) closeFloating(false);
    pop.hidden = !will;
    $('#btn-library').setAttribute('aria-expanded', will);
    S.libOpen = will;
    S.popOpen = will || !!S.pop;
    syncBrowser();
    if (will) { renderLibraries(); $('#btn-lib-add').focus(); }
  }

  async function renderLibraries() {
    if (!S.settings) S.settings = await invoke('get_settings');
    const dirs = S.settings.config.library_dirs || [];
    $('#libraries').innerHTML = dirs.map((d, i) => `<li title="${esc(d)}">${icon('folder')}<span class="n">${esc(d)}</span><button data-rm="${i}" aria-label="Remove ${esc(d)} from the library">${icon('x')}</button></li>`).join('');
    $('#lib-hint').hidden = dirs.length > 0;
    const n = dirs.length;
    $('#btn-library .lbl').textContent = n ? `Library ${n}` : 'Library';
  }

  async function saveLibraryDirs(dirs) {
    S.settings = await invoke('set_settings', { config: { ...S.settings.config, library_dirs: dirs } });
    renderLibraries();
  }

  // ───────────────────────── actions ─────────────────────────
  async function pickFolders() {
    const r = await T.dialog.open({ directory: true, multiple: true, title: 'Choose music folders' });
    return !r ? [] : Array.isArray(r) ? r : [r];
  }
  async function pickFiles() {
    const r = await T.dialog.open({ multiple: true, title: 'Choose audio files', filters: [{ name: 'Audio', extensions: AUDIO_EXT }] });
    return !r ? [] : Array.isArray(r) ? r : [r];
  }

  async function addPaths(paths) {
    if (!paths.length) return;
    toast(`Scanning ${paths.length === 1 ? paths[0].split(/[\\/]/).pop() : paths.length + ' items'}, including subfolders`);
    try {
      const before = S.tracks.size;
      const res = await invoke('add_paths', { paths });
      setTracks(res.tracks);
      const added = res.tracks.length - before;
      toast(added > 0 ? `Added ${added} track${added === 1 ? '' : 's'}. ${res.stats.missing} missing lyrics.` : 'No new audio files found', added > 0 ? 'good' : '');
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  // `replace` undefined searches exactly the tracks named; true/false follows the Replace existing switch.
  async function startSearch(ids, autoWrite, replace) {
    try {
      const n = await invoke('search', { ids, autoWrite, replace });
      if (n === 0) toast(replace === false ? 'Those tracks already have lyrics. Turn on Replace existing in Files to search them anyway.' : ids.length ? 'Those tracks are already being searched' : 'Every track already has lyrics');
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  async function runAll() {
    if (S.progress?.running) { invoke('cancel_search'); return; }
    if (!runIds().length) { toast(S.view.length ? 'Every track listed already has lyrics. Turn on Replace existing in Files to search them anyway.' : 'No tracks in the list'); return; }
    const where = S.settings?.config.write_tags === false ? 'saved as sidecar files' : 'embedded in the audio files';
    const ok = await confirmBox('Fetch lyrics?', () => {
      const n = runIds().length;
      return `Searches ${n} listed track${n === 1 ? '' : 's'}. Lyrics found are ${where} right away.`;
    }, 'Fetch lyrics', true);
    if (!ok) return;
    const ids = runIds();
    if (ids.length) startSearch(ids, true, S.replace);
  }

  async function dropEmbedded() {
    const ids = [...S.tracks.values()].filter((t) => t.has_lyrics).map((t) => t.id);
    if (!ids.length) { toast('No track in the list has embedded lyrics'); return; }
    try {
      setTracks((await invoke('remove_tracks', { ids })).tracks);
      toast(`Took ${ids.length} track${ids.length === 1 ? '' : 's'} with lyrics out of the list. Files are untouched.`, 'good');
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  async function embedIds(ids) {
    try {
      const n = await invoke('apply_found', { ids });
      toast(n ? `Embedded lyrics in ${n} file${n === 1 ? '' : 's'}` : 'Nothing found to embed yet', n ? 'good' : '');
      if (S.active) setActive(S.active);
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  async function embedFromFiles(ids) {
    if (!ids.length) { toast('Select tracks first'); return; }
    try {
      const r = await invoke('embed_sidecar', { ids, kind: kindArg(), replace: S.replace });
      const parts = [];
      if (r.embedded) parts.push(`${r.embedded} embedded from ${kindName()}`);
      if (r.kept) parts.push(`${r.kept} kept, they already have lyrics`);
      if (r.missing) parts.push(`${r.missing} had no ${kindName()} file`);
      if (r.failed) parts.push(`${r.failed} failed`);
      toast(parts.join(' · ') || 'Nothing to do', r.embedded ? 'good' : r.failed ? 'bad' : '');
      if (S.active) setActive(S.active);
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  async function writeFiles(ids) {
    if (!ids.length) { toast('Select tracks first'); return; }
    try {
      const r = await invoke('export_lyrics', { ids, kind: kindArg(), replace: S.replace });
      const parts = [];
      if (r.written) parts.push(`Wrote ${r.written} ${kindName()} file${r.written === 1 ? '' : 's'} next to the tracks`);
      if (r.kept) parts.push(`${r.kept} already existed`);
      if (r.none) parts.push(S.kind === 'lrc' ? `${r.none} had no synced lyrics` : `${r.none} had no lyrics`);
      for (const failure of r.failures || []) parts.push(failure);
      if (parts.length) logEvent(parts.join('\n'));
      toast(parts.join(' · ') || 'Nothing to write', r.failures?.length ? 'bad' : r.written ? 'good' : '', r.failures?.length ? 12000 : 4200);
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  async function removeLyrics(ids) {
    const ok = await confirmBox('Remove lyrics?', `This deletes the embedded lyrics from ${ids.length} file${ids.length === 1 ? '' : 's'}. The audio is not touched.`, 'Remove lyrics');
    if (!ok) return;
    try {
      const n = await invoke('delete_lyrics', { ids });
      toast(`Removed lyrics from ${n} file${n === 1 ? '' : 's'}`, 'good');
      if (S.active) setActive(S.active);
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  async function pasteLyrics() {
    const t = activeTrack();
    if (!t) return;
    let text = '';
    try { text = (await clip.read()) || ''; } catch { toast('Clipboard unavailable', 'bad'); return; }
    if (!text.trim()) { toast('The clipboard is empty'); return; }
    if (t.has_lyrics && !(await confirmBox('Replace the lyrics?', `${t.title} already has lyrics. Replace them with the ${text.trim().length} characters on the clipboard?`, 'Replace'))) return;
    try {
      await invoke('save_lyrics', { id: t.id, text });
      toast('Pasted and saved to the file', 'good');
      setActive(t.id);
    } catch (e) { toast(errText(e), 'bad', 7000); }
  }

  // ───────────────────────── settings ─────────────────────────
  let draft = null;

  async function openSettings() {
    S.settings = await invoke('get_settings');
    draft = JSON.parse(JSON.stringify(S.settings.config));
    renderSettings();
    openSheet($('#settings'), '#stabs [aria-selected="true"]');
  }
  const closeSettings = () => closeSheet($('#settings'));

  function providerOrder() {
    const cat = new Map(S.settings.providers.map((p) => [p.id, p]));
    const on = draft.providers.map((id) => ({ ...(cat.get(id) || { id, name: id, note: 'Source unavailable' }), enabled: true }));
    const off = S.settings.providers.filter((p) => !draft.providers.includes(p.id)).map((p) => ({ ...p, enabled: false }));
    return [...on, ...off];
  }

  function renderSettings() {
    $('#provider-list').innerHTML = providerOrder().map((p, i, arr) => `<li class="${p.enabled ? '' : 'off'}" data-id="${esc(p.id)}">
      <div class="ord"><button data-mv="-1" aria-label="Move ${esc(p.name)} up" ${i === 0 || !p.enabled ? 'disabled' : ''}>${icon('up')}</button><button data-mv="1" aria-label="Move ${esc(p.name)} down" ${!p.enabled || arr[i + 1]?.enabled === false || i === arr.length - 1 ? 'disabled' : ''}>${icon('down')}</button></div>
      <div class="grow"><div class="nm">${esc(p.name)} ${p.kind === 'plugin' ? '<span class="tag lit">plugin</span>' : ''}</div><div class="nt">${esc(p.note)}</div></div>
      <label class="switch"><input type="checkbox" aria-label="Use ${esc(p.name)}" ${p.enabled ? 'checked' : ''}><i></i></label></li>`).join('');

    const g = draft;
    const row = (k, lbl, sub, ctl) => `<div class="frow"><label class="lbl" for="f-${k}"><b>${lbl}</b><span>${sub}</span></label>${ctl}</div>`;
    const sw = (k) => `<span class="switch"><input id="f-${k}" type="checkbox" data-k="${k}" ${g[k] ? 'checked' : ''}><i></i></span>`;
    const rng = (k, min, max, step) => `<span style="display:flex;gap:12px;align-items:center"><input id="f-${k}" type="range" data-k="${k}" min="${min}" max="${max}" step="${step}" value="${g[k]}"><b class="mono" data-v="${k}" style="min-width:36px;text-align:right">${g[k]}</b></span>`;
    const sel = (k, opts) => `<select id="f-${k}" data-k="${k}">${k === 'search_engine' ? engineOptions(opts, g[k]) : opts.map((o) => `<option ${g[k] === o ? 'selected' : ''}>${o}</option>`).join('')}</select>`;
    $('#general-form').innerHTML = [
      row('concurrency', 'Simultaneous searches', 'Tracks searched at the same time', rng('concurrency', 1, 20, 1)),
      row('provider_timeout_secs', 'Source timeout', 'Seconds to wait for one source before moving on', rng('provider_timeout_secs', 2, 30, 1)),
      row('min_score', 'Match strictness', 'Minimum similarity for a result to be accepted', rng('min_score', 0.5, 1, 0.05)),
      row('synced_only', 'Synced lyrics only', 'Accept results with timed lines', sw('synced_only')),
      row('tag_clean_enabled', 'Clean track titles', 'Remove common edition and release labels before searching', sw('tag_clean_enabled')),
      row('tag_clean_pattern', 'Extra title cleanup', 'Optional regular expression to remove from track titles',
        `<div class="setting-field"><input id="f-tag_clean_pattern" type="text" data-k="tag_clean_pattern" value="${esc(g.tag_clean_pattern || '')}" spellcheck="false" aria-describedby="tag-clean-error"><span id="tag-clean-error" class="setting-error" role="alert"></span></div>`),
      row('lastfm_api_key', 'Last.fm API key', 'Optional, enables artist and title corrections',
        '<input id="f-lastfm_api_key" type="password" data-k="lastfm_api_key" autocomplete="off" spellcheck="false">'),
      row('write_tags', 'Write lyrics into the files', 'Embed in the audio tags (ID3 USLT, Vorbis, MP4)', sw('write_tags')),
      row('sidecar', 'Also save a sidecar file', 'A .lrc or .txt next to the track', sel('sidecar', ['off', 'auto', 'lrc', 'txt'])),
      row('overwrite', 'Replace existing lyrics', 'Search and overwrite even when a track already has lyrics', sw('overwrite')),
      row('keep_mtime', 'Keep file modification date', 'Restore the date after tagging', sw('keep_mtime')),
      row('remove_section_tags', 'Remove section headers', 'Drop lines such as [Chorus] or [Solo]', sw('remove_section_tags')),
      row('prepend_newline', 'Blank line at the start', 'Insert an empty first line', sw('prepend_newline')),
      row('search_engine', 'Web search engine', 'Used by the Web tab', sel('search_engine', S.settings.engines)),
      row('proxy', 'Proxy', 'http:// or socks5://, empty for none', `<input id="f-proxy" type="text" data-k="proxy" value="${esc(g.proxy || '')}" placeholder="none">`),
    ].join('');
    $('#f-lastfm_api_key').value = g.lastfm_api_key || '';

    const plugins = S.settings.providers.filter((p) => p.kind === 'plugin');
    $('#plugin-list').innerHTML = plugins.length
      ? plugins.map((p) => `<li><div class="grow"><div class="nm">${esc(p.name)}</div><div class="nt">${esc(p.id)} · ${esc(p.note)}</div></div><span class="tag ${draft.providers.includes(p.id) ? 'lit' : ''}">${draft.providers.includes(p.id) ? 'enabled' : 'disabled'}</span></li>`).join('')
      : '<li><div class="grow nt">No plugins installed yet.</div></li>';
    $('#plugins-dir').textContent = S.settings.plugins_dir;
    $('#config-path').textContent = S.settings.config_path;
  }

  function wireSettings() {
    $('#provider-list').addEventListener('click', (e) => {
      const btn = e.target.closest('[data-mv]');
      if (!btn || btn.disabled) return;
      const id = btn.closest('li').dataset.id;
      const i = draft.providers.indexOf(id);
      const j = i + Number(btn.dataset.mv);
      if (i < 0 || j < 0 || j >= draft.providers.length) return;
      [draft.providers[i], draft.providers[j]] = [draft.providers[j], draft.providers[i]];
      renderSettings();
      $(`#provider-list li[data-id="${CSS.escape(id)}"] [data-mv="${btn.dataset.mv}"]`)?.focus();
    });
    $('#provider-list').addEventListener('change', (e) => {
      if (e.target.type !== 'checkbox') return;
      const id = e.target.closest('li').dataset.id;
      if (e.target.checked) { if (!draft.providers.includes(id)) draft.providers.push(id); } else draft.providers = draft.providers.filter((x) => x !== id);
      renderSettings();
    });
    $('#general-form').addEventListener('input', (e) => {
      const k = e.target.dataset.k;
      if (!k) return;
      if (e.target.type === 'checkbox') draft[k] = e.target.checked;
      else if (e.target.type === 'range') { draft[k] = Number(e.target.value); const v = $(`[data-v="${k}"]`); if (v) v.textContent = e.target.value; }
      else if (k === 'proxy' || k === 'lastfm_api_key') draft[k] = e.target.value.trim() || null;
      else if (k === 'tag_clean_pattern') {
        draft[k] = e.target.value.trim() ? e.target.value : null;
        e.target.removeAttribute('aria-invalid');
        $('#tag-clean-error').textContent = '';
      }
      else draft[k] = e.target.value;
    });
    const tabs = $$('#stabs [role=tab]');
    const pick = (b) => {
      tabs.forEach((x) => { const on = x === b; x.setAttribute('aria-selected', on); x.tabIndex = on ? 0 : -1; });
      ['providers', 'general', 'plugins'].forEach((n) => { $(`#s-${n}`).hidden = n !== b.dataset.s; });
    };
    tabs.forEach((b) => { b.tabIndex = b.getAttribute('aria-selected') === 'true' ? 0 : -1; });
    $('#stabs').addEventListener('click', (e) => { const b = e.target.closest('[role=tab]'); if (b) pick(b); });
    $('#stabs').addEventListener('keydown', (e) => {
      if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
      const i = tabs.indexOf(document.activeElement);
      const n = tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
      pick(n); n.focus();
    });
    $('#settings-close').onclick = closeSettings;
    $('#settings').addEventListener('click', (e) => { if (e.target.id === 'settings') closeSettings(); });
    $('#settings').addEventListener('keydown', (e) => { if (e.key === 'Escape') closeSettings(); });
    $('#settings-save').onclick = async () => {
      try {
        draft.library_dirs = S.settings.config.library_dirs;
        S.settings = await invoke('set_settings', { config: draft });
        S.replace = !!S.settings.config.overwrite;
        renderTop();
        $('#web-engine').innerHTML = engineOptions(S.settings.engines, S.settings.config.search_engine);
        toast('Settings saved', 'good');
        closeSettings();
        renderLibraries();
      } catch (e) {
        const message = errText(e);
        if (message.startsWith('tag_clean_pattern is not a valid regex')) {
          pick(tabs.find((b) => b.dataset.s === 'general'));
          const field = $('#f-tag_clean_pattern');
          field.setAttribute('aria-invalid', 'true');
          $('#tag-clean-error').textContent = 'Enter a valid regular expression.';
          field.focus();
        } else toast(message, 'bad', 8000);
      }
    };
    $('#plugin-install').onclick = async () => {
      const r = await T.dialog.open({ directory: true, title: 'Choose a plugin folder (it contains plugin.toml)' });
      if (!r) return;
      try {
        S.settings = await invoke('install_plugin', { dir: Array.isArray(r) ? r[0] : r });
        draft = JSON.parse(JSON.stringify(S.settings.config));
        renderSettings();
        toast('Plugin installed and enabled', 'good');
      } catch (e) { toast(errText(e), 'bad', 8000); }
    };
    $('#plugin-test').onclick = async () => {
      const ul = $('#health-list');
      ul.innerHTML = '<li><i class="led searching" aria-hidden="true"></i><span>Testing every source</span></li>';
      try {
        const res = await invoke('test_providers');
        ul.innerHTML = res.map((r) => `<li><i class="led ${r.ok ? 'on' : 'error'}" aria-hidden="true"></i><b>${esc(r.id)}</b><span class="mono" style="flex:1;color:var(--muted)">${r.ok ? 'ok' : esc(r.detail)}</span><span class="mono" style="color:var(--muted)">${r.millis} ms</span></li>`).join('');
      } catch (e) { ul.innerHTML = `<li>${esc(errText(e))}</li>`; }
    };
  }

  // ───────────────────────── theme, text size, resizable inspector ─────────────────────────
  function setTheme(t) {
    const root = document.documentElement;
    root.classList.add('theme-switching');
    root.dataset.theme = t;
    store.set('lf-theme', t);
    $('#btn-theme').innerHTML = icon(t === 'light' ? 'moon' : 'sun');
    requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove('theme-switching')));
  }

  function setLyricsSize(px) {
    S.lyricsSize = Math.min(28, Math.max(11, px));
    document.documentElement.style.setProperty('--lyrics-size', `${S.lyricsSize}px`);
    store.set('lf-lyrics-size', String(S.lyricsSize));
  }

  const DETAIL_MIN = 340;
  const detailMax = () => Math.max(DETAIL_MIN, Math.min(1000, window.innerWidth - 480 - 6));
  function setDetailWidth(w, save = true) {
    const width = Math.round(Math.min(detailMax(), Math.max(DETAIL_MIN, w)));
    $('#app').style.setProperty('--detail-w', `${width}px`);
    const sp = $('#splitter');
    sp.setAttribute('aria-valuenow', width);
    sp.setAttribute('aria-valuemax', detailMax());
    if (save) store.set('lf-detail-w', String(width));
    return width;
  }

  function wireSplitter() {
    const sp = $('#splitter');
    setDetailWidth(Number(store.get('lf-detail-w')) || 520, false);
    let raf = 0;
    const end = () => {
      if (!S.dragging) return;
      S.dragging = false;
      sp.classList.remove('drag');
      syncBrowser();
    };
    sp.addEventListener('pointerdown', (e) => {
      S.dragging = true;
      sp.setPointerCapture(e.pointerId);
      sp.classList.add('drag');
      syncBrowser(); // hides the native browser while dragging so it cannot swallow the pointer
      e.preventDefault();
    });
    sp.addEventListener('pointermove', (e) => {
      if (!S.dragging) return;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => setDetailWidth(window.innerWidth - e.clientX - 3));
    });
    sp.addEventListener('pointerup', end);
    sp.addEventListener('pointercancel', end);
    sp.addEventListener('dblclick', () => { setDetailWidth(520); syncBrowser(); });
    sp.addEventListener('keydown', (e) => {
      const cur = Number(sp.getAttribute('aria-valuenow')) || 520;
      const step = e.shiftKey ? 80 : 24;
      const next = e.key === 'ArrowLeft' ? cur + step : e.key === 'ArrowRight' ? cur - step : e.key === 'Home' ? detailMax() : e.key === 'End' ? DETAIL_MIN : null;
      if (next == null) return;
      e.preventDefault();
      setDetailWidth(next);
      syncBrowser();
    });
    window.addEventListener('resize', () => {
      setDetailWidth(Number(sp.getAttribute('aria-valuenow')) || 520, false);
      for (const ms of [120, 400]) setTimeout(() => { lastRect = ''; }, ms);
    });
  }

  // ───────────────────────── wiring ─────────────────────────
  function wire() {
    paintIcons();
    wireLog();
    $('#rows').addEventListener('click', onRowClick);
    $('#table').addEventListener('scroll', renderRows, { passive: true });
    new ResizeObserver(() => renderRows()).observe($('#table'));

    $('#filter').addEventListener('input', (e) => { S.query = e.target.value; refresh(); });
    const vt = $$('#viewtabs [role=tab]');
    vt.forEach((b) => { b.tabIndex = b.getAttribute('aria-selected') === 'true' ? 0 : -1; });
    $('#viewtabs').addEventListener('click', (e) => { const b = e.target.closest('[role=tab]'); if (b) setFilter(b.dataset.filter); });
    $('#viewtabs').addEventListener('keydown', (e) => {
      if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
      const i = vt.findIndex((b) => b.dataset.filter === S.filter);
      setFilter(vt[(i + (e.key === 'ArrowRight' ? 1 : vt.length - 1)) % vt.length].dataset.filter, true);
    });

    const addFolder = async () => addPaths(await pickFolders());
    $('#btn-add-2').onclick = addFolder;
    $('#btn-import').onclick = (e) => openFloating($('#import-pop'), e.currentTarget, '#mn-import-files');
    $('#mn-import-files').onclick = async () => { closeFloating(); addPaths(await pickFiles()); };
    $('#mn-import-folder').onclick = () => { closeFloating(); addFolder(); };

    // library menu
    $('#btn-library').onclick = () => toggleLibrary();
    document.addEventListener('pointerdown', (e) => { if (S.popOpen && !e.target.closest('.pop-wrap')) toggleLibrary(false); });
    $('#library-pop').addEventListener('keydown', (e) => { if (e.key === 'Escape') { toggleLibrary(false); $('#btn-library').focus(); } });
    $('#btn-scan').onclick = () => {
      const dirs = S.settings?.config.library_dirs || [];
      if (!dirs.length) { toast('Add a library folder first'); return; }
      toggleLibrary(false);
      addPaths(dirs);
    };
    $('#btn-lib-add').onclick = async () => {
      const picked = await pickFolders();
      if (!picked.length) return;
      await saveLibraryDirs([...new Set([...(S.settings.config.library_dirs || []), ...picked])]);
      addPaths(picked);
    };
    $('#libraries').addEventListener('click', async (e) => {
      const b = e.target.closest('[data-rm]');
      if (!b) return;
      const dirs = [...S.settings.config.library_dirs];
      dirs.splice(Number(b.dataset.rm), 1);
      await saveLibraryDirs(dirs);
    });

    $('#btn-run').onclick = runAll;
    $('#btn-settings').onclick = openSettings;
    $('#btn-theme').onclick = () => setTheme(document.documentElement.dataset.theme === 'light' ? 'dark' : 'light');

    $('#sel-all').onclick = toggleSelectAll;
    $('#sel-search').onclick = () => startSearch([...S.sel], false, S.replace);
    $('#sel-embed').onclick = () => embedIds([...S.sel]);
    $('#sel-files').onclick = (e) => openFileOptions(e.currentTarget, 'sel');
    $('#sel-more').onclick = (e) => openFloating($('#more-pop'), e.currentTarget, '#mn-drop');
    $('#mn-remove').onclick = () => { closeFloating(); removeLyrics([...S.sel]); };
    $('#mn-drop').onclick = async () => { closeFloating(); setTracks((await invoke('remove_tracks', { ids: [...S.sel] })).tracks); };
    $('#mn-clear-embedded').onclick = () => { closeFloating(); dropEmbedded(); };

    // lyric files options: format, replace, and the two actions
    $('#fp-kind').addEventListener('click', (e) => { const b = e.target.closest('[data-kind]'); if (!b) return; S.kind = b.dataset.kind; store.set('lf-kind', S.kind); paintFileOptions(); });
    $('#fp-replace').addEventListener('change', (e) => setReplace(e.target.checked));
    $('#fp-embed').onclick = () => { const ids = scopeIds(); closeFloating(); embedFromFiles(ids); };
    $('#fp-write').onclick = () => { const ids = scopeIds(); closeFloating(); writeFiles(ids); };
    document.addEventListener('pointerdown', (e) => { if (S.pop && !e.target.closest('.floating') && !S.pop.anchor.contains(e.target)) closeFloating(); });
    $('#file-pop').addEventListener('keydown', (e) => { if (e.key === 'Escape') { const a = S.pop?.anchor; closeFloating(); a?.focus(); } });
    $('#more-pop').addEventListener('keydown', (e) => { if (e.key === 'Escape') { const a = S.pop?.anchor; closeFloating(); a?.focus(); } });

    // player
    $('#pl-btn').onclick = togglePlay;
    const seek = $('#pl-seek');
    seek.addEventListener('input', () => { S.seeking = true; renderPlayer(); });
    seek.addEventListener('change', () => {
      const dur = (S.player?.duration) || activeTrack()?.duration || 0;
      if (dur) invoke('player_seek', { secs: (seek.value / 1000) * dur });
      S.seeking = false;
    });

    // lyrics toolbar
    const tid = () => S.active;
    $('#act-embed').onclick = () => embedIds([tid()]);
    $('#act-copy').onclick = async () => { try { await clip.write(shownLyrics()); toast('Lyrics copied to the clipboard', 'good'); } catch { toast('Clipboard unavailable', 'bad'); } };
    $('#act-edit').onclick = () => { setTab('edit', true); $('#edit-text').focus(); };
    $('#act-requeue').onclick = () => startSearch([tid()], false);
    $('#act-more').onclick = (e) => openFloating($('#track-pop'), e.currentTarget, '#tm-paste');
    // the file manager's own name, so the action says where the file will be shown
    const ua = navigator.userAgent;
    $('#tm-reveal-label').textContent = /Mac/.test(ua) ? 'Show in Finder' : /Windows/.test(ua) ? 'Show in Explorer' : 'Show in file manager';
    $('#tm-paste').onclick = () => { closeFloating(); pasteLyrics(); };
    $('#tm-files').onclick = () => { const a = $('#act-more'); closeFloating(); openFileOptions(a, 'active'); };
    $('#tm-reveal').onclick = () => { closeFloating(); invoke('reveal_track', { id: tid() }).catch((e) => toast(errText(e), 'bad')); };
    $('#tm-path').onclick = async () => { closeFloating(); try { await clip.write(activeTrack().path); toast('File path copied', 'good'); } catch { toast('Clipboard unavailable', 'bad'); } };
    $('#tm-remove').onclick = () => { closeFloating(); removeLyrics([tid()]); };
    for (const id of ['#import-pop', '#track-pop']) $(id).addEventListener('keydown', (e) => { if (e.key === 'Escape') { const a = S.pop?.anchor; closeFloating(); a?.focus(); } });
    $('#font-down').onclick = () => setLyricsSize(S.lyricsSize - 1);
    $('#font-up').onclick = () => setLyricsSize(S.lyricsSize + 1);
    setLyricsSize(Number(store.get('lf-lyrics-size')) || 14);
    S.tbMode = 'both';
    applyToolbarMode();

    const tabs = $$('#tabs [role=tab]');
    tabs.forEach((b) => { b.tabIndex = b.getAttribute('aria-selected') === 'true' ? 0 : -1; });
    $('#tabs').addEventListener('click', (e) => { const b = e.target.closest('[role=tab]'); if (b) setTab(b.dataset.tab); });
    $('#tabs').addEventListener('keydown', (e) => {
      if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
      const i = tabs.findIndex((b) => b.dataset.tab === S.tab);
      setTab(tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length].dataset.tab, true);
    });

    $('#edit-save').onclick = async () => {
      const ta = $('#edit-text');
      try {
        const fromFound = !!S.detail?.found?.plain && ta.value.trim() === S.detail.found.plain.trim();
        await invoke('save_lyrics', { id: Number(ta.dataset.id), text: ta.value, fromFound });
        toast('Saved to the file', 'good');
        setActive(Number(ta.dataset.id));
      } catch (e) { toast(errText(e), 'bad', 7000); }
    };
    $('#edit-revert').onclick = () => { const ta = $('#edit-text'); ta.value = ta.dataset.orig || ''; };
    $('#edit-paste').onclick = async () => {
      try { $('#edit-text').value = await clip.read(); toast('Pasted. Review it, then Save to file.'); } catch { toast('Clipboard unavailable', 'bad'); }
    };

    // web panel
    $('#web-go').onclick = goWeb;
    $('#web-ai').onclick = () => {
      const t = activeTrack();
      const url = ENGINES.google_ai(t ? `${t.artist} - ${t.title} lyrics` : $('#web-query').value);
      invoke('open_external', { url }).then(() => toast('Opened Google AI Mode in your browser', 'good')).catch((e) => toast(errText(e), 'bad'));
    };
    $('#web-query').addEventListener('keydown', (e) => { if (e.key === 'Enter') goWeb(); });
    $('#web-back').onclick = () => invoke('browser_back');
    $('#web-forward').onclick = () => invoke('browser_forward');
    $('#web-reload').onclick = () => invoke('browser_reload');
    $('#web-zoom-out').onclick = () => invoke('browser_zoom', { delta: -0.1 });
    $('#web-zoom-in').onclick = () => invoke('browser_zoom', { delta: 0.1 });
    $('#web-use').onclick = async () => {
      try {
        const text = await invoke('browser_selection');
        if (!text) { toast('Select the lyrics on the page first'); return; }
        $('#edit-text').value = text;
        setTab('edit', true);
        toast('Selection in the editor. Check it, then Save.', 'good');
      } catch (e) { toast(errText(e), 'bad'); }
    };
    $('#web-external').onclick = () => { const u = S.webUrl || urlFor($('#web-query').value); if (u) invoke('open_external', { url: u }).catch((e) => toast(errText(e), 'bad')); };
    $('#web-engine').addEventListener('change', () => { if ($('#web-query').value.trim()) goWeb(); });
    // Mobile view: pages load with a phone layout (default); the toggle rebuilds the panel at the same address.
    S.mobile = store.get('lf-mobile') !== 'off';
    const paintMobile = () => $('#web-mobile').setAttribute('aria-pressed', S.mobile);
    paintMobile();
    if (!S.mobile) invoke('browser_mobile', { mobile: false });
    $('#web-mobile').onclick = async () => {
      S.mobile = !S.mobile;
      store.set('lf-mobile', S.mobile ? 'on' : 'off');
      paintMobile();
      await invoke('browser_mobile', { mobile: S.mobile });
      browserOpen = false;
      const url = S.webUrl || urlFor($('#web-query').value);
      S.webUrl = url;
      if (browserWanted()) { browserOpen = true; invoke('browser_show', { url, rect: slotRect() }).catch((e) => toast(`Browser: ${errText(e)}`, 'bad')); watchRect(); }
    };

    document.addEventListener('keydown', (e) => {
      if (S.modalOpen) return;
      const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName);
      const inControl = document.activeElement?.closest('#tabs, #viewtabs, #stabs, #splitter, .toolbar, .player, .popover, button');
      if (e.key === 'Escape') {
        if (S.pop) closeFloating();
        else if (S.sel.size) { S.sel.clear(); renderRows(); renderSelbar(); renderTop(); }
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a' && !typing) {
        e.preventDefault();
        S.sel = new Set(S.view.map((t) => t.id));
        renderRows(); renderSelbar(); renderTop();
      } else if (e.key === ' ' && !typing && !inControl && S.active != null) {
        e.preventDefault();
        togglePlay();
      } else if (!typing && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key) && S.view.length && !inControl) {
        e.preventDefault();
        const i = S.view.findIndex((t) => t.id === S.active);
        const n = e.key === 'Home' ? 0 : e.key === 'End' ? S.view.length - 1 : Math.min(S.view.length - 1, Math.max(0, i + (e.key === 'ArrowDown' ? 1 : -1)));
        const id = S.view[n].id;
        if (e.shiftKey && S.anchor != null) {
          const a = S.view.findIndex((t) => t.id === S.anchor);
          S.sel = new Set(S.view.slice(Math.min(a, n), Math.max(a, n) + 1).map((t) => t.id));
        } else selectOnly(id);
        setActive(id);
        const box = $('#table');
        if (n * ROW_H < box.scrollTop) box.scrollTop = n * ROW_H;
        else if ((n + 1) * ROW_H > box.scrollTop + box.clientHeight) box.scrollTop = (n + 1) * ROW_H - box.clientHeight;
        renderRows(); renderSelbar(); renderTop();
      }
    });

    S.kind = ['auto', 'lrc', 'txt'].includes(store.get('lf-kind')) ? store.get('lf-kind') : 'auto';
    invoke('get_settings').then((s) => {
      S.settings = s;
      if (s.startup_notice) { logEvent(s.startup_notice); toast(s.startup_notice, 'bad', 15000); }
      S.replace = !!s.config.overwrite;
      renderTop();
      $('#web-engine').innerHTML = engineOptions(s.engines, s.config.search_engine);
      if (!canEmbed()) {
        // no in-window page here: keep the search box and the Browser button, drop the page controls
        ['#web-back', '#web-forward', '#web-reload', '#web-zoom-out', '#web-zoom-in', '#web-mobile', '#web-use'].forEach((sel) => { $(sel).hidden = true; });
        $('#web-hint').textContent = 'Copy the lyrics from your browser, then paste them in Edit.';
        $('#webslot').innerHTML = '<span>Search opens the results in your browser.</span>';
      }
      renderLibraries();
    });

    wireSettings();
    wireSplitter();
    setTheme(store.get('lf-theme') || (matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'));

    // events from the backend
    T.event.listen('session-log', (e) => logEvent(e.payload));
    T.event.listen('operation-warning', (e) => { logEvent(e.payload); toast(e.payload, 'bad', 8000); });
    T.event.listen('track', (e) => {
      const t = e.payload;
      S.tracks.set(t.id, t);
      if (t.status === 'found') logEvent(`Found ${t.name} via ${pname(t.provider)}`);
      if (t.status === 'notfound') logEvent(`Not found: ${t.name}${t.note ? '\n' + t.note : ''}`);
      if (t.status === 'error') logEvent(`Error: ${t.name}${t.note ? '\n' + t.note : ''}`);
      if (t.status === 'written') logEvent(`Wrote lyrics for ${t.name}`);
      if (t.status === 'searching') S.current = `${t.title}${t.provider ? ' via ' + pname(t.provider) : ''}`;
      computeView();
      recount();
      $('#spacer').style.height = `${S.view.length * ROW_H}px`;
      renderRows();
      renderSelbar();
      renderTop();
      if (S.active === t.id) { clearTimeout(wire._t); wire._t = setTimeout(() => setActive(t.id), 120); }
    });
    T.event.listen('progress', (e) => { S.progress = e.payload; if (!e.payload.running) S.progressAt = Date.now(); renderTop(); });
    T.event.listen('stats', () => recount());
    // A source wants a bot check passed: a browser window opens, solves itself if it can, and the engine reuses its cookies.
    // A source sits behind a bot check the hidden browser could not pass by itself: say so in the status
    // bar; the check only opens when the person chooses Verify. Nothing pops up on its own.
    const showCheck = (host) => {
      $('#check-text').textContent = `${host} needs a quick human check`;
      $('#checkbar').hidden = false;
    };
    T.event.listen('challenge', (e) => {
      const { provider, url } = e.payload;
      logEvent(`${pname(provider)} is waiting for a bot check`);
      invoke('open_challenge', { url, provider }).catch((err) => logEvent(`Bot check error: ${errText(err)}`));
    });
    T.event.listen('check-needed', (e) => { logEvent(`${e.payload.host} needs a human check`); showCheck(e.payload.host); });
    T.event.listen('check-cleared', (e) => {
      $('#checkbar').hidden = true;
      logEvent(`${e.payload.host} check passed`);
      toast(`${e.payload.host} check passed`, 'good');
    });
    $('#check-open').onclick = () => invoke('show_check').catch((err) => toast(errText(err), 'bad'));
    $('#check-skip').onclick = () => { $('#checkbar').hidden = true; invoke('dismiss_check'); };
    // menu-bar / tray actions
    T.event.listen('tray-action', (e) => {
      if (e.payload === 'stop') { if (S.progress?.running) invoke('cancel_search'); return; }
      if (e.payload === 'find-all' && !S.progress?.running) {
        const ids = runIds();
        if (ids.length) startSearch(ids, true, S.replace); else toast('Every listed track has lyrics');
      }
    });
    T.event.listen('player', (e) => {
      S.player = e.payload;
      if (e.payload.state === 'error' && e.payload.error) toast(e.payload.error, 'bad', 6000);
      if (S.active != null && e.payload.id === S.active) renderPlayer();
    });

    // drops on the window, and files handed over by the OS (dropped on the app icon, "Open with")
    const veil = $('#dropveil');
    T.event.listen('tauri://drag-enter', () => veil.classList.add('on'));
    T.event.listen('tauri://drag-over', () => veil.classList.add('on'));
    T.event.listen('tauri://drag-leave', () => veil.classList.remove('on'));
    T.event.listen('tauri://drag-drop', (e) => { veil.classList.remove('on'); addPaths(e.payload.paths || []); });
    const takeOpened = async () => { const p = await invoke('take_pending_open'); if (p.length) addPaths(p); };
    T.event.listen('open-paths', takeOpened);

    invoke('list_tracks').then((r) => { setTracks(r.tracks); takeOpened(); });
  }

  // Hook used by the built-in self-test (LYRIXEARCH_SELFTEST=1) and for debugging.
  window.__lf = {
    addPaths,
    state: S,
    urlFor,
    selectFirst: () => { const t = S.view[0]; if (t) { selectOnly(t.id); setActive(t.id); renderRows(); renderSelbar(); } return !!t; },
    selectId: (id) => { selectOnly(id); setActive(id); renderRows(); renderSelbar(); },
    setTab,
  };

  document.addEventListener('DOMContentLoaded', wire);
})();
