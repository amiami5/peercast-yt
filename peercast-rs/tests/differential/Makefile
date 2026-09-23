# C++ 版と Rust 版の差分テスト。使い方: make && ./diff
#
# C++ 版は WITH_RUST_CORE なしでコンパイルした cgi.cpp と str.cpp (従来の実装そのもの)、
# Rust 版は libpeercast_rs.a をそのまま使う。

CORE = ../../../core/common
RUST = ../..
# -fsigned-char: C++ 版には char の符号で結果が変わる箇所がある。Rust 版は CPU によらず
# 符号付きとして扱うので、比較の基準も CPU によらず符号付きにそろえる (ARM でも同じ結果になる)。
CXXFLAGS = -O1 -g -std=c++11 -fsigned-char -D_UNIX -I$(CORE) -I../../../core/unix -I$(RUST)/include -Wall -Wno-unused-function

diff: diff.cpp cgi.o str.o regexp.o common.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

%.o: $(CORE)/%.cpp
	$(CXX) $(CXXFLAGS) -c $< -o $@

$(RUST)/target/release/libpeercast_rs.a: $(wildcard $(RUST)/src/*.rs) $(RUST)/Cargo.toml
	cargo build --release --manifest-path $(RUST)/Cargo.toml

clean:
	rm -f diff *.o

diff_strutil: diff_strutil.cpp cgi.o str.o regexp.o common.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

diff_md5_gnuid: diff_md5_gnuid.cpp md5.o gnuid.o regexp.o common.o str.o cgi.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

diff_jis: diff_jis.cpp jis.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl

diff_string: diff_string.cpp _string.o jis.o $(RUST)/target/release/libpeercast_rs.a
	$(CXX) $(CXXFLAGS) $^ -o $@ -pthread -ldl
