use std::{
    path::{Path, PathBuf},
    process::exit,
};

use clap::Parser;
use miette::IntoDiagnostic;
use serde::Deserialize;
use thiserror::Error;
use toml::Value;

#[derive(Debug, Error)]
enum Error {
    #[error("Imported module from '{0}' doesn't exist: '{1}'")]
    ImportNotFound(PathBuf, PathBuf),

    #[error("Invalid command in '{0}'. '{1}'")]
    InvalidCommand(PathBuf, Value),
    #[error("Invalid command in '{0}'. {1}")]
    InvalidCommandGeneric(PathBuf, &'static str),
    #[error("Invalid command in '{0}'. {1}")]
    InvalidCommandGenericOwned(PathBuf, String),

    #[error("'{0}': {1}")]
    TomlDeserError(PathBuf, toml::de::Error),
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AurHelper {
    Yay,
    Paru,
}

#[derive(Debug, Deserialize)]
struct RawConfigTable {
    aur_helper: Option<AurHelper>,
}

#[derive(Debug, Deserialize)]
struct RawModuleTable {
    #[serde(default)]
    includes: Includes,
    #[serde(default)]
    commands: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawMrowFile {
    config: Option<RawConfigTable>,
    module: RawModuleTable,
}

impl RawMrowFile {
    fn new(path: PathBuf) -> Result<RawMrowFile> {
        Ok(toml::from_str(&std::fs::read_to_string(&path)?)
            .map_err(|err| Error::TomlDeserError(path, err))?)
    }
}

#[derive(Debug, Deserialize)]
struct ConfigTable {
    aur_helper: Option<AurHelper>,
}

#[derive(Debug, Clone)]
struct Command {
    owner: PathBuf,
    kind: CommandKind,
}

#[derive(Debug, Clone)]
enum CommandKind {
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
    },
    CopyFile {
        from_path: String,
        to_path: String,
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
    commands: Vec<CommandKind>,
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

    fn new(path: PathBuf) -> Result<MrowFile> {
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
            let mut commands = Vec::with_capacity(raw.module.commands.len());

            for raw in raw.module.commands {
                let command = match raw {
                    Value::String(command) => CommandKind::RunCommand { command },
                    Value::Array(commands) => CommandKind::RunCommands {
                        commands: commands
                            .into_iter()
                            .map(|v| {
                                v.as_str()
                                    .map(ToString::to_string)
                                    .ok_or(Error::InvalidCommand(path.clone(), v))
                            })
                            .collect::<Result<Vec<_>>>()?,
                    },
                    Value::Table(mut table) => {
                        let kind = table
                            .remove("kind")
                            .and_then(|v| v.as_str().map(ToString::to_string))
                            .ok_or(Error::InvalidCommandGeneric(
                                path.clone(),
                                "Missing command kind.",
                            ))?;

                        match kind.as_str() {
                            "install-package" => {
                                let package = table
                                    .remove("package")
                                    .and_then(|v| v.as_str().map(ToString::to_string))
                                    .ok_or(Error::InvalidCommandGeneric(
                                        path.clone(),
                                        "Missing 'package' key in install-package command.",
                                    ))?;

                                let aur = table
                                    .remove("aur")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

                                CommandKind::InstallPackage { package, aur }
                            }

                            "install-packages" => {
                                let packages = table
                                    .remove("packages")
                                    .and_then(|v| match v {
                                        Value::Array(v) => Some(v),
                                        _ => None,
                                    })
                                    .ok_or(Error::InvalidCommandGeneric(
                                        path.clone(),
                                        "Missing 'package' key in install-package command.",
                                    ))?
                                    .into_iter()
                                    .map(|v| {
                                        v.as_str()
                                            .map(ToString::to_string)
                                            .ok_or(Error::InvalidCommand(path.clone(), v))
                                    })
                                    .collect::<Result<Vec<_>>>()?;

                                let aur = table
                                    .remove("aur")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

                                CommandKind::InstallPackages { packages, aur }
                            }

                            "write-file" => {
                                let file_path = table
                                    .remove("path")
                                    .map(|v| {
                                        v.as_str()
                                            .map(ToString::to_string)
                                            .ok_or(Error::InvalidCommand(path.clone(), v))
                                    })
                                    .ok_or(Error::InvalidCommandGeneric(
                                        path.clone(),
                                        "Missing 'path' key in write-file command.",
                                    ))??;

                                let content = table
                                    .remove("content")
                                    .map(|v| {
                                        v.as_str()
                                            .map(ToString::to_string)
                                            .ok_or(Error::InvalidCommand(path.clone(), v))
                                    })
                                    .ok_or(Error::InvalidCommandGeneric(
                                        path.clone(),
                                        "Missing 'content' key in write-file command.",
                                    ))??;

                                let overwrite = table
                                    .remove("overwrite")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

                                CommandKind::WriteFile {
                                    path: PathBuf::from(file_path),
                                    content,
                                    overwrite,
                                }
                            }

                            "symlink" => {
                                let from_path = table
                                    .remove("from")
                                    .map(|v| {
                                        v.as_str()
                                            .map(ToString::to_string)
                                            .ok_or(Error::InvalidCommand(path.clone(), v))
                                    })
                                    .ok_or(Error::InvalidCommandGeneric(
                                        path.clone(),
                                        "Missing 'from' key in write-file command.",
                                    ))??;

                                let to_path = table
                                    .remove("to")
                                    .map(|v| {
                                        v.as_str()
                                            .map(ToString::to_string)
                                            .ok_or(Error::InvalidCommand(path.clone(), v))
                                    })
                                    .ok_or(Error::InvalidCommandGeneric(
                                        path.clone(),
                                        "Missing 'to' key in write-file command.",
                                    ))??;

                                let delete_existing = table
                                    .remove("delete-existing")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or_default();

                                CommandKind::Symlink {
                                    from: Self::resolve_path(&from_path, &dir),
                                    to: Self::resolve_path(&to_path, &dir),
                                    delete_existing,
                                }
                            }

                            _ => {
                                return Err(Error::InvalidCommandGenericOwned(
                                    path.clone(),
                                    format!("Invalid command kind: {kind}"),
                                ))
                            }
                        }
                    }

                    value => return Err(Error::InvalidCommand(path.clone(), value)),
                };
                commands.push(command);
            }

            ModuleTable {
                includes: raw.module.includes,
                commands,
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
            MrowFile::new(path)
        } else {
            Err(Error::ImportNotFound(file.path.clone(), path))
        }
    })
    .collect()
}

fn get_all_commands(base: &MrowFile) -> Result<Vec<CommandKind>> {
    let includes = gather_includes(base)?;

    includes
        .iter()
        .filter(|include| include.module.commands.is_empty() && include.module.includes.empty())
        .for_each(|include| {
            println!(
                "[?] '{}' has no commands or includes.",
                include.path.to_string_lossy()
            )
        });

    let mut commands = base.module.commands.clone();
    for include in includes {
        commands.extend(get_all_commands(&include)?);
    }
    Ok(commands)
}

fn _main() -> Result<()> {
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

    let root = MrowFile::new(root_file)?;
    let all_commands = get_all_commands(&root)?;
    dbg!(all_commands);

    // println!("[*] NOTE: Adjust your sudo timestamp_timeout value to be longer than the install should take otherwise it may eventually ask for authentication again.");
    // println!("[*] NOTE: To avoid this, CTRL+C and run `sudo visudo -f $USER`. Then paste the following line:");
    // println!("Defaults timestamp_timeout=<TIME_IN_MINUTES>");
    // println!("---------");
    // println!("[*] Enter your user password. The rest of the install wont require any user interaction. Go make tea!");

    // let sudo_out = std::process::Command::new("sudo").args(["ls"]).output()?;
    // if !sudo_out.status.success() {
    //     println!("[!] sudo check failed:");
    //     println!("{}", String::from_utf8_lossy(sudo_out.stderr.as_slice()));
    //     exit(-1);
    // }

    Ok(())
}

fn main() -> miette::Result<()> {
    _main().into_diagnostic()?;
    Ok(())
}
