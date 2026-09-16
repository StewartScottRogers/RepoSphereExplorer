#!/usr/bin/env bash
# Installs or uninstalls Repos Explorer for the current user on Linux and macOS.
#
# Install downloads one release's graphical application, terminal application
# and service for this machine, verifies every file against the release's
# signed update manifest before placing anything, and puts the three side by
# side in a per-user folder. No administrator rights are needed.
#
# On macOS the three go inside "Repos Explorer.app" in that folder, because a
# Mac expects an application and not three files: an icon in Finder, a name in
# Launchpad, and the Dock showing the application rather than a terminal. It
# is the same bundle the disk image carries, so a Mac has one layout however
# it was installed, and the executables are also linked beside it so the
# command line can still name them.
#
# Verification is the scheme the in-application updater uses: each file's
# Secure Hash Algorithm 256 (SHA-256) digest must match the manifest, and the
# manifest's Ed25519 signature over that digest must verify against the public
# key compiled into the release's `verify` program. That program is itself
# checked against its manifest digest before it is run, and verifies itself
# along with the rest.
#
# Uninstall stops application and service processes started from the install
# folder, removes what install placed, and says what it removed. --purge also
# removes the per-user data folder (the journal, the Repos Directory
# configuration and window settings), and refuses unless --yes is given or the
# CI environment variable is "true".
#
# Usage:
#   install.sh [--tag <tag> | --from-directory <dir> [--unsigned-test-manifest]]
#              [--prefix <dir>] [--bin-dir <dir>]
#   install.sh --uninstall [--purge [--yes]] [--prefix <dir>]
#
#   --tag <tag>                 release to install (default: latest)
#   --from-directory <dir>      install already-built release files and their
#                               manifest.json from <dir> instead of downloading
#   --unsigned-test-manifest    with --from-directory only: check digests but NOT
#                               signatures, for a hand-made test manifest
#   --prefix <dir>              where to install (Linux default
#                               ~/.local/share/RepoSphereExplorer, macOS
#                               ~/Applications/RepoSphereExplorer)
#   --bin-dir <dir>             Linux only: where the command links go
#                               (default ~/.local/bin)

set -euo pipefail

REPOSITORY="StewartScottRogers/RepoSphereExplorer"
LATEST_MANIFEST_URL="https://stewartscottrogers.github.io/RepoSphereExplorer/latest.json"
INSTALLED="RepoSphereExplorerGui RepoSphereExplorerTui service"
RECEIPT="installed-files.txt"

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

tag="latest"
tag_given=""
from_directory=""
unsigned=""
uninstall=""
purge=""
yes=""
prefix=""
bin_dir=""

while [ $# -gt 0 ]; do
    case "$1" in
        --tag) tag="${2:?--tag needs a value}"; tag_given=1; shift 2 ;;
        --from-directory) from_directory="${2:?--from-directory needs a value}"; shift 2 ;;
        --unsigned-test-manifest) unsigned=1; shift ;;
        --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
        --bin-dir) bin_dir="${2:?--bin-dir needs a value}"; shift 2 ;;
        --uninstall) uninstall=1; shift ;;
        --purge) purge=1; shift ;;
        --yes) yes=1; shift ;;
        -h | --help) sed -n '2,43p' "$0"; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done

# Empty everywhere but macOS, where it names the application bundle the three
# executables go inside.
bundle=""

case "$(uname -s)/$(uname -m)" in
    Linux/x86_64)
        target="x86_64-unknown-linux-gnu"
        data_directory="${XDG_DATA_HOME:-$HOME/.local/share}/RepoSphereExplorer"
        prefix="${prefix:-$data_directory}"
        bin_dir="${bin_dir:-$HOME/.local/bin}"
        ;;
    Darwin/arm64)
        target="aarch64-apple-darwin"
        data_directory="$HOME/Library/Application Support/RepoSphereExplorer"
        prefix="${prefix:-$HOME/Applications/RepoSphereExplorer}"
        bundle="Repos Explorer.app"
        [ -z "$bin_dir" ] || fail "--bin-dir is for Linux only"
        ;;
    *) fail "no release is built for $(uname -s) on $(uname -m)" ;;
esac

# The property list macOS reads to learn that a folder is an application:
# what it is called, which of the three executables to start, which icon to
# draw and what version it is.
#
# crates/macos-bundle writes this very same text for the disk image, and its
# the_install_script_writes_the_same_plist test fails the moment the two part
# company. It is duplicated rather than shared because this script is
# downloaded and run on its own, with no checkout and no cargo beside it.
write_info_plist() {
    local path="$1" version="$2"
    cat > "$path" <<INFO_PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.stewartscottrogers.RepoSphereExplorer</string>
    <key>CFBundleName</key>
    <string>Repos Explorer</string>
    <key>CFBundleDisplayName</key>
    <string>Repos Explorer</string>
    <key>CFBundleExecutable</key>
    <string>RepoSphereExplorerGui</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon.icns</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
INFO_PLIST
}

# Processes whose executable lives under the folder $1.
pids_in() {
    local folder="$1" pid args exe
    ps -A -ww -o pid= -o args= 2>/dev/null | while read -r pid args; do
        exe="$args"
        if [ -r "/proc/$pid/exe" ]; then
            exe="$(readlink "/proc/$pid/exe" 2>/dev/null || echo "$args")"
        fi
        case "$exe" in "$folder"/*) echo "$pid" ;; esac
    done
}

stop_processes_in() {
    local pid pids
    pids="$(pids_in "$1")"
    [ -n "$pids" ] || return 0
    for pid in $pids; do
        echo "stopped $(ps -o args= -p "$pid" 2>/dev/null || true) (process $pid)"
        kill "$pid" 2>/dev/null || true
    done
    for _ in $(seq 1 30); do
        [ -n "$(pids_in "$1")" ] || return 0
        sleep 0.5
    done
    for pid in $(pids_in "$1"); do kill -9 "$pid" 2>/dev/null || true; done
}

do_uninstall() {
    local receipt_path="$prefix/$RECEIPT" path
    [ -f "$receipt_path" ] || fail "nothing installed by this script at $prefix (no $RECEIPT there)"
    if [ -n "$purge" ] && [ -z "$yes" ] && [ "${CI:-}" != "true" ]; then
        fail "refusing --purge: it deletes $data_directory, which holds this machine's journal and Repos Directory configuration. Run again with --yes if that is what you want."
    fi

    stop_processes_in "$prefix"
    while IFS= read -r path; do
        if [ -n "$path" ] && { [ -e "$path" ] || [ -L "$path" ]; }; then
            rm -f "$path"
            echo "removed $path"
        fi
    done < "$receipt_path"
    rm -f "$receipt_path"
    if [ -n "$bundle" ]; then
        # The bundle's folders are the shape of the install rather than files
        # it placed, so the receipt does not list them. Remove them once what
        # they held has gone, and leave any that a reader has put something
        # else in.
        local folder
        for folder in "Contents/MacOS" "Contents/Resources" "Contents" ""; do
            folder="$prefix/$bundle${folder:+/$folder}"
            if rmdir "$folder" 2>/dev/null; then
                echo "removed $folder"
            fi
        done
    fi
    if rmdir "$prefix" 2>/dev/null; then
        echo "removed $prefix"
    else
        echo "left $prefix in place; it still holds: $(ls -A "$prefix" | tr '\n' ' ')"
    fi

    if [ -n "$purge" ]; then
        if [ -d "$data_directory" ]; then
            stop_processes_in "$data_directory"
            find "$data_directory" -mindepth 1 | sed 's/^/removed /'
            rm -rf "$data_directory"
            echo "removed $data_directory"
        else
            echo "no data folder at $data_directory"
        fi
    fi
}

# The value of "key" in one flattened manifest object, $1.
field() {
    printf '%s\n' "$1" | sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p"
}

download() {
    curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2" || fail "could not download $1"
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

do_install() {
    [ -z "$tag_given" ] || [ -z "$from_directory" ] || fail "--tag and --from-directory cannot be used together"
    [ -z "$unsigned" ] || [ -n "$from_directory" ] || fail "--unsigned-test-manifest is only for a local --from-directory test"
    [ ! -f "$prefix/$RECEIPT" ] || fail "Repos Explorer is already installed at $prefix; uninstall it first"

    staging="$(mktemp -d "${TMPDIR:-/tmp}/rse-install.XXXXXX")"
    trap 'rm -rf "$staging"' EXIT

    local manifest="$staging/manifest.json"
    if [ -n "$from_directory" ]; then
        [ -f "$from_directory/manifest.json" ] || fail "no manifest.json in $from_directory"
        cp "$from_directory/manifest.json" "$manifest"
    elif [ "$tag" = "latest" ]; then
        download "$LATEST_MANIFEST_URL" "$manifest"
    else
        download "https://github.com/$REPOSITORY/releases/download/$tag/manifest.json" "$manifest"
    fi

    # One manifest object per line. The manifest's values are names, triples,
    # addresses and hexadecimal, none of which holds a brace or a quote.
    local objects version
    objects="$(tr -d '\r\n' < "$manifest" | tr '{' '\n')"
    version="$(field "$(printf '%s\n' "$objects" | sed -n "1,2p" | tr -d '\n')" version | sed -n 1p)"
    echo "release $version for $target"

    local wanted="$INSTALLED"
    [ -n "$unsigned" ] || wanted="$INSTALLED verify"

    local binary object missing="" published="" files="" name url sha file
    for binary in $wanted; do
        object="$(printf '%s\n' "$objects" \
            | grep -E "\"binary\"[[:space:]]*:[[:space:]]*\"$binary\"" \
            | grep -E "\"target\"[[:space:]]*:[[:space:]]*\"$target\"" | sed -n 1p || true)"
        [ -n "$object" ] || missing="$missing $binary"
    done
    if [ -n "$missing" ]; then
        published="$(printf '%s\n' "$objects" | grep -E "\"target\"[[:space:]]*:[[:space:]]*\"$target\"" \
            | while IFS= read -r object; do field "$object" binary; done | tr '\n' ' ' || true)"
        fail "release $version publishes no${missing} for $target (it publishes: ${published})"
    fi

    # The bundle's icon, wanted rather than required: it is not an executable,
    # and a release cut before the bundle existed publishes none. An install
    # from one of those gets the application without a drawing on it, which is
    # a worse icon rather than a failed install.
    if [ -n "$bundle" ]; then
        object="$(printf '%s\n' "$objects" \
            | grep -E "\"binary\"[[:space:]]*:[[:space:]]*\"AppIcon\"" \
            | grep -E "\"target\"[[:space:]]*:[[:space:]]*\"$target\"" | sed -n 1p || true)"
        [ -z "$object" ] || wanted="$wanted AppIcon"
    fi

    for binary in $wanted; do
        object="$(printf '%s\n' "$objects" \
            | grep -E "\"binary\"[[:space:]]*:[[:space:]]*\"$binary\"" \
            | grep -E "\"target\"[[:space:]]*:[[:space:]]*\"$target\"" | sed -n 1p)"
        url="$(field "$object" url)"
        sha="$(field "$object" sha256 | tr 'A-F' 'a-f')"
        name="${url##*/}"
        file="$staging/$name"
        if [ -n "$from_directory" ]; then
            [ -f "$from_directory/$name" ] || fail "no $name in $from_directory"
            cp "$from_directory/$name" "$file"
        else
            download "$url" "$file"
        fi
        [ "$(sha256_of "$file")" = "$sha" ] \
            || fail "refusing $name: its digest does not match the manifest, so it is not the file that was released"
        eval "file_$binary=\"\$file\""
        files="$files $file"
    done

    if [ -n "$unsigned" ]; then
        echo "WARNING: SIGNATURES NOT CHECKED (--unsigned-test-manifest): digests only, against a manifest nobody signed." >&2
    else
        chmod +x "$file_verify"
        # shellcheck disable=SC2086 # $files is a list of paths without spaces
        "$file_verify" "$manifest" $files || fail "refusing to install: signature verification failed"
    fi

    mkdir -p "$prefix"
    local placed="" destination into="$prefix" inside
    if [ -n "$bundle" ]; then
        into="$prefix/$bundle/Contents/MacOS"
        mkdir -p "$into" "$prefix/$bundle/Contents/Resources"
        inside="$prefix/$bundle/Contents/Info.plist"
        write_info_plist "$inside" "$version"
        placed="$placed$inside"$'\n'
        echo "placed $inside"
        if [ -n "${file_AppIcon:-}" ]; then
            inside="$prefix/$bundle/Contents/Resources/AppIcon.icns"
            cp "$file_AppIcon" "$inside"
            placed="$placed$inside"$'\n'
            echo "placed $inside"
        else
            echo "release $version publishes no icon for the bundle, so macOS will draw the generic one"
        fi
    fi
    for binary in $INSTALLED; do
        destination="$into/$binary"
        eval "cp \"\$file_$binary\" \"\$destination\""
        chmod +x "$destination"
        placed="$placed$destination"$'\n'
        echo "placed $destination"
    done
    if [ -n "$bundle" ]; then
        # Links beside the bundle, so the command line can still name the
        # three the way it could before they moved inside - the same service
        # the Linux install's links do.
        for binary in $INSTALLED; do
            ln -sfn "$bundle/Contents/MacOS/$binary" "$prefix/$binary"
            placed="$placed$prefix/$binary"$'\n'
            echo "linked $prefix/$binary"
        done
    fi
    if [ "$target" = "x86_64-unknown-linux-gnu" ]; then
        mkdir -p "$bin_dir"
        for binary in RepoSphereExplorerGui RepoSphereExplorerTui; do
            ln -sfn "$prefix/$binary" "$bin_dir/$binary"
            placed="$placed$bin_dir/$binary"$'\n'
            echo "linked $bin_dir/$binary"
        done
    fi
    printf '%s' "$placed" > "$prefix/$RECEIPT"
    echo "installed Repos Explorer $version in $prefix"
}

if [ -n "$purge" ] && [ -z "$uninstall" ]; then
    fail "--purge only goes with --uninstall"
fi
if [ -n "$uninstall" ]; then do_uninstall; else do_install; fi
