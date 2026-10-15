#!/usr/bin/env bash
# Fail when a Windows executable imports the C or C++ runtime DLLs. The agent must run on
# a host without the Visual C++ Redistributable (AC-NFR-05).
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: check-windows-imports.sh <exe>" >&2
  exit 2
fi

imports="$(objdump -p "$1" | sed -n 's/^[[:space:]]*DLL Name: //p' | sort -u)"
[ -n "$imports" ] || { echo "error: no import table found in $1" >&2; exit 1; }
echo "$imports"

if echo "$imports" | grep -Eiq '^(vcruntime|msvcp|msvcr|concrt|ucrtbase|api-ms-win-crt-)'; then
  echo "error: $1 imports a C or C++ runtime DLL; build with +crt-static" >&2
  exit 1
fi
