use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_xtask::{Cargo, ico, layers, parity, stats, version};
use serde_json::{Value, json};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wordcraft-xtask-golden-{}-{}", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, rel: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn xtask_cmd(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask")).args(args).current_dir(cwd).output().unwrap()
}

fn assert_output(output: Output, expected: &[u8]) {
    assert!(output.status.success(), "stdout:\n{}\nstderr:\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, expected);
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}

fn layers_config(root: &Path) -> layers::Config {
    layers::Config {
        root: root.into(),
        layers: [
            ("L0", names(&["wordcraft-geom", "wordcraft-control-key"])),
            ("L1", names(&["wordcraft-doc", "wordcraft-fonts", "wordcraft-metafile"])),
            ("L2", names(&["wordcraft-layout", "wordcraft-docx", "wordcraft-docbin", "wordcraft-formats"])),
            ("L3", names(&["wordcraft-render", "wordcraft-pdf"])),
            ("L4", names(&["wordcraft-engine"])),
            ("L5", names(&["wordcraft-mcp", "wordcraft-zotero"])),
            ("L6", names(&["wordcraft-ui-egui", "wordcraft-testkit"])),
        ]
        .into_iter()
        .map(|(label, members)| (label.into(), members))
        .collect(),
        exempt: names(&["wordcraft", "wordcraft-cli", "wordcraft-web", "xtask"]),
        standalone: names(&["wordcraft-control-key"]),
        testkits: names(&["wordcraft-testkit"]),
        allowed_edges: vec![("wordcraft-fonts".into(), "wordcraft-doc".into())],
        external_rules: vec![layers::ExternalRule {
            dependencies: names(&["egui", "eframe", "winit", "egui_kittest", "rfd", "bevy*"]),
            allowed_from: names(&["wordcraft-ui-egui", "wordcraft-testkit"]),
            allowed_dev_from: Vec::new(),
        }],
        ..layers::Config::default()
    }
}

fn describe_layer<'a>(cfg: &'a layers::Config, name: &str) -> &'a str {
    if cfg.exempt.iter().any(|m| m == name) {
        "exempt"
    } else if cfg.standalone.iter().any(|m| m == name) {
        "L0 standalone"
    } else if cfg.testkits.iter().any(|m| m == name) {
        "testkit"
    } else {
        cfg.layers.iter().find(|(_, members)| members.iter().any(|m| m == name)).map(|(label, _)| label.as_str()).unwrap_or("UNREGISTERED")
    }
}

fn format_layers_report(cfg: &layers::Config, report: &layers::Report) -> String {
    let mut out = String::from("Dependency layering (plan/architecture.md §3)\n\n");
    out.push_str(&format!("{:<28} {:<14} workspace deps\n", "crate", "layer"));
    for c in &report.crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", d.name.strip_prefix("wordcraft-").unwrap_or(&d.name))
            })
            .collect();
        out.push_str(&format!("{:<28} {:<14} {}\n", c.name, describe_layer(cfg, &c.name), ws.join(", ")));
    }
    out.push('\n');
    if report.ok {
        out.push_str(&format!("OK: {} crates, no layering violations.\n", report.crates.len()));
    } else {
        out.push_str(&format!("{} violation(s):\n", report.violations.len()));
        for v in &report.violations {
            out.push_str(&format!("  - {v}\n"));
        }
    }
    out
}

fn golden_metadata() -> Value {
    let mut meta: Value = serde_json::from_str(include_str!("golden/metadata.json")).unwrap();
    if meta["workspace_members"].is_null() {
        let mut members = Vec::new();
        for pkg in meta["packages"].as_array_mut().unwrap() {
            let name = pkg["name"].as_str().unwrap().to_owned();
            pkg["id"] = Value::String(name.clone());
            members.push(Value::String(name));
        }
        meta["workspace_members"] = Value::Array(members);
    }
    meta
}

#[test]
fn help_keeps_local_text_and_shared_syntax() {
    let output = xtask_cmd(&["--help"], &repo_root());
    assert!(output.status.success());
    let suffix = output.stdout.strip_prefix(include_bytes!("golden/help.stdout")).expect("help output must start with golden/help.stdout");
    let expected = format!(
        "\nShared task syntax:\n  {}\n  {}\n  {}\n  {}\n  {}\n",
        version::cli_help(),
        stats::cli_help(),
        ico::cli_help(),
        layers::cli_help(),
        parity::cli_help(),
    );
    assert_eq!(suffix, expected.as_bytes());
}

#[test]
fn version_show_and_dry_run_plan_match_golden() {
    assert_output(xtask_cmd(&["version"], &repo_root()), include_bytes!("golden/version.stdout"));

    let temp = Temp::new();
    temp.write("Cargo.toml", include_bytes!("golden/version.input.toml"));
    let mut cfg =
        version::Config { cargo: Cargo { root: temp.0.clone(), ..Cargo::default() }, refresh_lockfile: false, ..version::Config::default() };

    let show_plan = version::compute(&cfg, &version::Args::Show).unwrap();
    assert_eq!(version::render(&show_plan).as_bytes(), include_bytes!("golden/version.stdout"));

    let set_args = version::Args::parse(&["set".into(), "1.2.3-rc.1".into()]).unwrap();
    let set_plan = version::compute(&cfg, &set_args).unwrap();
    assert_eq!(version::render(&set_plan).as_bytes(), include_bytes!("golden/version-set.stdout"));
    assert_eq!(set_plan.edits.len(), 1);
    assert_eq!(set_plan.edits[0].2.as_bytes(), include_bytes!("golden/version.updated.toml"));
    // Dry-run compute must not modify Cargo.toml on disk before apply.
    assert_eq!(std::fs::read(temp.0.join("Cargo.toml")).unwrap(), include_bytes!("golden/version.input.toml"));

    version::apply(&set_plan).unwrap();
    assert_eq!(std::fs::read(temp.0.join("Cargo.toml")).unwrap(), include_bytes!("golden/version.updated.toml"));

    let crlf = include_str!("golden/version.input.toml").replace('\n', "\r\n");
    assert_eq!(version::rewrite(&cfg, &crlf, "1.2.3-rc.1").unwrap(), include_str!("golden/version.updated.toml").replace('\n', "\r\n"));
    for ok in ["0.1.0", "1.20.300", "1.0.0-rc.1", "1.0.0-alpha", "1.0.0-x-y.2"] {
        assert!(version::validate(ok).is_ok(), "{ok}");
    }
    for bad in ["1.0", "1.0.0.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "1.0.0+meta", "1.a.0", ""] {
        assert!(version::validate(bad).is_err(), "{bad}");
    }
    cfg.refresh_lockfile = true;
    assert!(cfg.online_fallback && !cfg.strict_prerelease && !cfg.bounded_core && !cfg.rollback_on_failure);
}

#[test]
fn stats_replays_golden_counts_and_live_workspace_matches() {
    let temp = Temp::new();
    let rows: Value = serde_json::from_str(include_str!("golden/stats-counts.json")).unwrap();
    let mut packages = Vec::new();
    for row in rows.as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let counts = &row["counts"];
        let files = counts["files"].as_u64().unwrap() as usize;
        let lines = counts["lines"].as_u64().unwrap() as usize;
        let tests = counts["tests"].as_u64().unwrap() as usize;
        let proptests = counts["proptests"].as_u64().unwrap() as usize;
        let mut src = "#[test]\n".repeat(tests);
        src += &"proptest! {}\n".repeat(proptests);
        src += &"\n".repeat(lines - tests - proptests);
        temp.write(format!("{name}/src/0.rs"), src);
        for idx in 1..files {
            temp.write(format!("{name}/src/{idx}.rs"), "");
        }
        packages.push(json!({ "name": name, "manifest_path": format!("{name}/Cargo.toml") }));
    }
    let cfg = stats::Config { cargo: Cargo { root: temp.0.clone(), ..Cargo::default() }, follow_symlinks: true, ..stats::Config::default() };
    let report = stats::from_metadata(&cfg, &json!({ "packages": packages }), &stats::Args::default()).unwrap();
    assert_eq!(serde_json::to_value(&report.rows).unwrap(), rows);
    assert_eq!(stats::render(&report).as_bytes(), include_bytes!("golden/stats.stdout"));

    let live = xtask_cmd(&["stats"], &repo_root());
    assert!(live.status.success(), "{}", String::from_utf8_lossy(&live.stderr));
    let unchanged =
        |text: &str| text.lines().filter(|line| !line.starts_with("xtask ") && !line.starts_with("TOTAL ")).collect::<Vec<_>>().join("\n");
    assert_eq!(unchanged(&String::from_utf8(live.stdout).unwrap()), unchanged(include_str!("golden/stats.stdout")));

    let excl = Temp::new();
    excl.write("src/lib.rs", "// #[test] commented\n#[test]\nfn a() {}\nproptest! {\n}\n");
    for ignored in ["target", "fuzz", ".hidden"] {
        excl.write(format!("{ignored}/ignored.rs"), "#[test]\nfn ignored() {}\n");
    }
    assert_eq!(stats::count(&cfg, &excl.0).unwrap(), stats::Counts { files: 1, lines: 5, tests: 1, proptests: 1 });
}

#[test]
fn ico_stdout_and_bytes_match_golden_and_committed_icon_is_valid() {
    let temp = Temp::new();
    let inputs = [
        ("16.png", include_bytes!("golden/icon-16.bin")),
        ("32.png", include_bytes!("golden/icon-32.bin")),
        ("48.png", include_bytes!("golden/icon-48.bin")),
        ("128.png", include_bytes!("golden/icon-128.bin")),
        ("256.png", include_bytes!("golden/icon-256.bin")),
    ];
    for (name, bytes) in inputs {
        temp.write(name, bytes);
    }
    assert_output(xtask_cmd(&["ico", "icon.ico", "16.png", "32.png", "48.png", "128.png", "256.png"], &temp.0), include_bytes!("golden/ico.stdout"));
    assert_eq!(std::fs::read(temp.0.join("icon.ico")).unwrap(), include_bytes!("golden/icon.ico"));

    assert!(ico::pack(&[]).is_err());
    assert!(ico::pack(&[b"GIF89a".to_vec()]).is_err());
    assert_eq!(ico::png_size(include_bytes!("golden/icon-48.bin")).unwrap(), (48, 48));

    let committed = repo_root().join("assets/app-icon/wordcraft.ico");
    if let Ok(bytes) = std::fs::read(&committed) {
        assert_eq!(&bytes[..4], &[0, 0, 1, 0]);
        let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        assert!(count >= 4, "expected several sizes in {}", committed.display());
    }
}

#[test]
fn layers_report_matches_golden_metadata_and_live_workspace() {
    let cfg = layers_config(&repo_root());
    let crates = layers::from_metadata(&golden_metadata()).unwrap();
    let report = layers::check_graph(&cfg, &crates).unwrap();
    assert!(report.ok, "{:?}", report.violations);
    assert_eq!(format_layers_report(&cfg, &report).as_bytes(), include_bytes!("golden/layers.stdout"));

    assert_output(xtask_cmd(&["layers"], &repo_root()), include_bytes!("golden/layers.stdout"));

    let wasm: Vec<_> = crates
        .iter()
        .filter(|c| cfg.layers.iter().take(6).any(|(_, m)| m.contains(&c.name)) || c.name == "wordcraft-ui-egui")
        .map(|c| c.name.as_str())
        .collect();
    assert!(wasm.contains(&"wordcraft-control-key"));
    assert!(wasm.contains(&"wordcraft-engine"));
    assert!(wasm.contains(&"wordcraft-ui-egui"));
    assert!(!wasm.contains(&"wordcraft"));
    assert!(!wasm.contains(&"wordcraft-cli"));
    assert!(!wasm.contains(&"wordcraft-web"));
    assert!(!wasm.contains(&"xtask"));
}

#[test]
fn layers_policy_enforces_wordcraft_rules() {
    let cfg = layers_config(&repo_root());
    let edge = |from: &str, to: &str, kind: layers::DepKind, workspace: bool| layers::Crate {
        name: from.into(),
        deps: vec![layers::Dep { name: to.into(), kind, workspace }],
    };
    use layers::DepKind::*;

    // Allowed edges: downward, intra-layer fonts -> doc, self dev-dep, testkit dev-dep, testkit -> L4, apps/xtask exempt.
    for valid in [
        edge("wordcraft-doc", "wordcraft-geom", Normal, true),
        edge("wordcraft-fonts", "wordcraft-doc", Normal, true),
        edge("wordcraft-doc", "wordcraft-doc", Dev, true),
        edge("wordcraft-render", "wordcraft-testkit", Dev, true),
        edge("wordcraft-testkit", "wordcraft-engine", Normal, true),
        edge("wordcraft-ui-egui", "egui", Normal, false),
        edge("wordcraft", "egui", Normal, false),
        edge("wordcraft-cli", "wordcraft-ui-egui", Normal, true),
        edge("wordcraft-web", "wordcraft-ui-egui", Normal, true),
        edge("xtask", "wordcraft-ui-egui", Normal, true),
    ] {
        assert!(layers::check(&cfg, std::slice::from_ref(&valid)).unwrap().is_empty(), "{valid:?}");
    }

    // Forbidden edges: upward, sideways, reverse intra-layer doc -> fonts, standalone workspace dep,
    // testkit normal dep, testkit -> L6 ui-egui, UI crates below L6, unregistered crate.
    for invalid in [
        edge("wordcraft-doc", "wordcraft-engine", Normal, true),
        edge("wordcraft-geom", "wordcraft-doc", Dev, true),
        edge("wordcraft-layout", "wordcraft-docx", Normal, true),
        edge("wordcraft-doc", "wordcraft-fonts", Normal, true),
        edge("wordcraft-control-key", "wordcraft-geom", Normal, true),
        edge("wordcraft-render", "wordcraft-testkit", Normal, true),
        edge("wordcraft-testkit", "wordcraft-ui-egui", Normal, true),
        edge("wordcraft-engine", "egui", Normal, false),
        edge("wordcraft-mcp", "winit", Normal, false),
        edge("wordcraft-doc", "bevy_ecs", Normal, false),
        layers::Crate { name: "wordcraft-mystery".into(), deps: Vec::new() },
    ] {
        assert!(!layers::check(&cfg, std::slice::from_ref(&invalid)).unwrap().is_empty(), "{invalid:?}");
    }
}

#[cfg(unix)]
#[test]
fn parity_matches_golden_markdown_and_stdout() {
    use std::os::unix::fs::PermissionsExt;

    let temp = Temp::new();
    temp.write("xtask/Cargo.toml", "[package]\nname = \"xtask\"\nversion = \"0.1.0\"\n");
    std::fs::create_dir_all(temp.0.join("docs")).unwrap();
    temp.write("generated.md", include_bytes!("golden/parity.md"));
    let stub = temp.write("cargo-stub.sh", "#!/bin/sh\ncat generated.md\n");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("parity")
        .current_dir(&temp.0)
        .env("CARGO", &stub)
        .env("CARGO_MANIFEST_DIR", temp.0.join("xtask"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let normalized = String::from_utf8(output.stdout).unwrap().replace(&temp.0.display().to_string(), "ROOT");
    assert_eq!(normalized.as_bytes(), include_bytes!("golden/parity.stdout"));
    assert_eq!(std::fs::read(temp.0.join("docs/parity-checklist.md")).unwrap(), include_bytes!("golden/parity.md"));

    // `--check` passes when fresh and fails without overwriting when stale.
    let check_ok = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["parity", "--check"])
        .current_dir(&temp.0)
        .env("CARGO", &stub)
        .env("CARGO_MANIFEST_DIR", temp.0.join("xtask"))
        .output()
        .unwrap();
    assert!(check_ok.status.success(), "{}", String::from_utf8_lossy(&check_ok.stderr));

    temp.write("docs/parity-checklist.md", b"stale\n");
    let check_stale = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["parity", "--check"])
        .current_dir(&temp.0)
        .env("CARGO", &stub)
        .env("CARGO_MANIFEST_DIR", temp.0.join("xtask"))
        .output()
        .unwrap();
    assert!(!check_stale.status.success());
    assert_eq!(std::fs::read(temp.0.join("docs/parity-checklist.md")).unwrap(), b"stale\n");
}
