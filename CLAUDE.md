# BeatCheck (RSS Reader)

Central config: `~/.claude/CLAUDE.md`
Rust rules: `~/.claude/rules/rust-projects.md`

## Build & Run

```bash
cargo build              # Dev build
cargo build --release    # Release build
cargo run                # Run TUI
cargo run -- --refresh   # Headless refresh
cargo run -- --import feeds.opml  # Import OPML
cargo test               # Run tests
cargo clippy             # Lint
cargo fmt                # Format
```

## Architecture

- **Async TUI**: tokio + ratatui with non-blocking operations
- **Database**: SQLite via tokio-rusqlite (7-day retention, auto-compaction)
- **Config**: TOML at `~/.config/beatcheck/config.toml`

### Modules

| Module | Purpose |
|--------|---------|
| `app.rs` | Central state, async channels |
| `tui/ui.rs` | Rendering (split-pane layout) |
| `tui/handler.rs` | Key bindings |
| `db/repository.rs` | Database operations |
| `feed/fetcher.rs` | RSS/Atom fetching, auto-discovery |
| `feed/opml.rs` | Import/export |
| `ai/summarizer.rs` | Claude API for summaries |
| `services/raindrop.rs` | Raindrop.io bookmarking |

### Async Pattern

1. Action triggers `tokio::spawn`
2. Task sends result via channel (`summary_tx`, `refresh_tx`, `discovery_tx`)
3. Main loop polls channels every 100ms
4. Results update state and trigger redraw

## Filter Modes

`f` key cycles: Unread -> Starred -> All

## Spinners

Braille animation: `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`

## Recent Changes (2026-02-01)

### Browser Cookie Support Enhancement

**Previous:** Only supported Firefox cookies for fetching paywalled articles

**Updated:** Now supports both Chrome and Firefox with intelligent fallback

**Implementation:**
- **Primary:** Chrome/Chromium cookies (Windows FILETIME epoch)
  - Database: `~/.config/google-chrome/Default/Cookies`
  - Timestamp: `(unix_timestamp + 11_644_473_600) * 1_000_000` microseconds
  - Table: `cookies` with `host_key`, `expires_utc`

- **Fallback:** Firefox cookies (Unix epoch)
  - Database: `~/.mozilla/firefox/*/cookies.sqlite`
  - Timestamp: `unix_timestamp` in seconds
  - Table: `moz_cookies` with `host`, `expiry`

**Loading Strategy:**
1. Try Chrome first (most common browser)
2. Fall back to Firefox if Chrome unavailable
3. Per-domain cookie filtering for article URLs
4. Filters expired cookies using correct epoch format

**Files Modified:**
- `src/services/content_fetcher.rs` - Added Chrome support with Firefox fallback
  - `get_chrome_cookies_internal()` - Chrome cookie loading
  - `get_firefox_cookies_internal()` - Firefox cookie loading (preserved)
  - `find_firefox_cookies()` - Firefox profile detection

**Commit:** `5d299cb` - feat: add Chrome cookie support with Firefox fallback

**Benefits:**
- Works with whichever browser user has installed
- Automatic browser detection and selection
- Better compatibility for paywalled content access
