#include "sstream.h"
#include "assets.h"
#ifdef WITH_RUST_CORE
#include "peercast_rs.h"
#endif

using namespace std;

// ------------------------------------------------------------
#ifdef WITH_RUST_CORE
// 拡張子から決めるのは Rust (peercast-rs の src/public.rs)。
static string MIMEType(const string& path)
{
    return pcrs_assets_mime_type(reinterpret_cast<const uint8_t*>(path.data()), path.size());
}
#else
static string MIMEType(const string& path)
{
    using namespace str;

    if (has_suffix(path, ".htm") || has_suffix(path, ".html"))
    {
        return MIME_HTML;
    }else if (has_suffix(path, ".css"))
    {
        return MIME_CSS;
    }else if (has_suffix(path, ".jpg"))
    {
        return MIME_JPEG;
    }else if (has_suffix(path, ".gif"))
    {
        return MIME_GIF;
    }else if (has_suffix(path, ".png"))
    {
        return MIME_PNG;
    }else if (has_suffix(path, ".js"))
    {
        return MIME_JS;
    }else if (has_suffix(path, ".svg"))
    {
        return "image/svg+xml";
    }else if (has_suffix(path, ".ico"))
    {
        return "image/vnd.microsoft.icon";
    }else
    {
        return "application/octet-stream";
    }
}
#endif // WITH_RUST_CORE

// ------------------------------------------------------------
AssetsController::AssetsController(const std::string& documentRoot)
    : mapper("/assets", documentRoot)
{
}

// --------------------------------------
#include <sys/types.h>
#include <sys/stat.h>
#include <unistd.h>

static time_t mtime(const char *path)
{
    struct stat st;
    if (stat(path, &st) == -1)
        return -1;
    else
        return st.st_mtime;
}

// ------------------------------------------------------------
#include "cgi.h"
HTTPResponse AssetsController::operator()(const HTTPRequest& req, Stream& stream, Host& remoteHost)
{
    auto path = mapper.toLocalFilePath(req.path);

    if (path.empty())
        return HTTPResponse::notFound();

    StringStream mem;
    FileStream   file;

    time_t last_modified = mtime(path.c_str());
#ifdef WITH_RUST_CORE
    // If-Modified-Since の読み取りと比べるのは Rust (peercast-rs の src/public.rs)。
    auto ims = req.headers.get("If-Modified-Since");
    if (pcrs_assets_not_modified(last_modified, reinterpret_cast<const uint8_t*>(ims.data()), ims.size()))
        return HTTPResponse::notModified({{ "Last-Modified", cgi::rfc1123Time(last_modified) }});
#else
    time_t if_modified_since = -1;

    if (req.headers.get("If-Modified-Since").size())
        if_modified_since = cgi::parseHttpDate(req.headers.get("If-Modified-Since"));

    if (last_modified != -1 && if_modified_since != -1)
        if (last_modified <= if_modified_since)
            return HTTPResponse::notModified({{ "Last-Modified", cgi::rfc1123Time(last_modified) }});
#endif // WITH_RUST_CORE

    file.openReadOnly(path.c_str());
    file.writeTo(mem, file.length());

    string body = mem.str();
    map<string,string> headers = {
        {"Content-Type",MIMEType(path)},
        {"Content-Length",to_string(body.size())}
    };

    if (last_modified != -1)
        headers["Last-Modified"] = cgi::rfc1123Time(last_modified);

    return HTTPResponse::ok(headers, body);
}
