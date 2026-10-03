//! Lexical tripwire for stored process environments (upstream 0.70.0
//! `ProcessEnvironmentStorageTests`).
//!
//! Scans the shipped Rust sources for struct, union and enum-variant fields whose name contains
//! `env` and whose type is a string map or a string pair list, including `Option`, `Box`, `Arc`,
//! reference, slice and local `type` alias spellings. Each one must use `ProcessEnvironment`.
//! Function parameters and locals are transient, not storage. This is not a type checker:
//! generic parameters, differently named fields and explicit logging of an unwrapped map still
//! need review.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex_lite::Regex;

/// Shipped source roots, relative to the repository root.
const SCANNED_ROOTS: &[&str] = &["rust/src", "apps/desktop-tauri/src-tauri/src"];

/// Reviewed fields that may keep a plain environment, as `(path, "Owner.field: Type")`.
///
/// Each entry must match exactly one declaration; stale or duplicate matches fail the guard.
const REVIEWED_EXCEPTIONS: &[(&str, &str)] = &[];

/// Optional path segments before a type name, such as `std::collections::`.
const PATH: &str = r"(?:[A-Za-z_][A-Za-z0-9_]*::)*";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    line: usize,
    declaration: String,
}

/// Scanner patterns, compiled once per test binary.
struct Patterns {
    item: Regex,
    alias: Regex,
    field: Regex,
    variant_name: Regex,
    reference: Regex,
    protected: Regex,
    container: Regex,
    map: Regex,
    pairs: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let compile = |pattern: &str| {
            Regex::new(pattern).unwrap_or_else(|error| panic!("pattern {pattern}: {error}"))
        };
        // A string-like key or value: String, OsString, &str, &OsStr, Box/Arc/Rc<str>, Cow<str>.
        let text = r"(?:{PATH}(?:String|OsString)|&(?:'[A-Za-z_][A-Za-z0-9_]* )?(?:str|OsStr)|{PATH}(?:Box|Arc|Rc)<(?:str|OsStr)>|{PATH}Cow<(?:'[A-Za-z_][A-Za-z0-9_]*,)?(?:str|OsStr)>)"
            .replace("{PATH}", PATH);
        Patterns {
            item: compile(r"\b(struct|union|enum)\s+([A-Za-z_][A-Za-z0-9_]*)"),
            alias: compile(r"\btype\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^=;]*>)?\s*=\s*([^;]+);"),
            field: compile(
                r"(?s)^\s*(?:#\[[^\]]*\]\s*)*(?:pub\b\s*(?:\([^)]*\)\s*)?)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(.+?)\s*$",
            ),
            variant_name: compile(r"([A-Za-z_][A-Za-z0-9_]*)\s*$"),
            reference: compile(r"^&(?:'[A-Za-z_][A-Za-z0-9_]* ?)?(?:mut )?"),
            protected: compile(&format!(r"^{PATH}ProcessEnvironment<")),
            container: compile(&format!(
                r"^{PATH}(?:Option|Box|Arc|Rc|Mutex|RwLock|RefCell|Cell)<(.*)>$"
            )),
            map: compile(&format!(
                r"^{PATH}(?:HashMap|BTreeMap|IndexMap)<{text},{text}(?:,.*)?>$"
            )),
            pairs: compile(&format!(
                r"^(?:{PATH}(?:Vec|VecDeque|Box)<\({text},{text}\)>|{PATH}Box<\[\({text},{text}\)\]>|\[\({text},{text}\)(?:;[^\]]+)?\])$"
            )),
        }
    })
}

#[test]
fn shipped_environment_storage_uses_the_redacting_wrapper() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the codexbar crate lives one level below the repository root");
    let mut files = Vec::new();
    for root in SCANNED_ROOTS {
        let directory = repo.join(root);
        assert!(
            directory.is_dir(),
            "missing scan root {}",
            directory.display()
        );
        collect_rust_files(&directory, &mut files);
    }
    files.sort();
    assert!(
        files.len() > 100,
        "expected the full source tree, found {} files",
        files.len()
    );

    let sources: Vec<(String, String)> = files
        .iter()
        .map(|path| {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            (relative_path(repo, path), sanitize(&text))
        })
        .collect();
    let aliases = environment_aliases(sources.iter().map(|(_, source)| source.as_str()));
    let findings: Vec<(String, Field)> = sources
        .iter()
        .flat_map(|(path, source)| {
            unprotected_fields(source, &aliases)
                .into_iter()
                .map(move |field| (path.clone(), field))
        })
        .collect();

    let problems = review(&findings, REVIEWED_EXCEPTIONS);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn scanner_recognizes_storage_spellings_without_flagging_parameters_or_wrapped_fields() {
    let source = sanitize(SPELLINGS);
    let aliases = environment_aliases([source.as_str()]);
    assert_eq!(
        aliases,
        BTreeSet::from(["EnvMap".to_string(), "EnvPairs".to_string()])
    );
    let found: Vec<String> = unprotected_fields(&source, &aliases)
        .into_iter()
        .map(|field| field.declaration)
        .collect();
    assert_eq!(
        found,
        [
            "Example.env: HashMap<String, String>",
            "Example.environment: std::collections::HashMap<String, String>",
            "Example.env_override: Option<HashMap<String, String>>",
            "Example.base_env: BTreeMap<String, String>",
            "Example.child_env: Vec<(OsString, OsString)>",
            "Example.borrowed_env: &'a HashMap<String, String>",
            "Example.shared_env: Arc<BTreeMap<String, String>>",
            "Example.alias_env: EnvMap",
            "Example.qualified_alias_env: roots::EnvPairs",
            "Example.slice_env: &'a [(&'static str, &'static str)]",
            "Generic.generic_env: HashMap<String, String>",
            "Launch::Child.env: BTreeMap<OsString, OsString>",
        ]
    );
}

#[test]
fn field_lines_point_at_the_declaration() {
    let source = sanitize(
        "struct A {\n    // env: HashMap<String, String>,\n    env:\n        HashMap<String, String>,\n}\n",
    );
    assert_eq!(
        unprotected_fields(&source, &BTreeSet::new()),
        [Field {
            line: 3,
            declaration: "A.env: HashMap<String, String>".to_string(),
        }]
    );
}

#[test]
fn reviewed_exceptions_must_match_exactly_one_declaration() {
    let field = |line, declaration: &str| Field {
        line,
        declaration: declaration.to_string(),
    };
    let findings = [
        (
            "a.rs".to_string(),
            field(3, "Kept.env: HashMap<String, String>"),
        ),
        (
            "a.rs".to_string(),
            field(9, "Kept.env: HashMap<String, String>"),
        ),
        (
            "b.rs".to_string(),
            field(4, "Open.env: Vec<(OsString, OsString)>"),
        ),
    ];
    let exceptions = [
        ("a.rs", "Kept.env: HashMap<String, String>"),
        ("c.rs", "Gone.env: HashMap<String, String>"),
    ];
    assert_eq!(
        review(&findings, &exceptions),
        [
            "Duplicate reviewed exception match: a.rs:9: Kept.env: HashMap<String, String>",
            "Unprotected environment storage: b.rs:4: Open.env: Vec<(OsString, OsString)>",
            "Remove stale reviewed exception: c.rs: Gone.env: HashMap<String, String>",
        ]
    );
    assert!(review(&findings[..1], &exceptions[..1]).is_empty());
}

#[test]
fn sanitize_blanks_comments_and_literals_but_keeps_offsets() {
    let source = "let a = \"{ env }\"; // { env }\nlet b = r#\"{\"#; /* { /* } */ } */ let c = '{';\nlet d: &'static str = \"\\\"}\";";
    let sanitized = sanitize(source);
    assert_eq!(sanitized.len(), source.len());
    assert_eq!(sanitized.matches('\n').count(), 2);
    assert!(!sanitized.contains('{') && !sanitized.contains('}'));
    assert!(!sanitized.contains("env"));
    assert!(sanitized.contains("&'static str"));
}

/// Storage spellings for the self-test, including declarations the guard must ignore.
const SPELLINGS: &str = r###"
type EnvMap = HashMap<String, String>;
pub type EnvPairs = Vec<(OsString, OsString)>;
type Callback = Box<dyn Fn() -> u32>;

#[derive(Debug)]
pub struct Example<'a> {
    env: HashMap<String, String>,
    pub environment: std::collections::HashMap<String, String>,
    pub(crate) env_override: Option<HashMap<String, String>>,
    #[serde(default, rename = "x,}")]
    base_env:
        BTreeMap<String, String>,
    child_env: Vec<(OsString, OsString)>,
    borrowed_env: &'a HashMap<String, String>,
    shared_env: Arc<BTreeMap<String, String>>,
    alias_env: EnvMap,
    qualified_alias_env: roots::EnvPairs,
    slice_env: &'a [(&'static str, &'static str)],
    protected_env: ProcessEnvironment<HashMap<String, String>>,
    protected_optional_env: Option<crate::process_environment::ProcessEnvironment<EnvMap>>,
    env_names: Vec<String>,
    headers: HashMap<String, String>,
    callback_env: Box<dyn Fn(&str) -> HashMap<String, String>>,
    // commented_env: HashMap<String, String>,
    /* block_env: HashMap<String, String>, */
}

pub struct Wrapper<T>(HashMap<String, String>, T);

pub struct Generic<F>
where
    F: Fn(u32) -> u32,
{
    generic_env: HashMap<String, String>,
    callback: F,
}

enum Launch {
    Child { program: String, env: BTreeMap<OsString, OsString> },
    Plain(HashMap<String, String>),
    Idle,
}

fn run(environment: HashMap<String, String>) -> HashMap<String, String> {
    let env: HashMap<String, String> = environment;
    let text = "struct Fake { env: HashMap<String, String> }";
    let quote = '"';
    env
}
"###;

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn relative_path(repo: &Path, path: &Path) -> String {
    path.strip_prefix(repo)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Blanks comments and the contents of string and char literals. Byte offsets and newlines are
/// kept, so braces, commas and keywords inside them cannot confuse the scan and line numbers
/// still match the file.
fn sanitize(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for byte in &mut out[from..to.min(bytes.len())] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let mut index = 0;
    while index < bytes.len() {
        let next = bytes.get(index + 1).copied();
        if bytes[index] == b'/' && next == Some(b'/') {
            let end = bytes[index..]
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(bytes.len(), |offset| index + offset);
            blank(&mut out, index, end);
            index = end;
        } else if bytes[index] == b'/' && next == Some(b'*') {
            let mut depth = 0usize;
            let mut end = index;
            while end < bytes.len() {
                if bytes[end] == b'/' && bytes.get(end + 1) == Some(&b'*') {
                    depth += 1;
                    end += 2;
                } else if bytes[end] == b'*' && bytes.get(end + 1) == Some(&b'/') {
                    depth -= 1;
                    end += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    end += 1;
                }
            }
            blank(&mut out, index, end);
            index = end;
        } else if let Some((content, hashes)) = raw_string_start(bytes, index) {
            let mut end = content;
            while end < bytes.len()
                && !(bytes[end] == b'"'
                    && bytes[end + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|&&byte| byte == b'#')
                        .count()
                        == hashes)
            {
                end += 1;
            }
            blank(&mut out, content, end);
            index = end + 1 + hashes;
        } else if bytes[index] == b'"' {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            blank(&mut out, index + 1, end);
            index = end + 1;
        } else if bytes[index] == b'\'' {
            index = char_literal_end(bytes, index).map_or(index + 1, |end| {
                blank(&mut out, index + 1, end - 1);
                end
            });
        } else {
            index += 1;
        }
    }
    String::from_utf8(out).expect("blanking whole literals keeps UTF-8 valid")
}

/// The content start and `#` count of a raw string literal (`r"`, `r#"`, `br"`, `cr"`) at `index`.
fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let previous_is_identifier =
        index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
    if previous_is_identifier {
        return None;
    }
    let mut cursor = index;
    if matches!(bytes.get(cursor), Some(b'b' | b'c')) {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'r') {
        return None;
    }
    cursor += 1;
    let hashes = bytes[cursor..]
        .iter()
        .take_while(|&&byte| byte == b'#')
        .count();
    cursor += hashes;
    (bytes.get(cursor) == Some(&b'"')).then_some((cursor + 1, hashes))
}

/// The end (one past the closing quote) of a char literal at `index`, or `None` for a lifetime.
fn char_literal_end(bytes: &[u8], index: usize) -> Option<usize> {
    let first = *bytes.get(index + 1)?;
    if first == b'\\' {
        let close = bytes
            .get(index + 3..)?
            .iter()
            .position(|&byte| byte == b'\'')?;
        return Some(index + 3 + close + 1);
    }
    let width = match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    (bytes.get(index + 1 + width) == Some(&b'\'')).then_some(index + 2 + width)
}

/// Local `type` aliases that name an environment map, resolved through aliases of aliases.
fn environment_aliases<'a>(sources: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let definitions: Vec<(String, String)> = sources
        .into_iter()
        .flat_map(|source| {
            patterns()
                .alias
                .captures_iter(source)
                .map(|captures| (captures[1].to_string(), captures[2].to_string()))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut names = BTreeSet::new();
    loop {
        let before = names.len();
        for (name, target) in &definitions {
            if is_environment_type(target, &names) == Some(true) {
                names.insert(name.clone());
            }
        }
        if names.len() == before {
            return names;
        }
    }
}

/// Environment-named fields of every struct, union and enum variant in `source` whose type is an
/// unwrapped environment.
fn unprotected_fields(source: &str, aliases: &BTreeSet<String>) -> Vec<Field> {
    let mut fields = Vec::new();
    for captures in patterns().item.captures_iter(source) {
        let keyword = &captures[1];
        let owner = &captures[2];
        let header_end = captures.get(0).expect("whole match").end();
        let Some(open) = body_open(source.as_bytes(), header_end) else {
            continue;
        };
        let close = matching_brace(source.as_bytes(), open);
        if keyword == "enum" {
            for (variant, start, end) in enum_variant_bodies(source, open + 1, close) {
                fields.extend(fields_in(
                    source,
                    start,
                    end,
                    &format!("{owner}::{variant}"),
                    aliases,
                ));
            }
        } else {
            fields.extend(fields_in(source, open + 1, close, owner, aliases));
        }
    }
    fields
}

/// The opening brace of a named-field body after an item name, skipping generics and a `where`
/// clause. Tuple and unit items have no named fields and return `None`.
fn body_open(bytes: &[u8], mut index: usize) -> Option<usize> {
    let skip_space = |index: &mut usize| {
        while bytes.get(*index).is_some_and(u8::is_ascii_whitespace) {
            *index += 1;
        }
    };
    skip_space(&mut index);
    if bytes.get(index) == Some(&b'<') {
        let mut depth = 0usize;
        while index < bytes.len() {
            match bytes[index] {
                b'-' if bytes.get(index + 1) == Some(&b'>') => index += 1,
                b'<' => depth += 1,
                b'>' => {
                    depth -= 1;
                    if depth == 0 {
                        index += 1;
                        break;
                    }
                }
                _ => {}
            }
            index += 1;
        }
        skip_space(&mut index);
    }
    match bytes.get(index)? {
        b'{' => Some(index),
        b'(' | b';' => None,
        _ => {
            // A `where` clause: its bounds may hold parentheses and arrows, never braces.
            let offset = bytes[index..]
                .iter()
                .position(|&byte| byte == b'{' || byte == b';')?;
            (bytes[index + offset] == b'{').then_some(index + offset)
        }
    }
}

fn matching_brace(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (index, &byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

/// `(variant, start, end)` for each enum variant with named fields between `start` and `end`.
fn enum_variant_bodies(source: &str, start: usize, end: usize) -> Vec<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut bodies = Vec::new();
    let mut nesting = 0usize;
    let mut segment = start;
    let mut index = start;
    while index < end {
        match bytes[index] {
            b'(' | b'[' => nesting += 1,
            b')' | b']' => nesting = nesting.saturating_sub(1),
            b',' if nesting == 0 => segment = index + 1,
            b'{' if nesting == 0 => {
                let close = matching_brace(bytes, index);
                if let Some(name) = patterns().variant_name.captures(&source[segment..index]) {
                    bodies.push((name[1].to_string(), index + 1, close));
                }
                index = close;
            }
            _ => {}
        }
        index += 1;
    }
    bodies
}

/// Environment-named fields between `start` and `end` whose type is an unwrapped environment.
fn fields_in(
    source: &str,
    start: usize,
    end: usize,
    owner: &str,
    aliases: &BTreeSet<String>,
) -> Vec<Field> {
    split_top_level(source, start, end)
        .into_iter()
        .filter_map(|(segment_start, segment_end)| {
            let captures = patterns()
                .field
                .captures(&source[segment_start..segment_end])?;
            let name = captures.get(1).expect("field name");
            let ty = &captures[2];
            if !name.as_str().to_ascii_lowercase().contains("env")
                || is_environment_type(ty, aliases) != Some(true)
            {
                return None;
            }
            let offset = segment_start + name.start();
            Some(Field {
                line: source[..offset].matches('\n').count() + 1,
                declaration: format!("{owner}.{}: {}", name.as_str(), display_type(ty)),
            })
        })
        .collect()
}

/// Comma-separated ranges between `start` and `end`, ignoring commas nested in brackets.
fn split_top_level(source: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    let mut nesting = 0usize;
    let mut segment = start;
    let mut index = start;
    while index < end {
        match bytes[index] {
            b'-' if bytes.get(index + 1) == Some(&b'>') => index += 1,
            b'(' | b'[' | b'{' | b'<' => nesting += 1,
            b')' | b']' | b'}' | b'>' => nesting = nesting.saturating_sub(1),
            b',' if nesting == 0 => {
                ranges.push((segment, index));
                segment = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    if !source[segment..end].trim().is_empty() {
        ranges.push((segment, end));
    }
    ranges
}

/// `Some(true)` for an unwrapped environment type, `Some(false)` for one already inside
/// `ProcessEnvironment`, `None` for anything else.
fn is_environment_type(ty: &str, aliases: &BTreeSet<String>) -> Option<bool> {
    let patterns = patterns();
    let mut current = patterns
        .reference
        .replace(&compact_type(ty), "")
        .into_owned();
    loop {
        if patterns.protected.is_match(&current) {
            return Some(false);
        }
        let Some(inner) = patterns
            .container
            .captures(&current)
            .map(|captures| captures[1].to_string())
        else {
            break;
        };
        current = patterns.reference.replace(&inner, "").into_owned();
    }
    let alias = current.rsplit("::").next().unwrap_or(&current);
    (patterns.map.is_match(&current)
        || patterns.pairs.is_match(&current)
        || aliases.contains(alias))
    .then_some(true)
}

/// Drops whitespace, keeping one space only between identifier characters (`'a mut`, `'static str`).
fn compact_type(ty: &str) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '\'';
    let mut out = String::new();
    let mut pending_space = false;
    for c in ty.chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && out.chars().last().is_some_and(is_word) && is_word(c) {
            out.push(' ');
        }
        pending_space = false;
        out.push(c);
    }
    out
}

/// The type as written, with whitespace runs collapsed for stable reports.
fn display_type(ty: &str) -> String {
    ty.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" ,", ",")
}

/// Problems for unreviewed findings, plus stale or duplicate reviewed exceptions.
fn review(findings: &[(String, Field)], exceptions: &[(&str, &str)]) -> Vec<String> {
    let mut matches: HashMap<(&str, &str), usize> = HashMap::new();
    let mut problems = Vec::new();
    for (path, field) in findings {
        let reviewed = exceptions
            .iter()
            .find(|(exception_path, declaration)| {
                exception_path == path && *declaration == field.declaration
            })
            .copied();
        match reviewed {
            Some(exception) => {
                let count = matches.entry(exception).or_default();
                *count += 1;
                if *count > 1 {
                    problems.push(format!(
                        "Duplicate reviewed exception match: {path}:{}: {}",
                        field.line, field.declaration
                    ));
                }
            }
            None => problems.push(format!(
                "Unprotected environment storage: {path}:{}: {}",
                field.line, field.declaration
            )),
        }
    }
    for exception in exceptions {
        if !matches.contains_key(exception) {
            problems.push(format!(
                "Remove stale reviewed exception: {}: {}",
                exception.0, exception.1
            ));
        }
    }
    problems
}
