#!/bin/sh
# Prints the workspace version, the one every package is named after.
sed -n 's/^version = "\(.*\)"$/\1/p' "$(dirname "$0")/../Cargo.toml" | head -1
