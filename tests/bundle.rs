use swagweld::{Info, Source, is_source_spec};

const ROOT: &str = "root";

fn bundle(sources: &[Source], info: &Info) -> anyhow::Result<String> {
    swagweld::bundle(sources, info, ROOT).map(|b| b.yaml)
}

fn src(file: &str, yaml: &str) -> Source {
    Source {
        file: file.into(),
        yaml: yaml.into(),
    }
}

fn info() -> Info {
    Info {
        title: "t".into(),
        version: "0.0.0".into(),
        description: None,
    }
}

fn parse(yaml: &str) -> serde_norway::Value {
    serde_norway::from_str(yaml).unwrap()
}

fn paths_of(yaml: &str) -> Vec<String> {
    let v: serde_norway::Value = serde_norway::from_str(yaml).unwrap();
    v["paths"]
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn relative_server_url_prefixes_paths() {
    let out = bundle(
        &[src(
            "a.swagger.yaml",
            "openapi: 3.0.0\nservers:\n  - url: /users/v1\npaths:\n  /items:\n    get: {}\n",
        )],
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
        &[src(
            "a.swagger.yaml",
            "openapi: 3.0.0\nservers:\n  - url: /\npaths:\n  /items:\n    get: {}\n",
        )],
        &info(),
    )
    .unwrap();
    assert_eq!(paths_of(&out), ["/items"]);
}

#[test]
fn disjoint_methods_on_same_path_are_merged() {
    let out = bundle(
        &[
            src(
                "a.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    post: {}\n",
            ),
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
            src(
                "a.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n",
            ),
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
    bundle(&[src("bad.swagger.yaml", yaml)], &info())
        .unwrap_err()
        .to_string()
}

#[test]
fn invalid_yaml_fails_naming_file() {
    assert!(err_of("a: [").contains("bad.swagger.yaml"));
}

#[test]
fn missing_openapi_key_fails_naming_file() {
    let e = err_of("paths: {}\n");
    assert!(
        e.contains("bad.swagger.yaml") && e.contains("openapi"),
        "{e}"
    );
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
        assert!(
            e.contains("bad.swagger.yaml") && e.contains("/paths/~1a/get/responses/200"),
            "{e}"
        );
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
    let i = Info {
        title: "T".into(),
        version: "1.2.3".into(),
        description: Some("D".into()),
    };
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
    let out = bundle(
        &[src("a.swagger.yaml", "openapi: 3.0.0\npaths: {}\n")],
        &info(),
    )
    .unwrap();
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
    assert_eq!(
        v["paths"]["/a"]["get"]["security"][0]["key"]
            .as_sequence()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        v["paths"]["/a"]["post"]["security"]
            .as_sequence()
            .unwrap()
            .len(),
        0
    );
    assert!(v["paths"]["/b"]["get"].get("security").is_none());
}

#[test]
fn tags_are_combined_by_name_and_identical_duplicates_deduped() {
    let out = bundle(
        &[
            src(
                "a.swagger.yaml",
                "openapi: 3.0.0\ntags:\n  - name: x\n    description: d\npaths: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.0.0\ntags:\n  - name: x\n    description: d\n  - name: y\npaths: {}\n",
            ),
        ],
        &info(),
    )
    .unwrap();
    let names: Vec<_> = parse(&out)["tags"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["x", "y"]);
}

#[test]
fn tag_with_different_content_fails_naming_both_files() {
    let e = bundle(
        &[
            src(
                "a.swagger.yaml",
                "openapi: 3.0.0\ntags:\n  - name: x\n    description: one\npaths: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.0.0\ntags:\n  - name: x\n    description: two\npaths: {}\n",
            ),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("a.swagger.yaml") && e.contains("b.swagger.yaml") && e.contains('x'),
        "{e}"
    );
}

#[test]
fn paths_differing_only_in_template_names_collide() {
    let err = bundle(
        &[
            src(
                "a.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /users/{id}:\n    get: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /users/{userId}:\n    post: {}\n",
            ),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    for s in [
        "a.swagger.yaml",
        "b.swagger.yaml",
        "/users/{id}",
        "/users/{userId}",
    ] {
        assert!(err.contains(s), "{err}");
    }
}

#[test]
fn conflicting_path_level_field_fails() {
    let err = bundle(
        &[
            src(
                "a.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    summary: one\n    get: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    summary: two\n    post: {}\n",
            ),
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
            src(
                "a.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    summary: same\n    get: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.0.0\npaths:\n  /items:\n    summary: same\n    post: {}\n",
            ),
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
    let out = bundle(
        &[src("a.swagger.yaml", &a), src("b.swagger.yaml", &b)],
        &info(),
    )
    .unwrap();
    let v = parse(&out);
    assert_eq!(v["servers"][0]["url"].as_str(), Some("https://h.example"));
    assert_eq!(v["servers"].as_sequence().unwrap().len(), 1);
    assert!(v["paths"]["/a/x"].get("servers").is_none());
}

#[test]
fn differing_origins_become_per_path_servers() {
    let a = spec_with_servers("  - url: https://one.example/a\n", "/x", "get");
    let b = spec_with_servers("  - url: https://two.example/b\n", "/y", "get");
    let out = bundle(
        &[src("a.swagger.yaml", &a), src("b.swagger.yaml", &b)],
        &info(),
    )
    .unwrap();
    let v = parse(&out);
    assert!(v.get("servers").is_none());
    assert_eq!(
        v["paths"]["/a/x"]["servers"][0]["url"].as_str(),
        Some("https://one.example")
    );
    assert_eq!(
        v["paths"]["/b/y"]["servers"][0]["url"].as_str(),
        Some("https://two.example")
    );
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
    assert_eq!(
        v["servers"][0]["url"].as_str(),
        Some("https://{env}.example")
    );
    assert_eq!(
        v["servers"][0]["variables"]["env"]["default"].as_str(),
        Some("prod")
    );
    assert!(v["servers"][0]["variables"].get("ver").is_none());
}

#[test]
fn servers_with_different_path_parts_fail_naming_file() {
    let a = spec_with_servers(
        "  - url: https://h.example/a\n  - url: https://i.example/b\n",
        "/x",
        "get",
    );
    let err = bundle(&[src("a.swagger.yaml", &a)], &info())
        .unwrap_err()
        .to_string();
    assert!(err.contains("a.swagger.yaml"), "{err}");
}

#[test]
fn same_path_with_different_servers_fails_naming_files_and_path() {
    let a = spec_with_servers("  - url: https://one.example\n", "/x", "get");
    let b = spec_with_servers("  - url: https://two.example\n", "/x", "post");
    let err = bundle(
        &[src("a.swagger.yaml", &a), src("b.swagger.yaml", &b)],
        &info(),
    )
    .unwrap_err()
    .to_string();
    for needle in ["a.swagger.yaml", "b.swagger.yaml", "/x"] {
        assert!(err.contains(needle), "{err}");
    }
}

const PATHS: &str = "paths:\n  /items:\n    get: {}\n";

fn weld(sources: &[Source]) -> (serde_norway::Value, Vec<String>) {
    let b = swagweld::bundle(sources, &info(), ROOT).unwrap();
    (parse(&b.yaml), b.warnings)
}

#[test]
fn identical_components_are_deduped_silently() {
    let yaml =
        format!("openapi: 3.0.0\n{PATHS}components:\n  schemas:\n    User:\n      type: string\n");
    let (v, warnings) = weld(&[
        src("a.swagger.yaml", &yaml),
        src("b.swagger.yaml", &yaml.replace("/items", "/other")),
    ]);
    assert_eq!(v["components"]["schemas"].as_mapping().unwrap().len(), 1);
    assert!(warnings.is_empty());
}

#[test]
fn differing_components_are_renamed_in_every_source_and_warn() {
    let a =
        format!("openapi: 3.0.0\n{PATHS}components:\n  schemas:\n    User:\n      type: string\n");
    let b = "openapi: 3.0.0\npaths:\n  /other:\n    get: {}\ncomponents:\n  schemas:\n    User:\n      type: integer\n";
    let (v, warnings) = weld(&[src("a.swagger.yaml", &a), src("users/b.swagger.yaml", b)]);
    let schemas = &v["components"]["schemas"];
    assert_eq!(
        schemas["a_User"]["type"],
        serde_norway::Value::from("string")
    );
    assert_eq!(
        schemas["b_User"]["type"],
        serde_norway::Value::from("integer")
    );
    assert!(schemas.get("User").is_none());
    assert_eq!(warnings.len(), 2);
    for needle in ["a.swagger.yaml", "users/b.swagger.yaml", "User", "a_User"] {
        assert!(
            warnings.iter().any(|w| w.contains(needle)),
            "{needle}: {warnings:?}"
        );
    }
}

#[test]
fn local_refs_in_renamed_source_are_rewritten() {
    let a = "openapi: 3.0.0\npaths:\n  /items:\n    get:\n      responses:\n        '200':\n          content:\n            application/json:\n              schema:\n                $ref: '#/components/schemas/User'\ncomponents:\n  schemas:\n    User:\n      type: string\n";
    let b = format!(
        "openapi: 3.0.0\n{}components:\n  schemas:\n    User:\n      type: integer\n",
        PATHS.replace("/items", "/other")
    );
    let (v, _) = weld(&[src("a.swagger.yaml", a), src("b.swagger.yaml", &b)]);
    let schema =
        &v["paths"]["/items"]["get"]["responses"]["200"]["content"]["application/json"]["schema"];
    assert_eq!(
        schema["$ref"],
        serde_norway::Value::from("#/components/schemas/a_User")
    );
}

#[test]
fn renamed_name_that_still_collides_fails() {
    let a =
        format!("openapi: 3.0.0\n{PATHS}components:\n  schemas:\n    User:\n      type: string\n");
    let b = format!(
        "openapi: 3.0.0\n{}components:\n  schemas:\n    User:\n      type: integer\n",
        PATHS.replace("/items", "/other")
    );
    let err = bundle(
        &[src("x/a.swagger.yaml", &a), src("y/a.swagger.yaml", &b)],
        &info(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("a_User"), "{err}");
}

#[test]
fn security_schemes_with_different_content_fail_instead_of_renaming() {
    let a = format!(
        "openapi: 3.0.0\n{PATHS}components:\n  securitySchemes:\n    auth:\n      type: http\n      scheme: basic\n"
    );
    let b = format!(
        "openapi: 3.0.0\n{}components:\n  securitySchemes:\n    auth:\n      type: http\n      scheme: bearer\n",
        PATHS.replace("/items", "/other")
    );
    let err = bundle(
        &[src("a.swagger.yaml", &a), src("b.swagger.yaml", &b)],
        &info(),
    )
    .unwrap_err()
    .to_string();
    for needle in ["auth", "a.swagger.yaml", "b.swagger.yaml"] {
        assert!(err.contains(needle), "{err}");
    }
}

fn op(path: &str, id: &str) -> String {
    format!("openapi: 3.0.0\npaths:\n  {path}:\n    get:\n      operationId: {id}\n")
}

#[test]
fn clashing_operation_ids_are_renamed_in_every_source_and_warn() {
    let (v, warnings) = weld(&[
        src("a.swagger.yaml", &op("/items", "getById")),
        src("users/b.swagger.yaml", &op("/other", "getById")),
    ]);
    assert_eq!(
        v["paths"]["/items"]["get"]["operationId"],
        serde_norway::Value::from("a_getById")
    );
    assert_eq!(
        v["paths"]["/other"]["get"]["operationId"],
        serde_norway::Value::from("b_getById")
    );
    assert_eq!(warnings.len(), 2);
    for needle in [
        "a.swagger.yaml",
        "users/b.swagger.yaml",
        "getById",
        "b_getById",
    ] {
        assert!(
            warnings.iter().any(|w| w.contains(needle)),
            "{needle}: {warnings:?}"
        );
    }
}

#[test]
fn unique_operation_ids_are_untouched() {
    let (v, warnings) = weld(&[
        src("a.swagger.yaml", &op("/items", "listA")),
        src("b.swagger.yaml", &op("/other", "listB")),
    ]);
    assert_eq!(
        v["paths"]["/items"]["get"]["operationId"],
        serde_norway::Value::from("listA")
    );
    assert!(warnings.is_empty());
}

#[test]
fn links_to_renamed_operation_ids_are_rewritten() {
    let a = format!(
        "{}      responses:\n        '200':\n          description: ok\n          links:\n            self:\n              operationId: getById\n",
        op("/items", "getById")
    );
    let (v, _) = weld(&[
        src("a.swagger.yaml", &a),
        src("b.swagger.yaml", &op("/other", "getById")),
    ]);
    let link = &v["paths"]["/items"]["get"]["responses"]["200"]["links"]["self"];
    assert_eq!(link["operationId"], serde_norway::Value::from("a_getById"));
}

#[test]
fn renamed_operation_id_that_still_collides_fails() {
    let err = bundle(
        &[
            src("x/a.swagger.yaml", &op("/items", "getById")),
            src("y/a.swagger.yaml", &op("/other", "getById")),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("a_getById"), "{err}");
}

#[test]
fn colliding_path_items_are_renamed_and_their_refs_rewritten() {
    let a = "openapi: 3.1.0\npaths:\n  /items:\n    $ref: '#/components/pathItems/Shared'\ncomponents:\n  pathItems:\n    Shared:\n      get: {}\n";
    let b = "openapi: 3.1.0\npaths:\n  /other:\n    get: {}\ncomponents:\n  pathItems:\n    Shared:\n      post: {}\n";
    let (v, warnings) = weld(&[src("a.swagger.yaml", a), src("b.swagger.yaml", b)]);
    let items = &v["components"]["pathItems"];
    assert!(items["a_Shared"].get("get").is_some());
    assert!(items["b_Shared"].get("post").is_some());
    assert_eq!(
        v["paths"]["/items"]["$ref"],
        serde_norway::Value::from("#/components/pathItems/a_Shared")
    );
    assert_eq!(warnings.len(), 2);
}

fn webhook(file: &str, body: &str) -> Source {
    src(
        file,
        &format!(
            "openapi: 3.1.0\nservers:\n  - url: /base\npaths:\n  /{file}:\n    get: {{}}\nwebhooks:\n  ping:\n    post:\n      description: {body}\n"
        ),
    )
}

#[test]
fn identical_webhooks_are_deduped_without_base_path() {
    let (v, warnings) = weld(&[
        webhook("a.swagger.yaml", "x"),
        webhook("b.swagger.yaml", "x"),
    ]);
    assert_eq!(v["webhooks"].as_mapping().unwrap().len(), 1);
    assert!(v["webhooks"]["ping"].get("post").is_some());
    assert!(warnings.is_empty());
}

#[test]
fn differing_webhooks_are_renamed_with_spec_name_and_warn() {
    let (v, warnings) = weld(&[
        webhook("a.swagger.yaml", "x"),
        webhook("b.swagger.yaml", "y"),
    ]);
    assert!(v["webhooks"].get("ping").is_none());
    assert!(v["webhooks"].get("a_ping").is_some());
    assert!(v["webhooks"].get("b_ping").is_some());
    assert_eq!(warnings.len(), 2);
    assert!(warnings.iter().all(|w| w.contains("webhooks.ping")));
}

fn schema(file: &str, version: &str, user: &str) -> Source {
    src(
        file,
        &format!(
            "openapi: {version}\npaths:\n  /{file}:\n    get: {{}}\ncomponents:\n  schemas:\n    User:\n{user}"
        ),
    )
}

#[test]
fn bundle_declares_highest_openapi_version() {
    let (v, _) = weld(&[
        src("a.swagger.yaml", "openapi: 3.0.3\npaths: {}\n"),
        src("b.swagger.yaml", "openapi: 3.1.1\npaths: {}\n"),
        src("c.swagger.yaml", "openapi: 3.0.10\npaths: {}\n"),
    ]);
    assert_eq!(v["openapi"], serde_norway::Value::from("3.1.1"));
}

#[test]
fn all_3_0_sources_are_left_unconverted() {
    let (v, _) = weld(&[schema(
        "a.swagger.yaml",
        "3.0.3",
        "      type: string\n      nullable: true\n",
    )]);
    assert_eq!(v["openapi"], serde_norway::Value::from("3.0.3"));
    assert_eq!(
        v["components"]["schemas"]["User"],
        parse("type: string\nnullable: true\n")
    );
}

#[test]
fn nullable_type_becomes_null_type_when_bundle_is_3_1() {
    let (v, _) = weld(&[
        schema(
            "a.swagger.yaml",
            "3.0.3",
            "      type: object\n      properties:\n        name:\n          type: string\n          nullable: true\n",
        ),
        src("b.swagger.yaml", "openapi: 3.1.0\npaths: {}\n"),
    ]);
    assert_eq!(
        v["components"]["schemas"]["User"]["properties"]["name"],
        parse("type: [string, 'null']\n")
    );
}

#[test]
fn nullable_ref_becomes_any_of_with_null() {
    let (v, _) = weld(&[
        schema(
            "a.swagger.yaml",
            "3.0.3",
            "      $ref: '#/components/schemas/Name'\n      nullable: true\n    Name:\n      type: string\n",
        ),
        src("b.swagger.yaml", "openapi: 3.2.0\npaths: {}\n"),
    ]);
    assert_eq!(
        v["components"]["schemas"]["User"],
        parse("anyOf:\n  - $ref: '#/components/schemas/Name'\n  - type: 'null'\n")
    );
}

#[test]
fn boolean_exclusive_bounds_become_numeric() {
    let (v, _) = weld(&[
        schema(
            "a.swagger.yaml",
            "3.0.3",
            "      type: integer\n      minimum: 1\n      exclusiveMinimum: true\n      maximum: 9\n      exclusiveMaximum: false\n",
        ),
        src("b.swagger.yaml", "openapi: 3.1.0\npaths: {}\n"),
    ]);
    assert_eq!(
        v["components"]["schemas"]["User"],
        parse("type: integer\nexclusiveMinimum: 1\nmaximum: 9\n")
    );
}

#[test]
fn equivalent_3_0_and_3_1_schemas_are_deduped() {
    let (v, warnings) = weld(&[
        schema(
            "a.swagger.yaml",
            "3.0.3",
            "      type: string\n      nullable: true\n",
        ),
        schema("b.swagger.yaml", "3.1.0", "      type: [string, 'null']\n"),
    ]);
    assert_eq!(v["components"]["schemas"].as_mapping().unwrap().len(), 1);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn differing_json_schema_dialects_fail_naming_files() {
    let err = bundle(
        &[
            src(
                "a.swagger.yaml",
                "openapi: 3.1.0\njsonSchemaDialect: https://one\npaths: {}\n",
            ),
            src(
                "b.swagger.yaml",
                "openapi: 3.1.0\njsonSchemaDialect: https://two\npaths: {}\n",
            ),
        ],
        &info(),
    )
    .unwrap_err()
    .to_string();
    for needle in ["a.swagger.yaml", "b.swagger.yaml", "jsonSchemaDialect"] {
        assert!(err.contains(needle), "{err}");
    }
}

#[test]
fn shared_json_schema_dialect_is_kept() {
    let (v, _) = weld(&[src(
        "a.swagger.yaml",
        "openapi: 3.1.0\njsonSchemaDialect: https://one\npaths: {}\n",
    )]);
    assert_eq!(
        v["jsonSchemaDialect"],
        serde_norway::Value::from("https://one")
    );
}

#[test]
fn root_level_spec_is_named_after_the_root_directory() {
    let a =
        format!("openapi: 3.0.0\n{PATHS}components:\n  schemas:\n    User:\n      type: string\n");
    let b = a.replace("/items", "/other").replace("string", "integer");
    let (v, _) = weld(&[src("b.swagger.yaml", &b), src("swagger.yaml", &a)]);
    assert!(
        v["components"]["schemas"].get("root_User").is_some(),
        "{v:?}"
    );
}

#[test]
fn source_spec_file_names() {
    for name in [
        "swagger.yaml",
        "swagger.yml",
        "u.swagger.yaml",
        "v.swagger.yml",
    ] {
        assert!(is_source_spec(name), "{name}");
    }
    for name in ["openapi.yaml", "myswagger.yaml", "swagger.json"] {
        assert!(!is_source_spec(name), "{name}");
    }
}
