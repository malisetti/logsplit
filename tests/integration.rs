use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

#[test]
fn jsonl_split_by_level_produces_level_log_files() {
    let temp = TempDir::new().expect("tempdir");
    let input = temp.path().join("input.jsonl");
    fs::write(
        &input,
        "{\"level\":\"info\",\"msg\":\"a\"}\n{\"level\":\"error\",\"msg\":\"b\"}\n{\"level\":\"info\",\"msg\":\"c\"}\n",
    )
    .expect("write input");

    let out = temp.path().join("out");
    fs::create_dir_all(&out).expect("create out dir");

    Command::cargo_bin("logsplit")
        .expect("binary exists")
        .args([
            "--by",
            "level",
            "--out-dir",
            out.to_str().expect("out path utf8"),
            "--format",
            "jsonl",
            input.to_str().expect("input path utf8"),
        ])
        .assert()
        .success();

    let info = fs::read_to_string(out.join("info.log")).expect("info.log");
    assert!(info.contains("\"msg\":\"a\""));
    assert!(info.contains("\"msg\":\"c\""));
    assert_eq!(info.lines().count(), 2);

    let error = fs::read_to_string(out.join("error.log")).expect("error.log");
    assert!(error.contains("\"msg\":\"b\""));
    assert_eq!(error.lines().count(), 1);
}

#[test]
fn missing_field_routes_to_missing_bucket() {
    let temp = TempDir::new().expect("tempdir");
    let input = temp.path().join("input.jsonl");
    fs::write(&input, "{\"level\":\"info\"}\n{\"msg\":\"no level\"}\n").expect("write input");

    let out = temp.path().join("out");
    fs::create_dir_all(&out).expect("create out dir");

    Command::cargo_bin("logsplit")
        .expect("binary exists")
        .args([
            "--by",
            "level",
            "--out-dir",
            out.to_str().expect("out path utf8"),
            "--format",
            "jsonl",
            input.to_str().expect("input path utf8"),
        ])
        .assert()
        .success();

    let missing = fs::read_to_string(out.join("__missing__.log")).expect("__missing__.log");
    assert!(missing.contains("\"msg\":\"no level\""));
    assert_eq!(missing.lines().count(), 1);

    let info = fs::read_to_string(out.join("info.log")).expect("info.log");
    assert_eq!(info.lines().count(), 1);
}

#[test]
fn strict_parse_exits_1_on_bad_line() {
    let temp = TempDir::new().expect("tempdir");
    let input = temp.path().join("bad.jsonl");
    fs::write(&input, "{\"level\":\"info\"}\nnot json\n").expect("write input");

    let out = temp.path().join("out");
    fs::create_dir_all(&out).expect("create out dir");

    Command::cargo_bin("logsplit")
        .expect("binary exists")
        .args([
            "--by",
            "level",
            "--out-dir",
            out.to_str().expect("out path utf8"),
            "--format",
            "jsonl",
            "--strict-parse",
            input.to_str().expect("input path utf8"),
        ])
        .assert()
        .code(1);
}
