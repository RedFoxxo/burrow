#!/bin/sh
# Starts the Linux build for this CPU. A bundle's platform overrides tell Linux
# from macOS and Windows but not x64 from arm64; exec replaces this shell, so
# nothing stays running next to burrow.
dir=$(dirname "$0")
case $(uname -m) in
  aarch64 | arm64) exec "$dir/burrow-linux-arm64" "$@" ;;
  *) exec "$dir/burrow-linux-x64" "$@" ;;
esac
