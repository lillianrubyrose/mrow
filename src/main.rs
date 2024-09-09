#![warn(clippy::pedantic)]
#![allow(clippy::too_many_lines)]

mod mrow_lua;
mod mrow_toml;

use std::{
	env::VarError,
	ffi::OsStr,
	path::{Path, PathBuf},
	process::exit,
	rc::Rc,
	sync::{LazyLock, Mutex},
};

use clap::Parser;
use log::{debug, error, info, warn};
use miette::IntoDiagnostic;
use mlua::{Lua, StdLib};
use regex::Regex;
use serde::Deserialize;
use thiserror::Error;
use toml::Value;

#[derive(Debug, Error)]
enum Error {
	#[error("This tool is made for Arch Linux, if you're running an Arch derivative and still getting this message open an issue @ https://github.com/lillianrubyrose/mrow")]
	NotArch,

	#[error("Imported module from '{0}' doesn't exist: '{1}'")]
	TomlImportNotFound(PathBuf, PathBuf),
	#[error("Invalid step in '{0}'. '{1}'")]
	TomlInvalidStepData(PathBuf, Value),
	#[error("Invalid step in '{0}'. {1}")]
	TomlInvalidStep(PathBuf, String),

	#[error("Step in '{0}' failed. {1}")]
	StepFailed(String, String),

	#[error("'{0}': {1}")]
	Toml(PathBuf, toml::de::Error),
	#[error(transparent)]
	Io(#[from] std::io::Error),
	#[error(transparent)]
	Var(#[from] VarError),
	#[error(transparent)]
	Lua(#[from] mlua::Error),
}

type Result<T> = miette::Result<T, Error>;

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum AurHelper {
	Yay,
	Paru,
}

#[derive(Debug, Clone)]
struct Step {
	owner: PathBuf,
	relative_path_str: String,
	kind: StepKind,
}

#[derive(Debug, Clone)]
enum StepKind {
	InstallPackage {
		package: String,
		aur: bool,
	},
	InstallPackages {
		packages: Vec<String>,
		aur: bool,
	},
	CopyFile {
		from: PathBuf,
		to: PathBuf,
		as_root: bool,
	},
	Symlink {
		from: PathBuf,
		to: PathBuf,
		delete_existing: bool,
	},
	RunCommand {
		command: String,
	},
	RunCommands {
		commands: Vec<String>,
	},
	RunScript {
		path: PathBuf,
	},
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
	/// The directory where your 'mrow.toml' resides. Defaults to CWD
	#[arg(short, long)]
	dir: Option<String>,

	/// Doesn't execute any commands, just logs them and what they would do.
	#[arg(long)]
	debug: bool,
}

/// Resolves a given path string to an absolute path.
///
/// This function handles the following cases:
/// - If the path starts with `~`, it expands it to the user's home directory.
/// - If the path is relative, it joins it with the provided `base_path`.
/// - If the path is already absolute, it is returned as is.
fn resolve_path(from_path: &str, base_path: &Path) -> PathBuf {
	let mut resolved_path = PathBuf::from(from_path);

	// Expand the home directory symbol
	if resolved_path.starts_with("~/") {
		if let Some(home_dir) = dirs::home_dir() {
			let home_str = home_dir.to_string_lossy();
			resolved_path = PathBuf::from(&*home_str).join(&from_path[2..]);
		}
	} else if resolved_path.is_relative() {
		resolved_path = base_path.join(resolved_path);
	}

	resolved_path
}

fn collapse_path(base_dir: &Path, path: &Path) -> PathBuf {
	let mut parts = vec![];
	let mut parent = path.parent().unwrap_or_else(|| {
		unreachable!("the program doesn't allow for placing a mrow.toml file in the root of a filesystem")
	});
	while parent != base_dir {
		if let Some(name) = parent.file_name() {
			parts.push(name);
		} else if parent.ends_with("..") {
			parent = parent.parent().unwrap_or_else(|| {
				unreachable!("the program doesn't allow for placing a mrow.toml file in the root of a filesystem")
			});
		}

		parent = parent.parent().unwrap_or_else(|| {
			unreachable!("the program doesn't allow for placing a mrow.toml file in the root of a filesystem")
		});
	}

	PathBuf::new().join(parts.into_iter().rev().collect::<PathBuf>()).join(
		path.file_name()
			.unwrap_or_else(|| unreachable!("linux requires that directories have names")),
	)
}

fn check_os_release() -> Result<()> {
	let regex = Regex::new(r#"(\w+)="?([^"|^\n]+)"#).unwrap_or_else(|_| unreachable!("regex should always be valid"));

	let mut is_arch = false;
	for line in std::fs::read_to_string("/etc/os-release")?.lines() {
		let captures = regex.captures(line).expect("Failed to parse os-release file");
		let key = captures.get(1).expect("non-standard os-release file???").as_str();
		let value = captures.get(2).expect("non-standard os-release file???").as_str();

		if (key == "ID" || key == "ID_LIKE") && value.to_lowercase() == "arch" {
			is_arch = true;
			break;
		}
	}

	if !is_arch {
		return Err(Error::NotArch);
	}

	Ok(())
}

fn install_packages(
	debug: bool,
	owner: &Path,
	packages: &[String],
	aur_flag: bool,
	aur_helper: Option<AurHelper>,
) -> Result<()> {
	let (command, extra_args) = if let Some(aur_helper) = aur_helper {
		match aur_helper {
			AurHelper::Yay => ("yay", vec!["-Sy"]),
			AurHelper::Paru => ("paru", if aur_flag { vec!["-Sya"] } else { vec!["-Sy"] }),
		}
	} else {
		("sudo", vec!["pacman", "-Sy"])
	};

	let mut cmd = std::process::Command::new(command);
	cmd.args(extra_args.clone())
		.arg("--noconfirm")
		.arg("--needed")
		.args(packages);

	if debug {
		debug!("{cmd:?}");
	} else {
		let cmd = cmd.output()?;
		if !cmd.status.success() {
			return Err(Error::StepFailed(
				owner.to_string_lossy().into_owned(),
				String::from_utf8_lossy(&cmd.stderr).into_owned(),
			));
		}
	}

	Ok(())
}

fn run_command_raw<S: AsRef<OsStr>>(debug: bool, owner: &Path, command: &str, args: &[S], dir: &str) -> Result<()> {
	let mut cmd = std::process::Command::new(command);
	cmd.args(args).current_dir(dir);

	if debug {
		debug!("{cmd:?}");
	} else {
		let cmd = cmd.output()?;
		if !cmd.status.success() {
			return Err(Error::StepFailed(
				owner.to_string_lossy().into_owned(),
				String::from_utf8_lossy(&cmd.stderr).into_owned(),
			));
		}
	}

	Ok(())
}

fn run_command(debug: bool, owner: &Path, command: &str) -> Result<()> {
	let command_and_args = command.split(' ').collect::<Vec<_>>();
	let mut cmd = std::process::Command::new(command_and_args[0]);
	cmd.args(&command_and_args[1..]);

	if debug {
		debug!("{cmd:?}");
	} else {
		let cmd = cmd.output()?;
		if !cmd.status.success() {
			return Err(Error::StepFailed(
				owner.to_string_lossy().into_owned(),
				String::from_utf8_lossy(&cmd.stderr).into_owned(),
			));
		}
	}

	Ok(())
}

fn run_commands(debug: bool, owner: &Path, commands: &[String]) -> Result<()> {
	for command in commands {
		let chained_commands = command.split("&&");
		for command in chained_commands {
			run_command(debug, owner, command.trim())?;
		}
	}

	Ok(())
}

fn _main() -> Result<()> {
	colog::default_builder().filter_level(log::LevelFilter::Debug).init();

	check_os_release()?;

	let args = Args::parse();
	let base_dir = match args.dir {
		Some(ref dir) => PathBuf::from(dir).canonicalize()?,
		None => std::env::current_dir()?,
	};

	if !base_dir.exists() {
		error!("Dir '{}' doesn't exist!", base_dir.to_string_lossy());
		exit(-1);
	}

	let mut lua = true;
	let mut root_file = base_dir.join("mrow.luau");
	if !root_file.exists() {
		root_file = base_dir.join("mrow.toml");
		lua = false;
		if !root_file.exists() {
			error!("No mrow.toml or mrow.luau found in '{}'", base_dir.to_string_lossy());
			exit(-1);
		}
	}

	let username = std::env::var("USER")?;

	warn!("If the expected username is not '{username}' then CTRL-C and re-run!");
	warn!(
		"Adjust your sudo timestamp_timeout value to be longer than the install should take otherwise it may \
		 eventually ask for authentication again."
	);
	warn!(
		"To avoid this, CTRL+C and run `sudo visudo -f {username}`. Then paste the following line:Defaults \
		 timestamp_timeout=<TIME_IN_MINUTES>"
	);
	println!();
	info!(
		"Enter your user password. The rest of the install wont require any user interaction unless it fails.Go make \
		 tea!"
	);

	let hostname = std::fs::read_to_string("/etc/hostname")?;
	let hostname = hostname.trim();
	let (all_steps, aur_helper) = if lua {
		mrow_lua::process(base_dir, &root_file, hostname)?
	} else {
		mrow_toml::process(&base_dir, &root_file, hostname)?
	};

	if !args.debug {
		let sudo_out = std::process::Command::new("sudo").args(["ls"]).output()?;
		if !sudo_out.status.success() {
			error!("sudo elevation failed:");
			error!("{}", String::from_utf8_lossy(sudo_out.stderr.as_slice()));
			exit(-1);
		}
	}

	println!();
	if let Some(aur_helper) = aur_helper {
		let name = match aur_helper {
			AurHelper::Yay => "yay",
			AurHelper::Paru => "paru-bin",
		};

		match run_command(args.debug, &root_file, &format!("pacman -Qi {name}")) {
			Ok(()) => {
				info!("AUR helper {name} is already installed, skipping install");
			}
			Err(Error::StepFailed(..)) => {
				info!("AUR helper {name} not installed, installing now!");

				info!("Installing prerequisite packages (base-devel group and git)");
				install_packages(
					args.debug,
					&root_file,
					&["base-devel".into(), "git".into()],
					false,
					None,
				)?;

				info!("Cloning {name} repo into /opt/{name}");
				run_commands(
					args.debug,
					&root_file,
					&[
						format!("sudo git clone https://aur.archlinux.org/{name}.git /opt/{name}"),
						format!("sudo chown -R {username}: /opt/{name}"),
					],
				)?;

				info!("Building and installing {name}");
				run_command_raw(
					args.debug,
					&root_file,
					"makepkg",
					&["-si", "--noconfirm"],
					&format!("/opt/{name}"),
				)?;

				info!("{name} installed");
			}
			Err(err) => Err(err)?,
		}
	}

	if aur_helper.is_none() {
		for step in &all_steps {
			if let StepKind::InstallPackage { package: _, aur: true }
			| StepKind::InstallPackages { packages: _, aur: true } = step.kind
			{
				error!(
					"An install package step in '{}' requires AUR but there is no AUR helper set in your mrow.toml",
					step.relative_path_str
				);
				exit(-1);
			}
		}
	}

	for step in all_steps {
		match step.kind {
			StepKind::InstallPackage { package, aur } => {
				info!(
					"[{}] Installing {}package: {}",
					step.relative_path_str,
					if aur { "AUR " } else { "" },
					package
				);

				install_packages(args.debug, &step.owner, &[package], aur, aur_helper.filter(|_| aur))?;
			}
			StepKind::InstallPackages { packages, aur } => {
				info!(
					"[{}] Installing {}packages:\n{}",
					step.relative_path_str,
					if aur { "AUR " } else { "" },
					packages.join("\n")
				);

				install_packages(args.debug, &step.owner, &packages, aur, aur_helper.filter(|_| aur))?;
			}
			StepKind::CopyFile { from, to, as_root } => {
				info!(
					"[{}] Copying file '{}' to '{}'{}",
					step.relative_path_str,
					from.to_string_lossy(),
					to.to_string_lossy(),
					if as_root { " as root" } else { "" }
				);

				run_commands(
					args.debug,
					&step.owner,
					&[format!(
						"{}cp {} {}",
						if as_root { "sudo " } else { "" },
						from.to_string_lossy(),
						to.to_string_lossy()
					)],
				)?;
			}
			StepKind::Symlink {
				from,
				to,
				delete_existing,
			} => {
				info!(
					"[{}] Creating symlink from '{}' to '{}'{}",
					step.relative_path_str,
					from.to_string_lossy(),
					to.to_string_lossy(),
					if delete_existing {
						" deleting anything in its current place"
					} else {
						""
					}
				);

				if to.exists() && !delete_existing {
					warn!("Not creating symlink as the destination already exists");
					continue;
				}

				if to.exists() {
					if let Some(to_parent) = to.parent() {
						run_commands(
							args.debug,
							&step.owner,
							&[format!("mkdir -p {}", to_parent.to_string_lossy())],
						)?;
					}
				}

				run_commands(
					args.debug,
					&step.owner,
					&[format!("ln -s {} {}", from.to_string_lossy(), to.to_string_lossy())],
				)?;
			}
			StepKind::RunCommand { command } => {
				info!("[{}] Running command '{}'", step.relative_path_str, &command);

				run_commands(args.debug, &step.owner, &[command])?;
			}
			StepKind::RunCommands { commands } => {
				info!(
					"[{}] Running commands:\n{}",
					step.relative_path_str,
					commands.join("\n")
				);

				run_commands(args.debug, &step.owner, &commands)?;
			}
			StepKind::RunScript { path } => {
				info!(
					"[{}] Running shell script '{}'",
					step.relative_path_str,
					path.to_string_lossy()
				);

				run_command_raw(
					args.debug,
					&step.owner,
					"sh",
					&[&path.to_string_lossy().into_owned()],
					&path
						.parent()
						.unwrap_or_else(|| {
							unreachable!(
								"the program doesn't allow for placing a mrow.toml file in the root of a filesystem"
							)
						})
						.to_string_lossy(),
				)?;
			}
		}
	}

	Ok(())
}

fn main() -> miette::Result<()> {
	_main().into_diagnostic()?;
	Ok(())
}
