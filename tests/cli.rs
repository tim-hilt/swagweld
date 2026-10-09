use assert_cmd::Command;
use std::fs;
use tempfile::tempdir;

fn swagweld(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("swagweld").unwrap();
    cmd.current_dir(dir);
    cmd
}

#[test]
fn bundles_a_source_spec_into_dist() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("users")).unwrap();
    fs::write(
        dir.path().join("users/users.swagger.yaml"),
        "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n",
    )
    .unwrap();

    swagweld(dir.path()).assert().success();

    let bundle = fs::read_to_string(dir.path().join("dist/swagger.yaml")).unwrap();
    assert!(bundle.contains("/items:"), "{bundle}");
    assert!(bundle.contains("version: 0.0.0"), "{bundle}");
}

fn write_spec(dir: &std::path::Path, rel: &str, yaml: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, yaml).unwrap();
}

const SPEC: &str = "openapi: 3.0.0\npaths:\n  /items:\n    get: {}\n";

#[test]
fn output_flag_writes_to_given_path_creating_parents() {
    let dir = tempdir().unwrap();
    write_spec(dir.path(), "a.swagger.yaml", SPEC);
    swagweld(dir.path()).args(["-o", "out/nested/b.yaml"]).assert().success();
    assert!(dir.path().join("out/nested/b.yaml").exists());
    assert!(!dir.path().join("dist").exists());
}

#[test]
fn fails_when_no_source_spec_found() {
    let dir = tempdir().unwrap();
    swagweld(dir.path()).assert().failure();
    assert!(!dir.path().join("dist").exists());
}

#[test]
fn error_writes_no_bundle() {
    let dir = tempdir().unwrap();
    write_spec(dir.path(), "a.swagger.yaml", SPEC);
    write_spec(dir.path(), "b.swagger.yaml", "swagger: '2.0'\n");
    swagweld(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("b.swagger.yaml"));
    assert!(!dir.path().join("dist").exists());
}

#[test]
fn title_is_directory_name_and_output_is_deterministic() {
    let dir = tempdir().unwrap();
    write_spec(dir.path(), "z/z.swagger.yaml", "openapi: 3.0.0\npaths:\n  /z:\n    get: {}\n");
    write_spec(dir.path(), "a/a.swagger.yaml", "openapi: 3.0.0\npaths:\n  /a:\n    get: {}\n");
    swagweld(dir.path()).assert().success();
    let first = fs::read_to_string(dir.path().join("dist/swagger.yaml")).unwrap();
    swagweld(dir.path()).assert().success();
    let second = fs::read_to_string(dir.path().join("dist/swagger.yaml")).unwrap();
    // second run also sees dist/swagger.yaml, which must not count as a Source Spec
    assert_eq!(first, second);
    assert!(first.find("/a:").unwrap() < first.find("/z:").unwrap());
    let name = dir.path().file_name().unwrap().to_str().unwrap();
    assert!(first.contains(&format!("title: {name}")), "{first}");
}
