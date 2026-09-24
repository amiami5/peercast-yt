#include <limits.h>

#include "common.h"
#include "mapper.h"
#include "str.h"
#include "_string.h"
#ifdef WITH_RUST_CORE
#include "rustbridge.h"
#endif

using namespace std;
using namespace str;

FileSystemMapper::FileSystemMapper(const string& aVirtualPath, const string& aDocumentRoot)
    : virtualPath(aVirtualPath)
{
    auto dr = realPath(aDocumentRoot);
    if (dr.empty())
    {
        throw GeneralException(String::format("Document root `%s` inaccessible", aDocumentRoot.c_str()));
    }
    documentRoot = dr;
}

string FileSystemMapper::toLocalFilePath(const string& vpath)
{
    return toLocalFilePath(vpath, {}).first;
}

#ifdef WITH_RUST_CORE
// 試すパスの順は Rust (peercast-rs の src/mapper.rs) が決める。
pair<string,string> FileSystemMapper::resolvePath(const string& rawPath, const vector<string>& langs)
{
    rustbridge::JoinedParts l(langs);
    auto cands = rustbridge::takeVec(pcrs_mapper_candidates(reinterpret_cast<const uint8_t*>(rawPath.data()), rawPath.size(),
                                                            l.data(), l.joined.size(), l.lens.data(), l.lens.size()));
    for (size_t i = 0; i + 1 < cands.size(); i += 2)
    {
        auto r = realPath(cands[i]);
        if (r != "")
            return make_pair(r, cands[i + 1]);
    }
    return make_pair("", "");
}
#else
pair<string,string> FileSystemMapper::resolvePath(const string& rawPath, const vector<string>& langs)
{
    // if there's a language neutral version, return it
    if (realPath(rawPath) != "")
        return make_pair(realPath(rawPath), "");

    // otherwise, try each of the extensions
    for (auto ext : langs)
    {
        auto r = realPath(rawPath + "." + ext);
        if (r != "")
            return make_pair(r, ext);
    }

    // default to the English version if there is one
    auto r = realPath(rawPath + ".en");
    if (r != "")
        return make_pair(r, "en");

    return make_pair("", "");
}
#endif // WITH_RUST_CORE

// path を絶対パスに直す。path が指すファイルが存在しない場合は空文字列を返す。
#ifdef _UNIX
string FileSystemMapper::realPath(const string& path)
{
    char resolvedPath[PATH_MAX];
    char *p = realpath(path.c_str(), resolvedPath);

    if (!p)
        return "";
    else
        return resolvedPath;
}
#endif
#ifdef WIN32
#include <windows.h>
#include "Shlwapi.h"
string FileSystemMapper::realPath(const string& path)
{
    char resolvedPath[4096];
    char* ret = _fullpath(resolvedPath, path.c_str(), 4096);
    if (ret == nullptr)
    {
        LOG_ERROR("_fullpath: Failed to resolve `%s`", path.c_str());
        return "";
    }

    if (PathFileExistsA(resolvedPath))
        return resolvedPath;
    else
        return "";
}
#endif

#ifdef WITH_RUST_CORE
// パスの付け替えとディレクトリトラバーサルの判断は Rust (peercast-rs の src/mapper.rs)。
pair<string,string> FileSystemMapper::toLocalFilePath(const string& vpath, const vector<string>& langs)
{
    rustbridge::RustBuf fp;
    if (!pcrs_mapper_local_path(reinterpret_cast<const uint8_t*>(virtualPath.data()), virtualPath.size(),
                                reinterpret_cast<const uint8_t*>(documentRoot.data()), documentRoot.size(),
                                reinterpret_cast<const uint8_t*>(vpath.data()), vpath.size(), fp.out()))
        return make_pair("", "");

    auto filePath = fp.str();

    string resolvedPath, resolvedLang;
    tie(resolvedPath, resolvedLang) = resolvePath(filePath, langs);

    if (resolvedPath == "")
    {
        LOG_ERROR("Cannot resolve path %s", filePath.c_str());
        return make_pair("", "");
    }

    // ディレクトリトラバーサルチェック
    if (!pcrs_mapper_inside(reinterpret_cast<const uint8_t*>(documentRoot.data()), documentRoot.size(),
                            reinterpret_cast<const uint8_t*>(resolvedPath.data()), resolvedPath.size()))
    {
        LOG_ERROR("Possible directory traversal attack!");
        return make_pair("", "");
    }

    return make_pair(resolvedPath, resolvedLang);
}
#else
pair<string,string> FileSystemMapper::toLocalFilePath(const string& vpath, const vector<string>& langs)
{
    if (virtualPath == vpath || !is_prefix_of(virtualPath + "/", vpath))
        return make_pair("", "");

    auto filePath = replace_prefix(vpath, virtualPath, documentRoot);

    string resolvedPath, resolvedLang;
    tie(resolvedPath, resolvedLang) = resolvePath(filePath, langs);

    if (resolvedPath == "")
    {
        LOG_ERROR("Cannot resolve path %s", filePath.c_str());
        return make_pair("", "");
    }

    // ディレクトリトラバーサルチェック
    if (documentRoot == resolvedPath || !is_prefix_of(documentRoot, resolvedPath))
    {
        LOG_ERROR("Possible directory traversal attack!");
        return make_pair("", "");
    }

    return make_pair(resolvedPath, resolvedLang);
}
#endif // WITH_RUST_CORE
