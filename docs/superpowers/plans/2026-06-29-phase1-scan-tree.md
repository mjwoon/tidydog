# TidyDog Phase 1: Scan → Tree Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire a full vertical slice: folder selection → Rust scan with BLAKE3 hashing + SQLite upsert → React tree render, proving FE↔Rust↔FS↔DB integration.

**Architecture:** Dialog plugin handles OS folder picker. `scan_directory` Tauri command recursively traverses the selected folder using `std::fs::read_dir`, hashes each regular file with BLAKE3, upserts metadata into SQLite (`app_data_dir/index.db`), and returns a `FileNode` tree. The frontend renders the tree using a recursive `TreeNode` React component styled to match `design/mockup_main_shell.html`.

**Tech Stack:** Tauri 2, Rust (rusqlite/bundled, blake3, mime_guess), React 19 + TypeScript, Tailwind v4, @tauri-apps/plugin-dialog

## Global Constraints

- **Read-only phase**: no file move/rename/delete/write. Only DB writes to `app_data_dir/index.db`.
- DB lives at `app.path().app_data_dir()` → macOS `~/Library/Application Support/com.mjwoon.tidydog/index.db`.
- `files` schema is locked: exact DDL in Task 2. No column additions or renames.
- `tidydog-core` is NOT called in this phase.
- `invoke` from `@tauri-apps/api/core`; dialog from `@tauri-apps/plugin-dialog`.
- Design single source: `design/mockup_main_shell.html`. No red-family colors anywhere.
- Tailwind v4 only (`@import "tailwindcss"` + `@theme`). No `@tailwind` directives, no PostCSS config.
- `vite.config.ts` Tauri server block must not be touched.

## File Map

| File | Action | Responsibility |
|---|---|---|
| `src-tauri/Cargo.toml` | Modify | Add rusqlite(bundled), blake3, mime_guess |
| `src-tauri/src/db.rs` | **Create** | `init_db()`: open SQLite, WAL pragma, create `files` schema |
| `src-tauri/src/scanner.rs` | **Create** | `FileNode` struct, `scan_recursive()`: traverse + hash + upsert |
| `src-tauri/src/lib.rs` | Modify | Add `db`/`scanner` modules, `scan_directory` command, register in handler |
| `src-tauri/capabilities/default.json` | Modify | Add `dialog:default` permission (done by `tauri add dialog`) |
| `index.html` | Modify | Add Pretendard (jsDelivr) + IBM Plex Mono (Google Fonts) CDN links |
| `src/styles.css` | Modify | `@theme` color/font tokens + `:root` aliases + mockup CSS |
| `src/types.ts` | **Create** | Shared `FileNode` TypeScript interface |
| `src/components/TreeNode.tsx` | **Create** | Recursive tree node: toggle, icon, name, size, depth indentation |
| `src/components/Topbar.tsx` | **Create** | 56px topbar: dog brand, folder-pill, settings icon |
| `src/components/DogMascot.tsx` | **Create** | Floating dog SVG with bob animation + state badge |
| `src/App.tsx` | Modify | Full shell: Topbar + Sidebar(tree) + Main(placeholder chat) + DogMascot |

---

### Task 1: Dialog Plugin + Rust Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/capabilities/default.json` (auto by CLI)
- Modify: `src-tauri/src/lib.rs` (auto by CLI — adds plugin init)

**Interfaces:**
- Produces: `@tauri-apps/plugin-dialog` available in JS; `tauri_plugin_dialog` available in Rust; rusqlite/blake3/mime_guess available as Rust crates

- [ ] **Step 1: Add dialog plugin via Tauri CLI**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run tauri add dialog
```
Expected: output mentions "Added plugin dialog to Cargo.toml" and updates capabilities.

- [ ] **Step 2: Install JS binding**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm install @tauri-apps/plugin-dialog
```
Expected: exits 0, `@tauri-apps/plugin-dialog` in node_modules.

- [ ] **Step 3: Add remaining Rust dependencies**

Open `src-tauri/Cargo.toml`. In `[dependencies]`, add after the existing lines:

```toml
rusqlite = { version = "0.31", features = ["bundled"] }
blake3 = "1"
mime_guess = "2"
```

Full `[dependencies]` block should look like (dialog line added by CLI):
```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
tauri-plugin-dialog = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tidydog-core = { path = "../crates/tidydog-core" }
rusqlite = { version = "0.31", features = ["bundled"] }
blake3 = "1"
mime_guess = "2"
```

- [ ] **Step 4: Verify compilation**

```bash
cd /Users/mjwoon/Workspace/tidydog && cargo check 2>&1 | tail -5
```
Expected: `Finished` with no errors. (First run will download crates — may take 1–2 min.)

- [ ] **Step 5: Verify capabilities have dialog permission**

```bash
cat /Users/mjwoon/Workspace/tidydog/src-tauri/capabilities/default.json
```
Expected: `"dialog:default"` or similar dialog permission in the `permissions` array. If missing, add it manually:
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "opener:default",
    "dialog:default"
  ]
}
```

---

### Task 2: DB Layer (`src-tauri/src/db.rs`)

**Files:**
- Create: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod db;`)

**Interfaces:**
- Produces: `db::init_db(app_data_dir: &Path) -> Result<rusqlite::Connection, String>`

- [ ] **Step 1: Create `src-tauri/src/db.rs`**

```rust
use rusqlite::Connection;
use std::path::Path;

pub fn init_db(app_data_dir: &Path) -> Result<Connection, String> {
    let db_path = app_data_dir.join("index.db");
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS files (
             id            INTEGER PRIMARY KEY,
             content_hash  TEXT    NOT NULL,
             current_path  TEXT    NOT NULL UNIQUE,
             size          INTEGER NOT NULL,
             mtime         INTEGER NOT NULL,
             created_at    INTEGER,
             mime          TEXT,
             ext           TEXT,
             read_level    TEXT    CHECK(read_level IN ('content','metadata')),
             summary       TEXT, topic_tags TEXT, language TEXT, cluster_id INTEGER,
             dup_group     TEXT, proposed_dest TEXT, risk_score REAL,
             last_action   TEXT, last_seen INTEGER, indexed_at INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_files_hash ON files(content_hash);",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}
```

- [ ] **Step 2: Register module in `src-tauri/src/lib.rs`**

Add `mod db;` at the top of `lib.rs` (before the existing code):

```rust
mod db;
```

- [ ] **Step 3: Verify compilation**

```bash
cd /Users/mjwoon/Workspace/tidydog && cargo check 2>&1 | tail -5
```
Expected: `Finished` with no errors.

---

### Task 3: Scanner (`src-tauri/src/scanner.rs`)

**Files:**
- Create: `src-tauri/src/scanner.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod scanner;`)

**Interfaces:**
- Produces:
  - `scanner::FileNode` (Serialize) — `{ name: String, path: String, is_dir: bool, size: Option<u64>, ext: Option<String>, children: Vec<FileNode> }`
  - `scanner::scan_recursive(path: &Path, depth: usize, max_depth: usize, conn: &Connection) -> Option<FileNode>`

- [ ] **Step 1: Create `src-tauri/src/scanner.rs`**

```rust
use rusqlite::Connection;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub ext: Option<String>,
    pub children: Vec<FileNode>,
}

/// Returns None to signal "skip this entry" (symlink, hidden, permission error).
/// Errors reading children are swallowed so one bad entry doesn't abort the scan.
pub fn scan_recursive(
    path: &Path,
    depth: usize,
    max_depth: usize,
    conn: &Connection,
) -> Option<FileNode> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    // Skip hidden entries
    if name.starts_with('.') {
        return None;
    }

    let path_str = path.to_string_lossy().to_string();

    if path.is_dir() {
        let mut children = Vec::new();
        if depth < max_depth {
            if let Ok(read_dir) = fs::read_dir(path) {
                let mut entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
                // Directories first, then files; both alphabetical
                entries.sort_by(|a, b| {
                    let a_dir = a.path().is_dir();
                    let b_dir = b.path().is_dir();
                    match (a_dir, b_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a.file_name().cmp(&b.file_name()),
                    }
                });
                for entry in &entries {
                    if let Some(child) =
                        scan_recursive(&entry.path(), depth + 1, max_depth, conn)
                    {
                        children.push(child);
                    }
                }
            }
        }
        Some(FileNode {
            name,
            path: path_str,
            is_dir: true,
            size: None,
            ext: None,
            children,
        })
    } else if path.is_file() {
        let metadata = fs::metadata(path).ok()?;
        let size = metadata.len();

        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let created_at = metadata
            .created()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        // BLAKE3: stream through file to avoid loading into memory
        let content_hash = {
            let mut file = fs::File::open(path).ok()?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher).ok()?;
            hasher.finalize().to_hex().to_string()
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Upsert: insert or update size/mtime/hash/timestamps
        let _ = conn.execute(
            "INSERT INTO files
                 (content_hash, current_path, size, mtime, created_at,
                  mime, ext, read_level, last_seen, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'metadata', ?8, ?9)
             ON CONFLICT(current_path) DO UPDATE SET
                 content_hash = excluded.content_hash,
                 size         = excluded.size,
                 mtime        = excluded.mtime,
                 last_seen    = excluded.last_seen,
                 indexed_at   = excluded.indexed_at",
            rusqlite::params![
                content_hash,
                path_str,
                size as i64,
                mtime,
                created_at,
                mime,
                ext,
                now,
                now
            ],
        );

        Some(FileNode {
            name,
            path: path_str,
            is_dir: false,
            size: Some(size),
            ext,
            children: vec![],
        })
    } else {
        None // symlinks, special devices — skip
    }
}
```

- [ ] **Step 2: Register module in `src-tauri/src/lib.rs`**

```rust
mod db;
mod scanner;
```

- [ ] **Step 3: Verify compilation**

```bash
cd /Users/mjwoon/Workspace/tidydog && cargo check 2>&1 | tail -5
```
Expected: `Finished` with no errors.

---

### Task 4: Wire `scan_directory` Command

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `db::init_db`, `scanner::scan_recursive`, `scanner::FileNode`
- Produces: Tauri command `scan_directory(root: String, max_depth?: number) -> FileNode` callable from JS as `invoke('scan_directory', { root, maxDepth })`

- [ ] **Step 1: Replace `src-tauri/src/lib.rs` entirely**

```rust
mod db;
mod scanner;

use std::fs;
use tauri::Manager;

#[tauri::command]
fn health() -> String {
    format!("TidyDog core v{} · ready", tidydog_core::core_version())
}

#[tauri::command]
fn scan_directory(
    app: tauri::AppHandle,
    root: String,
    max_depth: Option<usize>,
) -> Result<scanner::FileNode, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;
    let depth = max_depth.unwrap_or(10);
    let path = std::path::Path::new(&root);
    scanner::scan_recursive(path, 0, depth, &conn)
        .ok_or_else(|| format!("Cannot scan root path: {}", root))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![health, scan_directory])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

> **Note:** If `npm run tauri add dialog` already added `.plugin(tauri_plugin_dialog::init())` to lib.rs, the above replaces that version. The final file must have both `health` and `scan_directory` registered, and both opener and dialog plugins initialized.

- [ ] **Step 2: Verify full workspace compiles**

```bash
cd /Users/mjwoon/Workspace/tidydog && cargo check 2>&1 | tail -8
```
Expected: `Checking tidydog-core v0.1.0`, `Checking tidydog v0.1.0`, `Finished`.

---

### Task 5: Design Tokens + Fonts

**Files:**
- Modify: `index.html`
- Modify: `src/styles.css`

**Interfaces:**
- Produces: CSS custom properties `--bg`, `--surface`, `--surface-2`, `--ink`, `--muted`, `--line`, `--primary`, `--primary-soft`, `--accent`, `--caution`, `--caution-soft`, `--r`, `--r-sm`, `--shadow`, `--ui`, `--mono` available globally; Tailwind utilities `bg-bg`, `bg-surface`, `text-primary`, `font-mono`, etc. available; Pretendard + IBM Plex Mono loaded.

- [ ] **Step 1: Add font CDN links to `index.html`**

Replace the entire `index.html`:

```html
<!doctype html>
<html lang="ko">
  <head>
    <meta charset="UTF-8" />
    <link rel="icon" type="image/svg+xml" href="/vite.svg" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>TidyDog</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/static/pretendard.min.css">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&display=swap" rel="stylesheet">
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 2: Replace `src/styles.css` with design tokens + mockup CSS**

```css
@import "tailwindcss";

/* Tailwind v4 design tokens — creates bg-bg, bg-surface, text-primary, font-mono, etc. */
@theme {
  --color-bg: #E9EDEB;
  --color-surface: #FFFFFF;
  --color-surface-2: #F4F6F5;
  --color-ink: #20272A;
  --color-muted: #6A7570;
  --color-line: #DCE2DF;
  --color-primary: #2C6E63;
  --color-primary-soft: #DDEBE7;
  --color-accent: #E3A03A;
  --color-caution: #B07A24;
  --color-caution-soft: #F1E7D2;
  --font-sans: "Pretendard Variable", Pretendard, system-ui, sans-serif;
  --font-mono: "IBM Plex Mono", monospace;
}

@layer base {
  /* Aliases matching mockup CSS var names exactly */
  :root {
    --bg: var(--color-bg);
    --surface: var(--color-surface);
    --surface-2: var(--color-surface-2);
    --ink: var(--color-ink);
    --muted: var(--color-muted);
    --line: var(--color-line);
    --primary: var(--color-primary);
    --primary-soft: var(--color-primary-soft);
    --accent: var(--color-accent);
    --caution: var(--color-caution);
    --caution-soft: var(--color-caution-soft);
    --ui: var(--font-sans);
    --mono: var(--font-mono);
    --r: 12px;
    --r-sm: 8px;
    --shadow: 0 1px 2px rgba(0,0,0,.04), 0 4px 16px rgba(20,40,35,.05);
  }

  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

  body {
    font-family: var(--ui);
    background: var(--bg);
    color: var(--ink);
    height: 100vh;
    display: flex;
    flex-direction: column;
    -webkit-font-smoothing: antialiased;
  }

  #root {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
}

@layer components {
  /* topbar */
  .topbar {
    height: 56px; display: flex; align-items: center; gap: 16px;
    padding: 0 18px; background: var(--surface);
    border-bottom: 1px solid var(--line); flex-shrink: 0;
  }
  .brand {
    display: flex; align-items: center; gap: 9px;
    font-weight: 800; letter-spacing: -.02em; font-size: 17px;
  }
  .brand .mark { width: 26px; height: 26px; }
  .folder-pill {
    margin-left: 6px; display: flex; align-items: center; gap: 8px;
    font-size: 13px; color: var(--muted); background: var(--surface-2);
    border: 1px solid var(--line); padding: 6px 11px;
    border-radius: 999px; cursor: pointer;
  }
  .folder-pill b {
    color: var(--ink); font-weight: 600;
    font-family: var(--mono); font-size: 12.5px;
  }
  .folder-pill:hover { filter: brightness(.97); }
  .spacer { flex: 1; }
  .icon-btn {
    width: 34px; height: 34px; display: grid; place-items: center;
    border-radius: 9px; border: 1px solid transparent; color: var(--muted); cursor: pointer;
  }
  .icon-btn:hover { background: var(--surface-2); }

  /* layout */
  .shell { flex: 1; display: flex; min-height: 0; }
  .sidebar {
    width: 268px; background: var(--surface);
    border-right: 1px solid var(--line);
    display: flex; flex-direction: column; min-height: 0;
  }
  .side-head {
    padding: 14px 16px 8px; font-size: 12px;
    font-weight: 600; color: var(--muted); letter-spacing: .02em;
  }
  .tree { flex: 1; overflow: auto; padding: 0 8px; }
  .node {
    display: flex; align-items: center; gap: 8px;
    padding: 7px 8px; border-radius: var(--r-sm);
    font-size: 13.5px; cursor: pointer; color: var(--ink);
    user-select: none;
  }
  .node:hover { background: var(--surface-2); }
  .node .tw { width: 14px; color: var(--muted); font-size: 11px; text-align: center; flex-shrink: 0; }
  .node .ic { width: 16px; text-align: center; opacity: .85; flex-shrink: 0; }
  .node .nm { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .node .sz { font-size: 11.5px; color: var(--muted); font-family: var(--mono); flex-shrink: 0; }
  .side-foot { padding: 10px 12px; border-top: 1px solid var(--line); }
  .add-btn {
    width: 100%; display: flex; align-items: center; justify-content: center; gap: 7px;
    font-size: 13px; font-weight: 600; color: var(--primary);
    background: var(--primary-soft); border: none;
    border-radius: var(--r-sm); padding: 9px; cursor: pointer;
    font-family: var(--ui);
  }
  .add-btn:hover { filter: brightness(.97); }

  /* main / chat */
  .main {
    flex: 1; display: flex; flex-direction: column; min-width: 0; position: relative;
  }
  .chat {
    flex: 1; overflow: auto; padding: 26px 28px;
    display: flex; flex-direction: column; gap: 16px;
  }
  .composer {
    padding: 14px 28px 20px;
    border-top: 1px solid var(--line); background: var(--bg);
  }
  .input-bar {
    display: flex; align-items: center; gap: 10px;
    background: var(--surface); border: 1px solid var(--line);
    border-radius: 14px; padding: 11px 14px; box-shadow: var(--shadow);
  }
  .input-bar input {
    flex: 1; border: none; outline: none;
    font-family: var(--ui); font-size: 14.5px;
    background: transparent; color: var(--ink);
  }
  .input-bar input::placeholder { color: var(--muted); }
  .input-bar input:disabled { opacity: .5; cursor: not-allowed; }
  .send-btn {
    width: 34px; height: 34px; border: none; border-radius: 9px;
    background: var(--primary); color: #fff;
    cursor: pointer; display: grid; place-items: center;
  }
  .send-btn:disabled { opacity: .4; cursor: not-allowed; }

  /* floating dog */
  .dog {
    position: absolute; right: 24px; bottom: 96px;
    width: 64px; height: 64px;
    filter: drop-shadow(0 6px 10px rgba(20,40,35,.18));
    animation: bob 3.4s ease-in-out infinite;
  }
  .dog .state {
    position: absolute; top: -8px; right: -6px;
    background: var(--surface); border: 1px solid var(--line);
    font-size: 10.5px; font-weight: 600; color: var(--muted);
    padding: 2px 7px; border-radius: 999px;
  }
  @keyframes bob {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-7px); }
  }
  @media (prefers-reduced-motion: reduce) { .dog { animation: none; } }

  /* scanning indicator */
  .scan-hint {
    padding: 20px 16px; color: var(--muted);
    font-size: 13.5px; text-align: center;
  }
}
```

- [ ] **Step 3: Verify frontend build**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run build 2>&1 | tail -8
```
Expected: exits 0, CSS bundle grows (Tailwind + custom CSS).

---

### Task 6: `FileNode` Type + `TreeNode` Component

**Files:**
- Create: `src/types.ts`
- Create: `src/components/TreeNode.tsx`

**Interfaces:**
- Produces:
  - `FileNode` interface exported from `src/types.ts`
  - `TreeNode` component: `({ node: FileNode, depth: number }) => JSX.Element`

- [ ] **Step 1: Create `src/types.ts`**

```typescript
export interface FileNode {
  name: string;
  path: string;
  is_dir: boolean;
  size?: number;
  ext?: string;
  children: FileNode[];
}
```

- [ ] **Step 2: Create `src/components/TreeNode.tsx`**

```typescript
import { useState } from "react";
import { FileNode } from "../types";

function getIcon(node: FileNode): string {
  if (node.is_dir) return "📁";
  const ext = (node.ext ?? "").toLowerCase();
  if (["pdf", "hwpx", "hwp", "docx", "doc"].includes(ext)) return "📄";
  if (["png", "jpg", "jpeg", "gif", "webp", "heic", "svg"].includes(ext)) return "🖼";
  if (["dmg", "pkg", "exe", "zip", "tar", "gz", "7z"].includes(ext)) return "📦";
  if (["txt", "md", "rtf"].includes(ext)) return "🗒";
  if (["mp4", "mov", "avi", "mkv", "m4v"].includes(ext)) return "🎬";
  if (["mp3", "wav", "aac", "flac", "m4a"].includes(ext)) return "🎵";
  return "📄";
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)}K`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)}M`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)}G`;
}

interface Props {
  node: FileNode;
  depth: number;
}

export function TreeNode({ node, depth }: Props) {
  const [isOpen, setIsOpen] = useState(depth === 0);

  // Each depth level adds 18px indent; base padding is 8px (from .node)
  const extraIndent = depth * 18;

  const toggle = node.is_dir ? (isOpen ? "▾" : "▸") : "";
  const sizeLabel = node.is_dir
    ? String(node.children.length)
    : node.size !== undefined
    ? formatSize(node.size)
    : "";

  return (
    <>
      <div
        className="node"
        style={{ paddingLeft: `${8 + extraIndent}px` }}
        onClick={() => node.is_dir && setIsOpen((o) => !o)}
      >
        <span className="tw">{toggle}</span>
        <span className="ic">{getIcon(node)}</span>
        <span className="nm">{node.name}</span>
        <span className="sz">{sizeLabel}</span>
      </div>
      {node.is_dir && isOpen &&
        node.children.map((child) => (
          <TreeNode key={child.path} node={child} depth={depth + 1} />
        ))}
    </>
  );
}
```

- [ ] **Step 3: Verify TypeScript compilation**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run build 2>&1 | grep -E "error|Error|Finished|✓"
```
Expected: no TypeScript errors, `✓ built in`.

---

### Task 7: App Shell (Topbar, DogMascot, App layout)

**Files:**
- Create: `src/components/Topbar.tsx`
- Create: `src/components/DogMascot.tsx`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes:
  - `open` from `@tauri-apps/plugin-dialog` — `open({ directory: true, multiple: false }) => Promise<string | null>`
  - `invoke<FileNode>('scan_directory', { root: string, maxDepth: number })` from `@tauri-apps/api/core`
  - `TreeNode` from `./components/TreeNode`
  - `FileNode` from `./types`
- Produces: complete running app shell; DoD 1–3 visually verifiable

- [ ] **Step 1: Create `src/components/Topbar.tsx`**

```typescript
interface Props {
  folderPath: string | null;
  onFolderSelect: () => void;
}

// Inline dog SVG — same paths as mockup_main_shell.html <symbol id="dog">
function DogMark() {
  return (
    <svg className="mark" viewBox="0 0 64 64">
      <ellipse cx="32" cy="49" rx="17" ry="12" fill="#E3A03A"/>
      <circle cx="32" cy="29" r="16" fill="#E9A845"/>
      <path d="M17 19 Q11 27 17 35 Q24 31 24 22 Z" fill="#C8842B"/>
      <path d="M47 19 Q53 27 47 35 Q40 31 40 22 Z" fill="#C8842B"/>
      <ellipse cx="32" cy="34" rx="9" ry="7" fill="#F6DCA9"/>
      <circle cx="26.5" cy="27" r="2.2" fill="#2A211A"/>
      <circle cx="37.5" cy="27" r="2.2" fill="#2A211A"/>
      <ellipse cx="32" cy="32" rx="2.6" ry="2" fill="#2A211A"/>
      <path d="M32 34 Q32 37 29.5 37 M32 34 Q32 37 34.5 37"
        stroke="#2A211A" strokeWidth="1.2" fill="none" strokeLinecap="round"/>
    </svg>
  );
}

export function Topbar({ folderPath, onFolderSelect }: Props) {
  const displayPath = folderPath
    ? folderPath.replace(/^\/Users\/[^/]+/, "~")
    : null;

  return (
    <div className="topbar">
      <div className="brand">
        <DogMark />
        TidyDog
      </div>
      {displayPath ? (
        <div className="folder-pill" onClick={onFolderSelect}>
          정리 대상 <b>{displayPath}</b> ▾
        </div>
      ) : (
        <div className="folder-pill" onClick={onFolderSelect}>
          폴더 선택 ▾
        </div>
      )}
      <div className="spacer" />
      <div className="icon-btn" title="설정">⚙</div>
    </div>
  );
}
```

- [ ] **Step 2: Create `src/components/DogMascot.tsx`**

```typescript
export function DogMascot({ state = "대기 중" }: { state?: string }) {
  return (
    <div className="dog">
      <span className="state">{state}</span>
      <svg viewBox="0 0 64 64">
        <ellipse cx="32" cy="49" rx="17" ry="12" fill="#E3A03A"/>
        <circle cx="32" cy="29" r="16" fill="#E9A845"/>
        <path d="M17 19 Q11 27 17 35 Q24 31 24 22 Z" fill="#C8842B"/>
        <path d="M47 19 Q53 27 47 35 Q40 31 40 22 Z" fill="#C8842B"/>
        <ellipse cx="32" cy="34" rx="9" ry="7" fill="#F6DCA9"/>
        <circle cx="26.5" cy="27" r="2.2" fill="#2A211A"/>
        <circle cx="37.5" cy="27" r="2.2" fill="#2A211A"/>
        <ellipse cx="32" cy="32" rx="2.6" ry="2" fill="#2A211A"/>
        <path d="M32 34 Q32 37 29.5 37 M32 34 Q32 37 34.5 37"
          stroke="#2A211A" strokeWidth="1.2" fill="none" strokeLinecap="round"/>
      </svg>
    </div>
  );
}
```

- [ ] **Step 3: Replace `src/App.tsx`**

```typescript
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FileNode } from "./types";
import { TreeNode } from "./components/TreeNode";
import { Topbar } from "./components/Topbar";
import { DogMascot } from "./components/DogMascot";

export default function App() {
  const [folderPath, setFolderPath] = useState<string | null>(null);
  const [tree, setTree] = useState<FileNode | null>(null);
  const [scanning, setScanning] = useState(false);

  async function selectAndScan() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setFolderPath(selected);
    setScanning(true);
    try {
      const result = await invoke<FileNode>("scan_directory", {
        root: selected,
        maxDepth: 10,
      });
      setTree(result);
    } catch (err) {
      console.error("scan_directory failed:", err);
    } finally {
      setScanning(false);
    }
  }

  return (
    <>
      <Topbar folderPath={folderPath} onFolderSelect={selectAndScan} />
      <div className="shell">
        <aside className="sidebar">
          <div className="side-head">폴더 트리</div>
          <div className="tree">
            {scanning && <div className="scan-hint">스캔 중…</div>}
            {!scanning && !tree && (
              <div className="scan-hint">폴더를 선택하면 트리를 표시합니다.</div>
            )}
            {!scanning && tree && <TreeNode node={tree} depth={0} />}
          </div>
          <div className="side-foot">
            <button className="add-btn" onClick={selectAndScan}>
              ＋ 폴더 추가
            </button>
          </div>
        </aside>

        <main className="main">
          <div className="chat">
            {!folderPath && (
              <div className="scan-hint" style={{ marginTop: "40px" }}>
                왼쪽에서 폴더를 선택하면 TidyDog이 분석을 시작합니다.
              </div>
            )}
          </div>
          <div className="composer">
            <div className="input-bar">
              <input
                placeholder="무엇을 정리할까요? (Phase 2에서 활성화됩니다)"
                disabled
              />
              <button className="send-btn" disabled>↑</button>
            </div>
          </div>
          <DogMascot state={scanning ? "생각 중" : "대기 중"} />
        </main>
      </div>
    </>
  );
}
```

- [ ] **Step 4: Verify full frontend build**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run build 2>&1 | tail -10
```
Expected: exits 0, no TypeScript errors.

---

### Task 8: Final DoD Verification

**Files:** none modified

- [ ] **Step 1: Run the app**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run tauri dev
```

- [ ] **Step 2: Verify DoD 1 — OS folder picker opens**

Click the "폴더 선택 ▾" pill or "＋ 폴더 추가". macOS folder picker dialog must appear.

- [ ] **Step 3: Verify DoD 2 — Scan populates tree**

Select a folder (e.g. `~/Downloads`). Tree must render with real file/folder names, toggleable directories, and size labels.

- [ ] **Step 4: Verify DoD 3 — React tree renders correctly**

- Directory nodes show ▾/▸ toggle, click toggles expand/collapse.
- File nodes show appropriate emoji icon, name, and human-readable size (K/M/G).
- Fonts: UI text is Pretendard, size/path data is IBM Plex Mono.
- Colors match design tokens: primary (#2C6E63), bg (#E9EDEB), no red anywhere.
- Dog mascot bobs in lower-right.

- [ ] **Step 5: Verify DoD 4 — DB written, no duplicate rows**

After scanning, check the DB:
```bash
sqlite3 ~/Library/Application\ Support/com.mjwoon.tidydog/index.db \
  "SELECT id, ext, size, read_level FROM files LIMIT 10;"
```
Expected: rows with `read_level='metadata'` and real file data.

Then scan the same folder again (click folder-pill → select same folder). Row count must not grow:
```bash
sqlite3 ~/Library/Application\ Support/com.mjwoon.tidydog/index.db \
  "SELECT COUNT(*) FROM files;"
```
Expected: same count as before (upsert, not insert).

- [ ] **Step 6: Report**

List all created/modified files and paste 3+ sample rows from `SELECT id, current_path, ext, size, content_hash FROM files LIMIT 5;`
