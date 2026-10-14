// Development stand-in for the Tauri backend. It only activates when the page is opened in a
// plain browser (no window.__TAURI__), so the UI can be previewed and tested without the app.
(function () {
  if (window.__TAURI__) return;

  const listeners = {};
  const emit = (name, payload) => (listeners[name] || []).forEach((f) => f({ event: name, payload }));

  const artists = [
    ['Aurora Vale', 'Glass Harbour', 'Tidewater', 'Soft Static'],
    ['The Lanterns', 'Paper Moons', 'Night Ferry', 'Slow Circuit'],
    ['Mira Okafor', 'Copper Sky', 'Hollow Bells', 'Late Bloom'],
    ['Kasumi', '夜明けのうた', 'Harbour Lights', 'Blue Hours'],
  ];
  let nextId = 1;
  const tracks = [];
  const mk = (artist, album, title, hasLyrics) => ({
    id: nextId++, path: `/Music/${artist}/${album}/${title}.flac`, name: `${title}.flac`, folder: `/Music/${artist}/${album}`,
    artist, title, album, format: 'flac', duration: 90 + Math.floor(Math.random() * 200), has_lyrics: hasLyrics, status: 'idle', provider: null, score: null, synced: false, note: null,
  });
  artists.forEach((a, i) => {
    for (let k = 1; k <= 5; k++) tracks.push(mk(a[0], a[1], `${a[2 + (k % 2)]} ${k}`, (i + k) % 3 === 0));
  });
  const lyricsOf = {};
  const cfg = {
    providers: ['local_files', 'genius', 'lrclib', 'lyricsfreak', 'ddg_search', 'netease', 'qqmusic', 'kugou', 'lyrics_ovh'],
    concurrency: 10, provider_timeout_secs: 8, penalty_max_failures: 3, penalty_window_secs: 60, penalty_cooldown_secs: 300,
    write_tags: true, sidecar: 'off', keep_mtime: true, overwrite: false, remove_section_tags: false, prepend_newline: false,
    proxy: null, min_score: 0.8, search_engine: 'google', library_dirs: ['/Users/demo/Music/Library'],
  };
  const catalog = [
    ['local_files', 'Local files', 'builtin', '.lrc / .txt next to the track, or in a Lyrics folder'],
    ['genius', 'Genius', 'builtin', 'Plain lyrics, wide catalogue'],
    ['lrclib', 'LRCLIB', 'builtin', 'Open database with synced lyrics'],
    ['lyricsfreak', 'LyricsFreak', 'builtin', 'Plain lyrics'],
    ['ddg_search', 'Web search (DuckDuckGo)', 'builtin', 'Finds a supported page when the others miss'],
    ['netease', 'NetEase Cloud Music', 'builtin', 'Synced lyrics, strong for Asian music'],
    ['qqmusic', 'QQ Music', 'builtin', 'Synced lyrics'],
    ['kugou', 'Kugou', 'builtin', 'Synced lyrics'],
    ['lyrics_ovh', 'lyrics.ovh', 'builtin', 'Plain lyrics, exact names only'],
    ['metallum', 'Metal Archives (via the shared metallum CLI)', 'plugin', '~/Library/Application Support/lyrixearch/plugins/metallum'],
  ].map(([id, name, kind, note]) => ({ id, name, kind, note }));
  const settings = () => ({ config: cfg, providers: catalog, engines: ['google', 'google_ai', 'brave_ai', 'bing', 'duckduckgo', 'yahoo', 'yandex'], plugins_dir: '~/Library/Application Support/lyrixearch/plugins', config_path: '~/Library/Application Support/lyrixearch/config.toml' });
  const list = () => {
    const w = tracks.filter((t) => t.has_lyrics).length;
    return { tracks: tracks.map((t) => ({ ...t })), stats: { total: tracks.length, with_lyrics: w, missing: tracks.length - w }, skipped: 0 };
  };
  const sample = (t) => `Placeholder verse for ${t.title}\nA couple of invented lines\nso the preview has some text\n\nPlaceholder chorus\nthat repeats a little\nand then fades out`;
  let cancelled = false;
  const player = { id: null, pos: 0, duration: 0, state: 'stopped', timer: null,
    tick() { if (player.state === 'playing') player.pos += 0.25; if (player.pos >= player.duration) { clearInterval(player.timer); player.state = 'ended'; } emit('player', { id: player.id, state: player.state, pos: player.pos, duration: player.duration, error: null }); } };

  const commands = {
    list_tracks: () => list(),
    add_paths: () => list(),
    clear_tracks: () => { tracks.length = 0; return list(); },
    remove_tracks: ({ ids }) => { ids.forEach((id) => { const i = tracks.findIndex((t) => t.id === id); if (i >= 0) tracks.splice(i, 1); }); return list(); },
    get_settings: () => settings(),
    set_settings: ({ config }) => { Object.assign(cfg, config); return settings(); },
    install_plugin: () => settings(),
    test_providers: async () => {
      await new Promise((r) => setTimeout(r, 500));
      return catalog.filter((p) => p.id !== 'local_files').map((p, i) => ({ id: p.id, ok: i !== 3, detail: i !== 3 ? 'ok' : 'blocked (HTTP 403)', millis: 200 + i * 130 }));
    },
    get_lyrics: ({ id }) => {
      const t = tracks.find((x) => x.id === id);
      if (!t) return null;
      return { found: lyricsOf[id] || null, embedded: t.has_lyrics ? sample(t) : null };
    },
    cancel_search: () => { cancelled = true; },
    search: async ({ ids, autoWrite, replace }) => {
      if (typeof autoWrite !== 'boolean') throw 'invalid args `autoWrite` for command `search`: command search missing required key autoWrite';
      const auto_write = autoWrite;
      cancelled = false;
      const work = tracks.filter((t) => (ids.length ? ids.includes(t.id) : !t.has_lyrics) && (replace === undefined || replace === null || replace || !t.has_lyrics));
      work.forEach((t) => { t.status = 'queued'; emit('track', { ...t }); });
      let done = 0, found = 0, written = 0, missed = 0;
      const prog = (running) => emit('progress', { done, total: work.length, found, written, missed, running });
      prog(true);
      (async () => {
        for (const t of work) {
          if (cancelled) { t.status = 'idle'; done++; emit('track', { ...t }); continue; }
          t.status = 'searching'; t.provider = 'lrclib'; emit('track', { ...t });
          await new Promise((r) => setTimeout(r, 250 + Math.random() * 450));
          done++;
          if (Math.random() < 0.8) {
            found++; t.provider = ['lrclib', 'genius', 'netease', 'metallum'][Math.floor(Math.random() * 4)];
            t.synced = t.provider === 'lrclib' || t.provider === 'netease'; t.score = 0.9 + Math.random() * 0.1;
            lyricsOf[t.id] = { source: t.provider, url: 'https://example.invalid/song', matched_artist: t.artist, matched_title: t.title, score: t.score, plain: sample(t), synced: t.synced ? '[00:01.00]placeholder' : null };
            if (auto_write) { t.status = 'written'; t.has_lyrics = true; written++; } else t.status = 'found';
          } else { missed++; t.status = 'notfound'; t.note = 'genius: no match\nlrclib: no match'; }
          emit('track', { ...t }); prog(true);
        }
        prog(false);
      })();
      return work.length;
    },
    apply_found: async ({ ids }) => {
      let n = 0;
      tracks.filter((t) => t.status === 'found' && (!ids.length || ids.includes(t.id))).forEach((t) => { t.status = 'written'; t.has_lyrics = true; n++; emit('track', { ...t }); });
      return n;
    },
    save_lyrics: async ({ id }) => { const t = tracks.find((x) => x.id === id); t.has_lyrics = true; t.status = 'written'; emit('track', { ...t }); return { ...t }; },
    delete_lyrics: async ({ ids }) => { let n = 0; tracks.filter((t) => ids.includes(t.id) && t.has_lyrics).forEach((t) => { t.has_lyrics = false; t.status = 'idle'; n++; emit('track', { ...t }); }); return n; },
    get_cover: ({ id }) => {
      const t = tracks.find((x) => x.id === id);
      if (!t) return null;
      let h = 0; for (const c of t.album) h = (h * 31 + c.charCodeAt(0)) % 360;
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="hsl(${h},18%,28%)"/><circle cx="32" cy="32" r="20" fill="none" stroke="hsl(${h},25%,60%)" stroke-width="3"/><circle cx="32" cy="32" r="6" fill="hsl(${h},25%,60%)"/></svg>`;
      return 'data:image/svg+xml;base64,' + btoa(svg);
    },
    reveal_track: () => {},
    embed_sidecar: async ({ ids, kind, replace }) => {
      let embedded = 0, missing = 0, kept = 0;
      ids.forEach((id, i) => {
        const t = tracks.find((x) => x.id === id);
        if (!t) return;
        if (t.has_lyrics && replace === false) { kept++; return; }
        if (i % 2 === 0) { t.has_lyrics = true; t.status = 'written'; t.provider = 'file'; embedded++; emit('track', { ...t }); } else missing++;
      });
      return { embedded, missing, kept, failed: 0 };
    },
    export_lyrics: ({ ids, kind, replace }) => ({ written: ids.length - 1 > 0 ? ids.length - 1 : ids.length, kept: ids.length > 1 && replace === false ? 1 : 0, none: 0 }),
    take_pending_open: () => [],
    browser_zoom: () => {},
    browser_mobile: () => {},
    player_load: ({ id, play }) => {
      const t = tracks.find((x) => x.id === id);
      clearInterval(player.timer);
      Object.assign(player, { id, pos: 0, duration: t ? t.duration : 120, state: play ? 'playing' : 'paused' });
      player.tick();
      if (play) player.timer = setInterval(player.tick, 250);
    },
    player_toggle: () => {
      if (player.state === 'playing') { player.state = 'paused'; clearInterval(player.timer); }
      else { player.state = 'playing'; clearInterval(player.timer); player.timer = setInterval(player.tick, 250); }
      emit('player', { id: player.id, state: player.state, pos: player.pos, duration: player.duration, error: null });
    },
    player_seek: ({ secs }) => { player.pos = secs; emit('player', { id: player.id, state: player.state, pos: player.pos, duration: player.duration, error: null }); },
    player_stop: () => { clearInterval(player.timer); player.state = 'stopped'; emit('player', { id: player.id, state: 'stopped', pos: 0, duration: player.duration, error: null }); },
    browser_show: () => {}, browser_hide: () => {}, browser_back: () => {}, browser_forward: () => {}, browser_reload: () => {},
    open_external: () => {},
    web_search_url: ({ engine, artist, title }) => `https://www.google.com/search?q=${encodeURIComponent(artist + ' ' + title + ' lyrics')}&engine=${engine}`,
  };

  window.__MOCK__ = true;
  window.__TAURI__ = {
    core: { invoke: (cmd, args) => Promise.resolve().then(() => (commands[cmd] ? commands[cmd](args || {}) : Promise.reject(`mock: unknown command ${cmd}`))) },
    event: { listen: (name, fn) => { (listeners[name] = listeners[name] || []).push(fn); return Promise.resolve(() => {}); } },
    dialog: { open: async () => '/Users/demo/Music/New Folder' },
    clipboardManager: { readText: async () => 'Pasted placeholder line\nand a second line', writeText: async () => {} },
  };
})();
