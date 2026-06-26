# AGENTS.md

Agent guide for the **Code Profile Sync** repo. The design and current status live in
[`PLAN.md`](./PLAN.md) — read it before non-trivial work and keep it in sync when the
design changes. This file only covers how to work in the repo and the things that will
bite you; it links to PLAN.md rather than restating it.

## What this is

A Rust CLI (GUI later) that syncs **settings + extensions** across the profiles of a
VS Code OSS–based editor against a declarative TOML config. Editors are **discovered by
binary and identified via `product.json`**, not by config directory — see
[PLAN.md §0](./PLAN.md). Test targets are **Code - OSS** and **VSCodium** (both on the
dev machine; VS Code/Cursor are not installed). For the full feature status and what's
left, see the "Implementation status" and "Remaining work / roadmap" sections of PLAN.md.

## Build / check

```sh
cargo build
cargo clippy --all-targets
cargo fmt
cargo test
```

Toolchain is pinned in `rust-toolchain.toml` (don't bump without reason).

## Code style — non-negotiable

- Follow the user-global **`rust-code-style`** skill.
- Lints are strict (see `Cargo.toml`): `unwrap_used`, `expect_used`, `todo`,
  `unimplemented`, `indexing_slicing`, `arithmetic_side_effects`, `as_conversions`,
  `print_stdout`/`print_stderr` all warn; `unsafe_code` is **denied**.
- Errors use **`anyhow`** with `?`/`.context(...)`; arithmetic is checked
  (`saturating_add`, `try_from`); all user-facing output goes through `ui.rs` (the only
  place the `print_*` lints are scoped). To silence a lint, prefer
  `#[expect(lint, reason = "…")]` on the smallest scope (the repo requires a reason).
- Match the surrounding code's idioms, naming, and comment density.

## Domain landmines (get these wrong and you corrupt a user's editor)

Each links to the authoritative explanation in PLAN.md.

- **Editor must be closed for writes** — it owns `storage.json`/`extensions.json` and
  overwrites on exit. Gate on a running process; allow `--force`. See
  [PLAN.md §3.4](./PLAN.md).
- **`useDefaultFlags`** — a profile may inherit a resource from Default; never write a
  profile-local file for an inherited one. [PLAN.md §1.4](./PLAN.md).
- **Extensions = shared install + per-profile membership** — adds are tiered (pool →
  vendored copy → editor CLI); removals edit only the membership list; never delete shared
  folders; refuse removing from Default (its list *is* the shared pool). VSIX-source
  extensions are vendored. [PLAN.md §1.2 and §4](./PLAN.md).
- **Shared pool collision** — Code - OSS and VSCodium share `~/.vscode-oss/extensions`; one
  extensions dir ≠ one editor (matters for `gc`). [PLAN.md §1.2](./PLAN.md).
- **The Default profile** lives at `User/` root, not `User/profiles/`, and is configured
  under `[default]`, never `[profiles.Default]`. [PLAN.md §1.1 and §2](./PLAN.md).
- **Settings files are JSONC** — parse tolerantly; nulls are stripped (TOML has no null).
  [PLAN.md §5](./PLAN.md).
- **Always write atomically** (temp + rename), back up before the first write, and honor
  `--dry-run`.

## Conventions

- Use the **`git-conventions`** skill for commits; **`agent-attribution`** for any
  user-visible content/commits.
- Project paths contain spaces — quote them; never `cd` to CWD; never backslash-escape.
- AI files (`AGENTS.md`, `CLAUDE.md`) are globally git-ignored; force-add to commit them.
