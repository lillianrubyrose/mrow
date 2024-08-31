#![warn(clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::{
    path::{Path, PathBuf},
    process::exit,
};

use clap::Parser;
use miette::IntoDiagnostic;
use rand::{distributions::Alphanumeric, prelude::Distribution, rngs::OsRng};
use regex::Regex;
use serde::Deserialize;
use thiserror::Error;
use toml::Value;

#[derive(Debug, Error)]
enum Error {
    #[error("This tool is made to work with Arch Linux and may work with some derivatives")]
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
    StepFailed(PathBuf, String),

    #[error("'{0}': {1}")]
    Toml(PathBuf, toml::de::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
    WriteFile {
        path: PathBuf,
        content: String,
        overwrite: bool,
        as_root: bool,
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

    fn new(path: &Path) -> Result<MrowFile> {
        let dir = path
            .parent()
            .unwrap_or_else(|| unreachable!("Don't run in '/' you goober."))
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
                            .ok_or(Error::InvalidStepGeneric(
                                path.clone(),
                                "Missing step kind.",
                            ))?;

                        match kind.as_str() {
                            "install-package" => {
                                let package = table
                                    .remove("package")
                                    .and_then(|v| v.as_str().map(ToString::to_string))
                                    .ok_or(Error::InvalidStepGeneric(
                                        path.clone(),
                                        "Missing 'package' key in install-package step.",
                                    ))?;

                                let aur = table
                                    .remove("aur")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

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

                                let aur = table
                                    .remove("aur")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

                                StepKind::InstallPackages { packages, aur }
                            }

                            "write-file" => {
                                let file_path = table
                                    .remove("path")
                                    .map(|v| {
                                        v.as_str()
                                            .map(ToString::to_string)
                                            .ok_or(Error::InvalidStep(path.clone(), v))
                                    })
                                    .ok_or(Error::InvalidStepGeneric(
                                        path.clone(),
                                        "Missing 'path' key in write-file step.",
                                    ))??;

                                let content = table
                                    .remove("content")
                                    .map(|v| {
                                        v.as_str()
                                            .map(ToString::to_string)
                                            .ok_or(Error::InvalidStep(path.clone(), v))
                                    })
                                    .ok_or(Error::InvalidStepGeneric(
                                        path.clone(),
                                        "Missing 'content' key in write-file step.",
                                    ))??;

                                let overwrite = table
                                    .remove("overwrite")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

                                let as_root = table
                                    .remove("as-root")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

                                StepKind::WriteFile {
                                    path: Self::resolve_path(&file_path, &dir),
                                    content,
                                    overwrite,
                                    as_root,
                                }
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

                                let as_root = table
                                    .remove("as-root")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

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

fn gather_includes(file: &MrowFile) -> Result<Vec<MrowFile>> {
    match &file.module.includes {
        Includes::None => vec![],
        Includes::One(include) => vec![PathBuf::from(include)],
        Includes::Many(includes) => includes.iter().map(PathBuf::from).collect(),
    }
    .into_iter()
    .map(|path| file.dir.join(path))
    .map(|path| {
        if path.exists() {
            MrowFile::new(&path)
        } else {
            Err(Error::ImportNotFound(file.path.clone(), path))
        }
    })
    .collect()
}

fn get_all_steps(base: &MrowFile) -> Result<Vec<Step>> {
    let includes = gather_includes(base)?;

    includes
        .iter()
        .filter(|include| include.module.steps.is_empty() && include.module.includes.empty())
        .for_each(|include| {
            println!(
                "[?] '{}' has no steps or includes.",
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
            kind,
        })
        .collect::<Vec<_>>();
    for include in includes {
        steps.extend(get_all_steps(&include)?);
    }
    Ok(steps)
}

fn check_os_release() -> Result<()> {
    let regex = Regex::new(r#"(\w+)="?([^"|^\n]+)"#)
        .unwrap_or_else(|_| unreachable!("should never happen"));

    let mut is_arch = false;
    for line in std::fs::read_to_string("/etc/os-release")?.lines() {
        let captures = regex
            .captures(line)
            .expect("Failed to parse os-release file");
        let key = captures
            .get(1)
            .expect("non-standard os-release file???")
            .as_str();
        let value = captures
            .get(2)
            .expect("non-standard os-release file???")
            .as_str();

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

fn rand_str(length: usize) -> String {
    Alphanumeric
        .sample_iter(&mut OsRng)
        .take(length)
        .map(char::from)
        .collect()
}

fn install_packages(
    debug: bool,
    owner: PathBuf,
    packages: &[String],
    aur_helper: Option<AurHelper>,
) -> Result<()> {
    let (command, flags) = if let Some(aur_helper) = aur_helper {
        match aur_helper {
            AurHelper::Yay => ("yay", "-Syu"),
            AurHelper::Paru => ("paru", "-Syua"),
        }
    } else {
        ("pacman", "-Syu")
    };

    if debug {
        println!("[D] {command} {flags} {}", packages.join(" "));
    } else {
        let cmd = std::process::Command::new(command)
            .arg(flags)
            .arg("--noconfirm")
            .args(packages)
            .output()?;
        if !cmd.status.success() {
            return Err(Error::StepFailed(
                owner,
                String::from_utf8_lossy(&cmd.stderr).into_owned(),
            ));
        }
    }

    Ok(())
}

fn run_commands(debug: bool, owner: PathBuf, commands: &[String]) -> Result<()> {
    for command in commands {
        let command_and_args = command.split(' ').collect::<Vec<_>>();
        if debug {
            println!("[D] {command_and_args:?}");
        } else {
            let cmd = std::process::Command::new(command_and_args[0])
                .args(&command_and_args[1..])
                .output()?;
            if !cmd.status.success() {
                return Err(Error::StepFailed(
                    owner,
                    String::from_utf8_lossy(&cmd.stderr).into_owned(),
                ));
            }
        }
    }

    Ok(())
}

fn _main() -> Result<()> {
    check_os_release()?;

    let args = Args::parse();
    let base_dir = match args.dir {
        Some(dir) => PathBuf::from(dir).canonicalize()?,
        None => std::env::current_dir()?,
    };

    if !base_dir.exists() {
        println!("[!] '{}' doesn't exist!", base_dir.to_string_lossy());
        exit(-1);
    }

    let root_file = base_dir.join("mrow.toml");
    if !root_file.exists() {
        println!("[!] No mrow.toml found in '{}'", base_dir.to_string_lossy());
        exit(-1);
    }

    let root = MrowFile::new(&root_file)?;
    let all_steps = get_all_steps(&root)?;
    let aur_helper = root.config.and_then(|c| c.aur_helper);

    if !args.debug {
        println!("[*] NOTE: Adjust your sudo timestamp_timeout value to be longer than the install should take otherwise it may eventually ask for authentication again.");
        println!("[*] NOTE: To avoid this, CTRL+C and run `sudo visudo -f $USER`. Then paste the following line:");
        println!("Defaults timestamp_timeout=<TIME_IN_MINUTES>");
        println!("---------");
        println!("[*] Enter your user password. The rest of the install wont require any user interaction unless it fails. Go make tea!");

        let sudo_out = std::process::Command::new("sudo").args(["ls"]).output()?;
        if !sudo_out.status.success() {
            println!("[!] sudo check failed:");
            println!("{}", String::from_utf8_lossy(sudo_out.stderr.as_slice()));
            exit(-1);
        }
    }

    for step in all_steps {
        match step.kind {
            StepKind::InstallPackage { package, aur } => {
                if args.debug {
                    println!("[D] InstallPackage package={package} aur={aur}");
                }

                install_packages(args.debug, step.owner.clone(), &[package], aur_helper)?;
            }
            StepKind::InstallPackages { packages, aur } => {
                if args.debug {
                    println!("[D] InstallPackages packages={packages:?} aur={aur}");
                }

                install_packages(args.debug, step.owner.clone(), &packages, aur_helper)?;
            }
            StepKind::WriteFile {
                path,
                content,
                overwrite,
                as_root,
            } => {
                if args.debug {
                    println!("[D] WriteFile path={path:?} content={content} overwrite={overwrite} as_root={as_root}");
                    continue;
                }

                if path.exists() && !overwrite {
                    println!("[D] File already exists");
                    continue;
                }

                let parent = path.parent().unwrap_or_else(|| unreachable!());

                if as_root {
                    let tmp = format!("/tmp/{}", rand_str(16));
                    run_commands(
                        args.debug,
                        step.owner.clone(),
                        &[
                            format!("sudo mkdir -p {}", parent.to_string_lossy()),
                            format!("echo {content} | sudo tee {tmp}"),
                            format!("sudo chown root: {tmp}"),
                            format!("sudo mv {tmp} {}", path.to_string_lossy()),
                        ],
                    )?;
                } else {
                    std::fs::create_dir_all(parent)?;
                    std::fs::write(path, content)?;
                }
            }
            StepKind::CopyFile { from, to, as_root } => {
                if args.debug {
                    println!("[D] CopyFile from={from:?} to={to:?} as_root={as_root}");
                    continue;
                }

                let to_parent = to.parent().unwrap_or_else(|| unreachable!());

                if as_root {
                    run_commands(
                        args.debug,
                        step.owner.clone(),
                        &[format!(
                            "sudo cp {} {}",
                            from.to_string_lossy(),
                            to.to_string_lossy()
                        )],
                    )?;
                } else {
                    std::fs::create_dir_all(to_parent)?;
                    std::fs::copy(from, to)?;
                }
            }
            StepKind::Symlink {
                from,
                to,
                delete_existing,
            } => {
                if args.debug {
                    println!(
                        "[D] Symlink from={from:?} to={to:?} delete_existing={delete_existing}"
                    );
                }

                if to.exists() && !delete_existing {
                    println!("[D] File already exists");
                    continue;
                }

                if !args.debug {
                    let to_parent = to.parent().unwrap_or_else(|| unreachable!());
                    std::fs::create_dir_all(to_parent)?;

                    if delete_existing {
                        if to.is_dir() {
                            std::fs::remove_dir_all(&to)?;
                        } else {
                            std::fs::remove_file(&to)?;
                        }
                    }
                }

                run_commands(
                    args.debug,
                    step.owner.clone(),
                    &[format!(
                        "ln -s {} {}",
                        from.to_string_lossy(),
                        to.to_string_lossy()
                    )],
                )?;
            }
            StepKind::RunCommand { command } => {
                if args.debug {
                    println!("[D] RunCommand command={command}");
                }

                run_commands(args.debug, step.owner.clone(), &[command])?;
            }
            StepKind::RunCommands { commands } => {
                if args.debug {
                    println!("[D] RunCommands commands={commands:?}");
                }

                run_commands(args.debug, step.owner.clone(), &commands)?;
            }
        }
    }

    Ok(())
}

fn main() -> miette::Result<()> {
    _main().into_diagnostic()?;
    Ok(())
}
