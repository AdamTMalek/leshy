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
