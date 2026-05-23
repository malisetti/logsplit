#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use logsplit::{open_input_paths, process_lines, InputFormat, Router};
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(
    name = "logsplit",
    version,
    about = "Split structured log streams into per-field-value files"
)]
struct Cli {
    /// Field name to route on
    #[arg(long)]
    by: String,

    /// Output directory for split logs
    #[arg(long)]
    out_dir: PathBuf,

    /// Input format: jsonl, logfmt, or auto
    #[arg(long, default_value = "auto")]
    format: String,

    /// Exit with code 1 when any input line fails to parse
    #[arg(long)]
    strict_parse: bool,

    /// Input files (stdin when omitted)
    files: Vec<PathBuf>,
}

#[derive(Debug, Error)]
enum AppError {
    #[error("invalid --format: expected jsonl, logfmt, or auto")]
    InvalidFormat,
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("router error: {0}")]
    Router(#[from] logsplit::RouterError),
}

fn run(cli: Cli) -> Result<bool, AppError> {
    let format = InputFormat::parse_arg(&cli.format).ok_or(AppError::InvalidFormat)?;
    let mut router = Router::new(cli.out_dir, cli.by)?;

    let mut had_parse_errors = false;

    if cli.files.is_empty() {
        let stdin = io::stdin();
        let reader = stdin.lock();
        let lines = reader.lines();
        let parse_had_errors = process_lines(lines, format, &mut router, cli.strict_parse)?;
        had_parse_errors |= parse_had_errors;
    } else {
        let files = open_input_paths(&cli.files)?;
        for file in files {
            let reader = BufReader::new(file);
            let lines = reader.lines();
            let parse_had_errors = process_lines(lines, format, &mut router, cli.strict_parse)?;
            had_parse_errors |= parse_had_errors;
        }
    }

    Ok(had_parse_errors)
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            err.print().ok();
            return ExitCode::from(2);
        }
    };

    match run(cli) {
        Ok(had_parse_errors) => {
            if had_parse_errors {
                ExitCode::from(1)
            } else {
                ExitCode::from(0)
            }
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}
