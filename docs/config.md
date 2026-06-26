# Config reference

The editor config is a declarative TOML file describing the **desired state** of one
editor's profiles. There is one self-contained file per editor (e.g. `vscodium.toml`),
selected with `--editor`/`--config`. This document is the authoritative reference; it
stands alone and needs no editor tooling.

A machine-readable [JSON Schema](../schema/config.schema.json) is published alongside it.
Generated configs carry a first-line directive so TOML-aware editors (Taplo / the "Even
Better TOML" extension) offer completion, hover docs, and validation:

```toml
#:schema https://raw.githubusercontent.com/viell-dev/code-profile-manager/main/schema/config.schema.json
```

In-editor validation is best-effort: it fetches the schema over the network and tracks
the `main` branch, so it may run ahead of an older installed binary. The binary itself
always validates structurally via `serde` on load — the schema only adds authoring
ergonomics.

## Layout

```toml
[editor]                 # which editor this config targets (optional overrides)
[global]                 # settings + extensions applied to every profile
[groups.<name>]          # reusable bundles a profile can include
[default]                # the built-in Default profile
[profiles.<name>]        # a named profile
```

An effective per-profile state is computed by layering these (see
[Merge precedence](#merge-precedence)).

## `[editor]`

How the config refers to / overrides the target editor. All fields optional.

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Editor name to match during discovery — `product.json` `nameShort` or `applicationName`, e.g. `"VSCodium"` or `"Code - OSS"`. |
| `binary` | path | Explicit launcher path, bypassing `PATH` discovery. |
| `user_dir` | path | Override for the editor's `User/` directory (wrapper script, `--user-data-dir`, or portable install). |
| `extensions_dir` | path | Override for the shared extensions directory. Note: Code - OSS and VSCodium share `~/.vscode-oss/extensions` by default. |

```toml
[editor]
name = "VSCodium"
# binary = "/usr/bin/codium"
# user_dir = "…"
# extensions_dir = "…"
```

## `[global]` and `[groups.<name>]`

Both are **layers**: a reusable bundle of settings and extensions.

| Field | Type | Description |
|-------|------|-------------|
| `settings` | table | VS Code settings keys → values (see [Settings](#settings)). |
| `extensions` | array | Extension IDs (see [Extensions](#extensions)). |

`[global]` applies to **every** managed profile. A `[groups.<name>]` applies only to
profiles that list it in their `groups`. Groups model "common across most" without
repeating items in each profile.

```toml
[global]
extensions = ["editorconfig.editorconfig", "usernamehw.errorlens"]
[global.settings]
"editor.formatOnSave" = true

[groups.web]
extensions = ["dbaeumer.vscode-eslint", "esbenp.prettier-vscode"]
[groups.web.settings]
"editor.defaultFormatter" = "esbenp.prettier-vscode"
```

## `[default]`

The built-in **Default** profile. It is always present and cannot be renamed. It takes
the same fields as a named profile **except** `icon` and `use_default` (it is the profile
others inherit *from*, so inheritance flags don't apply).

| Field | Type | Description |
|-------|------|-------------|
| `groups` | array | Group names this profile includes. |
| `settings` | table | Profile-specific settings (highest precedence). |
| `extensions` | array | Profile-specific extensions. |
| `exclude_extensions` | array | Extension IDs to drop even if a group/global adds them. |

The Default profile is configured **here**, never under `[profiles.*]`. A config
containing `[profiles.Default]` is rejected on load.

```toml
[default]
extensions = ["brunnerh.insert-unicode"]
```

## `[profiles.<name>]`

A named profile. Use a quoted key for names with spaces (e.g. `[profiles."TaqsWeb V2"]`).

| Field | Type | Description |
|-------|------|-------------|
| `icon` | string | Codicon ID used as the profile icon (e.g. `"package"`, `"briefcase"`). See the [codicon reference](https://microsoft.github.io/vscode-codicons/dist/codicon.html). |
| `groups` | array | Group names to include, applied in listed order. |
| `settings` | table | Profile-specific settings (highest precedence). |
| `extensions` | array | Profile-specific extensions. |
| `exclude_extensions` | array | Extension IDs to drop even if a group/global adds them. |
| `use_default` | table | Resources inherited from Default (see [`use_default`](#use_default)). |

```toml
[profiles.Rust]
icon = "package"
extensions = ["rust-lang.rust-analyzer", "tamasfe.even-better-toml"]
use_default = { keybindings = true }

[profiles."TaqsWeb V2"]
icon = "briefcase"
groups = ["web"]
exclude_extensions = ["usernamehw.errorlens"]
use_default = { keybindings = true }
```

## Settings

A `settings` table maps dotted VS Code setting names to any JSON-compatible value:

```toml
[global.settings]
"editor.formatOnSave" = true
"files.trimTrailingWhitespace" = true
```

`null` is **not** supported — TOML has no null type, and any `null` is stripped on write,
so a setting explicitly set to `null` is not managed.

## Extensions

Extension IDs take the form `publisher.name`, optionally pinned as
`publisher.name@version`. Pins are parsed but **not** enforced for membership in v1
(newest compatible is acceptable; the resolved version is recorded in the snapshot so it
doesn't churn). The ID must match:

```text
^[a-z0-9][a-z0-9-]*\.[a-z0-9][a-z0-9-]*(@.+)?$
```

Local **VSIX-source** extensions need no special config field: they are vendored
automatically into the app home's `vendor/extensions/` on pull/sync and restored on push,
so the config stays portable across machines without a marketplace.

## `use_default`

Records, per profile, which resource types are **inherited from the Default profile**
instead of being profile-local (VS Code's `useDefaultFlags`). A `true` value means the
editor reads the Default profile's copy and the tool writes no profile-local file for that
resource.

```toml
use_default = { keybindings = true }
```

Recognized keys (VS Code's `ProfileResourceType`):

`settings`, `keybindings`, `snippets`, `tasks`, `extensions`, `globalState`, `mcp`,
`prompts`, `languageModels`.

`[default]` has no `use_default` (nothing to inherit from).

## Merge precedence

The effective desired state for a profile is computed deterministically:

- **settings** — `[global].settings` → each included `[groups.*].settings` (in listed
  order) → the profile's / `[default]`'s `settings`. Later wins, per **top-level key**
  (the value is replaced wholesale; recursive object deep-merge is future work).
- **extensions** — `union(global, groups…, profile)` minus `exclude_extensions`. A set of
  IDs; versions are ignored for membership.
- **use_default** — taken from the profile itself, not inherited from global/groups.
- A profile that inherits a resource (`use_default.<resource> = true`) ignores the
  resolved settings/extensions for that resource.

**Consolidation** is the inverse refactor (run at `init` and from the interactive menu):
items shared across all profiles are hoisted into `[global]` while keeping the resolved
per-profile state identical.
