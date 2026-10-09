use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use swagweld::{Info, Source, bundle_with_warnings};

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Where to write the Bundle
    #[arg(short, long, default_value = "dist/swagger.yaml")]
    output: PathBuf,
    /// Bundle info.title (default: current directory name)
    #[arg(short, long)]
    title: Option<String>,
    /// Bundle info.version
    #[arg(short = 'v', long, default_value = "0.0.0")]
    api_version: String,
    /// Bundle info.description
    #[arg(short, long)]
    description: Option<String>,
    /// Silence warnings
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    let mut files = Vec::new();
    let output = cwd.join(&cli.output);
    for entry in ignore::WalkBuilder::new(&cwd).require_git(false).build() {
        let path = entry?.into_path();
        let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        let is_spec = ["swagger.yaml", "swagger.yml"]
            .iter()
            .any(|s| name == *s || name.ends_with(&format!(".{s}")));
        if path.is_file() && is_spec && path != output {
            files.push(path.strip_prefix(&cwd)?.to_path_buf());
        }
    }
    files.sort();
    anyhow::ensure!(!files.is_empty(), "no *.swagger.yaml Source Spec found");

    let sources = files
        .iter()
        .map(|f| {
            Ok(Source {
                file: f.display().to_string(),
                yaml: std::fs::read_to_string(cwd.join(f))
                    .with_context(|| format!("reading {}", f.display()))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let title = cli
        .title
        .unwrap_or_else(|| cwd.file_name().map_or("".into(), |n| n.to_string_lossy().into()));
    let info = Info { title, version: cli.api_version, description: cli.description };
    let root_name = cwd.file_name().map_or("".into(), |n| n.to_string_lossy());
    let (out, warnings) = bundle_with_warnings(&sources, &info, &root_name)?;
    if !cli.quiet {
        for warning in warnings {
            eprintln!("warning: {warning}");
        }
    }
    if let Some(parent) = cli.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cli.output, out)?;
    Ok(())
}
