//! Track playback in the backend, so every format the decoder supports plays the same way on
//! macOS, Windows and Linux (a webview `<audio>` tag cannot do Ogg/Opus everywhere).
//!
//! One thread owns the audio device (it must not move between threads) and is driven by
//! [`Cmd`] messages. It reports state back as `player` events about four times a second.

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player as Sink, Source};
use serde::Serialize;
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub enum Cmd {
    /// Load a track. `hint` is the duration from the tags, used when the decoder cannot tell.
    Load {
        id: u64,
        path: PathBuf,
        play: bool,
        hint: Option<f64>,
    },
    Toggle,
    Seek(f64),
    Stop,
}

#[derive(Serialize, Clone)]
pub struct PlayerEvent {
    pub id: Option<u64>,
    /// `playing`, `paused`, `stopped`, `ended` or `error`.
    pub state: &'static str,
    pub pos: f64,
    pub duration: Option<f64>,
    pub error: Option<String>,
}

pub struct Player {
    tx: Sender<Cmd>,
}

impl Player {
    pub fn start(app: AppHandle) -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("audio".into()).spawn(move || run(app, rx)).expect("spawn audio thread");
        Self { tx }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

struct Current {
    id: u64,
    path: PathBuf,
    duration: Option<f64>,
    sink: Sink,
    ended: bool,
}

fn run(app: AppHandle, rx: Receiver<Cmd>) {
    // Opened on first use: a machine without an audio device only fails when you press play.
    let mut device: Option<MixerDeviceSink> = None;
    let mut cur: Option<Current> = None;

    let emit = |id: Option<u64>, state: &'static str, pos: f64, duration: Option<f64>, error: Option<String>| {
        let _ = app.emit("player", PlayerEvent { id, state, pos, duration, error });
    };

    let load = |device: &mut Option<MixerDeviceSink>, id: u64, path: &PathBuf, play: bool, hint: Option<f64>| -> Result<Current, String> {
        if device.is_none() {
            *device = Some(DeviceSinkBuilder::open_default_sink().map_err(|e| format!("no audio output available: {e}"))?);
        }
        let file = File::open(path).map_err(|e| format!("cannot open the file: {e}"))?;
        let source = Decoder::try_from(file).map_err(|e| format!("this format cannot be played: {e}"))?;
        let duration = source.total_duration().map(|d| d.as_secs_f64()).or(hint);
        let sink = Sink::connect_new(device.as_ref().expect("opened above").mixer());
        sink.append(source);
        if !play {
            sink.pause();
        }
        Ok(Current { id, path: path.clone(), duration, sink, ended: false })
    };

    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Cmd::Load { id, path, play, hint }) => {
                if let Some(old) = cur.take() {
                    old.sink.stop();
                }
                match load(&mut device, id, &path, play, hint) {
                    Ok(c) => {
                        emit(Some(id), if play { "playing" } else { "paused" }, 0.0, c.duration, None);
                        cur = Some(c);
                    }
                    Err(e) => emit(Some(id), "error", 0.0, hint, Some(e)),
                }
            }
            Ok(Cmd::Toggle) => {
                if let Some(c) = cur.as_mut() {
                    if c.ended {
                        // finished earlier: play it again from the start
                        let (id, path, dur) = (c.id, c.path.clone(), c.duration);
                        cur = None;
                        match load(&mut device, id, &path, true, dur) {
                            Ok(n) => {
                                emit(Some(id), "playing", 0.0, n.duration, None);
                                cur = Some(n);
                            }
                            Err(e) => emit(Some(id), "error", 0.0, dur, Some(e)),
                        }
                    } else if c.sink.is_paused() {
                        c.sink.play();
                        emit(Some(c.id), "playing", c.sink.get_pos().as_secs_f64(), c.duration, None);
                    } else {
                        c.sink.pause();
                        emit(Some(c.id), "paused", c.sink.get_pos().as_secs_f64(), c.duration, None);
                    }
                }
            }
            Ok(Cmd::Seek(secs)) => {
                if let Some(c) = cur.as_mut() {
                    let target = secs.max(0.0);
                    match c.sink.try_seek(Duration::from_secs_f64(target)) {
                        Ok(()) => {
                            c.ended = false;
                            let state = if c.sink.is_paused() { "paused" } else { "playing" };
                            emit(Some(c.id), state, target, c.duration, None);
                        }
                        Err(e) => emit(Some(c.id), "error", c.sink.get_pos().as_secs_f64(), c.duration, Some(format!("cannot seek in this file: {e}"))),
                    }
                }
            }
            Ok(Cmd::Stop) => {
                if let Some(c) = cur.take() {
                    c.sink.stop();
                    emit(Some(c.id), "stopped", 0.0, c.duration, None);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(c) = cur.as_mut() {
                    if !c.ended && !c.sink.is_paused() {
                        if c.sink.empty() {
                            c.ended = true;
                            emit(Some(c.id), "ended", c.duration.unwrap_or(0.0), c.duration, None);
                        } else {
                            emit(Some(c.id), "playing", c.sink.get_pos().as_secs_f64(), c.duration, None);
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}
