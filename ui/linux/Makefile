# Uncomment following line to enabled RTMP fetch support
WITH_RTMP = yes

# Use the Rust implementation (needs cargo).  See docs/rust-migration.md.
#   make WITH_RUST=no    : build everything from the original C++ code
# The parts can also be switched individually:
#   WITH_RUST_RTMP       : rtmp-server (../../rtmp-server-rs instead of ../../rtmp-server)
#   WITH_RUST_CORE       : parts of the core library (../../peercast-rs, linked into peercast)
# Change them on the command line or in Makefile.local.  Run "make clean"
# when switching, since the output file names are the same.
WITH_RUST = yes
WITH_RUST_RTMP = $(WITH_RUST)
WITH_RUST_CORE = $(WITH_RUST)
RUST_RTMP = ../../rtmp-server-rs
RUST_CORE = ../../peercast-rs
RUST_CORE_LIB = $(RUST_CORE)/target/release/libpeercast_rs.a
CARGO ?= cargo

CPPFLAGS = -g -Wall -std=c++11 -D_UNIX -D_REENTRANT -DADD_BACKTRACE $(INCLUDES) $(shell pkg-config openssl --cflags)
ifeq ($(WITH_RTMP),yes)
  CPPFLAGS += -DWITH_RTMP
endif

LDFLAGS = -fuse-ld=gold -pthread -rdynamic
LIBS = $(shell pkg-config openssl --libs)
ifeq ($(WITH_RTMP),yes)
  LIBS += -lrtmp
endif

OS = $(shell uname -s | tr A-Z a-z)
ARCH = $(shell uname -m | tr A-Z a-z)

DISTARCHIVE = peercast-yt-$(OS)-$(ARCH).tar.gz
APPIMAGE = Peercast_YT-$(ARCH).AppImage
APPIMAGETOOL = appimagetool-$(ARCH).AppImage
LINUXDEPLOY = linuxdeploy-$(ARCH).AppImage
AR = ar

INCLUDES = -I../../core/common -I../../core/unix

ifeq ($(WITH_RUST_CORE),yes)
  CPPFLAGS += -DWITH_RUST_CORE
  INCLUDES += -I$(RUST_CORE)/include
  LIBS += $(RUST_CORE_LIB)
  RUST_CORE_DEP = $(RUST_CORE_LIB)
endif

TARGET = peercast
RTMPSERVER = rtmp-server
CORE = ../../core
RTMP = ../../rtmp-server
SRCHTMLTAR = ../html.tar
HTML = html
SRCPUBLICTAR = ../public.tar
PUBLIC = public
ASSETS = ../assets/
CGIBIN = ../cgi-bin

LIBOBJSTATIC = libpeercast.a

SYSSOURCE = $(wildcard $(CORE)/unix/*.cpp)

CORESOURCE = $(wildcard $(CORE)/common/*.cpp) 

COREOBJ = $(notdir $(CORESOURCE:.cpp=.o))
SYSOBJ = $(notdir $(SYSSOURCE:.cpp=.o))

LINUXSOURCE = main.cpp
LINUXOBJ = $(notdir $(LINUXSOURCE:.cpp=.o))

RTMPSOURCE = $(wildcard $(RTMP)/*.cpp)
RTMPOBJ = $(notdir $(RTMPSOURCE:.cpp=.o))

DEPENDFILE = .deps

PREFIX ?= /usr/local
bindir=$(PREFIX)/bin
docdir=$(PREFIX)/share/doc/peercast
sharedir=$(PREFIX)/share/peercast

-include Makefile.local

.PHONY: all clean build-html deps $(SRCHTMLTAR) $(SRCPUBLICTAR)

# Here follow the generic build rules.
all: $(TARGET) $(LIBOBJSTATIC) $(DISTARCHIVE) $(RTMPSERVER)

deps:
	$(CXX) $(CPPFLAGS) -MM $(CORESOURCE) $(SYSSOURCE) $(LINUXSOURCE) $(RTMPSOURCE) > $(DEPENDFILE)

$(SRCHTMLTAR):
	$(MAKE) -C .. html.tar

$(HTML): $(SRCHTMLTAR)
	rm -rf $(HTML)
	tar xf $(SRCHTMLTAR)
	touch $(HTML)

$(SRCPUBLICTAR):
	$(MAKE) -C .. public.tar

$(PUBLIC): $(SRCPUBLICTAR)
	rm -rf $(PUBLIC)
	tar xf $(SRCPUBLICTAR)
	touch $(PUBLIC)

$(DISTARCHIVE): $(TARGET) $(HTML) $(PUBLIC) $(ASSETS) $(RTMPSERVER)
	rm -rf peercast-yt
	mkdir peercast-yt
	cp -R $(HTML) $(PUBLIC) $(TARGET) $(ASSETS) $(CGIBIN) $(RTMPSERVER) ../../licenses ../../core/LICENSE peercast-yt/
	strip peercast-yt/$(TARGET)
	strip peercast-yt/$(RTMPSERVER)
	tar czf $(DISTARCHIVE) peercast-yt

install:
	mkdir -p $(bindir)
	install -s $(TARGET) $(RTMPSERVER) $(bindir)
	mkdir -p $(docdir)
	cp -R ../../licenses ../../core/LICENSE $(docdir)
	mkdir -p $(sharedir)
	cp -R $(HTML) $(PUBLIC) $(ASSETS) $(CGIBIN) $(sharedir)
	mkdir -p $(PREFIX)/share/applications/
	cp peercast.desktop $(PREFIX)/share/applications/
	mkdir -p $(PREFIX)/share/pixmaps/
	cp peercast.png $(PREFIX)/share/pixmaps/

$(APPIMAGE): $(APPIMAGETOOL) $(LINUXDEPLOY) $(TARGET) $(RTMPSERVER) peercast.desktop peercast.png
	rm -rf AppDir/
	PREFIX=AppDir/usr make install
	./$(LINUXDEPLOY) --appdir=AppDir -e AppDir/usr/bin/peercast  --deploy-deps-only="AppDir/usr/bin/rtmp-server" -d peercast.desktop -i peercast.png
	./$(APPIMAGETOOL) AppDir

$(APPIMAGETOOL):
	if [ ! -f $(APPIMAGETOOL) ]; then wget https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-$(ARCH).AppImage; chmod +x $(APPIMAGETOOL); fi

$(LINUXDEPLOY):
	if [ ! -f $(LINUXDEPLOY) ]; then wget https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$(ARCH).AppImage; chmod +x $(LINUXDEPLOY); fi

clean:
	$(MAKE) -C .. clean
	rm -f $(LINUXOBJ) $(COREOBJ) $(SYSOBJ) $(TARGET) ${LIBOBJSTATIC} $(COREOBJSTATIC) $(DEPENDFILE) $(RTMPSERVER) $(RTMPOBJ)
	rm -f $(ALLOBJ:.o=.d)
	rm -rf $(HTML) $(PUBLIC)
	rm -rf $(RUST_RTMP)/target $(RUST_CORE)/target
	rm -rf peercast-yt/
	rm -f $(DISTARCHIVE)

# ヘッダーの依存関係 (*.d) をコンパイルのついでに書き出し、ヘッダーが変わったら作り直す。
# Makefile (フラグ) が変わったときも全部作り直す。
DEPFLAGS = -MMD -MP
ALLOBJ = $(COREOBJ) $(SYSOBJ) $(LINUXOBJ) $(RTMPOBJ)
$(ALLOBJ): Makefile

$(COREOBJ): %.o : $(CORE)/common/%.cpp
	$(CXX) $(CPPFLAGS) $(DEPFLAGS) -c $< -o $@

$(SYSOBJ): %.o : $(CORE)/unix/%.cpp
	$(CXX) $(CPPFLAGS) $(DEPFLAGS) -c $< -o $@

$(RTMPOBJ): %.o : $(RTMP)/%.cpp
	$(CXX) $(CPPFLAGS) $(DEPFLAGS) -c $< -o $@

$(LINUXOBJ): %.o : %.cpp
	$(CXX) $(CXXFLAGS) $(CPPFLAGS) $(DEPFLAGS) -c $< -o $@

$(LIBOBJSTATIC): $(COREOBJ) $(SYSOBJ)
	$(AR) rcs $(LIBOBJSTATIC) $(COREOBJ) $(SYSOBJ)

$(TARGET): $(LINUXOBJ) $(LIBOBJSTATIC) $(RUST_CORE_DEP)
	$(CXX) $(LDFLAGS) -o $@ $(LINUXOBJ) $(LIBOBJSTATIC) $(LIBS)

define CHECK_CARGO
	@command -v $(CARGO) >/dev/null 2>&1 || { \
	  echo "error: '$(CARGO)' not found. Install Rust (e.g. 'sudo apt install cargo')," >&2; \
	  echo "       or build without Rust: make WITH_RUST=no" >&2; \
	  exit 1; }
endef

$(RUST_CORE_LIB): $(RUST_CORE)/Cargo.toml $(wildcard $(RUST_CORE)/src/*.rs $(RUST_CORE)/src/*/*.rs) $(wildcard $(RUST_CORE)/include/*.h)
	$(CHECK_CARGO)
	$(CARGO) build --release --manifest-path $(RUST_CORE)/Cargo.toml

ifeq ($(WITH_RUST_RTMP),yes)
# Rust version: no dependency on the C++ core library or OpenSSL.
$(RTMPSERVER): $(RUST_RTMP)/Cargo.toml $(wildcard $(RUST_RTMP)/src/*.rs)
	$(CHECK_CARGO)
	$(CARGO) build --release --manifest-path $(RUST_RTMP)/Cargo.toml
	cp $(RUST_RTMP)/target/release/rtmp-server $@
else
$(RTMPSERVER): $(RTMPOBJ) $(LIBOBJSTATIC) $(RUST_CORE_DEP)
	$(CXX) $(LDFLAGS) -o $@ $(RTMPOBJ) $(LIBOBJSTATIC) $(LIBS)
endif

-include $(DEPENDFILE)
-include $(ALLOBJ:.o=.d)
