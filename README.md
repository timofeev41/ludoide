# Ludo

A small, fast, native IDE for data engineering, written in Rust ([egui](https://github.com/emilk/egui)).
It's built for ClickHouse work, benthos/dbt YAML, SQL scratches and the odd Airflow/FastAPI Python project.

![Ludo](docs/screenshot.png)

## Features

**Databases (right panel)**
- ClickHouse over HTTP(S). Add several connections; a URL + user pair can only exist once.
- Passwords are stored in the **macOS Keychain** (service "Ludo database connection"), never in `settings.json`.
  Passwords from older settings files move there automatically on first launch.
- Browse databases → tables / views / materialized views / dictionaries / skip indices → columns,
  with a filter box. Row counts and sizes show on hover.
- **Quick inspect**: double-click a table (or use the context menu) to run `SELECT * … LIMIT 500`.
- **Show CREATE statement** for an object, or the **whole database DDL** (all `CREATE` statements).
- Run SQL from any `.sql` file: `⌘↵` runs the statement at the cursor (or the selection),
  `⌘⇧↵` runs the whole script statement by statement. Cancel sends `KILL QUERY`.
- Results grid with types, NULLs, copying a cell/row/TSV; rows read and bytes read shown for every query.
- **DB Log**: every request with timestamp, endpoint, query id, server summary header, HTTP status and timing.
- SQL linting with the active connection's dialect (sqlparser). It's advisory: the parser doesn't know every
  ClickHouse extension.

**Editor**
- Up to 10 tabs (when you open an 11th, the least recently used *saved* tab closes). Tabs and folders are restored on the next launch.
- Syntax highlighting for SQL, Python, YAML, Shell, Markdown, JSON and TOML.
- Selection + `(` `[` `{` `<` `"` `'` `` ` `` wraps the selection.
- `⌥⇧↑` / `⌥⇧↓` move the current line(s), `Tab` / `⇧Tab` indent/outdent the selected lines,
  `⌘/` toggles comments (`--` for SQL, `#` for Python/Shell/YAML). Enter keeps indentation and adds a level after `:` `(` `[` `{`.
- `⌘F` find / replace.
- YAML lint: syntax errors, duplicate keys (reported on the duplicate line), tab indentation, odd indentation, trailing whitespace.
- Markdown: edit, split view or preview (toolbar toggles).

**Workspace**
- One or several folders per workspace (Explorer → *Add folder*), with new/rename/delete/reveal in the context menu.
- **Scratches** (`~/LudoScratches`, configurable) are always listed, whatever project is open. You can make subfolders and
  SQL/Python/shell/Markdown scratches; `⌘N` creates a new SQL scratch. SQL scratches run on the active connection,
  Python scratches on the selected interpreter.

**Terminals and Python**
- Several terminal sessions (a real PTY with 256 colors and truecolor). Double-click a tab to rename it. Each one starts as a login shell in the project root.
- Python: pick an interpreter executable (or *Detect* Homebrew/pyenv/`.venv`s), create a venv, and run the current file with `⌘↵`.
  It runs in a terminal tab through your login shell, so `PATH` matches your normal terminal.
- **Claude Code**: the ✨ button opens `claude` in a terminal in the current project. The editor toolbar's ✨ starts it with the current file as context.

**Settings** (`⌘,`) are split into *General* (theme, interface/editor font from system fonts, font size, scale in 10% steps, indentation 2/4/6),
*Coding* (interpreters, connections, linting, limits), *Terminal* (shell, font size, scrollback, env vars) and *Others*
(scratches folder, session restore, autosave, Claude command). They're stored in `~/Library/Application Support/ludo/settings.json`.

## Shortcuts

| Action | Keys |
|---|---|
| Run statement / selection · run script | `⌘↵` · `⌘⇧↵` |
| Next / previous tab | `⌃Tab` / `⌃⇧Tab` (also `⌘⇧]` / `⌘⇧[`), `⌘1…9` |
| Save · close tab · new SQL scratch | `⌘S` · `⌘W` · `⌘N` |
| Toggle explorer · bottom panel · databases | `⌘B` · `⌘J` · `⌘⇧D` |
| New terminal | `⌘⇧`` ` |
| Find / replace | `⌘F` |
| Zoom interface | `⌘+` / `⌘-` |
| Settings | `⌘,` |

## Build

```sh
cargo run --release -- ~/path/to/project      # run from source
scripts/bundle_macos.sh                        # dist/Ludo.app (universal) + zip
scripts/bundle_macos.sh --native               # host architecture only (faster)
```

Every push builds a universal `Ludo.app` on GitHub Actions (artifact **Ludo-macos**). Publishing a GitHub release
attaches `Ludo-<version>-macos.zip` to it automatically. For an existing release, run the **build** workflow manually
(Actions → build → Run workflow) and enter the tag.

The app is ad-hoc signed, not notarized, so macOS blocks it the first time. After unzipping, either run
`xattr -dr com.apple.quarantine Ludo.app` or open it once and allow it in System Settings → Privacy & Security → *Open Anyway*.

## Architecture

```
src/
  app.rs            state, per-frame logic, global shortcuts, run actions
  settings.rs       settings + session persistence (JSON)
  theme.rs          palettes → egui visuals          (add a theme: one entry in THEMES)
  languages.rs      language registry                (add a language: one entry in LANGUAGES)
  editor/           document model, text ops (unit-tested), highlighter, editor widget
  lint/             sql.rs, yaml.rs                  (add a linter: a module + a match arm)
  db/               Driver trait, ClickHouse driver, background worker
  terminal.rs       PTY + vt100 terminal widget
  ui/               panels: explorer, databases, bottom panel, settings, find bar, chrome
```

### Adding a database engine

1. Add a variant to `db::DriverKind`, and fill in its `name`, `default_url`, `sql_dialect` and `connect`.
2. Implement `db::Driver` (`databases`, `objects`, `columns`, `object_ddl`, `database_ddl`, `query`, `cancel`, `inspect_sql`) in a new module.

The panel, results grid, log, linting and connection editor work unchanged, because they only talk to the trait.

## Development

```sh
cargo test                                               # unit tests (on Linux: `keyctl session - cargo test`
                                                         # to exercise the kernel keyring instead of skipping)
LUDO_CH_URL=http://localhost:8123 cargo test -- --ignored # live ClickHouse test
```

The live test expects an `analytics` database:

```sql
CREATE DATABASE analytics;
CREATE TABLE analytics.events (ts DateTime, user_id UInt64, event LowCardinality(String), props Map(String,String),
  amount Nullable(Float64), INDEX idx_event event TYPE set(100) GRANULARITY 4) ENGINE = MergeTree ORDER BY (event, ts);
CREATE TABLE analytics.daily (d Date, event String, c UInt64) ENGINE = SummingMergeTree ORDER BY (d, event);
CREATE MATERIALIZED VIEW analytics.mv_daily TO analytics.daily AS SELECT toDate(ts) d, event, count() c FROM analytics.events GROUP BY d, event;
INSERT INTO analytics.events SELECT now() - number, number % 1000, ['click','view','buy'][number % 3 + 1],
  map('k', toString(number)), if(number % 5 = 0, NULL, number / 10) FROM numbers(100000);
CREATE VIEW analytics.v_buys AS SELECT * FROM analytics.events WHERE event = 'buy';
```

## Known limitations (v0.1)

- The app is ad-hoc signed, so after installing a new build macOS asks once more for Keychain access
  (choose *Always Allow*).
- The terminal has no mouse text selection yet (paste with `⌘V` works).
- There's no autocomplete yet. SQL lint warnings are advisory.
- Text doesn't wrap in the editor, and very large files (more than ~5 MB) will feel slower.
