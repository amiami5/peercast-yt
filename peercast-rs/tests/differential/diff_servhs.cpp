// 段階8b: HTTP の要求の処理 (core/common/servhs.cpp) のうち、Rust に移した解釈と判断の、C++ 版と
// Rust 版の差分テスト。
//
// C++ 版は、外から呼べるものは Rust を使わずにビルドしたコア一式 (cxxcore.a) の関数 (nextCGIarg、
// Servent::hasValidAuthToken、Servent::fileNameToMimeType、ServMgr::isValidHtmlPath、stristr など)、
// Servent のメソッドの途中にあるものは、servhs.cpp の元のコードをこのファイルにそのまま写したもの。
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <random>
#include <string>
#include <vector>

#include "cgi.h"
#include "chanmgr.h"
#include "http.h"
#include "peercast_rs.h"
#include "regexp.h"
#include "rustbridge.h"
#include "servent.h"
#include "servmgr.h"
#include "str.h"
#include "../../../tests/mockpeercast.h"

static long g_cases = 0, g_bad = 0;
static std::mt19937 rng(20260928);
static unsigned R(unsigned n) { return rng() % n; }

extern "C" void __wrap__Z9LOG_TRACEPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_INFOPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_WARNPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_ERRORPKcz(const char*, ...) {}

static std::string show(const std::string& s)
{
    std::string r;
    char b[8];
    for (unsigned char c : s)
    {
        if (c >= 0x20 && c < 0x7f && c != '\\') r += (char)c;
        else { snprintf(b, sizeof b, "\\x%02x", c); r += b; }
    }
    return r;
}

static void report(const char* what, const std::string& in, const std::string& a, const std::string& b)
{
    g_cases++;
    if (a != b && g_bad++ < 30)
        printf("  [違い] %s %s\n    C++ =%s\n    Rust=%s\n", what, show(in.substr(0, 600)).c_str(),
               show(a.substr(0, 600)).c_str(), show(b.substr(0, 600)).c_str());
}

static const uint8_t* u8(const char* s) { return reinterpret_cast<const uint8_t*>(s); }
static std::string take(pcrs_buf b) { return rustbridge::RustBuf(b).str(); }

// ---------------------------------------------------------------- 入力

static std::string randStr(int maxLen = 60)
{
    static const char* toks[] = {
        "GET /", "POST /", "GIV", "pcp", "SOURCE", " ", "/", "?", "&", "=", ";", "; ", ":", "%", "%2F", "%41", "%%",
        "HTTP/1.", "HTTP/1.1", "http/1.0", "ICE/1.0", "/admin?", "/admin/?", "/html/", "/html/index.html", "/admin.cgi",
        "/pls/", "/stream/", "/channel/", "/api/1", "/public", "/public/", "/assets/", "/cgi-bin/", "/cgi-bin/flv.cgi",
        "/cmd?", "pass=", "song=", "mount=", "url=", "id=", "auth=", "cmd=", "html/", "html/ja", "ja", "en", "7144_id=",
        "8000_id", ".htm", ".html", ".css", ".jpg", ".gif", ".png", ".js", ".ico", ".HTM", "/play.html", "/relayinfo.html",
        "/head.html", "connections.html", "editinfo.html", "icy-name", "ICY-BR", "ice-url", "x-audiocast-genre",
        "ice-description", "Authorization", "x-peercast-channelid:", "ice-password", "content-type", "audio/mpeg",
        "application/ogg", "audio/x-mpegurl", "application/x-peercast-pcp", "text/plain", "application/binary",
        "Content-Type", "Status", "Location", "localhost", "127.0.0.1", "[::1]", "-", "_", ".", "0", "1", "9", "12", "-1",
        "4294967297", "99999999999999999999", " +", "a", "Z", "http://", "https://", "servername", "icymeta", "htmlPath",
        "filt_ip", "filt_bn", "filt_pr", "filt_nw", "filt_di", "filt_", "auth", "cookie", "expire", "never", "session",
        "channel_feed_url", "allowHTML1", "yp", "port", "logLevel", "\t", "\r", "\n", "\xe3\x81\x82", "\xff",
        "0123456789ABCDEF0123456789abcdef",
    };
    std::string s;
    int n = R(maxLen / 4 + 1);
    for (int i = 0; i < n && (int) s.size() < maxLen; i++)
    {
        if (R(5) == 0)
            s += (char)(1 + R(255));   // NUL は含めない (C の文字列)
        else
            s += toks[R(sizeof toks / sizeof toks[0])];
    }
    return s;
}

// ---------------------------------------------------------------- 写したもの (servhs.cpp の元のコード)

// cgi::unescape は、C++ 版は不正な %XX で初期化していない値を使う (段階 1 の既知の違い)。WITH_RUST_CORE の
// ビルドと同じく Rust 版を使って、ここで比べたいもの (servhs の解釈と判断) だけを比べる。
static std::string unesc(const std::string& s)
{
    return take(pcrs_cgi_unescape(u8(s.data()), s.size()));
}

// cgi::Query (cgi.cpp) を写したもの。unescape だけ上のものにした
static std::string queryGet(const std::string& queryString, const std::string& key)
{
    std::map<std::string, std::vector<std::string>> dict;
    auto assignments = str::split(queryString, "&");
    for (auto& assignment : assignments)
    {
        if (assignment.empty())
            continue;
        auto sides = str::split(assignment, "=");
        if (sides.size() == 1)
            dict[sides[0]] = {};
        else
            dict[sides[0]].push_back(unesc(sides[1]));
    }
    auto it = dict.find(key);
    if (it == dict.end() || it->second.size() == 0)
        return "";
    return it->second[0];
}


static bool isRequest(const char* cmdLine, const char* rq) { return strncmp(cmdLine, rq, strlen(rq)) == 0; }

static int cxx_request_kind(const char* cmdLine, const char* password)
{
    if (isRequest(cmdLine, "GET /")) return PCRS_REQ_GET;
    else if (isRequest(cmdLine, "POST /")) return PCRS_REQ_POST;
    else if (isRequest(cmdLine, "GIV")) return PCRS_REQ_GIV;
    else if (isRequest(cmdLine, "pcp")) return PCRS_REQ_PCP;
    else if (isRequest(cmdLine, "SOURCE")) return PCRS_REQ_SOURCE;
    else if (password[0] != '\0' && isRequest(cmdLine, password)) return PCRS_REQ_SHOUTCAST;
    else return PCRS_REQ_BAD;
}

// handshakeGET の振り分け。fn の後ろの状態 (NUL を書いたあと) も返す
static std::string cxx_get_route(std::string line)
{
    std::vector<char> cmdLine(line.begin(), line.end());
    cmdLine.resize(cmdLine.size() + 8, 0);
    char* fn = cmdLine.data() + 4;

    char *pt = strstr(fn, HTTP_PROTO1);
    if (pt)
        pt[-1] = 0;

    int route;
    if (strncmp(fn, "/admin?", 7) == 0) route = PCRS_GET_ADMIN;
    else if (strncmp(fn, "/admin/?", 8) == 0) route = PCRS_GET_ADMIN_SLASH;
    else if (strcmp(fn, "/html/index.html") == 0) route = PCRS_GET_HTML_INDEX;
    else if (strncmp(fn, "/html/", 6) == 0) route = PCRS_GET_HTML;
    else if (strncmp(fn, "/admin.cgi", 10) == 0) route = PCRS_GET_ADMIN_CGI;
    else if (strncmp(fn, "/pls/", 5) == 0) route = PCRS_GET_PLS;
    else if (strncmp(fn, "/stream/", 8) == 0) route = PCRS_GET_STREAM;
    else if (strncmp(fn, "/channel/", 9) == 0) route = PCRS_GET_CHANNEL;
    else if (strcmp(fn, "/api/1") == 0) route = PCRS_GET_API1;
    else if (strcmp(fn, "/public")== 0 || strncmp(fn, "/public/", strlen("/public/"))==0) route = PCRS_GET_PUBLIC;
    else if (str::is_prefix_of("/assets/", fn)) route = PCRS_GET_ASSETS;
    else if (str::is_prefix_of("/cgi-bin/", fn))
        route = str::has_prefix(fn, "/cgi-bin/flv.cgi") ? PCRS_GET_CGI_BIN_FLV : PCRS_GET_CGI_BIN;
    else if (str::is_prefix_of("/cmd?", fn)) route = PCRS_GET_CMD;
    else route = PCRS_GET_OTHER;
    return std::to_string(route) + " " + std::string(cmdLine.data(), line.size());
}

static std::string rs_get_route(std::string line)
{
    std::vector<char> cmdLine(line.begin(), line.end());
    cmdLine.resize(cmdLine.size() + 8, 0);
    char* fn = cmdLine.data() + 4;
    bool hasCut = false;
    ptrdiff_t cut = 0;
    int route = pcrs_servhs_get_route(u8(fn), strlen(fn), &hasCut, &cut);
    if (hasCut)
        fn[cut] = 0;
    return std::to_string(route) + " " + std::string(cmdLine.data(), line.size());
}

static void termArgs(char *str)
{
    if (str)
    {
        int slen = strlen(str);
        for (int i=0; i<slen; i++)
            if (str[i]=='&') str[i] = 0;
    }
}

static std::string cxx_admin_cgi(std::string s)
{
    std::vector<char> buf(s.begin(), s.end());
    buf.push_back(0);
    char* fn = buf.data();
    const char *pwdArg = getCGIarg(fn, "pass=");
    const char *songArg = getCGIarg(fn, "song=");
    const char *mountArg = getCGIarg(fn, "mount=");
    const char *urlArg = getCGIarg(fn, "url=");
    if (!(pwdArg && songArg))
        return "none";
    termArgs(fn);
    return std::string("song=") + songArg + " mount=" + (mountArg ? std::string("1:") + mountArg : "0") +
           " url=" + (urlArg ? std::string("1:") + urlArg : "0");
}

static std::string rs_admin_cgi(const std::string& s)
{
    pcrs_buf song, mount, url;
    bool hasMount, hasUrl;
    bool ok = pcrs_servhs_admin_cgi(u8(s.c_str()), s.size(), &song, &hasMount, &mount, &hasUrl, &url);
    std::string a = take(song), m = take(mount), u = take(url);
    if (!ok)
        return "none";
    return "song=" + a + " mount=" + (hasMount ? "1:" + m : "0") + " url=" + (hasUrl ? "1:" + u : "0");
}

static std::string cxx_post_route(const char* cmdLine)
{
    auto vec = str::split(cmdLine, " ");
    if (vec.size() != 3)
        return "400";
    std::string args;
    auto vec2 = str::split(vec[1], "?", 2);
    if (vec2.size() == 2)
        args = vec2[1];
    std::string path = vec2[0];
    int route = path == "/api/1" ? PCRS_POST_API1 : path == "/" ? PCRS_POST_PUSH : path == "/admin" ? PCRS_POST_ADMIN : PCRS_POST_OTHER;
    return std::to_string(route) + " " + args;
}

static std::string rs_post_route(const std::string& s)
{
    pcrs_buf args;
    int r = pcrs_servhs_post_route(u8(s.c_str()), s.size(), &args);
    std::string a = take(args);
    if (r < 0)
        return "400";
    return std::to_string(r) + " " + a;
}

// handshakeSOURCE。行の前に NUL を 1 つ置き、後ろは 0 で埋めた (C++ 版が読むのは 0) バッファの上で動かす
static std::string cxx_source(const std::string& line)
{
    std::vector<char> buf(line.size() + 32, 0);
    memcpy(buf.data() + 1, line.data(), line.size());
    char* in = buf.data() + 1;
    char *mount = nullptr;
    std::string password = "(none)";
    char *ps;
    if ((ps = strstr(in, "ICE/1.0")) != nullptr)
    {
        mount = in+7;
        *ps = 0;
    }else{
        mount = in+strlen(in);
        while (*--mount)
            if (*mount == '/')
            {
                mount[-1] = 0; // password preceeds
                break;
            }
        String loginPassword;
        loginPassword.set(in+7);
        password = loginPassword.cstr();
    }
    String loginMount;
    if (mount)
        loginMount.set(mount);
    return password + " | " + loginMount.cstr();
}

static std::string rs_source(const std::string& line)
{
    pcrs_buf pw, mnt;
    bool icy = pcrs_servhs_source(u8(line.c_str()), line.size(), &pw, &mnt);
    std::string p = take(pw), m = take(mnt);
    std::string password = "(none)";
    if (icy)
    {
        String loginPassword;
        loginPassword.set(p.c_str());
        password = loginPassword.cstr();
    }
    String loginMount;
    loginMount.set(m.c_str());
    return password + " | " + loginMount.cstr();
}

static std::string cxx_cookie(const std::string& arg, unsigned short port)
{
    const std::string idKey = str::STR(port, "_id");
    auto assignments = str::split(arg, "; ");
    for (auto assignment : assignments) {
        auto sides = str::split(assignment, "=", 2);
        if (sides.size() != 2) {
            return "invalid";
        } else if (sides[0] == idKey) {
            return "found " + sides[1];
        }
    }
    return "none";
}

static std::string rs_cookie(const std::string& arg, unsigned short port)
{
    pcrs_buf id;
    int r = pcrs_servhs_cookie_id(u8(arg.data()), arg.size(), port, &id);
    std::string v = take(id);
    return r == 2 ? "invalid" : r == 1 ? "found " + v : "none";
}

// CMD_apply の繰り返しの中身。状態を変える代わりに、何をするか (と値) を書く
static std::string cxx_apply(const char* cmd)
{
    std::string out;
    auto op = [&](int key, int v, const std::string& s) { out += std::to_string(key) + ":" + std::to_string(v) + ":" + s + "|"; };
    char arg[MAX_CGI_LEN];
    char curr[MAX_CGI_LEN];
    const char *cp = cmd;
    while ((cp = nextCGIarg(cp, curr, arg)) != nullptr)
    {
        if (strcmp(curr, "servername") == 0) op(PCRS_APPLY_SERVER_NAME, 0, unesc(arg));
        else if (strcmp(curr, "serveractive") == 0) op(PCRS_APPLY_SERVER_ACTIVE, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "port") == 0) op(PCRS_APPLY_PORT, atoi(arg), "");
        else if (strcmp(curr, "icymeta") == 0)
        {
            int iv = atoi(arg);
            if (iv < 0) iv = 0;
            else if (iv > 16384) iv = 16384;
            op(PCRS_APPLY_ICY_META, iv, "");
        }else if (strcmp(curr, "passnew") == 0) op(PCRS_APPLY_PASS_NEW, 0, arg);
        else if (strcmp(curr, "root") == 0) op(PCRS_APPLY_ROOT, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "brroot") == 0) op(PCRS_APPLY_BR_ROOT, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "getupd") == 0) op(PCRS_APPLY_GET_UPD, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "huint") == 0) op(PCRS_APPLY_HU_INT, atoi(arg), "");
        else if (strcmp(curr, "forceip") == 0) op(PCRS_APPLY_FORCE_IP, 0, arg);
        else if (strcmp(curr, "htmlPath") == 0)
        {
            const std::string newPath = std::string("html/") + arg;
            op(PCRS_APPLY_HTML_PATH, ServMgr::isValidHtmlPath(newPath), newPath);
        }else if (strcmp(curr, "djmsg") == 0) op(PCRS_APPLY_DJ_MSG, 0, unesc(arg));
        else if (strcmp(curr, "pcmsg") == 0) op(PCRS_APPLY_PC_MSG, 0, unesc(arg));
        else if (strcmp(curr, "maxcin") == 0) op(PCRS_APPLY_MAX_CIN, atoi(arg), "");
        else if (strcmp(curr, "maxsin") == 0) op(PCRS_APPLY_MAX_SIN, atoi(arg), "");
        else if (strcmp(curr, "maxup") == 0) op(PCRS_APPLY_MAX_UP, atoi(arg), "");
        else if (strcmp(curr, "maxrelays") == 0) op(PCRS_APPLY_MAX_RELAYS, atoi(arg), "");
        else if (strcmp(curr, "maxdirect") == 0) op(PCRS_APPLY_MAX_DIRECT, atoi(arg), "");
        else if (strcmp(curr, "maxrelaypc") == 0) op(PCRS_APPLY_MAX_RELAY_PC, atoi(arg), "");
        else if (strncmp(curr, "filt_", 5) == 0)
        {
            char *fs = curr+5;
            if (strncmp(fs, "ip", 2) == 0) op(PCRS_APPLY_FILT_IP, 0, unesc(arg));
            else if (strncmp(fs, "bn", 2) == 0) op(PCRS_APPLY_FILT_BAN, 0, "");
            else if (strncmp(fs, "pr", 2) == 0) op(PCRS_APPLY_FILT_PRIVATE, 0, "");
            else if (strncmp(fs, "nw", 2) == 0) op(PCRS_APPLY_FILT_NETWORK, 0, "");
            else if (strncmp(fs, "di", 2) == 0) op(PCRS_APPLY_FILT_DIRECT, 0, "");
        }
        else if (strcmp(curr, "channel_feed_url") == 0)
        {
            if (strcmp(arg, "") != 0) op(PCRS_APPLY_CHANNEL_FEED_URL, 0, unesc(arg));
        }
        else if (strcmp(curr, "clientactive") == 0) op(PCRS_APPLY_CLIENT_ACTIVE, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "yp") == 0) op(PCRS_APPLY_YP, 0, unesc(arg));
        else if (strcmp(curr, "deadhitage") == 0) op(PCRS_APPLY_DEAD_HIT_AGE, atoi(arg), "");
        else if (strcmp(curr, "refresh") == 0) op(PCRS_APPLY_REFRESH, atoi(arg), "");
        else if (strcmp(curr, "chat") == 0) op(PCRS_APPLY_CHAT, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "randomizechid") == 0) op(PCRS_APPLY_RANDOMIZE_CHID, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "public_directory") == 0) op(PCRS_APPLY_PUBLIC_DIRECTORY, 1, "");
        else if (strcmp(curr, "auth") == 0)
        {
            if (strcmp(arg, "cookie") == 0) op(PCRS_APPLY_AUTH, 1, "");
            else if (strcmp(arg, "http") == 0) op(PCRS_APPLY_AUTH, 2, "");
        }else if (strcmp(curr, "expire") == 0)
        {
            if (strcmp(arg, "session") == 0) op(PCRS_APPLY_EXPIRE, 0, "");
            else if (strcmp(arg, "never") == 0) op(PCRS_APPLY_EXPIRE, 1, "");
        }
        else if (strcmp(curr, "logLevel") == 0) op(PCRS_APPLY_LOG_LEVEL, atoi(arg), "");
        else if (strcmp(curr, "allowHTML1") == 0) op(PCRS_APPLY_ALLOW_HTML, atoi(arg) ? 1 : 0, "");
        else if (strcmp(curr, "allowNetwork1") == 0) op(PCRS_APPLY_ALLOW_NETWORK, atoi(arg) ? 1 : 0, "");
        else if (strcmp(curr, "allowBroadcast1") == 0) op(PCRS_APPLY_ALLOW_BROADCAST, atoi(arg) ? 1 : 0, "");
        else if (strcmp(curr, "allowDirect1") == 0) op(PCRS_APPLY_ALLOW_DIRECT, atoi(arg) ? 1 : 0, "");
        else if (strcmp(curr, "transcoding_enabled") == 0) op(PCRS_APPLY_TRANSCODING, strcmp(arg, "1") == 0, "");
        else if (strcmp(curr, "preset") == 0) op(PCRS_APPLY_PRESET, 0, arg);
        else if (strcmp(curr, "audio_codec") == 0) op(PCRS_APPLY_AUDIO_CODEC, 0, arg);
        else if (strcmp(curr, "preferredTheme") == 0) op(PCRS_APPLY_PREFERRED_THEME, 0, arg);
        else if (strcmp(curr, "accentColor") == 0) op(PCRS_APPLY_ACCENT_COLOR, 0, arg);
    }
    return out;
}

static std::string rs_apply(const char* cmd)
{
    std::string out;
    pcrs_servhs_apply_ops(u8(cmd), strlen(cmd), &out, [](void* c, int key, int32_t v, const uint8_t* s, size_t len) {
        *static_cast<std::string*>(c) += std::to_string(key) + ":" + std::to_string(v) + ":" +
                                         std::string(reinterpret_cast<const char*>(s), len) + "|";
    });
    return out;
}

static std::string cxx_redirect(const char* cmd)
{
    char buf[MAX_CGI_LEN];
    Sys::strcpy_truncate(buf, sizeof(buf), cmd);
    const char *j = getCGIarg(buf, "url=");
    if (!j)
        return "400";
    termArgs(buf);
    String url;
    url.set(j, String::T_ESC);
    url.convertTo(String::T_ASCII);
    if (!url.startsWith("http://") && !url.startsWith("https://"))
        url.prepend("http://");
    return url.cstr();
}

static std::string rs_redirect(const char* cmd)
{
    char buf[MAX_CGI_LEN];
    Sys::strcpy_truncate(buf, sizeof(buf), cmd);
    pcrs_buf out;
    bool ok = pcrs_servhs_redirect_url(u8(buf), strlen(buf), &out);
    std::string u = take(out);
    return ok ? std::string(u.c_str()) : "400";
}

static std::string cxx_referer(std::string referer, const std::string& newHtmlPath)
{
    auto vec = Regexp("html/[^/]+").exec(referer);
    if (vec.size())
    {
        auto pos = referer.find(vec[0]);
        if (pos != std::string::npos)
            return referer.substr(0, pos) + newHtmlPath + referer.substr(pos + vec[0].size());
        return "(npos)";
    }
    return "(none)";
}

static std::string rs_referer(const std::string& referer, const std::string& path)
{
    pcrs_buf out;
    bool ok = pcrs_servhs_rewrite_referer(u8(referer.data()), referer.size(), u8(path.data()), path.size(), &out);
    std::string r = take(out);
    return ok ? r : "(none)";
}

static bool cxx_is_decimal(const std::string& str)
{
    static const Regexp decimal("^(0|[1-9][0-9]*)$");
    return decimal.matches(str);
}

// readICYHeader。何をするかを書く
static std::string cxx_icy(const char* cmdLine, const char* arg)
{
    auto isHeader = [&](const char* hs) { return stristr(cmdLine, hs) != nullptr; };
    if (isHeader("x-audiocast-name") || isHeader("icy-name") || isHeader("ice-name")) return "name";
    else if (isHeader("x-audiocast-url") || isHeader("icy-url") || isHeader("ice-url")) return "url";
    else if (isHeader("x-audiocast-bitrate") || (isHeader("icy-br")) || isHeader("ice-bitrate") || isHeader("icy-bitrate")) return "bitrate";
    else if (isHeader("x-audiocast-genre") || isHeader("ice-genre") || isHeader("icy-genre")) return "genre";
    else if (isHeader("x-audiocast-description") || isHeader("ice-description")) return "desc";
    else if (isHeader("Authorization")) return "auth";
    else if (isHeader(PCX_HS_CHANNELID)) return "id";
    else if (isHeader("ice-password")) return "password";
    else if (isHeader("content-type"))
    {
        if (stristr(arg, MIME_OGG)) return "type OGG";
        else if (stristr(arg, MIME_XOGG)) return "type OGG";
        else if (stristr(arg, MIME_MP3)) return "type MP3";
        else if (stristr(arg, MIME_XMP3)) return "type MP3";
        else if (stristr(arg, MIME_RAW)) return "type RAW";
        else if (stristr(arg, MIME_XPCP)) return "type PCP";
        else if (stristr(arg, MIME_XSCPLS)) return "type PLS";
        else if (stristr(arg, MIME_PLS)) return "type PLS";
        else if (stristr(arg, MIME_XPLS)) return "type PLS";
        else if (stristr(arg, MIME_M3U)) return "type PLS";
        else if (stristr(arg, MIME_MPEGURL)) return "type PLS";
        else if (stristr(arg, MIME_TEXT)) return "type PLS";
        return "type none";
    }
    return "other";
}

static std::string rs_icy(const char* cmdLine, const char* arg)
{
    static const char* names[] = { "name", "url", "bitrate", "genre", "desc", "auth", "id", "password", "content-type", "other" };
    int k = pcrs_servhs_icy_header(u8(cmdLine), strlen(cmdLine));
    if (k == PCRS_ICY_CONTENT_TYPE)
    {
        const char* t = pcrs_servhs_icy_content_type(u8(arg), strlen(arg));
        return std::string("type ") + (t ? t : "none");
    }
    return names[k];
}

// handshakeLocalFile のページの種類と id
static std::string cxx_local_file(const char* fn)
{
    auto idOf = [&](const std::vector<std::string>& vec) { String id = queryGet(vec[1], "id").c_str(); return std::string(id.cstr()); };
    if (str::contains(fn, "/play.html"))
    {
        auto vec = str::split(fn, "?");
        if (vec.size() != 2) return "play 400";
        return "play " + idOf(vec);
    }else if (str::contains(fn, "/relayinfo.html") || str::contains(fn, "/head.html"))
    {
        auto vec = str::split(fn, "?");
        if (vec.size() != 2) return "relay 400";
        return "relay " + idOf(vec);
    }else if (str::contains(fn, "connections.html") || str::contains(fn, "editinfo.html"))
    {
        auto vec = str::split(fn, "?");
        if (vec.size() == 2) return "conn " + idOf(vec);
        return "conn -";
    }
    return "plain";
}

static std::string rs_local_file(const char* fn)
{
    bool splitOk;
    pcrs_buf id;
    int page = pcrs_servhs_local_file(u8(fn), strlen(fn), &splitOk, &id);
    String idStr = take(id).c_str();
    switch (page)
    {
    case PCRS_PAGE_PLAY: return splitOk ? std::string("play ") + idStr.cstr() : "play 400";
    case PCRS_PAGE_RELAY_INFO: return splitOk ? std::string("relay ") + idStr.cstr() : "relay 400";
    case PCRS_PAGE_CONNECTIONS: return splitOk ? std::string("conn ") + idStr.cstr() : "conn -";
    default: return "plain";
    }
}

static std::string cxx_server_name(const std::string& host)
{
    if (!Regexp("^[A-Za-z0-9\\-_.]+:\\d+$").exec(host).empty())
        return "ok " + str::split(host, ":")[0];
    return "none";
}

static std::string rs_server_name(const std::string& host)
{
    pcrs_buf out;
    bool ok = pcrs_servhs_cgi_server_name(u8(host.data()), host.size(), &out);
    std::string v = take(out);
    return ok ? "ok " + v : "none";
}

static std::string cxx_header_line(const std::string& line)
{
    static Regexp headerPattern("^([A-Za-z\\-]+):\\s*(.*)$");
    auto caps = headerPattern.exec(line);
    if (caps.size() == 0)
        return "invalid";
    return caps[1] + " | " + caps[2];
}

static std::string rs_header_line(const std::string& line)
{
    pcrs_buf name, value;
    bool ok = pcrs_servhs_cgi_header_line(u8(line.data()), line.size(), &name, &value);
    std::string n = take(name), v = take(value);
    return ok ? n + " | " + v : "invalid";
}

static std::string cxx_jrpc_len(const std::string& lenstr)
{
    int content_length = -1;
    if (!lenstr.empty())
        content_length = atoi(lenstr.c_str());
    if (content_length == -1) return "HTTP/1.0 411 Length required";
    if (content_length <= 0) return HTTP_SC_BADREQUEST;
    if (content_length > HTTP::MAX_REQUEST_BODY) return "HTTP/1.0 413 Request Entity Too Large";
    return std::to_string(content_length);
}

static std::string rs_jrpc_len(const std::string& lenstr)
{
    const char* status = nullptr;
    int n = pcrs_servhs_jrpc_body_length(u8(lenstr.c_str()), strlen(lenstr.c_str()), HTTP::MAX_REQUEST_BODY, &status);
    return n < 0 ? std::string(status) : std::to_string(n);
}

// ---------------------------------------------------------------- 比べる

static void oneCase()
{
    std::string s = randStr(R(8) == 0 ? 700 : 80);
    const char* cs = s.c_str();

    // 要求の行
    std::string pw = R(3) ? "" : randStr(8);
    report("request_kind", s + " / " + pw, std::to_string(cxx_request_kind(cs, pw.c_str())),
           std::to_string(pcrs_servhs_request_kind(u8(cs), s.size(), u8(pw.c_str()), strlen(pw.c_str()))));
    report("is_http", s, std::to_string(stristr(cs, HTTP_PROTO1) != nullptr), std::to_string(pcrs_servhs_is_http(u8(cs), s.size())));
    std::string get = "GET " + s;
    report("get_route", get, cxx_get_route(get), rs_get_route(get));
    report("admin_cgi", s, cxx_admin_cgi(s), rs_admin_cgi(s));
    report("post_route", s, cxx_post_route(cs), rs_post_route(s));
    {
        GnuID a;
        auto *idstr = strstr(cs, "/");
        if (idstr)
            a.fromStr(idstr+1);
        GnuID b;
        pcrs_servhs_giv_id(u8(cs), s.size(), b.id);
        report("giv_id", s, a.str(), b.str());
    }
    std::string src = (R(2) ? "SOURCE" : "") + s;
    report("source", src, cxx_source(src), rs_source(src));

    // 認証
    {
        for (auto& b : chanMgr->broadcastID.id) b = rng();
        GnuID id;
        for (auto& b : id.id) b = rng();
        std::string req = s;
        if (R(3))
        {
            std::string hex = R(2) ? id.str() : str::downcase(id.str());
            req = hex + (R(2) ? ".flv" : "") + "?auth=" + (R(2) ? chanMgr->authToken(id) : s);
            if (R(4) == 0) req += "&" + s;
        }
        report("valid_auth_token", req, std::to_string(Servent::hasValidAuthToken(req)),
               std::to_string(pcrs_servhs_valid_auth_token(u8(req.data()), req.size(), chanMgr->broadcastID.id)));
        unsigned short port = R(2) ? 7144 : R(65536);
        std::string cookie = s;
        if (R(2)) cookie = "a=b; " + std::to_string(port) + "_id=" + s;
        report("cookie", cookie, cxx_cookie(cookie, port), rs_cookie(cookie, port));
    }

    // CGI の引数
    {
        auto v = rustbridge::takeVec(pcrs_servhs_cgi_args(u8(cs), s.size()));
        std::string a, b;
        char arg[MAX_CGI_LEN], curr[MAX_CGI_LEN];
        const char* cp = cs;
        while ((cp = nextCGIarg(cp, curr, arg)) != nullptr)
            a += std::string(curr) + "=" + arg + "&";
        for (size_t i = 0; i + 1 < v.size(); i += 2)
            b += v[i] + "=" + v[i + 1] + "&";
        report("cgi_args", s, a, b);
    }
    report("apply", s, cxx_apply(cs), rs_apply(cs));
    report("redirect", s, cxx_redirect(cs), rs_redirect(cs));
    std::string path = "html/" + (R(2) ? "en" : randStr(6));
    report("rewrite_referer", s, cxx_referer(s, path), rs_referer(s, path));
    report("is_valid_html_path", s, std::to_string(ServMgr::isValidHtmlPath(s)),
           std::to_string(pcrs_servhs_is_valid_html_path(u8(s.data()), s.size())));
    std::string dec = R(2) ? s : std::to_string(R(1000));
    report("is_decimal", dec, std::to_string(cxx_is_decimal(dec)), std::to_string(pcrs_servhs_is_decimal(u8(dec.data()), dec.size())));

    // 放送
    {
        std::string arg = randStr(20);
        report("icy_header", s + " / " + arg, cxx_icy(cs, arg.c_str()), rs_icy(cs, arg.c_str()));
    }

    // ローカルのファイル
    {
        String fileName = cs;
        const char* a = Servent::fileNameToMimeType(fileName);
        const char* b = pcrs_servhs_mime_type(u8(fileName.c_str()), strlen(fileName.c_str()));
        report("mime_type", s, a ? a : "(null)", b ? b : "(null)");
        report("local_file", s, cxx_local_file(cs), rs_local_file(cs));
        std::string root = R(2) ? "/home/x/peercast/" : randStr(260);
        String f1 = root.c_str();
        f1.append(cs);
        std::string f2 = take(pcrs_servhs_local_file_name(u8(root.c_str()), strlen(root.c_str()), u8(cs), s.size()));
        String f2s = f2.c_str();
        report("local_file_name", root + " + " + s, f1.cstr(), f2s.cstr());
    }

    // CGI スクリプト、JSON-RPC
    report("cgi_server_name", s, cxx_server_name(s), rs_server_name(s));
    std::string line = R(3) ? s : std::string(R(2) ? "Status" : "Content-Type") + ":" + s;
    report("cgi_header_line", line, cxx_header_line(line), rs_header_line(line));
    std::string len = R(3) ? s : std::to_string((int) rng() % 3000000 - 1000);
    report("jrpc_body_length", len, cxx_jrpc_len(len), rs_jrpc_len(len));
}

int main(int argc, char** argv)
{
    long n = argc > 1 ? atol(argv[1]) : 100000;
    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();
    for (long i = 0; i < n; i++)
        oneCase();
    printf("比較件数 %ld、説明のつかない違い %ld\n", g_cases, g_bad);
    return g_bad ? 1 : 0;
}
