set -euo pipefail
umask 022

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
binary_path="${1:-$project_root/target/release/mint-pulse}"
output_dir="${2:-$project_root/dist}"
upstream_version="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$project_root/Cargo.toml")"
package_version="${upstream_version/-/~}-1"
architecture="$(dpkg --print-architecture)"

if [[ ! -f "$binary_path" || ! -x "$binary_path" ]]; then
    printf 'Build the release binary before packaging: %s\n' "$binary_path" >&2
    exit 1
fi

dpkg --validate-version "$package_version"

if [[ "$("$binary_path" --version)" != "mint-pulse $upstream_version" ]]; then
    printf 'The binary version does not match Cargo.toml\n' >&2
    exit 1
fi

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    SOURCE_DATE_EPOCH="$(git -C "$project_root" log -1 --format=%ct)"
fi

if [[ ! "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
    printf 'SOURCE_DATE_EPOCH must be a Unix timestamp\n' >&2
    exit 1
fi

export SOURCE_DATE_EPOCH
staging_dir="$(mktemp -d -t mint-pulse-deb.XXXXXXXX)"
trap 'rm -rf -- "$staging_dir"' EXIT
package_root="$staging_dir/package"
install -d -m 0755 "$package_root/DEBIAN" "$package_root/usr/bin" \
    "$package_root/usr/share/applications" "$package_root/usr/share/doc/mint-pulse" "$staging_dir/debian"
install -m 0755 "$binary_path" "$package_root/usr/bin/mint-pulse"
install -m 0644 "$project_root/packaging/mint-pulse.desktop" "$package_root/usr/share/applications/mint-pulse.desktop"
install -m 0644 "$project_root/LICENSE" "$package_root/usr/share/doc/mint-pulse/copyright"
install -m 0644 "$project_root/packaging/README.Debian" "$package_root/usr/share/doc/mint-pulse/README.Debian"
install -m 0644 "$project_root/packaging/control" "$staging_dir/debian/control"

dependencies="$(cd "$staging_dir" && dpkg-shlibdeps -O -e"$package_root/usr/bin/mint-pulse")"
dependencies="${dependencies#shlibs:Depends=}"

if [[ -z "$dependencies" || "$dependencies" == *$'\n'* ]]; then
    printf 'Cannot determine runtime dependencies\n' >&2
    exit 1
fi

installed_size="$(du -sk "$package_root/usr" | cut -f1)"
cat > "$package_root/DEBIAN/control" <<EOF
Package: mint-pulse
Version: $package_version
Section: utils
Priority: optional
Architecture: $architecture
Maintainer: Sam Zaitcev <22082957+SamZaitcev@users.noreply.github.com>
Installed-Size: $installed_size
Depends: $dependencies, fonts-dejavu-core
Homepage: https://github.com/SamZaitcev/mint-pulse
Description: Lightweight system graphs for the Cinnamon tray
 Displays CPU, RAM, swap and filesystem occupancy in a horizontal XApp tray.
 Graphs update every two seconds and support dark, light and high-contrast themes.
 A Cinnamon desktop with the XApp Status Applet is required.
EOF

release_date="$(date -u --date="@$SOURCE_DATE_EPOCH" -R)"
cat > "$staging_dir/changelog" <<EOF
mint-pulse ($package_version) unstable; urgency=medium

  * Package the Rust Cinnamon tray indicator.

 -- Sam Zaitcev <22082957+SamZaitcev@users.noreply.github.com>  $release_date
EOF
gzip -n -9 -c "$staging_dir/changelog" > "$package_root/usr/share/doc/mint-pulse/changelog.Debian.gz"
mkdir -p -- "$output_dir"
filename="mint-pulse_${package_version}_${architecture}.deb"
dpkg-deb --build --root-owner-group "$package_root" "$output_dir/$filename"

(
    cd "$output_dir"
    sha256sum "$filename" > SHA256SUMS
)

printf 'Built %s\n' "$output_dir/$filename"
