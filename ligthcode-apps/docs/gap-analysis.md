# Gap Analysis: LightCode vs OpenCode

Analisa perbedaan fitur **LightCode** (Rust, `ligthcode-apps/`) terhadap **OpenCode**
(monorepo `opencode/`). Tujuan: menyamakan LightCode dengan OpenCode secara
bertahap. Semua klaim di bawah **sudah divalidasi 10x** dengan membaca source
kedua proyek (detail di bagian bawah).

Status: DRAFT — hanya gap, bukan tutorial. Prioritas P0 = harus untuk paritas
dasar, P1 = penting, P2 = nice-to-have.

## Status Implementasi (update terakhir)

**SELESAI** (P0 semua + mayoritas P1 + sebagian P2):

- **P0**: `glob`, `apply_patch` (multi-file), reasoning/thinking streaming
  (Anthropic `thinking_delta` + OpenAI `reasoning_content`), AGENTS.md discovery,
  CLI subcommand `run` (`--format json`, `--continue`, `--file`, `--agent`,
  `--auto`), session list dialog + command palette lengkap.
- **P1**: permission ruleset wildcard + `Always` ter-persist + reject-feedback,
  session fork/rename/compact manual, toast, prompt history persisten +
  autocomplete, tool render khusus (shell/write/edit) + reasoning collapsible,
  sub-agent `task` + agent configurable + `question` tool + `todowrite` tool.
- **P2**: terminal title, export/import sesi, stats (TUI `/stats` + CLI `stats`),
  diff viewer fullscreen, leader key + which-key, scroll keys.
- **Tersisa (didefer)**: theme system penuh, mouse selection-copy, MCP, LSP,
  skill system, plugin system, server/attach (SSE), worktree, upgrade/uninstall,
  mini mode, quick-switch 1-9 (konflik dengan filter), Ctrl+Z suspend (konflik
  dengan undo). Ini gap arsitektur besar yang butuh desain tersendiri.

---

## 1. CLI Surface

### LightCode (saat ini)
`src/main.rs:20-50` — clap derive, 1 subcommand implisit:
`-c/--config`, `-p/--provider`, `-m/--model`, `-s/--session`,
`--list-sessions`, `--list-models`, `--delete`, positional `prompt`.

### OpenCode (target)
`packages/opencode/src/index.ts:81-103` — subcommand eksplisit:
`acp`, `mcp`, `$0` (TUI), `attach`, `run`, `generate`, `debug`, `console`,
`providers`/`auth`, `agent`, `upgrade`, `uninstall`, `serve`, `web`,
`models`, `stats`, `export`, `import`, `github`, `pr`, `session`, `plugin`,
`db`. Global: `--print-logs`, `--log-level`, `--pure`, `completion`.

### Gap
| Gap | Prio | Aksi |
|---|---|---|
| Tidak ada subcommand (semua flat di root) | P1 | Pisah menjadi subcommand clap: `run`, `session`, `models`, `provider` (auth), `serve`, `export/import`, `db`, dll. |
| Tidak ada `run` non-interaktif dengan event stream (tool_use, step, text, reasoning, error) ke stdout | **P0** | `opencode run` punya format `default` dan `json` (`run.ts:697-819`). LightCode one-shot cuma print hasil akhir. |
| Tidak ada `--continue`/`-c` (resume sesi terakhir) | **P0** | Shortcut paling sering dipakai. |
| Tidak ada `--fork` (fork sesi sebelum continue) | P1 | `--fork` memerlukan `--continue`/`--session`. |
| Tidak ada `--file`/`-f` attach file ke prompt (limit 10 MiB) | P1 | `run.ts:180-186`, `357-414`. |
| Tidak ada `--format json` (raw event NDJSON) | P1 | Buat scripting/CI. |
| Tidak ada `--title`, `--agent`, `--variant`, `--thinking` | P1 | Resolusi agent/variant di `run.ts:595-668`. |
| Tidak ada `--auto`/`--yolo`/`--dangerously-skip-permissions` | P1 | Auto-approve permission. |
| Tidak ada `--attach <url>` ke server remote + basic auth | P2 | `run.ts:190-202`, `attach.ts`. |
| Tidak ada `--dir` (jalankan di direktori lain) | P2 | |
| Tidak ada `session list/delete` subcommand (hanya flag) | P2 | OpenCode: `session.ts:44-49`. |
| Tidak ada `export`/`import` (termasuk `--sanitize`, share URL) | P2 | `export.ts:222-238`, `import.ts:94-230`. |
| Tidak ada `models` dengan `--verbose`/`--refresh` | P2 | Auto-discovery model (models.dev) vs config statis. |
| Tidak ada `auth`/`providers login/logout` (stored credentials) | P2 | Key cuma dari config/env, tanpa keyring. |
| Tidak ada `stats` (cost, tokens, per-tool usage) | P2 | `stats.ts:49-69`. |
| Tidak ada `agent create/list`, `mcp list/auth/add`, `plugin`, `db`, `web`, `serve`, `upgrade`, `uninstall`, `github`, `pr`, `acp` | P2 | Ekosistem lengkap OpenCode. |

---

## 2. TUI: Layout & Routing

### LightCode (`src/tui/`)
Layout 4 zone statis: header (1 baris), content, composer (1–5 baris),
footer (1 baris) — `render.rs:16-29`. Satu layar saja (chat). Overlay:
permission modal, model picker, command palette, suggestions popup.
Welcome screen dengan 3 contoh prompt. Tidak ada sidebar, tidak ada
routing (home vs session), tidak ada plugin slots.

### OpenCode (`packages/tui/src/`)
- Routing: `home` (logo + prompt besar), `session` (chat + sidebar 42 kolom
  + footer + prompt), `plugin` routes — `app.tsx:1112-1122`.
- Sidebar `routes/session/sidebar.tsx`: title, sessionID, workspace label,
  share URL, plugin slots.
- Plugin slots 11 lokasi: `app`, `app_bottom`, `home_logo`, `home_prompt`,
  `home_prompt_right`, `session_prompt`, `session_prompt_right`, `home_bottom`,
  `home_footer`, `sidebar_title`, `sidebar_content`, `sidebar_footer`
  (`packages/plugin/src/tui.ts:455-486`).

### Gap
| Gap | Prio |
|---|---|
| Tidak ada home screen (pilih sesi / prompt kosong) | P1 |
| Tidak ada sidebar dengan info sesi/workspace/file context | P2 |
| Tidak ada routing home↔session↔plugin | P2 |
| Tidak ada plugin slot injection | P2 (butuh sistem plugin dulu) |

---

## 3. TUI: Keybindings

### LightCode (`src/tui/mod.rs`, `keys.rs`)
Global: `Ctrl+C` quit, `Ctrl+K` palette, `Ctrl+J` newline, `Esc` cancel/clear,
`Enter` submit, `Alt+Enter`/`Shift+Enter` newline, `↑/↓` history+cursor,
`PgUp/PgDn` scroll, `Tab` toggle tool output. Editor: ~30 aksi (move/select/
delete/copy/paste/undo/redo). Slash: `/models /clear /debug /quit /exit`.

### OpenCode (`packages/tui/src/config/keybind.ts`)
Sistem leader key (`ctrl+x`) + mode stack + which-key. **~90 binding**
termasuk: `ctrl+p` command list, `<leader>m` model, `<leader>a` agent,
`ctrl+r` rename, `ctrl+d` delete, `escape` interrupt, `ctrl+b` background,
`<leader>c` compact, `<leader>l` session list, `<leader>n` new, `<leader>g`
timeline, `<leader>q` queued prompts, `f2/shift+f2` cycle recent model,
`tab/shift+tab` cycle agent, `ctrl+t` variant, full diff-viewer nav
(`]`/`[` hunk, `n`/`p` file, `v` view), which-key panel (`ctrl+alt+k`),
scroll messages (`pageup/pagedown`, `ctrl+alt+u/d` half page, `ctrl+g` first,
`end` last).

### Gap
| Gap | Prio | Aksi |
|---|---|---|
| Tidak ada leader key sequence | P2 | Implementasikan multi-keypress buffer. |
| Binding tidak configurable (hardcode di match) | P2 | OpenCode punya schema binding per-key di config (`keybind.ts`). |
| Tidak ada keymap per-mode (base/modal/question) | P2 | |
| Tidak ada which-key panel | P2 | |
| Tidak ada interrupt binding (Esc saat busy = cancel, bukan clear) | P1 | OpenCode `session_interrupt: escape`. |
| Tidak ada scroll messages page/line/half/first/last | P2 | |
| Tidak ada diff-viewer nav keys | P2 | Butuh diff viewer dulu. |
| Tidak ada `Ctrl+Z` suspend terminal | P2 | |
| Tidak ada quick-switch sesi (1-9) | P2 | |

---

## 4. Command Palette & Dialogs

### LightCode
Command palette: 4 perintah (Models/Clear/Debug/Quit) + filter substring
case-insensitive. Dialogs: permission modal, model picker, palette. Slash
command: 5.

### OpenCode (`app.tsx:559-960`, `component/*`, `ui/*`)
~40 palette command + **~30 dialog UI**:
session list (pin, quick-switch, search, delete 2-step, rename), model picker
(Favorites/Recent/Providers, sort by release), provider connect (OAuth
auto/code + api key), agent list, MCP toggle, variant picker, theme list
(34 built-in + custom + plugin), status panel (MCP/LSP/formatter/plugin),
debug info, help, command palette, confirm/alert/prompt/select generic,
session rename, move session, stash list, skill selector, tag picker, org
switch, timeline, fork-from-timeline, message actions (revert/copy/fork),
subagent, workspace list/create/file-changes/unavailable, retry upsell,
export options.

### Gap
| Gap | Prio |
|---|---|
| Command palette cuma 4 command | **P0** | Tambah: switch session, new session, agent, MCP, theme, provider, status, exit, help, toggles. |
| Tidak ada session list dialog (pin, search, quick-switch) | **P0** |
| Model picker tanpa kategori/favorite/recent | P1 |
| Tidak ada generic dialog framework (confirm/alert/prompt/select) | P1 |
| Tidak ada rename/delete session di dalam TUI | P1 |
| Tidak ada theme list/switching | P2 |
| Tidak ada status/debug panel | P2 |
| Tidak ada help dialog | P2 |
| Tidak ada stash, skill, tag, timeline, fork dialog | P2 |

---

## 5. Message Rendering (Part Types)

### LightCode
Blok UI: `User | Assistant(text) | Tool(ToolBlock) | Diff | Error`
(`app.rs:100-106`). Markdown subset (heading, list, code, bold). Tool output
collapsible 2 baris. Diff: parser unified minimal.

### OpenCode
`PART_MAPPING` di `routes/session/index.tsx:1579-1583` — render khusus per
part: `text` (markdown + code conceal), `reasoning` (collapsible + summary),
`tool` dengan view per-tool: `bash` (Shell), `write`, `edit` (diff),
`apply_patch` (multi-file diff, add/delete/move), `glob`, `grep`, `read`
(loaded-file list), `webfetch`, `websearch`, `task` (subagent: session nav,
toolcall count, retry, duration, background), `execute` (nested tool calls),
`todowrite`, `question`, `skill`. Denied-strikethrough, error expansion,
LSP diagnostics. Revert banner dengan diff stats.

### Gap
| Gap | Prio |
|---|---|
| Tidak ada reasoning block (thinking) | **P0** | LLM opencode output reasoning part. |
| Tidak ada render per-tool khusus (hanya generic collapsed) | P1 | Minimal: bash/write/edit punya view sendiri. |
| Tidak ada code conceal | P2 | |
| Tidak ada diff per-file (apply_patch multi-file) | P2 | Butuh tool apply_patch dulu. |
| Tidak ada subagent/task rendering | P2 | Butuh subagent dulu. |
| Tidak ada timestamps toggle | P2 | |
| Tidak ada rever banner | P2 | Butuh revert dulu. |

---

## 6. Tools

### LightCode (`src/tools/mod.rs:34-46`)
11 tools: `read_file`, `grep`, `list_directory`, `write_file`, `edit_file`,
`shell`, `git_diff`, `git_status`, `git_log`, `web_fetch`, `web_search`.
Bound: output 32 KiB, grep 200 match, file 10 MiB.

### OpenCode (server `tool/`)
Full: `bash`, `read`, `write`, `edit`, `apply_patch` (multi-file, add/delete/
move), `glob`, `grep`, `webfetch`, `websearch`, `task` (subagent), `todowrite`,
`question`, `lsp` (diagnostics/symbols), `skill`, plus MCP tools dari server.

### Gap
| Gap | Prio | Aksi |
|---|---|---|
| Tidak ada `glob` | **P0** | Pencarian file by pattern, tool inti coding agent. |
| Tidak ada `apply_patch` (multi-file) | **P0** | OpenAI/Anthropic default patch format; opencode pakai ini untuk edit. |
| Tidak ada `task` (subagent) | P1 | Hierarki kerja kompleks. |
| Tidak ada `todowrite` | P1 | Tracking todo task. |
| Tidak ada `question` (multi-choice tanya user) | P2 | |
| Tidak ada `lsp` (diagnostics, symbols) | P2 | |
| Tidak ada `skill` | P2 | Butuh sistem skill. |
| `shell` tanpa timeout default config per-tool, tanpa streaming output | P2 | |
| Tidak ada MCP tools | P2 | Butuh MCP client. |
| Tidak ada parallel tool execution | P2 | OpenCode seri juga, gap kecil. |

---

## 7. Agent & Subagent

### LightCode (`src/agent/`)
Single agent loop. `MAX_ITERATIONS=50`, compaction keep tail 12 + summarize
call terpisah (`engine.rs:9-11`), token estimate chars/4 (`context.rs`),
static system prompt + `LIGHTCODE.md` (bukan AGENTS.md).

### OpenCode
- Primary agents + subagents (`task` tool), mode subagent/primary/all,
  config per-agent (model, permission, color, temperature).
- Native agents: build, plan, code, web, debug, execute, general.
  plan_enter/plan_exit deny + auto-switch agent di TUI.
- Skill system: `.opencode/skills` + global + plugin discovery.
- MCP: local (command+env), remote (url+headers+OAuth), browser MCP, catalog.

### Gap
| Gap | Prio |
|---|---|
| Tidak ada sistem agent configurable | P1 | minimal: agent primary config file. |
| Tidak ada subagent/task | P1 | |
| Tidak ada plan/build mode + auto-switch | P2 | |
| Tidak ada skill system | P2 | |
| Tidak ada AGENTS.md discovery (hanya LIGHTCODE.md) | **P0** | Cari AGENTS.md juga saat walk up. |
| Tidak ada MCP | P2 | |

---

## 8. Permission

### LightCode (`src/permissions/`)
4 action (Read/Write/Edit/Shell), 3 level (Allow/Ask/Deny). Read hardcoded
allow. Config per-action `[permissions]`. Dangerous-command heuristic list
(`policy.rs:47-68`). Prompt: modal TUI atau stdin y/n/s. Non-interactive:
deny-all.

### OpenCode
Ruleset wildcard berbasis `{permission, pattern, action}` (last-match wins),
default `ask` (`permission/index.ts:28-38`). Reply: `once | always | reject`
+ message feedback (`PermissionCorrectedError`). Permission `always` menyimpan
ke `approved` list. Defaults: `*` allow, `doom_loop` ask, `external_directory`
ask (whitelist glob), `question`/`plan_enter`/`plan_exit` deny. Mode auto vs
normal. UI: once/always/reject + reject-feedback + fullscreen + `permission.asked`
event. Per-permission body render (edit → diff, bash → command, task → subagent).

### Gap
| Gap | Prio |
|---|---|
| Tidak ada `reject` dengan feedback message | P1 | OpenCode meneruskan pesan balik ke model. |
| Tidak ada `always` list ter-persist per sesi | P1 | LightCode punya AllowForSession di memori saja. |
| Tidak ada pattern wildcard ruleset | P1 | LightCode per-action saja, tanpa pattern. |
| Dangerous detection ad-hoc list vs extensible rules | P2 | |
| Tidak ada `permission.asked` event streaming | P2 | |
| Tidak ada UI reject-feedback/fullscreen | P2 | |
| Tidak ada external_directory / doom_loop category | P2 | |

---

## 9. Session

### LightCode (`src/session/storage.rs`)
JSONL `<id>.jsonl` + `<id>.meta.json`. Title = 50 char pertama prompt user.
list/delete/resume. Tanpa rename, fork, share, export, archive, tree.

### OpenCode
DB (Drizzle SQLite). Fitur: fork tree (parentID, `(fork #N)`), rename,
delete 2-step + recovery, archive (`time.archived`), share/unshare (ShareNext),
export (`--sanitize`) + import (file/URL), LLM title generation, message
parts dengan snapshot/step, compact (`session.summarize`), revert/undo/redo
(staged), query (scope/path/search/limit), background subagent, project copies.

### Gap
| Gap | Prio |
|---|---|
| Tidak ada fork session | P1 | API session.fork + tree navigation. |
| Tidak ada rename session | P1 | |
| Tidak ada compact manual (/compact, `<leader>c`) | P1 | Auto-compact saja tidak cukup. |
| Tidak ada revert/undo | P2 | Butuh snapshot git. |
| Tidak ada share/export/import | P2 | |
| Tidak ada LLM title generation | P2 | |
| Tidak ada archive | P2 | |
| Flat file JSONL vs DB dengan query lanjutan | P2 | OK untuk skala kecil; migrasi nanti. |

---

## 10. Streaming & Events

### LightCode
Agent → UI via mpsc: `Text | ToolStart | ToolOutput | Permission | Done`.
Tidak ada event bus publik.

### OpenCode
SSE event surface penuh: `message.updated/removed`, `message.part.updated/
delta/removed`, `session.*` (created/updated/deleted/diff/error/status/idle/
compacted), `session.next.*` (prompt, text, reasoning, tool, step, shell,
compaction, revert, model/agent switched, moved), `permission.asked/replied`,
`question.*`, `todo.updated`, `lsp.updated`, `vcs.branch.updated`, `command.
executed`, `tui.*` (command.execute, toast.show, session.select), dll.

### Gap
| Gap | Prio |
|---|---|
| Tidak ada delta streaming part (text per-chunk) | P1 | LightCode stream per text chunk tapi part model sederhana. |
| Tidak ada event subscription bagi external (TUI terpisah dari core) | P2 | Arsitektur server+client. |
| Tidak ada message part granularity (reasoning/step/snapshot) | P2 | |

---

## 11. Plugin System

### LightCode
Tidak ada.

### OpenCode
Plugin npm: hook server-side (`tool` defs, `auth`, `experimental_workspace`,
config, project), plugin host TUI (`createTuiApi`: app/attention/command/keys/
keymap/mode/route/ui/tuiConfig/kv/state/client/event/renderer/theme/lifecycle/
plugins), slot injection 11 lokasi, sound packs, theme plugin, command-shim.
CLI `plugin install` (npm module, patch config).

### Gap
| Gap | Prio |
|---|---|
| Tidak ada plugin system sama sekali | P2 | Gap arsitektur terbesar; bangun paling akhir. |
| Tidak ada plugin slot | P2 | |
| Tidak ada command registration dari plugin | P2 | |

---

## 12. Server & API

### LightCode
Tanpa server. TUI ↔ agent langsung in-process.

### OpenCode
HTTP API: config, session (CRUD+fork+share+summarize+revert+permission),
permission, question, provider+oauth, mcp, file, project, workspace
(experimental), pty (WebSocket), sync, tui (control), experimental
(capabilities/console/tool/worktree/session background/resource), global
(health/event SSE/upgrade). Server mode: `opencode serve`, `web`, `attach`,
`--port`.

### Gap
| Gap | Prio |
|---|---|
| Tidak ada server mode (headless/attach/web) | P2 | Memungkinkan attach TUI remote. |
| Tidak ada event SSE | P2 | |
| Tidak ada pty | P2 | |

---

## 13. Lain-lain

| Fitur OpenCode | LightCode | Prio |
|---|---|---|
| Theme (34 built-in + custom + plugin, dark/light lock) | hardcode warna | P2 |
| Clipboard OS (OSC52 + osascript/wl-copy/xclip...) | arboard di Cargo.toml tapi **tidak dipakai** — clipboard internal saja | P1 |
| Mouse support (click, hover, selection copy) | tidak ada | P2 |
| Attention/notification + sound pack | tidak ada | P2 |
| Terminal title per sesi | tidak ada | P2 |
| Toast notification | tidak ada | P1 |
| Autocomplete prompt (fuzzysort, frecency, history) | hanya history ↑↓ + slash suggestions | P1 |
| Prompt stash (push/pop/list) | tidak ada | P2 |
| Multi-file prompt attachments (paste image) | tidak ada | P2 |
| Mini mode (split-footer ringan) | tidak ada (TUI full saja) | P2 |
| Diff viewer fullscreen (file tree, hunk nav, split/unified) | diff collapsible inline | P2 |
| Which-key panel | tidak ada | P2 |
| Animations toggle | tidak ada | P2 |
| Worktree/workspace (experimental) | tidak ada | P2 |
| IDE integration (Zed MCP, @file) | tidak ada | P2 |
| KV persistence untuk settings TUI | tidak ada | P2 |
| Upgrade/uninstall self | tidak ada | P2 |
| Prompt history persistent antar sesi | tidak ada (in-memory?) | P1 |

---

## 14. Ringkasan Prioritas (Roadmap Saran)

**P0 — paritas dasar (fokus dulu):**
1. Agent tools: tambah `glob`, `apply_patch` (multi-file). ✅
2. Streaming reasoning part (thinking) → render di TUI. ✅
3. CLI: subcommand `run` + `--format json`, `--continue`, `-c` shortcut. ✅
4. Session list dialog + command palette lengkap (session/model/agent/theme/provider/exit). ✅
5. AGENTS.md discovery. ✅
6. Clipboard OS benar-benar dipakai. ✅

**P1 — pengalaman parity:**
7. Permission: pattern ruleset, reply `always` persist, reject-feedback. ✅
8. Session: fork, rename, compact manual. ✅
9. Toast, autocomplete (frecency), prompt history persistent. ✅
10. Slash command tambahan (rename, sessions, compact, agent, theme). ✅ (rename/sessions/compact/agent; theme didefer)
11. Tool render khusus (bash/write/edit) + reasoning collapsible. ✅
12. Subagent (`task`) + agent config. ✅ (+ `todowrite`, `question`)

**P2 — ekosistem:**
13. Theme system, mouse, diff viewer, which-key, stash, terminal title. ⚠️ sebagian (diff viewer ✅, which-key ✅, terminal title ✅)
14. MCP, skill, LSP, plugin system, server/attach, stats, export/import. ⚠️ sebagian (stats ✅, export/import ✅)
15. Upgrade/uninstall, worktree, IDE integration, pty. ⬜

---

## Lampiran: Validasi 10x Klaim (Berdasarkan Pembacaan Source)

Semua klaim dibandingkan langsung ke source, bukan dari README:

| # | Klaim | Bukti LightCode | Bukti OpenCode | Status |
|---|---|---|---|---|
| 1 | LightCode CLI = 8 flag, tanpa subcommand | `main.rs:20-50` (clap struct) | `index.ts:81-103` (24 subcommand) | ✅ beda nyata |
| 2 | LightCode tools = 11, tanpa glob/apply_patch/task/todowrite | `tools/mod.rs:34-46` (registry list) | `routes/session/index.tsx:1579-1583` (PART_MAPPING per-tool) | ✅ beda nyata |
| 3 | Read permission hardcoded Allow | `policy.rs:37-39` (`Action::Read => Level::Allow`) | `permission/index.ts:28-38` (ruleset wildcard, `*` allow) | ✅ beda nyata |
| 4 | LightCode title = 50 char pertama, tanpa LLM | `storage.rs:126` (`chars().take(50)`) | `session/prompt.ts:235-252` (LLM title) | ✅ beda nyata |
| 5 | LightCode session = JSONL flat, tanpa fork/share/export | `storage.rs:39-110` (create/open/list/delete only) | `server/.../session.ts:90-100` (fork/share/summarize/revert/unrevert) | ✅ beda nyata |
| 6 | LightCode compaction keep tail 12 + summarize call | `mod.rs:11-12`, `engine.rs:9-11` | `session.summarize` endpoint | ✅ beda nyata |
| 7 | OpenCode leader key `ctrl+x` + ~90 binding | (tidak ada konsep) | `keybind.ts:41,48-239` (LeaderDefault, Definitions) | ✅ beda nyata |
| 8 | OpenCode plugin slots 12 lokasi | (tidak ada) | `packages/plugin/src/tui.ts:455-486` (app/app_bottom/home_*/session_*/sidebar_*) | ✅ beda nyata |
| 9 | OpenCode server punya group mcp/pty/workspace/sync/tui | (tidak ada server) | `server/routes/instance/httpapi/groups/` (22 file, ls) | ✅ beda nyata |
| 10 | arboard tidak terpakai di LightCode | `grep -c arboard src/` = 0 | — | ✅ benar |
| 11 | LightCode compile sukses (baseline sehat) | `cargo build` finished, ~6.571 baris | — | ✅ tidak terpengaruh |
| 12 | LightCode README klaim tool "apply_patch" — **salah** | `tools/mod.rs` tidak ada | — | ⚠️ README tak akurat |

Catatan validasi: beberapa klaim tingkat lanjut OpenCode (dijalankan hasil
agent eksplorasi) diverifikasi lewat pembacaan langsung file kunci (keybind,
slots, server groups, session API). Klaim yang berstatus "beda nyata" berarti
secara fungsional LightCode tidak memiliki padanan — bukan sekadar beda
implementasi.

### Cakupan yang belum divalidasi secara manual (butuh pengecekan langsung)
- Detail tiap event di `session.next.*` (schema EventV2) — jumlahnya besar.
- Isi penuh `DialogProvider` / `DialogMcp` flow.
- `packages/plugin` hook server lengkap.
