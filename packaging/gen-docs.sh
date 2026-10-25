#!/bin/sh
# Writes the forager CLI's shell completions and man pages into a share/
# tree, laid out as they are installed:
#
#   bash-completion/completions/forager
#   zsh/site-functions/_forager
#   fish/vendor_completions.d/forager.fish
#   man/man1/forager.1, forager-<subcommand>.1
#
# Usage: packaging/gen-docs.sh <forager binary> <share dir>
set -eu

bin=$1
share=$2
rm -rf "$share"
mkdir -p "$share/bash-completion/completions" "$share/zsh/site-functions" \
  "$share/fish/vendor_completions.d" "$share/man/man1"
"$bin" completions bash > "$share/bash-completion/completions/forager"
"$bin" completions zsh > "$share/zsh/site-functions/_forager"
"$bin" completions fish > "$share/fish/vendor_completions.d/forager.fish"
"$bin" man "$share/man/man1"
