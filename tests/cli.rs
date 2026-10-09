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
    swagweld(dir.path())
        .args(["-o", "out/nested/b.yaml"])
        .assert()
        .success();
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
    write_spec(
        dir.path(),
        "z/z.swagger.yaml",
        "openapi: 3.0.0\npaths:\n  /z:\n    get: {}\n",
    );
    write_spec(
        dir.path(),
        "a/a.swagger.yaml",
        "openapi: 3.0.0\npaths:\n  /a:\n    get: {}\n",
    );
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

fn spec_at(path: &str) -> String {
    format!("openapi: 3.0.0\npaths:\n  /{path}:\n    get: {{}}\n")
}

fn bundle_of(dir: &std::path::Path) -> String {
    fs::read_to_string(dir.join("dist/swagger.yaml")).unwrap()
}

#[test]
fn discovers_all_filename_patterns_and_ignores_other_yaml() {
    let dir = tempdir().unwrap();
    for (f, p) in [
        ("swagger.yaml", "a"),
        ("x/swagger.yml", "b"),
        ("y/u.swagger.yaml", "c"),
        ("z/v.swagger.yml", "d"),
        ("other.yaml", "nope"),
        ("w/openapi.yml", "nope2"),
    ] {
        write_spec(dir.path(), f, &spec_at(p));
    }
    swagweld(dir.path()).assert().success();
    let out = bundle_of(dir.path());
    for p in ["/a:", "/b:", "/c:", "/d:"] {
        assert!(out.contains(p), "{out}");
    }
    assert!(!out.contains("/nope"), "{out}");
}

#[test]
fn skips_gitignored_files() {
    let dir = tempdir().unwrap();
    write_spec(dir.path(), ".gitignore", "ignored/\n");
    write_spec(dir.path(), "a.swagger.yaml", &spec_at("kept"));
    write_spec(dir.path(), "ignored/b.swagger.yaml", &spec_at("dropped"));
    swagweld(dir.path()).assert().success();
    let out = bundle_of(dir.path());
    assert!(out.contains("/kept:") && !out.contains("/dropped"), "{out}");
}

#[test]
fn skips_hidden_directories() {
    let dir = tempdir().unwrap();
    write_spec(dir.path(), "a.swagger.yaml", &spec_at("kept"));
    write_spec(dir.path(), ".hidden/b.swagger.yaml", &spec_at("dropped"));
    swagweld(dir.path()).assert().success();
    let out = bundle_of(dir.path());
    assert!(out.contains("/kept:") && !out.contains("/dropped"), "{out}");
}

#[test]
fn output_path_is_never_a_source_spec() {
    let dir = tempdir().unwrap();
    write_spec(dir.path(), "a.swagger.yaml", &spec_at("kept"));
    // stale output from a previous run, with a matching name and a conflicting path
    write_spec(dir.path(), "out/bundle.swagger.yaml", &spec_at("kept"));
    swagweld(dir.path())
        .args(["-o", "out/bundle.swagger.yaml"])
        .assert()
        .success();
    let out = fs::read_to_string(dir.path().join("out/bundle.swagger.yaml")).unwrap();
    assert!(out.contains("/kept:"), "{out}");
}

#[test]
fn metadata_flags_and_defaults() {
    let dir = tempdir().unwrap();
    write_spec(dir.path(), "a.swagger.yaml", SPEC);
    swagweld(dir.path())
        .args(["-t", "My API", "-v", "2.0.0", "-d", "Desc"])
        .assert()
        .success();
    let b = fs::read_to_string(dir.path().join("dist/swagger.yaml")).unwrap();
    assert!(
        b.contains("title: My API")
            && b.contains("version: 2.0.0")
            && b.contains("description: Desc"),
        "{b}"
    );

    let dir = tempdir().unwrap();
    write_spec(dir.path(), "a.swagger.yaml", SPEC);
    swagweld(dir.path()).assert().success();
    let b = fs::read_to_string(dir.path().join("dist/swagger.yaml")).unwrap();
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        b.contains(&format!("title: {name}"))
            && b.contains("version: 0.0.0")
            && !b.contains("description"),
        "{b}"
    );
}

#[test]
fn capital_v_prints_swagweld_version() {
    let dir = tempdir().unwrap();
    swagweld(dir.path())
        .arg("-V")
        .assert()
        .success()
        .stdout(predicates::str::contains(env!("CARGO_PKG_VERSION")));
}

fn write_colliding(dir: &std::path::Path) {
    let spec = |path: &str, ty: &str| {
        format!(
            "openapi: 3.0.0\npaths:\n  {path}:\n    get: {{}}\ncomponents:\n  schemas:\n    User:\n      type: {ty}\n"
        )
    };
    write_spec(dir, "a.swagger.yaml", &spec("/a", "string"));
    write_spec(dir, "b.swagger.yaml", &spec("/b", "integer"));
}

#[test]
fn collision_warnings_go_to_stderr() {
    let dir = tempdir().unwrap();
    write_colliding(dir.path());
    swagweld(dir.path())
        .assert()
        .success()
        .stderr(predicates::str::contains("a_User"));
}

#[test]
fn quiet_silences_warnings() {
    let dir = tempdir().unwrap();
    write_colliding(dir.path());
    swagweld(dir.path()).arg("-q").assert().success().stderr("");
}
