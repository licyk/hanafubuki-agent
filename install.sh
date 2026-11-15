#!/usr/bin/env bash

main() (
    set -euo pipefail
    local version=latest architecture=auto install_dir=${HANAFUBUKI_AGENT_INSTALL_DIR:-} no_path=false
    while (($#)); do
        case "$1" in
            --version|--architecture|--install-dir)
                if (($# < 2)) || [[ -z $2 || $2 == --* ]]; then fail "Missing value for $1"; fi
                case "$1" in
                    --version) version=$2 ;;
                    --architecture) architecture=$2 ;;
                    --install-dir) install_dir=$2 ;;
                esac
                shift 2 ;;
            --no-path-update) no_path=true; shift ;;
            -h|--help) usage; return ;;
            *) fail "Unknown option: $1" ;;
        esac
    done
    case "$architecture" in auto|x64|arm64) ;; *) fail 'Architecture must be auto, x64 or arm64.' ;; esac
    local system
    system=$(uname -s)
    case "$system" in
        MINGW*|MSYS*|CYGWIN*) install_windows "$version" "$architecture" "$install_dir" "$no_path"; return ;;
        Linux|Darwin) ;;
        *) fail "Unsupported operating system: $system" ;;
    esac
    command -v curl >/dev/null || fail 'curl is required.'
    if [[ $architecture == auto ]]; then
        case "$(uname -m)" in
            x86_64|amd64) architecture=x64 ;;
            aarch64|arm64) architecture=arm64 ;;
            *) fail 'Only x64 and ARM64 are supported.' ;;
        esac
        if [[ $system == Darwin && $(sysctl -n hw.optional.arm64 2>/dev/null || true) == 1 ]]; then architecture=arm64; fi
    fi
    local target=x86_64
    [[ $architecture != arm64 ]] || target=aarch64
    if [[ $system == Linux ]]; then target+=-unknown-linux-gnu; else target+=-apple-darwin; fi
    local base=${HANAFUBUKI_AGENT_RELEASE_BASE_URL:-https://github.com/licyk/hanafubuki-agent/releases}
    base=${base%/}
    if [[ $version == latest ]]; then
        local location
        location=$(curl -fsSL --connect-timeout 30 --max-time 120 -o /dev/null -w '%{url_effective}' "$base/latest")
        version=${location##*/}
    fi
    version=${version#v}
    [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] || fail 'Expected a release version such as 0.4.3.'
    install_dir=${install_dir:-$HOME/.local/bin}
    case "$install_dir" in *:*|*$'\n'*) fail 'The installation directory cannot contain a colon or newline.' ;; esac
    mkdir -p -- "$install_dir"
    install_dir=$(cd -- "$install_dir" && pwd)
    local staging directory archive expected actual
    staging=$(mktemp -d "$install_dir/.hanafubuki-agent-install.XXXXXX")
    trap 'rm -rf -- "$staging"' EXIT
    directory="hanafubuki-agent-$version-$target"
    archive="$directory.tar.gz"
    printf 'Downloading hanafubuki-agent %s (%s)...\n' "$version" "$target"
    download "$base/download/v$version/$archive" "$staging/$archive"
    download "$base/download/v$version/$archive.sha256" "$staging/checksum"
    expected=$(awk 'NR == 1 {print $1}' "$staging/checksum")
    [[ $expected =~ ^[0-9a-fA-F]{64}$ ]] || fail 'The release checksum is invalid.'
    if command -v sha256sum >/dev/null; then
        actual=$(sha256sum "$staging/$archive" | awk '{print $1}')
    else
        actual=$(shasum -a 256 "$staging/$archive" | awk '{print $1}')
    fi
    [[ $(printf '%s' "$expected" | tr 'A-F' 'a-f') == "$actual" ]] || fail 'Downloaded archive SHA-256 does not match.'
    tar xzf "$staging/$archive" -C "$staging" "$directory/hanafubuki-agent" \
        "$directory/LICENSE" "$directory/NOTICE" "$directory/THIRD-PARTY-LICENSES.txt"
    chmod 755 "$staging/$directory/hanafubuki-agent"
    [[ $("$staging/$directory/hanafubuki-agent" --version) == "hanafubuki-agent $version" ]] || fail 'Downloaded executable reports an unexpected version.'
    mkdir -p "$install_dir/hanafubuki-agent-notices"
    cp "$staging/$directory/"{LICENSE,NOTICE,THIRD-PARTY-LICENSES.txt} "$install_dir/hanafubuki-agent-notices/"
    mv -f "$staging/$directory/hanafubuki-agent" "$install_dir/hanafubuki-agent"
    if ! $no_path; then add_path "$install_dir"; fi
    printf 'Installed %s/hanafubuki-agent\nOpen a new terminal, then run hanafubuki-agent.\n' "$install_dir"
)

fail() { printf '%s\n' "$*" >&2; exit 1; }
download() { curl -fsSL --retry 3 --connect-timeout 30 --max-time 600 "$1" -o "$2"; }

add_path() {
    local directory=$1 profile line existing found=false shell=${SHELL:-sh}
    case "${shell##*/}" in
        zsh) profile="${ZDOTDIR:-$HOME}/.zshrc" ;;
        bash)
            if [[ $(uname -s) == Darwin ]]; then profile="$HOME/.bash_profile"; else profile="$HOME/.bashrc"; fi ;;
        *) profile="$HOME/.profile" ;;
    esac
    printf -v line 'export PATH=%q:' "$directory"
    line+="\"\$PATH\" # hanafubuki-agent"
    mkdir -p -- "$(dirname -- "$profile")"
    if [[ -f $profile ]]; then
        while IFS= read -r existing || [[ -n $existing ]]; do
            if [[ $existing == "$line" ]]; then found=true; break; fi
        done < "$profile"
    fi
    if ! $found; then
        printf '\n%s\n' "$line" >> "$profile"
    fi
    printf 'PATH configured in %s.\n' "$profile"
}

install_windows() {
    local version=$1 architecture=$2 install_dir=$3 no_path=$4 backend staging='' powershell
    command -v cygpath >/dev/null || fail 'cygpath is required; use Git Bash or MSYS2.'
    if command -v powershell.exe >/dev/null; then powershell=powershell.exe; else powershell=pwsh.exe; fi
    command -v "$powershell" >/dev/null || fail 'Windows PowerShell or PowerShell 7 is required.'
    backend=''
    if [[ -f ${BASH_SOURCE[0]:-} ]]; then backend="$(dirname -- "${BASH_SOURCE[0]}")/install.ps1"; fi
    if [[ -z $backend || ! -f $backend ]]; then
        staging=$(mktemp -d)
        trap 'rm -rf -- "$staging"' EXIT
        backend="$staging/install.ps1"
        download https://raw.githubusercontent.com/licyk/hanafubuki-agent/main/install.ps1 "$backend"
    fi
    local -a args=(-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$(cygpath -aw -- "$backend")" -Version "$version" -Architecture "$architecture")
    [[ -z $install_dir ]] || args+=(-InstallDir "$(cygpath -aw -- "$install_dir")")
    if $no_path; then args+=(-NoPathUpdate); fi
    MSYS2_ARG_CONV_EXCL='*' MSYS_NO_PATHCONV=1 "$powershell" "${args[@]}"
}

usage() {
    cat <<'HELP'
Usage: bash install.sh [options]
Install hanafubuki-agent on Linux, macOS, or Windows (Git Bash/MSYS2).
  --version VERSION       latest (default), or a version such as 0.4.3
  --architecture ARCH     auto (default), x64, or arm64
  --install-dir DIRECTORY Override HANAFUBUKI_AGENT_INSTALL_DIR
  --no-path-update        Do not change shell profiles or Windows user PATH
  -h, --help              Show help
Default directory: ~/.local/bin, or %LOCALAPPDATA%/hanafubuki-agent/bin on Windows.
HELP
}

main "$@"
