# TidyDog Phase 0: Walking Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up an end-to-end Tauri 2 + React/TypeScript + Rust skeleton where clicking a button invokes a Rust `health` command and displays "TidyDog core vX.Y.Z · ready" on screen, with Tailwind v4 styling and a Cargo workspace linking `tidydog-core`.

**Architecture:** Tauri 2 desktop app with React/TS frontend (Vite) and Rust backend. Cargo workspace at repo root holds `src-tauri` (Tauri binary crate) and `crates/tidydog-core` (pure Rust library); `src-tauri` takes `tidydog-core` as a path dependency and calls `core_version()` from the `health` Tauri command.

**Tech Stack:** Tauri 2, Rust (edition 2021), React 19, TypeScript 5.8, Vite 7, Tailwind CSS v4 (`@tailwindcss/vite` plugin), `@tauri-apps/api` v2

## Global Constraints

- Workspace root `Cargo.toml` MUST have `resolver = "2"`.
- Tailwind v4 only: `@import "tailwindcss"` in CSS. No `@tailwind` directives, no PostCSS, no `tailwind.config.js`.
- `invoke` must import from `@tauri-apps/api/core` (NOT `@tauri-apps/api/tauri`).
- `vite.config.ts` Tauri server settings (`clearScreen`, `server.port 1420`, `strictPort`, `host`, `hmr`, `watch.ignored`) must never be deleted or modified.
- No file I/O, networking, LLM, or DB in this phase.

## Current State (as of 2026-06-29)

Scaffolding (spec Step 1) is **already done**: Tauri 2 + React/TS template is installed at `/Users/mjwoon/Workspace/tidydog/` with `node_modules/` present. Remaining work: Steps 2–6.

## File Map

| File | Action | Responsibility |
|---|---|---|
| `Cargo.toml` | **Create** | Workspace root — lists members, sets resolver |
| `crates/tidydog-core/Cargo.toml` | **Create** | Library crate metadata |
| `crates/tidydog-core/src/lib.rs` | **Create** | `core_version()` function |
| `src-tauri/Cargo.toml` | **Modify** | Add `tidydog-core` path dep |
| `src-tauri/src/lib.rs` | **Modify** | Replace `greet` with `health` command |
| `vite.config.ts` | **Modify** | Add Tailwind v4 plugin (preserve all Tauri settings) |
| `src/styles.css` | **Create** | Global CSS entry — `@import "tailwindcss"` |
| `src/main.tsx` | **Modify** | Import `./styles.css` |
| `src/App.tsx` | **Modify** | Minimal health-check UI with Tailwind classes |

---

### Task 1: Baseline Verification

**Files:**
- Read-only: `src-tauri/src/lib.rs`, `vite.config.ts`

**Interfaces:**
- Produces: confirmed clean build baseline before any modifications

- [ ] **Step 1: Verify Rust compiles**

```bash
cd /Users/mjwoon/Workspace/tidydog && cargo check --manifest-path src-tauri/Cargo.toml
```
Expected: no errors (warnings about unused code are OK).

- [ ] **Step 2: Verify frontend build compiles**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run build
```
Expected: exits 0, `dist/` directory created.

---

### Task 2: Tailwind v4 Setup

**Files:**
- Modify: `vite.config.ts`
- Create: `src/styles.css`
- Modify: `src/main.tsx`

**Interfaces:**
- Consumes: working Vite + React build from Task 1
- Produces: Tailwind utility classes are applied at runtime; `npm run build` still passes

- [ ] **Step 1: Install Tailwind v4 packages**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm install tailwindcss @tailwindcss/vite
```
Expected: exits 0, `tailwindcss` and `@tailwindcss/vite` appear in `node_modules/`.

- [ ] **Step 2: Add Tailwind plugin to vite.config.ts**

Replace the entire contents of `vite.config.ts` with the following (preserving all Tauri settings):

```typescript
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
```

- [ ] **Step 3: Create global CSS file**

Create `src/styles.css` with exactly:

```css
@import "tailwindcss";
```

- [ ] **Step 4: Import styles.css from main.tsx**

Replace `src/main.tsx`:

```typescript
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

- [ ] **Step 5: Verify build still passes**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run build
```
Expected: exits 0. If TypeScript reports missing `@tailwindcss/vite` types, that is acceptable at this stage.

---

### Task 3: Cargo Workspace Configuration

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/tidydog-core/Cargo.toml`
- Create: `crates/tidydog-core/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: existing `src-tauri/Cargo.toml`
- Produces: `cargo check` passes from workspace root; `tidydog_core::core_version()` is callable from `src-tauri`

- [ ] **Step 1: Create workspace root Cargo.toml**

Create `/Users/mjwoon/Workspace/tidydog/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["src-tauri", "crates/tidydog-core"]
```

- [ ] **Step 2: Create tidydog-core crate directory and Cargo.toml**

```bash
mkdir -p /Users/mjwoon/Workspace/tidydog/crates/tidydog-core/src
```

Create `crates/tidydog-core/Cargo.toml`:

```toml
[package]
name = "tidydog-core"
version = "0.1.0"
edition = "2021"

[lib]
name = "tidydog_core"
```

- [ ] **Step 3: Create tidydog-core/src/lib.rs**

Create `crates/tidydog-core/src/lib.rs`:

```rust
pub fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
```

- [ ] **Step 4: Add tidydog-core dependency to src-tauri/Cargo.toml**

Open `src-tauri/Cargo.toml` and add the following line inside `[dependencies]`:

```toml
tidydog-core = { path = "../crates/tidydog-core" }
```

The full `[dependencies]` section should now read:

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tidydog-core = { path = "../crates/tidydog-core" }
```

- [ ] **Step 5: Verify workspace compiles**

```bash
cd /Users/mjwoon/Workspace/tidydog && cargo check
```
Expected: `Checking tidydog-core v0.1.0` and `Checking tidydog v0.1.0` both appear, exits 0.

---

### Task 4: Health Command Wire-up

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `tidydog_core::core_version() -> &'static str` (from Task 3)
- Produces: Tauri command `health` returning `"TidyDog core v0.1.0 · ready"`; `greet` command removed

- [ ] **Step 1: Replace src-tauri/src/lib.rs**

Replace the entire file:

```rust
#[tauri::command]
fn health() -> String {
    format!("TidyDog core v{} · ready", tidydog_core::core_version())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![health])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 2: Verify Rust compiles**

```bash
cd /Users/mjwoon/Workspace/tidydog && cargo check
```
Expected: exits 0, no errors.

---

### Task 5: Frontend Roundtrip

**Files:**
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: Tauri command `health` (returns `string`) from Task 4
- Produces: minimal UI that calls `invoke<string>("health")` on button click and renders the result with Tailwind classes

- [ ] **Step 1: Replace src/App.tsx**

```typescript
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [status, setStatus] = useState<string>("");

  async function checkHealth() {
    const result = await invoke<string>("health");
    setStatus(result);
  }

  return (
    <main className="flex flex-col items-center justify-center min-h-screen bg-gray-50 gap-6">
      <h1 className="text-3xl font-bold text-gray-800">TidyDog</h1>
      <button
        onClick={checkHealth}
        className="px-6 py-3 bg-blue-600 text-white rounded-lg font-semibold hover:bg-blue-700 transition-colors"
      >
        Check Health
      </button>
      {status && (
        <p className="text-green-700 font-mono bg-green-50 px-4 py-2 rounded border border-green-200">
          {status}
        </p>
      )}
    </main>
  );
}

export default App;
```

- [ ] **Step 2: Verify TypeScript + frontend build**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run build
```
Expected: exits 0, no TypeScript errors.

---

### Task 6: Final DoD Verification

**Files:** none modified

- [ ] **Step 1: Run the app and verify all 4 DoD criteria**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run tauri dev
```

Verify visually:
1. Desktop window opens (DoD 1)
2. Click "Check Health" → text "TidyDog core v0.1.0 · ready" appears (DoD 2 — FE↔Rust roundtrip)
3. Button is blue, title is large and bold (DoD 3 — Tailwind classes applied)
4. `cargo check` earlier showed both `tidydog-core` and `tidydog` workspace members (DoD 4 — Cargo workspace)

- [ ] **Step 2: Report files created/modified**

After verification, report:
- Created: `Cargo.toml`, `crates/tidydog-core/Cargo.toml`, `crates/tidydog-core/src/lib.rs`, `src/styles.css`, `docs/superpowers/plans/2026-06-29-tidydog-phase0.md`
- Modified: `vite.config.ts`, `src/main.tsx`, `src/App.tsx`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`
