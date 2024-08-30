use std::{path::PathBuf, process::exit};

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Includes {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AurHelper {
    Yay,
    Paru,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ConfigTable {
    includes: Option<Includes>,
    aur_helper: Option<AurHelper>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
enum Command {
    InstallPackage {
        name: String,
        #[serde(default)]
        aur: bool,
    },
    OverwriteFile {
        content: String,
    },
    SingleCommand(String),
}

#[derive(Debug, Deserialize)]
struct ModuleTable {
    #[serde(default)]
    commands: Vec<Command>,
}

#[derive(Debug, Deserialize)]
struct MrowRoot {
    config: Option<ConfigTable>,
    module: Option<ModuleTable>,
}

#[derive(Debug, Deserialize)]
struct MrowModule {
    module: Option<ModuleTable>,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The directory where your 'mrow.toml' resides. Defaults to CWD
    #[arg(short, long)]
    dir: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let base_dir = match args.dir {
        Some(dir) => PathBuf::from(dir).canonicalize().into_diagnostic()?,
        None => std::env::current_dir().into_diagnostic()?,
    };

    if !base_dir.exists() {
        println!("[!] '{}' doesn't exist!", base_dir.to_string_lossy());
        exit(-1);
    }

    let mrow_file = base_dir.join("mrow.toml");
    if !mrow_file.exists() {
        println!("[!] No mrow.toml found in '{}'", base_dir.to_string_lossy());
        exit(-1);
    }

    let root: MrowRoot =
        toml::from_str(&std::fs::read_to_string(mrow_file).into_diagnostic()?).into_diagnostic()?;
    if root.config.is_none() && root.module.is_none() {
        println!("[!] There's neither a [config] or [module] table in your mrow.toml");
        exit(-1);
    }

    let included_modules: Vec<MrowModule> = root
        .config
        .as_ref()
        .map(|v| {
            v.includes.as_ref().map(|includes| match includes {
                Includes::One(include) => vec![PathBuf::from(include)],
                Includes::Many(includes) => includes.iter().map(PathBuf::from).collect(),
            })
        })
        .unwrap_or_default()
        .unwrap_or_default()
        .into_iter()
        .map(|path| std::fs::read_to_string(base_dir.join(path)))
        .collect::<std::io::Result<Vec<String>>>()
        .into_diagnostic()?
        .into_iter()
        .map(|content| toml::from_str(&content))
        .collect::<Result<Vec<MrowModule>, toml::de::Error>>()
        .into_diagnostic()?;

    let mut commands: Vec<Command> = root
        .module
        .map(|module| module.commands)
        .unwrap_or_default();

    for module in included_modules {
        if let Some(module) = module.module {
            commands.extend(module.commands);
        }
    }

    dbg!(commands);

    Ok(())
}
