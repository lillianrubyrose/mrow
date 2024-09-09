# mrow

mrow is a tool for automating tasks on Arch Linux systems after OS installation.

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
   ./target/release/mrow --dir <path>
   ```

### Github Release
[Latest Download](https://github.com/lillianrubyrose/mrow/releases/latest)

## CLI Usage
```sh
mrow --dir <path>
```

Arguments:
- `--dir <path>` (Optional): Directory where your `mrow.{toml,luau}` resides. Defaults to current working directory.
- `--debug` (Optional): Doesn't execute any commands, just logs them and what they would do.
- `--single-module <path>` (Optional): Executes only this module and no other steps.

## Getting Started

You can view the [LuaU usage here](./README-LUA.md) or the [TOML usage here](./README-TOML.md).

You can also view [my personal mrowfiles](https://github.com/lillianrubyrose/mrowfiles) as an example.

## Contribution

Feel free to fork this repository and create pull requests. If you find any issues or have feature requests, please [open an issue](https://github.com/lillianrubyrose/mrow/issues/new).

## License

This project is dual licensed under both the [MIT License](./LICENSE-MIT) and [Apache License 2.0](./LICENSE-APACHE).

---

Feel free to [open an issue](https://github.com/lillianrubyrose/mrow/issues/new) at if you encounter any problems or have suggestions.
