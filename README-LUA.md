# mrow lua usage

## Quickstart

1. Create a new directory to use (`mkdir ~/mrowfiles`)

2. **Create a `mrow.luau` File**

   Here's bare minimum `mrow.luau`:

   ```lua
   return (function(): MrowRoot
      return {
         init = function()
            mrow.run_command("touch cutie")
         end,
      };
   end)();
   ```

3. **Run**

   `./path/to/mrow`

## AUR

To install AUR packages through mrow, you must specify an AUR helper to use.

mrow supports [Yay](https://github.com/Jguer/yay) and [Paru](https://github.com/Morganamilo/paru).

```lua
return (function(): MrowRoot
   return {
      ...
      aur_helper = "yay" -- or paru
   };
end)();
```

## Includes

In any module you can use `require` *almost* as usual.

Paths can be relative to the parent of the module, absolute, or prefixed with `@/` to start at the parent of `mrow.luau`.
(If `mrow.luau` was in `~/mrowfiles/` and you did `require("@/modules/keys")` from ANY module, it would search for it at `~/mrowfiles/modules/keys.luau`)

```lua
require("../meow")
require("@/modules/keys")
```

## Available modules

You have access to all default LuaU modules and a `mrow` module.

The mrow module contains:
```lua
hostname: string;
base_dir: string;

function install_package(package: string, aur: boolean?) end
function install_packages(packages: {[number]: string}, aur: boolean?) end
function copy_file(from: string, to: string, as_root: boolean?) end
function symlink(from: string, to: string, delete_existing: boolean?) end
function run_command(command: string) end
function run_commands(commands: {[number]: string}) end
function run_script(path: string) end
```

mrow also adds some globals:
```lua
type AurHelper = "yay" | "paru"
type MrowRoot = { init: () -> (), aur_helper: AurHelper? }

function log_info(message: string)  end
function log_warn(message: string)  end
function log_error(message: string) end
function log_debug(message: string) end
```

## All step kinds

- **Install Package**

   ```lua
   mrow.install_package("vim", false) -- optional: aur = false, defaults to false. can be omitted entirely.
   ```

- **Install Multiple Packages**

   ```lua
   mrow.install_packages({"vim", "htop"}, false) -- optional: aur = false, defaults to false. can be omitted entirely.
   ```

- **Copy File**

   Paths can be absolute or relative to the parent of the module.

   ```lua
   mrow.copy_file("relative/config.json", "/app/config.json", true) -- optional: as_root = false, defaults to false. can be omitted entirely.
   ```

- **Create Symlink**

   Paths can be absolute or relative to the parent of the module.

   ```lua
   mrow.symlink("dots/kitty.conf", "~/.config/kitty/kitty.conf", true) -- optional: delete_existing = false, defaults to false. can be omitted entirely.
   ```

- **Run Command**

   ```lua
   mrow.run_command("sudo mkdir -p /var/lib/meow")
   ```

- **Run Multiple Commands**

   ```lua
   mrow.run_commands({"sudo mkdir -p /var/lib/meow", "sudo mkdir -p /var/lib/bark"})
   ```

- **Run Script**

   Paths can be absolute or relative to the parent of the module.

   ```lua
   mrow.run_script("path/to/script")
   ```
