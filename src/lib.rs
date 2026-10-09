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
    for source in sources {
        let spec: Value = serde_norway::from_str(&source.yaml)
            .with_context(|| format!("{}: invalid YAML", source.file))?;
        validate(source, &spec)?;
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
            let base = base_path(&spec);
            for (path, item) in spec_paths {
                let path = path.as_str().unwrap_or_default();
                let path = join_paths(&base, path);
                let Value::Mapping(item) = item else { continue };
                let merged = paths.entry(path.clone().into()).or_insert_with(|| Mapping::new().into());
                let merged = merged.as_mapping_mut().unwrap();
                for (key, value) in item {
                    if merged.contains_key(key) {
                        if let Some(method) = key.as_str().filter(|k| METHODS.contains(k)) {
                            let first = &owners[&(path.clone(), method.to_string())];
                            anyhow::bail!(
                                "{path} {method} is defined in both {first} and {}",
                                source.file
                            );
                        }
                        continue;
                    }
                    if let Some(method) = key.as_str().filter(|k| METHODS.contains(k)) {
                        owners.insert((path.clone(), method.to_string()), source.file.clone());
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
    if !tags.is_empty() {
        root.insert("tags".into(), Value::Sequence(tags.into_iter().map(|(t, _)| t).collect()));
    }
    root.insert("paths".into(), Value::Mapping(paths));
    if !components.is_empty() {
        root.insert("components".into(), Value::Mapping(components));
    }
    Ok(serde_norway::to_string(&Value::Mapping(root))?)
}

/// The path part of the first `servers[].url`, or empty (`/`) if none.
fn base_path(spec: &Value) -> String {
    let url = spec["servers"][0]["url"].as_str().unwrap_or("/");
    match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("", |i| &rest[i..]).to_string(),
        None => url.to_string(),
    }
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
