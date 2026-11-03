# Legacy xtask golden fixtures

Captured on 2026-11-03 from WordCraft `b47079fc` (`main`) before deleting the
duplicated task modules. All fixtures are first-party outputs or synthetic test
inputs under the 200 KB budget.

- `help.stdout`: original `cargo xtask --help` output.
- `layers.stdout`, `metadata.json`: original `cargo xtask layers` output and
  reduced workspace metadata graph.
- `stats.stdout`, `stats-counts.json`: original `cargo xtask stats` table and
  per-crate counts.
- `version.stdout`, `version-set.stdout`, `version.input.toml`,
  `version.updated.toml`: original workspace version output and manifest
  rewrite to `1.2.3-rc.1`.
- `ico.stdout`, `icon-16.bin`, `icon-32.bin`, `icon-48.bin`, `icon-128.bin`,
  `icon-256.bin`, `icon.ico`: synthetic square PNG IHDR payloads and packed
  ICO output/stdout.
- `parity.md`, `parity.stdout`: generated `docs/parity-checklist.md` and
  normalized command stdout (`ROOT/docs/parity-checklist.md`).
