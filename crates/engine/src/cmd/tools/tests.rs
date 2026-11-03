use super::*;

#[test]
fn compare_marks_changes() {
    let mut s = Session::new(Document::from_text("The cat sat.\nKeep this.\nOld line."));
    let r = s.run("review.compare", &json!({"text": "The dog sat.\nKeep this.\nNew line here."})).unwrap();
    assert!(r["changes"].as_u64().unwrap() >= 2);
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The dog sat.\nKeep this.\nNew line here.");
}

#[test]
fn compare_options_ignore_case_and_white_space() {
    let original = "The Cat sat.\nTwo  spaces here.";
    let revised = "The cat sat.\nTwo spaces here.";
    // By default both differences are changes.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "author": "Reviewer"})).unwrap();
    assert_eq!(r["changes"], 2);
    assert!(s.doc.revisions.iter().any(|x| x.author == "Reviewer"));
    // Ignoring case and white space: no changes, and the result reads as the revised text.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "caseChanges": false, "whiteSpace": false})).unwrap();
    assert_eq!(r["changes"], 0);
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
    // Character level marks only the changed letters.
    let mut s = Session::new(Document::from_text("colour"));
    s.run("review.compare", &json!({"text": "color", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "u");
    // Two dates that happen to share digits in a different order ("01.09.2026" vs
    // "19.10.2026") must come out as one clean replacement, not a scrambled chain of
    // single-character deletions and insertions wherever a digit coincidentally recurs.
    let mut s = Session::new(Document::from_text("zawarta w dniu 01.09.2026 pomiędzy:"));
    s.run("review.compare", &json!({"text": "zawarta w dniu 19.10.2026 pomiędzy:", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "01.09");
    assert_eq!(inserted, "19.10");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.del.is_some()).count(), 1, "one clean deleted run, not several scattered ones");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.ins.is_some()).count(), 1, "one clean inserted run, not several scattered ones");
    // Formatting differences become formatting changes.
    let mut s = Session::new(Document::from_text("Same words."));
    let mut rev = Document::from_text("Same words.");
    rev.format_range(&rev.start_of(StoryRef::Body), &rev.end_of(StoryRef::Body), &|c| c.bold = Some(true)).unwrap();
    let path = std::env::temp_dir().join(format!("wc-compare-{}.docx", std::process::id()));
    crate::io::save_path(&path, &rev).unwrap();
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(r["changes"], 1);
    assert!(s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format));
}

#[test]
fn collapse_noise_matches_merges_short_interior_runs_but_keeps_prefix_suffix() {
    // "01.09" vs "19.10": a pure LCS finds a scrambled chain of single-character matches
    // ('1', '.', '0') threaded through what a reader expects to be one clean replacement.
    let ops = lcs_ops(&['0', '1', '.', '0', '9'], &['1', '9', '.', '1', '0'], &mut { u64::MAX });
    assert!(ops.contains(&Op::Same), "the raw LCS does find spurious matches here, or this test proves nothing");
    assert_eq!(collapse_noise_matches(&ops), vec![Op::Del, Op::Del, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Ins, Op::Ins, Op::Ins]);
    // A plain single-character replacement flanked by a real common prefix and suffix has no
    // interior match to begin with (the DP's middle is pure delete/insert), so it's untouched.
    let ops = lcs_ops(&['a', 'b', 'c', 'x', 'd', 'e', 'f'], &['a', 'b', 'c', 'y', 'd', 'e', 'f'], &mut { u64::MAX });
    assert_eq!(collapse_noise_matches(&ops), ops);
}

#[test]
fn compare_at_word_level_still_matches_a_whole_unchanged_word() {
    // `collapse_noise_matches` only runs for character-level compares (wired up in
    // `word_diff`): a single matched *word* between two edits is a real, meaningful match
    // (unlike a single matched character), and must never be folded into a replacement.
    let mut s = Session::new(Document::from_text("the cat sat"));
    s.run("review.compare", &json!({"text": "a cat ran"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert!(
        p.run_ranges().any(|(r, c)| c.del.is_none() && c.ins.is_none() && p.text[r].trim() == "cat"),
        "the unchanged word must survive as plain text"
    );
}

#[test]
fn compare_keeps_text_boxes_of_the_original() {
    // Text boxes are anchored in body paragraphs: one after changed text, one alone in an
    // otherwise empty paragraph, one after text the revised document keeps. Their anchors must
    // survive the compare (marked deleted, since the revised text has no boxes), so no text is lost.
    for show_in in ["original", "revised", "new"] {
        let mut s = Session::new(Document::from_text("Body one.\n\nKeep."));
        let mut boxes = Vec::new();
        for (para, off, text) in [(0usize, "Body one.".len(), "Alpha boxword gamma."), (1, 0, "Vml onlyword here."), (2, "Keep.".len(), "Kept box.")]
        {
            s.run("caret.set", &json!({"pos": Pos::body(para, off)})).unwrap();
            let id = s.run("insert.textBox", &json!({"text": text})).unwrap()["story"].as_u64().unwrap() as u32;
            boxes.push((id, text));
        }
        s.run("caret.set", &json!({"pos": Pos::body(0, 0)})).unwrap();
        s.run("review.compare", &json!({"text": "Totally new first.\nSecond new line.\nKeep.", "showIn": show_in})).unwrap();
        for (id, text) in &boxes {
            let anchor = s.doc.text_box_anchor(*id).unwrap_or_else(|| panic!("{show_in}: the box \"{text}\" lost its anchor"));
            assert_eq!(s.doc.plain_text(StoryRef::Part(*id)), *text);
            let p = s.doc.para(anchor.story, &anchor.path).unwrap();
            assert!(
                p.props_of_char(anchor.off - wordcraft_doc::para::OBJ.len_utf8()).del.is_some(),
                "{show_in}: the box \"{text}\" is not marked deleted"
            );
        }
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Body one.\n\nKeep.");
        for (id, text) in &boxes {
            assert!(s.doc.text_box_anchor(*id).is_some(), "{show_in}: rejecting lost the box \"{text}\"");
            assert_eq!(s.doc.plain_text(StoryRef::Part(*id)), *text);
        }
    }
}

#[test]
fn compare_with_a_revised_file_keeps_its_text_boxes_backed() {
    // Text boxes in both documents, the revised one from a file: every anchor in the result
    // shows a text box story of the result, and both documents' boxes are there, marked.
    fn with_boxes(text: &str, boxes: &[(usize, usize, &str)]) -> Session {
        let mut s = Session::new(Document::from_text(text));
        for (para, off, text) in boxes {
            s.run("caret.set", &json!({"pos": Pos::body(*para, *off)})).unwrap();
            s.run("insert.textBox", &json!({"text": text})).unwrap();
        }
        s
    }
    /// Each text box anchored in the body: its story's text, and whether its anchor is deleted
    /// or inserted.
    fn boxes(d: &Document) -> Vec<(String, bool, bool)> {
        let mut out = Vec::new();
        for path in d.para_paths(StoryRef::Body) {
            let p = d.para(StoryRef::Body, &path).unwrap();
            for (off, o) in p.objects_with_offsets() {
                for id in o.text_boxes() {
                    let part = d.parts.get(&id).unwrap_or_else(|| panic!("an anchor shows story {id}, which the result lacks"));
                    assert_eq!(part.kind, wordcraft_doc::PartKind::TextBox, "story {id}");
                    let c = p.props_of_char(off);
                    out.push((d.plain_text(StoryRef::Part(id)), c.del.is_some(), c.ins.is_some()));
                }
            }
        }
        out.sort();
        out
    }
    let rev = with_boxes("Body one.\nNew middle.\nKeep.", &[(1, "New middle.".len(), "Revised box."), (2, "Keep.".len(), "Kept box.")]);
    let path = std::env::temp_dir().join(format!("wc-compare-boxes-{}.docx", std::process::id()));
    crate::io::save_path(&path, &rev.doc).unwrap();
    let t = |t: &str, del, ins| (t.to_string(), del, ins);
    for show_in in ["original", "revised", "new"] {
        for (then, left) in [("review.acceptAll", "Revised box."), ("review.rejectAll", "Original box.")] {
            let mut s = with_boxes("Body one.\nOld middle.\nKeep.", &[(1, "Old middle.".len(), "Original box."), (2, "Keep.".len(), "Kept box.")]);
            s.run("review.compare", &json!({"path": path.to_string_lossy(), "showIn": show_in})).unwrap();
            assert_eq!(boxes(&s.doc), [t("Kept box.", false, false), t("Original box.", true, false), t("Revised box.", false, true)], "{show_in}");
            s.run(then, &json!({})).unwrap();
            assert_eq!(boxes(&s.doc), [t("Kept box.", false, false), t(left, false, false)], "{show_in} {then}");
        }
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn compare_bounds_total_diff_work() {
    // Many long, slightly different paragraphs: each pair alone fits one comparison table,
    // but the work over all pairs must stay bounded. Pairs past the budget are compared
    // whole (deleted and inserted), and every pair still counts as a change.
    let words = 3000;
    let paras = 12;
    let doc = |rev: bool| -> String {
        (0..paras)
            .map(|p| (0..words).map(|w| if w % 10 == 0 && rev { format!("x{p}w{w}") } else { format!("p{p}w{w}") }).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (original, revised) = (doc(false), doc(true));
    let mut s = Session::new(Document::from_text(&original));
    let r = s.run("review.compare", &json!({"text": revised})).unwrap();
    assert_eq!(r["changes"], paras);
    // A pair diffed word by word keeps most of its text unmarked; one replaced whole keeps
    // only its common start and end.
    let word_diffed = |b: &Arc<Block>| {
        let p = b.as_para().unwrap();
        let same: usize = p.run_ranges().filter(|(_, c)| c.ins.is_none() && c.del.is_none()).map(|(r, _)| r.len()).sum();
        same * 2 > p.text.len()
    };
    assert!(word_diffed(&s.doc.body[0]));
    let diffed = s.doc.body.iter().filter(|b| word_diffed(b)).count() as u64;
    let cells = (words as u64) * (words as u64);
    assert!(diffed * cells <= 50_000_000 + cells, "{diffed} pairs diffed word by word");
    assert!(!word_diffed(s.doc.body.last().unwrap()));
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
}

/// #53: words marked inserted or deleted across the body — the diff-quality metric.
fn compare_changed_words(s: &Session) -> usize {
    let mut n = 0;
    for b in &s.doc.body {
        let Block::Para(p) = &**b else { continue };
        for (r, c) in p.run_ranges() {
            if c.ins.is_some() || c.del.is_some() {
                n += p.text[r].split_whitespace().count();
            }
        }
    }
    n
}

/// #53: two 10,000-paragraph documents over a 50-word vocabulary, 20 scattered single-word
/// changes. No paragraph is unique in either document, so there is nothing to patience-anchor
/// on; past the LCS cap the quadratic diff replaced the whole matching middle wholesale
/// (thousands of changed paragraphs), where the anchored diff keeps exactly the 20 changed
/// paragraphs and their <= 40 changed words.
#[test]
fn compare_repetitive_vocabulary_keeps_scattered_edits() {
    let vocab: Vec<String> = (0..50).map(|i| format!("term{i}")).collect();
    let old: Vec<String> = (0..10_000).map(|i| vocab[i % 50].clone()).collect();
    let mut new = old.clone();
    for k in 0..20 {
        let at = 199 + 487 * k;
        new[at] = vocab[(at + 7) % 50].clone();
    }
    let mut s = Session::new(Document::from_text(&old.join("\n")));
    let start = std::time::Instant::now();
    let r = s.run("review.compare", &json!({"text": new.join("\n")})).unwrap();
    eprintln!("10,000-paragraph pair: {} changes, {} changed words, {:?}", r["changes"], compare_changed_words(&s), start.elapsed());
    assert_eq!(r["changes"], 20, "the matching middle was replaced wholesale");
    assert!(compare_changed_words(&s) <= 40, "{} changed words", compare_changed_words(&s));
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), new.join("\n"));
}

/// #53: the anchored diff must not enlarge the diff on the existing fixtures: the changed
/// elements and changed words per fixture, as measured with the capped quadratic LCS, hold.
#[test]
fn compare_fixture_changed_word_counts_hold() {
    let run = |original: &str, revised: &str, level: &str| {
        let mut s = Session::new(Document::from_text(original));
        let r = s.run("review.compare", &json!({"text": revised, "level": level})).unwrap();
        (r["changes"].as_u64().unwrap(), compare_changed_words(&s))
    };
    let cases = [
        ("one word replaced", "The cat sat.", "The dog sat.", "word", 1, 2),
        ("a line replaced", "The cat sat.\nKeep this.\nOld line.", "The dog sat.\nKeep this.\nNew line here.", "word", 2, 7),
        ("deleted and inserted paragraphs", "alpha\nbeta\ngamma\ndelta", "alpha\ngamma\ndelta epsilon", "word", 2, 4),
        ("a whole line replaced", "a b c", "x y z", "word", 1, 6),
        ("dates at character level", "zawarta w dniu 01.09.2026 pomiedzy:", "zawarta w dniu 19.10.2026 pomiedzy:", "character", 1, 2),
        ("a matched word between edits", "the cat sat", "a cat ran", "word", 1, 4),
    ];
    for (name, original, revised, level, changes, words) in cases {
        let (got_changes, got_words) = run(original, revised, level);
        assert_eq!((got_changes, got_words), (changes, words), "{name}");
    }
    // The budget fixture of `compare_bounds_total_diff_work`: five pairs diffed word by word
    // (600 changed words each), the rest replaced whole with their common edges kept.
    let words = 3000;
    let paras = 12;
    let doc = |rev: bool| -> String {
        (0..paras)
            .map(|p| (0..words).map(|w| if w % 10 == 0 && rev { format!("x{p}w{w}") } else { format!("p{p}w{w}") }).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(run(&doc(false), &doc(true), "word"), (12, 44_874), "budget-limited pairs");
}

/// #53: the worst shapes at 50,000 paragraphs a side — repeated vocabulary with scattered
/// edits (histogram anchors, then Myers per gap), a block swap of repeated paragraphs, and two
/// documents with nothing in common — all finish bounded and fast, and no quadratic table is
/// built above the old cap: the only table-shaped allocation left is Myers' saved frontiers.
#[test]
fn compare_fifty_thousand_paragraph_pairs_stay_bounded() {
    PEAK_TABLE_CELLS.store(0, std::sync::atomic::Ordering::Relaxed);
    let start = std::time::Instant::now();
    let vocab: Vec<String> = (0..50).map(|i| format!("w{i}")).collect();
    let old: Vec<String> = (0..50_000).map(|i| vocab[i % 50].clone()).collect();
    let mut new = old.clone();
    for k in 0..100 {
        let at = 101 + 431 * k;
        new[at] = vocab[(at + 11) % 50].clone();
    }
    let mut s = Session::new(Document::from_text(&old.join("\n")));
    let r = s.run("review.compare", &json!({"text": new.join("\n")})).unwrap();
    eprintln!("50,000-paragraph pair: {} changes, {} changed words", r["changes"], compare_changed_words(&s));
    assert_eq!(r["changes"], 100, "the matching middle was replaced wholesale");
    assert!(compare_changed_words(&s) <= 200, "{} changed words", compare_changed_words(&s));

    let block = |first: &str, second: &str| (0..50_000).map(|i| if i < 25_000 { first } else { second }.to_string()).collect::<Vec<_>>().join("\n");
    let mut s = Session::new(Document::from_text(&block("alpha", "beta")));
    let r = s.run("review.compare", &json!({"text": block("beta", "alpha")})).unwrap();
    eprintln!("block swap: {} changes, {} changed words", r["changes"], compare_changed_words(&s));
    // A block swap of repeated paragraphs has no monotone match beyond one half.
    assert_eq!(compare_changed_words(&s), 50_000);

    let old: Vec<String> = (0..50_000).map(|i| format!("x{i}")).collect();
    let new: Vec<String> = (0..50_000).map(|i| format!("y{i}")).collect();
    let mut s = Session::new(Document::from_text(&old.join("\n")));
    let r = s.run("review.compare", &json!({"text": new.join("\n")})).unwrap();
    eprintln!("nothing in common: {} changes", r["changes"]);
    assert_eq!(r["changes"], 50_000);

    let elapsed = start.elapsed();
    let peak = PEAK_TABLE_CELLS.load(std::sync::atomic::Ordering::Relaxed);
    eprintln!("50,000-paragraph pairs: {elapsed:?}, peak table {peak} cells");
    assert!(elapsed < std::time::Duration::from_secs(30), "{elapsed:?}");
    // No O(n·m) table above the cap: Myers' frontiers for a segment of at most DIRECT
    // elements span at most DIRECT² + 3·DIRECT + 3 cells, under the old table cap of
    // MAX_LCS × MAX_LCS — where a 50,000 × 50,000 compare would want 2.5 billion.
    const _: () = assert!(DIRECT * DIRECT + 3 * DIRECT + 3 <= MAX_LCS * MAX_LCS);
    assert!(peak <= MAX_LCS as u64 * MAX_LCS as u64, "{peak} table cells");
}

#[test]
fn macros_record_and_run() {
    let mut s = Session::new(Document::new());
    s.run("tools.recordMacro", &json!({"name": "hi"})).unwrap();
    s.run("text.insert", &json!({"text": "Hi "})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("tools.recordMacro", &json!({})).unwrap();
    s.run("tools.macros", &json!({"run": "hi"})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hi Hi ");
    s.run("edit.repeat", &json!({})).unwrap();
}

#[test]
fn autocorrect_on_space() {
    let mut s = Session::new(Document::new());
    for t in ["teh", " ", "Teh", " ", "(c)", " "] {
        s.run("text.insert", &json!({"text": t})).unwrap();
    }
    // The first word of a sentence is capitalised too.
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The The © ");
}

#[test]
fn protection_blocks_edits() {
    let mut s = Session::new(Document::from_text("locked"));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    // The "none" mode is the shorthand for stopping the protection, and it doesn't stop a
    // protected document: only the explicit stop does (#1327 follow-up).
    assert!(s.run("review.restrict", &json!({"mode": "none"})).is_err());
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());
}

#[test]
fn accessibility_and_quick_parts() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = s.run("file.accessibility", &json!({})).unwrap();
    assert!(r["issues"].is_array());
    s.run("select.text", &json!({"text": "Membership"})).unwrap();
    s.run("insert.quickParts", &json!({"save": "m"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.quickParts", &json!({"insert": "m"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).ends_with("Membership"));
}

/// #407 Language: the selection's language and "do not check", the default language and
/// the detect preference; a bad tag is refused.
#[test]
fn proofing_language_options() {
    let mut s = Session::new(Document::from_text("Bonjour"));
    s.run("select.all", &json!({})).unwrap();
    s.run("review.language", &json!({"lang": "fr-FR", "noProof": true, "detect": false})).unwrap();
    let rc = s.doc.para_at(&Pos::body(0, 1)).map(|p| p.props_at(1).clone()).unwrap();
    assert_eq!((rc.lang.as_deref(), rc.no_proof), (Some("fr-FR"), Some(true)));
    assert!(!s.prefs.detect_language);
    s.run("review.language", &json!({"lang": "de-DE", "default": true})).unwrap();
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("de-DE"));
    assert!(s.run("review.language", &json!({"lang": "<script>"})).is_err());
}

/// The inspector's removals delete straight from the document, so they aren't tracked: on a
/// document that tracks changes the text would go with no revision to show for it, and the
/// revision trail would be broken behind the user's back. The removal is refused instead,
/// and the document is left as it was.
#[test]
fn inspect_removals_are_refused_while_the_document_tracks_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    // Tracking without Restrict Editing isolates the Inspector guard from the protected
    // document check added separately in #3285.
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    assert!(s.doc.settings.track_changes, "the document doesn't track changes");
    let before = s.doc.clone();
    let err = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap_err();
    assert!(matches!(err, CmdError::Disabled(_)), "{err}");
    assert_eq!(s.doc, before, "the document changed despite the refusal");
    // Reading the document stays allowed.
    let found = s.run("file.inspect", &json!({})).unwrap();
    assert_eq!(found["found"]["hiddenText"], true, "{found}");
}

/// A document that doesn't track changes is inspected as before: the hidden text goes.
#[test]
fn inspect_removals_work_when_the_document_does_not_track_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    assert!(!s.doc.settings.track_changes);
    let out = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap();
    assert_eq!(out["residual"], json!([]), "{out}");
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(!text.contains("Hidden"), "the hidden text is still there: {text}");
    assert!(text.contains("Visible text."), "{text}");
}
