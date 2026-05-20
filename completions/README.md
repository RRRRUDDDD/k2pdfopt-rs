# k2pdfopt-rs shell completions

This directory holds shell completion scripts auto-generated from the `k2cli::CliArgs` clap
definition by `tools/gen_assets` (Step 10.2 of the rewrite plan).

**Do not edit the generated files by hand.** Regenerate after any CLI flag change with:

```sh
cargo run --release -p gen-assets
```

In CI / pre-commit, the same tool's `--check` mode aborts when the on-disk artifacts drift
from the current `CliArgs` definition:

```sh
cargo run --release -p gen-assets -- --check
```

## Recommended scope (Step 11.12 / v0.2 P1-8)

`gen-assets` ships completions for **all five shells** below. Rationale (Open Question 10.2.B
resolved at Step 11.12):

- **Maintenance is free**: artifacts are auto-generated, no manual upkeep cost.
- **`--check` mode guards drift**: any CLI flag change without regenerating fails CI.
- **`gen-assets` already covers** bash / zsh / fish / powershell / elvish via clap-generated
  completion targets (one source of truth → five emitted files).
- **No platform bias**: ship all five, let the user pick their shell.

For each shell, **per-user install is the default** (no sudo, no PATH pollution); the
system-wide variant is offered as a one-off snippet for multi-user machines (CI runners,
shared servers).

## Install (per shell)

### Bash

```sh
# system-wide
sudo cp completions/bash/k2pdfopt.bash /etc/bash_completion.d/k2pdfopt

# or per-user
mkdir -p ~/.local/share/bash-completion/completions
cp completions/bash/k2pdfopt.bash ~/.local/share/bash-completion/completions/k2pdfopt
```

Then open a new shell.

### Zsh

```sh
# add to fpath, e.g.
mkdir -p ~/.zfunc
cp completions/zsh/_k2pdfopt ~/.zfunc/
# in ~/.zshrc
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

### Fish

```sh
mkdir -p ~/.config/fish/completions
cp completions/fish/k2pdfopt.fish ~/.config/fish/completions/
```

### PowerShell (Windows / cross-platform)

```powershell
# Per-session
. completions/powershell/k2pdfopt.ps1

# Persistent: source it from $PROFILE
Add-Content -Path $PROFILE -Value '. "<repo>/completions/powershell/k2pdfopt.ps1"'
```

### Elvish

```sh
mkdir -p ~/.config/elvish/lib
cp completions/elvish/k2pdfopt.elv ~/.config/elvish/lib/
# in ~/.config/elvish/rc.elv
use k2pdfopt
```

## See also

- `docs/k2pdfopt-rs.1` — Roff man page (also produced by `gen-assets`).
- `docs/migration-from-c.md` — v2.55 ↔ Rust CLI compatibility matrix (Step 10.1).
- `docs/compat-matrix.md` — v0.2.0 vs C v2.55 parameter matrix (Step 11.12).
- `../.rewrite-log.md` — Step 11.12 entry resolves Open Question 10.2.B (5-shell scope).
