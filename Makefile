# Makefile for sheep-rhel
#
# Targets:
#   make build        — build release binaries with cargo
#   make appimage     — build x86_64 AppImage (requires release binaries)
#   make clean        — remove build artifacts

.PHONY: build appimage clean

build:
	cargo build --release

appimage: build
	./scripts/build-appimage.sh

clean:
	rm -rf target/release/sheep-rhel-x86_64.AppImage
	rm -rf target/appimage
