# C++ 版と Rust 版の差分テスト。使い方: make && ./diff
#
# C++ 版は WITH_RUST_CORE なしでコンパイルした cgi.cpp と str.cpp (従来の実装そのもの)、
# Rust 版は libpeercast_rs.a をそのまま使う。

CORE = ../../../core/common
RUST = ../..
# -fsigned-char: C++ 版には char の符号で結果が変わる箇所がある。Rust 版は CPU によらず
# 符号付きとして扱うので、比較の基準も CPU によらず符号付きにそろえる (ARM でも同じ結果になる)。
# -ftrivial-auto-var-init=zero: C++ 版が初期化していないスタックの値を読む箇所 (読み出しが足りなかった
# ときのバッファの残りなど) を 0 にそろえる。Rust 版はそこを 0 として扱うので、比べられるようになる。
CXXFLAGS = -O1 -g -std=c++11 -fsigned-char -ftrivial-auto-var-init=zero -D_UNIX -I$(CORE) -I../../../core/unix -I$(RUST)/include -Wall -Wno-unused-function

diff: diff.cpp cgi.o str.o regexp.o common.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

%.o: $(CORE)/%.cpp
	$(CXX) $(CXXFLAGS) -c $< -o $@

$(RUST)/target/release/libpeercast_rs.a: $(wildcard $(RUST)/src/*.rs $(RUST)/src/*/*.rs) $(RUST)/Cargo.toml
	cargo build --release --manifest-path $(RUST)/Cargo.toml

clean:
	rm -f diff diff_strutil diff_md5_gnuid diff_jis diff_string diff_http diff_amf0_dechunk diff_xml diff_url diff_media diff_template *.o cxxcore.a

diff_strutil: diff_strutil.cpp cgi.o str.o regexp.o common.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

diff_md5_gnuid: diff_md5_gnuid.cpp md5.o gnuid.o regexp.o common.o str.o cgi.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

diff_jis: diff_jis.cpp jis.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

diff_string: diff_string.cpp _string.o jis.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

# Rust を使わない C++ のコア一式 (core/common と core/unix)。クラスごと比べたいときに使う。
UNIX = ../../../core/unix
CXXCORE_OBJS = $(patsubst $(CORE)/%.cpp,%.o,$(filter-out $(CORE)/rustcore.cpp,$(wildcard $(CORE)/*.cpp))) \
               strerror.o usocket.o usubprog.o usys.o
CXXCORE_LIBS = $(shell pkg-config openssl --libs) -lrtmp -pthread -ldl

%.o: $(UNIX)/%.cpp
	$(CXX) $(CXXFLAGS) $(shell pkg-config openssl --cflags) -c $< -o $@

cxxcore.a: $(CXXCORE_OBJS)
	rm -f $@
	$(AR) rcs $@ $^

diff_http: diff_http.cpp cxxcore.a $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ $(CXXCORE_LIBS)

diff_amf0_dechunk: diff_amf0_dechunk.cpp cxxcore.a $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ $(CXXCORE_LIBS)

diff_xml: diff_xml.cpp cxxcore.a $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ $(CXXCORE_LIBS)

diff_url: diff_url.cpp cxxcore.a $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ $(CXXCORE_LIBS)

# Channel::newPacket と updateInfo (記録する) と LOG_* (捨てる) を横取りする
MEDIA_WRAP = -Wl,--wrap=_ZN7Channel9newPacketER10ChanPacket -Wl,--wrap=_ZN7Channel10updateInfoERK8ChanInfo \
             -Wl,--wrap=_Z9LOG_TRACEPKcz -Wl,--wrap=_Z9LOG_DEBUGPKcz -Wl,--wrap=_Z8LOG_INFOPKcz \
             -Wl,--wrap=_Z8LOG_WARNPKcz -Wl,--wrap=_Z9LOG_ERRORPKcz

diff_media: diff_media.cpp cxxcore.a $(RUST)/target/release/libpeercast_rs.a $(CORE)/rustmedia.h $(CORE)/rustbridge.h
	$(CXX) $(CXXFLAGS) $(filter-out %.h,$^) -o $@ $(MEDIA_WRAP) $(CXXCORE_LIBS)

diff_template: diff_template.cpp cxxcore.a $(RUST)/target/release/libpeercast_rs.a $(CORE)/rusttemplate.h $(CORE)/rustbridge.h
	$(CXX) $(CXXFLAGS) $(filter-out %.h,$^) -o $@ $(CXXCORE_LIBS)
