use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const HELP: &str = "\
Usage: cargo xtask <task>

Tasks:
	fmt      Format all code
	lint     Run clippy with warnings denied
	test     Run all tests
	bench    Run benchmarks
	ci       Check formatting, lint, and test (what CI runs)
	help     Show this message
";

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
	let task = env::args().nth(1);
	let result = match task.as_deref() {
		Some("fmt") => fmt(false),
		Some("lint") => lint(),
		Some("test") => test(),
		Some("bench") => bench(),
		Some("ci") => ci(),
		None | Some("help" | "-h" | "--help") => {
			print!("{HELP}");
			Ok(())
		}
		Some(other) => Err(format!("unknown task `{other}`\n\n{HELP}")),
	};

	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}

fn fmt(check: bool) -> Result {
	let mut args = vec!["fmt", "--all"];
	if check {
		args.extend(["--", "--check"]);
	}
	cargo(&args)
}

fn lint() -> Result {
	cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
}

fn test() -> Result {
	cargo(&["test", "--workspace"])
}

fn bench() -> Result {
	cargo(&["bench", "--package", "pka-bigfixed"])
}

fn ci() -> Result {
	fmt(true)?;
	lint()?;
	test()
}

fn cargo(args: &[&str]) -> Result {
	let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
	eprintln!("$ cargo {}", args.join(" "));
	let status = Command::new(&cargo)
		.args(args)
		.current_dir(project_root())
		.status()
		.map_err(|e| format!("failed to run `{cargo}`: {e}"))?;
	if status.success() {
		Ok(())
	} else {
		Err(format!("`cargo {}` failed with {status}", args.join(" ")))
	}
}

fn project_root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.expect("xtask lives in a subdirectory of the project root")
		.to_path_buf()
}
