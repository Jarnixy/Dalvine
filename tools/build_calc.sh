#!/bin/sh
# Builds tests/fixtures/calc.dex from tests/fixtures/calc/Calc.java (needs javac + Android SDK d8).
set -e
cd "$(dirname "$0")/../tests/fixtures"
D8=${D8:-$(ls -d "$HOME"/Android/Sdk/build-tools/*/d8 | sort | tail -1)}
tmp=$(mktemp -d)
javac --release 8 -d "$tmp" calc/Calc.java
"$D8" --min-api 21 --output "$tmp" "$tmp"/*.class
mv "$tmp/classes.dex" calc.dex
rm -rf "$tmp"
