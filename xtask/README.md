# WordCraft xtask

`cargo xtask --help` retains the local task descriptions and appends the shared
syntax for compatible commands. `craft-xtask` is an xtask-only dependency from
`cloudbring/craft-libs` (`algo/integration`); repoint it to
`storytold/craft-libs` after upstream merge.

## Shared tasks

- **version:** `craft_xtask::version::run` with default policy on the workspace
  root (byte-preserving `[workspace.package]` version edits, staging under
  `target/`, offline lockfile refresh with online fallback, plus `tag`).
- **stats:** `craft_xtask::stats::run` with `follow_symlinks = true` (bounded,
  cycle-safe traversal excluding `target`, `fuzz`, and dot directories, plus
  `--exact` harness counts).
- **ico:** `craft_xtask::ico::run` with default config (square PNGs up to
  256 px packed into an `.ico` at caller-relative paths).
- **layers:** `craft_xtask::layers::compute` with WordCraft's L0–L6 table,
  standalone `wordcraft-control-key`, dev-only `wordcraft-testkit`, intra-layer
  `wordcraft-fonts -> wordcraft-doc`, exempt apps/tooling, and UI-toolkit rules,
  formatted into WordCraft's table and summary report.
- **parity:** `craft_xtask::parity::compute` and `apply` in command-output mode
  (`cargo run -q -p wordcraft-cli -- parity --markdown`), printing WordCraft's
  headline summary line and written checklist path (and supporting `--check`).

## Local tasks

- **assets** (`src/assets.rs`): checks that every asset file is listed in
  `ATTRIBUTION.md`.
- **visual-diff** (`src/visual_diff.rs`): compares two PDFs page by page via
  Poppler (`pdftoppm`, `pdftotext`).
- **wasm, test, i18n-report, language, ci, corpus**: WordCraft build, audit,
  and corpus orchestration in `src/main.rs`.
