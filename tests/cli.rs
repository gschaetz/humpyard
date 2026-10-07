use std::process::Command;

fn run(args: &[&str], key: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_switchyard-conductor"));
    cmd.args(args).env_remove("CLI_TEST_KEY");
    if let Some(key) = key {
        cmd.env("CLI_TEST_KEY", key);
    }
    cmd.output().unwrap()
}

fn write_config(name: &str, body: &str) -> String {
    let path = std::env::temp_dir().join(format!("sc-{}-{name}.toml", std::process::id()));
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

const GOOD: &str = r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "CLI_TEST_KEY"
[targets.m]
endpoints = [{ provider = "p", model = "m" }]
"#;

#[test]
fn check_config_accepts_valid_file() {
    let path = write_config("good", GOOD);
    let out = run(&["check-config", &path], Some("k"));
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn check_config_fails_on_unknown_key() {
    let path = write_config("unknown", &format!("bogus = 1\n{GOOD}"));
    let out = run(&["check-config", &path], Some("k"));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("bogus"));
}

#[test]
fn check_config_names_missing_env_var() {
    let path = write_config("nokey", GOOD);
    let out = run(&["check-config", &path], None);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("CLI_TEST_KEY"));
}

#[test]
fn serve_refuses_invalid_config() {
    let path = write_config("serve-bad", "nonsense");
    let out = run(&["serve", "--config", &path], Some("k"));
    assert!(!out.status.success());
}
