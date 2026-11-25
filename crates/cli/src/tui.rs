//! Interactive terminal UI: track table on the left, lyrics preview on the right.

use crate::picker::{Outcome, Picker};
use anyhow::Result;
use crossterm::event::{self, Event as CtEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use lyrixearch_core::{output, pending, scan, tags, util, Config, Engine, Event, Lyrics, Sidecar, SidecarEmbed, Track};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Gauge, Paragraph, Row as TRow, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    Queued,
    Searching(String),
    Found,
    NotFound(String),
    Written,
    Error(String),
}

struct Item {
    id: usize,
    track: Track,
    status: Status,
    found: Option<Lyrics>,
    embedded: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    With,
    Without,
}

enum Mode {
    Normal,
    Picker(Box<Picker>),
    Search(String),
    ConfirmDelete,
    Help,
}

struct App {
    handle: Handle,
    cfg: Config,
    engine: Arc<Engine>,
    items: Vec<Item>,
    next_id: usize,
    visible: Vec<usize>,
    table: TableState,
    marked: HashSet<usize>,
    filter: Filter,
    text: String,
    mode: Mode,
    tx: UnboundedSender<Event>,
    rx: UnboundedReceiver<Event>,
    cancel: Arc<AtomicBool>,
    queued_total: usize,
    queued_done: usize,
    run_found: usize,
    run_missed: usize,
    current: String,
    scroll: u16,
    message: String,
    browser: usize,
    quit: bool,
}

impl App {
    fn new(handle: Handle, cfg: Config) -> Result<Self> {
        let (tx, rx) = unbounded_channel();
        Ok(Self {
            engine: Engine::new(cfg.clone())?,
            handle,
            cfg,
            items: Vec::new(),
            next_id: 0,
            visible: Vec::new(),
            table: TableState::default(),
            marked: HashSet::new(),
            filter: Filter::All,
            text: String::new(),
            mode: Mode::Normal,
            tx,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            queued_total: 0,
            queued_done: 0,
            run_found: 0,
            run_missed: 0,
            current: String::new(),
            scroll: 0,
            message: "press ? for help".into(),
            browser: 0,
            quit: false,
        })
    }

    fn add_paths(&mut self, paths: &[PathBuf]) {
        let files = scan::collect_audio(paths, true);
        let pending = pending::load();
        let mut restored = 0;
        let known: HashSet<PathBuf> = self.items.iter().map(|i| i.track.path.clone()).collect();
        let (mut added, mut bad) = (0, 0);
        for f in files.into_iter().filter(|f| !known.contains(f)) {
            match tags::read_track(&f) {
                Ok(track) => {
                    let embedded = if track.has_lyrics { tags::read_lyrics(&track.path) } else { None };
                    let found = pending.as_ref().ok().and_then(|map| map.get(&util::pending_key(&track.path))).cloned();
                    restored += usize::from(found.is_some());
                    let status = if found.is_some() { Status::Found } else { Status::Idle };
                    self.items.push(Item { id: self.next_id, track, status, found, embedded });
                    self.next_id += 1;
                    added += 1;
                }
                Err(_) => bad += 1,
            }
        }
        self.message = format!("added {added} track(s){}", if bad > 0 { format!(", {bad} unreadable") } else { String::new() });
        if restored > 0 {
            self.message.push_str(&format!(" · restored {restored} pending result(s)"));
        }
        if let Err(e) = pending {
            self.message.push_str(&format!(" · could not load pending lyrics: {e:#}"));
        }
        self.refilter();
    }

    fn forget_pending(&mut self, paths: &[PathBuf]) {
        if !paths.is_empty() {
            if let Err(e) = pending::update(|map| {
                for path in paths {
                    map.remove(path);
                }
            }) {
                self.message.push_str(&format!(" · could not clear pending lyrics: {e:#}"));
            }
        }
    }

    /// Add what the picker chose, remember it for "run last again", and optionally search just the new tracks.
    fn finish_add(&mut self, paths: Vec<PathBuf>, search_new: bool) {
        self.cfg.last_added = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        self.cfg.tui_search_new = search_new;
        let saved = self.cfg.save().is_ok();
        let before = self.items.len();
        self.add_paths(&paths);
        if !saved {
            self.message.push_str(" (could not save settings)");
        }
        if search_new && self.items.len() > before {
            let work = (before..self.items.len())
                .filter(|&i| self.items[i].found.is_none() && (self.cfg.overwrite || !self.items[i].track.has_lyrics))
                .collect::<Vec<_>>();
            if !work.is_empty() {
                let added = self.message.clone();
                self.queue(work);
                self.message = format!("{added}; {}", self.message);
            }
        }
    }

    fn refilter(&mut self) {
        let needle = self.text.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| match self.filter {
                Filter::All => true,
                Filter::With => it.track.has_lyrics,
                Filter::Without => !it.track.has_lyrics,
            })
            .filter(|(_, it)| {
                needle.is_empty()
                    || it.track.query.artist.to_lowercase().contains(&needle)
                    || it.track.query.title.to_lowercase().contains(&needle)
                    || it.track.display_name().to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        let sel = self.table.selected().unwrap_or(0).min(self.visible.len().saturating_sub(1));
        self.table.select(if self.visible.is_empty() { None } else { Some(sel) });
    }

    fn current(&self) -> Option<usize> {
        self.table.selected().and_then(|i| self.visible.get(i).copied())
    }

    /// Marked rows, or just the highlighted one.
    fn targets(&self) -> Vec<usize> {
        if self.marked.is_empty() {
            self.current().into_iter().collect()
        } else {
            let mut v: Vec<usize> = self.marked.iter().copied().filter(|i| self.visible.contains(i)).collect();
            v.sort_unstable();
            v
        }
    }

    fn start_search(&mut self, all: bool) {
        let idxs: Vec<usize> = if all { self.visible.clone() } else { self.targets() };
        let idxs: Vec<usize> = idxs.into_iter().filter(|&i| self.cfg.overwrite || !self.items[i].track.has_lyrics).collect();
        self.queue(idxs);
    }

    /// Queue these tracks for searching (those already queued or in flight are skipped).
    fn queue(&mut self, idxs: Vec<usize>) {
        let idxs: Vec<usize> = idxs.into_iter().filter(|&i| !matches!(self.items[i].status, Status::Queued | Status::Searching(_))).collect();
        if idxs.is_empty() {
            self.message = "nothing to search (tracks already have lyrics — press O to allow overwriting)".into();
            return;
        }
        if self.queued_done >= self.queued_total {
            self.queued_total = 0;
            self.queued_done = 0;
            self.run_found = 0;
            self.run_missed = 0;
            self.current.clear();
            self.cancel.store(false, Ordering::Relaxed);
        }
        self.queued_total += idxs.len();
        let mut work = Vec::new();
        for i in idxs {
            self.items[i].status = Status::Queued;
            work.push((self.items[i].id, self.items[i].track.query.clone()));
        }
        self.message = format!("searching {} track(s)…", work.len());
        let (engine, tx, cancel) = (self.engine.clone(), self.tx.clone(), self.cancel.clone());
        self.handle.spawn(async move { engine.run(work, tx, cancel).await });
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            let id = match &ev {
                Event::Trying { idx, .. } => *idx,
                Event::Found { idx, .. } | Event::Failed { idx, .. } | Event::NotFound { idx, .. } | Event::Cancelled { idx } => {
                    self.queued_done += 1;
                    *idx
                }
                Event::Skipped { .. } | Event::Tried { .. } => continue,
                Event::Challenge { provider, .. } => {
                    self.message = format!("{provider} wants a bot check passed: the desktop app opens a window for that");
                    continue;
                }
            };
            let Some(item) = self.items.iter_mut().find(|item| item.id == id) else { continue };
            match ev {
                Event::Trying { provider, .. } => {
                    self.current = format!("{} via {provider}", item.track.query.title);
                    item.status = Status::Searching(provider);
                }
                Event::Found { lyrics, .. } => {
                    self.run_found += 1;
                    if item.track.path.is_file() {
                        let key = util::pending_key(&item.track.path);
                        if let Err(e) = pending::update(|map| map.insert(key, (*lyrics).clone())) {
                            self.message = format!("could not save pending lyrics: {e:#}");
                        }
                    }
                    item.found = Some(*lyrics);
                    item.status = Status::Found;
                }
                Event::Failed { notes, .. } => item.status = Status::Error(notes.join("; ")),
                Event::NotFound { notes, .. } => {
                    self.run_missed += 1;
                    item.status = Status::NotFound(notes.join("; "));
                }
                Event::Cancelled { .. } => item.status = Status::Idle,
                Event::Skipped { .. } | Event::Tried { .. } | Event::Challenge { .. } => unreachable!(),
            }
        }
    }

    /// Embed the `.lrc` / `.txt` that sits next to each marked (or current) track.
    fn embed_from_files(&mut self) {
        let (mut ok, mut missing, mut failed) = (0, 0, 0);
        let mut written = Vec::new();
        for i in self.targets() {
            let item = &mut self.items[i];
            match output::embed_from_sidecar(&item.track, &self.cfg, true, None, false) {
                Ok(SidecarEmbed::Embedded(_)) => {
                    ok += 1;
                    if item.found.is_some() {
                        written.push(util::pending_key(&item.track.path));
                    }
                    item.track.has_lyrics = true;
                    item.embedded = tags::read_lyrics(&item.track.path);
                    item.found = None;
                    item.status = Status::Written;
                }
                Ok(SidecarEmbed::NoFile) => missing += 1,
                Ok(SidecarEmbed::AlreadyHad | SidecarEmbed::WouldEmbed(_)) => unreachable!("forced write with dry run disabled"),
                Err(e) => {
                    failed += 1;
                    item.status = Status::Error(format!("{e:#}"));
                }
            }
        }
        self.message = format!(
            "embedded {ok} from .lrc / .txt · {missing} had no matching file{}",
            if failed > 0 { format!(" · {failed} failed") } else { String::new() }
        );
        self.forget_pending(&written);
        self.refilter();
    }

    fn write_found(&mut self) {
        let mut ok = 0;
        let mut err = 0;
        let mut written = Vec::new();
        for i in self.targets() {
            let Some(l) = self.items[i].found.clone() else { continue };
            match self.engine.apply(&self.items[i].track, &l) {
                Ok(applied) if applied.tag_written || applied.sidecar.is_some() => {
                    ok += 1;
                    written.push(util::pending_key(&self.items[i].track.path));
                    self.items[i].status = Status::Written;
                    if applied.tag_written {
                        self.items[i].track.has_lyrics = true;
                        self.items[i].embedded = l.embed_text();
                    }
                }
                Ok(_) => {
                    err += 1;
                    self.items[i].status = Status::Error("no output written; check tag and sidecar settings".into());
                }
                Err(e) => {
                    err += 1;
                    self.items[i].status = Status::Error(format!("{e:#}"));
                }
            }
        }
        self.message = format!("wrote {ok} file(s){}", if err > 0 { format!(", {err} failed (see preview)") } else { String::new() });
        self.forget_pending(&written);
        self.refilter();
    }

    fn delete_embedded(&mut self) {
        let mut n = 0;
        for i in self.targets() {
            if self.items[i].track.has_lyrics && tags::remove_lyrics(&self.items[i].track.path, self.cfg.keep_mtime).is_ok() {
                self.items[i].track.has_lyrics = false;
                self.items[i].embedded = None;
                self.items[i].status = Status::Idle;
                n += 1;
            }
        }
        self.message = format!("removed lyrics from {n} file(s)");
        self.refilter();
    }

    fn edit_current(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let Some(i) = self.current() else { return Ok(()) };
        let start = self.items[i].found.as_ref().and_then(|l| l.embed_text()).or_else(|| self.items[i].embedded.clone()).unwrap_or_default();
        let tmp = std::env::temp_dir().join(format!("lyrixearch-edit-{}.txt", std::process::id()));
        std::fs::write(&tmp, &start)?;
        let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());

        ratatui::restore();
        let status = std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$LYRIXEARCH_FILE\"")).env("LYRIXEARCH_FILE", &tmp).status();
        *terminal = ratatui::init();

        let edited = std::fs::read_to_string(&tmp).unwrap_or_default();
        let _ = std::fs::remove_file(&tmp);
        match status {
            Ok(s) if s.success() && edited.trim() != start.trim() && !edited.trim().is_empty() => {
                match tags::write_lyrics(&self.items[i].track.path, edited.trim(), self.cfg.keep_mtime) {
                    Ok(()) => {
                        self.items[i].track.has_lyrics = true;
                        self.items[i].embedded = Some(edited.trim().to_string());
                        self.items[i].status = Status::Written;
                        self.message = "saved edited lyrics".into();
                        if self.items[i].found.take().is_some() {
                            self.forget_pending(&[util::pending_key(&self.items[i].track.path)]);
                        }
                    }
                    Err(e) => self.message = format!("save failed: {e:#}"),
                }
            }
            Ok(_) => self.message = "edit cancelled / unchanged".into(),
            Err(e) => self.message = format!("could not start `{editor}`: {e}"),
        }
        self.refilter();
        Ok(())
    }

    fn rebuild_engine(&mut self) {
        match Engine::new(self.cfg.clone()) {
            Ok(e) => self.engine = e,
            Err(e) => self.message = format!("{e:#}"),
        }
    }

    fn move_sel(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as isize;
        self.table.select(Some((cur + delta).clamp(0, self.visible.len() as isize - 1) as usize));
        self.scroll = 0;
    }

    fn on_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match &mut self.mode {
            Mode::Picker(p) => {
                let out = p.key(key);
                let search_new = p.search_new;
                match out {
                    Outcome::Stay => {}
                    Outcome::Cancel => {
                        self.cfg.tui_search_new = search_new;
                        self.mode = Mode::Normal;
                    }
                    Outcome::Add(paths) => {
                        self.mode = Mode::Normal;
                        self.finish_add(paths, search_new);
                    }
                }
                return Ok(());
            }
            Mode::Search(buf) => {
                match key.code {
                    KeyCode::Enter | KeyCode::Esc => {
                        if key.code == KeyCode::Esc {
                            buf.clear();
                        }
                        self.text = buf.clone();
                        self.mode = Mode::Normal;
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                        self.text = buf.clone();
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        self.text = buf.clone();
                    }
                    _ => {}
                }
                self.refilter();
                return Ok(());
            }
            Mode::ConfirmDelete => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    self.delete_embedded();
                }
                self.mode = Mode::Normal;
                return Ok(());
            }
            Mode::Help => {
                self.mode = Mode::Normal;
                return Ok(());
            }
            Mode::Normal => {}
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.quit = true,
            (KeyCode::Char('?'), _) => self.mode = Mode::Help,
            (KeyCode::Down | KeyCode::Char('j'), _) => self.move_sel(1),
            (KeyCode::Up | KeyCode::Char('k'), _) => self.move_sel(-1),
            (KeyCode::Home | KeyCode::Char('g'), _) => self.move_sel(isize::MIN / 2),
            (KeyCode::End | KeyCode::Char('G'), _) => self.move_sel(isize::MAX / 2),
            (KeyCode::PageDown, _) => self.scroll = self.scroll.saturating_add(10),
            (KeyCode::PageUp, _) => self.scroll = self.scroll.saturating_sub(10),
            (KeyCode::Char(' '), _) => {
                if let Some(i) = self.current() {
                    if !self.marked.remove(&i) {
                        self.marked.insert(i);
                    }
                    self.move_sel(1);
                }
            }
            (KeyCode::Char('A'), _) => {
                if self.marked.len() == self.visible.len() {
                    self.marked.clear();
                } else {
                    self.marked = self.visible.iter().copied().collect();
                }
            }
            (KeyCode::Char('a'), _) => self.mode = Mode::Picker(Box::new(Picker::new(&self.cfg.browse_roots, &self.cfg.last_added, self.cfg.tui_search_new))),
            (KeyCode::Char('/'), _) => self.mode = Mode::Search(self.text.clone()),
            (KeyCode::Char('f'), _) => {
                self.filter = match self.filter {
                    Filter::All => Filter::Without,
                    Filter::Without => Filter::With,
                    Filter::With => Filter::All,
                };
                self.refilter();
            }
            (KeyCode::Char('s') | KeyCode::Enter, _) => self.start_search(false),
            (KeyCode::Char('S'), _) => self.start_search(true),
            (KeyCode::Char('c'), _) => {
                self.cancel.store(true, Ordering::Relaxed);
                self.message = "cancelling queued searches…".into();
            }
            (KeyCode::Char('w'), _) => self.write_found(),
            (KeyCode::Char('l'), _) => self.embed_from_files(),
            (KeyCode::Char('o'), _) => {
                if let Some(i) = self.current() {
                    self.message = match util::reveal(&self.items[i].track.path) {
                        Ok(()) => "shown in the file browser".into(),
                        Err(e) => format!("could not open the file browser: {e}"),
                    };
                }
            }
            (KeyCode::Char('e'), _) => self.edit_current(terminal)?,
            (KeyCode::Char('d'), _) => {
                if self.targets().iter().any(|&i| self.items[i].track.has_lyrics) {
                    self.mode = Mode::ConfirmDelete;
                }
            }
            (KeyCode::Char('b'), _) => {
                if let Some(i) = self.current() {
                    let q = &self.items[i].track.query;
                    let engine = util::SEARCH_ENGINES[self.browser % util::SEARCH_ENGINES.len()];
                    self.message = match util::open_url(&util::search_url(engine, &q.artist, &q.title)) {
                        Ok(()) => format!("opened {engine} search"),
                        Err(e) => format!("could not open browser: {e}"),
                    };
                }
            }
            (KeyCode::Char('B'), _) => {
                self.browser += 1;
                self.message = format!("browser search engine: {}", util::SEARCH_ENGINES[self.browser % util::SEARCH_ENGINES.len()]);
            }
            (KeyCode::Char('r'), _) => {
                let rm: HashSet<usize> = self.targets().into_iter().collect();
                let discarded =
                    rm.iter().filter_map(|&i| self.items[i].found.as_ref().map(|_| util::pending_key(&self.items[i].track.path))).collect::<Vec<_>>();
                let mut idx = 0;
                self.items.retain(|_| {
                    let keep = !rm.contains(&idx);
                    idx += 1;
                    keep
                });
                self.marked.clear();
                self.refilter();
                self.message = format!("removed {} from the list (files untouched)", rm.len());
                self.forget_pending(&discarded);
            }
            (KeyCode::Char('t'), _) => {
                self.cfg.write_tags = !self.cfg.write_tags;
                self.rebuild_engine();
                self.message = format!("write tags: {}", self.cfg.write_tags);
            }
            (KeyCode::Char('x'), _) => {
                self.cfg.sidecar = match self.cfg.sidecar {
                    Sidecar::Off => Sidecar::Auto,
                    Sidecar::Auto => Sidecar::Lrc,
                    Sidecar::Lrc => Sidecar::Txt,
                    Sidecar::Txt => Sidecar::Off,
                };
                self.rebuild_engine();
                self.message = format!("sidecar files: {:?}", self.cfg.sidecar).to_lowercase();
            }
            (KeyCode::Char('O'), _) => {
                self.cfg.overwrite = !self.cfg.overwrite;
                self.rebuild_engine();
                self.message = format!("overwrite existing lyrics: {}", self.cfg.overwrite);
            }
            (KeyCode::Char('W'), _) => {
                let _ = self.cfg.save();
                self.message = "settings saved".into();
            }
            _ => {}
        }
        Ok(())
    }
}

pub fn run(handle: Handle, paths: Vec<PathBuf>, cfg: Config) -> Result<()> {
    let mut app = App::new(handle, cfg)?;
    // No paths given: open the saved library folders, if any; otherwise say how to start.
    let start: Vec<PathBuf> = if paths.is_empty() { app.cfg.library_dirs.iter().map(PathBuf::from).filter(|p| p.exists()).collect() } else { paths };
    // Open the screen first: reading the tags of a big folder on an external drive takes a while,
    // and a blank terminal looks like a hang.
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        if start.is_empty() {
            app.message = "no tracks yet: press a to add a folder, or run `lyrixearch tui ~/Music`  (? for help)".into();
        } else {
            app.message = "scanning folders and reading tags, one moment...".into();
            terminal.draw(|f| draw(f, &mut app))?;
            app.add_paths(&start);
        }
        event_loop(&mut terminal, &mut app)
    })();
    ratatui::restore();
    app.cancel.store(true, Ordering::Relaxed);
    result
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        app.drain_events();
        terminal.draw(|f| draw(f, app))?;
        if event::poll(Duration::from_millis(80))? {
            if let CtEvent::Key(k) = event::read()? {
                app.on_key(k, terminal)?;
            }
        }
    }
    Ok(())
}

const ACCENT: Color = Color::Rgb(137, 180, 250);
const DIM: Color = Color::Rgb(108, 112, 134);

fn status_cell(s: &Status) -> Span<'static> {
    match s {
        Status::Idle => Span::styled("·", Style::new().fg(DIM)),
        Status::Queued => Span::styled("…", Style::new().fg(Color::Yellow)),
        Status::Searching(_) => Span::styled("⟳", Style::new().fg(Color::Yellow)),
        Status::Found => Span::styled("✓", Style::new().fg(Color::Green).bold()),
        Status::NotFound(_) => Span::styled("✗", Style::new().fg(Color::Red)),
        Status::Written => Span::styled("●", Style::new().fg(Color::Cyan).bold()),
        Status::Error(_) => Span::styled("!", Style::new().fg(Color::Red).bold()),
    }
}

fn draw(f: &mut Frame, app: &mut App) {
    let [header, body, footer] = Layout::vertical([Constraint::Length(1), Constraint::Min(5), Constraint::Length(3)]).areas(f.area());

    let with = app.items.iter().filter(|i| i.track.has_lyrics).count();
    let filter_name = match app.filter {
        Filter::All => "all",
        Filter::With => "with lyrics",
        Filter::Without => "without lyrics",
    };
    f.render_widget(
        Line::from(vec![
            Span::styled(" lyrixearch ", Style::new().bg(ACCENT).fg(Color::Black).bold()),
            Span::raw(format!(
                "  {} files · {} with lyrics · {} left to search · {} not found · showing {} ({filter_name}){}",
                app.items.len(),
                with,
                app.items.len() - with,
                app.items.iter().filter(|i| matches!(i.status, Status::NotFound(_))).count(),
                app.visible.len(),
                if app.text.is_empty() { String::new() } else { format!(" · filter “{}”", app.text) }
            )),
        ]),
        header,
    );

    let [left, right] = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(body);

    // track table
    let rows: Vec<TRow> = app
        .visible
        .iter()
        .map(|&i| {
            let it = &app.items[i];
            let mark = if app.marked.contains(&i) { "▌" } else { " " };
            let src = it.found.as_ref().map(|l| l.source.clone()).unwrap_or_default();
            TRow::new(vec![
                Line::from(vec![Span::styled(mark, Style::new().fg(ACCENT)), status_cell(&it.status)]),
                Line::from(if it.track.has_lyrics { "♪" } else { " " }).style(Style::new().fg(Color::Magenta)),
                Line::from(it.track.query.artist.clone()),
                Line::from(it.track.query.title.clone()),
                Line::from(src).style(Style::new().fg(DIM)),
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(2), Constraint::Length(1), Constraint::Percentage(30), Constraint::Percentage(45), Constraint::Min(8)])
        .header(TRow::new(["", "", "Artist", "Title", "Source"]).style(Style::new().fg(DIM).bold()))
        .block(Block::bordered().border_type(BorderType::Rounded).title(" Tracks ").border_style(Style::new().fg(ACCENT)))
        .row_highlight_style(Style::new().bg(Color::Rgb(49, 50, 68)).add_modifier(Modifier::BOLD))
        .highlight_symbol("› ");
    f.render_stateful_widget(table, left, &mut app.table);

    // preview
    let mut lines: Vec<Line> = Vec::new();
    let mut title = " Lyrics ".to_string();
    if let Some(i) = app.current() {
        let it = &app.items[i];
        lines.push(Line::from(vec![
            Span::styled(it.track.query.title.clone(), Style::new().bold()),
            Span::raw("  "),
            Span::styled(it.track.query.artist.clone(), Style::new().fg(DIM)),
        ]));
        lines.push(Line::from(Span::styled(it.track.path.display().to_string(), Style::new().fg(DIM))));
        match &it.status {
            Status::Searching(p) => lines.push(Line::from(Span::styled(format!("searching… {p}"), Style::new().fg(Color::Yellow)))),
            Status::NotFound(n) => lines.push(Line::from(Span::styled(format!("not found — {n}"), Style::new().fg(Color::Red)))),
            Status::Error(e) => lines.push(Line::from(Span::styled(format!("error — {e}"), Style::new().fg(Color::Red)))),
            _ => {}
        }
        if let Some(l) = &it.found {
            title = format!(" Found via {} · {:.0}% {}", l.source, l.score * 100.0, if l.synced.is_some() { "· synced " } else { "" });
            lines.push(Line::from(Span::styled(format!("match: {} — {}", l.matched_artist, l.matched_title), Style::new().fg(Color::Green))));
            lines.push(Line::raw(""));
            lines.extend(l.embed_text().unwrap_or_default().lines().map(|s| Line::raw(s.to_string())));
        } else if let Some(e) = &it.embedded {
            title = " Embedded lyrics ".into();
            lines.push(Line::raw(""));
            lines.extend(e.lines().map(|s| Line::raw(s.to_string())));
        } else {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled("no lyrics — press s to search", Style::new().fg(DIM))));
        }
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((app.scroll, 0)).block(Block::bordered().border_type(BorderType::Rounded).title(title)),
        right,
    );

    // footer: progress + hints
    let [prog, hints] = Layout::vertical([Constraint::Length(1), Constraint::Length(2)]).areas(footer);
    if app.queued_total > 0 && app.queued_done < app.queued_total {
        f.render_widget(
            Gauge::default()
                .ratio(app.queued_done as f64 / app.queued_total as f64)
                .label(format!(
                    "Searching {}/{}{} · {} found · {} not found",
                    app.queued_done,
                    app.queued_total,
                    if app.current.is_empty() { String::new() } else { format!(" · {}", app.current) },
                    app.run_found,
                    app.run_missed
                ))
                .gauge_style(Style::new().fg(ACCENT).bg(Color::Rgb(49, 50, 68))),
            prog,
        );
    } else if app.queued_total > 0 {
        f.render_widget(
            Line::from(Span::styled(
                format!(" Done · {} found, {} not found. Press w to write what was found. {}", app.run_found, app.run_missed, app.message),
                Style::new().fg(Color::Yellow),
            )),
            prog,
        );
    } else {
        f.render_widget(Line::from(Span::styled(format!(" {}", app.message), Style::new().fg(Color::Yellow))), prog);
    }
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                key("a", "add"),
                key("space", "mark"),
                key("s", "search"),
                key("S", "search all"),
                key("w", "write"),
                key("e", "edit"),
                key("d", "delete"),
                key("l", "from .lrc/.txt"),
                key("o", "reveal"),
            ]),
            Line::from(vec![
                key("b", "browser"),
                key("f", "filter"),
                key("/", "find"),
                key("t", &format!("tags:{}", on(app.cfg.write_tags))),
                key("x", &format!("sidecar:{:?}", app.cfg.sidecar).to_lowercase()),
                key("?", "help"),
                key("q", "quit"),
            ]),
        ]),
        hints,
    );

    // overlays
    match &mut app.mode {
        Mode::Picker(p) => p.render(f),
        Mode::Search(buf) => prompt(f, "Filter by artist / title / filename", buf),
        Mode::ConfirmDelete => prompt(f, "Remove embedded lyrics from the selected tracks?  (y/N)", ""),
        Mode::Help => help(f),
        Mode::Normal => {}
    }
}

fn on(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn key(k: &str, label: &str) -> Span<'static> {
    Span::styled(format!("[{k}] {label}  "), Style::new().fg(DIM))
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, c, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w.min(area.width)), Constraint::Fill(1)]).areas(mid);
    c
}

fn prompt(f: &mut Frame, title: &str, buf: &str) {
    let area = centered(f.area(), 70, 3);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(format!("{buf}▏"))
            .block(Block::bordered().border_type(BorderType::Rounded).title(format!(" {title} ")).border_style(Style::new().fg(ACCENT))),
        area,
    );
}

fn help(f: &mut Frame) {
    let text = "\
 Navigation     ↑/↓ j/k move · g/G top/bottom · PgUp/PgDn scroll lyrics
 Selection      space mark/unmark · A mark all · (no marks = current row)
 Tracks         a add folders (shortcuts, browse, type a path, run last again) · r drop from list · / filter · f cycle with/without lyrics
 Search         s search marked/current · S search everything shown · c cancel
 Results        w write found lyrics · e edit in $EDITOR · d delete embedded lyrics
 Files          l embed the .lrc / .txt beside each track · o show in the file browser
 Browser        b search the web for the current track · B change search engine
 Settings       t tags on/off · x sidecar mode (off/auto/lrc/txt) · O overwrite · W save settings
 Other          ? this help · q quit

 Searching never writes anything by itself: review results, then press w.";
    let area = centered(f.area(), 92, 14);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().border_type(BorderType::Rounded).title(" Help — any key closes ").border_style(Style::new().fg(ACCENT)))
            .style(Style::new())
            .wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn search_events_follow_track_ids_after_removal() -> Result<()> {
        let mut app = App::new(Handle::current(), Config { providers: vec!["local_files".into()], ..Config::default() })?;
        let track = Track { path: "remaining.flac".into(), query: Default::default(), has_lyrics: false, format: "flac".into() };
        app.items.push(Item { id: 1, track, status: Status::Queued, found: None, embedded: None });
        app.queued_total = 2;
        for (idx, source) in [(1, "remaining"), (0, "removed")] {
            app.tx.send(Event::Found { idx, lyrics: Box::new(Lyrics { source: source.into(), ..Lyrics::default() }) })?;
        }
        app.drain_events();
        assert_eq!(app.items[0].found.as_ref().map(|lyrics| lyrics.source.as_str()), Some("remaining"));
        assert_eq!(app.queued_done, app.queued_total);
        assert_eq!(app.run_found, 1);
        Ok(())
    }
}
