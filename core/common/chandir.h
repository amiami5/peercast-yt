#ifndef _CHANDIR_H
#define _CHANDIR_H

#include <cstdlib>
#include <vector>
#include <stdexcept> // runtime_error

#include "cgi.h"
#include "str.h"
#include "gnuid.h"

#include "varwriter.h"
#include "threading.h"

class ChannelEntry
{
public:
    static std::vector<ChannelEntry> textToChannelEntries(const std::string& text, const std::string& aFeedUrl, std::vector<std::string>& errors);

#ifdef WITH_RUST_CORE
    // 欄の解釈は Rust 版 (peercast-rs の chandir.rs)。欄が 19 個に足りなければ runtime_error
    ChannelEntry(const std::vector<std::string>& fields, const std::string& aFeedUrl);
#else
    ChannelEntry(const std::vector<std::string>& fields, const std::string& aFeedUrl)
        : feedUrl(aFeedUrl)
    {
        if (fields.size() < 19)
            throw std::runtime_error("too few fields");

        name           = fields[0];
        id             = fields[1];
        tip            = fields[2];
        url            = str::is_http_url(fields[3]) ? fields[3] : "";
        genre          = fields[4];
        desc           = fields[5];
        numDirects     = std::atoi(fields[6].c_str());
        numRelays      = std::atoi(fields[7].c_str());
        bitrate        = std::atoi(fields[8].c_str());
        contentTypeStr = fields[9];
        trackArtist    = fields[10];
        trackAlbum     = fields[11];
        trackName      = fields[12];
        trackContact   = str::is_http_url(fields[13]) ? fields[13] : "";
        encodedName    = fields[14];
        uptime         = fields[15];
        status         = fields[16];
        comment        = fields[17];
        direct         = std::atoi(fields[18].c_str());
    }
#endif

    std::string chatUrl();
    std::string statsUrl();

    std::string name; // (再生不可) などが付くことがある。
    GnuID       id;
    std::string tip;
    std::string url;
    std::string genre;
    std::string desc;
    int         numDirects;
    int         numRelays;
    int         bitrate;
    std::string contentTypeStr;
    std::string trackArtist;
    std::string trackAlbum;
    std::string trackName;
    std::string trackContact;
    std::string encodedName; // URLエンコードされたチャンネル名。

    std::string uptime;
    std::string status;
    std::string comment;
    int         direct;

    std::string feedUrl; // チャットURL、統計URLを作成するために必要。

#ifdef WITH_RUST_CORE
private:
    friend struct ChannelEntryBuilder;
    explicit ChannelEntry(const std::string& aFeedUrl)
        : numDirects(0), numRelays(0), bitrate(0), direct(0), feedUrl(aFeedUrl) {}
#endif
};

class ChannelFeed
{
public:
    enum class Status {
        kUnknown,
        kOk,
        kError
    };

    ChannelFeed()
        : url("")
        , status(Status::kUnknown)
    {
    }

    ChannelFeed(std::string aUrl)
        : url(aUrl)
        , status(Status::kUnknown)
    {
    }

    static std::string statusToString(Status);

    std::string url;
    Status status;

    std::string log;
};

// 外部からチャンネルリストを取得して保持する。
class ChannelDirectory : public VariableWriter
{
public:
    enum UpdateMode {
        kUpdateAuto,
        kUpdateManual,
    };

    ChannelDirectory();

    int numChannels() const;
    int numFeeds() const;
    std::vector<ChannelFeed> feeds() const;
    bool addFeed(const std::string& url);
    void clearFeeds();

    int totalListeners() const;
    int totalRelays() const;

    bool update(UpdateMode mode = kUpdateAuto);

    bool writeChannelVariable(Stream& out, const String& varName, int index);
    bool writeFeedVariable(Stream& out, const String& varName, int index);

    std::vector<ChannelEntry> channels() const;

    std::string findTracker(const GnuID& id) const;
    std::shared_ptr<ChannelEntry> findEntry(const GnuID& id) const;

    amf0::Value getState() override;

    std::vector<ChannelEntry> m_channels;
    std::vector<ChannelFeed> m_feeds;

    unsigned int m_lastUpdate;
    mutable std::recursive_mutex m_lock;
};

#endif
