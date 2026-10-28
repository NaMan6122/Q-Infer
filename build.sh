#!/usr/bin/env bash
#
# Build Q-Infer and link `qinfer` onto your PATH.
#
# The binary is one surface with two modes: `qinfer` with no arguments opens the
# dashboard, `qinfer <command>` is the CLI (`detect`, `plan`, `spec-lint`, ...).
#
# The link is a symlink into target/<profile>/, not a copy, so the next
# `cargo build` is live everywhere without reinstalling. The cost of that is
# `cargo clean` — it leaves a dangling link, and you rerun this script.
#
# Usage: ./build.sh [options]
#   --install-dir <path>   where the link goes (default: ~/.local/bin)
#   --copy                 copy the binary instead of symlinking it
#   --debug                build the debug profile (default: release)
#   --no-install           build only; leave PATH alone
#   --add-to-path          append the install dir to your shell rc if missing
#   -h, --help             this text

set -euo pipefail

MUTED=$'\033[0;2m'
GREEN=$'\033[0;32m'
YELLOW=$'\033[0;33m'
RED=$'\033[0;31m'
NC=$'\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR"
INSTALL_DIR="$HOME/.local/bin"
PROFILE="release"
MODE="symlink"
INSTALL=true
ADD_TO_PATH=false
CARGO="${CARGO:-cargo}"

# Extend this list, not the loop below, when the workspace grows a second bin.
BINARIES=(qinfer)

usage() {
  sed -n '/^# Usage:/,/^#   -h, --help/p' "$0" | sed 's/^# \{0,1\}//'
}

die() { echo "${RED}error:${NC} $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install-dir)
      [[ $# -ge 2 ]] || die "--install-dir needs a value"
      INSTALL_DIR="$2"; shift 2 ;;
    --install-dir=*)
      INSTALL_DIR="${1#*=}"; shift ;;
    --copy)        MODE="copy"; shift ;;
    --debug)       PROFILE="debug"; shift ;;
    --no-install)  INSTALL=false; shift ;;
    --add-to-path) ADD_TO_PATH=true; shift ;;
    -h|--help)     usage; exit 0 ;;
    *)             die "unknown option: $1 (try --help)" ;;
  esac
done

[[ -n "$INSTALL_DIR" ]] || die "--install-dir was empty"

# ── Build ──────────────────────────────────────────────────────────────────
echo "${MUTED}Building qinfer (${PROFILE})…${NC}"
if [[ "$PROFILE" == "release" ]]; then
  "$CARGO" build --release --manifest-path "$ROOT/Cargo.toml" -p qinfer-tui
else
  "$CARGO" build --manifest-path "$ROOT/Cargo.toml" -p qinfer-tui
fi

# A green cargo exit does not guarantee the artefact: check before we point
# anything at it, so a stale binary is never silently the one on PATH.
for bin in "${BINARIES[@]}"; do
  [[ -x "$ROOT/target/$PROFILE/$bin" ]] || die "build reported success but target/$PROFILE/$bin is missing"
done

if [[ "$INSTALL" == false ]]; then
  echo "${MUTED}--no-install: built target/$PROFILE/* only${NC}"
  exit 0
fi

# ── Install ────────────────────────────────────────────────────────────────
mkdir -p "$INSTALL_DIR" || die "cannot create $INSTALL_DIR"
[[ -w "$INSTALL_DIR" ]] || die "$INSTALL_DIR is not writable (pick another with --install-dir)"

for bin in "${BINARIES[@]}"; do
  SRC="$ROOT/target/$PROFILE/$bin"
  DEST="$INSTALL_DIR/$bin"
  TMP="$DEST.tmp.$$"

  if [[ "$MODE" == "copy" ]]; then
    cp -f "$SRC" "$TMP"
    chmod 755 "$TMP"
    # macOS: cp invalidates the ad-hoc signature rustc applied, and the kernel
    # SIGKILLs the result ("Killed: 9"). Re-sign in place. A symlink needs no
    # such treatment — it leaves the original untouched.
    if [[ "$(uname -s)" == "Darwin" ]]; then
      codesign --force --sign - "$TMP" >/dev/null 2>&1 || true
    fi
  else
    ln -sfn "$SRC" "$TMP"
  fi

  # Rename over the old entry: a shell resolving `qinfer` mid-install sees the
  # old target or the new one, never a half-written file.
  mv -f "$TMP" "$DEST"

  target="$(readlink "$DEST" 2>/dev/null || true)"
  [[ -n "$target" ]] || target="$DEST"
  echo "${GREEN}✓${NC} $bin → ${MUTED}$target${NC}"
  # Prove the result runs, from the installed path rather than target/.
  echo "  $("$DEST" version 2>/dev/null | head -n 1 || echo 'version check failed')"
done

# ── PATH ───────────────────────────────────────────────────────────────────
echo
case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    echo "${MUTED}$INSTALL_DIR is already on PATH${NC}" ;;
  *)
    echo "${YELLOW}$INSTALL_DIR is not on PATH${NC} — add it for this shell:"
    echo "    export PATH=\"$INSTALL_DIR:\$PATH\""

    if [[ "$ADD_TO_PATH" == true ]]; then
      case "$(basename "${SHELL:-}")" in
        zsh)  rc="${ZDOTDIR:-$HOME}/.zshrc" ;;
        bash) rc="$HOME/.bashrc" ;;
        fish) rc="$HOME/.config/fish/config.fish" ;;
        *)    rc="" ;;
      esac
      if [[ -z "$rc" ]]; then
        echo "${YELLOW}  unknown shell (${SHELL:-unset}) — add the line above by hand${NC}"
      elif grep -qsF "$INSTALL_DIR" "$rc"; then
        echo "${MUTED}  $rc already mentions $INSTALL_DIR${NC}"
      else
        case "$rc" in
          *.fish) line="fish_add_path \"$INSTALL_DIR\"" ;;
          *)      line="export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
        esac
        printf '\n# qinfer\n%s\n' "$line" >> "$rc"
        echo "${GREEN}✓${NC} appended to ${MUTED}$rc${NC} (open a new shell)"
      fi
    else
      echo "${MUTED}  or pass --add-to-path to append that to your shell rc${NC}"
    fi ;;
esac

echo
echo "${GREEN}Done.${NC} ${MUTED}qinfer${NC} for the dashboard, ${MUTED}qinfer help${NC} for the CLI."
echo "${MUTED}The dashboard reads spikes/m0/ found by walking up from the current"
echo "directory, so run it inside this repo — elsewhere every tab is empty.${NC}"
