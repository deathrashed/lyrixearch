// Settings window: a section sidebar and rows of title, description and control. Every change saves at
// once (no Save button); the main window hears about it through the backend's "settings-changed" event.
(() => {
  'use strict';
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args || {});
  const { icon, esc, errText, store, ACCENTS, themeSetting, applyAppearance } = window.LX;
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];

  let settings = null;   // what the backend last confirmed
  let draft = null;      // what is being edited
  let section = 'general';

  const SECTIONS = {
    general: ['General', 'How tracks are searched'],
    sources: ['Sources', 'Where lyrics come from, tried from the top for each track'],
    files: ['Files', 'What is written into your music files'],
    appearance: ['Appearance', 'Theme, colour and size'],
    plugins: ['Plugins', 'Scripts that act as extra sources'],
    transcription: ['Transcription', 'When no source has the lyrics, AI writes them down from the track itself'],
    accounts: ['Accounts', 'Lyrics from services you have an account with. You sign in on their own page; the app never sees your password'],
    diagnostics: ['Diagnostics', 'Checks to run when something does not work, and a report to share'],
    about: ['About', ''],
  };
  const IS_MAC = /Mac/.test(navigator.userAgent);
  const BAR = IS_MAC ? 'menu bar' : 'tray';
  const ENGINE_LABELS = { google: 'Google', google_ai: 'Google AI Mode', brave_ai: 'Brave AI', bing: 'Bing', duckduckgo: 'DuckDuckGo', yahoo: 'Yahoo', yandex: 'Yandex' };

  function toast(msg, kind = '') {
    const el = document.createElement('div');
    el.className = 'toast';
    el.innerHTML = `<i class="led ${kind === 'bad' ? 'error' : 'on'}" aria-hidden="true"></i><span>${esc(msg)}</span>`;
    $('#toasts').append(el);
    setTimeout(() => { el.classList.add('out'); setTimeout(() => el.remove(), 200); }, kind === 'bad' ? 7000 : 2600);
  }

  // ───────── controls ─────────
  const row = (k, title, sub, ctl) => `<div class="sw-row"><label class="sw-label" for="f-${k}"><b>${title}</b>${sub ? `<span>${sub}</span>` : ''}</label><div class="sw-ctl">${ctl}</div></div>`;
  const group = (title, rows) => `${title ? `<h2 class="sw-group">${title}</h2>` : ''}<div class="sw-card">${rows.join('')}</div>`;
  const sw = (k) => `<span class="switch"><input id="f-${k}" type="checkbox" data-k="${k}" ${draft[k] ? 'checked' : ''}><i></i></span>`;
  const rng = (k, min, max, step, fmt = (v) => v) => `<span class="sw-range"><input id="f-${k}" type="range" data-k="${k}" min="${min}" max="${max}" step="${step}" value="${draft[k]}"><b data-v="${k}">${fmt(draft[k])}</b></span>`;
  const sel = (k, opts) => `<select id="f-${k}" data-k="${k}">${opts.map(([v, l]) => `<option value="${esc(v)}" ${String(draft[k]) === v ? 'selected' : ''}>${esc(l)}</option>`).join('')}</select>`;
  const txt = (k, ph = '', type = 'text') => `<input id="f-${k}" type="${type}" data-k="${k}" value="${esc(draft[k] || '')}" placeholder="${esc(ph)}" spellcheck="false" autocomplete="off">`;
  const seg = (name, cur, opts) => `<div class="seg" role="radiogroup" aria-label="${esc(name)}">${opts.map(([v, l]) => `<button role="radio" data-seg="${esc(name)}" data-v="${esc(v)}" aria-checked="${v === cur}">${esc(l)}</button>`).join('')}</div>`;

  function render() {
    const [title, sub] = SECTIONS[section];
    $('#sw-title').textContent = title;
    $('#sw-sub').textContent = sub;
    $$('.sw-nav button').forEach((b) => (b.dataset.sec === section ? b.setAttribute('aria-current', 'page') : b.removeAttribute('aria-current')));
    const body = $('#sw-body');
    if (section === 'general') {
      body.innerHTML = group('Searching', [
        row('concurrency', 'Simultaneous searches', 'Tracks searched at the same time', rng('concurrency', 1, 20, 1)),
        row('provider_timeout_secs', 'Source timeout', 'Seconds to wait for one source before moving on', rng('provider_timeout_secs', 2, 30, 1, (v) => `${v} s`)),
        row('min_score', 'Match strictness', 'How closely a result must match the track', rng('min_score', 0.5, 1, 0.05, (v) => `${Math.round(v * 100)}%`)),
        row('auto_fetch', 'Fetch new tracks automatically', 'Search tracks as soon as they are added, and embed what is found', sw('auto_fetch')),
        row('synced_only', 'Synced lyrics only', 'Accept only results with timed lines', sw('synced_only')),
      ]) + group('Track names', [
        row('tag_clean_enabled', 'Clean track titles', 'Ignore labels like (Remastered 2011) or [Live] when searching', sw('tag_clean_enabled')),
        row('tag_clean_pattern', 'Extra title cleanup', 'Optional regular expression removed from titles', `${txt('tag_clean_pattern', 'none')}<span class="setting-error" id="err-tag_clean_pattern" role="alert"></span>`),
        row('lastfm_api_key', 'Last.fm API key', 'Optional; corrects misspelled artist and title names', txt('lastfm_api_key', 'none', 'password')),
      ]) + group('Network', [
        row('search_engine', 'Web search engine', 'Used by the Web tab', sel('search_engine', settings.engines.map((e) => [e, ENGINE_LABELS[e] || e]))),
        row('proxy', 'Proxy', 'http:// or socks5:// address, empty for none', `${txt('proxy', 'none')}<span class="setting-error" id="err-proxy" role="alert"></span>`),
      ]) + group('Background', [
        row('keep_running', `Keep running in the ${BAR}`, `Closing the window leaves Lyrixearch in the ${BAR}, so a fetch carries on`, sw('keep_running')),
        row('notify_done', 'Notify when a fetch finishes', 'Only while the window is in the background', sw('notify_done')),
        row('keep_awake', 'Keep the computer awake', 'Stops sleep while a fetch runs', sw('keep_awake')),
        ...(IS_MAC ? [row('drop_shelf', 'Drop zone under the menu bar icon', 'Drag music near the icon and a drop zone opens to add it', sw('drop_shelf'))] : []),
      ]);
    } else if (section === 'files') {
      body.innerHTML = group('Embedding', [
        row('write_tags', 'Write lyrics into the files', 'Embed in the audio tags (ID3, Vorbis, MP4)', sw('write_tags')),
        row('overwrite', 'Replace existing lyrics', 'Search tracks that already have lyrics and overwrite them', sw('overwrite')),
        row('keep_mtime', 'Keep modification date', 'Restore the file date after writing', sw('keep_mtime')),
      ]) + group('Sidecar files', [
        row('word_timing', 'Word-by-word timing', 'Karaoke timing where a source has it (Apple Music, Deezer): in .lrc files and FLAC / Ogg tags; MP3 keeps line timing', sw('word_timing')),
        row('sidecar', 'Save a .lrc or .txt', 'Next to the track; Auto picks .lrc for synced lyrics', sel('sidecar', [['off', 'Off'], ['auto', 'Auto'], ['lrc', '.lrc only'], ['txt', '.txt only']])),
      ]) + group('Text', [
        row('remove_section_tags', 'Remove section headers', 'Drop lines such as [Chorus] or [Solo]', sw('remove_section_tags')),
        row('prepend_newline', 'Blank first line', 'Some players expect an empty first line', sw('prepend_newline')),
      ]) + `<p class="sw-note">Settings file: <code>${esc(settings.config_path)}</code></p>`;
    } else if (section === 'sources') {
      const cat = new Map(settings.providers.map((p) => [p.id, p]));
      const on = draft.providers.map((id) => ({ ...(cat.get(id) || { id, name: id, note: 'Not available' }), enabled: true }));
      const off = settings.providers.filter((p) => !draft.providers.includes(p.id)).map((p) => ({ ...p, enabled: false }));
      const list = [...on, ...off];
      body.innerHTML = `<div class="sw-card"><ul class="plist sw-plist">${list.map((p, i, arr) => `<li class="${p.enabled ? '' : 'off'}" data-id="${esc(p.id)}">
          <div class="ord"><button data-mv="-1" aria-label="Move ${esc(p.name)} up" ${i === 0 || !p.enabled ? 'disabled' : ''}>${icon('up')}</button><button data-mv="1" aria-label="Move ${esc(p.name)} down" ${!p.enabled || arr[i + 1]?.enabled === false || i === arr.length - 1 ? 'disabled' : ''}>${icon('down')}</button></div>
          <div class="grow"><div class="nm">${esc(p.name)} ${p.kind === 'plugin' ? '<span class="tag lit">Plugin</span>' : ''}</div><div class="nt">${esc(p.note)}</div></div>
          <label class="switch"><input type="checkbox" aria-label="Use ${esc(p.name)}" ${p.enabled ? 'checked' : ''}><i></i></label></li>`).join('')}</ul></div>
        <div class="sw-actions"><button class="btn" id="test-sources">${icon('refresh')}<span>Test all sources</span></button><span class="sw-note">Slow or failing sources are skipped for a while automatically.</span></div>
        <ul class="health" id="health"></ul>`;
    } else if (section === 'appearance') {
      const accent = store.get('lf-accent') || 'green';
      const scale = store.get('lf-scale') || '1';
      body.innerHTML = group('', [
        row('theme', 'Theme', 'Follow the system, or always dark or light', seg('theme', themeSetting(), [['system', 'System'], ['dark', 'Dark'], ['light', 'Light']])),
        row('accent', 'Accent colour', 'Buttons, selection and focus', `<div class="swatches" role="radiogroup" aria-label="Accent colour">${Object.entries(ACCENTS).map(([k, a]) => `<button role="radio" class="swatch" data-accent="${k}" aria-checked="${k === accent}" aria-label="${esc(a.name)}" title="${esc(a.name)}" style="--c:${a.dark}"></button>`).join('')}</div>`),
        row('scale', 'Interface scale', 'Size of everything in the main window', sel('__scale', [['0.9', '90%'], ['1', '100%'], ['1.1', '110%'], ['1.25', '125%']]).replace(`value="${scale}"`, `value="${scale}" selected`)),
        row('lyrics', 'Lyrics text size', 'In the Lyrics and Edit tabs', `<span class="sw-range"><input id="f-lyrics" type="range" min="11" max="28" step="1" value="${Number(store.get('lf-lyrics-size')) || 14}" data-local="lf-lyrics-size"><b data-v="lyrics">${Number(store.get('lf-lyrics-size')) || 14} px</b></span>`),
      ]);
    } else if (section === 'plugins') {
      const plugins = settings.providers.filter((p) => p.kind === 'plugin');
      body.innerHTML = `<div class="sw-card">${plugins.length
        ? `<ul class="plist sw-plist">${plugins.map((p) => `<li><div class="grow"><div class="nm">${esc(p.name)}</div><div class="nt">${esc(p.id)} · ${esc(p.note)}</div></div><span class="tag ${draft.providers.includes(p.id) ? 'lit' : ''}">${draft.providers.includes(p.id) ? 'Enabled' : 'Disabled'}</span></li>`).join('')}</ul>`
        : '<p class="sw-empty">No plugins installed yet.</p>'}</div>
        <div class="sw-actions"><button class="btn" id="plugin-install">${icon('plug')}<span>Install plugin folder…</span></button></div>
        <p class="sw-note">Plugins live in <code>${esc(settings.plugins_dir)}</code>. See docs/PLUGINS.md for the protocol.</p>`;
    } else if (section === 'transcription') {
      const st = tStatus || {};
      const engine = draft.transcribe_engine || 'local';
      const found = (path, what, hint) => (path ? `<span class="ok-text">Found</span> <code>${esc(path)}</code>` : `<span class="bad-text">Not found.</span> ${hint || `Install ${what}.`}`);
      const installHint = /Mac/.test(navigator.userAgent) ? 'Install it with <code>brew install whisper-cpp</code>.' : 'Install whisper.cpp from your package manager (for example <code>whisper.cpp</code>), or build it from github.com/ggml-org/whisper.cpp.';
      const ffHint = /Mac/.test(navigator.userAgent) ? 'Install it with <code>brew install ffmpeg</code>.' : 'Install <code>ffmpeg</code> from your package manager.';
      const modelCtl = st.model
        ? `<span class="ok-text">Ready</span> <code>${esc(st.model.split(/[\\/]/).pop())}</code>`
        : (tDownload ? `<span class="sw-range"><progress max="${tDownload.total || 1}" value="${tDownload.done}"></progress><b>${tDownload.total ? Math.round((tDownload.done / tDownload.total) * 100) : 0}%</b></span>` : `<button class="btn sm" id="t-download">${icon('download')}<span>Download model (550 MB)</span></button>`);
      body.innerHTML = group('', [
        row('t-use', 'Use AI transcription', 'Tried last, only when every other source missed', `<span class="switch"><input id="f-t-use" type="checkbox" data-t-use ${draft.providers.includes('ai_transcribe') ? 'checked' : ''}><i></i></span>`),
        row('transcribe_engine', 'Engine', 'On this computer (free, private), or a service with your own API key', sel('transcribe_engine', [['local', 'On this computer (whisper.cpp)'], ['groq', 'Groq (free tier, fast)'], ['openai', 'OpenAI']])),
        row('transcribe_language', 'Language', 'A language code such as en or de; empty detects it', txt('transcribe_language', 'auto')),
      ]) + (engine === 'local'
        ? group('On this computer', [
          `<div class="sw-row"><div class="sw-label"><b>whisper.cpp</b><span>${found(st.whisper, 'whisper.cpp', installHint)}</span></div></div>`,
          `<div class="sw-row"><div class="sw-label"><b>ffmpeg</b><span>${found(st.ffmpeg, 'ffmpeg', ffHint)}</span></div></div>`,
          `<div class="sw-row"><div class="sw-label"><b>Model</b><span>Whisper large-v3-turbo: good with singing, about a minute per song without a GPU</span></div><div class="sw-ctl">${modelCtl}</div></div>`,
        ])
        : group('Service', [
          row('transcribe_api_key', 'API key', engine === 'groq' ? 'From console.groq.com/keys' : 'From platform.openai.com/api-keys', txt('transcribe_api_key', 'none', 'password')),
        ])) + `<p class="sw-note">Transcriptions can be wrong, so they are marked for review: a search shows them, but Fetch does not write them into your files by itself. Embed one from the track's Lyrics tab once you have checked it.</p>`;
      if (!tStatus) loadTranscribeStatus();
    } else if (section === 'accounts') {
      body.innerHTML = `<div class="sw-card">${ACCOUNTS.map((a) => `<div class="sw-row" data-acct="${a.id}">
          <div class="sw-label"><b>${esc(a.name)}</b><span class="acct-status" id="acct-${a.id}">${acctText(a.id)}</span></div>
          <div class="sw-ctl acct-ctl">
            <span class="switch" title="Use ${esc(a.name)} as a source"><input type="checkbox" data-acct-use="${a.id}" aria-label="Use ${esc(a.name)} as a source" ${draft.providers.includes(a.id) ? 'checked' : ''}><i></i></span>
            <button class="btn sm" data-acct-go="${a.id}">${acctState[a.id]?.signedIn ? 'Sign out' : 'Sign in…'}</button>
          </div></div>`).join('')}</div>
        <p class="sw-note">Signing in opens the service's own page in a window of this app. Lookups then run in that signed-in page, hidden, using the service's web player; it closes again after two minutes without lookups. Apple Music needs a subscription. The CLI cannot use these accounts.</p>`;
      for (const a of ACCOUNTS) if (!acctState[a.id]) checkAccount(a.id);
    } else if (section === 'diagnostics') {
      body.innerHTML = `<div class="sw-card"><ul class="diag" id="diag"><li class="sw-empty">Checking…</li></ul></div>
        <div class="sw-actions"><button class="btn" id="diag-run">${icon('refresh')}<span>Check again</span></button><button class="btn" id="test-sources">${icon('globe')}<span>Test all sources</span></button><button class="btn" id="diag-copy">${icon('copy')}<span>Copy report</span></button><button class="btn" id="diag-reveal">${icon('reveal')}<span>${IS_MAC ? 'Show settings file in Finder' : 'Show settings file'}</span></button></div>
        <ul class="health" id="health"></ul>
        <p class="sw-note">The report has versions, settings locations and check results. It leaves out your API key and proxy password.</p>`;
      runDiagnostics();
    } else if (section === 'about') {
      body.innerHTML = `<div class="sw-about"><img src="assets/icon-256.png" alt="" width="96" height="96"><h2>Lyrixearch</h2><p class="sw-note" id="about-version"></p>
        <p>Find lyrics for your music, review them, and embed them in the files.</p>
        <div class="sw-actions center"><button class="btn" data-open="https://github.com/deathrashed/lyrixearch">${icon('external')}<span>Project page</span></button></div>
        <p class="sw-note">Lyrics are copyrighted by their owners; fetch them for personal use and respect each source's terms.</p></div>`;
      (T.app?.getVersion ? T.app.getVersion() : Promise.resolve('')).then((v) => { if (v) $('#about-version').textContent = `Version ${v}`; }).catch(() => {});
    }
    $$('[data-icon]', body).forEach((el) => { el.innerHTML = icon(el.dataset.icon); });
  }

  // ───────── transcription ─────────
  let tStatus = null;
  let tDownload = null;
  async function loadTranscribeStatus() {
    try { tStatus = await invoke('transcribe_status'); } catch { tStatus = {}; }
    if (section === 'transcription') render();
  }

  // ───────── accounts ─────────
  let ACCOUNTS = [];
  const acctState = {};
  const acctText = (id) => {
    const st = acctState[id];
    if (!st || st.checking) return 'Checking…';
    if (st.signedIn) return st.name ? `Signed in as ${esc(st.name)}` : 'Signed in';
    return st.error ? `Not signed in (${esc(st.error)})` : 'Not signed in';
  };
  async function checkAccount(id) {
    const was = acctState[id]?.signedIn;
    acctState[id] = { ...(acctState[id] || {}), checking: true };
    if (section === 'accounts') render();
    try { acctState[id] = await invoke('account_status', { id }); } catch (e) { acctState[id] = { signedIn: false, error: errText(e) }; }
    // just signed in: use it as a source
    if (acctState[id].signedIn && was === false && !draft.providers.includes(id)) { draft.providers.push(id); scheduleSave(); toast(`${ACCOUNTS.find((a) => a.id === id)?.name} is now a source`); }
    if (section === 'accounts') render();
  }

  // ───────── diagnostics ─────────
  let lastReport = [];
  async function runDiagnostics() {
    const ul = $('#diag');
    try {
      lastReport = await invoke('diagnose');
      const led = { ok: 'on', warn: 'searching', bad: 'error' };
      ul.innerHTML = lastReport.map((c) => `<li><i class="led ${led[c.level] || 'on'}" aria-hidden="true"></i><b>${esc(c.name)}</b><span>${esc(c.detail)}</span></li>`).join('');
    } catch (e) { ul.innerHTML = `<li>${esc(errText(e))}</li>`; }
  }
  const mark = (level) => (level === 'ok' ? 'OK  ' : level === 'warn' ? 'WARN' : 'FAIL');

  // ───────── saving ─────────
  let saveTimer = 0;
  function scheduleSave() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 350);
  }
  async function save() {
    try {
      settings = await invoke('set_settings', { config: draft });
      $$('.setting-error').forEach((e) => { e.textContent = ''; });
      $$('[aria-invalid]').forEach((e) => e.removeAttribute('aria-invalid'));
    } catch (e) {
      const m = errText(e);
      const field = m.startsWith('tag_clean_pattern') ? 'tag_clean_pattern' : m.startsWith('invalid proxy') ? 'proxy' : null;
      if (field && $(`#f-${field}`)) {
        $(`#f-${field}`).setAttribute('aria-invalid', 'true');
        $(`#err-${field}`).textContent = field === 'proxy' ? 'Enter an http:// or socks5:// address.' : 'Enter a valid regular expression.';
      } else toast(m, 'bad');
    }
  }

  function wire() {
    $('.sw-nav').addEventListener('click', (e) => { const b = e.target.closest('[data-sec]'); if (b) { section = b.dataset.sec; render(); } });
    const body = $('#sw-body');
    body.addEventListener('input', (e) => {
      const t = e.target;
      if (t.dataset.local) {
        store.set(t.dataset.local, t.value);
        $('[data-v="lyrics"]').textContent = `${t.value} px`;
        applyAppearance();
        return;
      }
      const k = t.dataset.k;
      if (!k) return;
      if (k === '__scale') return;
      if (t.type === 'checkbox') draft[k] = t.checked;
      else if (t.type === 'range') { draft[k] = Number(t.value); const v = $(`[data-v="${k}"]`); if (v) v.textContent = k === 'min_score' ? `${Math.round(t.value * 100)}%` : k === 'provider_timeout_secs' ? `${t.value} s` : t.value; }
      else if (k === 'proxy' || k === 'lastfm_api_key' || k === 'tag_clean_pattern' || k === 'transcribe_api_key' || k === 'transcribe_language') draft[k] = t.value.trim() ? (k === 'tag_clean_pattern' ? t.value : t.value.trim()) : null;
      else draft[k] = t.value;
      scheduleSave();
    });
    body.addEventListener('change', (e) => {
      const t = e.target;
      if (t.dataset.k === '__scale') { store.set('lf-scale', t.value); invoke('set_ui_scale', { scale: Number(t.value) }).catch((err) => toast(errText(err), 'bad')); return; }
      if (t.dataset.tUse !== undefined) {
        if (t.checked) { if (!draft.providers.includes('ai_transcribe')) draft.providers.push('ai_transcribe'); } else draft.providers = draft.providers.filter((p) => p !== 'ai_transcribe');
        scheduleSave();
        return;
      }
      if (t.dataset.k === 'transcribe_engine') { draft.transcribe_engine = t.value; scheduleSave(); render(); return; }
      if (t.dataset.acctUse) {
        const id = t.dataset.acctUse;
        if (t.checked) { if (!draft.providers.includes(id)) draft.providers.push(id); } else draft.providers = draft.providers.filter((p) => p !== id);
        scheduleSave();
        return;
      }
      const li = t.closest('.sw-plist li');
      if (li && t.type === 'checkbox' && section === 'sources') {
        const id = li.dataset.id;
        if (t.checked) { if (!draft.providers.includes(id)) draft.providers.push(id); } else draft.providers = draft.providers.filter((x) => x !== id);
        render(); scheduleSave();
      }
    });
    body.addEventListener('click', async (e) => {
      const mv = e.target.closest('[data-mv]');
      if (mv && !mv.disabled) {
        const id = mv.closest('li').dataset.id;
        const i = draft.providers.indexOf(id), j = i + Number(mv.dataset.mv);
        if (i < 0 || j < 0 || j >= draft.providers.length) return;
        [draft.providers[i], draft.providers[j]] = [draft.providers[j], draft.providers[i]];
        render(); scheduleSave();
        $(`.sw-plist li[data-id="${CSS.escape(id)}"] [data-mv="${mv.dataset.mv}"]`)?.focus();
        return;
      }
      const sg = e.target.closest('[data-seg]');
      if (sg) {
        $$(`[data-seg="${sg.dataset.seg}"]`).forEach((b) => b.setAttribute('aria-checked', b === sg));
        if (sg.dataset.seg === 'theme') { store.set('lf-theme', sg.dataset.v); applyAppearance(); }
        return;
      }
      const sw = e.target.closest('[data-accent]');
      if (sw) { store.set('lf-accent', sw.dataset.accent); $$('[data-accent]').forEach((b) => b.setAttribute('aria-checked', b === sw)); applyAppearance(); return; }
      const open = e.target.closest('[data-open]');
      if (open) { invoke('open_external', { url: open.dataset.open }).catch((err) => toast(errText(err), 'bad')); return; }
      if (e.target.closest('#test-sources')) {
        const ul = $('#health');
        ul.innerHTML = '<li><i class="led searching" aria-hidden="true"></i><span>Testing every source…</span></li>';
        try {
          const res = await invoke('test_providers');
          const name = (id) => settings.providers.find((p) => p.id === id)?.name || id;
          ul.innerHTML = res.map((r) => `<li><i class="led ${r.ok ? 'on' : 'error'}" aria-hidden="true"></i><b>${esc(name(r.id))}</b><span class="grow">${r.ok ? 'Working' : esc(r.detail)}</span><span class="sw-note">${r.millis} ms</span></li>`).join('');
        } catch (err) { ul.innerHTML = `<li>${esc(errText(err))}</li>`; }
      }
      const go = e.target.closest('[data-acct-go]');
      if (go) {
        const id = go.dataset.acctGo;
        if (acctState[id]?.signedIn) {
          go.disabled = true;
          try { await invoke('account_sign_out', { id }); } catch (err) { toast(errText(err), 'bad'); }
          acctState[id] = { signedIn: false };
          draft.providers = draft.providers.filter((p) => p !== id);
          scheduleSave();
          render();
        } else {
          acctState[id] = { signedIn: false };
          invoke('account_sign_in', { id }).catch((err) => toast(errText(err), 'bad'));
        }
        return;
      }
      if (e.target.closest('#t-download')) {
        tDownload = { done: 0, total: 0 };
        render();
        try { await invoke('transcribe_download_model'); toast('Whisper model ready'); } catch (err) { toast(errText(err), 'bad'); }
        tDownload = null;
        loadTranscribeStatus();
        return;
      }
      if (e.target.closest('#diag-run')) { runDiagnostics(); return; }
      if (e.target.closest('#diag-reveal')) { invoke('reveal_config').catch((err) => toast(errText(err), 'bad')); return; }
      if (e.target.closest('#diag-copy')) {
        const health = $$('#health li').map((li) => li.textContent.split(/\s+/).join(' ').trim()).filter(Boolean);
        const lines = ['Lyrixearch diagnostics', ...lastReport.map((c) => `${mark(c.level)} ${c.name}: ${c.detail}`)];
        if (health.length) lines.push('', 'Sources:', ...health);
        try { await T.clipboardManager.writeText(lines.join('\n')); toast('Report copied'); } catch { toast('Clipboard unavailable', 'bad'); }
        return;
      }
      if (e.target.closest('#plugin-install')) {
        const r = await T.dialog.open({ directory: true, title: 'Choose a plugin folder (it contains plugin.toml)' });
        if (!r) return;
        try {
          settings = await invoke('install_plugin', { dir: Array.isArray(r) ? r[0] : r });
          draft = JSON.parse(JSON.stringify(settings.config));
          render(); toast('Plugin installed and enabled');
        } catch (err) { toast(errText(err), 'bad'); }
      }
    });
    T.event.listen('model-download', (e) => {
      if (!tDownload) return;
      tDownload = e.payload;
      const bar = $('#sw-body progress');
      if (bar) { bar.max = e.payload.total || 1; bar.value = e.payload.done; const b = bar.nextElementSibling; if (b) b.textContent = `${e.payload.total ? Math.round((e.payload.done / e.payload.total) * 100) : 0}%`; }
    });
    // signing in worked (the app noticed and closed the window)
    T.event.listen('account-signed-in', (e) => {
      const { id, service, name } = e.payload;
      acctState[id] = { signedIn: true, name };
      if (!draft.providers.includes(id)) { draft.providers.push(id); scheduleSave(); }
      toast(name ? `Signed in to ${service} as ${name}` : `Signed in to ${service}`);
      if (section === 'accounts') render();
    });
    // a sign-in window was closed: look again
    T.event.listen('accounts-changed', () => { for (const a of ACCOUNTS) checkAccount(a.id); });
    // the main window saved settings too (library folders): keep its changes in this draft
    T.event.listen('settings-changed', (e) => {
      settings = e.payload;
      draft.library_dirs = settings.config.library_dirs;
    });
    // appearance changed in the other window, or the system theme flipped
    window.addEventListener('storage', () => applyAppearance());
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyAppearance());
    document.addEventListener('keydown', (e) => { if (e.key === 'Escape') T.window?.getCurrentWindow?.().close(); });
  }

  async function init() {
    applyAppearance();
    $$('.sw-nav [data-icon]').forEach((el) => { el.innerHTML = icon(el.dataset.icon); });
    settings = await invoke('get_settings');
    ACCOUNTS = await invoke('account_list').catch(() => []);
    draft = JSON.parse(JSON.stringify(settings.config));
    wire();
    render();
  }
  document.addEventListener('DOMContentLoaded', () => init().catch((e) => { $('#sw-body').textContent = errText(e); }));
})();
