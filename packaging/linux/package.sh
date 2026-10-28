#!/bin/bash
# Build .deb and .rpm packages from the release binaries:
#   packaging/linux/package.sh <version> <target> <out-dir>
# Needs dpkg-deb and rpmbuild (sudo apt-get install rpm).
set -euo pipefail
version="${1#v}"; target="$2"; out="$(cd "$3" && pwd)"
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
bin="${BIN_DIR:-$root/target/$target/release}"
case "$target" in
  x86_64-*) deb_arch=amd64; rpm_arch=x86_64 ;;
  aarch64-*) deb_arch=arm64; rpm_arch=aarch64 ;;
  *) echo "unknown target $target" >&2; exit 1 ;;
esac

# The installed tree, shared by both.
stage="$(mktemp -d)"
install -Dm755 "$bin/datadrop" "$stage/usr/bin/datadrop"
install -Dm755 "$bin/datadrop-gui" "$stage/usr/bin/datadrop-gui"
install -Dm644 "$here/datadrop.desktop" "$stage/usr/share/applications/datadrop.desktop"
install -Dm644 "$here/../icon.png" "$stage/usr/share/icons/hicolor/512x512/apps/datadrop.png"
install -Dm644 "$here/../icon.svg" "$stage/usr/share/icons/hicolor/scalable/apps/datadrop.svg"
install -Dm644 "$root/LICENSE" "$stage/usr/share/doc/datadrop/copyright"
install -Dm644 "$root/README.md" "$stage/usr/share/doc/datadrop/README.md"

# .deb
deb="$(mktemp -d)"
cp -a "$stage/." "$deb/"
mkdir "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<CONTROL
Package: datadrop
Version: $version
Architecture: $deb_arch
Maintainer: The DataDrop authors <noreply@github.com>
Section: database
Priority: optional
Depends: libc6 (>= 2.35), libgcc-s1, libxkbcommon0, libgl1, libegl1
Recommends: libwayland-client0, libxkbcommon-x11-0, libxcursor1, libxrandr2, libxi6
Homepage: https://github.com/jabba2324/datadrop
Description: SQL client for PostgreSQL, MySQL, SQLite, SQL Server, ClickHouse and DuckDB
 A desktop app (datadrop-gui) with a schema browser, a SQL editor with
 autocompletion and an editable results grid, and a command line (datadrop)
 that does everything the app does.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "$out/datadrop_${version}_${deb_arch}.deb"

# .rpm (from the same files; nothing is compiled)
top="$(mktemp -d)"
mkdir -p "$top"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
cat > "$top/SPECS/datadrop.spec" <<SPEC
Name:           datadrop
Version:        ${version//-/_}
Release:        1
Summary:        SQL client for PostgreSQL, MySQL, SQLite, SQL Server, ClickHouse and DuckDB
License:        MIT
URL:            https://github.com/jabba2324/datadrop
BuildArch:      $rpm_arch
Requires:       libxkbcommon, mesa-libGL, mesa-libEGL
%global debug_package %{nil}
%global __strip /bin/true

%description
A desktop app (datadrop-gui) with a schema browser, a SQL editor with
autocompletion and an editable results grid, and a command line (datadrop)
that does everything the app does.

%install
cp -a $stage/. %{buildroot}/

%files
/usr/bin/datadrop
/usr/bin/datadrop-gui
/usr/share/applications/datadrop.desktop
/usr/share/icons/hicolor/512x512/apps/datadrop.png
/usr/share/icons/hicolor/scalable/apps/datadrop.svg
%license /usr/share/doc/datadrop/copyright
%doc /usr/share/doc/datadrop/README.md
SPEC
rpmbuild --define "_topdir $top" --target "$rpm_arch" -bb "$top/SPECS/datadrop.spec" >/dev/null
cp "$top"/RPMS/*/*.rpm "$out/"
ls -l "$out"
