use clap::{Args, Parser, Subcommand};
use leshy_core::{RepositoryIndex, index_repository};
use std::borrow::Cow;
use std::fs::canonicalize;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "leshy", bin_name = "leshy", version, about, long_about = None)]
struct MainArgs {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Indexes a repository
    Index(IndexArgs),
}

#[derive(Args, Debug)]
struct IndexArgs {
    /// Path to the repository which will be indexed. The argument may be either a relative or an absolute path.
    #[arg(value_name = "PATH", value_parser = validate_project_dir)]
    path: PathBuf,
}

fn validate_project_dir(string_path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(string_path);

    if !path.exists() {
        return Err(String::from("Path does not exist"));
    }

    if !path.is_dir() {
        return Err(String::from("Path is not a directory"));
    }

    canonicalize(path).map_err(|err| format!("Invalid path: {err}"))
}

fn run(main_args: MainArgs) -> Result<String, CliError> {
    match main_args.command {
        Commands::Index(index_args) => run_index(index_args.path),
    }
}

fn run_index(path: PathBuf) -> Result<String, CliError> {
    let index = index_repository(&path).map_err(|source| CliError::Index {
        path: path.clone(),
        source,
    })?;

    Ok(format_index_summary(&path, &index))
}

fn format_index_summary(path: &std::path::Path, index: &RepositoryIndex) -> String {
    format!(
        "Indexed repository: {}\nDirectories: {}\nFiles: {}",
        display_path(path),
        index.scan.directories.len(),
        index.scan.files.len()
    )
}

fn display_path(path: &std::path::Path) -> Cow<'_, str> {
    let path = path.to_string_lossy();

    #[cfg(windows)]
    if let Some(stripped) = path.strip_prefix(r"\\?\") {
        return Cow::Owned(stripped.to_string());
    }

    path
}

#[derive(Debug)]
enum CliError {
    Index {
        path: PathBuf,
        source: leshy_core::IndexError,
    },
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index { path, source } => {
                write!(f, "failed to index `{}`: {source}", display_path(path))
            }
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Index { source, .. } => Some(source),
        }
    }
}

fn main() -> ExitCode {
    match run(MainArgs::parse()) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use clap::CommandFactory;

    use super::{
        CliError, MainArgs, display_path, format_index_summary, run_index, validate_project_dir,
    };

    #[test]
    fn rejects_non_existent_path() {
        let missing_path = unique_temp_path("missing");
        let error = validate_project_dir(&missing_path.to_string_lossy())
            .expect_err("missing path must fail");

        assert_eq!(error, "Path does not exist");
    }

    #[test]
    fn rejects_non_directory_path() {
        let tempdir = TestDir::new();
        let file_path = tempdir.path().join("file.txt");
        fs::write(&file_path, "content").expect("temporary file");
        let file_path = file_path.to_string_lossy().into_owned();

        let error = validate_project_dir(&file_path).expect_err("file path must fail");

        assert_eq!(error, "Path is not a directory");
    }

    #[test]
    fn accepts_directory_and_returns_canonical_path() {
        let tempdir = TestDir::new();
        let nested = tempdir.path().join("repo");
        fs::create_dir(&nested).expect("nested directory");
        let relative = nested.to_string_lossy().into_owned();

        let validated = validate_project_dir(&relative).expect("directory path should validate");

        assert!(validated.is_absolute());
        assert!(validated.ends_with("repo"));
    }

    #[test]
    fn help_uses_leshy_binary_name() {
        let help = MainArgs::command().render_long_help().to_string();

        assert!(help.contains("Usage: leshy <COMMAND>"));
    }

    #[test]
    fn formats_runtime_index_failures_with_requested_action() {
        let tempdir = TestDir::new();
        let missing_after_validation = tempdir.path().join("repo");
        fs::create_dir(&missing_after_validation).expect("directory should exist for validation");
        let validated =
            validate_project_dir(&missing_after_validation.to_string_lossy()).expect("valid path");
        fs::remove_dir_all(&missing_after_validation).expect("directory should be removed");

        let error = run_index(validated).expect_err("indexing should fail after directory removal");

        let message = error.to_string();
        let display_path = display_path(&missing_after_validation);

        assert!(message.contains("failed to index"));
        assert!(message.contains(display_path.as_ref()));
        assert!(message.contains("failed to scan repository"));
        assert!(message.contains("failed to canonicalize repository root"));
        assert!(matches!(error, CliError::Index { .. }));
    }

    #[test]
    fn formats_successful_index_summary() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "");
        let validated =
            validate_project_dir(&tempdir.path().to_string_lossy()).expect("valid directory");
        let index = leshy_core::index_repository(&validated).expect("indexing should succeed");

        let summary = format_index_summary(&validated, &index);

        assert!(summary.contains(display_path(&validated).as_ref()));
        assert!(summary.contains("Directories: 2"));
        assert!(summary.contains("Files: 1"));
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "leshy-cli-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time should be valid")
                    .as_nanos()
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
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn unique_temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "leshy-cli-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos()
        ))
    }
}
