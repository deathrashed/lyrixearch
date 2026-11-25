# Plugins

A plugin is any executable that acts as an extra lyrics source. Lyrixearch starts it once per
track, sends the track details as JSON on **stdin**, and reads one JSON object from **stdout**.

## Install

```sh
lyrixearch plugins install plugins/metallum     # copies the folder and prints how to enable it
lyrixearch plugins list
lyrixearch plugins test metallum --artist "Band" --title "Song" --album "Album" --track 1
```

Plugins live in `<config dir>/lyrixearch/plugins/<name>/` (`lyrixearch plugins path` prints it;
on macOS that is `~/Library/Application Support/lyrixearch/plugins`). The GUI's
**Settings → Plugins → Install plugin folder…** does the same and enables the plugin.

A plugin is only used when its `id` is in the `providers` list (Settings → Sources, or
`lyrixearch config set providers a,b,c`). Order matters: the first source that returns lyrics wins.

## Manifest — `plugin.toml`

```toml
id = "metallum"                      # unique; used in `providers`
name = "Metal Archives"              # shown in the GUI
command = ["python3", "plugin.py"]   # run with the plugin folder as working directory
timeout_secs = 60                    # optional, default 30
max_parallel = 1                     # optional, default 2: lookups running at once
min_gap_ms = 300                     # optional, default 300: pause between starting two lookups
```

## Protocol v1

stdin (one JSON object):

```json
{"protocol":1,"artist":"…","title":"…","album":"…","album_artist":"…",
 "track_number":3,"duration_secs":241,"path":"/music/…/03 Song.flac"}
```

Fields can be `null`. `path` is the audio file, so a plugin may also look at its folder.

stdout (one JSON object; extra log lines before it are ignored):

```json
{"found":true,"plain":"…","synced":"[00:01.00]…","artist":"…","title":"…","url":"…","score":0.95}
```

or `{"found":false}` when the source simply has nothing. `plain` and/or `synced` must be present
when `found` is true. `score` (0–1) is optional and defaults to 0.9.

Problems on the plugin's side can be reported in the reply as well (both fields are optional;
plugins that never send them keep working):

```json
{"found":false,"error":"service unavailable"}
{"found":false,"error":"rate limited","retry_after":120}
```

`error` alone is a **failure**. With `retry_after` (seconds, held between 15 and 900) it means
"rate limited": the plugin is left alone for that long instead of collecting failures.

Exit status: `0` for a normal answer (including "not found"). Any non-zero exit is a
**failure** — the last line of stderr is shown, and repeated failures put the plugin in the
penalty box for a few minutes, like any other source.

## The bundled `metallum` plugin

`plugins/metallum` wraps the shared `metallum` CLI (`metallum lyrics <album> --artist <artist> --json`).
That CLI answers per album, so the plugin caches each album's reply for ten minutes and picks the
track by track number first, then by normalised title. "No such album" and ambiguous answers are
misses; a Metal Archives rate limit asks lyrixearch to wait a minute; other CLI errors are failures. It looks for the executable in
`$METALLUM_BIN`, then `PATH`, then `~/.local/bin/metallum`.
