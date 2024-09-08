#![warn(clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::{
	env::VarError,
	ffi::OsStr,
	path::{Path, PathBuf},
	process::exit,
};

use clap::Parser;
use log::{debug, error, info, warn};
use miette::IntoDiagnostic;
use regex::Regex;
use serde::Deserialize;
use thiserror::Error;
use toml::Value;

#[derive(Debug, Error)]
enum Error {
	#[error("This tool is made for Arch Linux, if you're running an Arch derivative and still getting this message open an issue @ https://github.com/lillianrubyrose/mrow")]
	NotArch,

	#[error("Imported module from '{0}' doesn't exist: '{1}'")]
	ImportNotFound(PathBuf, PathBuf),

	#[error("Invalid step in '{0}'. '{1}'")]
	InvalidStep(PathBuf, Value),
	#[error("Invalid step in '{0}'. {1}")]
	InvalidStepGeneric(PathBuf, &'static str),
	#[error("Invalid step in '{0}'. {1}")]
	InvalidStepGenericOwned(PathBuf, String),

	#[error("Step in '{0}' failed. {1}")]
	StepFailed(String, String),

	#[error("'{0}': {1}")]
	Toml(PathBuf, toml::de::Error),
	#[error(transparent)]
	Io(#[from] std::io::Error),
	#[error(transparent)]
	Var(#[from] VarError),
}

type Result<T> = miette::Result<T, Error>;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Includes {
	None,
	One(String),
	Many(Vec<String>),
}

impl Default for Includes {
	fn default() -> Self {
		Self::None
	}
}

impl Includes {
	fn empty(&self) -> bool {
		match self {
			Includes::None => true,
			Includes::One(include) => include.is_empty(),
			Includes::Many(includes) => includes.is_empty(),
		}
	}
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum AurHelper {
	Yay,
	Paru,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawConfigTable {
	aur_helper: Option<AurHelper>,
}

#[derive(Debug, Deserialize)]
struct RawModuleTable {
	#[serde(default)]
	includes: Includes,
	#[serde(default)]
	steps: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawMrowFile {
	config: Option<RawConfigTable>,
	module: RawModuleTable,
}

impl RawMrowFile {
	fn new(path: PathBuf) -> Result<RawMrowFile> {
		toml::from_str(&std::fs::read_to_string(&path)?).map_err(|err| Error::Toml(path, err))
	}
}

#[derive(Debug, Deserialize)]
struct ConfigTable {
	aur_helper: Option<AurHelper>,
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

#[derive(Debug)]
struct ModuleTable {
	includes: Includes,
	steps: Vec<StepKind>,
}

#[derive(Debug)]
struct MrowFile {
	dir: PathBuf,
	path: PathBuf,

	/// This is relative to the root mrow.toml
	relative_path_str: String,

	config: Option<ConfigTable>,
	module: ModuleTable,
}

impl MrowFile {
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

	fn new(root_dir: &Path, path: &Path) -> Result<MrowFile> {
		let relative_path = {
			if root_dir == PathBuf::new() {
				PathBuf::from("mrow.toml")
			} else {
				let mut parts = vec![];
				let mut parent = path.parent().unwrap_or_else(|| {
					unreachable!("the program doesn't allow for placing a mrow.toml file in the root of a filesystem")
				});
				while parent != root_dir {
					parts.push(
						parent
							.file_name()
							.unwrap_or_else(|| unreachable!("linux requires that directories have names")),
					);
					parent = parent.parent().unwrap_or_else(|| {
						unreachable!(
							"the program doesn't allow for placing a mrow.toml file in the root of a filesystem"
						)
					});
				}

				PathBuf::new().join(parts.into_iter().rev().collect::<PathBuf>()).join(
					path.file_name()
						.unwrap_or_else(|| unreachable!("linux requires that directories have names")),
				)
			}
		};

		let dir = path
			.parent()
			.unwrap_or_else(|| {
				unreachable!("the program doesn't allow for placing a mrow.toml file in the root of a filesystem")
			})
			.to_path_buf();
		let path = path.canonicalize()?;

		let raw = RawMrowFile::new(path.clone())?;
		let config = raw
			.config
			.map(|RawConfigTable { aur_helper }| ConfigTable { aur_helper });

		let module: ModuleTable = {
			let mut steps = Vec::with_capacity(raw.module.steps.len());

			for raw in raw.module.steps {
				let step = match raw {
					Value::String(command) => StepKind::RunCommand { command },
					Value::Array(commands) => StepKind::RunCommands {
						commands: commands
							.into_iter()
							.map(|v| {
								v.as_str()
									.map(ToString::to_string)
									.ok_or(Error::InvalidStep(path.clone(), v))
							})
							.collect::<Result<Vec<_>>>()?,
					},
					Value::Table(mut table) => {
						let kind = table
							.remove("kind")
							.and_then(|v| v.as_str().map(ToString::to_string))
							.ok_or(Error::InvalidStepGeneric(path.clone(), "Missing step kind."))?;

						match kind.as_str() {
							"install-package" => {
								let package = table
									.remove("package")
									.and_then(|v| v.as_str().map(ToString::to_string))
									.ok_or(Error::InvalidStepGeneric(
										path.clone(),
										"Missing 'package' key in install-package step.",
									))?;

								let aur = table.remove("aur").and_then(|v| v.as_bool()).unwrap_or_default();

								StepKind::InstallPackage { package, aur }
							}

							"install-packages" => {
								let packages = table
									.remove("packages")
									.and_then(|v| match v {
										Value::Array(v) => Some(v),
										_ => None,
									})
									.ok_or(Error::InvalidStepGeneric(
										path.clone(),
										"Missing 'package' key in install-package step.",
									))?
									.into_iter()
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::InvalidStep(path.clone(), v))
									})
									.collect::<Result<Vec<_>>>()?;

								let aur = table.remove("aur").and_then(|v| v.as_bool()).unwrap_or_default();

								StepKind::InstallPackages { packages, aur }
							}

							"copy-file" => {
								let from_path = table
									.remove("from")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::InvalidStep(path.clone(), v))
									})
									.ok_or(Error::InvalidStepGeneric(
										path.clone(),
										"Missing 'from' key in copy-file step.",
									))??;

								let to_path = table
									.remove("to")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::InvalidStep(path.clone(), v))
									})
									.ok_or(Error::InvalidStepGeneric(
										path.clone(),
										"Missing 'to' key in copy-file step.",
									))??;

								let as_root = table.remove("as-root").and_then(|v| v.as_bool()).unwrap_or_default();

								StepKind::CopyFile {
									from: Self::resolve_path(&from_path, &dir),
									to: Self::resolve_path(&to_path, &dir),
									as_root,
								}
							}

							"symlink" => {
								let from_path = table
									.remove("from")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::InvalidStep(path.clone(), v))
									})
									.ok_or(Error::InvalidStepGeneric(
										path.clone(),
										"Missing 'from' key in write-file step.",
									))??;

								let to_path = table
									.remove("to")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::InvalidStep(path.clone(), v))
									})
									.ok_or(Error::InvalidStepGeneric(
										path.clone(),
										"Missing 'to' key in write-file step.",
									))??;

								let delete_existing = table
									.remove("delete-existing")
									.and_then(|v| v.as_bool())
									.unwrap_or_default();

								StepKind::Symlink {
									from: Self::resolve_path(&from_path, &dir),
									to: Self::resolve_path(&to_path, &dir),
									delete_existing,
								}
							}

							"run-script" => {
								let script_path = table
									.remove("path")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::InvalidStep(path.clone(), v))
									})
									.ok_or(Error::InvalidStepGeneric(
										path.clone(),
										"Missing 'from' key in write-file step.",
									))??;

								StepKind::RunScript {
									path: Self::resolve_path(&script_path, &dir),
								}
							}

							_ => {
								return Err(Error::InvalidStepGenericOwned(
									path.clone(),
									format!("Invalid step kind: {kind}"),
								))
							}
						}
					}

					value => return Err(Error::InvalidStep(path.clone(), value)),
				};
				steps.push(step);
			}

			ModuleTable {
				includes: raw.module.includes,
				steps,
			}
		};

		Ok(MrowFile {
			dir,
			path,
			relative_path_str: relative_path.to_string_lossy().into_owned(),
			config,
			module,
		})
	}
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

fn gather_includes(root_dir: &Path, file: &MrowFile) -> Result<Vec<MrowFile>> {
	match &file.module.includes {
		Includes::None => vec![],
		Includes::One(include) => vec![PathBuf::from(include)],
		Includes::Many(includes) => includes.iter().map(PathBuf::from).collect(),
	}
	.into_iter()
	.map(|path| file.dir.join(path))
	.map(|path| {
		if path.exists() {
			MrowFile::new(root_dir, &path)
		} else {
			Err(Error::ImportNotFound(file.path.clone(), path))
		}
	})
	.collect()
}

fn get_all_steps(root_dir: &Path, base: &MrowFile) -> Result<Vec<Step>> {
	let includes = gather_includes(root_dir, base)?;

	includes
		.iter()
		.filter(|include| include.module.steps.is_empty() && include.module.includes.empty())
		.for_each(|include| {
			warn!(
				"'{}' is a no-op since it contains no steps or includes.",
				include.path.to_string_lossy()
			);
		});

	let mut steps = base
		.module
		.steps
		.iter()
		.cloned()
		.map(|kind| Step {
			owner: base.path.clone(),
			relative_path_str: base.relative_path_str.clone(),
			kind,
		})
		.collect::<Vec<_>>();
	for include in includes {
		steps.extend(get_all_steps(root_dir, &include)?);
	}
	Ok(steps)
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
		Some(dir) => PathBuf::from(dir).canonicalize()?,
		None => std::env::current_dir()?,
	};

	if !base_dir.exists() {
		error!("Dir '{}' doesn't exist!", base_dir.to_string_lossy());
		exit(-1);
	}

	let root_file = base_dir.join("mrow.toml");
	if !root_file.exists() {
		error!("No mrow.toml found in '{}'", base_dir.to_string_lossy());
		exit(-1);
	}

	let root = MrowFile::new(&PathBuf::new(), &root_file)?;
	let all_steps = get_all_steps(&root.dir, &root)?;
	let aur_helper = root.config.and_then(|c| c.aur_helper);

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

		match run_command(args.debug, &root.path, &format!("pacman -Qi {name}")) {
			Ok(()) => {
				info!("AUR helper {name} is already installed, skipping install");
			}
			Err(Error::StepFailed(..)) => {
				info!("AUR helper {name} not installed, installing now!");

				info!("Installing prerequisite packages (base-devel group and git)");
				install_packages(
					args.debug,
					&root.path,
					&["base-devel".into(), "git".into()],
					false,
					None,
				)?;

				info!("Cloning {name} repo into /opt/{name}");
				run_commands(
					args.debug,
					&root.path,
					&[
						format!("sudo git clone https://aur.archlinux.org/{name}.git /opt/{name}"),
						format!("sudo chown -R {username}: /opt/{name}"),
					],
				)?;

				info!("Building and installing {name}");
				run_command_raw(
					args.debug,
					&root.path,
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
