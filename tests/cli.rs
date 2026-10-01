use std::process::Command;

fn source_fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("a.rs"),
        "fn first() {\n    let α = 1;\n}\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("b.rs"),
        "fn second() {\n    let β = 2;\n}\n",
    )
    .unwrap();
    directory
}

fn report(directory: &std::path::Path, options: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg(directory)
        .args(options)
        .env("RAYON_NUM_THREADS", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn source_output_is_opt_in() {
    let directory = source_fixture();
    let default = report(directory.path(), &["--color=always"]);
    assert!(!default.contains("Structural similarities"));
    assert!(!default.contains("let α"));
    assert!(!default.contains('\x1b'));
    let plain = report(directory.path(), &["--show-similarities", "--color=never"]);
    assert!(plain.contains("Structural similarities"));
    assert!(plain.contains("let α"));
    assert!(plain.contains("let β"));
    assert!(plain.contains("=    2 |"));
    assert!(!plain.contains('\x1b'));
}

#[test]
fn explicit_color_override() {
    let directory = source_fixture();
    let auto = report(directory.path(), &["--show-similarities"]);
    assert!(!auto.contains('\x1b'));
    let color = report(directory.path(), &["--show-similarities", "--color=always"]);
    assert!(color.contains("\x1b[38;5;90m"));
    assert!(color.contains("\x1b[48;5;194m"));
    assert!(color.contains('α') && color.contains('β'));
}

#[test]
fn partial_source_matches() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("a.rs"),
        "fn first() { let x = a + b; }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("b.rs"),
        "fn second() { let y = a - b; }\n",
    )
    .unwrap();
    let output = report(directory.path(), &["--show-similarities", "--color=never"]);
    assert!(output.contains("~    1 |"), "{output}");
}

#[test]
fn cli_errors_are_nonzero() {
    let directory = tempfile::tempdir().unwrap();
    for options in [vec!["--threshold=NaN"], Vec::new()] {
        let output = Command::new(env!("CARGO_BIN_EXE_df"))
            .arg(directory.path().join("missing"))
            .args(options)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("No clones found"));
    }
}

#[test]
fn fail_on_clones_returns_nonzero() {
    let directory = source_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg(directory.path())
        .arg("--fail-on-clones")
        .env("RAYON_NUM_THREADS", "1")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Found"));
}

#[test]
fn fail_on_clones_accepts_clean_source() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("a.rs"), "fn first() { let a = 1; }\n").unwrap();
    std::fs::write(
        directory.path().join("b.rs"),
        "fn second() { println!(\"unique\"); }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg(directory.path())
        .arg("--fail-on-clones")
        .output()
        .unwrap();

    assert!(output.status.success());
}

#[test]
fn rounded_near_keeps_label() {
    let directory = tempfile::tempdir().unwrap();
    let mut first = String::from("fn f() {\n");
    let mut second = first.clone();
    for index in 0..120 {
        first.push_str(&format!("let x{index} = a + b;\n"));
        let operator = if index == 60 { "-" } else { "+" };
        second.push_str(&format!("let x{index} = a {operator} b;\n"));
    }
    first.push_str("}\n");
    second.push_str("}\n");
    std::fs::write(directory.path().join("a.rs"), first).unwrap();
    std::fs::write(directory.path().join("b.rs"), second).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg(directory.path())
        .env("RAYON_NUM_THREADS", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("100% similarity, near)"), "{stdout}");
}
