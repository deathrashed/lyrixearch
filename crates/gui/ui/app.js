(() => {
  'use strict';
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args || {});
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];
  // row heights per layout; album headers in the Albums layout
  const HEIGHTS = { comfortable: 56, compact: 32, albums: 32, album: 46 };
  const AUDIO_EXT = ['mp3', 'flac', 'm4a', 'mp4', 'aac', 'ogg', 'oga', 'opus', 'wma', 'wav', 'aif', 'aiff', 'ape', 'wv'];

  const { ICONS, icon, esc, errText, store } = window.LX;
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
    layout: 'comfortable', sort: 'artist', collapsed: new Set(), items: [], offs: [0],
    pane: 'overview', act: null,
  };

  const fmtTime = (s) => { s = Math.max(0, Math.round(s || 0)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };

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
  const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : s);
  function stateLabel(t) { return cap(stateWord(t)); }
  function stateWord(t) {
    switch (t.status) {
      case 'queued': return 'queued';
      case 'searching': return t.provider ? `searching · ${pname(t.provider)}` : 'searching';
      case 'found': return `found · ${pname(t.provider)}${t.synced ? ' · synced' : ''}`;
      case 'written': return t.provider ? `embedded · ${pname(t.provider)}` : 'embedded';
      case 'notfound': return 'no match';
      case 'error': return 'failed';
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
    // Found, No match and Failed only show while they hold tracks (or are the one being shown)
    for (const k of ['found', 'notfound', 'error']) $(`#viewtabs [data-filter="${k}"]`).hidden = !n[k] && S.filter !== k;
    renderPane();
  }

  // Tracks the big button searches: everything in the list below it, minus the ones that already
  // have lyrics unless replacing is on.
  const runIds = () => S.view.filter((t) => S.replace || !t.has_lyrics).map((t) => t.id);

  function computeView() {
    const q = S.query.trim().toLowerCase();
    const p = predicates[S.filter];
    S.view = [...S.tracks.values()].filter((t) => p(t) && (!q || `${t.artist} ${t.title} ${t.album || ''} ${t.name}`.toLowerCase().includes(q)));
    const by = (x, y) => (x || '').localeCompare(y || '', undefined, { numeric: true, sensitivity: 'base' });
    const albumOrder = (a, b) => by(a.album_artist || a.artist, b.album_artist || b.artist) || by(a.album, b.album) || (a.track_number || 0) - (b.track_number || 0) || by(a.name, b.name);
    const sorts = {
      artist: (a, b) => by(a.artist, b.artist) || by(a.album, b.album) || (a.track_number || 0) - (b.track_number || 0) || by(a.name, b.name),
      album: (a, b) => by(a.album, b.album) || albumOrder(a, b),
      title: (a, b) => by(a.title, b.title) || by(a.artist, b.artist),
      status: (a, b) => statusRank(a) - statusRank(b) || by(a.artist, b.artist) || by(a.title, b.title),
      added: (a, b) => b.id - a.id,
    };
    // the Albums layout always groups by album artist and album
    S.view.sort(S.layout === 'albums' ? albumOrder : sorts[S.sort] || sorts.artist);
  }

  // Status order: what needs attention first
  function statusRank(t) {
    return { searching: 0, queued: 1, found: 2, error: 3, notfound: 4 }[t.status] ?? (t.has_lyrics ? 6 : 5);
  }

  // ───────────────────────── list items: track rows, and album headers in the Albums layout ─────────────────────────
  const albumKey = (t) => `${(t.album_artist || t.artist || '').toLowerCase()}\u0001${(t.album || '').toLowerCase()}`;

  function buildItems() {
    const items = [];
    if (S.layout === 'albums') {
      let cur = null;
      for (const t of S.view) {
        const key = albumKey(t);
        if (!cur || cur.key !== key) {
          cur = { kind: 'album', key, album: t.album || 'No album', artist: t.album_artist || t.artist || 'Unknown artist', first: t.id, n: 0, have: 0, tracks: [] };
          items.push(cur);
        }
        cur.n++;
        if (t.has_lyrics) cur.have++;
        cur.tracks.push(t.id);
        if (!S.collapsed.has(key)) items.push({ kind: 'track', t });
      }
    } else {
      for (const t of S.view) items.push({ kind: 'track', t });
    }
    const h = S.layout === 'compact' || S.layout === 'albums' ? HEIGHTS.compact : HEIGHTS.comfortable;
    const offs = [0];
    for (const it of items) offs.push(offs[offs.length - 1] + (it.kind === 'album' ? HEIGHTS.album : h));
    S.items = items;
    S.offs = offs;
    $('#spacer').style.height = `${offs[offs.length - 1]}px`;
  }

  /** First item whose bottom is below `y`. */
  function itemAt(y) {
    let lo = 0, hi = S.items.length;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (S.offs[mid + 1] <= y) lo = mid + 1; else hi = mid; }
    return lo;
  }

  /** Keep a track's row in view (opening its album when it is folded). */
  function scrollToTrack(id) {
    let i = S.items.findIndex((it) => it.kind === 'track' && it.t.id === id);
    if (i < 0 && S.layout === 'albums') {
      const t = S.tracks.get(id);
      if (t) { S.collapsed.delete(albumKey(t)); buildItems(); i = S.items.findIndex((it) => it.kind === 'track' && it.t.id === id); }
    }
    if (i < 0) return;
    const box = $('#table');
    const top = S.offs[i], bottom = S.offs[i + 1];
    if (top < box.scrollTop) box.scrollTop = top;
    else if (bottom > box.scrollTop + box.clientHeight) box.scrollTop = bottom - box.clientHeight;
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
    buildItems();
    $('#empty').hidden = S.tracks.size > 0;
    renderRows();
    renderSelbar();
    renderTop();
  }

  const rowBtns = (t) => `<span class="btns">
          <button data-act="search" title="Search lyrics for this track" aria-label="Search lyrics for ${esc(t.title)}">${icon('search')}</button>
          <button data-act="reveal" title="Show in file browser" aria-label="Show ${esc(t.name)} in the file browser">${icon('reveal')}</button>
        </span>`;

  function rowHtml(t) {
    const cls = ['row', S.layout === 'comfortable' ? 'c' : 'k', S.sel.has(t.id) ? 'sel' : '', S.active === t.id ? 'active' : ''].join(' ');
    const attrs = `class="${cls}" role="option" id="row-${t.id}" aria-selected="${S.sel.has(t.id) || S.active === t.id}" data-id="${t.id}" title="${esc(t.path)}"`;
    if (S.layout === 'comfortable') {
      const cover = S.covers.get(t.id);
      const sub = [t.artist, t.album].filter(Boolean).map(esc).join(' · ');
      return `<div ${attrs} style="height:${HEIGHTS.comfortable}px">
        <div class="thumb">${cover ? `<img alt="" src="${cover}">` : icon('note')}</div>
        <div class="lines"><span class="t1"><i class="led ${lampClass(t)}" aria-hidden="true"></i><span>${esc(t.title || t.name)}</span></span><span class="t2">${sub || '&nbsp;'}</span></div>
        <div class="side"><span class="src ${stateTone(t)}">${esc(stateLabel(t))}</span><span class="dur">${t.duration ? fmtTime(t.duration) : ''}</span></div>
        ${rowBtns(t)}</div>`;
    }
    const lead = S.layout === 'albums' ? `<span class="no">${t.track_number || ''}</span>` : '';
    return `<div ${attrs} style="height:${HEIGHTS.compact}px">
      <i class="led ${lampClass(t)}" aria-hidden="true"></i>${lead}
      <span class="k-title">${esc(t.title || t.name)}</span>
      <span class="k-artist">${esc(t.artist)}</span>
      ${S.layout === 'albums' ? '' : `<span class="k-album">${esc(t.album || '')}</span>`}
      <span class="src ${stateTone(t)}">${esc(stateLabel(t))}</span>
      <span class="dur">${t.duration ? fmtTime(t.duration) : ''}</span>
      ${rowBtns(t)}</div>`;
  }

  function albumHtml(a) {
    const open = !S.collapsed.has(a.key);
    const cover = S.covers.get(a.first);
    const done = a.have === a.n;
    return `<div class="grp${open ? ' open' : ''}" role="presentation" data-key="${esc(a.key)}" style="height:${HEIGHTS.album}px">
      <button class="grp-toggle" aria-expanded="${open}" aria-label="${open ? 'Fold' : 'Unfold'} ${esc(a.album)}">${icon('right')}</button>
      <div class="thumb sm">${cover ? `<img alt="" src="${cover}">` : icon('library')}</div>
      <div class="grp-text"><b>${esc(a.album)}</b><span>${esc(a.artist)}</span></div>
      <span class="grp-count${done ? ' done' : ''}">${a.have}/${a.n}</span>
      <span class="grp-bar" aria-hidden="true"><i style="width:${Math.round((a.have / a.n) * 100)}%"></i></span>
    </div>`;
  }

  function renderRows() {
    const box = $('#table');
    const top = box.scrollTop;
    const h = box.clientHeight || 600;
    const start = Math.max(0, itemAt(top) - 6);
    const end = Math.min(S.items.length, itemAt(top + h) + 6);
    const rows = $('#rows');
    rows.style.transform = `translateY(${S.offs[start] || 0}px)`;
    const slice = S.items.slice(start, end);
    rows.innerHTML = slice.map((it) => (it.kind === 'album' ? albumHtml(it) : rowHtml(it.t))).join('');
    if (S.active != null) box.setAttribute('aria-activedescendant', `row-${S.active}`); else box.removeAttribute('aria-activedescendant');
    if (S.layout !== 'compact') wantCovers(slice.map((it) => (it.kind === 'album' ? S.tracks.get(it.first) : it.t)).filter(Boolean));
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
    const grp = ev.target.closest('.grp');
    if (grp) {
      const key = grp.dataset.key;
      S.collapsed.has(key) ? S.collapsed.delete(key) : S.collapsed.add(key);
      buildItems(); renderRows();
      return;
    }
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
  // right-click on a row: act on the selection, or on just that row when it is not selected
  function openContextMenu(ev) {
    const row = ev.target.closest('.row');
    if (!row) return;
    ev.preventDefault();
    const id = Number(row.dataset.id);
    if (!S.sel.has(id)) { selectOnly(id); setActive(id); renderRows(); renderSelbar(); renderTop(); }
    const n = S.sel.size, one = n === 1;
    const el = $('#ctx-pop');
    $('#cx-copy').hidden = !one;
    $('#cx-reveal').hidden = !one;
    $('#cx-path-label').textContent = one ? 'Copy file path' : `Copy ${n} file paths`;
    $('#cx-remove-label').textContent = one ? 'Delete lyrics from file…' : `Delete lyrics from ${n} files…`;
    $('#cx-embed').disabled = ![...S.sel].some((i) => S.tracks.get(i)?.status === 'found');
    closeFloating(false);
    if (S.libOpen) toggleLibrary(false);
    el.hidden = false;
    const w = el.offsetWidth, h = el.offsetHeight;
    el.style.left = `${Math.round(Math.max(8, Math.min(ev.clientX, innerWidth - w - 8)))}px`;
    el.style.top = `${Math.round(ev.clientY + h > innerHeight - 8 ? Math.max(8, ev.clientY - h) : ev.clientY)}px`;
    S.pop = { el, anchor: row };
    S.popOpen = true;
    syncBrowser();
    $$('button:not([hidden]):not(:disabled)', el)[0]?.focus();
  }

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
    if (!t) { renderPane(); return syncBrowser(); }

    $('#d-title').textContent = t.name;
    $('#d-title').title = t.path;
    $('#d-led').className = `led ${lampClass(t)}`;
    const tags = [`<span class="tag ${stateTone(t)}">${esc(stateLabel(t))}</span>`, `<span class="tag">${esc((t.format || '').toUpperCase())}</span>`];
    if (t.status === 'found' || t.status === 'written') tags.push(`<span class="tag">${Math.round((t.score || 0) * 100)}%</span>`);
    if (t.synced) tags.push('<span class="tag lit">Synced</span>');
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
      pre.textContent = t.status === 'notfound' ? 'No source had these lyrics. Try the Web tab, paste them in, or embed a matching .lrc or .txt file. Details shows what each source said.' : 'No lyrics yet.';
    }
    $('#act-embed').hidden = !d.found;
    $('#act-copy').disabled = !text;
    $('#tm-remove').hidden = !t.has_lyrics;

    const ta = $('#edit-text');
    ta.dataset.id = t.id;
    ta.value = d.found?.plain || d.embedded || '';
    ta.dataset.orig = ta.value;

    renderInfo(t, d);

    const q = $('#web-query');
    if (q.dataset.for !== String(t.id)) {
      q.value = `${t.artist} - ${t.title} lyrics`;
      q.dataset.for = String(t.id);
      S.webUrl = null;
    }
    // a new track loads its own search instead of keeping the last track's page
    syncBrowser(S.webUrl === null);
  }

  // ───────────────────────── right pane without a track: library overview and fetch activity ─────────────────────────
  function ring(frac, big, small, tone = 'var(--ok)', size = 120) {
    const r = 46, c = 2 * Math.PI * r, f = Math.max(0, Math.min(1, frac || 0));
    return `<svg class="ring" viewBox="0 0 108 108" width="${size}" height="${size}" role="img" aria-label="${esc(big)} ${esc(small)}"><circle cx="54" cy="54" r="${r}" class="track"/><circle cx="54" cy="54" r="${r}" class="fill" style="stroke:${tone};stroke-dasharray:${(c * f).toFixed(1)} ${c.toFixed(1)}"/><text x="54" y="55" text-anchor="middle" class="big">${esc(big)}</text><text x="54" y="73" text-anchor="middle" class="small">${esc(small)}</text></svg>`;
  }
  const tile = (n, label, tone, filter) => `<button class="tile" ${filter ? `data-filter-go="${filter}"` : 'disabled'}><i class="led ${tone}" aria-hidden="true"></i><b>${n}</b><span>${label}</span></button>`;
  const fmtDur = (ms) => { const sec = Math.max(0, Math.round(ms / 1000)); const h = Math.floor(sec / 3600); const m = Math.floor((sec % 3600) / 60); return h ? `${h}h ${m}m` : `${m}:${String(sec % 60).padStart(2, '0')}`; };

  function setPane(p) {
    S.pane = p;
    $$('#pane-tabs [role=tab]').forEach((b) => b.setAttribute('aria-selected', b.dataset.pane === p));
    renderPane();
  }

  let paneFrame = 0;
  function renderPane() {
    if (paneFrame) return;
    paneFrame = requestAnimationFrame(() => {
      paneFrame = 0;
      if (S.active != null || !$('#overview')) return;
      $('#overview').innerHTML = S.pane === 'activity' ? activityHtml() : overviewHtml();
    });
  }

  function overviewHtml() {
    const all = [...S.tracks.values()];
    if (!all.length) return '<div class="pane-empty"><h3>Your library at a glance</h3><p>Import music and this shows how many tracks have lyrics, which albums need work, and how each fetch went.</p></div>';
    const has = all.filter((t) => t.has_lyrics).length;
    const count = (st) => all.filter((t) => t.status === st).length;
    const cards = [];
    cards.push(`<section class="dx-card ov-top">${ring(has / all.length, `${Math.round((has / all.length) * 100)}%`, 'with lyrics')}
      <div class="ov-sum"><b>${has} of ${all.length} tracks</b><span>have lyrics in the file</span>
      <div class="tiles">${tile(has, 'Embedded', 'has', 'has')}${tile(all.length - has, 'Missing', 'missing', 'missing')}${count('found') ? tile(count('found'), 'Found', 'found', 'found') : ''}${count('notfound') ? tile(count('notfound'), 'No match', 'notfound', 'notfound') : ''}${count('error') ? tile(count('error'), 'Failed', 'error', 'error') : ''}</div></div></section>`);
    // albums that need the most work
    const albums = new Map();
    for (const t of all) {
      const k = albumKey(t);
      const a = albums.get(k) || { album: t.album || 'No album', artist: t.album_artist || t.artist, n: 0, have: 0 };
      a.n++; if (t.has_lyrics) a.have++;
      albums.set(k, a);
    }
    const worst = [...albums.values()].filter((a) => a.have < a.n).sort((a, b) => (b.n - b.have) - (a.n - a.have) || a.have / a.n - b.have / b.n).slice(0, 8);
    if (worst.length) {
      cards.push(`<section class="dx-card"><h3>Albums missing lyrics <span>${[...albums.values()].filter((a) => a.have < a.n).length} of ${albums.size}</span></h3><ul class="ov-albums">${worst.map((a) => `<li><button data-album="${esc(a.album)}" title="Show this album's tracks"><span class="ov-name"><b>${esc(a.album)}</b><span>${esc(a.artist)}</span></span><span class="grp-bar"><i style="width:${Math.round((a.have / a.n) * 100)}%"></i></span><span class="ov-miss">${a.n - a.have} missing</span></button></li>`).join('')}</ul></section>`);
    } else {
      cards.push('<section class="dx-card"><h3>Albums</h3><p class="dx-note">Every album has lyrics.</p></section>');
    }
    if (S.act?.end) cards.push(`<section class="dx-card"><h3>Last fetch <span>${fmtDur(S.act.end - S.act.start)}</span></h3>${sourcesHtml()}<button class="btn sm" data-pane-go="activity">Open activity</button></section>`);
    return cards.join('');
  }

  function sourcesHtml() {
    const src = Object.entries(S.act?.sources || {}).sort((a, b) => b[1].found - a[1].found || b[1].n - a[1].n);
    if (!src.length) return '<p class="dx-note">No source has answered yet.</p>';
    const max = Math.max(...src.map(([, v]) => v.n));
    return `<ul class="act-src">${src.map(([id, v]) => {
      const w = (k) => ((v[k] || 0) / max) * 100;
      return `<li><b>${esc(pname(id))}</b><span class="stack" role="img" aria-label="${v.found} found, ${v.miss} no match, ${v.fail + v.timeout} failed"><i class="f" style="width:${w('found')}%"></i><i class="m" style="width:${w('miss')}%"></i><i class="t" style="width:${w('timeout')}%"></i><i class="x" style="width:${w('fail')}%"></i></span><span class="act-num">${v.found} found · ${v.n ? Math.round(v.ms / v.n) : 0} ms</span></li>`;
    }).join('')}</ul><div class="act-legend"><span><i class="f"></i>Found</span><span><i class="m"></i>No match</span><span><i class="t"></i>Timed out</span><span><i class="x"></i>Failed</span></div>`;
  }

  function activityHtml() {
    const a = S.act;
    if (!a) return '<div class="pane-empty"><h3>No fetch yet</h3><p>Press Fetch, or search a selection, and this shows the progress, what each source is doing, and every result as it lands.</p></div>';
    const p = S.progress || { done: 0, total: 0, found: 0, written: 0, missed: 0, running: false };
    const now = a.end || Date.now();
    const elapsed = now - a.start;
    const rate = p.done / Math.max(elapsed / 60000, 1 / 60);
    const left = p.running && rate > 0 ? ((p.total - p.done) / rate) * 60000 : 0;
    const all = [...S.tracks.values()];
    const searching = all.filter((t) => t.status === 'searching');
    const queued = all.filter((t) => t.status === 'queued').length;
    const failed = a.recent.filter((r) => r.status === 'error').length;
    const cards = [];
    cards.push(`<section class="dx-card ov-top">${ring(p.total ? p.done / p.total : 0, `${p.done}/${p.total}`, p.running ? 'searched' : 'done', p.running ? 'var(--accent)' : 'var(--ok)')}
      <div class="ov-sum"><b>${p.running ? 'Fetching lyrics' : 'Fetch finished'}</b>
      <span>${fmtDur(elapsed)} elapsed · ${Math.round(rate)} tracks a minute${p.running && left ? ` · about ${fmtDur(left)} left` : ''}</span>
      <div class="tiles">${tile(p.found, 'Found', 'found', null)}${tile(p.written, 'Embedded', 'has', null)}${tile(p.missed, 'No match', 'notfound', null)}${tile(failed, 'Failed', 'error', null)}</div>
      ${p.running ? '<button class="btn sm" data-act-stop>Stop</button>' : ''}</div></section>`);
    if (p.running) {
      cards.push(`<section class="dx-card"><h3>Searching now <span>${searching.length} at once · ${queued} waiting</span></h3>${searching.length ? `<ul class="act-now">${searching.slice(0, 12).map((t) => `<li data-go="${t.id}"><i class="led searching" aria-hidden="true"></i><span class="act-t"><b>${esc(t.title)}</b> ${esc(t.artist)}</span><span class="pill">${esc(pname(t.provider) || 'starting')}</span></li>`).join('')}</ul>` : '<p class="dx-note">Starting…</p>'}</section>`);
    }
    cards.push(`<section class="dx-card"><h3>Sources</h3>${sourcesHtml()}</section>`);
    if (a.recent.length) {
      const word = { found: 'Found', written: 'Embedded', notfound: 'No match', error: 'Failed' };
      const lamp = { found: 'found', written: 'has', notfound: 'notfound', error: 'error' };
      cards.push(`<section class="dx-card"><h3>Latest results</h3><ul class="act-now">${a.recent.slice(0, 14).map((r) => { const t = S.tracks.get(r.id); return t ? `<li data-go="${t.id}"><i class="led ${lamp[r.status]}" aria-hidden="true"></i><span class="act-t"><b>${esc(t.title)}</b> ${esc(t.artist)}</span><span class="act-num">${word[r.status]}${r.provider && r.status !== 'notfound' && r.status !== 'error' ? ` · ${esc(pname(r.provider))}` : ''}</span></li>` : ''; }).join('')}</ul></section>`);
    }
    return cards.join('');
  }

  // ───────────────────────── Tidy: text clean-ups that open in Edit ─────────────────────────
  /** What the tools work on: the editor when it has unsaved changes, otherwise the shown lyrics. */
  const editableText = () => { const ta = $('#edit-text'); return ta.value !== ta.dataset.orig ? ta.value : shownLyrics(); };
  const TIDY = {
    // every line followed by a blank line (a common scraping artefact): close the gaps, keep verses
    spacing(t) {
      const lines = t.replace(/\r\n?/g, '\n').split('\n');
      const blanks = lines.filter((l) => !l.trim()).length;
      if (blanks >= lines.length / 3) {
        return t.replace(/\r\n?/g, '\n').replace(/\n[ \t]*\n(?![ \t]*\n)/g, '\n').replace(/\n([ \t]*\n){2,}/g, '\n\n').trim();
      }
      return t.replace(/\n([ \t]*\n){2,}/g, '\n\n').trim();
    },
    sections: (t) => t.split('\n').filter((l) => !/^\s*[\[(]\s*(verse|chorus|pre-?chorus|post-?chorus|bridge|intro|outro|hook|refrain|interlude|instrumental|solo|guitar solo|breakdown|coda|skit|spoken|part)\b[^\])]*[\])]\s*$/i.test(l)).join('\n').replace(/\n{3,}/g, '\n\n').trim(),
    timestamps: (t) => t.split('\n').map((l) => l.replace(/\[\d{1,3}:\d{1,2}(?:[.:]\d{1,3})?\]/g, '').replace(/<\d{1,3}:\d{1,2}(?:[.:]\d{1,3})?>/g, '').replace(/^\[(ar|ti|al|by|offset|length|re|ve|au):[^\]]*\]$/i, '').trimEnd()).join('\n').replace(/^\n+/, '').replace(/\n{3,}/g, '\n\n').trim(),
    credits: (t) => t.split('\n').filter((l) => !/^\s*(written by|writer|songwriters?|composer|composed by|lyricist|lyrics by|music by|produced by|producer|arranged by|作词|作詞|作曲|编曲|編曲)\s*[:：]/i.test(l)).join('\n').trim(),
    spaces: (t) => t.split('\n').map((l) => l.replace(/[ \t\u00a0]+/g, ' ').trim()).join('\n').replace(/\n{3,}/g, '\n\n').trim(),
  };

  // ───────────────────────── Details tab: match, lyrics timeline, search trail, file ─────────────────────────
  const LRC_TIME = /\[(\d{1,3}):(\d{1,2}(?:[.:]\d{1,3})?)\]/g;
  /** Seconds of every timed line in LRC text. */
  function lrcTimes(lrc) {
    const out = [];
    for (const line of String(lrc || '').split('\n')) {
      if (!line.replace(LRC_TIME, '').trim()) continue; // a timestamp alone marks a pause, not a line
      for (const m of line.matchAll(LRC_TIME)) out.push(Number(m[1]) * 60 + Number(m[2].replace(':', '.')));
    }
    return out.sort((a, b) => a - b);
  }
  const fmtBytes = (n) => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
  const fmtMs = (ms) => (ms >= 1000 ? `${(ms / 1000).toFixed(1)} s` : `${ms} ms`);

  function gauge(score) {
    const pct = Math.round(Math.max(0, Math.min(1, score)) * 100);
    const r = 26, c = 2 * Math.PI * r;
    const tone = pct >= 90 ? 'var(--ok)' : pct >= 80 ? 'var(--accent)' : 'var(--led-amber)';
    return `<svg class="gauge" viewBox="0 0 64 64" role="img" aria-label="Match ${pct}%"><circle cx="32" cy="32" r="${r}" class="track"/><circle cx="32" cy="32" r="${r}" class="fill" style="stroke:${tone};stroke-dasharray:${(c * pct / 100).toFixed(1)} ${c.toFixed(1)}"/><text x="32" y="37" text-anchor="middle">${pct}%</text></svg>`;
  }

  function trailTone(result) {
    if (result === 'found') return 'on';
    if (result === 'searching') return 'searching';
    if (result.startsWith('skipped')) return 'off';
    if (result === 'no match' || result.startsWith('only unsynced')) return 'idle';
    return 'error';
  }
  function trailText(result) {
    if (result === 'found') return 'Found';
    if (result === 'searching') return 'Searching…';
    if (result === 'no match') return 'No match';
    return result.charAt(0).toUpperCase() + result.slice(1);
  }

  function renderInfo(t, d) {
    const cards = [];
    const f = d.found;
    if (f || t.has_lyrics) {
      const score = f ? f.score : t.score;
      const src = f ? pname(f.source) : t.provider ? pname(t.provider) : 'In the file';
      cards.push(`<section class="dx-card"><h3>Match</h3><div class="dx-match">${score ? gauge(score) : ''}<div class="dx-match-text">
        <b>${esc(src)}</b>${f ? `<span>${esc(f.matched_artist)} — ${esc(f.matched_title)}</span>` : '<span>Lyrics embedded in the file</span>'}
        ${f?.url ? `<a class="link" href="#" data-open="${esc(f.url)}">Open source page</a>` : ''}</div></div></section>`);
    }
    const text = f?.plain || d.embedded || '';
    if (text) {
      const lines = text.split('\n').filter((l) => l.trim());
      const words = lines.reduce((n, l) => n + l.trim().split(/\s+/).length, 0);
      const verses = text.split(/\n\s*\n/).filter((v) => v.trim()).length;
      const times = lrcTimes(f?.synced || d.embedded_synced);
      const dur = t.duration || (times.length ? times[times.length - 1] + 5 : 0);
      let timeline = '';
      if (times.length && dur) {
        const ticks = times.map((s) => `<i style="left:${Math.min(100, (s / dur) * 100).toFixed(2)}%"></i>`).join('');
        const playing = S.player && S.player.id === t.id && S.player.duration ? `<b class="dx-head" style="left:${Math.min(100, (S.player.position / S.player.duration) * 100).toFixed(2)}%"></b>` : '';
        timeline = `<div class="dx-timeline" role="img" aria-label="${times.length} timed lines between ${fmtTime(times[0])} and ${fmtTime(times[times.length - 1])}">${ticks}${playing}</div>
          <div class="dx-axis"><span>0:00</span><span>First line ${fmtTime(times[0])} · last ${fmtTime(times[times.length - 1])}</span><span>${fmtTime(dur)}</span></div>`;
      }
      const stat = (n, label) => `<div class="dx-stat"><b>${n}</b><span>${label}</span></div>`;
      cards.push(`<section class="dx-card"><h3>Lyrics</h3><div class="dx-stats">${stat(lines.length, 'lines')}${stat(words, 'words')}${stat(verses, verses === 1 ? 'verse' : 'verses')}${stat(times.length ? 'Synced' : 'Plain', times.length ? `${times.length} timed lines` : 'no timing')}</div>${timeline}</section>`);
    }
    const trail = d.trail || [];
    if (trail.length) {
      const max = Math.max(1, ...trail.map((a) => a.millis));
      const total = trail.reduce((n, a) => n + a.millis, 0);
      const rows = trail.map((a) => `<li><i class="led ${trailTone(a.result)}" aria-hidden="true"></i><b>${esc(pname(a.provider))}</b>
        <span class="dx-why" title="${esc(a.result)}">${esc(trailText(a.result))}</span>
        <span class="dx-bar"><i style="width:${a.millis ? Math.max(3, (a.millis / max) * 100).toFixed(1) : 0}%"></i></span><span class="dx-ms">${a.millis ? fmtMs(a.millis) : ''}</span></li>`).join('');
      cards.push(`<section class="dx-card"><h3>Search trail <span>${trail.length} source${trail.length === 1 ? '' : 's'} · ${fmtMs(total)}</span></h3><ul class="dx-trail">${rows}</ul></section>`);
    } else if (t.note) {
      cards.push(`<section class="dx-card"><h3>Last search</h3><p class="dx-note">${esc(t.note).replace(/\n/g, '<br>')}</p></section>`);
    }
    const rows = [['Artist', t.artist], ['Title', t.title], ['Album', t.album || '—'], ['Duration', t.duration ? fmtTime(t.duration) : '—'], ['Format', (t.format || '').toUpperCase()]];
    if (d.size) rows.push(['Size', fmtBytes(d.size)]);
    rows.push(['File', t.name], ['Folder', t.folder]);
    cards.push(`<section class="dx-card"><h3>File</h3><dl class="info">${rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd${k === 'Folder' ? ' class="link"' : ''}>${esc(v)}</dd>`).join('')}</dl></section>`);
    $('#info').innerHTML = cards.join('');
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
  const slotRect = () => { const r = $('#webslot').getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height, vh: innerHeight, scale: Number(store.get('lf-scale')) || 1 }; };
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

  // "Fetch new tracks automatically": tracks added while a fetch runs wait for it to finish
  let autoQueue = [];
  function autoFetch(ids) {
    const todo = ids.filter((id) => S.replace || !S.tracks.get(id)?.has_lyrics);
    if (!todo.length) return;
    if (S.progress?.running) { autoQueue.push(...todo); return; }
    startSearch(todo, true, S.replace);
  }
  function drainAutoQueue() {
    if (!autoQueue.length || S.progress?.running) return;
    const ids = autoQueue.filter((id) => S.tracks.has(id));
    autoQueue = [];
    if (ids.length) startSearch(ids, true, S.replace);
  }

  /** Add files and folders to the list; returns the ids of the tracks that were new. */
  async function addPaths(paths) {
    if (!paths.length) return [];
    toast(`Scanning ${paths.length === 1 ? paths[0].split(/[\\/]/).pop() : paths.length + ' items'}, including subfolders`);
    try {
      const known = new Set(S.tracks.keys());
      const res = await invoke('add_paths', { paths });
      setTracks(res.tracks);
      const fresh = res.tracks.filter((t) => !known.has(t.id)).map((t) => t.id);
      toast(fresh.length > 0 ? `Added ${fresh.length} track${fresh.length === 1 ? '' : 's'}. ${res.stats.missing} missing lyrics.` : 'No new audio files found', fresh.length > 0 ? 'good' : '');
      if (S.settings?.config.auto_fetch && fresh.length) autoFetch(fresh);
      return fresh;
    } catch (e) { toast(errText(e), 'bad', 7000); return []; }
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

  // ───────────────────────── theme, text size, resizable inspector ─────────────────────────
  // theme, accent and lyrics size are shared with the settings window (see shared.js)
  function paintAppearance() {
    const theme = window.LX.applyAppearance();
    $('#btn-theme').innerHTML = icon(theme === 'light' ? 'moon' : 'sun');
    S.lyricsSize = Number(store.get('lf-lyrics-size')) || 14;
  }
  function setTheme(t) { store.set('lf-theme', t); paintAppearance(); }
  function setLyricsSize(px) { store.set('lf-lyrics-size', String(Math.min(28, Math.max(11, px)))); paintAppearance(); }

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
    $('#btn-settings').onclick = () => invoke('open_settings').catch((e) => toast(errText(e), 'bad'));
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
    // Tidy: each change opens in the Edit tab for review (Revert undoes it, Save writes it)
    $('#act-tools').onclick = (e) => {
      const synced = S.detail?.found?.synced || S.detail?.embedded_synced;
      $('[data-tool="copy-lrc"]').disabled = !synced;
      $('[data-tool="timestamps"]').disabled = !/\[\d{1,3}:\d{2}/.test(editableText());
      openFloating($('#tools-pop'), e.currentTarget);
    };
    $('#tools-pop').addEventListener('click', async (e) => {
      const b = e.target.closest('[data-tool]');
      if (!b || b.disabled) return;
      closeFloating();
      const tool = b.dataset.tool;
      if (tool === 'copy-lrc' || tool === 'copy-plain') {
        const text = tool === 'copy-lrc' ? (S.detail?.found?.synced || S.detail?.embedded_synced || '') : TIDY.timestamps(shownLyrics());
        try { await clip.write(text); toast(tool === 'copy-lrc' ? 'Copied with timing (LRC)' : 'Copied as plain text', 'good'); } catch { toast('Clipboard unavailable', 'bad'); }
        return;
      }
      const before = editableText();
      const after = TIDY[tool](before);
      if (after === before) { toast('Nothing to change'); return; }
      const ta = $('#edit-text');
      ta.value = after;
      setTab('edit');
      toast('Review the change, then Save. Revert undoes it.');
    });
    $('#tools-pop').addEventListener('keydown', (e) => { if (e.key === 'Escape') { const a = S.pop?.anchor; closeFloating(); a?.focus(); } });
    // the file manager's own name, so the action says where the file will be shown
    const ua = navigator.userAgent;
    $('#tm-reveal-label').textContent = /Mac/.test(ua) ? 'Show in Finder' : /Windows/.test(ua) ? 'Show in Explorer' : 'Show in file manager';
    $('#tm-paste').onclick = () => { closeFloating(); pasteLyrics(); };
    // arrow keys move between the items of any open menu
    document.addEventListener('keydown', (e) => {
      const menu = e.target.closest?.('.menu');
      if (!menu || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) return;
      const items = $$('button:not([hidden]):not(:disabled)', menu);
      if (!items.length) return;
      e.preventDefault();
      const i = items.indexOf(document.activeElement);
      const next = e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : (i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
      items[next].focus();
    });
    // layout and sort
    S.layout = ['comfortable', 'compact', 'albums'].includes(store.get('lf-layout')) ? store.get('lf-layout') : 'comfortable';
    S.sort = ['artist', 'album', 'title', 'status', 'added'].includes(store.get('lf-sort')) ? store.get('lf-sort') : 'artist';
    const paintView = () => {
      $$('#view-pop [data-layout]').forEach((b) => b.setAttribute('aria-checked', b.dataset.layout === S.layout));
      $$('#view-pop [data-sort]').forEach((b) => { b.setAttribute('aria-checked', b.dataset.sort === S.sort); b.disabled = S.layout === 'albums'; });
      $('#table').dataset.layout = S.layout;
    };
    paintView();
    $('#btn-view').onclick = (e) => openFloating($('#view-pop'), e.currentTarget);
    $('#view-pop').addEventListener('click', (e) => {
      const b = e.target.closest('[data-layout], [data-sort]');
      if (!b || b.disabled) return;
      if (b.dataset.layout) { S.layout = b.dataset.layout; store.set('lf-layout', S.layout); }
      if (b.dataset.sort) { S.sort = b.dataset.sort; store.set('lf-sort', S.sort); }
      paintView();
      closeFloating();
      refresh();
      if (S.active != null) scrollToTrack(S.active);
    });
    $('#view-pop').addEventListener('keydown', (e) => { if (e.key === 'Escape') { const a = S.pop?.anchor; closeFloating(); a?.focus(); } });

    // library overview / activity pane
    $('#pane-tabs').addEventListener('click', (e) => { const b = e.target.closest('[data-pane]'); if (b) setPane(b.dataset.pane); });
    $('#overview').addEventListener('click', (e) => {
      const f = e.target.closest('[data-filter-go]');
      if (f) { setFilter(f.dataset.filterGo); return; }
      const al = e.target.closest('[data-album]');
      if (al) { $('#filter').value = al.dataset.album; S.query = al.dataset.album; setFilter('all'); return; }
      const go = e.target.closest('[data-go]');
      if (go) { const id = Number(go.dataset.go); selectOnly(id); setActive(id); scrollToTrack(id); renderRows(); renderSelbar(); return; }
      if (e.target.closest('[data-pane-go]')) { setPane('activity'); return; }
      if (e.target.closest('[data-act-stop]')) invoke('cancel_search');
    });
    setInterval(() => { if (S.progress?.running && S.pane === 'activity' && S.active == null) renderPane(); }, 1000);
    // the status line opens the activity
    const status = $('#status');
    status.tabIndex = 0;
    status.title = 'Show fetch activity';
    const openActivity = () => { S.sel.clear(); setActive(null); setPane('activity'); renderRows(); renderSelbar(); renderTop(); };
    status.addEventListener('click', (e) => { if (!e.target.closest('button')) openActivity(); });
    status.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openActivity(); } });

    // keyboard shortcuts (⌘ on macOS, Ctrl elsewhere)
    document.addEventListener('keydown', (e) => {
      if (S.modalOpen || !(e.metaKey || e.ctrlKey) || e.altKey) return;
      const k = e.key.toLowerCase();
      const act = {
        f: () => { $('#filter').focus(); $('#filter').select(); },
        o: () => (e.shiftKey ? $('#mn-import-folder') : $('#mn-import-files')).click(),
        enter: () => $('#btn-run').click(),
        ',': () => $('#btn-settings').click(),
        1: () => S.active != null && setTab('lyrics', true),
        2: () => S.active != null && setTab('edit', true),
        3: () => S.active != null && setTab('web', true),
        4: () => S.active != null && setTab('info', true),
      }[k];
      if (act) { e.preventDefault(); act(); }
    });
    const shortcut = /Mac/.test(navigator.userAgent) ? '⌘' : 'Ctrl+';
    $('#filter').title = `Filter tracks (${shortcut}F)`;
    $('#btn-run').title = `Search every listed track and embed the lyrics found (${shortcut}Enter)`;
    $('#btn-settings').title = `Settings (${shortcut},)`;
    $('#btn-import').title = `Add tracks (${shortcut}O files, ${shortcut}Shift+O folder)`;

    // row context menu
    $('#table').addEventListener('contextmenu', openContextMenu);
    $('#cx-reveal-label').textContent = $('#tm-reveal-label').textContent;
    const ctxIds = () => [...S.sel];
    $('#cx-search').onclick = () => { closeFloating(); startSearch(ctxIds(), false, S.replace); };
    $('#cx-embed').onclick = () => { closeFloating(); embedIds(ctxIds()); };
    $('#cx-files').onclick = () => { const a = S.pop?.anchor; closeFloating(); openFileOptions(a || $('#sel-files'), 'sel'); };
    $('#cx-copy').onclick = () => { closeFloating(); $('#act-copy').click(); };
    $('#cx-reveal').onclick = () => { closeFloating(); invoke('reveal_track', { id: ctxIds()[0] }).catch((e) => toast(errText(e), 'bad')); };
    $('#cx-path').onclick = async () => {
      closeFloating();
      const paths = ctxIds().map((i) => S.tracks.get(i)?.path).filter(Boolean);
      try { await clip.write(paths.join('\n')); toast(paths.length === 1 ? 'File path copied' : `${paths.length} file paths copied`, 'good'); } catch { toast('Clipboard unavailable', 'bad'); }
    };
    $('#cx-drop').onclick = async () => { closeFloating(); setTracks((await invoke('remove_tracks', { ids: ctxIds() })).tracks); };
    $('#cx-remove').onclick = () => { closeFloating(); removeLyrics(ctxIds()); };
    $('#ctx-pop').addEventListener('keydown', (e) => { if (e.key === 'Escape') { closeFloating(); $('#table').focus(); } });
    // no browser menu (Reload, Inspect) elsewhere; text fields and selected text keep theirs for copy and paste
    document.addEventListener('contextmenu', (e) => {
      if (e.defaultPrevented || e.target.closest('input, textarea, [contenteditable="true"]') || String(getSelection()).trim()) return;
      e.preventDefault();
    });

    $('#tm-files').onclick = () => { const a = $('#act-more'); closeFloating(); openFileOptions(a, 'active'); };
    $('#tm-reveal').onclick = () => { closeFloating(); invoke('reveal_track', { id: tid() }).catch((e) => toast(errText(e), 'bad')); };
    $('#tm-path').onclick = async () => { closeFloating(); try { await clip.write(activeTrack().path); toast('File path copied', 'good'); } catch { toast('Clipboard unavailable', 'bad'); } };
    $('#tm-remove').onclick = () => { closeFloating(); removeLyrics([tid()]); };
    for (const id of ['#import-pop', '#track-pop']) $(id).addEventListener('keydown', (e) => { if (e.key === 'Escape') { const a = S.pop?.anchor; closeFloating(); a?.focus(); } });
    $('#font-down').onclick = () => setLyricsSize(S.lyricsSize - 1);
    $('#font-up').onclick = () => setLyricsSize(S.lyricsSize + 1);
    paintAppearance();
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
      const inControl = document.activeElement?.closest('#tabs, #viewtabs, #splitter, .toolbar, .player, .popover, button');
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
        scrollToTrack(id);
        renderRows(); renderSelbar(); renderTop();
      }
    });

    if (/Mac/.test(navigator.userAgent)) document.documentElement.classList.add('mac');
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

    wireSplitter();
    paintAppearance();
    // the settings window changed appearance, or the system switched light/dark
    window.addEventListener('storage', paintAppearance);
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', paintAppearance);
    // window-level appearance the backend applies: interface scale
    const scale = Number(store.get('lf-scale')) || 1;
    if (scale !== 1) invoke('set_ui_scale', { scale }).catch(() => {});
    // settings saved in the settings window
    T.event.listen('settings-changed', (e) => {
      S.settings = e.payload;
      S.replace = !!S.settings.config.overwrite;
      $('#web-engine').innerHTML = engineOptions(S.settings.engines, S.settings.config.search_engine);
      renderTop();
      renderLibraries();
    });

    // events from the backend
    T.event.listen('session-log', (e) => logEvent(e.payload));
    T.event.listen('operation-warning', (e) => { logEvent(e.payload); toast(e.payload, 'bad', 8000); });
    T.event.listen('track', (e) => {
      const t = e.payload;
      const before = S.tracks.get(t.id)?.status;
      S.tracks.set(t.id, t);
      if (S.act && before !== t.status && ['found', 'written', 'notfound', 'error'].includes(t.status)) {
        S.act.recent.unshift({ id: t.id, status: t.status, provider: t.provider, at: Date.now() });
        S.act.recent.length = Math.min(S.act.recent.length, 40);
      }
      if (t.status === 'found') logEvent(`Found ${t.name} via ${pname(t.provider)}`);
      if (t.status === 'notfound') logEvent(`Not found: ${t.name}${t.note ? '\n' + t.note : ''}`);
      if (t.status === 'error') logEvent(`Error: ${t.name}${t.note ? '\n' + t.note : ''}`);
      if (t.status === 'written') logEvent(`Wrote lyrics for ${t.name}`);
      if (t.status === 'searching') S.current = `${t.title}${t.provider ? ' via ' + pname(t.provider) : ''}`;
      computeView();
      recount();
      buildItems();
      renderRows();
      renderSelbar();
      renderTop();
      if (S.active === t.id) { clearTimeout(wire._t); wire._t = setTimeout(() => setActive(t.id), 120); }
    });
    T.event.listen('progress', (e) => {
      const p = e.payload;
      if (p.running && (!S.act || S.act.end)) {
        S.act = { start: Date.now(), end: 0, sources: {}, recent: [] };
        if (S.active == null) setPane('activity');
      }
      if (S.act && !p.running && !S.act.end) S.act.end = Date.now();
      S.progress = p;
      renderPane();
      if (!e.payload.running) { S.progressAt = Date.now(); setTimeout(drainAutoQueue, 300); }
      renderTop();
    });
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
      // the window is brought forward first; this asks the same question as the Fetch button
      if (e.payload === 'find-all' && !S.progress?.running) runAll();
    });
    T.event.listen('player', (e) => {
      S.player = e.payload;
      if (e.payload.state === 'error' && e.payload.error) toast(e.payload.error, 'bad', 6000);
      if (S.active != null && e.payload.id === S.active) {
        renderPlayer();
        // the playhead on the Details timeline
        const head = $('#info .dx-head');
        if (S.tab === 'info' && head && e.payload.duration) head.style.left = `${Math.min(100, (e.payload.position / e.payload.duration) * 100).toFixed(2)}%`;
        else if (S.tab === 'info' && !head && e.payload.state === 'playing' && S.detail) renderInfo(activeTrack(), S.detail);
      }
    });
    // a source finished for the shown track: refresh its search trail
    let trailTimer = 0;
    T.event.listen('trail', (e) => {
      const { provider, result, millis } = e.payload;
      if (S.act && provider && result) {
        const src = S.act.sources[provider] || (S.act.sources[provider] = { found: 0, miss: 0, fail: 0, timeout: 0, ms: 0, n: 0 });
        src[result] = (src[result] || 0) + 1;
        src.ms += millis || 0;
        src.n++;
        renderPane();
      }
      if (e.payload.id !== S.active) return;
      clearTimeout(trailTimer);
      trailTimer = setTimeout(async () => {
        const id = S.active;
        try { const d = await invoke('get_lyrics', { id }); if (S.active === id) { S.detail = d; renderInfo(activeTrack(), d); } } catch { /* track removed */ }
      }, 250);
    });
    $('#info').addEventListener('click', (e) => {
      const open = e.target.closest('[data-open]');
      if (open) { e.preventDefault(); invoke('open_external', { url: open.dataset.open }).catch((err) => toast(errText(err), 'bad')); return; }
      if (e.target.closest('dd.link') && S.active != null) invoke('reveal_track', { id: S.active }).catch((err) => toast(errText(err), 'bad'));
    });

    // drops on the window, and files handed over by the OS (dropped on the app icon, "Open with")
    const veil = $('#dropveil');
    // only drops on this webview: the menu bar drop zone is a webview of its own and handles its drops
    const own = { target: { kind: 'Webview', label: 'ui' } };
    T.event.listen('tauri://drag-enter', () => veil.classList.add('on'), own);
    T.event.listen('tauri://drag-over', () => veil.classList.add('on'), own);
    T.event.listen('tauri://drag-leave', () => veil.classList.remove('on'), own);
    T.event.listen('tauri://drag-drop', (e) => { veil.classList.remove('on'); addPaths(e.payload.paths || []); }, own);
    const takeOpened = async () => { const p = await invoke('take_pending_open'); if (p.length) addPaths(p); };
    T.event.listen('open-paths', takeOpened);
    // the menu bar drop zone: "Add" shows the tracks, "Add and fetch" also searches the new ones
    T.event.listen('shelf-drop', async (e) => {
      const ids = await addPaths(e.payload.paths || []);
      if (!e.payload.fetch || !ids.length) return;
      if (S.progress?.running) { toast('A fetch is already running; the new tracks are in the list'); return; }
      const todo = ids.filter((id) => S.replace || !S.tracks.get(id)?.has_lyrics);
      if (todo.length) startSearch(todo, true, S.replace); else toast('The new tracks already have lyrics');
    });

    invoke('list_tracks').then((r) => { setTracks(r.tracks); takeOpened(); });
  }

  // Hook used by the built-in self-test (LYRIXEARCH_SELFTEST=1) and for debugging.
  window.__lf = {
    addPaths,
    state: S,
    urlFor,
    selectFirst: () => { const t = S.view[0]; if (t) { selectOnly(t.id); setActive(t.id); renderRows(); renderSelbar(); } return !!t; },
    selectId: (id) => { if (id == null) S.sel.clear(); else selectOnly(id); setActive(id); renderRows(); renderSelbar(); },
    setTab,
  };

  document.addEventListener('DOMContentLoaded', wire);
})();
