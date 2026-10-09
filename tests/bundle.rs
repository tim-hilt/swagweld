use swagweld::{Source, bundle};

fn src(file: &str, yaml: &str) -> Source {
    Source { file: file.into(), yaml: yaml.into() }
}

fn paths_of(yaml: &str) -> Vec<String> {
    let v: serde_norway::Value = serde_norway::from_str(yaml).unwrap();
    v["paths"].as_mapping().unwrap().keys().map(|k| k.as_str().unwrap().to_string()).collect()
}

#[test]
fn relative_server_url_prefixes_paths() {
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\nservers:\n  - url: /users/v1\npaths:\n  /items:\n    get: {}\n")],
        "t",
    )
    .unwrap();
    assert_eq!(paths_of(&out), ["/users/v1/items"]);
}

#[test]
fn absolute_server_url_yields_base_path() {
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\nservers:\n  - url: https://host.example/users/v1/\npaths:\n  /items:\n    get: {}\n")],
        "t",
    )
    .unwrap();
    assert_eq!(paths_of(&out), ["/users/v1/items"]);
}

#[test]
fn root_base_path_leaves_paths_unchanged() {
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\nservers:\n  - url: /\npaths:\n  /items:\n    get: {}\n")],
        "t",
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
        "t",
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
        "t",
    )
    .unwrap_err()
    .to_string();
    for needle in ["a.swagger.yaml", "b.swagger.yaml", "/items", "get"] {
        assert!(err.contains(needle), "{err}");
    }
}

fn err_of(yaml: &str) -> String {
    bundle(&[src("bad.swagger.yaml", yaml)], "t").unwrap_err().to_string()
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
        "t",
    )
    .unwrap();
    let v: serde_norway::Value = serde_norway::from_str(&out).unwrap();
    assert_eq!(v["components"]["responses"]["Ok"]["description"], "ok");
    assert_eq!(v["paths"]["/a"]["x-ext"], "custom");
}
