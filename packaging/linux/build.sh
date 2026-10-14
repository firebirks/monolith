#!/bin/sh
# Builds the Linux archive of the desktop client (packaging/README.md): the
# release binary, its desktop entry, its icon, an installer for the user's
# home and the licences, in one tar.gz that unpacks into a folder of its
# own. Run it from anywhere in the repository; it needs git, cargo, GNU tar
# and sha256sum. It writes to target/package/.
set -eu

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
name="monolith-desktop-$version-linux-$(uname -m)"
out="target/package/$name"
rm -rf "$out" "target/package/$name.tar.gz" "target/package/$name.tar.gz.sha256"
mkdir -p "$out/licenses"

# No symbols in the binary; nothing else differs from a release build.
CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked -p monolith-desktop
cargo run --release --locked --quiet -p monolith-desktop --example icon -- "$out/monolith.png" 256

cp target/release/monolith-desktop "$out/"
cp packaging/linux/monolith.desktop packaging/linux/install.sh "$out/"
cp packaging/linux/README.txt "$out/"
cp LICENSE "$out/"
cp crates/monolith-ui-kit/assets/licenses/* "$out/licenses/"
chmod 0755 "$out/monolith-desktop" "$out/install.sh"
chmod 0644 "$out/monolith.png" "$out/monolith.desktop" "$out/README.txt" "$out/LICENSE" "$out"/licenses/*

# The same files give the same archive: names sorted, no owner, the time of
# the commit.
stamp=$(git log -1 --format=%ct)
tar -C target/package --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@$stamp" -czf "target/package/$name.tar.gz" "$name"
(cd target/package && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
cat "target/package/$name.tar.gz.sha256"
