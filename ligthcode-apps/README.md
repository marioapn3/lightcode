# LightCode

Coding agent ringan, native, di dalam terminal. Ditulis dalam **Rust**.

Bekerja seperti OpenCode / Claude Code / Codex CLI: kamu ketik tugas, agent
membaca repo, mencari file, mengubah kode, menjalankan perintah, memperbaiki
error, dan menampilkan hasilnya — secara streaming.

```
lightcode
    ↓
"Perbaiki bug autentikasi"
    ↓
agent cari file → baca → edit → tunjukkan diff → jalankan test → selesai
```

## Fitur

- **TUI** keyboard-first: timeline coding-agent (reasoning diringkas jadi
  `◌ Thinking...` / `✓ Thought for Xs`, tool activity compact, output
  collapsible, code block & diff render khusus dengan border + line numbers),
  **file edit menampilkan diff aktual** (before/after filesystem), **agent
  mode PLAN/BUILD/AUTO** (Shift+Tab atau `/mode`), modal izin, picker
  model/agent/sesi (**scoped per workspace**), command palette (`Ctrl+K`),
  leader key + which-key (`Ctrl+X`), **@-file mention autocomplete**, toast,
  autocomplete riwayat, status bar, cancel (Esc)
- **Agent loop** iteratif: tool call → izin → eksekusi → hasil → lanjut
- **Tool bawaan**: `read_file`, `write_file`, `edit_file`, `apply_patch`,
  `grep`, `glob`, `list_directory`, `shell`, `git_diff`, `git_status`,
  `git_log`, `web_search`, `web_fetch`, `task` (sub-agent), `todowrite`,
  `question`
- **Multi-provider**: OpenAI-compatible (OpenAI, OpenRouter, OpenCode Go, server
  lokal apa pun), Anthropic
- **Sesi persisten**: resume, list, delete, rename, fork, export/import
  (SQLite-free, file JSONL)
- **Izin granular**: pattern ruleset wildcard, `Always` ter-persist, tolak
  dengan alasan (feedback ke model)
- **Agent configurable**: `[agents.<name>]` dengan model + system prompt,
  ganti via `/agent`
- **Kontek terbatas**: output tool dibatasi, riwayat di-compact otomatis +
  manual (`/compact`)
- **Ringan**: idle ~1 MB RAM, startup instan, tanpa proses latar

## Install

### Homebrew (macOS)

```bash
brew install marioapn3/lightcode/lightcode
```

### One-liner (Linux & macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/marioapn3/lightcode/main/ligthcode-apps/install.sh | bash
```

Unduh binary prebuilt dari GitHub Releases; fallback ke `cargo install`.

### Cargo

Persyaratan: [Rust](https://rustup.rs) (cargo).

```bash
cargo install --git https://github.com/marioapn3/lightcode --locked
```

Biner terpasang ke `~/.cargo/bin/lightcode` (pastikan `~/.cargo/bin` ada di
`PATH`). Re-install dari sumber lokal setelah perubahan kode:

```bash
cargo install --path ligthcode-apps --locked --force
```

## Quickstart (OpenCode Go)

1. Subscribe & ambil API key di https://opencode.ai/auth
2. Simpan key:
   ```bash
   echo 'export OPENCODE_GO_API_KEY="zen-..."' >> ~/.zshrc && source ~/.zshrc
   ```
3. Jalan:
   ```bash
   lightcode
   ```

## Penggunaan

```bash
lightcode                          # TUI, sesi baru
lightcode --continue               # lanjut sesi terakhir
lightcode -s <session-id>          # lanjut sesi lama
lightcode run "jelaskan repo ini"  # one-shot, hasil ke stdout
lightcode run --format json "..."  # one-shot, event NDJSON (scripting/CI)
lightcode run -f file.rs "..."     # lampirkan file ke prompt
lightcode run --agent coder "..."  # jalan sebagai agent bernama dari config
lightcode --auto run "..."         # auto-approve semua izin (hati-hati)
lightcode session list             # daftar sesi
lightcode session show <id>        # tampilkan isi sesi
lightcode session rename <id> <title>
lightcode session fork <id>        # salin sesi
lightcode session delete <id>
lightcode session export <id>      # JSON ke stdout
lightcode session import <file.json>
lightcode models                   # daftar model provider aktif
lightcode providers                # daftar provider terkonfigurasi
lightcode stats [<id>]             # hitungan pesan & token
lightcode config                   # info config ter-resolve
lightcode -p openai -m gpt-5       # paksa provider + model tertentu
```

### Di dalam TUI

| Tombol | Aksi |
|---|---|
| `Enter` | kirim prompt / izinkan |
| `Alt+Enter` (Option) / `Shift+Enter` / `Ctrl+J` | baris baru |
| `Esc` | cancel agent / tolak izin / kosongkan input |
| `Ctrl+C` | keluar |
| `Ctrl+K` | command palette |
| `Ctrl+X` | leader key (which-key) — lalu `n`/`l`/`m`/`g`/`s`/`h`/`q` dll. |
| `Ctrl+D` | diff viewer fullscreen (dari blok diff) |
| `Ctrl+G` | scroll ke atas |
| `Ctrl+↑` / `Ctrl+↓` | pilih item timeline (tool/diff/reasoning) |
| `Enter` (saat terpilih) | buka/tutup output item |
| `Tab` | tampilkan/sembunyikan semua output tool & reasoning |
| `Shift+Tab` | ganti agent mode (PLAN → BUILD → AUTO) |
| `Cmd+C` | salin output ke clipboard (item terpilih, atau pesan terakhir) |
| `Ctrl+O` | salin output (cross-platform) |
| mouse drag | seleksi teks di konten (highlight) — lalu `Cmd+C`/`Ctrl+O` salin |
| scroll wheel | scroll percakapan |
| `PgUp` / `PgDn` | scroll riwayat |
| `↑` / `↓` | riwayat input / pilih di picker |

Perintah (ketik lalu Enter, suggestions muncul saat mulai dengan `/`):

| Perintah | Aksi |
|---|---|
| `/mode` | pilih agent mode (PLAN/BUILD/AUTO) |
| `/mode plan` / `/mode build` / `/mode auto` | langsung ganti mode |
| `/models` | ganti model (`↑`/`↓` pilih, `Enter` pilih) |
| `/agent` | ganti agent/mode dari config |
| `/sessions` | daftar & pindah sesi (`Del` hapus, filter dengan ketik) |
| `/new` | sesi baru |
| `/status` | info sesi / workspace |
| `/stats` | hitungan pesan & token |
| `/compact` | compact riwayat manual |
| `/rename <title>` | ganti judul sesi |
| `/fork` | fork sesi aktif |
| `/help` | pintasan & slash command |
| `/clear` | bersihkan layar |
| `/debug` | info sesi |
| `/quit` / `/exit` | keluar |

Komposer (editor teks lengkap):

| Tombol | Aksi |
|---|---|
| `Enter` | kirim prompt |
| `Shift+Enter` / `Alt+Enter` / `Ctrl+J` | baris baru |
| `↑` / `↓` | gerak kursor (baris pertama/terakhir = riwayat input) |
| `←` / `→` | gerak kursor |
| `Option+←/→` | pindah kata |
| `Option+Backspace` | hapus kata sebelumnya |
| `Cmd+←/→` atau `Ctrl+A/E` | awal/akhir baris |
| `Cmd+Backspace` atau `Ctrl+U` | hapus ke awal baris |
| `Cmd+A` / `Ctrl+A` | pilih semua |
| `Shift+←/→/↑/↓` | seleksi |
| `Cmd+C` / `Cmd+X` / `Cmd+V` | copy / cut / paste |
| `Cmd+Z` / `Cmd+Shift+Z` (atau `Ctrl+Z`/`Ctrl+Y`) | undo / redo |

Paste dari terminal (bracketed paste): konten kecil/multiline dimasukkan langsung
sebagai **satu operasi** (bukan per-karakter, newline tidak memicu submit).
Paste besar (>20 baris / >2000 char) diringkas jadi satu baris
`[Pasted text · N lines · M chars]` — konten penuh tersimpan, tekan **`Alt+P`**
untuk ekspansi, dan Enter tetap submit konten lengkapnya.

## Agent mode

Tiga mode dengan perilaku runtime nyata (bukan sekadar label):

| Mode | Perilaku |
|---|---|
| **PLAN** | Read-only. Analisis + rencana implementasi. Tool mutasi (`write_file`, `edit_file`, `apply_patch`, `shell`, `task`, `todowrite`) **diblokir di runtime**. |
| **BUILD** | Mode standar: baca, edit, buat, hapus, jalankan test — sesuai sistem izin. |
| **AUTO** | Eksekusi otonom: auto-approve aksi rutin, tapi tetap meminta izin untuk perintah berbahaya dan menghormati aturan Deny. |

Ganti dengan **`Shift+Tab`** (cycle) atau **`/mode`** / **`/mode plan`**.
Mode tampil di header (`[PLAN]`/`[BUILD]`/`[AUTO]`), bertahan per sesi, dan
ter-persist — resume sesi mengembalikan mode terakhir. Saat agent sedang
berjalan, perubahan mode berlaku untuk turn berikutnya.

## @-file mention

Ketik `@` di komposer untuk autocomplete file/direktori repo (fuzzy search,
hormati `.gitignore`, `node_modules`/`target`/dll. disembunyikan):

```text
› fix authentication in @auth.ser
                       ↓
┌─ Files · @auth.ser ────────────────┐
│ › src/auth/auth.service.ts         │
│   src/auth/auth.service.spec.ts    │
└────────────────────────────────────┘
  Enter → @src/auth/auth.service.ts
```

- `↑/↓` pilih, `Enter`/`Tab` pilih, `Esc` tutup tanpa mengubah input.
- Navigasi direktori: `@src/auth/` → tampilkan isi; direktori bisa dipilih.
- Saat submit, isi file yang disebut **di-inject ke konteks agent**
  (file inline, direktori cuma metadata + listing dangkal — tidak pernah
  membuang seluruh isi direktori besar ke konteks). Path yang tak ada
  dilaporkan ke model agar ditangani dengan baik.

## Konfigurasi

### Lokasi file config

| Urutan | Lokasi |
|---|---|
| 1 | `lightcode --config <file>` |
| 2 | env `LIGHTCODE_CONFIG` |
| 3 | **`lightcode.json`** (atau `lightcode.toml`) di folder proyek |
| 4 | macOS: `~/Library/Application Support/lightcode/config.json` — Linux: `~/.config/lightcode/config.json` |

> **Per-laptop.** Config berada di mesin lokal masing-masing developer, bukan di
> repo. Setiap orang mengatur provider/API key-nya sendiri di
> `~/Library/Application Support/lightcode/config.json`. `lightcode.json` di
> proyek bersifat opsional — kalau dipakai, masukkan ke `.gitignore` agar tidak
> ke-commit (berisi API key).

Format JSON **atau** TOML (didukung keduanya; dideteksi dari ekstensi).

### Contoh `lightcode.json` (di root proyek)

```json
{
  "agent": {
    "provider": "opencode-go",
    "max_context_tokens": 60000,
    "max_iterations": 50
  },
  "provider": {
    "opencode-go": {
      "model": "deepseek-v4-flash"
    },
    "openai": {
      "model": "gpt-4o",
      "api_key": "sk-..."           // atau pakai env OPENAI_API_KEY
    }
  },
  "permissions": {
    "edit": "ask",
    "write": "ask",
    "shell": "ask"
  }
}
```

Versi TOML yang setara:

```toml
[agent]
provider = "opencode-go"
max_iterations = 50

[provider.opencode-go]
model = "deepseek-v4-flash"

[permissions]
shell = "ask"
```

### Provider custom (base URL server sendiri)

Provider **OpenAI-compatible** apa pun bisa ditambahkan cukup dengan
`base_url` (+ key kalau perlu). Formatnya kompatibel dengan config opencode:

```json
{
  "agent": { "provider": "lokal-kantor" },
  "provider": {
    "lokal-kantor": {
      "name": "LOKAL-KANTOR",
      "options": {
        "baseURL": "http://100.118.237.83:20128/v1",
        "apiKey": "rahasia-kantor"     // kosongkan "" kalau server tanpa auth
      },
      "models": {
        "codex/gpt-5.6-sol": { "name": "GPT 5.6 Sol" },
        "codex/gpt-5.6-luna": { "name": "GPT 5.6 Luna" }
      }
    }
  }
}
```

Catatan:

- `baseURL` / `baseUrl` / `base_url` semuanya diterima.
- `apiKey` kosong (`""`) = tanpa autentikasi (header di-skip).
- Bila `models` diisi, `--list-models` menampilkannya dan model pertama jadi
  default; ganti dengan `-m codex/gpt-5.6-sol`.

Provider OpenAI-compatible lain tinggal atur `base_url` + `api_key`:

```json
{
  "provider": {
    "openrouter": {
      "base_url": "https://openrouter.ai/api/v1",
      "model": "openrouter/auto",
      "api_key": "sk-or-..."
    }
  }
}
```

### Env var API key

Key bisa dari config atau env: `{PROVIDER}_API_KEY` (strip jadi `_`, huruf besar).
Contoh: `OPENAI_API_KEY`, `OPENCODE_GO_API_KEY`, `ANTHROPIC_API_KEY`,
`OPENROUTER_API_KEY`, `TAVILY_API_KEY` (web search), `LOKAL_KANTOR_API_KEY`.

## Provider

| Provider | Env var | Endpoint default |
|---|---|---|
| `openai` | `OPENAI_API_KEY` | `https://api.openai.com/v1` |
| `opencode-go` | `OPENCODE_GO_API_KEY` | `https://opencode.ai/zen/go/v1` |
| `openrouter` | `OPENROUTER_API_KEY` | `https://openrouter.ai/api/v1` |
| `anthropic` | `ANTHROPIC_API_KEY` | `https://api.anthropic.com/v1` |
| apa pun + `base_url` | `{NAME}_API_KEY` | dari config |

- **OpenCode Go** (OpenAI-compatible): grok-4.5, glm-5.x, kimi-k*, deepseek-v4-*,
  mimo-*, hy3, dll.
- **Anthropic** juga melayani model Go yang format Anthropic — set
  `base_url` ke `https://opencode.ai/zen/go/v1` dan `model` qwen/minimax.
- Belum didukung: OpenAI Responses API (`gpt-5.6-luna`).

## Sesi

Sesi **scoped per workspace** — ditentukan dari Git repo root (atau direktori
ternormalisasi kalau bukan Git). Diluncurkan dari `~/Code/project-a` hanya
melihat sesi `project-a`; dari `project-b` hanya sesi `project-b`. Sesi dibuat
di dalam satu project tidak bocor ke project lain; resume sesi dari project
lain ditolak dengan jelas.

Disimpan di `~/Library/Application Support/lightcode/sessions` (macOS) atau
`~/.local/share/lightcode/sessions`. Bisa di-override dengan `LIGHTCODE_DATA_DIR`.

```bash
lightcode                    # buat sesi baru di workspace saat ini
lightcode --continue         # lanjut sesi terakhir di workspace ini
lightcode -s <id>            # lanjut (harus sesi workspace ini)
lightcode session list       # sesi workspace ini saja
lightcode session list --all # semua workspace + (unscoped)
lightcode session adopt <id> # pindahkan sesi dari workspace lain ke sini
lightcode session adopt all  # pindahkan semua sesi unscoped ke sini
lightcode session rename <id> <title>
lightcode session fork <id>
lightcode session delete <id>
lightcode session export <id> > backup.json
lightcode session import backup.json
```

Sesi lama (sebelum scoping, tanpa info workspace) tetap aman di tempatnya
sebagai *unscoped* — tidak dihapus, tidak otomatis tampil di semua project.
`session list --all` menampilkannya, dan `session adopt all` memindahkan ke
workspace aktif.

## Izin

Konfigurasi per-action (`[permissions]`) + ruleset pattern wildcard
(last-match wins). Contoh config:

```json
{
  "permissions": {
    "shell": "ask",
    "rules": [
      { "permission": "shell", "pattern": "git status", "action": "allow" },
      { "permission": "write", "pattern": "**/*.lock", "action": "deny" },
      { "permission": "*", "pattern": "**/vendor/**", "action": "deny" }
    ]
  }
}
```

Saat modal izin muncul: `Enter` Allow · `Esc`/`n` Deny · `A` Allow for session ·
`W` Always (ter-persist per sesi) · `R` Deny + alasan (feedback ke model).

## Agent configurable

```json
{
  "agents": {
    "coder": { "model": "gpt-5", "system_prompt": "You are a careful Rust engineer." },
    "reviewer": { "systemPrompt": "Review diffs strictly and reject bad changes." }
  }
}
```

Ganti di TUI dengan `/agent` atau `Ctrl+X` lalu `g`. Sub-agent (`task` tool)
mewarisi provider & tools dari agent utama.

## Web

- `web_fetch` jalan tanpa key.
- `web_search` pakai `TAVILY_API_KEY`; fallback DuckDuckGo (mungkin diblokir di
  sebagian jaringan).

## Debug: log provider

Aktifkan log untuk melihat request/response provider (berguna saat stream
error):

```bash
LIGHTCODE_LOG=1 lightcode                      # log di samping folder sesi
LIGHTCODE_LOG=/tmp/lc.log lightcode            # path custom
```

Log mencatat request (provider, model, url), status response, dan error stream
termasuk snippet body yang sudah diterima. Saat stream diputus server sebelum
ada output, LightCode **otomatis retry sekali** — jika tetap gagal, error
ditampilkan dengan status + snippet biar mudah didiagnosis.

## Pengembangan

```bash
cd ligthcode-apps
cargo build                  # debug
cargo build --release        # release
cargo test                   # unit test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt
```

Struktur:

```
ligthcode-apps/
├── src/
│   ├── main.rs          # CLI subcommands (run/session/models/stats/...) + wiring
│   ├── agent/           # loop, sub-agent (task), reasoning, compaction, agent defs
│   ├── tools/           # read, write, edit, apply_patch, grep, glob, shell, git, web, task, todowrite, question
│   ├── diff.rs          # unified-diff generator + file snapshot (edit diffs)
│   ├── workspace.rs     # workspace resolution (git root / normalized dir)
│   ├── files/           # file index + fuzzy search (@-mention)
│   ├── mentions.rs      # @-mention detection + context resolution
│   ├── providers/       # openai, anthropic (OpenAI-compatible via base_url)
│   ├── permissions/     # policy + ruleset wildcard + deteksi perintah berbahaya
│   ├── session/         # storage JSONL scoped per workspace + fork/rename/adopt
│   ├── history.rs       # prompt history persisten + autocomplete
│   ├── web/             # fetch + search (terisolasi)
│   └── tui/             # Ratatui: app, input, render, pickers, dialogs
└── docs/performance.md  # pengukuran performa
```

Referensi arsitektur OpenCode: `plan/file.md` (folder parent).
