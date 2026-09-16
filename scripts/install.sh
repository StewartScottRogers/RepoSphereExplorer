#!/usr/bin/env bash
# Installs or uninstalls Repos Explorer for the current user on Linux and macOS.
#
# Install downloads one release's graphical application, terminal application
# and service for this machine, verifies every file against the release's
# signed update manifest before placing anything, and puts the three side by
# side in a per-user folder. No administrator rights are needed.
#
<<<<<<< HEAD
# On macOS the three go inside "Repos Explorer.app" in that folder, because a
# Mac expects an application and not three files: an icon in Finder, a name in
# Launchpad, and the Dock showing the application rather than a terminal. It
# is the same bundle the disk image carries, so a Mac has one layout however
# it was installed, and the executables are also linked beside it so the
# command line can still name them.
=======
# On Linux it also does what a free desktop expects of an installed
# application: a desktop entry in ~/.local/share/applications, so it appears
# in the applications menu, and the themed icon in ~/.local/share/icons, so
# the menu and the window have a picture. The icon travels inside this
# script, because this script travels on its own.
>>>>>>> 229a912 (feat(#562): Linux gets a desktop entry, themed icons and a one-file AppImage)
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
#              [--prefix <dir>] [--bin-dir <dir>] [--xdg-data-dir <dir>]
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
#   --xdg-data-dir <dir>        Linux only: where the desktop entry and the
#                               themed icons go (default ~/.local/share, or
#                               XDG_DATA_HOME). A second copy installed beside
#                               a real one points this somewhere harmless, so
#                               that removing it does not take the real
#                               install's menu entry away.

set -euo pipefail

REPOSITORY="StewartScottRogers/RepoSphereExplorer"
LATEST_MANIFEST_URL="https://stewartscottrogers.github.io/RepoSphereExplorer/latest.json"
INSTALLED="RepoSphereExplorerGui RepoSphereExplorerTui service"
RECEIPT="installed-files.txt"

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

# BEGIN GENERATED ICONS
# Written by `cargo run -p icon` from assets/RepoSphereExplorer.svg. Change
# the drawing and run that command; changing this by hand makes the script
# and the drawing two different pictures.
ICON_NAME="reposphereexplorer"
ICON_SIZES="16 32 48 64 128 256"

# The drawing, for the theme's scalable directory.
icon_svg() {
    cat <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
  <title>Repos Explorer</title>
  <desc>A folder of working copies: the shape the application draws for a
  directory, holding a listing whose first row is a checkout, badged with the
  accent-coloured marker the Contents pane puts beside one.</desc>
  <defs>
    <linearGradient id="accent" gradientUnits="userSpaceOnUse" x1="0" y1="8" x2="0" y2="248">
      <stop offset="0" stop-color="#2589e0"/>
      <stop offset="1" stop-color="#004a86"/>
    </linearGradient>
  </defs>
  <rect x="8" y="8" width="240" height="240" rx="54" fill="url(#accent)"/>
  <path d="M38 100a12 12 0 0 1 12-12h40l16 16h68a12 12 0 0 1 12 12v86a12 12 0 0 1-12 12H50a12 12 0 0 1-12-12z" fill="#b6d6f2"/>
  <path d="M38 118h148v84a12 12 0 0 1-12 12H50a12 12 0 0 1-12-12z" fill="#ffffff"/>
  <rect x="58" y="142" width="96" height="13" rx="6.5" fill="#0f7ad2"/>
  <rect x="58" y="173" width="66" height="13" rx="6.5" fill="#b9c9d8"/>
  <circle cx="194" cy="70" r="54" fill="url(#accent)"/>
  <circle cx="194" cy="70" r="35" fill="#ffffff"/>
</svg>
SVG
}

# The drawing rendered at one size, as the bytes of a portable network
# graphic (PNG) on standard output.
icon_png() {
    case "$1" in
    16)
        base64 -d <<'PNG_16'
iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAB90lEQVR4nJWTP0xTURTGv9u+ttQY
2gLP1qGlmFYFB3RwYdDRgYEuOhpcTNTBGCcSHQ0JiSEuxBgTjYuLmudI4sJg0jgYBgjQhFBKSQjQ
tB1oy/tzD/feQnmFUMKXvPfun3d+5zsn92o41M3ptSRZnjEihNFBjKHCfPz38quBvJrLV/pdbpwD
X3ABeYm9yL1Jz7Dk5FISprN2XsBQrAsv7+no7vLiw9w2sus1wO8d0FBrZLjwfZ4+PkwhHgmo8WA0
jtGZFRSr9beabTXC1Kyko46CpUJBDVe7GdZ3zWsat01IQKIniNv9IfVDzXTwZ3GnDTC7WMKDW71q
XCw3sLBRBdnWimYLgOzlcFzH2J1YK+BG9BIKpZoay+/jz/N4ej+usn+a20C1bosd2lIOJOC6HkCq
z98CpPqibQ7+58v4ni3gb67kWiU0SyABEBnTegBnKa3HMJIKY3hitrXGmAtAxFEom3j+s3gq+NlI
L0aHQjIhmo5dANUDBSAkIn58e3Tch2DApx63ZccFwJEDCYBwUN+3sLq529q/ErmM/lhPmxt+EkCO
XeGclAOZ7e5gAmdpb99uc+DxsIomaje4ZU1P/cgKPEcnTf36B265euD3GeoIejPvx4nThS4T87An
jvH66/EZzkwmYVFG1BLuHMkq8DEDxkReTg8A7orR3TutFW8AAAAASUVORK5CYII=
PNG_16
        ;;
    32)
        base64 -d <<'PNG_32'
iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAADRElEQVR4nMWXX0hTURzHv3f+mc7U
/JP/0m2a5cx8iCDrpQIroychiN4kH4IeKqUIhbSCwAgie4gIe4woCFqBofSQ0EtkEqnV8O+0KWKz
ppXu7zmdc9O5u7u7rXmtD5zdc8859/y+O78/29XgPxMfPGBqGzf6BFxm3QMUMEIFBMDKLj1xFFct
zcXWoLlVtl4fPUcIbcc6otEIDcNNW27LBJRcszSw21v4J9DGsUumdr8AU9tno8tDx6Ei23O1OFSW
KvZtDg9eDv3AgpP457UJQrGludwqxoDT5blCpd5YE601eajfky0ZszncOPV4Ep9mnPhjE63sUq/h
N9RH9lOfD2q0+t0ZMuOcwo2JuHdcj9QELK/1HuPjogBCfEZK2KAK7WQI4ysUZSTisGmDuI4QksbH
RBfwAbUoytCGnS9MT5DYU11AJCilcgFERQHdH+dQU5GlON81aJfYC3kCF4+WwpSfKnt4cm4RN16M
YNGtLLjl2Qj2lqQjLVlWZNHxegoDtgXJmGZFwEory9WFNM7RZ+lEcXxvpSCctC+i+mYv+6Zf/c99
+eZEq3kYLU+HJGtDngClBOHgIu7U7Qw5Z35ng7lvion4hbr7HxANshhIYXmaqYtDLNTvM6C8IAUd
r8Ywu+CK6hnZCZTmJKPKoEOsVBn0aKjWi74++6Bf5vNglithQDVjaaIGlYVp2MFauKrJkWcBWY2B
+SUfBmaWEA3pSXGozE+WDrK9ItUYWQwEBmFz5zQevncgWvovlEHPym3gXiQaAdIsWHVBU3WuZMNw
8HXBa4OrXlQCEHACfEMuImYo+XsBgSew5PKITYlUnRYJ8copG/UJKMXA6JQd03blNKoozsPmTelQ
FhBDDAS6oEyfg4JsZQOZaRHqxVpdwI83opFw9qN1AasKE2ytgXdnv/+EWvC9lAQIAib8AojX18Mu
dbzf+XYE5t5y7CrJxlroG7OLeykjmMVP/pF04q7R63Kr+rc8EvHaxGLno9NWMYe8g52OOFPNPIva
I+AxsM6N/QA1up+c6eK2/UlMLN1vhG0H59mC9RbR6Ht+3v/6J3sbSaptM7q9QgOL4Fo2bYAq0AlB
EMyJ8bTdaW62Bs6o9zoUI78BrKaC0eCdN3IAAAAASUVORK5CYII=
PNG_32
        ;;
    48)
        base64 -d <<'PNG_48'
iVBORw0KGgoAAAANSUhEUgAAADAAAAAwCAYAAABXAvmHAAAFOUlEQVR4nNWab0wbZRzHv9cCo/wr
iPyZdm0NA8vYlCVz2RsjL5SpmxE1xkVNxJeLMWMuwZDFUGIWEqZxL4zGaCIxMSPGZBiHjsWYmWWG
zCnoINQUXMeAMZatLX9qKe09Ps+VHv137T2wHvghD8/dc89z93zv+f2e33N31eF/TpaaSrauG08Q
iPWEoBgaIAjwCNANOVq3/ZK2rtIBW+c1a0gQ2wmEJrqrSceTQIWQXj3RdTjaHnIlq5BUQHXnWLNI
8BE2ruPxeHQCjjrbtnfHH0gQsP3E33Z619uxCaGj0TF2/GF7bFkUVSdGm4kofIlNjKAjb44fr+2W
9yMbts5R69KyOEiLNsRsTMXZqK3Ygh0VuRi47sPoLT/m/GJCPerg3pwsod7RVuti+/Is5A+E7JBs
nkBLdlTm4r3GSuyz5stlR1byb4fceP/8TIwQOhMa/QHY6WYz25dHwNL+lxsaO22jrQgfNJlQlKtX
rDPpCeBQ9z80X44u9l3veERSrGP/rPbBBkLEYpqgVTIZs3Dy+dSdZ5iKc/DZK5b49nmsz7IAhFAP
kQ6ThqmloRxGQ+rOR6jbasDLjxbHtBdC2CMLEElQ07vP0lM2I3horDXGtA+R0E5WLjmxyFQpB+V7
zraSHHr3Va1iZGorDVLHI9BYZWF5+CxUANFQQGGOjreJZG5EXBUQ6W14BOgfiHYChqcXwMvI9GLM
CEAgt1i2OgIaCmD0j9zB/rpS1fXPDd+NGwHiYbksQMkHbFsLUFNZgFT8NHIbvkAIPHx+cUq1gLl/
g/jmygy1kmgfIDMsT2lCjXXlOLTPhHTsNheh6wcnl4hLY258eN6FY41WpOv8S5/+CY8vEFNOhPCK
IexNzISSpN0WdVOduTQPrc9Ww5AlJD2PUjp57hqOnB6VOpmMG3f9ePGTQVydnEtoixVzSjmNEo51
ERPx7sEa/OHypKw3ccdH67jl/Z7L0/jx6iye2VWGugcKsPPBQvw67sbw1AItv614HkLIqgDFaZRz
XWcuzZdSOs5cmUQvTRE8iwGcHpgCDwKiBShOo5lZmb6wx0SFGvDFz2Pczi8jRAlQMqGGaqN0oUyw
12zCa3srcOb3aSk5bvLFBlUmVFaYjdJ8vpDPAzv3O/urpMS4/60+1W1jTEhpGtX20QaxkTZdXSF+
BDSOxMmIjrTpENT4QDwT7gAmPAGsh110Van0HMA1AkTVLBTL4x874fWrv0gyDtQW4evXLckPcgiA
mkgc7wRm+ni3Xswl2YrHeKK4qkgcz8W3q5FJ1m5CGj/QKMHlxGqmUa0n0nVNo2pMaNY9j3nfEtSQ
pdfDUlkCLniceC0mNOScBg8lhQYU5eeqrr8WEwq/VkHyVx/xmCvU39Gy4gIYtmSDB57XMpLZgzMS
2yzlUsoUGY/EmWbN0yj7JiWK64uw9wIeAXQEXCzPWmk4RN8vJlRaXApCKxw35+Xoqgq94GKZ5MT+
nsMXqAhvvKO0fXUJCxqIYNdg1+JwYi/rM2u7+rRCSC+1qzeiT9x32Yn7XnXiwGNVKDfmIRPMen3o
+22cq40gCL2RbVmALjtoDy6BflIVEt6lnKVCNg/Em1+ElsiiXl6YB4f7PXrb0zP0VUpTeAmxOZMO
wmFfz9EBxAtgiI7+IaHmSRbkGqRQvemS2BH67tip6D4nnfz1z3U107h2igrh+wqRKeiXSRq3WkLf
t3YnHlIgt6nTGliGfeWnBhslxMt+apCTDbu/t82VrIK68Huws4EuPazUyKzQBJ2LGrsLZ9supKu5
8euHdfIfWZpIYAGgNFkAAAAASUVORK5CYII=
PNG_48
        ;;
    64)
        base64 -d <<'PNG_64'
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
PNG_64
        ;;
    128)
        base64 -d <<'PNG_128'
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
PNG_128
        ;;
    256)
        base64 -d <<'PNG_256'
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
AJHGmACINPZHfyfe5t4irGMAAAAASUVORK5CYII=
PNG_256
        ;;
    *) fail "the icon is not drawn at $1 pixels" ;;
    esac
}
# END GENERATED ICONS

# The freedesktop desktop entry, on standard output. $1 is the graphical
# application it runs.
#
# StartupWMClass is what a desktop environment matches an open window to this
# entry by, and so what hands the window this entry's icon and keeps one
# taskbar button rather than two. It is the application id the graphical
# application gives itself - `gui::XDG_APP_ID`, which Slint sets as the X11
# WM_CLASS and the Wayland app_id - and `crates/gui/tests/desktop_entry.rs`
# holds the two to each other.
desktop_entry() {
    cat <<DESKTOP
[Desktop Entry]
Type=Application
Name=Repos Explorer
Comment=A front door to the working copies source control checks out on this machine
Exec="$1" %f
Icon=$ICON_NAME
Terminal=false
Categories=Development;Utility;FileTools;
StartupWMClass=$ICON_NAME
DESKTOP
}

# Tells the desktop environment that the applications and the icons changed.
# $1 is the applications directory and $2 the icon theme's. Both commands are
# optional: a machine may carry neither, and a desktop environment that reads
# the directories as it finds them needs neither.
refresh_desktop() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$1" >/dev/null 2>&1 || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        # --ignore-theme-index: a user's own icon directory carries no
        # index.theme, and without this the command refuses it.
        gtk-update-icon-cache --ignore-theme-index --quiet --force "$2" >/dev/null 2>&1 || true
    fi
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
xdg_data_dir=""

while [ $# -gt 0 ]; do
    case "$1" in
        --tag) tag="${2:?--tag needs a value}"; tag_given=1; shift 2 ;;
        --from-directory) from_directory="${2:?--from-directory needs a value}"; shift 2 ;;
        --unsigned-test-manifest) unsigned=1; shift ;;
        --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
        --bin-dir) bin_dir="${2:?--bin-dir needs a value}"; shift 2 ;;
        --xdg-data-dir) xdg_data_dir="${2:?--xdg-data-dir needs a value}"; shift 2 ;;
        --uninstall) uninstall=1; shift ;;
        --purge) purge=1; shift ;;
        --yes) yes=1; shift ;;
<<<<<<< HEAD
        -h | --help) sed -n '2,43p' "$0"; exit 0 ;;
=======
        -h | --help) sed -n '2,48p' "$0"; exit 0 ;;
>>>>>>> 229a912 (feat(#562): Linux gets a desktop entry, themed icons and a one-file AppImage)
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
        xdg_data_dir="${xdg_data_dir:-${XDG_DATA_HOME:-$HOME/.local/share}}"
        applications="$xdg_data_dir/applications"
        icons="$xdg_data_dir/icons/hicolor"
        ;;
    Darwin/arm64)
        target="aarch64-apple-darwin"
        data_directory="$HOME/Library/Application Support/RepoSphereExplorer"
        prefix="${prefix:-$HOME/Applications/RepoSphereExplorer}"
        bundle="Repos Explorer.app"
        [ -z "$bin_dir" ] || fail "--bin-dir is for Linux only"
        [ -z "$xdg_data_dir" ] || fail "--xdg-data-dir is for Linux only"
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
<<<<<<< HEAD
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
=======
    if [ "$target" = "x86_64-unknown-linux-gnu" ]; then
        # The icon theme's directories are shared with the desktop's own
        # icons, so rmdir is the right tool: it takes away the ones this
        # install left empty and refuses the rest.
        local size
        for size in $ICON_SIZES; do
            rmdir "$icons/${size}x${size}/apps" "$icons/${size}x${size}" 2>/dev/null || true
        done
        rmdir "$icons/scalable/apps" "$icons/scalable" 2>/dev/null || true
        refresh_desktop "$applications" "$icons"
>>>>>>> 229a912 (feat(#562): Linux gets a desktop entry, themed icons and a one-file AppImage)
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

        # What makes the application appear in the applications menu, and
        # gives its window and that menu entry a picture rather than a blank
        # square. Every file written here goes in the receipt, so uninstall
        # takes away exactly what this made.
        local entry="$applications/$ICON_NAME.desktop" size icon_directory
        mkdir -p "$applications"
        desktop_entry "$prefix/RepoSphereExplorerGui" > "$entry"
        placed="$placed$entry"$'\n'
        echo "placed $entry"
        for size in $ICON_SIZES; do
            icon_directory="$icons/${size}x${size}/apps"
            mkdir -p "$icon_directory"
            icon_png "$size" > "$icon_directory/$ICON_NAME.png"
            placed="$placed$icon_directory/$ICON_NAME.png"$'\n'
        done
        mkdir -p "$icons/scalable/apps"
        icon_svg > "$icons/scalable/apps/$ICON_NAME.svg"
        placed="$placed$icons/scalable/apps/$ICON_NAME.svg"$'\n'
        echo "placed the icon under $icons, at $(echo $ICON_SIZES | tr ' ' ',') pixels and as a drawing"
        refresh_desktop "$applications" "$icons"
    fi
    printf '%s' "$placed" > "$prefix/$RECEIPT"
    echo "installed Repos Explorer $version in $prefix"
}

if [ -n "$purge" ] && [ -z "$uninstall" ]; then
    fail "--purge only goes with --uninstall"
fi
if [ -n "$uninstall" ]; then do_uninstall; else do_install; fi
