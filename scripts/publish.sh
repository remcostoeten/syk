#!/usr/bin/env bash
set -euo pipefail

readonly REPOSITORY="remcostoeten/syk"
readonly CRATE="syk-search"
readonly AUR_HOST_KEY="aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN"

usage() {
    echo "usage: scripts/publish.sh crates|package|aur vX.Y.Z" >&2
    exit 2
}

[[ $# -eq 2 && ${2:-} =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
readonly TASK=$1 TAG=$2 VERSION=${2#v}

check_version() {
    local declared
    declared=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
    if [[ $declared != "$VERSION" ]]; then
        echo "publish: the tag is $TAG and Cargo.toml says $declared." >&2
        exit 1
    fi
}

publish_crate() {
    if [[ -z ${CARGO_REGISTRY_TOKEN:-} ]]; then
        echo "publish: CARGO_REGISTRY_TOKEN is not set, so crates.io is skipped."
        return
    fi
    local status
    status=$(curl -s -o /dev/null -w '%{http_code}' -A "syk-release" \
        "https://crates.io/api/v1/crates/$CRATE/$VERSION")
    if [[ $status == 200 ]]; then
        echo "publish: $CRATE $VERSION is on crates.io already."
        return
    fi
    cargo publish --locked
}

checksum() {
    curl -fsSL "https://github.com/$REPOSITORY/releases/download/$TAG/syk-search-$1-unknown-linux-gnu.tar.xz.sha256" |
        cut -d ' ' -f 1
}

package() {
    local folder=$1 x86 arm
    x86=$(checksum x86_64)
    arm=$(checksum aarch64)
    [[ $x86 =~ ^[0-9a-f]{64}$ && $arm =~ ^[0-9a-f]{64}$ ]]
    sed -e "s/^pkgver=.*/pkgver=$VERSION/" \
        -e "s/^pkgrel=.*/pkgrel=1/" \
        -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$x86')/" \
        -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$arm')/" \
        packaging/aur/PKGBUILD >"$folder/PKGBUILD"
    if command -v makepkg >/dev/null; then
        (cd "$folder" && makepkg -f --noconfirm --nodeps && makepkg --printsrcinfo >.SRCINFO)
    else
        chmod 777 "$folder"
        docker run --rm -v "$(realpath "$folder"):/package" archlinux:base-devel bash -c \
            'useradd -m builder && cd /package && su builder -c "makepkg -f --noconfirm --nodeps && makepkg --printsrcinfo > .SRCINFO"'
    fi
}

publish_aur() {
    if [[ -z ${AUR_SSH_KEY:-} ]]; then
        echo "publish: AUR_SSH_KEY is not set, so the AUR is skipped."
        return
    fi
    local work
    work=$(mktemp -d)
    trap 'rm -rf "$work" 2>/dev/null || true' RETURN
    mkdir "$work/build"
    package "$work/build"
    printf '%s\n' "$AUR_SSH_KEY" >"$work/key"
    chmod 600 "$work/key"
    printf '%s\n' "$AUR_HOST_KEY" >"$work/hosts"
    export GIT_SSH_COMMAND="ssh -i $work/key -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$work/hosts"
    git clone -q ssh://aur@aur.archlinux.org/syk-bin.git "$work/aur"
    cp "$work/build/PKGBUILD" "$work/build/.SRCINFO" "$work/aur/"
    git -C "$work/aur" add PKGBUILD .SRCINFO
    if git -C "$work/aur" diff --cached --quiet; then
        echo "publish: the AUR has $VERSION already."
        return
    fi
    git -C "$work/aur" -c user.name="Remco Stoeten" \
        -c user.email="remcostoeten@users.noreply.github.com" \
        commit -q -m "Release $VERSION"
    git -C "$work/aur" push -q origin HEAD:master
}

check_version
case $TASK in
    crates) publish_crate ;;
    package)
        folder=$(mktemp -d)
        package "$folder"
        echo "publish: the package is in $folder"
        ;;
    aur) publish_aur ;;
    *) usage ;;
esac
