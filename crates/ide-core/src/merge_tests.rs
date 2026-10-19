use super::*;

const TWO_CONFLICTS: &str = "\
fn main() {
<<<<<<< HEAD
    println!(\"ours\");
=======
    println!(\"theirs\");
>>>>>>> feature
    shared();
<<<<<<< HEAD
    a();
||||||| base
    original();
=======
    b();
    c();
>>>>>>> feature
}
";

#[test]
fn parses_conflicts_with_and_without_a_base() {
    let merge = Merge::parse(TWO_CONFLICTS);
    assert_eq!(
        merge.segments(),
        [
            Segment::Common("fn main() {\n".into()),
            Segment::Conflict(0),
            Segment::Common("    shared();\n".into()),
            Segment::Conflict(1),
            Segment::Common("}\n".into()),
        ]
    );
    let [first, second] = merge.conflicts() else {
        panic!("expected two conflicts");
    };
    assert_eq!(first.current, "    println!(\"ours\");\n");
    assert_eq!(first.incoming, "    println!(\"theirs\");\n");
    assert_eq!(first.base, None);
    assert_eq!(first.current_label, "HEAD");
    assert_eq!(first.incoming_label, "feature");
    assert_eq!(second.base.as_deref(), Some("    original();\n"));
    assert_eq!(second.incoming, "    b();\n    c();\n");
    assert_eq!(&merge.text()[merge.range(0)], first.original);
    assert_eq!(&merge.text()[merge.range(1)], second.original);
    assert_eq!(merge.unresolved(), 2);
    assert_eq!(
        merge.side_text(Side::Current),
        "fn main() {\n    println!(\"ours\");\n    shared();\n    a();\n}\n"
    );
    assert_eq!(
        merge.side_text(Side::Incoming),
        "fn main() {\n    println!(\"theirs\");\n    shared();\n    b();\n    c();\n}\n"
    );
}

#[test]
fn leaves_unterminated_and_malformed_markers_as_text() {
    let text = "a\n<<<<<<< HEAD\nb\n=======\nc\n<<<<<<<< not a marker\n";
    let merge = Merge::parse(text);
    assert!(merge.conflicts().is_empty());
    assert_eq!(merge.segments(), [Segment::Common(text.into())]);
    assert_eq!(merge.unresolved(), 0);

    let merge = Merge::parse("x\n<<<<<<< HEAD\n<<<<<<< HEAD\ny\n=======\nz\n>>>>>>> b\n");
    assert_eq!(merge.conflicts().len(), 1);
    assert_eq!(
        merge.segments()[0],
        Segment::Common("x\n<<<<<<< HEAD\n".into())
    );
}

#[test]
fn parses_markers_without_trailing_newline_or_labels() {
    let merge = Merge::parse("<<<<<<<\none\n=======\ntwo\n>>>>>>>");
    let conflict = &merge.conflicts()[0];
    assert_eq!(conflict.current, "one\n");
    assert_eq!(conflict.incoming, "two\n");
    assert_eq!(conflict.current_label, "");
    assert_eq!(merge.range(0), 0..merge.text().len());
}

#[test]
fn resolving_rewrites_the_conflict_and_moves_later_ranges() {
    let mut merge = Merge::parse(TWO_CONFLICTS);
    let (range, text) = merge.resolve(0, Choice::Incoming).unwrap();
    assert_eq!(range.start, "fn main() {\n".len());
    assert_eq!(text, "    println!(\"theirs\");\n");
    assert_eq!(merge.choice(0), Choice::Incoming);
    assert_eq!(merge.choice(1), Choice::Unresolved);
    assert_eq!(merge.resolve(0, Choice::Incoming), None);

    merge.resolve(1, Choice::CurrentThenIncoming).unwrap();
    assert_eq!(merge.unresolved(), 0);
    assert_eq!(
        merge.text(),
        "fn main() {\n    println!(\"theirs\");\n    shared();\n    a();\n    b();\n    c();\n}\n"
    );
    merge.resolve(1, Choice::Base).unwrap();
    assert_eq!(merge.choice(1), Choice::Base);
    merge.resolve(0, Choice::Unresolved).unwrap();
    assert_eq!(merge.choice(0), Choice::Unresolved);
    assert_eq!(merge.choice(1), Choice::Base);
    assert_eq!(merge.resolve(0, Choice::Custom), None);
}

#[test]
fn manual_edits_inside_a_conflict_make_it_custom() {
    let mut merge = Merge::parse(TWO_CONFLICTS);
    merge.resolve(0, Choice::Current).unwrap();
    let edited = merge.text().replace("\"ours\"", "\"mine\"");
    merge.sync(&edited);
    assert_eq!(merge.choice(0), Choice::Custom);
    assert_eq!(&merge.text()[merge.range(0)], "    println!(\"mine\");\n");
    assert_eq!(merge.choice(1), Choice::Unresolved);

    let undone = merge.text().replace("\"mine\"", "\"ours\"");
    merge.sync(&undone);
    assert_eq!(merge.choice(0), Choice::Current);
}

#[test]
fn edits_outside_conflicts_shift_ranges_without_touching_them() {
    let mut merge = Merge::parse(TWO_CONFLICTS);
    merge.resolve(0, Choice::Current).unwrap();
    let edited = format!("// header\n{}", merge.text());
    merge.sync(&edited);
    let edited = merge.text().replace("shared", "common");
    merge.sync(&edited);
    assert_eq!(merge.choice(0), Choice::Current);
    assert_eq!(merge.choice(1), Choice::Unresolved);
    assert_eq!(&merge.text()[merge.range(1)], merge.conflicts()[1].original);

    let end = merge.range(0).end;
    let mut typed = merge.text().to_string();
    typed.insert(end, 'x');
    merge.sync(&typed);
    assert_eq!(merge.choice(0), Choice::Current);

    let start = merge.range(0).start;
    let mut typed = merge.text().to_string();
    typed.insert(start, 'y');
    merge.sync(&typed);
    assert_eq!(merge.choice(0), Choice::Custom);
}

#[test]
fn insertions_into_an_empty_resolution_go_to_the_first_conflict() {
    let text = "<<<<<<< a\n=======\nx\n>>>>>>> b\n<<<<<<< a\ny\n=======\n>>>>>>> b\n";
    let mut merge = Merge::parse(text);
    merge.resolve(0, Choice::Current).unwrap();
    merge.resolve(1, Choice::Incoming).unwrap();
    assert_eq!(merge.text(), "");
    assert_eq!((merge.range(0), merge.range(1)), (0..0, 0..0));
    merge.sync("z\n");
    assert_eq!(merge.range(0), 0..2);
    assert_eq!(merge.range(1), 2..2);
    assert_eq!(merge.choice(0), Choice::Custom);
    assert_eq!(merge.choice(1), Choice::Incoming);
}

#[test]
fn sync_respects_multibyte_boundaries() {
    let mut merge = Merge::parse("é\n<<<<<<< a\nü\n=======\nö\n>>>>>>> b\n");
    merge.resolve(0, Choice::Incoming).unwrap();
    merge.sync("è\nö\n");
    assert_eq!(merge.range(0), "è\n".len().."è\nö\n".len());
    assert_eq!(merge.choice(0), Choice::Incoming);
}

#[test]
fn toggling_sides_follows_acceptance_order() {
    use Choice::*;
    assert_eq!(Unresolved.toggle(Side::Current, true), Current);
    assert_eq!(Current.toggle(Side::Incoming, true), CurrentThenIncoming);
    assert_eq!(Incoming.toggle(Side::Current, true), IncomingThenCurrent);
    assert_eq!(
        CurrentThenIncoming.toggle(Side::Current, true),
        CurrentThenIncoming
    );
    assert_eq!(CurrentThenIncoming.toggle(Side::Current, false), Incoming);
    assert_eq!(IncomingThenCurrent.toggle(Side::Current, false), Incoming);
    assert_eq!(Current.toggle(Side::Current, false), Unresolved);
    assert_eq!(Custom.toggle(Side::Incoming, true), Incoming);
    assert!(CurrentThenIncoming.accepts(Side::Incoming));
    assert!(!Base.accepts(Side::Current));
}

#[test]
fn joins_sides_without_a_trailing_newline() {
    let merge = Merge::parse("<<<<<<< a\none=======\n");
    assert!(merge.conflicts().is_empty());
    let conflict = Conflict {
        current: "one".into(),
        base: None,
        incoming: "two".into(),
        current_label: String::new(),
        incoming_label: String::new(),
        original: String::new(),
    };
    assert_eq!(
        conflict.text(Choice::CurrentThenIncoming).as_deref(),
        Some("one\ntwo")
    );
    assert_eq!(conflict.classify("one\ntwo"), Choice::CurrentThenIncoming);
    assert_eq!(conflict.classify("three"), Choice::Custom);
}
