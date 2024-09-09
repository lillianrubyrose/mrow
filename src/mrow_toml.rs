use crate::*;

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
struct HostInclude {
	hostname: String,
	#[serde(default)]
	includes: Includes,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawConfigTable {
	aur_helper: Option<AurHelper>,
	#[serde(default)]
	host_includes: Vec<HostInclude>,
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

#[derive(Debug, Clone)]
struct ConfigTable {
	aur_helper: Option<AurHelper>,
	host_includes: Vec<HostInclude>,
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
	fn new(root_dir: &Path, path: &Path) -> Result<MrowFile> {
		let relative_path = collapse_path(root_dir, path);

		let dir = path
			.parent()
			.unwrap_or_else(|| {
				unreachable!("the program doesn't allow for placing a mrow.toml file in the root of a filesystem")
			})
			.to_path_buf();
		let path = path.canonicalize()?;

		let raw = RawMrowFile::new(path.clone())?;
		let config = raw.config.filter(|_| relative_path == PathBuf::from("mrow.toml")).map(
			|RawConfigTable {
			     aur_helper,
			     host_includes,
			 }| ConfigTable {
				aur_helper,
				host_includes,
			},
		);

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
									.ok_or(Error::TomlInvalidStepData(path.clone(), v))
							})
							.collect::<Result<Vec<_>>>()?,
					},
					Value::Table(mut table) => {
						let kind = table
							.remove("kind")
							.and_then(|v| v.as_str().map(ToString::to_string))
							.ok_or(Error::TomlInvalidStep(path.clone(), "Missing step kind.".into()))?;

						match kind.as_str() {
							"install-package" => {
								let package = table
									.remove("package")
									.and_then(|v| v.as_str().map(ToString::to_string))
									.ok_or(Error::TomlInvalidStep(
										path.clone(),
										"Missing 'package' key in install-package step.".into(),
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
									.ok_or(Error::TomlInvalidStep(
										path.clone(),
										"Missing 'package' key in install-package step.".into(),
									))?
									.into_iter()
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::TomlInvalidStepData(path.clone(), v))
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
											.ok_or(Error::TomlInvalidStepData(path.clone(), v))
									})
									.ok_or(Error::TomlInvalidStep(
										path.clone(),
										"Missing 'from' key in copy-file step.".into(),
									))??;

								let to_path = table
									.remove("to")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::TomlInvalidStepData(path.clone(), v))
									})
									.ok_or(Error::TomlInvalidStep(
										path.clone(),
										"Missing 'to' key in copy-file step.".into(),
									))??;

								let as_root = table.remove("as-root").and_then(|v| v.as_bool()).unwrap_or_default();

								StepKind::CopyFile {
									from: resolve_path(&from_path, &dir),
									to: resolve_path(&to_path, &dir),
									as_root,
								}
							}

							"symlink" => {
								let from_path = table
									.remove("from")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::TomlInvalidStepData(path.clone(), v))
									})
									.ok_or(Error::TomlInvalidStep(
										path.clone(),
										"Missing 'from' key in write-file step.".into(),
									))??;

								let to_path = table
									.remove("to")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::TomlInvalidStepData(path.clone(), v))
									})
									.ok_or(Error::TomlInvalidStep(
										path.clone(),
										"Missing 'to' key in write-file step.".into(),
									))??;

								let delete_existing = table
									.remove("delete-existing")
									.and_then(|v| v.as_bool())
									.unwrap_or_default();

								StepKind::Symlink {
									from: resolve_path(&from_path, &dir),
									to: resolve_path(&to_path, &dir),
									delete_existing,
								}
							}

							"run-script" => {
								let script_path = table
									.remove("path")
									.map(|v| {
										v.as_str()
											.map(ToString::to_string)
											.ok_or(Error::TomlInvalidStepData(path.clone(), v))
									})
									.ok_or(Error::TomlInvalidStep(
										path.clone(),
										"Missing 'from' key in write-file step.".into(),
									))??;

								StepKind::RunScript {
									path: resolve_path(&script_path, &dir),
								}
							}

							_ => {
								return Err(Error::TomlInvalidStep(
									path.clone(),
									format!("Invalid step kind: {kind}"),
								))
							}
						}
					}

					value => return Err(Error::TomlInvalidStepData(path.clone(), value)),
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

fn gather_includes(root_dir: &Path, file: &MrowFile, includes: &Includes) -> Result<Vec<MrowFile>> {
	match &includes {
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
			Err(Error::TomlImportNotFound(file.path.clone(), path))
		}
	})
	.collect()
}

fn get_all_steps(root_dir: &Path, base: &MrowFile, host_includes: Option<Includes>) -> Result<Vec<Step>> {
	let mut includes = match host_includes.map(|i| gather_includes(root_dir, base, &i)) {
		Some(Ok(includes)) => includes,
		Some(Err(err)) => Err(err)?,
		None => vec![],
	};
	includes.extend(gather_includes(root_dir, base, &base.module.includes)?);

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
		steps.extend(get_all_steps(root_dir, &include, None)?);
	}
	Ok(steps)
}

pub fn process(base_dir: &Path, root_file: &Path, hostname: &str) -> Result<(Vec<Step>, Option<AurHelper>)> {
	let root = MrowFile::new(base_dir, root_file)?;
	let aur_helper = root.config.as_ref().and_then(|c| c.aur_helper);

	let all_steps = get_all_steps(
		&root.dir,
		&root,
		root.config
			.as_ref()
			.map(|c| c.host_includes.clone())
			.and_then(|i| i.into_iter().find(|i| i.hostname == hostname))
			.map(|i| i.includes),
	)?;

	Ok((all_steps, aur_helper))
}
