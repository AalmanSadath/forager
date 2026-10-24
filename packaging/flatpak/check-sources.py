#!/usr/bin/env python3
"""Fails if the Flatpak's offline sources miss anything the lockfiles need.

A crate or npm tarball missing from cargo-sources.json or node-sources.json
only shows up as an offline-build failure deep inside flatpak-builder, so
this compares them up front. Run by regen-sources.sh and by CI.
"""
import json
import sys
import tomllib
from pathlib import Path

here = Path(__file__).resolve().parent
root = here.parent.parent
failed = False

cargo = (here / "cargo-sources.json").read_text()
lock = tomllib.loads((root / "Cargo.lock").read_text())
missing = sorted(
    f"{p['name']}-{p['version']}"
    for p in lock["package"]
    if p.get("source", "").startswith("registry+")
    and f"{p['name']}-{p['version']}" not in cargo
)
if missing:
    failed = True
    print(f"cargo-sources.json misses {len(missing)} crate(s), e.g. {', '.join(missing[:10])}", file=sys.stderr)

node = (here / "node-sources.json").read_text()
npm = json.loads((root / "gui" / "package-lock.json").read_text())
missing = sorted(
    f"{path.split('node_modules/')[-1]}@{pkg['version']}"
    for path, pkg in npm["packages"].items()
    if path.startswith("node_modules/")
    and pkg.get("resolved", "").endswith(".tgz")
    and pkg["resolved"] not in node
)
if missing:
    failed = True
    print(f"node-sources.json misses {len(missing)} package(s), e.g. {', '.join(missing[:10])}", file=sys.stderr)

if failed:
    print("Run packaging/flatpak/regen-sources.sh.", file=sys.stderr)
    sys.exit(1)
print("Flatpak sources cover Cargo.lock and gui/package-lock.json.")
