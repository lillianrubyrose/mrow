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

    #[error("Invalid step in '{0}'. '{1}'")]
    InvalidStep(PathBuf, Value),
    #[error("Invalid step in '{0}'. {1}")]
    InvalidStepGeneric(PathBuf, &'static str),
    #[error("Invalid step in '{0}'. {1}")]
    InvalidStepGenericOwned(PathBuf, String),

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
    steps: Vec<Value>,
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
        steps: Vec<String>,
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
            let mut steps = Vec::with_capacity(raw.module.steps.len());

            for raw in raw.module.steps {
                let step = match raw {
                    Value::String(command) => StepKind::RunCommand { command },
                    Value::Array(commands) => StepKind::RunCommands {
                        steps: commands
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

                                StepKind::WriteFile {
                                    path: Self::resolve_path(&file_path, &dir),
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

fn get_all_steps(base: &MrowFile) -> Result<Vec<Step>> {
    let includes = gather_includes(base)?;

    includes
        .iter()
        .filter(|include| include.module.steps.is_empty() && include.module.includes.empty())
        .for_each(|include| {
            println!(
                "[?] '{}' has no steps or includes.",
                include.path.to_string_lossy()
            )
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
    let all_steps = get_all_steps(&root)?;
    dbg!(all_steps);

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
