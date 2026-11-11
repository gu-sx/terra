# 0003. Command line and library location

## Status

Accepted

## Context

Brief §10 sketches a command line (`music tui --local`, `music tui --server …`, `music serve`, `music import`) and says that `music` alone should eventually launch the normal TUI. Brief §13.9 leaves configuration open: profile selection, the default library, file locations and launch commands. Phase 1 (§11) needs only Mode A (§3): a local library, scanned from folders on this machine, browsed in the TUI.

`terra-storage` gives the local library an explicit lifecycle: `LocalLibrary::create` makes a library in a directory, `migrate` applies the migrations an older one lacks, and `open` refuses a library that needs a migration rather than changing it. `add_root`, `list_roots`, `remove_root` and `scan` manage and read the folders. Until now the binary had one command, `terra tui --demo`, on the synthetic sample library. Work package #11 proposed the commands below; this record writes them down with the points the implementation settled.

The library has to live somewhere when the user names no directory. The candidates for finding the XDG data directory were `dirs` 7.0.0, `directories` 6.0.0 and `etcetera` 0.11.0. `dirs` and `directories` follow each platform's own convention, which on macOS is `~/Library/Application Support`. `etcetera` offers that too, and also `choose_base_strategy`, which uses the XDG Base Directory specification on every Unix, macOS included, and the known folders on Windows. Its `Xdg::data_dir` reads `XDG_DATA_HOME`, ignores the value unless it is an absolute path, as the specification asks, and falls back to `~/.local/share`. `etcetera` 0.11.0 is already in the lockfile through SQLx, so it adds no new crate.

## Decision

**Commands.** The binary has these commands:

- `terra init` creates the library, or applies the migrations it lacks. Running it again changes nothing. No other command creates or migrates a library.
- `terra root add <DIR>`, `terra root list` and `terra root remove <DIR>` manage the folders the library scans. `root add` prints the path it stored, which is absolute with symbolic links resolved. `root remove` of a folder that is not a root fails.
- `terra scan` reads every root into the library. It prints its progress on standard error, a count rewritten in place on a terminal and otherwise one line per file with its path, and then a one-line summary on standard output. A file that cannot be read is listed on standard error and the exit status stays 0; only a failure of the library itself, such as of the database, exits non-zero.
- `terra tui`, and `terra` without a command, open the local library in the TUI. `terra tui --demo` opens the synthetic sample library instead.

Every command except `init` and `tui --demo` opens an existing, up-to-date library. When there is no library, or it needs a migration, the command exits with status 1 and a message that says to run `terra init`, and leaves the directory as it was. Wrong arguments exit with status 2, as clap does.

**Library location.** The library directory is, in this order:

1. the `--library DIR` option, which every command accepts;
2. the `TERRA_LIBRARY` environment variable, unless it is empty;
3. `terra` in the XDG data directory: `$XDG_DATA_HOME/terra` when `XDG_DATA_HOME` is an absolute path, otherwise `~/.local/share/terra`, as `etcetera` 0.11.0's `choose_base_strategy` gives it. On Windows that is the roaming application data folder.

An empty `TERRA_LIBRARY` counts as unset, the usual reading of an empty variable, so the variable is read by terra and not through clap's `env` attribute, which would reject it.

**No configuration file.** Phase 1 reads no configuration file; the option and the variable are all there is. `--local` and `--server` arrive with the remote backend in Phase 3.

**Backend.** `crates/terra/src/backend.rs` defines `enum Backend { Local(LocalLibrary) }`, which implements `LibraryBackend` by delegating to the variant. The TUI takes it, and Phase 3 adds a remote variant beside the local one. The TUI runs on the runtime the command line builds, a current-thread Tokio runtime with its drivers enabled, so that the library's connection pool lives on one runtime from opening to closing.

## Consequences

- A user starts with `terra init`, `terra root add ~/Music` and `terra scan`, then runs `terra`. Tests and scripts point every command at a directory of their own with `TERRA_LIBRARY` or `--library`, and never touch the user's library.
- A newer terra never migrates a library behind the user's back: opening one that needs a migration fails until `terra init` runs. An older terra refuses a library a newer one made.
- The same library is reachable from every command without a configuration file, so Phase 1 settles nothing about one. A later configuration file slots in between the environment variable and the XDG default, or replaces the default; it needs its own record.
- Still open under §13.9: profiles, selecting a remote server, a last-used backend, and the launch commands of Phase 3 and later (`serve`, `import`).
