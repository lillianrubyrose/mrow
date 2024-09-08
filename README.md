# Mrow

Mrow is a tool designed specifically for Arch Linux to automate various system configuration and software installation steps. This tool reads from a `mrow.toml` file and executes a series of steps defined within the file.

## Prerequisites

- Rust (for building from source)

## Installation

### Building from Source

1. **Clone the Repository**
   ```sh
   git clone https://github.com/lillianrubyrose/mrow.git
   cd mrow
   ```

2. **Build the Project**
   ```sh
   cargo build --release
   ```

3. **Run the Binary**
   ```sh
   ./target/release/mrow --dir <directory_with_toml>
   ```

## Usage

1. **Create a `mrow.toml` File**

   Here's an example `mrow.toml`:

   ```toml
   [config]
   aur_helper = "yay"  # or "paru"

   [module]
   includes = ["include1.toml", "include2.toml"]

   [[module.steps]]
   kind = "install-package"
   package = "vim"
   aur = false

   [[module.steps]]
   kind = "run-command"
   command = "echo Hello, World!"
   ```

2. **Run Mrow**

   ```sh
   mrow --dir <directory_with_mrow_toml>
   ```

   Arguments:
   - `--dir` (Optional): Directory where your `mrow.toml` resides. Defaults to current working directory.
   - `--debug` (Optional): Doesn't execute any commands, just logs them and what they would do.

## Contribution

Feel free to fork this repository and create pull requests. If you find any issues or have feature requests, please [open an issue](https://github.com/lillianrubyrose/mrow/issues/new).

### Examples

[My personal mrowfiles for my system](https://github.com/lillianrubyrose/mrowfiles)

#### Step Types

1. **Install Package**

   ```toml
   [[module.steps]]
   kind = "install-package"
   package = "vim"
   aur = false # optional, defaults to false
   ```

2. **Install Multiple Packages**

   ```toml
   [[module.steps]]
   kind = "install-packages"
   packages = ["vim", "htop"]
   aur = false # optional, defaults to false
   ```

3. **Copy File**

   ```toml
   [[module.steps]]
   kind = "copy-file"
   from = "/path/to/source"
   to = "/path/to/destination"
   as-root = true # optional, defaults to false
   ```

4. **Create Symlink**

   ```toml
   [[module.steps]]
   kind = "symlink"
   from = "/path/to/source"
   to = "/path/to/symlink"
   delete-existing = true # optional, defaults to false
   ```

5. **Run Command**

   ```toml
   [[module.steps]]
   kind = "run-command"
   command = "echo Hello, World!"
   ```

6. **Run Script**

   ```toml
   [[module.steps]]
   kind = "run-script"
   path = "/path/to/script.sh"
   ```

## License

This project is dual licensed under both the [MIT License](./LICENSE-MIT) and [Apache License 2.0](./LICENSE-APACHE).

---

Feel free to [open an issue](https://github.com/lillianrubyrose/mrow/issues/new) at if you encounter any problems or have suggestions.
