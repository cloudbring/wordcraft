//! Old-vs-new differential for the craft-ooxml reader swap (#122), run over a real-file corpus
//! (cloudbring/artcraft-algorithms#184, evidence standard: research/VERIFICATION.md section 1.1).
//!
//! OLD is this crate at the swap's merge-base (`docx_old`, the `package =` rename in
//! Cargo.toml); NEW is the workspace crate. Per file, each side reports:
//! - whether the file opens (an open error is the side's `Err`, so craft-oracle classifies
//!   both-fail / only-old-fails / only-new-fails),
//! - the import warnings, in order,
//! - a canonical semantic dump of the Document: each top-level field as serde JSON, plus its Debug
//!   unmodelled content the model keeps for saving (`preserved`), which serde skips,
//! - open -> save: every part of the written package, in order, with its bytes.
//!
//! The judge compares the open result, warnings, each semantic section, the saved part list and
//! order, each part's canonical XML (craft-compare) and then its bytes. Intended differences live
//! in `expected.toml`; anything else is a Regression.
//!
//! ```text
//! WORDCRAFT_OFFICE_CORPUS=/path/to/corpus cargo test --release --manifest-path tools/ooxml-oracle/Cargo.toml -- --ignored --nocapture
//! ```
//!
//! (the corpus root holds the `office-*` trees). Output (report.json,
//! report.md, details.txt) goes to `WORDCRAFT_ORACLE_OUT` (default `target/ooxml-oracle`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use craft_compare::Verdict;
use craft_compare::xml::compare_xml;
use craft_oracle::{Differential, FnJudge, FnSide, Outcome, Policy, Registry};

type Parts = Vec<(String, Vec<u8>)>;

/// What one side produced for one file.
#[derive(Clone, Debug, PartialEq)]
struct Dump {
    warnings: Vec<String>,
    /// Section name -> canonical text.
    sections: BTreeMap<String, String>,
    /// The saved package (part name, bytes) in written order; `Err` when saving failed.
    saved: Result<Parts, String>,
}

thread_local! {
    static LINES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// One logger for both sides (the `log` facade has one global logger): it collects the warnings
/// logged on the calling thread, which is where a side runs.
struct Capture;
impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            LINES.with(|l| l.borrow_mut().push(r.args().to_string()));
        }
    }
    fn flush(&self) {}
}
static CAPTURE: Capture = Capture;

fn install_capture() {
    let _ = log::set_logger(&CAPTURE);
    log::set_max_level(log::LevelFilter::Warn);
    LINES.with(|l| l.borrow_mut().clear());
}

/// The side implementation, instantiated once per revision of the crate: only the crate name
/// differs, so both dumps are built by the same code.
macro_rules! side {
    ($name:ident, $docx:ident) => {
        mod $name {
            use super::{BTreeMap, Dump, parts_of};
            use super::{LINES, install_capture};

            fn put(sections: &mut BTreeMap<String, String>, key: String, v: &serde_json::Value) {
                sections.insert(key, serde_json::to_string(v).unwrap_or_else(|e| format!("<unserializable: {e}>")));
            }

            pub fn run(bytes: &[u8]) -> Result<Dump, String> {
                install_capture();
                let doc = $docx::read(bytes).map_err(|e| e.to_string())?;
                let mut sections = BTreeMap::new();
                match serde_json::to_value(&doc) {
                    Ok(json) => {
                        if let Some(obj) = json.as_object() {
                            for (k, v) in obj {
                                put(&mut sections, format!("doc.{k}"), v);
                            }
                        } else {
                            put(&mut sections, "doc".into(), &json);
                        }
                    }
                    Err(e) => {
                        sections.insert("doc.serde_error".into(), e.to_string());
                    }
                }
                // Debug also shows what serde skips (passthrough parts, caches).
                sections.insert("doc.debug".into(), format!("{doc:?}"));
                let saved = $docx::write(&doc).map_err(|e| e.to_string()).and_then(|b| parts_of(&b));
                let warnings = LINES.with(|l| std::mem::take(&mut *l.borrow_mut()));
                Ok(Dump { warnings, sections, saved })
            }
        }
    };
}

side!(old_side, docx_old);
side!(new_side, wordcraft_docx);

fn parts_of(bytes: &[u8]) -> Result<Parts, String> {
    let bad = |e: &dyn std::fmt::Display| format!("saved package unreadable: {e}");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| bad(&e))?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| bad(&e))?;
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| bad(&e))?;
        out.push((f.name().to_string(), data));
    }
    Ok(out)
}

/// Section name with the sheet index folded away, so a metric names the kind of difference.
fn section_kind(key: &str) -> String {
    let (head, tail) = key.split_once('.').unwrap_or((key, ""));
    let head = head.split('[').next().unwrap_or(head);
    format!("{head}.{tail}")
}

fn is_xml(name: &str) -> bool {
    [".xml", ".rels", ".vml"].iter().any(|e| name.ends_with(e))
}

/// Every difference between two dumps as `(metric, detail)`; empty when they agree to the byte.
/// Metrics: `save_result`, `warnings`, `sem:<section kind>`, `pkg_parts`, `pkg_order`, `pkg_xml`,
/// `pkg_bytes` (canonical XML equal, bytes differ), `pkg_bin`.
fn differences(old: &Dump, new: &Dump) -> Vec<(String, String)> {
    let mut d = Vec::new();
    if old.warnings != new.warnings {
        d.push(("warnings".to_string(), format!("old {:?}\n    new {:?}", old.warnings, new.warnings)));
    }
    let keys: BTreeSet<_> = old.sections.keys().chain(new.sections.keys()).collect();
    for k in keys {
        let (a, b) = (old.sections.get(k), new.sections.get(k));
        if a != b {
            let clip = |s: Option<&String>| s.map(|s| s.chars().take(300).collect::<String>()).unwrap_or_else(|| "<absent>".into());
            d.push((format!("sem:{}", section_kind(k)), format!("{k}\n    old {}\n    new {}", clip(a), clip(b))));
        }
    }
    match (&old.saved, &new.saved) {
        (Ok(a), Ok(b)) => package_differences(a, b, &mut d),
        (Err(a), Err(b)) if a == b => {}
        (a, b) => d.push(("save_result".into(), format!("old {:?}; new {:?}", a.as_ref().map(Vec::len), b.as_ref().map(Vec::len)))),
    }
    d
}

fn package_differences(a: &Parts, b: &Parts, d: &mut Vec<(String, String)>) {
    let names = |p: &Parts| p.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    let (na, nb) = (names(a), names(b));
    let (sa, sb): (BTreeSet<_>, BTreeSet<_>) = (na.iter().collect(), nb.iter().collect());
    if sa != sb {
        d.push((
            "pkg_parts".into(),
            format!("only old {:?}; only new {:?}", sa.difference(&sb).collect::<Vec<_>>(), sb.difference(&sa).collect::<Vec<_>>()),
        ));
    } else if na != nb {
        d.push(("pkg_order".into(), format!("old {na:?}\n    new {nb:?}")));
    }
    let new_parts: BTreeMap<_, _> = b.iter().map(|(n, x)| (n.as_str(), x)).collect();
    for (name, x) in a {
        let Some(y) = new_parts.get(name.as_str()) else { continue };
        if x == *y {
            continue;
        }
        if is_xml(name) {
            let (tx, ty) = (String::from_utf8_lossy(x), String::from_utf8_lossy(y));
            match compare_xml(&tx, &ty) {
                Ok(None) => d.push(("pkg_bytes".into(), format!("{name}: canonical XML equal, bytes differ"))),
                Ok(Some(at)) => d.push(("pkg_xml".into(), format!("{name}: first XML difference at {at}"))),
                Err(e) => d.push(("pkg_xml".into(), format!("{name}: not comparable: {e:?}"))),
            }
        } else {
            d.push(("pkg_bin".into(), format!("{name}: {} vs {} bytes", x.len(), y.len())));
        }
    }
}

fn judge(old: &Dump, new: &Dump) -> Verdict {
    let d = differences(old, new);
    // Byte differences under equal canonical XML are tolerated (Within); anything else is the
    // first difference found.
    match d.iter().find(|(m, _)| m != "pkg_bytes") {
        Some((m, _)) => Verdict::Exceeds { metric: m.clone(), value: d.len() as f64, limit: 0.0 },
        None if d.is_empty() => Verdict::Identical,
        None => Verdict::Within { metric: "pkg_bytes".into(), value: d.len() as f64 },
    }
}

fn corpus_files(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with('.') || name.starts_with("~$") {
                continue;
            }
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["docx", "docm", "dotx"].iter().any(|x| e.eq_ignore_ascii_case(x))) {
                out.push(p);
            }
        }
    }
    let mut all = Vec::new();
    for tree in ["office-poi", "office-openxml-sdk", "office-libreoffice"] {
        walk(&root.join(tree), &mut all);
    }
    all.into_iter().map(|p| (p.strip_prefix(root).unwrap_or(&p).to_string_lossy().into_owned(), p)).collect()
}

#[test]
#[ignore = "needs WORDCRAFT_OFFICE_CORPUS (the corpus root holding office-poi, office-openxml-sdk, office-libreoffice)"]
fn old_and_new_readers_agree_on_the_office_corpus() {
    let Some(root) = std::env::var_os("WORDCRAFT_OFFICE_CORPUS") else {
        eprintln!("WORDCRAFT_OFFICE_CORPUS is not set; nothing to compare");
        return;
    };
    let root = PathBuf::from(root);
    let files = corpus_files(&root);
    assert!(!files.is_empty(), "no .docx/.docm/.dotx files under {}", root.display());
    let out = PathBuf::from(std::env::var_os("WORDCRAFT_ORACLE_OUT").unwrap_or_else(|| "target/ooxml-oracle".into()));
    std::fs::create_dir_all(&out).expect("create output dir");

    let registry = Registry::from_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("expected.toml")).expect("expected.toml");
    let policy = Policy::default();
    let diff = Differential::new(FnSide(old_side::run), FnSide(new_side::run), FnJudge(judge))
        .timeout(Duration::from_secs(300))
        .expected(registry)
        .policy(policy)
        .artifact_dir(out.join("failures"));
    let lazy = files.iter().map(|(n, p)| (n.clone(), std::fs::read(p).unwrap_or_default()));
    let report = diff.run_files(lazy);
    report.write(&out, &policy).expect("write report");

    // Detail pass: re-run both sides on every file that is not identical and list all differences
    // (the judge reports the first), and the both-fail pairs whose messages differ.
    let mut details = String::new();
    let (mut both_fail_same, mut both_fail_differ) = (0, 0);
    for r in &report.results {
        let path = &files.iter().find(|(n, _)| *n == r.name).expect("file").1;
        match &r.outcome {
            Outcome::Identical => {}
            Outcome::BothFail { old_err, new_err } => {
                if old_err == new_err {
                    both_fail_same += 1;
                } else {
                    both_fail_differ += 1;
                    let _ = writeln!(details, "{} [both fail, different messages]\n  old: {old_err}\n  new: {new_err}", r.name);
                }
            }
            o => {
                let bytes = std::fs::read(path).unwrap_or_default();
                let _ = writeln!(details, "{} [{}]", r.name, o.name());
                match (old_side::run(&bytes), new_side::run(&bytes)) {
                    (Ok(a), Ok(b)) => {
                        for (m, text) in differences(&a, &b) {
                            let _ = writeln!(details, "  {m}: {text}");
                        }
                    }
                    (a, b) => {
                        let _ = writeln!(details, "  old: {:?}\n  new: {:?}", a.map(|_| "ok"), b.map(|_| "ok"));
                    }
                }
            }
        }
    }
    let _ = writeln!(details, "\nboth-fail identical messages: {both_fail_same}; different messages: {both_fail_differ}");
    std::fs::write(out.join("details.txt"), &details).expect("write details");
    eprintln!("{}", report.to_markdown(&policy));
    eprintln!("files: {}; both-fail identical messages: {both_fail_same}; different messages: {both_fail_differ}", files.len());
    assert_eq!(report.exit_code(&policy), 0, "regressions: see {}", out.join("details.txt").display());
}
