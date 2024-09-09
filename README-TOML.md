# mrow toml usage

## Quickstart

1. Create a new directory to use (`mkdir ~/mrowfiles`)

2. **Create a `mrow.toml` File**

   Here's bare minimum `mrow.toml`:

   ```toml
   [module]
   steps = [ "touch cutie" ]
   ```

3. **Run**

   `./path/to/mrow`

## AUR

To install AUR packages through mrow, you must specify an AUR helper to use.

mrow supports [Yay](https://github.com/Jguer/yay) and [Paru](https://github.com/Morganamilo/paru).

```toml
[config]
aur-helper = "yay" # or "paru"
```

## Includes

In any module you can specify a list of other modules to include.

Paths can be relative to the parent of the module or absolute.

```toml
[module]
includes = "c/c.toml"

or

includes = ["a.toml", "b/b.toml"]
```

## Hostname includes

You can include modules based on the hostname (`/etc/hostname`) of the machine.

You may only put this key in the root (`mrow.toml`).

```toml
[config]
host-includes = [
   { hostname = "vm", includes = "hosts/vm.toml" } # you can also put a list of includes here
]
```

## List of all step kinds

- **Install Package**

   ```toml
   [[module.steps]]
   kind = "install-package"
   package = "vim"
   aur = false # optional, defaults to false
   ```

- **Install Multiple Packages**

   ```toml
   [[module.steps]]
   kind = "install-packages"
   packages = ["vim", "htop"]
   aur = false # optional, defaults to false
   ```

- **Copy File**

   Paths can be absolute or relative to the parent of the module.

   ```toml
   [[module.steps]]
   kind = "copy-file"
   from = "/path/to/source"
   to = "/path/to/destination"
   as-root = true # optional, defaults to false
   ```

- **Create Symlink**

   Paths can be absolute or relative to the parent of the module.

   ```toml
   [[module.steps]]
   kind = "symlink"
   from = "/path/to/source"
   to = "/path/to/symlink"
   delete-existing = true # optional, defaults to false
   ```

- **Run Command**

   ```toml
   [[module.steps]]
   kind = "run-command"
   command = "echo meow"
   ```

- **Run Multiple Commands**

   ```toml
   [[module.steps]]
   kind = "run-commands"
   commands = ["echo meow", "echo bark"]
   ```

- **Run Script**

   Paths can be absolute or relative to the parent of the module.

   ```toml
   [[module.steps]]
   kind = "run-script"
   path = "/path/to/script.sh"
