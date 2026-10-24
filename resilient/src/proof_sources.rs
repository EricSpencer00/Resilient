//! Frozen translation inputs are collected from the buffers actually parsed.
use crate::imports::{ImportTrace, TracedTarget};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_FILES: usize = 256;
const MAX_EDGES: usize = 1024;
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
struct Input {
    sha256: String,
    bytes: u64,
}
type Edges = BTreeMap<(PathBuf, String), (PathBuf, Option<String>)>;

#[derive(Default)]
pub(crate) struct Capture {
    files: BTreeMap<PathBuf, Input>,
    edges: Edges,
    builtins: BTreeSet<(PathBuf, String)>,
    entry: PathBuf,
}

fn fail(message: impl std::fmt::Display) -> String {
    format!("source binding: {message}")
}
fn canonical(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(fail)
}
fn input(source: &str) -> Input {
    Input {
        sha256: crate::cert_sign::sha256_hex(source.as_bytes()),
        bytes: source.len() as u64,
    }
}

impl Capture {
    pub(crate) fn new(entry: &Path, source: &str) -> Result<Self, String> {
        let mut capture = Self {
            entry: canonical(entry)?,
            ..Self::default()
        };
        capture.parsed(entry, source)?;
        Ok(capture)
    }
    pub(crate) fn json(&self) -> Result<Value, String> {
        let mut root = self
            .entry
            .parent()
            .ok_or_else(|| fail("entry has no parent"))?
            .to_path_buf();
        while !self.files.keys().all(|path| path.starts_with(&root)) {
            if !root.pop() {
                return Err(fail("no common input root"));
            }
        }
        let relative = |path: &Path| -> Result<String, String> {
            let path = path.strip_prefix(&root).map_err(fail)?;
            let text = path
                .to_str()
                .ok_or_else(|| fail("input path is not UTF-8"))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            safe_path(&text)?;
            Ok(text)
        };
        let mut files = Vec::new();
        for (path, file) in &self.files {
            files.push(json!({"path":relative(path)?,"sha256":file.sha256,"bytes":file.bytes}));
        }
        let mut edges = Vec::new();
        for ((from, specifier), (target, namespace)) in &self.edges {
            edges.push(json!({"from":relative(from)?,"specifier":specifier,"target":relative(target)?,"namespace":namespace}));
        }
        let mut builtins = Vec::new();
        for (from, module) in &self.builtins {
            builtins.push(json!({"from":relative(from)?,"module":module}));
        }
        Ok(
            json!({"version":1,"entry":relative(&self.entry)?,"files":files,"imports":edges,"builtins":builtins}),
        )
    }
}

impl ImportTrace for Capture {
    fn parsed(&mut self, path: &Path, source: &str) -> Result<(), String> {
        let path = canonical(path)?;
        let value = input(source);
        if value.bytes > MAX_FILE_BYTES {
            return Err(fail("input exceeds per-file byte limit"));
        }
        if let Some(previous) = self.files.get(&path) {
            if previous != &value {
                return Err(fail("input changed between parsing operations"));
            }
            return Ok(());
        }
        if self.files.len() >= MAX_FILES
            || self.files.values().map(|v| v.bytes).sum::<u64>() + value.bytes > MAX_TOTAL_BYTES
        {
            return Err(fail("input inventory resource limit exceeded"));
        }
        self.files.insert(path, value);
        Ok(())
    }
    fn edge(
        &mut self,
        from: &Path,
        specifier: &str,
        target: &Path,
        namespace: Option<&str>,
    ) -> Result<(), String> {
        let key = (canonical(from)?, specifier.to_string());
        let value = (canonical(target)?, namespace.map(str::to_string));
        if let Some(previous) = self.edges.get(&key) {
            if previous != &value {
                return Err(fail("import resolution changed during compilation"));
            }
            return Ok(());
        }
        if self.edges.len() + self.builtins.len() >= MAX_EDGES {
            return Err(fail("import edge limit exceeded"));
        }
        self.edges.insert(key, value);
        Ok(())
    }
    fn builtin(&mut self, from: &Path, module: &str) -> Result<(), String> {
        self.builtins.insert((canonical(from)?, module.into()));
        if self.edges.len() + self.builtins.len() > MAX_EDGES {
            return Err(fail("import edge limit exceeded"));
        }
        Ok(())
    }
    fn depth(&mut self, depth: usize) -> Result<(), String> {
        if depth > 64 {
            Err(fail("import depth limit exceeded"))
        } else {
            Ok(())
        }
    }
}

fn safe_path(text: &str) -> Result<PathBuf, String> {
    let path = Path::new(text);
    if text.is_empty()
        || text.len() > 4096
        || text.contains('\\')
        || text.contains(':')
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || text
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(fail("inventory paths must be normalized relative paths"));
    }
    Ok(path.to_path_buf())
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 4096)
        .ok_or_else(|| fail(format!("missing or invalid {key}")))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| fail(format!("missing or invalid {key}")))
}

struct Replay {
    root: PathBuf,
    files: BTreeMap<PathBuf, Input>,
    edges: Edges,
    builtins: BTreeSet<(PathBuf, String)>,
    seen_files: BTreeSet<PathBuf>,
    seen_edges: BTreeSet<(PathBuf, String)>,
    seen_builtins: BTreeSet<(PathBuf, String)>,
}

impl Replay {
    fn relative(&self, path: &Path) -> Result<PathBuf, String> {
        let path = canonical(path)?;
        if !path.starts_with(&self.root) {
            return Err(fail("input escapes chosen source root"));
        }
        path.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .map_err(fail)
    }
    fn contained_file(&self, path: &Path) -> Result<(), String> {
        let relative = self.relative(path)?;
        if !self.files.contains_key(&relative) {
            return Err(fail("undeclared input"));
        }
        let metadata = fs::metadata(path).map_err(fail)?;
        if !metadata.is_file() {
            return Err(fail("input is not a regular file"));
        }
        let expected = &self.files[&relative];
        if metadata.len() != expected.bytes || metadata.len() > MAX_FILE_BYTES {
            return Err(fail("input byte length mismatch or limit exceeded"));
        }
        Ok(())
    }
}

impl ImportTrace for Replay {
    fn resolve(&mut self, from: &Path, specifier: &str) -> Result<Option<TracedTarget>, String> {
        let from = self.relative(from)?;
        let key = (from, specifier.into());
        let (target, namespace) = self
            .edges
            .get(&key)
            .ok_or_else(|| fail("parsed import has no authenticated edge"))?;
        self.seen_edges.insert(key);
        let target = self.root.join(target);
        self.contained_file(&target)?;
        Ok(Some(TracedTarget {
            path: target,
            namespace: namespace.clone(),
        }))
    }
    fn builtin(&mut self, from: &Path, module: &str) -> Result<(), String> {
        let key = (self.relative(from)?, module.into());
        if !self.builtins.contains(&key) {
            return Err(fail("parsed builtin has no authenticated declaration"));
        }
        self.seen_builtins.insert(key);
        Ok(())
    }
    fn before_read(&mut self, path: &Path) -> Result<(), String> {
        self.contained_file(path)
    }
    fn parsed(&mut self, path: &Path, source: &str) -> Result<(), String> {
        let path = self.relative(path)?;
        if self.files.get(&path) != Some(&input(source)) {
            return Err(fail("input content mismatch"));
        }
        self.seen_files.insert(path);
        Ok(())
    }
    fn depth(&mut self, depth: usize) -> Result<(), String> {
        if depth > 64 {
            Err(fail("import depth limit exceeded"))
        } else {
            Ok(())
        }
    }
}

pub(crate) fn verify(manifest: &str, root: &Path) -> Result<(), String> {
    let manifest: Value = serde_json::from_str(manifest).map_err(fail)?;
    let binding = manifest
        .get("source_binding")
        .filter(|v| v.is_object())
        .ok_or_else(|| fail("manifest has no input inventory"))?;
    if binding.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(fail("unsupported inventory version"));
    }
    let entry = safe_path(text(binding, "entry")?)?;
    let raw_files = array(binding, "files")?;
    let raw_edges = array(binding, "imports")?;
    let raw_builtins = array(binding, "builtins")?;
    if raw_files.is_empty()
        || raw_files.len() > MAX_FILES
        || raw_edges.len() + raw_builtins.len() > MAX_EDGES
    {
        return Err(fail("inventory resource limit exceeded"));
    }
    let mut files = BTreeMap::new();
    let mut total = 0;
    for file in raw_files {
        let path = safe_path(text(file, "path")?)?;
        let sha256 = text(file, "sha256")?;
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(fail("malformed SHA-256"));
        }
        let bytes = file
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| fail("invalid input byte length"))?;
        if bytes > MAX_FILE_BYTES {
            return Err(fail("per-file byte limit exceeded"));
        }
        total += bytes;
        if total > MAX_TOTAL_BYTES {
            return Err(fail("total byte limit exceeded"));
        }
        if files
            .insert(
                path,
                Input {
                    sha256: sha256.into(),
                    bytes,
                },
            )
            .is_some()
        {
            return Err(fail("duplicate inventory input"));
        }
    }
    if !files.contains_key(&entry) {
        return Err(fail("entry missing from inventory"));
    }
    let mut edges = Edges::new();
    for edge in raw_edges {
        let from = safe_path(text(edge, "from")?)?;
        let target = safe_path(text(edge, "target")?)?;
        let specifier = text(edge, "specifier")?.to_string();
        let namespace = match edge.get("namespace") {
            Some(Value::Null) => None,
            Some(Value::String(s)) if !s.is_empty() && s.len() <= 4096 => Some(s.clone()),
            _ => return Err(fail("invalid import namespace")),
        };
        if !files.contains_key(&from) || !files.contains_key(&target) {
            return Err(fail("import edge names undeclared input"));
        }
        if edges
            .insert((from, specifier), (target, namespace))
            .is_some()
        {
            return Err(fail("duplicate import edge"));
        }
    }
    let mut builtins = BTreeSet::new();
    for builtin in raw_builtins {
        let from = safe_path(text(builtin, "from")?)?;
        let module = text(builtin, "module")?.to_string();
        if !files.contains_key(&from) || !builtins.insert((from, module)) {
            return Err(fail("invalid or duplicate builtin declaration"));
        }
    }
    let root = canonical(root)?;
    if !root.is_dir() {
        return Err(fail("chosen source root is not a directory"));
    }
    let mut replay = Replay {
        root,
        files,
        edges,
        builtins,
        seen_files: BTreeSet::new(),
        seen_edges: BTreeSet::new(),
        seen_builtins: BTreeSet::new(),
    };
    // Check every declared path before opening any source, including symlinks and FIFOs.
    for path in replay.files.keys() {
        replay.contained_file(&replay.root.join(path))?;
    }
    let entry = replay.root.join(entry);
    let file = fs::File::open(&entry).map_err(fail)?;
    let mut source = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut source)
        .map_err(fail)?;
    replay.parsed(&entry, &source)?;
    let (mut program, errors) = crate::parse(&source);
    if !errors.is_empty() {
        return Err(fail("entry parser rejected input"));
    }
    let mut loaded = HashSet::new();
    loaded.insert(canonical(&entry)?);
    crate::imports::expand_uses_traced(
        &mut program,
        &entry,
        &mut loaded,
        &mut Vec::new(),
        &mut replay,
        false,
    )
    .map_err(fail)?;
    if replay.seen_files.len() != replay.files.len()
        || replay.seen_edges.len() != replay.edges.len()
        || replay.seen_builtins.len() != replay.builtins.len()
    {
        return Err(fail("unreachable input or unused import edge"));
    }
    Ok(())
}
