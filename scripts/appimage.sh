#!/usr/bin/env bash
# Builds ReposExplorer-x86_64.AppImage: the one-file Linux option, made
# executable and run, with no install step at all.
#
# It carries the graphical application, the terminal application, the
# service, the desktop entry and the themed icon, in the layout an AppImage
# uses: AppRun, the entry and the icon at the root, everything else under
# usr/. Inside, the three binaries sit beside each other exactly as an
# install puts them, so the graphical application finds `service` next to
# itself and starts it the way it always does.
#
# An AppImage cannot update itself in place - it is one file, mounted read
# only, and the running process is inside it - so `--self-update` says so
# rather than failing obscurely. See `updater::appimage_advice`.
#
# Usage:
#   appimage.sh --from-directory <dir> [--output <file>]
#               [--appimagetool <file>]
#
#   --from-directory <dir>   where the built release files are, named as
#                            release.yml packages them:
#                            <binary>-x86_64-unknown-linux-gnu
#   --output <file>          where to write the AppImage (default
#                            ./ReposExplorer-x86_64.AppImage)
#   --appimagetool <file>    the tool to build with; downloaded from its own
#                            pinned release when this is not given

set -euo pipefail

APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
TARGET="x86_64-unknown-linux-gnu"
BINARIES="RepoSphereExplorerGui RepoSphereExplorerTui service"
ICON_NAME="reposphereexplorer"

fail() {
    echo "appimage.sh: $*" >&2
    exit 1
}

from_directory=""
output="$PWD/ReposExplorer-x86_64.AppImage"
appimagetool=""

while [ $# -gt 0 ]; do
    case "$1" in
        --from-directory) from_directory="${2:?--from-directory needs a value}"; shift 2 ;;
        --output) output="${2:?--output needs a value}"; shift 2 ;;
        --appimagetool) appimagetool="${2:?--appimagetool needs a value}"; shift 2 ;;
        -h | --help) sed -n '2,27p' "$0"; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done

[ -n "$from_directory" ] || fail "--from-directory is required"
[ -d "$from_directory" ] || fail "no directory at $from_directory"
command -v cargo >/dev/null 2>&1 || fail "cargo is needed: the icon is drawn by \`cargo run -p icon\`"

repository="$(cd "$(dirname "$0")/.." && pwd)"
staging="$(mktemp -d "${TMPDIR:-/tmp}/rse-appimage.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
appdir="$staging/ReposExplorer.AppDir"

mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications"
for binary in $BINARIES; do
    file="$from_directory/$binary-$TARGET"
    [ -f "$file" ] || fail "no $binary-$TARGET in $from_directory"
    cp "$file" "$appdir/usr/bin/$binary"
    chmod +x "$appdir/usr/bin/$binary"
done

# The same drawing the install places in a user's icon theme, rendered by
# the same command, so the AppImage and the install show one picture.
icons="$appdir/usr/share/icons/hicolor"
(cd "$repository" && cargo run --quiet -p icon -- --icons "$icons")
# What an AppImage puts at its root: the icon the desktop entry names, and
# the thumbnail a file manager draws for the file itself.
cp "$icons/256x256/apps/$ICON_NAME.png" "$appdir/$ICON_NAME.png"
cp "$icons/256x256/apps/$ICON_NAME.png" "$appdir/.DirIcon"

# The desktop entry. Exec names the binary rather than a path: the runtime
# mounts the AppImage somewhere different every time, and AppRun puts its
# own usr/bin first. Every other key matches the one the install writes -
# `crates/gui/tests/desktop_entry.rs` holds the two scripts to each other.
cat > "$appdir/$ICON_NAME.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Repos Explorer
Comment=A front door to the working copies source control checks out on this machine
Exec=RepoSphereExplorerGui %f
Icon=$ICON_NAME
Terminal=false
Categories=Development;Utility;FileTools;
StartupWMClass=$ICON_NAME
DESKTOP
cp "$appdir/$ICON_NAME.desktop" "$appdir/usr/share/applications/$ICON_NAME.desktop"

# What the runtime runs. `exec` so the graphical application is the process
# the desktop sees, and so closing the window ends the AppImage.
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
export PATH="$here/usr/bin:${PATH:-}"
exec "$here/usr/bin/RepoSphereExplorerGui" "$@"
APPRUN
chmod +x "$appdir/AppRun"

if [ -z "$appimagetool" ]; then
    appimagetool="$staging/appimagetool"
    curl --proto '=https' --tlsv1.2 -fsSL "$APPIMAGETOOL_URL" -o "$appimagetool" \
        || fail "could not download appimagetool from $APPIMAGETOOL_URL"
    chmod +x "$appimagetool"
fi

mkdir -p "$(dirname "$output")"
# appimagetool is itself an AppImage, and a build machine rarely has the
# filesystem-in-userspace (FUSE) library one mounts itself with. Unpacking
# it and running it from there needs neither.
ARCH=x86_64 "$appimagetool" --appimage-extract-and-run "$appdir" "$output" \
    || fail "appimagetool could not build $output"
chmod +x "$output"
echo "built $output ($(du -h "$output" | cut -f1))"
