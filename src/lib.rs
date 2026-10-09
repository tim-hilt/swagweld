use anyhow::{Context, Result};
use serde_norway::{Mapping, Value};
use std::collections::{BTreeMap, HashMap};

/// Component sections whose entries are referenced by `$ref`, so Collisions can be renamed.
/// Other sections (e.g. `securitySchemes`, referenced by name from `security`) fail instead.
const SECTIONS: [&str; 9] = [
    "schemas",
    "parameters",
    "responses",
    "requestBodies",
    "headers",
    "examples",
    "links",
    "callbacks",
    "pathItems",
];
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];
/// Source Spec file names: exactly one of these, or `<name>.` followed by one.
const SPEC_FILE_NAMES: [&str; 2] = ["swagger.yaml", "swagger.yml"];

/// A discovered Source Spec: its relative path and raw YAML text.
pub struct Source {
    pub file: String,
    pub yaml: String,
}

/// The Bundle's own `info` fields.
pub struct Info {
    pub title: String,
    pub version: String,
    pub description: Option<String>,
}

/// A welded Bundle and the warnings about Collisions resolved by renaming.
pub struct Bundle {
    pub yaml: String,
    pub warnings: Vec<String>,
}

/// Whether a file name marks a Source Spec.
pub fn is_source_spec(file_name: &str) -> bool {
    SPEC_FILE_NAMES
        .iter()
        .any(|s| file_name == *s || file_name.ends_with(&format!(".{s}")))
}

/// Weld Source Specs (already in sorted order) into a Bundle. Source paths are relative to a
/// root directory named `root_name`, which is the Spec Name of a root-level `swagger.yaml`.
pub fn bundle(sources: &[Source], info: &Info, root_name: &str) -> Result<Bundle> {
    let mut specs = sources
        .iter()
        .map(|source| Spec::parse(source, root_name))
        .collect::<Result<Vec<_>>>()?;
    // Upgrade before Collision detection, so equivalent 3.0 and 3.1 schemas dedupe.
    let openapi = unify_versions(&mut specs);
    let dialect = shared_dialect(&specs)?;
    let mut warnings = rename_components(&mut specs);
    warnings.extend(rename_webhooks(&mut specs));
    warnings.extend(rename_operation_ids(&mut specs)?);
    let shared = specs
        .windows(2)
        .all(|w| same_set(&w[0].origins, &w[1].origins));
    let tags = merge_tags(&specs)?;
    let paths = merge_paths(&specs, shared)?;
    let webhooks = merge_webhooks(&specs)?;
    let components = merge_components(&specs)?;

    let mut info_map = Mapping::new();
    info_map.insert("title".into(), info.title.as_str().into());
    info_map.insert("version".into(), info.version.as_str().into());
    if let Some(d) = &info.description {
        info_map.insert("description".into(), d.as_str().into());
    }

    let mut root = Mapping::new();
    root.insert("openapi".into(), openapi.into());
    root.insert("info".into(), Value::Mapping(info_map));
    if let Some(dialect) = dialect {
        root.insert("jsonSchemaDialect".into(), dialect);
    }
    if let Some(spec) = specs.first().filter(|s| shared && !s.origins.is_empty()) {
        root.insert("servers".into(), Value::Sequence(spec.origins.clone()));
    }
    if !tags.is_empty() {
        root.insert("tags".into(), Value::Sequence(tags));
    }
    root.insert("paths".into(), Value::Mapping(paths));
    if !webhooks.is_empty() {
        root.insert("webhooks".into(), Value::Mapping(webhooks));
    }
    if !components.is_empty() {
        root.insert("components".into(), Value::Mapping(components));
    }
    let yaml = serde_norway::to_string(&Value::Mapping(root))?;
    Ok(Bundle { yaml, warnings })
}

/// One parsed and validated Source Spec.
struct Spec<'a> {
    file: &'a str,
    /// The Spec Name, prefixing this spec's renamed Collisions.
    name: String,
    doc: Value,
    /// Path part shared by all `servers`, prepended to every path.
    base_path: String,
    /// Host parts of the absolute `servers`.
    origins: Vec<Value>,
}

impl<'a> Spec<'a> {
    fn parse(source: &'a Source, root_name: &str) -> Result<Self> {
        let doc: Value = serde_norway::from_str(&source.yaml)
            .with_context(|| format!("{}: invalid YAML", source.file))?;
        validate(source, &doc)?;
        let (base_path, origins) = split_servers(&source.file, &doc)?;
        Ok(Spec {
            file: &source.file,
            name: spec_name(&source.file, root_name),
            doc,
            base_path,
            origins,
        })
    }
}

/// The Spec Name: filename prefix of `<name>.swagger.yaml`, else the parent directory's name.
fn spec_name(file: &str, root_name: &str) -> String {
    let path = std::path::Path::new(file);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    for suffix in SPEC_FILE_NAMES {
        if let Some(prefix) = name.strip_suffix(&format!(".{suffix}")) {
            return prefix.to_string();
        }
    }
    match path.parent().and_then(|p| p.file_name()) {
        Some(dir) => dir.to_string_lossy().into(),
        None => root_name.to_string(),
    }
}

/// The highest `openapi` version of all specs. If it is above 3.0, 3.0 specs are upgraded to it.
fn unify_versions(specs: &mut [Spec]) -> String {
    let openapi = specs
        .iter()
        .filter_map(|s| s.doc["openapi"].as_str())
        .max_by_key(|v| version_key(v))
        .unwrap_or("3.0.0")
        .to_string();
    if !openapi.starts_with("3.0") {
        for spec in specs.iter_mut().filter(|s| {
            s.doc["openapi"]
                .as_str()
                .is_some_and(|v| v.starts_with("3.0"))
        }) {
            upgrade_schemas(&mut spec.doc);
        }
    }
    openapi
}

/// The `jsonSchemaDialect` all specs declaring one agree on.
fn shared_dialect(specs: &[Spec]) -> Result<Option<Value>> {
    let mut dialect: Option<(&Value, &str)> = None;
    for spec in specs {
        match (spec.doc.get("jsonSchemaDialect"), dialect) {
            (Some(d), Some((first, file))) if d != first => {
                anyhow::bail!("jsonSchemaDialect differs between {file} and {}", spec.file)
            }
            (Some(d), None) => dialect = Some((d, spec.file)),
            _ => {}
        }
    }
    Ok(dialect.map(|(d, _)| d.clone()))
}

/// Resolve one Collision: rename `name` to `<SpecName>_<name>` in each spec of `users`,
/// via `apply(doc, new_name)`. `label` names the colliding thing in the warnings.
/// Returns one warning per rename.
fn rename(
    specs: &mut [Spec],
    label: &str,
    name: &str,
    users: &[usize],
    apply: impl Fn(&mut Value, &str),
) -> Vec<String> {
    let files: Vec<&str> = users.iter().map(|&i| specs[i].file).collect();
    let files = files.join(", ");
    users
        .iter()
        .map(|&i| {
            let spec = &mut specs[i];
            let new = format!("{}_{name}", spec.name);
            apply(&mut spec.doc, &new);
            format!(
                "{label} collides between {files}; renamed to {new} in {}",
                spec.file
            )
        })
        .collect()
}

/// The spec indexes of every name whose values differ between specs.
fn differing<K>(groups: BTreeMap<K, Vec<(usize, &Value)>>) -> Vec<(K, Vec<usize>)> {
    groups
        .into_iter()
        .filter(|(_, g)| g.iter().any(|(_, v)| *v != g[0].1))
        .map(|(k, g)| (k, g.into_iter().map(|(i, _)| i).collect()))
        .collect()
}

/// Rename the key `old` of a mapping to `new`, if present.
fn rename_key(map: Option<&mut Value>, old: &str, new: &str) {
    if let Some(map) = map.and_then(Value::as_mapping_mut)
        && let Some(value) = map.remove(old)
    {
        map.insert(new.into(), value);
    }
}

/// Rename differing components of the same name, rewriting local `$ref`s to them.
fn rename_components(specs: &mut [Spec]) -> Vec<String> {
    let mut groups: BTreeMap<(&str, String), Vec<(usize, &Value)>> = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        for section in SECTIONS {
            let entries = spec.doc.get("components").and_then(|c| c.get(section));
            for (name, value) in entries.and_then(Value::as_mapping).into_iter().flatten() {
                if let Some(name) = name.as_str() {
                    groups
                        .entry((section, name.to_string()))
                        .or_default()
                        .push((i, value));
                }
            }
        }
    }
    let mut warnings = Vec::new();
    for ((section, name), users) in differing(groups) {
        let label = format!("components.{section}.{name}");
        warnings.extend(rename(specs, &label, &name, &users, |doc, new| {
            let section_map = doc.get_mut("components").and_then(|c| c.get_mut(section));
            rename_key(section_map, &name, new);
            let prefix = format!("#/components/{section}/");
            rewrite_refs(doc, &format!("{prefix}{name}"), &format!("{prefix}{new}"));
        }));
    }
    warnings
}

/// Rename differing webhooks of the same name.
fn rename_webhooks(specs: &mut [Spec]) -> Vec<String> {
    let mut groups: BTreeMap<String, Vec<(usize, &Value)>> = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        let hooks = spec.doc.get("webhooks").and_then(Value::as_mapping);
        for (name, value) in hooks.into_iter().flatten() {
            let name = name.as_str().unwrap_or_default().to_string();
            groups.entry(name).or_default().push((i, value));
        }
    }
    let mut warnings = Vec::new();
    for (name, users) in differing(groups) {
        let label = format!("webhooks.{name}");
        warnings.extend(rename(specs, &label, &name, &users, |doc, new| {
            rename_key(doc.get_mut("webhooks"), &name, new)
        }));
    }
    warnings
}

/// The operation objects of one spec.
fn operations(spec: &mut Value) -> impl Iterator<Item = &mut Mapping> {
    let items = spec
        .get_mut("paths")
        .and_then(Value::as_mapping_mut)
        .into_iter()
        .flat_map(|p| p.values_mut());
    items
        .filter_map(Value::as_mapping_mut)
        .flat_map(|item| {
            item.iter_mut()
                .filter(|(k, _)| METHODS.contains(&k.as_str().unwrap_or_default()))
        })
        .filter_map(|(_, op)| op.as_mapping_mut())
}

/// Rename `operationId`s used by several specs, rewriting `links` to them.
/// Fails if the renamed ids still collide.
fn rename_operation_ids(specs: &mut [Spec]) -> Result<Vec<String>> {
    let mut users: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, spec) in specs.iter_mut().enumerate() {
        for op in operations(&mut spec.doc) {
            if let Some(id) = op.get("operationId").and_then(Value::as_str) {
                let list = users.entry(id.to_string()).or_default();
                if !list.contains(&i) {
                    list.push(i);
                }
            }
        }
    }
    let mut warnings = Vec::new();
    for (id, users) in users.iter().filter(|(_, u)| u.len() > 1) {
        let label = format!("operationId {id}");
        warnings.extend(rename(specs, &label, id, users, |doc, new| {
            for op in operations(doc) {
                if op.get("operationId").and_then(Value::as_str) == Some(id) {
                    op.insert("operationId".into(), new.into());
                }
            }
            rewrite_links(doc, id, new);
        }));
    }
    let mut seen: HashMap<String, &str> = HashMap::new();
    for spec in specs.iter_mut() {
        for op in operations(&mut spec.doc) {
            let Some(id) = op.get("operationId").and_then(Value::as_str) else {
                continue;
            };
            match seen.insert(id.to_string(), spec.file) {
                Some(first) if first != spec.file => anyhow::bail!(
                    "operationId {id} is used by both {first} and {} even after renaming",
                    spec.file
                ),
                _ => {}
            }
        }
    }
    Ok(warnings)
}

fn rewrite_links(node: &mut Value, old: &str, new: &str) {
    match node {
        Value::Mapping(map) => {
            for (key, value) in map.iter_mut() {
                if key.as_str() == Some("links") {
                    for link in value
                        .as_mapping_mut()
                        .into_iter()
                        .flat_map(|l| l.values_mut())
                    {
                        if let Some(Value::String(id)) = link.get_mut("operationId")
                            && id == old
                        {
                            *id = new.to_string();
                        }
                    }
                }
                rewrite_links(value, old, new);
            }
        }
        Value::Sequence(items) => items.iter_mut().for_each(|v| rewrite_links(v, old, new)),
        _ => {}
    }
}

/// Point every `$ref` to `old` (or into it, `old/...`) at `new` instead.
fn rewrite_refs(node: &mut Value, old: &str, new: &str) {
    match node {
        Value::Mapping(map) => {
            for (key, value) in map.iter_mut() {
                match (key.as_str(), &mut *value) {
                    (Some("$ref"), Value::String(r)) => {
                        if let Some(tail) = r.strip_prefix(old)
                            && (tail.is_empty() || tail.starts_with('/'))
                        {
                            *r = format!("{new}{tail}");
                        }
                    }
                    _ => rewrite_refs(value, old, new),
                }
            }
        }
        Value::Sequence(items) => items.iter_mut().for_each(|v| rewrite_refs(v, old, new)),
        _ => {}
    }
}

/// Insert `entries` into `into`, deduping identical ones. Fails on a name already present with
/// different content (a Collision left after renaming). `label` prefixes names in errors;
/// `owners` remembers which file contributed each labelled name first.
fn merge_entries<'a>(
    into: &mut Mapping,
    label: &str,
    entries: &Mapping,
    file: &'a str,
    owners: &mut HashMap<String, &'a str>,
) -> Result<()> {
    for (name, value) in entries {
        let key = format!("{label}.{}", name.as_str().unwrap_or_default());
        match into.get(name) {
            Some(existing) if existing != value => anyhow::bail!(
                "{key} is defined differently in {} and {file}",
                owners[&key]
            ),
            Some(_) => {}
            None => {
                owners.insert(key, file);
                into.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(())
}

fn merge_components(specs: &[Spec]) -> Result<Mapping> {
    let mut components = Mapping::new();
    let mut owners = HashMap::new();
    for spec in specs {
        let sections = spec.doc.get("components").and_then(Value::as_mapping);
        for (section, entries) in sections.into_iter().flatten() {
            let (Some(section), Some(entries)) = (section.as_str(), entries.as_mapping()) else {
                continue;
            };
            let into = components
                .entry(section.into())
                .or_insert_with(|| Mapping::new().into());
            let label = format!("components.{section}");
            let into = into.as_mapping_mut().unwrap();
            merge_entries(into, &label, entries, spec.file, &mut owners)?;
        }
    }
    Ok(components)
}

fn merge_webhooks(specs: &[Spec]) -> Result<Mapping> {
    let mut webhooks = Mapping::new();
    let mut owners = HashMap::new();
    for spec in specs {
        if let Some(entries) = spec.doc.get("webhooks").and_then(Value::as_mapping) {
            merge_entries(&mut webhooks, "webhooks", entries, spec.file, &mut owners)?;
        }
    }
    Ok(webhooks)
}

/// All tags in first-seen order. Fails on a tag name defined differently.
fn merge_tags(specs: &[Spec]) -> Result<Vec<Value>> {
    let mut tags: Vec<(&Value, &str)> = Vec::new();
    for spec in specs {
        let spec_tags = spec.doc.get("tags").and_then(Value::as_sequence);
        for tag in spec_tags.into_iter().flatten() {
            match tags.iter().find(|(t, _)| t.get("name") == tag.get("name")) {
                Some((existing, _)) if *existing == tag => {}
                Some((_, first)) => anyhow::bail!(
                    "tag {} is defined differently in {first} and {}",
                    tag.get("name").and_then(Value::as_str).unwrap_or_default(),
                    spec.file
                ),
                None => tags.push((tag, spec.file)),
            }
        }
    }
    Ok(tags.into_iter().map(|(t, _)| t.clone()).collect())
}

/// All paths, prefixed with their spec's Base Path. Path items from several specs merge
/// field by field. Operations inherit their spec's top-level `security`; unless all specs
/// `share` the same server origins, path items carry their spec's origins as `servers`.
fn merge_paths(specs: &[Spec], shared: bool) -> Result<Mapping> {
    let mut paths = Mapping::new();
    let mut owners: HashMap<(String, String), &str> = HashMap::new();
    let mut templates: HashMap<String, (String, &str)> = HashMap::new();
    for spec in specs {
        let spec_paths = spec.doc.get("paths").and_then(Value::as_mapping);
        for (path, item) in spec_paths.into_iter().flatten() {
            let path = join_paths(&spec.base_path, path.as_str().unwrap_or_default());
            let Value::Mapping(item) = item else { continue };
            let mut item = item.clone();
            if !shared && !spec.origins.is_empty() {
                item.insert("servers".into(), Value::Sequence(spec.origins.clone()));
            }
            let (first_path, first) = templates
                .entry(template_shape(&path))
                .or_insert_with(|| (path.clone(), spec.file));
            if *first_path != path {
                anyhow::bail!(
                    "{first_path} in {first} and {path} in {} differ only in template names",
                    spec.file
                );
            }
            let merged = paths
                .entry(path.clone().into())
                .or_insert_with(|| Mapping::new().into());
            let merged = merged.as_mapping_mut().unwrap();
            for (key, value) in &item {
                if merged.contains_key(key) {
                    if let Some(method) = key.as_str().filter(|k| METHODS.contains(k)) {
                        let first = owners[&(path.clone(), method.to_string())];
                        anyhow::bail!(
                            "{path} {method} is defined in both {first} and {}",
                            spec.file
                        );
                    }
                    if merged[key] != *value {
                        let field = key.as_str().unwrap_or_default();
                        let first = owners[&(path.clone(), field.to_string())];
                        anyhow::bail!(
                            "{path} {field} is defined differently in {first} and {}",
                            spec.file
                        );
                    }
                    continue;
                }
                if let Some(field) = key.as_str() {
                    owners.insert((path.clone(), field.to_string()), spec.file);
                }
                let mut value = value.clone();
                if let (Some(security), Value::Mapping(op)) = (spec.doc.get("security"), &mut value)
                    && METHODS.contains(&key.as_str().unwrap_or_default())
                {
                    op.entry("security".into())
                        .or_insert_with(|| security.clone());
                }
                merged.insert(key.clone(), value);
            }
        }
    }
    Ok(paths)
}

/// A Source Spec's Base Path and its server origins (host part only).
/// Relative URLs have no origin. All servers must agree on the path part.
fn split_servers(file: &str, spec: &Value) -> Result<(String, Vec<Value>)> {
    let mut base: Option<String> = None;
    let mut origins: Vec<Value> = Vec::new();
    let servers = spec
        .get("servers")
        .and_then(Value::as_sequence)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for server in servers {
        let url = server.get("url").and_then(Value::as_str).unwrap_or("/");
        let (origin, path) = match url.split_once("://") {
            Some((scheme, rest)) => {
                let i = rest.find('/').unwrap_or(rest.len());
                (
                    Some(format!("{scheme}://{}", &rest[..i])),
                    rest[i..].to_string(),
                )
            }
            None => (None, url.to_string()),
        };
        let mut path = path;
        if let Some(Value::Mapping(vars)) = server.get("variables") {
            for (name, var) in vars {
                if let (Some(name), Some(default)) =
                    (name.as_str(), var.get("default").and_then(Value::as_str))
                {
                    path = path.replace(&format!("{{{name}}}"), default);
                }
            }
        }
        let path = join_paths("", &path);
        match &base {
            Some(b) if *b != path => {
                anyhow::bail!("{file}: servers declare different path parts ({b} and {path})")
            }
            _ => base = Some(path),
        }
        let Some(origin) = origin else { continue };
        let mut entry = Mapping::new();
        entry.insert("url".into(), origin.as_str().into());
        if let Some(Value::Mapping(vars)) = server.get("variables") {
            let used: Mapping = vars
                .iter()
                .filter(|(name, _)| {
                    origin.contains(&format!("{{{}}}", name.as_str().unwrap_or_default()))
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !used.is_empty() {
                entry.insert("variables".into(), Value::Mapping(used));
            }
        }
        let entry = Value::Mapping(entry);
        if !origins.contains(&entry) {
            origins.push(entry);
        }
    }
    Ok((base.unwrap_or_default(), origins))
}

fn same_set(a: &[Value], b: &[Value]) -> bool {
    a.iter().all(|x| b.contains(x)) && b.iter().all(|x| a.contains(x))
}

/// The path with every `{param}` name erased, so `/a/{id}` and `/a/{x}` match.
fn template_shape(path: &str) -> String {
    let mut out = String::new();
    let mut in_param = false;
    for c in path.chars() {
        match c {
            '{' => in_param = true,
            '}' => {
                in_param = false;
                out.push_str("{}");
            }
            _ if !in_param => out.push(c),
            _ => {}
        }
    }
    out
}

/// Join with exactly one slash between segments.
fn join_paths(base: &str, path: &str) -> String {
    let segments: Vec<&str> = base
        .split('/')
        .chain(path.split('/'))
        .filter(|s| !s.is_empty())
        .collect();
    format!("/{}", segments.join("/"))
}

/// Numeric version parts, so `3.0.10` orders above `3.0.3`.
fn version_key(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Rewrite OpenAPI 3.0 schema keywords into their 3.1 meaning, in place:
/// `nullable: true` adds `"null"` to `type`, or wraps a type-less schema (e.g. a `$ref`)
/// in `anyOf` with `{type: "null"}`; boolean `exclusiveMinimum/Maximum` absorb
/// `minimum/maximum`. Example data, `enum`/`const` values and `x-` extensions are left
/// untouched, since they may legitimately contain these keys as data.
fn upgrade_schemas(node: &mut Value) {
    match node {
        Value::Mapping(map) => {
            for (key, value) in map.iter_mut() {
                let key = key.as_str().unwrap_or_default();
                if !key.starts_with("x-")
                    && !["example", "examples", "enum", "const"].contains(&key)
                {
                    upgrade_schemas(value);
                }
            }
            for (exclusive, bound) in [
                ("exclusiveMinimum", "minimum"),
                ("exclusiveMaximum", "maximum"),
            ] {
                let Some(&Value::Bool(on)) = map.get(exclusive) else {
                    continue;
                };
                let bound = if on { map.shift_remove(bound) } else { None };
                match bound {
                    Some(bound) => map[exclusive] = bound,
                    None => drop(map.shift_remove(exclusive)),
                }
            }
            let Some(&Value::Bool(nullable)) = map.get("nullable") else {
                return;
            };
            map.shift_remove("nullable");
            if !nullable {
                return;
            }
            let null: Value = serde_norway::from_str("type: 'null'").unwrap();
            match map.get("type").cloned() {
                Some(t @ Value::String(_)) => map["type"] = Value::Sequence(vec![t, "null".into()]),
                _ => {
                    let schema = Value::Mapping(std::mem::take(map));
                    map.insert("anyOf".into(), Value::Sequence(vec![schema, null]));
                }
            }
        }
        Value::Sequence(items) => items.iter_mut().for_each(upgrade_schemas),
        _ => {}
    }
}

/// Reject non-3.x specs and external `$ref`s.
fn validate(source: &Source, spec: &Value) -> Result<()> {
    let file = &source.file;
    let version = spec.get("openapi").and_then(Value::as_str);
    match version {
        None => anyhow::bail!("{file}: missing `openapi` key"),
        Some(v) if !v.starts_with("3.") => anyhow::bail!("{file}: unsupported openapi version {v}"),
        _ => {}
    }
    check_refs(file, spec, &mut String::new())
}

fn check_refs(file: &str, node: &Value, pointer: &mut String) -> Result<()> {
    let len = pointer.len();
    match node {
        Value::Mapping(map) => {
            for (key, value) in map {
                if key.as_str() == Some("$ref")
                    && let Some(r) = value.as_str().filter(|r| !r.starts_with('#'))
                {
                    anyhow::bail!("{file}: external $ref `{r}` at {pointer}");
                }
                let key = match key {
                    Value::String(s) => s.clone(),
                    other => serde_norway::to_string(other)?.trim().to_string(),
                };
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                check_refs(file, value, pointer)?;
                pointer.truncate(len);
            }
        }
        Value::Sequence(items) => {
            for (i, value) in items.iter().enumerate() {
                pointer.push_str(&format!("/{i}"));
                check_refs(file, value, pointer)?;
                pointer.truncate(len);
            }
        }
        _ => {}
    }
    Ok(())
}
