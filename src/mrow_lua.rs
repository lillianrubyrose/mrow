use crate::*;

fn get_function_caller_path(lua: &Lua, base_dir: &Path) -> mlua::Result<PathBuf> {
	static TRACE_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
		Regex::new(r"^(.+[/|\\].+.luau):\d+[.+]?$").unwrap_or_else(|_| unreachable!("regex should always be valid"))
	});

	// debug.traceback gives something like:
	//
	// [string "src/main.rs:611:9"]:1
	// [string "src/main.rs:636:9"]:1 function install_package
	// /home/lily/Dev/projects/mrow/examples/lua/modules/term.luau:1
	// [string "src/main.rs:683:14"]:1
	// /home/lily/Dev/projects/mrow/examples/lua/hosts/nya.luau:3
	// [string "src/main.rs:683:14"]:1
	// [string "src/main.rs:704:22"]:1
	//
	// The first instance of a valid path is the caller. If there is none, assume root.
	let trace = lua.load(r"debug.traceback(nil, nil)").eval::<String>()?;
	Ok(match trace.lines().find_map(|l| TRACE_PATH_REGEX.captures(l)) {
		Some(captures) => {
			let Some(path) = captures.get(1) else { unreachable!() };
			PathBuf::from(path.as_str())
		}
		_ => base_dir.join("mrow.luau").clone(),
	})
}

pub fn process(base_dir: PathBuf, root_file: &Path, hostname: &str) -> Result<(Vec<Step>, Option<AurHelper>)> {
	let steps: Rc<Mutex<Vec<Step>>> = Rc::default();
	let aur_helper: Rc<Mutex<Option<AurHelper>>> = Rc::default();

	let lua = Lua::new();
	lua.sandbox(true)?;
	lua.load_from_std_lib(StdLib::ALL)?;
	lua.load(r"function install_package(package: string, aur: boolean) mrow.install_package(package, aur) end")
		.eval::<()>()?;

	let mrow_export = lua.create_table()?;
	mrow_export.set("hostname", hostname)?;
	mrow_export.set("base_dir", base_dir.to_string_lossy().trim())?;

	{
		let aur_helper = aur_helper.clone();
		mrow_export.set(
			"set_aur_helper",
			lua.create_function(move |_, helper: String| {
				*aur_helper.lock().unwrap() = Some(match helper.to_lowercase().as_str() {
					"yay" => AurHelper::Yay,
					"paru" => AurHelper::Paru,
					v => panic!("Invalid AUR helper: {v}"),
				});
				Ok(())
			})?,
		)?;
	}

	// Install package
	{
		let base_dir = base_dir.clone();
		let steps = steps.clone();
		mrow_export.set(
			"install_package",
			lua.create_function(move |lua, (package, aur): (String, Option<bool>)| {
				let owner = get_function_caller_path(lua, &base_dir)?;
				let relative_path_str = collapse_path(&base_dir, &owner).to_string_lossy().into_owned();
				let kind = StepKind::InstallPackage {
					package,
					aur: aur.unwrap_or_default(),
				};
				steps
					.lock()
					.map_err(|e| mlua::Error::runtime(e.to_string()))?
					.push(Step {
						owner,
						relative_path_str,
						kind,
					});
				Ok(())
			})?,
		)?;
	}

	// Install packages
	{
		let base_dir = base_dir.clone();
		let steps = steps.clone();
		mrow_export.set(
			"install_packages",
			lua.create_function(move |lua, (packages, aur): (Vec<String>, Option<bool>)| {
				let owner = get_function_caller_path(lua, &base_dir)?;
				let relative_path_str = collapse_path(&base_dir, &owner).to_string_lossy().into_owned();
				let kind = StepKind::InstallPackages {
					packages,
					aur: aur.unwrap_or_default(),
				};
				steps
					.lock()
					.map_err(|e| mlua::Error::runtime(e.to_string()))?
					.push(Step {
						owner,
						relative_path_str,
						kind,
					});
				Ok(())
			})?,
		)?;
	}

	// Copy file
	{
		let base_dir = base_dir.clone();
		let steps = steps.clone();
		mrow_export.set(
			"copy_file",
			lua.create_function(move |lua, (from, to, as_root): (String, String, Option<bool>)| {
				let owner = get_function_caller_path(lua, &base_dir)?;
				let Some(parent) = owner.parent() else { unreachable!() };
				let relative_path_str = collapse_path(&base_dir, &owner).to_string_lossy().into_owned();
				let kind = StepKind::CopyFile {
					from: resolve_path(&from, parent),
					to: resolve_path(&to, parent),
					as_root: as_root.unwrap_or_default(),
				};
				steps
					.lock()
					.map_err(|e| mlua::Error::runtime(e.to_string()))?
					.push(Step {
						owner,
						relative_path_str,
						kind,
					});
				Ok(())
			})?,
		)?;
	}

	// Symlink
	{
		let base_dir = base_dir.clone();
		let steps = steps.clone();
		mrow_export.set(
			"symlink",
			lua.create_function(
				move |lua, (from, to, delete_existing): (String, String, Option<bool>)| {
					let owner = get_function_caller_path(lua, &base_dir)?;
					let Some(parent) = owner.parent() else { unreachable!() };
					let relative_path_str = collapse_path(&base_dir, &owner).to_string_lossy().into_owned();
					let kind = StepKind::Symlink {
						from: resolve_path(&from, parent),
						to: resolve_path(&to, parent),
						delete_existing: delete_existing.unwrap_or_default(),
					};
					steps
						.lock()
						.map_err(|e| mlua::Error::runtime(e.to_string()))?
						.push(Step {
							owner,
							relative_path_str,
							kind,
						});
					Ok(())
				},
			)?,
		)?;
	}

	// Run command
	{
		let base_dir = base_dir.clone();
		let steps = steps.clone();
		mrow_export.set(
			"run_command",
			lua.create_function(move |lua, command: String| {
				let owner = get_function_caller_path(lua, &base_dir)?;
				let relative_path_str = collapse_path(&base_dir, &owner).to_string_lossy().into_owned();
				let kind = StepKind::RunCommand { command };
				steps
					.lock()
					.map_err(|e| mlua::Error::runtime(e.to_string()))?
					.push(Step {
						owner,
						relative_path_str,
						kind,
					});
				Ok(())
			})?,
		)?;
	}

	// Run commands
	{
		let base_dir = base_dir.clone();
		let steps = steps.clone();
		mrow_export.set(
			"run_commands",
			lua.create_function(move |lua, commands: Vec<String>| {
				let owner = get_function_caller_path(lua, &base_dir)?;
				let relative_path_str = collapse_path(&base_dir, &owner).to_string_lossy().into_owned();
				let kind = StepKind::RunCommands { commands };
				steps
					.lock()
					.map_err(|e| mlua::Error::runtime(e.to_string()))?
					.push(Step {
						owner,
						relative_path_str,
						kind,
					});
				Ok(())
			})?,
		)?;
	}

	// Run script
	{
		let base_dir = base_dir.clone();
		let steps = steps.clone();
		mrow_export.set(
			"run_script",
			lua.create_function(move |lua, path: String| {
				let owner = get_function_caller_path(lua, &base_dir)?;
				let relative_path_str = collapse_path(&base_dir, &owner).to_string_lossy().into_owned();
				let kind = StepKind::RunScript {
					path: resolve_path(&path, &base_dir),
				};
				steps
					.lock()
					.map_err(|e| mlua::Error::runtime(e.to_string()))?
					.push(Step {
						owner,
						relative_path_str,
						kind,
					});
				Ok(())
			})?,
		)?;
	}

	lua.globals().set("mrow", mrow_export)?;
	lua.globals()
		.set("_require", lua.globals().raw_get::<_, mlua::Function>("require")?)?;
	lua.globals().set(
		"require",
		lua.create_function(move |lua, relative_path: String| {
			let path = if let Some(relative_path) = relative_path.strip_prefix("@/") {
				base_dir.join(relative_path)
			} else {
				get_function_caller_path(lua, &base_dir)?
					.parent()
					.unwrap_or_else(|| {
						unreachable!(
							"the program doesn't allow for placing a mrow.luau file in the root of a filesystem"
						)
					})
					.to_path_buf()
					.join(relative_path)
			};

			lua.load(format!(r#"_require("{}")"#, path.to_string_lossy()))
				.eval::<mlua::Value>()
		})?,
	)?;

	let create_log_fn = |level: log::Level| {
		lua.create_function(move |_, message: String| {
			log::log!(level, "{message}");
			Ok(())
		})
	};
	lua.globals().set("log_info", create_log_fn(log::Level::Info)?)?;
	lua.globals().set("log_warn", create_log_fn(log::Level::Warn)?)?;
	lua.globals().set("log_debug", create_log_fn(log::Level::Debug)?)?;
	lua.globals().set("log_error", create_log_fn(log::Level::Error)?)?;

	let script = lua.load(std::fs::read_to_string(root_file)?);
	script.eval::<()>()?;

	let steps = std::mem::take(&mut *steps.lock().unwrap());
	let aur_helper = (*aur_helper.lock().unwrap()).take();
	Ok((steps, aur_helper))
}
