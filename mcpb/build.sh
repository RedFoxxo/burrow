#!/bin/sh
# Builds burrow.mcpb, the MCP Bundle Claude Desktop installs with one click and
# the MCP Registry lists, from a release's five binaries.
#
#   mcpb/build.sh <version> <folder with the release files> <output .mcpb>
#
# One bundle serves every system: a universal macOS binary, the Windows
# binary, and both Linux builds behind burrow-linux.sh.
set -eu

version=$1
assets=$2
out=$3
root=$(cd "$(dirname "$0")/.." && pwd)

if command -v lipo > /dev/null 2>&1; then
  lipo=lipo
elif command -v llvm-lipo > /dev/null 2>&1; then
  lipo=llvm-lipo
else
  echo "mcpb/build.sh: needs lipo (macOS) or llvm-lipo for the universal macOS binary" >&2
  exit 1
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/server"
"$lipo" -create "$assets/burrow-macos-arm64" "$assets/burrow-macos-x64" -output "$stage/server/burrow"
cp "$assets/burrow-windows-x64.exe" "$stage/server/burrow.exe"
cp "$assets/burrow-linux-x64" "$assets/burrow-linux-arm64" "$root/mcpb/burrow-linux.sh" "$stage/server/"
chmod 755 "$stage/server/burrow" "$stage/server/burrow-linux-x64" "$stage/server/burrow-linux-arm64" "$stage/server/burrow-linux.sh"
cp "$root/LICENSE" "$stage/"

# The version, and the tools as the server lists them (tests/golden/ pins that list).
jq --arg v "$version" --slurpfile golden "$root/tests/golden/tools.json" \
  '.version = $v | .tools = [$golden[0].tools[] | { name, description }]' \
  "$root/mcpb/manifest.json" > "$stage/manifest.json"

mcpb="npx -y @anthropic-ai/mcpb@2.1.2"
$mcpb validate "$stage/manifest.json"
$mcpb pack "$stage" "$out"
