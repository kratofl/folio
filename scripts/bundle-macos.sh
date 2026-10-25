#!/bin/sh
# Builds Folio.app into target/release, with the Safelight icon, so Folio
# opens from Finder or the Dock and starts on the welcome screen.
# Usage: scripts/bundle-macos.sh
# Needs only what ships with macOS: QuickLook renders the SVG icon, sips
# scales it and iconutil packs it.
set -eu

cd "$(dirname "$0")/.."
cargo build --release -p folio

app=target/release/Folio.app
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# macOS icons keep a margin around the shape: 824 of 1024 pixels.
{
    echo '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><g transform="translate(100 100) scale(0.8046875)">'
    sed -e '/<svg/d' -e '/<\/svg>/d' assets/brand/folio-icon.svg
    echo '</g></svg>'
} > "$work/icon.svg"
qlmanage -t -s 1024 -o "$work" "$work/icon.svg" > /dev/null
mkdir "$work/Folio.iconset"
for size in 16 32 128 256 512; do
    sips -z $size $size "$work/icon.svg.png" --out "$work/Folio.iconset/icon_${size}x${size}.png" > /dev/null
    double=$((size * 2))
    sips -z $double $double "$work/icon.svg.png" --out "$work/Folio.iconset/icon_${size}x${size}@2x.png" > /dev/null
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/folio/Cargo.toml | head -n 1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/folio "$app/Contents/MacOS/folio"
iconutil -c icns "$work/Folio.iconset" -o "$app/Contents/Resources/Folio.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Folio</string>
    <key>CFBundleDisplayName</key><string>Folio</string>
    <key>CFBundleIdentifier</key><string>com.github.kratofl.folio</string>
    <key>CFBundleExecutable</key><string>folio</string>
    <key>CFBundleIconFile</key><string>Folio</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.photography</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
echo "Built $app"
