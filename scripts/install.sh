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
iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAACDElEQVR4Ae3AA6AkWZbG8f937o3I
zKdyS2Oubdu2bdu2bdu2bWmMnpZKr54yMyLu+Xa3anqmhztr1a/yTI/+qqc/2GO8lc1xXgiJXXX5
M0/4mIfcCiCAR3zBk9474bv4VyjWhz3p0x/xjXrwFz3+wQzt6fwLHnvdnI96zTPszAtf8zv38cfP
OIK+PKRytHrrtPmXfPM7PJybT8wAeMy1N/Nm3/hE7ri0/Iw6javjRvxLbj4x437HFpXrd8Qzzg0P
rTkNGHHLyQUv/aBjABwNjV//h7M80K/8w3ne6MVOAXDHxRV/f/slPI1PrNM0AOKlbj7DW73Mddzv
UdducNv5IwBuO3/Ee377X/OBr3UzxxaVb/2d27m0nADfU3MaAPHIMzMefrrnfg8/fS0P9Je3XuSH
/vg2/uBJ53k2U3MasMUjr93gEWdmvCCPOHMdr/rw47zUp/wK95NMzWnAFnZy28WBD/2JO3huH/Kq
p3izxx4DQ04D95NMnaYBLGxzy4me733H67jfYtaxmHU8m2nTwLPI1JwGsMDJcj3y1DvPcb9rTmzx
oOtO8kA5DTyLTHWbdjONbRazjld4zC28IIfriTYN3C9Cu9XOn85x/Kov/fE/hkxemC/9yT8lx4H7
qe9+WgDlrb/ivZ3+Lv4VFHqf9tMf993ifm/9RQ9m9FtjH+eFkXbp9NP89KfcCvCP7orR3QsPzngA
AAAASUVORK5CYII=
PNG_16
        ;;
    32)
        base64 -d <<'PNG_32'
iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAESklEQVR4Ae3AA6AkWZbG8f937o3I
zKdyS2Oubdu2bdu2bdu2bWmMnpZKr54yMyLu+Xa3anqmhztr1S/470XluTz6i57+4CY+C3htw4P5
DyC4FfjtYj7nCZ/ykFt5NsQDPOKLn/pRmf5q/hNF6KOf/MkP+xquQDzTQz//CR8N+ir+S/hjnvbp
j/5qAAE8+ose/+D16KfzH+ix1854g0dtA3DH7sivPWmfvVVyv1mnhzzhUx5zawVYrcfPNuI/yme+
0XW87yuf5oHu2B34wB+5jcfdswJgteYzgfcNALd8LbeGW8Ot4dZwa7g13BpuDbeGW8Ot4dZwa7g1
3BpuDbeGW+N9X/EE7/vKp3luNx3v+ZZ3vIXtDtwabtPbAQRAZnuws+FsOBvOhrPhbDgbzoaz4Ww4
G86Gs+FsOBvOhrPhbLzPK5/mBbn5RM8bPnoLZyMzdwAqgLPxH+XmEzNemJuOdTgbz0QFcDb+q9jG
2XgmKkBm4z/Kr/zDed7oxU7xgvzy358js/FMVABn44E+8U0fzqOv3+a53Xb+iC/9xadwNDRekM/4
mafwKg89xs6i8ty+7ffu5O/u2OMBCABnw9lwNh517QaPvn6b5+eWUxt84ps+nEUFZ8PZcDacDWfD
2bjt3BGv9xV/xi///Vnud/uFFZ/500/mM37qSTgbzoazAVABnI372ckLc8upDb7hvV6G5+en//wO
fvov7uS2c4e817f/DS8CKkBm436bHZzcKPxbvO9rPojH3LDJt/3W07hvb82LgArgbNzv4dcseKUH
bfBv9UoPuoWPfr1b+Ls79vjI7/9b/u6OPV4IAsCt4dZwa9jmP8JL3LTDi9+0g1vDreHWcGu4Ndwa
bg2ACuBsPEsm97u0bPzdPUteFMfmhZe4fsFzyMTZeCGoAJmN+9nJ/T7lF+7iB/9qlxfV3378o7jl
RM/97CSz8UJQAZyN+9nmfp/8etdyy4meF8UtJ3puOdHzQLZxNl4IKoCz8SxO7nfLiZ5Pfr1r+Tdz
4my8EFQAZ+N+trnfcj2yXI+8INsbM7paeEFs42y8EFSAzMb97OR+T73zHHed2+MFebGHXMeNZ47x
gthJZuOFoAI4G8/i5H6PuuUabjh9jBfk5M4GL5QTZ+OFoAI4G/ezzf26Wji5s8G/lW2cjReCCoDb
M2weBHDfxQP+o9x38QBn4/mReAZABcip/TbwXgC/8KdP4af/7DG83ENP8+/xF087xy/86VN4wfTT
AAKYv/M3PXhaD0/nv1Cd9Q9Z/fCH3FoApr//hd3y6De6ZOcbY4MNNthggw022GCDDTbYYIMNNthg
gw022GCDDTbYYBPwMcOPf8QvAxSeKZ/wK3+sR77+Jew3xgYbbLDBBhtssMEGG2ywwQYbbLDBBhts
sMEGG2ywP6b97Md9NVcgnsv8rb/owcOkj7b91qAH8R/Cz5D00331V69++lNu5dkQ/734R6ymgtFY
y2KPAAAAAElFTkSuQmCC
PNG_32
        ;;
    48)
        base64 -d <<'PNG_48'
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
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
AzBgqQAAAABJRU5ErkJggg==
PNG_128
        ;;
    256)
        base64 -d <<'PNG_256'
iVBORw0KGgoAAAANSUhEUgAAAQAAAAEACAYAAABccqhmAAAk+klEQVR4Ae3AA6AkWZbG8f937o3I
zKdyS2Oubdu2bdu2bdu2bWmMnpZKr54yMyLu+Xa3anqmhztr1S+46qqr/r8iuOqqq/6/Irjqqqv+
vyK46qqr/r8iuOqqq/6/Irjqqqv+vyK46qqr/r8iuOqqq/6/ovJ/zKO/6ukPZqwPMvnSaR6M/NI8
k8yDDQ/mqqteAMGtFrdyP+uvQ9wq4q/ppmc84WMeciv/dyD+l3v0lz79tdK8tsVrY16bq676zyZ+
W+a3Q/z2Ez7xIb/D/16I/2Ve+quefvxo4K0wb214a6666r+Z4KcRP73R8zN//TEP2eV/D8T/Eo/6
4qe/VYP3Bt6aq676n+unC3z3Ez/5IT/D/3yI/8Fe+quefvxg4KOwPxo4zlVX/e+xi/TVWz1f89cf
85Bd/mdC/A/06C96+oOb8rOM3ho4zlVX/e+1K/TdxXzNEz7lIbfyPwvif5CX/qqnHz9c5VcZvTdX
XfV/jPB3b87jY/76Yx6yy/8MiP8hHvHFT/0ow2djjnPVVf9XiV3BZz/5kx/2Nfz3Q/w3e/gXPf21
pfwqm5fmqqv+n5D4ayLe58mf+JC/5r8P4r/RI774yV9l66O56qr/pyR/9ZM/+REfw38PxH+DR3/R
0x88efopo5fmqqv+nxP+66r6Nk/4lIfcyn8tgv9iD/vCp7z16PZXRi/NVVddhdFLj25/9bAvfMpb
818L8V/o4V/45K8yfDRXXXXV8yX46qd86iM+hv8aiP8iD/uCp3wX+L256qqr/gX67qd+2sPfh/98
iP9kL/1VTz++fzR9F/DWXHXVVS8ifff2RvmYv/6Yh+zynwfxn+ilv+rpxw+Oxt8yvDRXXXXVv4rg
r7c2utf56495yC7/ORD/SV76q55+fO9o/C3gpbnqqqv+rf56Z6N7nb/+mIfs8h+Pyn+S/aPhuzAv
zVVXXfXv8dL7h8NXAe/DfzyC/wQP+/wnfpfNW3PVVVf9uxne+2Gf/8Tv4j8ewX+wh33Bk77a8N5c
ddVV/2EM7/2wL3jSV/MfC/Ef6GFf+MS3dvqnuOqqq/5TKPQ2T/3UR/00/zEQ/0Ee/UWPf/Aw6a+A
41x11VX/WXb76pd5wqc85lb+/Qj+gwyTfgo4zlVXXfWf6fgw6af4j4H4D/DQz3vcVxt9FFddddV/
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
rkJggg==
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
