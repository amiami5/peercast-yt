# PeerCast YT (Rust 版) のビルドとインストール。
#
#   make                 # サーバーと、html などを含む配布用のディレクトリ build/peercast-yt
#   make install         # PREFIX (既定 /usr/local) の下にインストール。DESTDIR も使える
#   make uninstall
#   make check           # テスト (単体テストと、実際に起動して試すもの)
#   make dist            # build/peercast-yt-linux-<arch>.tar.gz
#   make appimage
#   make clean
#
# 必要なもの: cargo (Rust 1.70 以降)、OpenSSL、librtmp (WITH_RTMP=no なら不要)。
# 変数はコマンドラインか Makefile.local で変えられる。

WITH_RTMP ?= yes
PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo

BUILD = build
TARGET_DIR = $(BUILD)/target/release
DISTDIR = $(BUILD)/peercast-yt
OS := $(shell uname -s | tr A-Z a-z)
ARCH := $(shell uname -m | tr A-Z a-z)
DISTARCHIVE = $(BUILD)/peercast-yt-$(OS)-$(ARCH).tar.gz
APPIMAGE = $(BUILD)/Peercast_YT-$(ARCH).AppImage
APPIMAGETOOL = $(BUILD)/appimagetool-$(ARCH).AppImage
LINUXDEPLOY = $(BUILD)/linuxdeploy-$(ARCH).AppImage

ifeq ($(WITH_RTMP),yes)
  CARGO_FEATURES = --features peercast-rs/rtmp
endif

bindir = $(DESTDIR)$(PREFIX)/bin
sharedir = $(DESTDIR)$(PREFIX)/share/peercast
docdir = $(DESTDIR)$(PREFIX)/share/doc/peercast
appdir = $(DESTDIR)$(PREFIX)/share/applications
pixmapdir = $(DESTDIR)$(PREFIX)/share/pixmaps

-include Makefile.local

.PHONY: all cargo-build install uninstall check dist appimage clean

# 作り直すかどうかは cargo が決めるので、毎回呼ぶ。配布用のディレクトリは毎回作り直す
# (UI の生成とコピーだけなので速い)。
all: cargo-build
	rm -rf $(DISTDIR)
	mkdir -p $(DISTDIR)
	$(TARGET_DIR)/peercast-ui-gen ui $(DISTDIR)
	cp -R ui/assets licenses LICENSE $(DISTDIR)/
	cp $(TARGET_DIR)/peercast $(TARGET_DIR)/rtmp-server $(DISTDIR)/

cargo-build:
	@command -v $(CARGO) >/dev/null 2>&1 || { echo "error: '$(CARGO)' not found. Install Rust (e.g. 'sudo apt install cargo')." >&2; exit 1; }
	$(CARGO) build --release $(CARGO_FEATURES)

# install はビルドしない (sudo で cargo を動かさないため)。先に一般ユーザーで make しておく。
install:
	@test -x $(DISTDIR)/peercast || { echo "error: $(DISTDIR) がありません。先に (sudo を付けずに) make してください。" >&2; exit 1; }
	mkdir -p $(bindir) $(sharedir) $(docdir) $(appdir) $(pixmapdir)
	install -m 755 $(DISTDIR)/peercast $(DISTDIR)/rtmp-server $(bindir)/
	cp -R $(DISTDIR)/html $(DISTDIR)/public $(DISTDIR)/assets $(sharedir)/
	rm -rf $(sharedir)/cgi-bin
	cp -R licenses LICENSE $(docdir)/
	cp ui/linux/peercast.desktop $(appdir)/
	cp ui/linux/peercast.png $(pixmapdir)/

uninstall:
	rm -f $(bindir)/peercast $(bindir)/rtmp-server
	rm -rf $(sharedir) $(docdir)
	rm -f $(appdir)/peercast.desktop $(pixmapdir)/peercast.png

check:
	$(CARGO) test --release --workspace $(CARGO_FEATURES)

dist: all
	tar czf $(DISTARCHIVE) -C $(BUILD) peercast-yt

appimage: all $(APPIMAGETOOL) $(LINUXDEPLOY)
	rm -rf $(BUILD)/AppDir
	$(MAKE) install PREFIX=/usr DESTDIR=$(abspath $(BUILD)/AppDir)
	cd $(BUILD) && ./$(notdir $(LINUXDEPLOY)) --appdir=AppDir -e AppDir/usr/bin/peercast --deploy-deps-only=AppDir/usr/bin/rtmp-server \
	  -d ../ui/linux/peercast.desktop -i ../ui/linux/peercast.png
	cd $(BUILD) && ./$(notdir $(APPIMAGETOOL)) AppDir

$(APPIMAGETOOL):
	mkdir -p $(BUILD)
	wget -O $@ https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-$(ARCH).AppImage
	chmod +x $@

$(LINUXDEPLOY):
	mkdir -p $(BUILD)
	wget -O $@ https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$(ARCH).AppImage
	chmod +x $@

clean:
	$(CARGO) clean
	rm -rf $(DISTDIR) $(DISTARCHIVE) $(BUILD)/AppDir $(APPIMAGE)
