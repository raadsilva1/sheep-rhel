#!/bin/bash
# Build an x86_64 AppImage from target/release binaries.
#
# Usage:
#   cargo build --release
#   ./scripts/build-appimage.sh
#
# The resulting AppImage is written to target/release/sheep-rhel-x86_64.AppImage

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
RELEASE_DIR="${PROJECT_ROOT}/target/release"
APPDIR="${PROJECT_ROOT}/target/appimage/AppDir"
CACHE_DIR="${SCRIPT_DIR}/.cache"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
APPIMAGETOOL="${CACHE_DIR}/appimagetool-x86_64.AppImage"

# ---------------------------------------------------------------------------
# Validate release binaries exist
# ---------------------------------------------------------------------------
for bin in sheep-rhel sheep-run; do
    if [[ ! -x "${RELEASE_DIR}/${bin}" ]]; then
        echo "ERROR: ${RELEASE_DIR}/${bin} not found or not executable."
        echo "Run 'cargo build --release' first."
        exit 1
    fi
done

# ---------------------------------------------------------------------------
# Download appimagetool if missing
# ---------------------------------------------------------------------------
mkdir -p "${CACHE_DIR}"

if [[ ! -x "${APPIMAGETOOL}" ]]; then
    echo "Downloading appimagetool ..."
    curl -fsSL -o "${APPIMAGETOOL}" "${APPIMAGETOOL_URL}"
    chmod +x "${APPIMAGETOOL}"
    echo "appimagetool cached at ${APPIMAGETOOL}"
fi

# ---------------------------------------------------------------------------
# Create AppDir
# ---------------------------------------------------------------------------
echo "Creating AppDir ..."
rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin"

cp "${RELEASE_DIR}/sheep-rhel" "${APPDIR}/usr/bin/"
cp "${RELEASE_DIR}/sheep-run"  "${APPDIR}/usr/bin/"

# ---------------------------------------------------------------------------
# Create .desktop file
# ---------------------------------------------------------------------------
cat > "${APPDIR}/sheep-rhel.desktop" << 'EOF'
[Desktop Entry]
Name=sheep-rhel
Exec=sheep-rhel
Icon=sheep-rhel
Type=Application
Categories=System;
Terminal=true
Comment=Fault-tolerant provisioner for River Classic on RHEL 10.2
EOF

# ---------------------------------------------------------------------------
# Create AppRun launcher
# ---------------------------------------------------------------------------
cat > "${APPDIR}/AppRun" << 'EOF'
#!/bin/bash
# AppRun for sheep-rhel — forwards all arguments to the main binary.
exec "$(dirname "$(readlink -f "$0")")/usr/bin/sheep-rhel" "$@"
EOF
chmod +x "${APPDIR}/AppRun"

# ---------------------------------------------------------------------------
# Generate icon (256x256 PNG, Red Hat red #EE0000)
# ---------------------------------------------------------------------------
echo "Generating icon ..."
python3 -c '
import struct, zlib, sys

def png_chunk(chunk_type, data):
    chunk = chunk_type + data
    crc = zlib.crc32(chunk) & 0xffffffff
    return struct.pack(">I", len(data)) + chunk + struct.pack(">I", crc)

width, height = 256, 256
pixel = bytes([0xEE, 0x00, 0x00, 0xFF])
row = b"\x00" + pixel * width
raw = row * height
compressed = zlib.compress(raw, 9)

png = b"\x89PNG\r\n\x1a\n"
png += png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
png += png_chunk(b"IDAT", compressed)
png += png_chunk(b"IEND", b"")

with open(sys.argv[1], "wb") as f:
    f.write(png)
' "${APPDIR}/sheep-rhel.png"

# ---------------------------------------------------------------------------
# Build AppImage
# ---------------------------------------------------------------------------
echo "Building AppImage ..."
OUTPUT="${RELEASE_DIR}/sheep-rhel-x86_64.AppImage"
"${APPIMAGETOOL}" "${APPDIR}" "${OUTPUT}" 2>&1

echo ""
echo "✅ AppImage ready: ${OUTPUT}"
ls -lh "${OUTPUT}"
