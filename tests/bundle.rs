use swagweld::{Info, Source, bundle};

fn src(file: &str, yaml: &str) -> Source {
    Source { file: file.into(), yaml: yaml.into() }
}

fn info() -> Info {
    Info { title: "t".into(), version: "0.0.0".into(), description: None }
}

fn parse(yaml: &str) -> serde_norway::Value {
    serde_norway::from_str(yaml).unwrap()
}

fn paths_of(yaml: &str) -> Vec<String> {
    let v: serde_norway::Value = serde_norway::from_str(yaml).unwrap();
    v["paths"].as_mapping().unwrap().keys().map(|k| k.as_str().unwrap().to_string()).collect()
}

#[test]
fn relative_server_url_prefixes_paths() {
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\nservers:\n  - url: /users/v1\npaths:\n  /items:\n    get: {}\n")],
        &info(),
    )
    .unwrap();
    assert_eq!(paths_of(&out), ["/users/v1/items"]);
}

#[test]
fn absolute_server_url_yields_base_path() {
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\nservers:\n  - url: https://host.example/users/v1/\npaths:\n  /items:\n    get: {}\n")],
        &info(),
    )
    .unwrap();
    assert_eq!(paths_of(&out), ["/users/v1/items"]);
}

#[test]
fn root_base_path_leaves_paths_unchanged() {
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\nservers:\n  - url: /\npaths:\n  /items:\n    get: {}\n")],
        &info(),
    )
    .unwrap();
    assert_eq!(paths_of(&out), ["/items"]);
}

#[test]
fn disjoint_methods_on_same_path_are_merged() {
    let out = bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    post: {}\n"),
        ],
        &info(),
    )
    .unwrap();
    let v: serde_norway::Value = serde_norway::from_str(&out).unwrap();
    let item = v["paths"]["/items"].as_mapping().unwrap();
    assert!(item.contains_key("get") && item.contains_key("post"));
}

#[test]
fn same_path_and_method_fails_naming_both_files() {
    let err = bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n"),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    for needle in ["a.swagger.yaml", "b.swagger.yaml", "/items", "get"] {
        assert!(err.contains(needle), "{err}");
    }
}

fn err_of(yaml: &str) -> String {
    bundle(&[src("bad.swagger.yaml", yaml)], &info()).unwrap_err().to_string()
}

#[test]
fn invalid_yaml_fails_naming_file() {
    assert!(err_of("a: [").contains("bad.swagger.yaml"));
}

#[test]
fn missing_openapi_key_fails_naming_file() {
    let e = err_of("paths: {}\n");
    assert!(e.contains("bad.swagger.yaml") && e.contains("openapi"), "{e}");
}

#[test]
fn non_3x_version_fails_naming_file() {
    let e = err_of("swagger: \"2.0\"\npaths: {}\n");
    assert!(e.contains("bad.swagger.yaml"), "{e}");
    let e = err_of("openapi: 2.0.0\npaths: {}\n");
    assert!(e.contains("bad.swagger.yaml"), "{e}");
}

#[test]
fn external_ref_fails_naming_file_and_location() {
    for r in ["./other.yaml#/Foo", "https://x.example/s.yaml#/Foo"] {
        let e = err_of(&format!(
            "openapi: 3.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          $ref: '{r}'\n"
        ));
        assert!(e.contains("bad.swagger.yaml") && e.contains("/paths/~1a/get/responses/200"), "{e}");
    }
}

#[test]
fn internal_refs_components_and_extensions_are_kept() {
    let out = bundle(
        &[src(
            "a.swagger.yaml",
            "openapi: 3.0.0\nx-top: 1\npaths:\n  /a:\n    x-ext: custom\n    get:\n      responses:\n        '200':\n          $ref: '#/components/responses/Ok'\ncomponents:\n  responses:\n    Ok:\n      description: ok\n",
        )],
        &info(),
    )
    .unwrap();
    let v: serde_norway::Value = serde_norway::from_str(&out).unwrap();
    assert_eq!(v["components"]["responses"]["Ok"]["description"], "ok");
    assert_eq!(v["paths"]["/a"]["x-ext"], "custom");
}

#[test]
fn info_comes_from_arguments_not_source_specs() {
    let i = Info { title: "T".into(), version: "1.2.3".into(), description: Some("D".into()) };
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\ninfo:\n  title: Mine\n  version: '9'\nexternalDocs:\n  url: http://x\npaths: {}\n")],
        &i,
    )
    .unwrap();
    let v = parse(&out);
    assert_eq!(v["info"]["title"], "T");
    assert_eq!(v["info"]["version"], "1.2.3");
    assert_eq!(v["info"]["description"], "D");
    assert!(v.get("externalDocs").is_none());
}

#[test]
fn no_description_when_omitted() {
    let out = bundle(&[src("a.swagger.yaml", "openapi: 3.0.0\npaths: {}\n")], &info()).unwrap();
    assert!(parse(&out)["info"].get("description").is_none());
}

#[test]
fn top_level_security_is_pushed_down_to_operations_without_their_own() {
    let out = bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\nsecurity:\n  - key: []\npaths:\n  /a:\n    get: {}\n    post:\n      security: []\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\npaths:\n  /b:\n    get: {}\n"),
        ],
        &info(),
    )
    .unwrap();
    let v = parse(&out);
    assert!(v.get("security").is_none());
    assert_eq!(v["paths"]["/a"]["get"]["security"][0]["key"].as_sequence().unwrap().len(), 0);
    assert_eq!(v["paths"]["/a"]["post"]["security"].as_sequence().unwrap().len(), 0);
    assert!(v["paths"]["/b"]["get"].get("security").is_none());
}

#[test]
fn tags_are_combined_by_name_and_identical_duplicates_deduped() {
    let out = bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\ntags:\n  - name: x\n    description: d\npaths: {}\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\ntags:\n  - name: x\n    description: d\n  - name: y\npaths: {}\n"),
        ],
        &info(),
    )
    .unwrap();
    let names: Vec<_> = parse(&out)["tags"].as_sequence().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["x", "y"]);
}

#[test]
fn tag_with_different_content_fails_naming_both_files() {
    let e = bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\ntags:\n  - name: x\n    description: one\npaths: {}\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\ntags:\n  - name: x\n    description: two\npaths: {}\n"),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("a.swagger.yaml") && e.contains("b.swagger.yaml") && e.contains('x'), "{e}");
}

#[test]
fn paths_differing_only_in_template_names_collide() {
    let err = bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\npaths:\n  /users/{id}:\n    get: {}\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\npaths:\n  /users/{userId}:\n    post: {}\n"),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    for s in ["a.swagger.yaml", "b.swagger.yaml", "/users/{id}", "/users/{userId}"] {
        assert!(err.contains(s), "{err}");
    }
}

#[test]
fn conflicting_path_level_field_fails() {
    let err = bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    summary: one\n    get: {}\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    summary: two\n    post: {}\n"),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    for s in ["a.swagger.yaml", "b.swagger.yaml", "/items", "summary"] {
        assert!(err.contains(s), "{err}");
    }
}

#[test]
fn identical_path_level_fields_merge() {
    bundle(
        &[
            src("a.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    summary: same\n    get: {}\n"),
            src("b.swagger.yaml", "openapi: 3.0.0\npaths:\n  /items:\n    summary: same\n    post: {}\n"),
        ],
        &info(),
    )
    .unwrap();
}

fn spec_with_servers(servers: &str, path: &str, method: &str) -> String {
    format!("openapi: 3.0.0\nservers:\n{servers}paths:\n  {path}:\n    {method}: {{}}\n")
}

#[test]
fn shared_origins_become_top_level_servers() {
    let a = spec_with_servers("  - url: https://h.example/a\n", "/x", "get");
    let b = spec_with_servers("  - url: https://h.example/b\n", "/x", "get");
    let out = bundle(&[src("a.swagger.yaml", &a), src("b.swagger.yaml", &b)], &info()).unwrap();
    let v = parse(&out);
    assert_eq!(v["servers"][0]["url"].as_str(), Some("https://h.example"));
    assert_eq!(v["servers"].as_sequence().unwrap().len(), 1);
    assert!(v["paths"]["/a/x"].get("servers").is_none());
}

#[test]
fn differing_origins_become_per_path_servers() {
    let a = spec_with_servers("  - url: https://one.example/a\n", "/x", "get");
    let b = spec_with_servers("  - url: https://two.example/b\n", "/y", "get");
    let out = bundle(&[src("a.swagger.yaml", &a), src("b.swagger.yaml", &b)], &info()).unwrap();
    let v = parse(&out);
    assert!(v.get("servers").is_none());
    assert_eq!(v["paths"]["/a/x"]["servers"][0]["url"].as_str(), Some("https://one.example"));
    assert_eq!(v["paths"]["/b/y"]["servers"][0]["url"].as_str(), Some("https://two.example"));
}

#[test]
fn relative_server_urls_have_no_origin() {
    let a = spec_with_servers("  - url: /a\n", "/x", "get");
    let out = bundle(&[src("a.swagger.yaml", &a)], &info()).unwrap();
    let v = parse(&out);
    assert!(v.get("servers").is_none());
    assert!(v["paths"]["/a/x"].get("servers").is_none());
}

#[test]
fn path_variables_use_defaults_and_host_variables_are_kept() {
    let a = "openapi: 3.0.0\nservers:\n  - url: https://{env}.example/{ver}\n    variables:\n      env:\n        default: prod\n      ver:\n        default: v2\npaths:\n  /x:\n    get: {}\n";
    let out = bundle(&[src("a.swagger.yaml", a)], &info()).unwrap();
    let v = parse(&out);
    assert_eq!(paths_of(&out), ["/v2/x"]);
    assert_eq!(v["servers"][0]["url"].as_str(), Some("https://{env}.example"));
    assert_eq!(v["servers"][0]["variables"]["env"]["default"].as_str(), Some("prod"));
    assert!(v["servers"][0]["variables"].get("ver").is_none());
}

#[test]
fn servers_with_different_path_parts_fail_naming_file() {
    let a = spec_with_servers("  - url: https://h.example/a\n  - url: https://i.example/b\n", "/x", "get");
    let err = bundle(&[src("a.swagger.yaml", &a)], &info()).unwrap_err().to_string();
    assert!(err.contains("a.swagger.yaml"), "{err}");
}

#[test]
fn same_path_with_different_servers_fails_naming_files_and_path() {
    let a = spec_with_servers("  - url: https://one.example\n", "/x", "get");
    let b = spec_with_servers("  - url: https://two.example\n", "/x", "post");
    let err = bundle(&[src("a.swagger.yaml", &a), src("b.swagger.yaml", &b)], &info()).unwrap_err().to_string();
    for needle in ["a.swagger.yaml", "b.swagger.yaml", "/x"] {
        assert!(err.contains(needle), "{err}");
    }
}
