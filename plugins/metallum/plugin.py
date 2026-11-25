#!/usr/bin/env python3
"""lyrixearch plugin: Metal Archives lyrics through the shared `metallum` CLI.

Protocol v1 (see docs/PLUGINS.md): one JSON query on stdin, one JSON reply on stdout.
The CLI is album-level (`metallum lyrics <album> --artist <artist> --json`), so results are
cached per album for a few minutes to avoid re-running it for every track of the album.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

CACHE_TTL = 600  # seconds
RATE_LIMIT_WAIT = 60  # seconds to ask lyrixearch to wait when Metal Archives says "slow down"


def find_metallum() -> str:
    env = os.environ.get("METALLUM_BIN", "")
    if env:
        return str(Path(env).expanduser())
    found = shutil.which("metallum")
    if found:
        return found
    for cand in (Path.home() / ".local/bin/metallum",):
        if cand.is_file() and os.access(cand, os.X_OK):
            return str(cand)
    return ""


def norm(s: str) -> str:
    return re.sub(r"[^a-z0-9]+", "", (s or "").casefold())


def clean_lyrics(text: str) -> str:
    """Strip LRC stamps, keep stanza breaks, collapse doubled blank lines, trim the ends."""
    text = re.sub(r"\[\d{1,2}:\d{2}(?:\.\d{1,3})?\]", "", text)
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    out, prev_blank = [], False
    for raw in text.split("\n"):
        line = raw.strip()
        if not line:
            if not prev_blank:
                out.append("")
            prev_blank = True
            continue
        out.append(line)
        prev_blank = False
    while out and out[0] == "":
        out.pop(0)
    while out and out[-1] == "":
        out.pop()
    return "\n".join(out)


def _locked(path: Path):
    """Exclusive lock so parallel tracks of one album share a single metallum run (they then hit the cache)."""
    handle = open(path, "w")
    try:
        import fcntl
        fcntl.flock(handle, fcntl.LOCK_EX)
    except (ImportError, OSError):  # no fcntl (Windows): carry on without single-flight
        pass
    return handle


def album_result(binary: str, artist: str, album: str) -> dict:
    cache_dir = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "lyrixearch" / "metallum"
    key = hashlib.sha1(f"{artist}|{album}".casefold().encode()).hexdigest()
    cache = cache_dir / f"{key}.json"
    def cached():
        try:
            if cache.is_file() and time.time() - cache.stat().st_mtime < CACHE_TTL:
                return json.loads(cache.read_text())
        except (OSError, ValueError):
            pass
        return None

    hit = cached()
    if hit is not None:
        return hit
    try:
        cache_dir.mkdir(parents=True, exist_ok=True)
        lock = _locked(cache_dir / f"{key}.lock")
    except OSError:
        lock = None
    hit = cached()  # another track of this album may have finished while we waited for the lock
    if hit is not None:
        return hit
    env = {k: v for k, v in os.environ.items() if not k.startswith("MallocStackLogging") and k != "MallocNanoZone"}
    done = subprocess.run([binary, "lyrics", album, "--artist", artist, "--json"],
                          capture_output=True, text=True, env=env, check=False)
    if done.returncode != 0:
        # the CLI reports "ambiguous" / "not found" as a JSON error object: that is a miss, not a failure
        for blob in (done.stdout, done.stderr):
            try:
                err = json.loads(blob)
            except ValueError:
                continue
            if isinstance(err, dict):
                return err
        text = (done.stderr or done.stdout or "").strip()
        raise RuntimeError(f"metallum exited {done.returncode}: {text[-200:] if text else 'no output'}")
    value = json.loads(done.stdout)
    if not isinstance(value, dict):
        raise RuntimeError("metallum returned non-object JSON")
    try:
        cache_dir.mkdir(parents=True, exist_ok=True)
        cache.write_text(json.dumps(value))
    except OSError:
        pass
    return value


def pick_track(tracks: list, number, title: str):
    def pos(row):
        m = re.search(r"\d+", str(row.get("position") or ""))
        return int(m.group(0)) if m else 0

    if number:
        for row in tracks:
            if pos(row) == number:
                return row
    key = norm(title)
    if key:
        for row in tracks:
            if norm(str(row.get("title", ""))) == key:
                return row
    return None


def error_reply(result: dict) -> dict:
    """Map a metallum CLI error object to a protocol reply.

    "No such album" / ambiguous answers are a miss. A rate limit (or any error the CLI marks
    retryable) asks lyrixearch to back off; anything else is reported as a failure so it is seen.
    """
    error = str(result.get("error") or "unknown error")
    if error == "rate_limited" or result.get("retryable"):
        return {"found": False, "error": f"Metal Archives: {error}", "retry_after": RATE_LIMIT_WAIT}
    if result.get("error_kind") == "resolution" or error in ("not_found", "ambiguous"):
        return {"found": False}
    return {"found": False, "error": f"Metal Archives: {error}"}


def main() -> int:
    q = json.load(sys.stdin)
    artist = (q.get("album_artist") or q.get("artist") or "").strip()
    album = (q.get("album") or "").strip()
    if not artist or not album:
        print(json.dumps({"found": False}))
        return 0
    binary = find_metallum()
    if not binary:
        print("metallum executable not found (set METALLUM_BIN)", file=sys.stderr)
        return 1
    result = album_result(binary, artist, album)
    if not result.get("ok"):
        print(json.dumps(error_reply(result)))
        return 0
    tracks = [t for t in result.get("tracks", []) if isinstance(t, dict)]
    row = pick_track(tracks, q.get("track_number"), q.get("title", ""))
    lyrics = row.get("lyrics") if row else None
    if not row or not row.get("lyrics_available") or not isinstance(lyrics, str) or not lyrics.strip():
        print(json.dumps({"found": False}))
        return 0
    print(json.dumps({"found": True, "plain": clean_lyrics(lyrics), "artist": artist,
                      "title": row.get("title") or q.get("title"), "score": 1.0}))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # report as a plugin failure with a readable last line
        print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1)
