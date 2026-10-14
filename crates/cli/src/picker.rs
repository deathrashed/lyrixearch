//! Folder picker for the terminal UI: shortcuts from the config, "run last again", browsing
//! from any folder, and typing a path (with Tab completion).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const ACCENT: Color = Color::Rgb(137, 180, 250);
const DIM: Color = Color::Rgb(108, 112, 134);
const GOOD: Color = Color::Rgb(166, 227, 161);

#[derive(Clone)]
enum Entry {
    RunLast,
    Root(PathBuf),
    AddHere,
    Home,
    Dir(PathBuf),
    Type,
}

pub enum Outcome {
    Stay,
    Cancel,
    Add(Vec<PathBuf>),
}

pub struct Picker {
    /// `None` is the shortcuts screen.
    dir: Option<PathBuf>,
    entries: Vec<Entry>,
    state: ListState,
    picked: HashSet<PathBuf>,
    typing: Option<String>,
    roots: Vec<PathBuf>,
    last: Vec<PathBuf>,
    pub search_new: bool,
    note: String,
}

pub fn expand_home(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(p)
}

fn hidden(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "@eaDir" | "$RECYCLE.BIN" | "System Volume Information" | "lost+found")
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir() && !hidden(&e.file_name().to_string_lossy())).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
    v
}

/// Tab completion for a typed path: the folders that start with what was typed, extended to their common prefix.
fn complete(buf: &str) -> String {
    let expanded = expand_home(buf).to_string_lossy().into_owned();
    let (dir, prefix) = match expanded.rfind('/') {
        Some(i) => (expanded[..=i].to_string(), expanded[i + 1..].to_string()),
        None => return buf.to_string(),
    };
    let names: Vec<String> =
        subfolders(Path::new(&dir)).iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).filter(|n| n.starts_with(&prefix)).collect();
    match names.len() {
        0 => buf.to_string(),
        1 => format!("{dir}{}/", names[0]),
        _ => {
            let mut common = names[0].clone();
            for n in &names[1..] {
                while !n.starts_with(&common) {
                    common.pop();
                }
            }
            format!("{dir}{common}")
        }
    }
}

impl Picker {
    pub fn new(roots: &[String], last: &[String], search_new: bool) -> Self {
        let mut roots: Vec<PathBuf> = roots.iter().map(|r| expand_home(r)).collect();
        if roots.is_empty() {
            roots.extend(dirs::audio_dir().or_else(|| dirs::home_dir().map(|h| h.join("Music"))));
            roots.push(PathBuf::from(if cfg!(windows) { "C:\\" } else { "/Volumes" }));
        }
        let mut p = Self {
            dir: None,
            entries: Vec::new(),
            state: ListState::default(),
            picked: HashSet::new(),
            typing: None,
            roots,
            last: last.iter().map(|s| expand_home(s)).collect(),
            search_new,
            note: String::new(),
        };
        p.reload();
        p
    }

    fn reload(&mut self) {
        self.entries.clear();
        match &self.dir {
            None => {
                if !self.last.is_empty() {
                    self.entries.push(Entry::RunLast);
                }
                self.entries.extend(self.roots.iter().cloned().map(Entry::Root));
                self.entries.push(Entry::Type);
            }
            Some(d) => {
                self.entries.push(Entry::AddHere);
                self.entries.push(Entry::Home);
                self.entries.extend(subfolders(d).into_iter().map(Entry::Dir));
                self.entries.push(Entry::Type);
            }
        }
        self.state.select(Some(if self.dir.is_some() && self.entries.len() > 2 { 2 } else { 0 }));
    }

    fn enter(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.dir = Some(path);
            self.note.clear();
            self.reload();
        } else {
            self.note = format!("{} is not available (is the drive mounted?)", path.display());
        }
    }

    fn up(&mut self) {
        let Some(d) = self.dir.clone() else { return };
        if self.roots.contains(&d) {
            self.dir = None;
            self.reload();
            return;
        }
        match d.parent() {
            Some(p) if p != d => {
                let from = d.clone();
                self.dir = Some(p.to_path_buf());
                self.reload();
                // leave the cursor on the folder we came from
                if let Some(i) = self.entries.iter().position(|e| matches!(e, Entry::Dir(x) if *x == from)) {
                    self.state.select(Some(i));
                }
            }
            _ => {
                self.dir = None;
                self.reload();
            }
        }
    }

    fn type_path(&mut self) {
        let start = match &self.dir {
            Some(d) => format!("{}/", d.display()).replace("//", "/"),
            None => self.roots.first().map(|r| format!("{}/", r.display()).replace("//", "/")).unwrap_or_else(|| "/".into()),
        };
        self.typing = Some(start);
    }

    fn highlighted(&self) -> Option<&Entry> {
        self.state.selected().and_then(|i| self.entries.get(i))
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.entries.len() as isize;
        if n == 0 {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        self.state.select(Some((cur + delta).clamp(0, n - 1) as usize));
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if let Some(buf) = self.typing.as_mut() {
            match key.code {
                KeyCode::Esc => self.typing = None,
                KeyCode::Enter => {
                    let p = expand_home(buf.trim());
                    if p.exists() {
                        return Outcome::Add(vec![p]);
                    }
                    self.note = format!("no such path: {}", p.display());
                }
                KeyCode::Tab => *buf = complete(buf),
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => buf.clear(),
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            }
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Outcome::Cancel,
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Home | KeyCode::Char('g') => self.state.select(Some(0)),
            KeyCode::End | KeyCode::Char('G') => self.state.select(Some(self.entries.len().saturating_sub(1))),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => self.up(),
            KeyCode::Char('n') => self.search_new = !self.search_new,
            KeyCode::Char('/') | KeyCode::Char(':') | KeyCode::Char('~') => self.type_path(),
            KeyCode::Char('r') if !self.last.is_empty() => return Outcome::Add(self.last.clone()),
            KeyCode::Char(' ') => {
                if let Some(Entry::Dir(p) | Entry::Root(p)) = self.highlighted().cloned() {
                    if !self.picked.remove(&p) {
                        self.picked.insert(p);
                    }
                    self.move_by(1);
                }
            }
            KeyCode::Char('a') => {
                if !self.picked.is_empty() {
                    let mut v: Vec<PathBuf> = self.picked.iter().cloned().collect();
                    v.sort();
                    return Outcome::Add(v);
                }
                match self.highlighted().cloned() {
                    Some(Entry::Dir(p) | Entry::Root(p)) => return Outcome::Add(vec![p]),
                    Some(Entry::RunLast) => return Outcome::Add(self.last.clone()),
                    _ => {
                        if let Some(d) = self.dir.clone() {
                            return Outcome::Add(vec![d]);
                        }
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => match self.highlighted().cloned() {
                Some(Entry::RunLast) => return Outcome::Add(self.last.clone()),
                Some(Entry::Root(p) | Entry::Dir(p)) => self.enter(p),
                Some(Entry::AddHere) => {
                    if let Some(d) = self.dir.clone() {
                        return Outcome::Add(vec![d]);
                    }
                }
                Some(Entry::Home) => {
                    self.dir = None;
                    self.reload();
                }
                Some(Entry::Type) => self.type_path(),
                None => {}
            },
            _ => {}
        }
        Outcome::Stay
    }

    pub fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        let w = 84.min(area.width);
        let h = (self.entries.len() as u16 + 7).clamp(12, area.height.saturating_sub(2).max(12)).min(area.height);
        let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
        let [_, rect, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
        f.render_widget(Clear, rect);
        let title = match &self.dir {
            Some(d) => format!(" Add folders · {} ", d.display()),
            None => " Add folders · pick a shortcut ".into(),
        };
        let block = Block::bordered().border_type(BorderType::Rounded).title(title).border_style(Style::new().fg(ACCENT));
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let [list_area, foot] = Layout::vertical([Constraint::Fill(1), Constraint::Length(4)]).areas(inner);

        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|e| match e {
                Entry::RunLast => {
                    let names: Vec<String> =
                        self.last.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())).collect();
                    ListItem::new(Line::from(vec![
                        Span::styled("↻ Run last again  ", Style::new().fg(GOOD).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("({})", names.join(", ")), Style::new().fg(DIM)),
                    ]))
                }
                Entry::Root(p) => {
                    let ok = p.is_dir();
                    ListItem::new(Line::from(vec![
                        Span::raw(if self.picked.contains(p) { "[x] " } else { "▸ " }),
                        Span::raw(p.display().to_string()),
                        Span::styled(if ok { "" } else { "   (not available)" }, Style::new().fg(DIM)),
                    ]))
                }
                Entry::AddHere => ListItem::new(Span::styled("✓ Add this folder", Style::new().fg(GOOD).add_modifier(Modifier::BOLD))),
                Entry::Home => ListItem::new(Span::styled("⌂ back to the shortcuts", Style::new().fg(DIM))),
                Entry::Dir(p) => {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ListItem::new(format!("{} {name}/", if self.picked.contains(p) { "[x]" } else { "   " }))
                }
                Entry::Type => ListItem::new(Span::styled("✎ Type a path…  (/ also works)", Style::new().fg(DIM))),
            })
            .collect();
        f.render_stateful_widget(
            List::new(items).highlight_style(Style::new().bg(Color::Rgb(49, 50, 68)).add_modifier(Modifier::BOLD)).highlight_symbol("› "),
            list_area,
            &mut self.state,
        );

        let line1 = match &self.typing {
            Some(buf) => Line::from(vec![
                Span::styled("path: ", Style::new().fg(ACCENT)),
                Span::raw(format!("{buf}▏")),
                Span::styled("   Tab complete · Enter add · Esc back", Style::new().fg(DIM)),
            ]),
            None => Line::from(Span::styled(
                format!(
                    "↑↓ move · →/Enter open · ← up · space pick · a add · r run last · / type a path · Esc cancel · n = search the new tracks after adding: {}",
                    if self.search_new { "on" } else { "off" }
                ),
                Style::new().fg(DIM),
            )),
        };
        let line2 = Line::from(Span::styled(self.note.clone(), Style::new().fg(Color::Rgb(243, 139, 168))));
        f.render_widget(Paragraph::new(vec![line1, line2]).wrap(ratatui::widgets::Wrap { trim: true }), foot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_completes_to_the_common_prefix() {
        let base = std::env::temp_dir().join(format!("lx-pick-{}", std::process::id()));
        std::fs::create_dir_all(base.join("Soulseek")).unwrap();
        std::fs::create_dir_all(base.join("Spotiflac")).unwrap();
        std::fs::create_dir_all(base.join("YouTube")).unwrap();
        let b = base.display();
        assert_eq!(complete(&format!("{b}/Y")), format!("{b}/YouTube/"));
        assert_eq!(complete(&format!("{b}/S")), format!("{b}/S"));
        assert_eq!(complete(&format!("{b}/So")), format!("{b}/Soulseek/"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
