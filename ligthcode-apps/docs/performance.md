# LightCode Performance

Measured on macOS (Apple Silicon), Rust 1.90, release build (`cargo build --release`).

## Summary

LightCode is a native Rust binary with no background processes. Idle memory and
CPU are near-zero; startup is effectively instant.

## Measurements

| Metric | Value |
|---|---|
| Binary size (release) | 8.4 MB |
| Startup (warm, `--list-sessions`) | ~0.00 s |
| Startup (cold) | ~0.28 s |
| Peak RSS (`--help`) | 2.9 MB |
| TUI idle RSS | 1.3 MB |
| TUI idle CPU | 0.0% |

## How it stays small

- **Native Rust, no runtime/VM.** The only "background work" is a per-request
  HTTP stream and the TUI's 80 ms render tick (drawn only when idle/streaming).
- **Bounded tool output.** `read_file`/`grep`/`shell`/`git`/`web` cap their
  results (`MAX_TOOL_OUTPUT` = 32 KB, subprocess output drained but kept to
  64 KB, `MAX_FILE_BYTES` = 10 MB file-size gate, web responses ≤ 256 KB).
  A 500 MB log file is refused, not read into memory.
- **Lazy loading.** Only the working directory is walked by `grep` (via the
  `ignore` crate, respecting `.gitignore`); files are read on demand, never the
  whole repository.
- **Bounded history.** Conversation is estimated (chars/4) and compacted past
  `[agent] max_context_tokens` (default 60k) by replacing old messages with a
  model summary.
- **No history cloning.** `Provider::complete` takes borrowed message slices;
  tool schemas are cached once per `Registry`.

## Scaling checks

| Scenario | Design |
|---|---|
| Small repo | `grep` walks files only when asked |
| Large repo | `grep` skips files > 10 MB, hidden dirs, and git-ignored paths |
| Long conversation | `compact()` keeps the last 12 messages + a summary |
| Large tool output | truncated at 32 KB before entering the model context |
| Multiple tool calls | executed sequentially; results bounded |

## Measuring locally

```bash
cargo build --release
ls -lh target/release/lightcode

# startup + RSS
/usr/bin/time -l target/release/lightcode --help

# TUI idle RSS/CPU
LIGHTCODE_DATA_DIR=/tmp/lc OPENCODE_GO_API_KEY=x \
  script -q /dev/null target/release/lightcode &  # quit with Ctrl+C
ps -o rss=,pcpu= -p $(pgrep -f "target/release/lightcode" | head -1)
```

## Known trade-offs

- `grep` runs synchronous filesystem I/O inside an async tool (benign while
  tools execute sequentially; wrap in `spawn_blocking` if parallel tool
  execution is added).
- Token estimation is a heuristic (chars/4), not a real tokenizer; it is only
  used for compaction thresholds.
