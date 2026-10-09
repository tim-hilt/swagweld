use anyhow::{Context, Result};
use serde_norway::{Mapping, Value};
use std::collections::HashMap;

const METHODS: [&str; 8] = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];

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

/// Weld Source Specs (already in sorted order) into Bundle YAML.
pub fn bundle(sources: &[Source], info: &Info) -> Result<String> {
    let mut paths = Mapping::new();
    let mut tags: Vec<(Value, String)> = Vec::new();
    let mut components = Mapping::new();
    let mut owners: HashMap<(String, String), String> = HashMap::new();
    let mut templates: HashMap<String, (String, String)> = HashMap::new();
    let mut specs = Vec::new();
    for source in sources {
        let spec: Value = serde_norway::from_str(&source.yaml)
            .with_context(|| format!("{}: invalid YAML", source.file))?;
        validate(source, &spec)?;
        let (base, origins) = split_servers(&source.file, &spec)?;
        specs.push((spec, base, origins));
    }
    let shared = specs.windows(2).all(|w| same_set(&w[0].2, &w[1].2));
    let mut top_servers = Vec::new();
    if shared {
        top_servers = specs.first().map(|s| s.2.clone()).unwrap_or_default();
    }
    for (source, (spec, base, origins)) in sources.iter().zip(&specs) {
        if let Some(Value::Sequence(spec_tags)) = spec.get("tags") {
            for tag in spec_tags {
                match tags.iter().find(|(t, _)| t.get("name") == tag.get("name")) {
                    Some((existing, _)) if existing == tag => {}
                    Some((_, first)) => anyhow::bail!(
                        "tag {} is defined differently in {first} and {}",
                        tag.get("name").and_then(Value::as_str).unwrap_or_default(),
                        source.file
                    ),
                    None => tags.push((tag.clone(), source.file.clone())),
                }
            }
        }
        if let Some(Value::Mapping(sections)) = spec.get("components") {
            for (section, entries) in sections {
                let (Some(entries), Value::Mapping(into)) = (
                    entries.as_mapping(),
                    components.entry(section.clone()).or_insert_with(|| Mapping::new().into()),
                ) else {
                    continue;
                };
                for (name, value) in entries {
                    into.entry(name.clone()).or_insert_with(|| value.clone());
                }
            }
        }
        if let Some(Value::Mapping(spec_paths)) = spec.get("paths") {
            for (path, item) in spec_paths {
                let path = path.as_str().unwrap_or_default();
                let path = join_paths(base, path);
                let Value::Mapping(item) = item else { continue };
                let mut item = item.clone();
                if !shared && !origins.is_empty() {
                    item.insert("servers".into(), Value::Sequence(origins.clone()));
                }
                let (first_path, first) = templates
                    .entry(template_shape(&path))
                    .or_insert_with(|| (path.clone(), source.file.clone()));
                if *first_path != path {
                    anyhow::bail!(
                        "{first_path} in {first} and {path} in {} differ only in template names",
                        source.file
                    );
                }
                let merged = paths.entry(path.clone().into()).or_insert_with(|| Mapping::new().into());
                let merged = merged.as_mapping_mut().unwrap();
                for (key, value) in &item {
                    if merged.contains_key(key) {
                        if let Some(method) = key.as_str().filter(|k| METHODS.contains(k)) {
                            let first = &owners[&(path.clone(), method.to_string())];
                            anyhow::bail!(
                                "{path} {method} is defined in both {first} and {}",
                                source.file
                            );
                        }
                        if merged[key] != *value {
                            let field = key.as_str().unwrap_or_default();
                            let first = &owners[&(path.clone(), field.to_string())];
                            anyhow::bail!(
                                "{path} {field} is defined differently in {first} and {}",
                                source.file
                            );
                        }
                        continue;
                    }
                    if let Some(field) = key.as_str() {
                        owners.insert((path.clone(), field.to_string()), source.file.clone());
                    }
                    let mut value = value.clone();
                    if let (Some(security), Value::Mapping(op)) = (spec.get("security"), &mut value) {
                        if METHODS.contains(&key.as_str().unwrap_or_default()) {
                            op.entry("security".into()).or_insert_with(|| security.clone());
                        }
                    }
                    merged.insert(key.clone(), value);
                }
            }
        }
    }

    let mut info_map = Mapping::new();
    info_map.insert("title".into(), info.title.as_str().into());
    info_map.insert("version".into(), info.version.as_str().into());
    if let Some(d) = &info.description {
        info_map.insert("description".into(), d.as_str().into());
    }

    let mut root = Mapping::new();
    root.insert("openapi".into(), "3.0.0".into());
    root.insert("info".into(), Value::Mapping(info_map));
    if !top_servers.is_empty() {
        root.insert("servers".into(), Value::Sequence(top_servers));
    }
    if !tags.is_empty() {
        root.insert("tags".into(), Value::Sequence(tags.into_iter().map(|(t, _)| t).collect()));
    }
    root.insert("paths".into(), Value::Mapping(paths));
    if !components.is_empty() {
        root.insert("components".into(), Value::Mapping(components));
    }
    Ok(serde_norway::to_string(&Value::Mapping(root))?)
}

/// A Source Spec's Base Path and its server origins (host part only).
/// Relative URLs have no origin. All servers must agree on the path part.
fn split_servers(file: &str, spec: &Value) -> Result<(String, Vec<Value>)> {
    let mut base: Option<String> = None;
    let mut origins: Vec<Value> = Vec::new();
    let servers = spec.get("servers").and_then(Value::as_sequence).map(Vec::as_slice).unwrap_or_default();
    for server in servers {
        let url = server.get("url").and_then(Value::as_str).unwrap_or("/");
        let (origin, path) = match url.split_once("://") {
            Some((scheme, rest)) => {
                let i = rest.find('/').unwrap_or(rest.len());
                (Some(format!("{scheme}://{}", &rest[..i])), rest[i..].to_string())
            }
            None => (None, url.to_string()),
        };
        let mut path = path;
        if let Some(Value::Mapping(vars)) = server.get("variables") {
            for (name, var) in vars {
                if let (Some(name), Some(default)) = (name.as_str(), var.get("default").and_then(Value::as_str)) {
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
                .filter(|(name, _)| origin.contains(&format!("{{{}}}", name.as_str().unwrap_or_default())))
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
    let segments: Vec<&str> = base.split('/').chain(path.split('/')).filter(|s| !s.is_empty()).collect();
    format!("/{}", segments.join("/"))
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
                if key.as_str() == Some("$ref") {
                    if let Some(r) = value.as_str().filter(|r| !r.starts_with('#')) {
                        anyhow::bail!("{file}: external $ref `{r}` at {pointer}");
                    }
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
