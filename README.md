# SpeedyReader

A terminal-based RSS reader with AI-powered article summaries.

Built for personal use, entirely vibe coded with [Claude Code](https://claude.ai/code).

## Features

- **Split-pane TUI**: Feed content (top) + AI bullet-point summary (bottom)
- **Claude API integration**: Concise bullet-point summaries of articles
- **Feed discovery**: Add feeds by URL with automatic RSS/Atom detection
- **Raindrop.io integration**: Bookmark articles with AI summary in notes
- **Permanent delete**: Deleted articles won't return on refresh
- **OPML import**: Import feed subscriptions
- **SQLite caching**: Offline reading with 7-day retention
- **Auto-mark read**: Articles marked read after 2 seconds

## Installation

### From Source

Requires Rust 1.70+:

```bash
git clone https://github.com/leolaporte/rss-reader.git
cd rss-reader
cargo install --path .
```

## Configuration

Create `~/.config/speedy-reader/config.toml`:

```toml
# Required for AI summaries
claude_api_key = "sk-ant-..."

# Optional: Raindrop.io integration
raindrop_token = "..."
```

## Usage

```bash
# Run the TUI
speedy-reader

# Import OPML subscriptions
speedy-reader --import feeds.opml

# Headless refresh (for cron/systemd)
speedy-reader --refresh
```

### Key Bindings

| Key | Action |
|-----|--------|
| `j`/`k` or `↓`/`↑` | Navigate articles |
| `Enter` | Generate/show summary |
| `r` | Refresh all feeds |
| `a` | Add new feed |
| `s` | Toggle starred |
| `m` | Toggle read/unread |
| `o` | Open in browser |
| `e` | Email article |
| `b` | Bookmark to Raindrop.io |
| `f` | Cycle filter (Unread/Starred/All) |
| `g` | Regenerate summary |
| `d` | Delete article |
| `?` | Show help |
| `q` | Quit |

## Systemd Timer (Auto-refresh)

To refresh feeds automatically every hour:

```bash
# Copy service files
mkdir -p ~/.config/systemd/user
cp systemd/*.{service,timer} ~/.config/systemd/user/

# Enable timer
systemctl --user enable --now speedy-reader-refresh.timer
```

## License

MIT
