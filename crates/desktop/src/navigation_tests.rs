use super::*;

fn nth(source: &str, word: &str, n: usize) -> usize {
    let is_name = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    source
        .match_indices(word)
        .map(|(ix, _)| ix)
        .filter(|&start| {
            let before = source[..start].chars().next_back();
            let after = source[start + word.len()..].chars().next();
            !before.is_some_and(is_name) && !after.is_some_and(is_name)
        })
        .nth(n)
        .unwrap_or_else(|| panic!("{word:?} #{n} is not in the source"))
}

fn at(source: &str, word: &str, n: usize) -> Range<usize> {
    let start = nth(source, word, n);
    start..start + word.len()
}

#[track_caller]
fn assert_jump(language: &'static str, source: &str, from: (&str, usize), to: (&str, usize)) {
    let reference = at(source, from.0, from.1);
    let expected = Some(Lookup::Found(at(source, to.0, to.1)));
    for offset in [reference.start, reference.start + 1, reference.end] {
        assert_eq!(
            lookup(language, source, offset),
            expected,
            "{language}: {from:?} at byte {offset} should jump to {to:?}"
        );
    }
}

#[track_caller]
fn assert_search(
    language: &'static str,
    source: &str,
    from: (&str, usize),
    member: bool,
    fallback: Option<(&str, usize)>,
) {
    let offset = nth(source, from.0, from.1) + 1;
    assert_eq!(
        lookup(language, source, offset),
        Some(Lookup::Search(Search {
            name: from.0.into(),
            member,
            fallback: fallback.map(|(word, n)| at(source, word, n)),
        })),
        "{language}: {from:?}"
    );
}

#[test]
fn every_language_has_a_valid_definitions_query() {
    for language in [
        "c",
        "c_sharp",
        "go",
        "java",
        "javascript",
        "jsx",
        "rust",
        "typescript",
        "tsx",
        "zig",
    ] {
        definitions_query(language)
            .unwrap_or_else(|| panic!("{language} has no definitions query"))
            .unwrap_or_else(|error| panic!("invalid {language} definitions query: {error}"));
    }
    assert!(definitions_query("plain_text").is_none());
    assert_eq!(lookup("plain_text", "value value", 1), None);
}

const RUST: &str = r#"use std::collections::HashMap;
use crate::shapes::{Circle, Square as Tile};

struct Point { x: i32, y: i32 }

impl Point {
    fn new(x: i32) -> Self {
        let y = x;
        Self { x, y }
    }

    fn sum(&self) -> i32 {
        self.x + helper(self.y)
    }
}

fn helper(value: i32) -> i32 {
    let value = value + 1;
    let (a, b) = (value, 2);
    match Some(a) {
        Some(inner) if inner > b => inner,
        _ => value,
    }
}

fn closures() {
    let total = |item: i32| item * 2;
    for item in [1, 2] {
        total(item);
    }
    let map: HashMap<i32, i32> = HashMap::default();
    Point::new(1).sum();
    Circle::default();
    Tile::default();
}
"#;

#[test]
fn rust_functions_types_and_fields() {
    assert_jump("rust", RUST, ("helper", 0), ("helper", 1));
    assert_jump("rust", RUST, ("Point", 2), ("Point", 0));
    assert_jump("rust", RUST, ("Point", 1), ("Point", 0));
    assert_jump("rust", RUST, ("new", 1), ("new", 0));
    assert_jump("rust", RUST, ("sum", 1), ("sum", 0));
    assert_jump("rust", RUST, ("x", 4), ("x", 0));
    assert_jump("rust", RUST, ("y", 3), ("y", 0));
}

#[test]
fn rust_locals_follow_scope_and_shadowing() {
    assert_jump("rust", RUST, ("x", 2), ("x", 1));
    assert_jump("rust", RUST, ("x", 3), ("x", 1));
    assert_jump("rust", RUST, ("y", 2), ("y", 1));
    assert_jump("rust", RUST, ("value", 2), ("value", 0));
    assert_jump("rust", RUST, ("value", 3), ("value", 1));
    assert_jump("rust", RUST, ("value", 4), ("value", 1));
    assert_jump("rust", RUST, ("a", 1), ("a", 0));
    assert_jump("rust", RUST, ("b", 1), ("b", 0));
    assert_jump("rust", RUST, ("inner", 1), ("inner", 0));
    assert_jump("rust", RUST, ("inner", 2), ("inner", 0));
    assert_jump("rust", RUST, ("item", 1), ("item", 0));
    assert_jump("rust", RUST, ("item", 3), ("item", 2));
    assert_jump("rust", RUST, ("total", 1), ("total", 0));
}

#[test]
fn rust_imports_search_the_workspace_first() {
    assert_search("rust", RUST, ("HashMap", 1), false, Some(("HashMap", 0)));
    assert_search("rust", RUST, ("Circle", 1), false, Some(("Circle", 0)));
    assert_search("rust", RUST, ("Tile", 1), false, Some(("Tile", 0)));
    assert_search("rust", RUST, ("HashMap", 0), false, None);
    assert_search("rust", RUST, ("Some", 0), false, None);
    assert_search("rust", RUST, ("default", 0), true, None);
}

#[test]
fn rust_declarations_and_punctuation_do_not_jump() {
    for (word, n) in [("helper", 1), ("Point", 0), ("value", 1), ("inner", 0)] {
        let offset = nth(RUST, word, n) + 1;
        assert_eq!(lookup("rust", RUST, offset), None, "{word} #{n}");
    }
    assert_eq!(lookup("rust", RUST, RUST.find("=>").unwrap()), None);
    assert_eq!(lookup("rust", RUST, RUST.find("2)").unwrap()), None);
}

const TYPESCRIPT: &str = r#"import Base, { Shape as Figure } from "./shape";

interface Named { label: string }

class Circle extends Base implements Named {
  label = "circle";
  constructor(private radius: number) { super(); }
  area(scale: number): number {
    const doubled = this.radius * scale;
    return doubled + this.label.length;
  }
}

function build({ size, name: title }: Options, ...rest: Figure[]) {
  const circle = new Circle(size);
  const run = (factor) => circle.area(factor);
  for (const entry of rest) { run(entry); }
  return title;
}

export const created = build(options);
"#;

#[test]
fn typescript_classes_members_and_parameters() {
    assert_jump("typescript", TYPESCRIPT, ("Circle", 1), ("Circle", 0));
    assert_jump("typescript", TYPESCRIPT, ("Named", 1), ("Named", 0));
    assert_jump("typescript", TYPESCRIPT, ("scale", 1), ("scale", 0));
    assert_jump("typescript", TYPESCRIPT, ("doubled", 1), ("doubled", 0));
    assert_jump("typescript", TYPESCRIPT, ("label", 2), ("label", 1));
    assert_jump("typescript", TYPESCRIPT, ("radius", 1), ("radius", 0));
    assert_jump("typescript", TYPESCRIPT, ("size", 1), ("size", 0));
    assert_jump("typescript", TYPESCRIPT, ("title", 1), ("title", 0));
    assert_jump("typescript", TYPESCRIPT, ("area", 1), ("area", 0));
    assert_jump("typescript", TYPESCRIPT, ("factor", 1), ("factor", 0));
    assert_jump("typescript", TYPESCRIPT, ("run", 1), ("run", 0));
    assert_jump("typescript", TYPESCRIPT, ("entry", 1), ("entry", 0));
    assert_jump("typescript", TYPESCRIPT, ("rest", 1), ("rest", 0));
    assert_jump("typescript", TYPESCRIPT, ("build", 1), ("build", 0));
    assert_search(
        "typescript",
        TYPESCRIPT,
        ("Base", 1),
        false,
        Some(("Base", 0)),
    );
    assert_search(
        "typescript",
        TYPESCRIPT,
        ("Figure", 1),
        false,
        Some(("Figure", 0)),
    );
    assert_search("typescript", TYPESCRIPT, ("Options", 0), false, None);
    assert_search("typescript", TYPESCRIPT, ("options", 0), false, None);
}

const JAVASCRIPT: &str = r#"import { render } from "./view";

const LIMIT = 3;

function View({ items }) {
  return <List items={items} limit={LIMIT} />;
}

function List(props) {
  let [first, , last = LIMIT] = props.items;
  render(first, last);
}
"#;

#[test]
fn javascript_and_jsx_references() {
    for language in ["javascript", "jsx"] {
        assert_jump(language, JAVASCRIPT, ("List", 0), ("List", 1));
        assert_jump(language, JAVASCRIPT, ("items", 2), ("items", 0));
        assert_jump(language, JAVASCRIPT, ("LIMIT", 1), ("LIMIT", 0));
        assert_jump(language, JAVASCRIPT, ("LIMIT", 2), ("LIMIT", 0));
        assert_jump(language, JAVASCRIPT, ("props", 1), ("props", 0));
        assert_jump(language, JAVASCRIPT, ("first", 1), ("first", 0));
        assert_jump(language, JAVASCRIPT, ("last", 1), ("last", 0));
        assert_search(
            language,
            JAVASCRIPT,
            ("render", 1),
            false,
            Some(("render", 0)),
        );
    }
}

const GO: &str = r#"package shapes

import str "strings"

type Circle struct {
	Radius float64
}

const Pi = 3.14

func (c *Circle) Area(scale float64) float64 {
	area := Pi * c.Radius * scale
	for i, part := range parts(area) {
		area += float64(i) + part
	}
	return area
}

func Describe() string {
	c := &Circle{Radius: 2}
	return str.Repeat("x", int(c.Area(1)))
}
"#;

#[test]
fn go_receivers_fields_and_short_declarations() {
    assert_jump("go", GO, ("Circle", 1), ("Circle", 0));
    assert_jump("go", GO, ("Circle", 2), ("Circle", 0));
    assert_jump("go", GO, ("Pi", 1), ("Pi", 0));
    assert_jump("go", GO, ("c", 1), ("c", 0));
    assert_jump("go", GO, ("Radius", 1), ("Radius", 0));
    assert_jump("go", GO, ("scale", 1), ("scale", 0));
    assert_jump("go", GO, ("area", 1), ("area", 0));
    assert_jump("go", GO, ("area", 2), ("area", 0));
    assert_jump("go", GO, ("i", 1), ("i", 0));
    assert_jump("go", GO, ("part", 1), ("part", 0));
    assert_jump("go", GO, ("c", 3), ("c", 2));
    assert_jump("go", GO, ("Area", 1), ("Area", 0));
    assert_search("go", GO, ("str", 1), false, Some(("str", 0)));
    assert_search("go", GO, ("parts", 0), false, None);
}

const C: &str = r#"#define LIMIT 8

typedef struct Point {
  int x, *y;
} Point;

enum Color { RED, GREEN };

static int count = 0;
int add(int left, int right);

static Point *make(const char *name, int (*callback)(int value)) {
  Point *point = 0;
  for (int i = 0; i < LIMIT; i++) {
    count += add(i, callback(RED));
  }
  point->x = name[0];
  return point;
}

int add(int left, int right) { return left + right; }
"#;

#[test]
fn c_declarators_macros_and_fields() {
    assert_jump("c", C, ("LIMIT", 1), ("LIMIT", 0));
    assert_jump("c", C, ("Point", 2), ("Point", 1));
    assert_jump("c", C, ("Point", 3), ("Point", 1));
    assert_jump("c", C, ("count", 1), ("count", 0));
    assert_jump("c", C, ("add", 1), ("add", 2));
    assert_jump("c", C, ("i", 1), ("i", 0));
    assert_jump("c", C, ("i", 3), ("i", 0));
    assert_jump("c", C, ("callback", 1), ("callback", 0));
    assert_jump("c", C, ("RED", 1), ("RED", 0));
    assert_jump("c", C, ("point", 1), ("point", 0));
    assert_jump("c", C, ("x", 1), ("x", 0));
    assert_jump("c", C, ("name", 1), ("name", 0));
    assert_jump("c", C, ("left", 2), ("left", 1));
    assert_search("c", C, ("value", 0), false, None);
}

const JAVA: &str = r#"import java.util.List;

class Box<T> {
  private int count;
  static final String NAME = "box";

  Box(int count) { this.count = count; }

  int total(List<T> items) {
    int sum = count;
    for (T item : items) { sum += item.hashCode(); }
    Runnable run = () -> total(items);
    return sum + NAME.length();
  }
}
"#;

#[test]
fn java_members_parameters_and_loops() {
    assert_jump("java", JAVA, ("count", 3), ("count", 1));
    assert_jump("java", JAVA, ("count", 2), ("count", 0));
    assert_jump("java", JAVA, ("count", 4), ("count", 0));
    assert_jump("java", JAVA, ("T", 1), ("T", 0));
    assert_jump("java", JAVA, ("items", 1), ("items", 0));
    assert_jump("java", JAVA, ("items", 2), ("items", 0));
    assert_jump("java", JAVA, ("item", 1), ("item", 0));
    assert_jump("java", JAVA, ("sum", 2), ("sum", 0));
    assert_jump("java", JAVA, ("total", 1), ("total", 0));
    assert_jump("java", JAVA, ("NAME", 1), ("NAME", 0));
    assert_search("java", JAVA, ("List", 1), false, Some(("List", 0)));
}

const C_SHARP: &str = r#"using Alias = System.Text;

namespace Shapes {
  public class Circle {
    private double radius;
    public double Radius { get; set; }

    public Circle(double radius) { this.radius = radius; }

    public double Area(int scale) {
      var squared = radius * radius;
      Func<int, int> twice = n => n * 2;
      foreach (var part in Parts()) { squared += part; }
      return squared * twice(scale) + Radius;
    }
  }
}
"#;

#[test]
fn c_sharp_members_lambdas_and_properties() {
    assert_jump("c_sharp", C_SHARP, ("radius", 3), ("radius", 1));
    assert_jump("c_sharp", C_SHARP, ("radius", 2), ("radius", 0));
    assert_jump("c_sharp", C_SHARP, ("radius", 4), ("radius", 0));
    assert_jump("c_sharp", C_SHARP, ("squared", 1), ("squared", 0));
    assert_jump("c_sharp", C_SHARP, ("n", 1), ("n", 0));
    assert_jump("c_sharp", C_SHARP, ("part", 1), ("part", 0));
    assert_jump("c_sharp", C_SHARP, ("twice", 1), ("twice", 0));
    assert_jump("c_sharp", C_SHARP, ("scale", 1), ("scale", 0));
    assert_jump("c_sharp", C_SHARP, ("Radius", 1), ("Radius", 0));
    assert_search("c_sharp", C_SHARP, ("Parts", 0), false, None);
}

const ZIG: &str = r#"const std = @import("std");

pub const Point = struct {
    x: i32,

    pub fn init(x: i32) Point {
        return .{ .x = x };
    }

    fn doubled(self: Point) i32 {
        return self.x * 2;
    }
};

fn sum(items: []const i32) i32 {
    var total: i32 = 0;
    for (items) |item| {
        total += item;
    }
    const point = Point.init(total);
    return point.doubled() + std.math.maxInt(i8);
}
"#;

#[test]
fn zig_containers_payloads_and_locals() {
    assert_jump("zig", ZIG, ("Point", 1), ("Point", 0));
    assert_jump("zig", ZIG, ("Point", 3), ("Point", 0));
    assert_jump("zig", ZIG, ("x", 3), ("x", 1));
    assert_jump("zig", ZIG, ("x", 2), ("x", 0));
    assert_jump("zig", ZIG, ("x", 4), ("x", 0));
    assert_jump("zig", ZIG, ("items", 1), ("items", 0));
    assert_jump("zig", ZIG, ("item", 1), ("item", 0));
    assert_jump("zig", ZIG, ("total", 1), ("total", 0));
    assert_jump("zig", ZIG, ("total", 2), ("total", 0));
    assert_jump("zig", ZIG, ("init", 1), ("init", 0));
    assert_jump("zig", ZIG, ("doubled", 1), ("doubled", 0));
    assert_jump("zig", ZIG, ("point", 1), ("point", 0));
    assert_jump("zig", ZIG, ("std", 2), ("std", 0));
}

fn file(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn workspace() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let write = |path: &str, text: &str| {
        let path = file(&root, path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "src/main.rs",
        "mod shapes;\nfn main() { let c = Circle::new(); c.area(); }\n",
    );
    write(
        "src/shapes.rs",
        "pub struct Circle { radius: f64 }\r\nimpl Circle {\r\n    pub fn new() -> Self { todo!() }\r\n    pub fn area(&self) -> f64 { self.radius }\r\n}\r\n",
    );
    write("other/deep/shapes.rs", "pub struct Circle;\n");
    write("target/debug/generated.rs", "pub fn area() {}\n");
    write(".hidden/secret.rs", "pub fn area() {}\n");
    write("web/view.ts", "import { render } from './render';\n");
    write("web/render.js", "export function render() {}\n");
    write("notes.txt", "pub struct Circle;\n");
    (directory, root)
}

fn search(name: &str, member: bool) -> Search {
    Search {
        name: name.into(),
        member,
        fallback: None,
    }
}

#[test]
fn workspace_search_prefers_nearby_items_in_the_same_language() {
    let (_directory, root) = workspace();
    let origin = file(&root, "src/main.rs");
    let open = HashMap::new();
    let find = |name: &str, member: bool, language: &str, origin: &Path| {
        search_workspace(&root, Some(origin), language, &search(name, member), &open)
    };

    let hit = find("Circle", false, "rust", &origin).unwrap();
    assert_eq!(hit.path, file(&root, "src/shapes.rs"));
    let document = hit.document.unwrap();
    let text = document.text().to_string();
    assert_eq!(&text[hit.name.clone()], "Circle");
    assert_eq!(hit.name.start, "pub struct ".len());

    let hit = find("area", true, "rust", &origin).unwrap();
    assert_eq!(hit.path, file(&root, "src/shapes.rs"));
    assert_eq!(&text[hit.name], "area");

    let far = file(&root, "other/deep/main.rs");
    let hit = find("Circle", false, "rust", &far).unwrap();
    assert_eq!(hit.path, file(&root, "other/deep/shapes.rs"));

    assert!(find("radius", false, "rust", &origin).is_none());
    assert!(find("radius", true, "rust", &origin).is_some());
    assert_eq!(
        find("area", false, "rust", &origin).unwrap().path,
        file(&root, "src/shapes.rs")
    );
    assert!(find("main", false, "rust", &origin).is_none());

    let hit = find("render", false, "typescript", &file(&root, "web/view.ts")).unwrap();
    assert_eq!(hit.path, file(&root, "web/render.js"));
}

#[test]
fn workspace_search_reads_open_buffers_instead_of_disk() {
    let (_directory, root) = workspace();
    let shapes = file(&root, "src/shapes.rs");
    let edited = "// moved\npub struct Circle;\n".to_string();
    let open = HashMap::from([(shapes.clone(), edited.clone())]);
    let hit = search_workspace(
        &root,
        Some(&file(&root, "src/main.rs")),
        "rust",
        &search("Circle", false),
        &open,
    )
    .unwrap();
    assert_eq!(hit.path, shapes);
    assert!(hit.document.is_none());
    assert_eq!(&edited[hit.name], "Circle");
}
