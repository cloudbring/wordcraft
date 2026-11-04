# craft-ooxml reader swap: old-vs-new oracle (#122, artcraft-algorithms#184)

Claim under test: replacing wordcraft's hand-rolled OPC reader (`crates/docx/src/package.rs`) with
`craft_ooxml::opc::Package::open_with(.., &Tolerance::lenient())` preserves behaviour on real files.

- OLD: `wordcraft-docx` at the merge-base `bff0d1d9250a34e74d38f53bfe17195edb2da66b` (pre-swap upstream main).
- NEW: this branch, craft-ooxml and craft-archive pinned at craft-libs `7d46713` (backslash-name
  normalisation, DOCTYPE-tolerant content types that never expand entities, exact-case-first lookup;
  `Tolerance::lenient()` turns them on).
- Harness: `tools/ooxml-oracle` (its own cargo workspace, so the old crate's name does not make `-p`
  ambiguous), craft-oracle and craft-compare at craft-libs `7d46713`.

```text
WORDCRAFT_OFFICE_CORPUS=/path/to/corpus cargo test --release \
  --manifest-path tools/ooxml-oracle/Cargo.toml -- --ignored --nocapture
```

## What is compared, per file

1. Open outcome (ok / error), error text for both-fail pairs.
2. The import warnings (the `log::warn!` lines), in order.
3. The parsed `Document`: each top-level field as serde JSON, plus the whole `Document`'s Debug text
   (which also shows fields serde skips: passthrough parts, media index, caches).
4. Open then `write()`: the part set, part order, each XML part by canonical XML (craft-compare), and
   every other part byte for byte.

A file is Identical only if all four agree to the byte.

## Corpus and result

2,656 files (2,635 .docx, 21 .docm; no .dotx file is present), read-only, never copied into the repo.

| Corpus | Files | Identical | Expected difference | Both fail | Regression |
| --- | ---: | ---: | ---: | ---: | ---: |
| office-poi | 120 | 114 | 0 | 6 | 0 |
| office-openxml-sdk | 238 | 238 | 0 | 0 | 0 |
| office-libreoffice | 2,298 | 2,281 | 3 | 14 | 0 |
| **Total** | **2,656** | **2,633** | **3** | **20** | **0** |

OnlyOldFails 0, OnlyNewFails 0, Timeout 0, Panic 0. For the 2,633 identical files the Document, its
Debug text, the warnings, and the saved package (including bytes) are the same on both sides.

Unlike gridcraft and deckcraft there were no strict-reader failures to clear: no file opened only on
OLD or only on NEW (the old docx reader already accepted these names and DOCTYPEs).

### Both fail (20)

Every one is also a failure before the swap. 13 have identical messages (password-protected: 3;
`missing part: word/document.xml`: 7; malformed XML / entity / nesting limit: 3). 7 differ in text only:

- 6 not-a-zip files (truncated, encrypted CFB containers): `invalid Zip archive: Could not find EOCD`
  is now prefixed `ZIP error: `.
- `ofz9934-1.docx`: OLD said `Could not find EOCD`, NEW says `duplicate ZIP entry names`, which is
  the more exact cause. Still an error in both.

### Registered differences (3, expected.toml)

All three differ in the warnings only; the Document, Debug text and saved package are identical.

| File | Difference | Cause |
| --- | --- | --- |
| `ooxml/fail/LIBREOFFICE-DBDAZAEC.docx` | wording of the 9 unreadable-entry warnings; two `part ... missing` warnings spell `activex` instead of `activeX` | Fuzzed package with bad CRCs. Same entries skipped, same order; craft-ooxml words the warning differently. The file's local-header names disagree in case with its central directory (`unzip -p` reports "mismatching local filename"); the new reader spells the part by the rels path it resolved. The part is missing on both sides. |
| `uiwriter/ofz18563.docx` | wording as above, plus one new warning `missing [Content_Types].xml: using empty content types` | `[Content_Types].xml` is itself a corrupt deflate stream. Same fallback as before, now reported. |
| `ooxml/pass/tdf93284.docx` | two new warnings `duplicate relationship id rId5 / rId6 skipped` | The rels part repeats two ids. First definition wins on both sides; the new reader says so. |

## Regressions

None found, so no source fix and no red-then-green test was needed beyond step 0.

## Step 0

`crates/docx/Cargo.toml` re-pinned from 4d7f516 to 7d46713 (craft-ooxml and craft-archive;
`Cargo.lock` follows). `cargo test -p wordcraft-docx`: 136 unit + 385 integration tests pass.

## Not covered by the corpus

No corpus file exercises: backslash entry names or a DOCTYPE in `[Content_Types].xml` in a docx
(covered by the craft-libs tests and the gridcraft real files), percent-encoded internal targets, or
a .dotx. The corpus gives no evidence for those.
