# sqldev — Clipchamp demo recording script

A recording plan for a ~2-minute "what is sqldev?" demo, optimized for
[Clipchamp](https://app.clipchamp.com) (or any timeline editor). The
target audience is a SQL Server developer scrolling LinkedIn / Twitter
who has never heard of sqldev.

The script has three parallel tracks per scene:

- **🎬 Visual** — what's on screen.
- **🗣 VO** — what the narrator says (read once at a comfortable pace —
  each scene's VO is timed to fit the scene length).
- **⌨️ Commands** — exact terminal input. Every command is also in
  [`scripts/demo.sh`](../scripts/demo.sh) so you can drive the recording
  with [`asciinema`](https://asciinema.org) or `script(1)` and overlay it
  onto the video.

Total runtime target: **02:00**. Cut hard at 02:10 — TikTok / LinkedIn
truncate longer clips.

> **Before you record:** finish the **Pre-flight checklist** at the
> bottom of this file. Skipping it is how you end up with a redacted
> password on screen at frame 1247.

---

## Scene 1 — Hook (0:00 – 0:08, 8s)

- **🎬 Visual:** Black screen → fade in to the sqldev wordmark on a
  dark terminal background. Subtle typewriter cursor. Fade out the
  wordmark, fade in to a clean zsh prompt in `~/playground/sqldev-demo`.
- **🗣 VO:** *"If you've ever wished `sqlcmd` knew it was 2026, this is
  for you. Meet `sqldev` — a developer-friendly SQL Server CLI."*
- **⌨️ Commands:** *(none — title card only)*
- **On-screen text (lower third):** `sqldev — sqlcmd for humans, with migrations.`

## Scene 2 — Install (0:08 – 0:22, 14s)

- **🎬 Visual:** Terminal cuts to a fresh prompt. Run two commands.
  Highlight the version output with a yellow rectangle annotation
  (Clipchamp ▸ Shapes ▸ Outline rectangle).
- **🗣 VO:** *"It's a single static binary. Build from source today —
  pre-built tarballs land with the first GitHub Release."*
- **⌨️ Commands:**
  ```bash
  cargo build --release  # already cached for the demo, takes <2s
  sqldev --version
  ```
- **On-screen text (chip, top-right):** `v0.1.0 · pre-release`

## Scene 3 — `.sqldev.yml` (0:22 – 0:42, 20s)

- **🎬 Visual:** Split-screen.
  - Left pane: `bat .sqldev.yml` showing the dev + prod env blocks.
  - Right pane: `sqldev config show` output with `***REDACTED***`
    highlighted in green.
- **🗣 VO:** *"Drop a `.sqldev.yml` in your repo. Env blocks for dev,
  staging, prod. Secrets come from environment variables, never the
  file. `sqldev config show` always redacts them, so you can paste
  the output into a bug report."*
- **⌨️ Commands:**
  ```bash
  bat --plain .sqldev.yml
  sqldev config show
  ```
- **On-screen callout:** an arrow pointing at `***REDACTED***` →
  *"never written to disk"*.

## Scene 4 — `sqldev query` (0:42 – 1:05, 23s)

- **🎬 Visual:** Single full-screen terminal. Type each command at a
  natural pace. After the JSON command, pipe to `jq`.
- **🗣 VO:** *"Querying is one flag. Tab-separated by default — pipe-
  friendly. Add `--format json` and you get one row per object,
  ready for `jq`. Need to read SQL from a script? Pipe it on stdin."*
- **⌨️ Commands:**
  ```bash
  sqldev query --sql 'SELECT TOP 3 name FROM sys.tables ORDER BY name'

  sqldev query \
    --sql 'SELECT TOP 3 name, type_desc FROM sys.objects ORDER BY name' \
    --format json | jq

  echo 'SELECT @@VERSION AS v;' | sqldev query --format json | jq -r '.[0].v'
  ```
- **On-screen text (sticky, bottom):** `--format json | jq  ←  the killer feature`

## Scene 5 — `sqldev introspect` (1:05 – 1:30, 25s)

- **🎬 Visual:** Run `sqldev introspect > schema.json`. Cut to three
  `jq` queries that count schemas / tables / foreign keys, with each
  number animating in (Clipchamp ▸ Text ▸ Counter effect).
- **🗣 VO:** *"`sqldev introspect` walks the system catalog and emits
  the entire schema as one JSON document. Tables, columns, indexes,
  constraints, UDDTs, views, procs — six schemas, seventy-one tables,
  ninety foreign keys, all in one query. This is the contract that
  `diff`, `migrate`, and codegen are built on."*
- **⌨️ Commands:**
  ```bash
  sqldev introspect > schema.json
  jq '.schemas | length' schema.json
  jq '[.schemas[].tables[]] | length' schema.json
  jq '[.schemas[].tables[].foreign_keys[]?] | length' schema.json
  ```
- **On-screen overlay:** a stylized graph of the schema-graph JSON
  shape (use the `assets/schema-graph.png` asset; see Pre-flight).

## Scene 6 — Safety (1:30 – 1:48, 18s)

- **🎬 Visual:** Single terminal. Run the protected-env command. The
  red `Error:` line should be visible for at least 4 seconds.
- **🗣 VO:** *"Mark an env `protected: true` and `sqldev` refuses to
  trust the server cert. Future-proof: when `migrate` lands in M1.7,
  the same flag will require an explicit `--confirm prod` before any
  destructive change."*
- **⌨️ Commands:**
  ```bash
  sqldev --env prod query --sql 'SELECT 1' --trust-cert true
  ```
- **On-screen text (large, centered):** `protected: true · fail closed`

## Scene 7 — Roadmap + CTA (1:48 – 2:00, 12s)

- **🎬 Visual:** Cut from terminal to a card with three rows:
  - ✅ v0.1 — query, introspect, config (today)
  - 🛠 v0.2 — diff, explain, seed, codegen
  - 🚀 v1.0 — plugins, ecosystem
  Beneath: GitHub URL.
- **🗣 VO:** *"v0.1 is the walking skeleton. The interesting features —
  `diff`, `explain`, plugins — are next. Star the repo, file issues,
  and try the spike crates if you can't wait. Link in the description."*
- **⌨️ Commands:** *(none — outro card)*
- **On-screen text (CTA, centered):**
  `github.com/saurabh500/sqldev` ✦ `★ if you'd use this`

---

## Pre-flight checklist (do this before hitting record)

1. **Disable shell history sharing** in the recording shell:
   `unset HISTFILE` in zsh. Otherwise old commands bleed into the demo.
2. **Hide your prompt's git status** for cleaner frames. Either
   `cd ~/playground/sqldev-demo` (not in a repo) or temporarily set
   `PROMPT='%n@demo %1~ %# '`.
3. **Run every command once cold** so caches are warm. `cargo build --release`
   should already be done; `sqldev introspect` should already have been
   run once against the same DB so OS-level page caches are hot.
4. **Set the terminal font size** to 18pt or larger. 1080p video crops
   tiny text into mush.
5. **Use a 16:9 terminal window** at exactly 1920×1080 (or the
   Clipchamp default). 80 columns × 24 rows is a good fit.
6. **Theme:** light or dark, pick one — but no transparent backgrounds.
   `Solarized Dark` and `One Dark` both look great on camera.
7. **Audio:** record VO separately in Audacity / Clipchamp's built-in
   recorder. Don't try to nail timing while typing.
8. **Capture:** for terminal, use `script -q /tmp/demo.log -c 'zsh'`
   *or* OBS recording the terminal window. Avoid full-desktop capture —
   it picks up your menu bar.
9. **Clear the directory state** between takes:
   `rm -f schema.json; history -c`.
10. **Check the password leak risk:** before recording Scene 3, confirm
    `echo $DEV_SA_PASSWORD` is **not** in your shell's last command and
    that `bat .sqldev.yml` shows the `${VAR}` syntax, not the resolved
    value (it should — `sqldev` interpolates *after* read).

## Asset list

Drop these in `docs/assets/` (create the folder; nothing committed yet):

| File | Purpose | Source |
|---|---|---|
| `wordmark.png` | Title card. | Make in Figma; export 1920×1080 with transparent BG. |
| `schema-graph.png` | Scene 5 overlay. | Render with `mermaid-cli` from a small graph diagram. |
| `vo-master.wav` | Final voiceover. | Record in Audacity, normalize to -3 dBFS. |
| `bg-music.mp3` | Bed under VO. | Use any royalty-free track ≤ -22 LUFS. |

## Clipchamp timeline

```
Track 1 (video):   [Scene 1][Scene 2][Scene 3][Scene 4][Scene 5][Scene 6][Scene 7]
Track 2 (overlay):                  [callout][          ][graph][            ][cta]
Track 3 (text):    [title]   [chip] [labels] [labels  ] [labels] [warning] [cta]
Track 4 (VO):      ──────────────────────────── master VO ────────────────────────
Track 5 (music):   ──────────── bed at -22 LUFS, ducks under VO ──────────────────
```

Export at **1080p, 30fps, MP4 H.264** for LinkedIn / Twitter.
For YouTube Shorts, re-export 1080×1920 portrait by recropping the
terminal — or re-record with a portrait-shaped window.
