# aws-profile-switcher

Switch `AWS_PROFILE` in your current shell with a fuzzy picker, without
typing `export AWS_PROFILE=…` and without typing `eval`.

![Screenshot 2023-02-17 at 13 14 36](https://user-images.githubusercontent.com/1544026/219651127-754a8f60-4f16-4fce-8a04-62f0c67592a6.png)

A process can't change its parent shell's environment, so the binary prints
shell code and a small shell function (installed with `init`) evaluates it.
The rule is strict: **stdout carries properly quoted shell code, and only
when the exit code is 0.** Prompts, notes and errors go to stderr.

Profiles are read directly from `~/.aws/config` and `~/.aws/credentials`
(section headers only; credential values are never read). The `aws` CLI is
no longer needed for listing; it's only used for `aws sso login`.

## Install

**Homebrew** (macOS arm64/x86_64, Linux x86_64):

```sh
brew install peterkracik/tap/aws-profile-switcher
```

**Release archive** (macOS arm64/x86_64, Linux x86_64 static musl), from the
[releases page](https://github.com/peterkracik/aws-profile-switcher/releases):

```sh
shasum -a 256 -c aws-profile-switcher-aarch64-apple-darwin.tar.gz.sha256
tar -xzf aws-profile-switcher-aarch64-apple-darwin.tar.gz
# macOS: the binaries are not signed, so remove the download quarantine
xattr -d com.apple.quarantine ./aws-profile-switcher
mv aws-profile-switcher ~/.local/bin/   # or anywhere you like
```

**From source** (Rust 1.88+):

```sh
cargo install --git https://github.com/peterkracik/aws-profile-switcher --locked
```

## Shell setup

Add one line to your shell's rc file. It defines an `awsprofile` function
that runs the binary and evaluates its output for you.

| Shell | rc file | Line |
|---|---|---|
| zsh | `~/.zshrc` | `source <(aws-profile-switcher init zsh)` |
| bash | `~/.bashrc` | `eval "$(aws-profile-switcher init bash)"` |
| fish | `~/.config/fish/config.fish` | `aws-profile-switcher init fish \| source` |

- bash: use the `eval "$(…)"` form. `source <(…)` doesn't work in macOS's
  bundled bash 3.2.
- `--name <fn>` picks a different function name, e.g.
  `source <(aws-profile-switcher init zsh --name awsp)`.
- `init` writes the binary's absolute path into the function. If a `PATH`
  entry resolves to the same binary (e.g. Homebrew's
  `/opt/homebrew/bin/aws-profile-switcher` symlink), that path is used, so
  `brew upgrade` doesn't break open shells' functions. If you move the
  binary, open a new shell (or re-run the `init` line).

## Commands

Through the shell function:

| Command | What it does |
|---|---|
| `awsprofile` | Open the picker (cursor starts on the current `$AWS_PROFILE`) |
| `awsprofile <profile>` | Switch directly. An exact name wins; a unique case-insensitive substring is auto-selected with a note; otherwise the picker opens pre-filtered |
| `awsprofile -- <name>` | Force `<name>` to be a profile, e.g. a profile literally called `unset` |
| `awsprofile unset` | Unset `AWS_PROFILE` |
| `awsprofile current` | Print the current `AWS_PROFILE` (exit 1 if not set) |
| `awsprofile ls` | List profiles |
| `awsprofile exec <profile> -- <cmd> [args…]` | Run one command with `AWS_PROFILE=<profile>`, without changing your shell |
| `aws-profile-switcher init <bash\|zsh\|fish>` | Print the shell function |

Global options:

| Option | Effect |
|---|---|
| `--login` | Always run `aws sso login --profile <p>` before switching, even if the token looks valid or the profile isn't SSO |
| `--no-sso` | Skip the SSO token check |
| `--clear-env` | Also unset `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` |
| `--shell <posix\|bash\|zsh\|fish>` | Output dialect when calling the binary directly (default: posix `export`) |

`exec` needs the exact profile name: it does no substring matching and never opens the picker.

`AWS_CONFIG_FILE` and `AWS_SHARED_CREDENTIALS_FILE` are respected.

## Exit codes

| Exit code | Meaning | stdout |
|---|---|---|
| 0 | Switched, unset, listed, printed, or help/version | shell code or text |
| 1 | Cancelled (Esc / Ctrl-C), or `current` with nothing set | empty |
| 2 | Any error: usage, no profiles, unknown profile, no terminal for the picker, unreadable file, `aws sso login` failed | empty |
| 126 / 127 | `exec`: command can't be run / not found | empty |
| child's code | `exec`: the command's own exit code | the command's |

The picker needs an interactive terminal on stderr. Without one (for example
in a script) it fails immediately with exit 2; pass a profile name instead.

## SSO

For profiles with `sso_session` (or the legacy `sso_start_url`), the tool
checks the cached token in `~/.aws/sso/cache` before switching:

- **Valid:** switch.
- **Expired or missing, in a terminal:** asks whether to run
  `aws sso login --profile <p>`. The CLI may refresh the session on its own,
  so answering **n** still switches (with a warning). Esc cancels (exit 1).
- **Expired or missing, no terminal:** switches with a warning.
- `--login` always logs in; `--no-sso` never checks.

`aws sso login` output (URL and code) is shown on stderr, so it never ends up
in your shell. If you press Ctrl-C during the login, the shell reports 130
and nothing is changed.

Profiles that reach SSO through `source_profile` aren't detected; use
`--login` for those.

## Environment credentials warning

If `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` or `AWS_SESSION_TOKEN` are
set, they take precedence over `AWS_PROFILE` in the AWS CLI and SDKs. The tool
warns about this on stderr; pass `--clear-env` to unset them as part of the
switch.

## Migrating from the old `` eval `…` `` wrapper

The old setup still works, because running the binary with no arguments
still prints `export AWS_PROFILE='…'`:

```bash
# old ~/.zshrc
awsprofile() {
  eval `/PATH/TO/aws-profile-switcher`
}
```

Replace it with the `init` line from [Shell setup](#shell-setup). The new
function adds the subcommands, per-shell quoting and correct exit codes.

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

`tests/shell.rs` runs the shell functions in real bash, zsh and fish. A
missing shell is skipped unless it's listed in `APS_REQUIRE_SHELLS`, e.g.
`APS_REQUIRE_SHELLS=bash,zsh cargo test --locked` (CI requires all three on
Linux). `APS_BASH`, `APS_ZSH` and `APS_FISH` point the tests at a specific
binary, e.g. `APS_BASH=/bin/bash` for macOS bash 3.2.

## Releasing

1. Bump `version` in `Cargo.toml` and push a tag `v<version>`, e.g.
   `git tag v1.0.0 && git push origin v1.0.0`.
2. `.github/workflows/release.yml` tests, builds
   `aws-profile-switcher-<target>.tar.gz` (+ `.sha256`) for
   `aarch64-apple-darwin`, `x86_64-apple-darwin` and
   `x86_64-unknown-linux-musl`, and publishes a GitHub release.
3. Its `homebrew` job renders `Formula/aws-profile-switcher.rb` from
   `packaging/homebrew/aws-profile-switcher.rb.tmpl` (via
   `scripts/render-formula.sh`) and pushes it to
   [peterkracik/homebrew-tap](https://github.com/peterkracik/homebrew-tap).

The `homebrew` job needs the repository secret `HOMEBREW_TAP_TOKEN`: a
fine-grained personal access token with **Contents: Read and write** on
`peterkracik/homebrew-tap` only. Without it the job fails with a message
naming the secret.

**Dry run:** a pre-release tag such as `v1.0.0-rc1` (Cargo version still
`1.0.0`) runs the whole pipeline and publishes a GitHub *prerelease*, but
skips the tap update. Delete the prerelease and its tag afterwards.

## License

MIT, see [LICENSE](LICENSE).
