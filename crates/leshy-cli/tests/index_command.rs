use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn help_shows_leshy_binary_and_positional_path() {
    Command::new(binary_path())
        .arg("index")
        .arg("--help")
        .assert_success()
        .stdout_contains("Usage: leshy index <PATH>");
}

#[test]
fn index_command_succeeds_for_valid_directory() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "pub fn library() {}\n");

    let output = Command::new(binary_path())
        .arg("index")
        .arg(tempdir.path())
        .assert_success();

    output.stdout_contains("Indexed repository:");
    output.stdout_contains("Directories: 2");
    output.stdout_contains("Files: 1");
}

#[test]
fn index_command_honors_gitignore_rules() {
    let tempdir = TestDir::new();
    tempdir.write_git_config("");
    tempdir.write_file(".gitignore", "target/\n");
    tempdir.write_file("src/lib.rs", "pub fn library() {}\n");
    tempdir.write_file("target/generated.rs", "");

    let output = Command::new(binary_path())
        .arg("index")
        .arg(tempdir.path())
        .assert_success();

    output.stdout_contains("Directories: 2");
    output.stdout_contains("Files: 2");
}

#[test]
fn index_command_rejects_missing_path() {
    let missing_path = unique_temp_path("missing");

    Command::new(binary_path())
        .arg("index")
        .arg(&missing_path)
        .assert_failure()
        .stderr_contains("Path does not exist");
}

#[test]
fn index_command_rejects_file_path() {
    let tempdir = TestDir::new();
    let file_path = tempdir.path().join("file.txt");
    fs::write(&file_path, "content").expect("temporary file");

    Command::new(binary_path())
        .arg("index")
        .arg(&file_path)
        .assert_failure()
        .stderr_contains("Path is not a directory");
}

#[test]
fn index_command_reports_rust_parse_failures() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "fn broken( {\n");

    Command::new(binary_path())
        .arg("index")
        .arg(tempdir.path())
        .assert_failure()
        .stderr_contains("failed to parse repository")
        .stderr_contains("src/lib.rs")
        .stderr_contains("syntax errors");
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let unique = format!(
            "leshy-cli-integration-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir(&path).expect("temporary directory should be created");

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_file(&self, relative_path: &str, contents: &str) {
        let file_path = self.path.join(relative_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("parent directories should be created");
        }
        fs::write(file_path, contents).expect("file should be written");
    }

    fn write_git_config(&self, contents: &str) {
        fs::create_dir_all(self.path.join(".git")).expect("git directory should be created");
        fs::write(self.path.join(".git/config"), contents).expect("git config should be written");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "leshy-cli-integration-test-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos()
    ))
}

fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_leshy")
}

trait CommandAssertionExt {
    fn assert_success(&mut self) -> CommandOutput;
    fn assert_failure(&mut self) -> CommandOutput;
}

impl CommandAssertionExt for Command {
    fn assert_success(&mut self) -> CommandOutput {
        let output = self.output().expect("command should run");
        assert!(
            output.status.success(),
            "expected success, got status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        CommandOutput(output)
    }

    fn assert_failure(&mut self) -> CommandOutput {
        let output = self.output().expect("command should run");
        assert!(
            !output.status.success(),
            "expected failure, got status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        CommandOutput(output)
    }
}

struct CommandOutput(std::process::Output);

impl CommandOutput {
    fn stdout_contains(&self, needle: &str) -> &Self {
        let stdout = String::from_utf8_lossy(&self.0.stdout);
        assert!(
            stdout.contains(needle),
            "expected stdout to contain {needle:?}, got:\n{stdout}"
        );

        self
    }

    fn stderr_contains(&self, needle: &str) -> &Self {
        let stderr = String::from_utf8_lossy(&self.0.stderr);
        assert!(
            stderr.contains(needle),
            "expected stderr to contain {needle:?}, got:\n{stderr}"
        );

        self
    }
}
