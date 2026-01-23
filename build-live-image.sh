#!/bin/bash
# Wrapper script - delegates to loader/build-live-image.sh
cd "$(dirname "$0")/loader" && ./build-live-image.sh "$@"
