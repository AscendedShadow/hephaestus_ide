use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use ide_core::document::{Document, MAX_OPEN_BYTES};
use tree_sitter::{Node, Parser, Query, QueryCursor, QueryError, StreamingIterator as _, Tree};

use crate::syntax;

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;

const C_DEFINITIONS: &str = include_str!("navigation/c.scm");
const C_SHARP_DEFINITIONS: &str = include_str!("navigation/c_sharp.scm");
const GO_DEFINITIONS: &str = include_str!("navigation/go.scm");
const JAVA_DEFINITIONS: &str = include_str!("navigation/java.scm");
const JAVASCRIPT_DEFINITIONS: &str = include_str!("navigation/javascript.scm");
const RUST_DEFINITIONS: &str = include_str!("navigation/rust.scm");
const TYPESCRIPT_DEFINITIONS: &str = include_str!("navigation/typescript.scm");
const ZIG_DEFINITIONS: &str = include_str!("navigation/zig.scm");

const SCRIPT_FAMILY: [&str; 4] = ["javascript", "jsx", "typescript", "tsx"];
const MEMBER_NAMES: [&str; 3] = [
    "field_identifier",
    "property_identifier",
    "private_property_identifier",
];
const MEMBER_PARENTS: [&str; 11] = [
    "field_expression",
    "field_access",
    "member_expression",
    "selector_expression",
    "member_access_expression",
    "method_invocation",
    "scoped_identifier",
    "scoped_type_identifier",
    "qualified_type",
    "qualified_name",
    "nested_type_identifier",
];
const MEMBER_FIELDS: [&str; 4] = ["field", "property", "name", "member"];
const PATTERN_SKIPPED_FIELDS: [&str; 7] = [
    "type",
    "right",
    "key",
    "parameters",
    "size",
    "arguments",
    "condition",
];
const PATTERN_SKIPPED_KINDS: [&str; 1] = ["range_pattern"];
const SKIPPED_DIRECTORIES: [&str; 8] = [
    "target",
    "node_modules",
    "zig-out",
    "zig-cache",
    "bin",
    "obj",
    "dist",
    "build",
];
const MAX_WORKSPACE_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(Range<usize>),
    Search(Search),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub name: String,
    pub member: bool,
    pub fallback: Option<Range<usize>>,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub name: Range<usize>,
    pub document: Option<Document>,
}

type Rank = (Reverse<usize>, usize, PathBuf, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Local,
    Import,
    Item,
}

#[derive(Clone, Debug)]
struct Definition {
    name: Range<usize>,
    kind: Kind,
    member: bool,
    scope: usize,
    depth: usize,
    visible_from: usize,
}

impl Definition {
    fn accepts(&self, member: bool) -> bool {
        if member {
            self.kind != Kind::Local
        } else {
            !self.member
        }
    }
}

struct Analysis {
    tree: Tree,
    scopes: HashSet<usize>,
    definitions: Vec<Definition>,
}

struct Analyzer {
    parser: Parser,
    queries: HashMap<&'static str, Option<Query>>,
}

impl Analyzer {
    fn new() -> Self {
        Self {
            parser: Parser::new(),
            queries: HashMap::new(),
        }
    }

    fn analyze(&mut self, language: &'static str, source: &str) -> Option<Analysis> {
        let grammar = syntax::grammar(language)?;
        let query = self
            .queries
            .entry(language)
            .or_insert_with(|| definitions_query(language)?.ok())
            .as_ref()?;
        self.parser.set_language(&grammar).ok()?;
        let tree = self.parser.parse(source, None)?;
        let (scopes, definitions) = collect(query, tree.root_node(), source);
        Some(Analysis {
            tree,
            scopes,
            definitions,
        })
    }
}

fn definitions_query(language: &str) -> Option<Result<Query, QueryError>> {
    let definitions = match language {
        "c" => C_DEFINITIONS,
        "c_sharp" => C_SHARP_DEFINITIONS,
        "go" => GO_DEFINITIONS,
        "java" => JAVA_DEFINITIONS,
        "javascript" | "jsx" => JAVASCRIPT_DEFINITIONS,
        "rust" => RUST_DEFINITIONS,
        "typescript" | "tsx" => TYPESCRIPT_DEFINITIONS,
        "zig" => ZIG_DEFINITIONS,
        _ => return None,
    };
    Some(Query::new(&syntax::grammar(language)?, definitions))
}

fn collect(query: &Query, root: Node, source: &str) -> (HashSet<usize>, Vec<Definition>) {
    let capture = |name: &str| query.capture_index_for_name(name);
    let (scope_capture, definition_capture) = (capture("scope"), capture("definition"));
    let kinds = [
        (capture("item"), Kind::Item),
        (capture("local"), Kind::Local),
        (capture("import"), Kind::Import),
    ];
    let mut scopes = HashSet::from([root.id()]);
    let mut declared = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source.as_bytes());
    while let Some(found) = matches.next() {
        let definition = found
            .captures
            .iter()
            .find(|capture| Some(capture.index) == definition_capture)
            .map(|capture| capture.node);
        for capture in found.captures {
            if Some(capture.index) == scope_capture {
                scopes.insert(capture.node.id());
            } else if let Some(&(_, kind)) = kinds
                .iter()
                .find(|(index, _)| *index == Some(capture.index))
            {
                declared.push((capture.node, kind, definition));
            }
        }
    }

    let mut definitions = HashMap::new();
    for (node, kind, definition) in declared {
        let (scope, depth) = enclosing_scope(definition.unwrap_or(node), &scopes);
        let visible_from = match kind {
            Kind::Local => definition.map_or(node.start_byte(), |definition| definition.end_byte()),
            Kind::Item | Kind::Import => 0,
        };
        let mut names = Vec::new();
        if node.named_child_count() == 0 {
            names.push(node);
        } else {
            pattern_names(node, &mut names);
        }
        for name in names {
            if definitions
                .get(&name.id())
                .is_some_and(|existing: &Definition| existing.kind >= kind)
            {
                continue;
            }
            definitions.insert(
                name.id(),
                Definition {
                    name: name.byte_range(),
                    kind,
                    member: MEMBER_NAMES.contains(&name.kind()),
                    scope,
                    depth,
                    visible_from,
                },
            );
        }
    }
    let mut definitions: Vec<_> = definitions.into_values().collect();
    definitions.sort_by_key(|definition| definition.name.start);
    (scopes, definitions)
}

fn pattern_names<'t>(node: Node<'t>, names: &mut Vec<Node<'t>>) {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let child = cursor.node();
        let skipped = cursor
            .field_name()
            .is_some_and(|field| PATTERN_SKIPPED_FIELDS.contains(&field));
        if child.is_named() && !skipped {
            if is_name(child) {
                names.push(child);
            } else if !child.kind().contains("identifier")
                && !PATTERN_SKIPPED_KINDS.contains(&child.kind())
            {
                pattern_names(child, names);
            }
        }
        if !cursor.goto_next_sibling() {
            return;
        }
    }
}

fn is_name(node: Node) -> bool {
    node.is_named()
        && node.named_child_count() == 0
        && node.kind().contains("identifier")
        && !node.kind().starts_with("builtin")
}

fn ancestors<'t>(node: Node<'t>) -> impl Iterator<Item = Node<'t>> {
    std::iter::successors(node.parent(), Node::parent)
}

fn enclosing_scope(node: Node, scopes: &HashSet<usize>) -> (usize, usize) {
    let mut chain = ancestors(node).filter(|ancestor| scopes.contains(&ancestor.id()));
    let scope = chain.next().map_or(node.id(), |scope| scope.id());
    (scope, 1 + chain.count())
}

fn name_at(root: Node, offset: usize) -> Option<Node> {
    let at = |start: usize| {
        root.descendant_for_byte_range(start, start + 1)
            .filter(|node| is_name(*node))
    };
    at(offset).or_else(|| offset.checked_sub(1).and_then(at))
}

fn field_name(parent: Node, child: Node) -> Option<&'static str> {
    let mut cursor = parent.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        if cursor.node() == child {
            return cursor.field_name();
        }
        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

fn is_member_reference(node: Node) -> bool {
    MEMBER_NAMES.contains(&node.kind())
        || node.parent().is_some_and(|parent| {
            MEMBER_PARENTS.contains(&parent.kind())
                && field_name(parent, node).is_some_and(|field| MEMBER_FIELDS.contains(&field))
        })
}

pub fn lookup(language: &'static str, source: &str, offset: usize) -> Option<Lookup> {
    let analysis = Analyzer::new().analyze(language, source)?;
    let reference = name_at(analysis.tree.root_node(), offset)?;
    let range = reference.byte_range();
    let name = &source[range.clone()];
    if let Some(definition) = analysis
        .definitions
        .iter()
        .find(|definition| definition.name == range)
    {
        return (definition.kind == Kind::Import).then(|| {
            Lookup::Search(Search {
                name: name.into(),
                member: false,
                fallback: None,
            })
        });
    }

    let member = is_member_reference(reference);
    let enclosing: Vec<usize> = ancestors(reference)
        .filter(|ancestor| analysis.scopes.contains(&ancestor.id()))
        .map(|ancestor| ancestor.id())
        .collect();
    let candidates: Vec<&Definition> = analysis
        .definitions
        .iter()
        .filter(|definition| source[definition.name.clone()] == *name && definition.accepts(member))
        .collect();
    let search = |fallback| {
        Lookup::Search(Search {
            name: name.into(),
            member,
            fallback,
        })
    };

    let visible = candidates
        .iter()
        .filter(|definition| {
            definition.kind != Kind::Import
                && enclosing.contains(&definition.scope)
                && definition.visible_from <= range.start
        })
        .max_by_key(|definition| {
            (
                definition.depth,
                definition.visible_from,
                definition.name.start,
            )
        });
    if let Some(definition) = visible {
        return Some(Lookup::Found(definition.name.clone()));
    }
    if let Some(import) = candidates
        .iter()
        .find(|definition| definition.kind == Kind::Import)
    {
        return Some(search(Some(import.name.clone())));
    }
    if let Some(item) = candidates
        .iter()
        .find(|definition| definition.kind == Kind::Item)
    {
        return Some(Lookup::Found(item.name.clone()));
    }
    Some(search(None))
}

pub fn search_workspace(
    root: &Path,
    origin: Option<&Path>,
    language: &str,
    search: &Search,
    open: &HashMap<PathBuf, String>,
) -> Option<Hit> {
    let mut analyzer = Analyzer::new();
    let mut best: Option<(Rank, Hit)> = None;
    for path in source_files(root, language) {
        if origin == Some(path.as_path()) {
            continue;
        }
        let (source, document) = match open.get(&path) {
            Some(text) if text.contains(&search.name) => (text.clone(), None),
            Some(_) => continue,
            None => match read_candidate(&path, &search.name) {
                Some(document) => (document.text().to_string(), Some(document)),
                None => continue,
            },
        };
        let Some(analysis) = analyzer.analyze(syntax::language(Some(&path)), &source) else {
            continue;
        };
        for definition in &analysis.definitions {
            if definition.kind != Kind::Item
                || !definition.accepts(search.member)
                || source[definition.name.clone()] != search.name
            {
                continue;
            }
            let rank = (
                Reverse(shared_components(origin, &path)),
                definition.depth,
                path.clone(),
                definition.name.start,
            );
            if best.as_ref().is_none_or(|(best, _)| rank < *best) {
                let target = document
                    .as_ref()
                    .and_then(Document::path)
                    .map_or_else(|| path.clone(), Path::to_path_buf);
                best = Some((
                    rank,
                    Hit {
                        path: target,
                        name: definition.name.clone(),
                        document: document.clone(),
                    },
                ));
            }
        }
    }
    best.map(|(_, hit)| hit)
}

fn read_candidate(path: &Path, name: &str) -> Option<Document> {
    if fs::metadata(path).ok()?.len() > MAX_OPEN_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if !std::str::from_utf8(&bytes).is_ok_and(|text| text.contains(name)) {
        return None;
    }
    Document::open(path).ok()
}

fn shared_components(origin: Option<&Path>, path: &Path) -> usize {
    origin.and_then(Path::parent).map_or(0, |directory| {
        directory
            .components()
            .zip(path.components())
            .take_while(|(a, b)| a == b)
            .count()
    })
}

fn same_family(a: &str, b: &str) -> bool {
    a == b || (SCRIPT_FAMILY.contains(&a) && SCRIPT_FAMILY.contains(&b))
}

fn source_files(root: &Path, language: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    let mut entries = 0;
    while let Some(directory) = directories.pop() {
        let Ok(listing) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in listing.flatten() {
            entries += 1;
            if entries > MAX_WORKSPACE_ENTRIES {
                files.sort();
                return files;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with('.') && !SKIPPED_DIRECTORIES.contains(&name.as_ref()) {
                    directories.push(path);
                }
            } else if file_type.is_file() && same_family(language, syntax::language(Some(&path))) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
