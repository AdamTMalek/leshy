use clap::{Args, Parser, Subcommand};
use std::fs::canonicalize;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about, long_about = None)]
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
    /// Path to the repository which will be indexed. The argument may be either a relative
    /// or an absolute path.
    #[arg(short, long, value_name = "PROJECT_DIR", value_parser = validate_project_dir)]
    path: std::path::PathBuf,
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

fn main() {
    let main_args = MainArgs::parse();

    match &main_args.command {
        Commands::Index(index_args) => {
            println!("{:#?}", index_args);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::validate_project_dir;

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
