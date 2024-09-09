-- Types definition file for LuaU. Useful to load in luau-lsp

export type AurHelper = "yay" | "paru"
export type MrowRoot = { init: () -> (), aur_helper: AurHelper? }

declare mrow: {
   hostname: string,
   base_dir: string,

   install_package: @checked (package: string, aur: boolean?) -> (),
   install_packages: @checked (packages: {[number]: string}, aur: boolean?) -> (),
   copy_file: @checked (from: string, to: string, as_root: boolean?) -> (),
   symlink: @checked (from: string, to: string, delete_existing: boolean?) -> (),
   run_command: @checked (command: string) -> (),
   run_commands: @checked (commands: {[number]: string}) -> (),
   run_script: @checked (path: string) -> (),
}

-- overload this so luau-lsp doesn't freak out over invalid path
@checked declare function require(relative_path: string): any

@checked declare function log_info(message: string): ()
@checked declare function log_warn(message: string): ()
@checked declare function log_error(message: string): ()
@checked declare function log_debug(message: string): ()
