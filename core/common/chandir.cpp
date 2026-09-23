#include <algorithm>
#include <sstream>
#include <memory> // unique_ptr
#include <stdexcept> // runtime_error

#include "http.h"
#include "version2.h"
#include "socket.h"
#include "chandir.h"
#include "uri.h"
#include "servmgr.h"
#include "regexp.h"

using namespace std;

#ifdef WITH_RUST_CORE
#include "rustbridge.h"

// Rust が解釈した 1 行 (pcrs_chan_entry) から ChannelEntry を作る
struct ChannelEntryBuilder
{
    std::vector<ChannelEntry>* out;
    const std::string* feedUrl;
    std::vector<std::string>* errors;

    static std::string s(pcrs_bytes b)
    {
        return std::string(reinterpret_cast<const char*>(b.ptr), b.len);
    }

    static void fill(ChannelEntry& e, const pcrs_chan_entry* c)
    {
        e.name           = s(c->name);
        memcpy(e.id.id, c->id, 16);
        e.tip            = s(c->tip);
        e.url            = s(c->url);
        e.genre          = s(c->genre);
        e.desc           = s(c->desc);
        e.numDirects     = c->num_directs;
        e.numRelays      = c->num_relays;
        e.bitrate        = c->bitrate;
        e.contentTypeStr = s(c->content_type);
        e.trackArtist    = s(c->track_artist);
        e.trackAlbum     = s(c->track_album);
        e.trackName      = s(c->track_name);
        e.trackContact   = s(c->track_contact);
        e.encodedName    = s(c->encoded_name);
        e.uptime         = s(c->uptime);
        e.status         = s(c->status);
        e.comment        = s(c->comment);
        e.direct         = c->direct;
    }

    static void fillOne(void* ctx, const pcrs_chan_entry* c) noexcept
    {
        fill(*static_cast<ChannelEntry*>(ctx), c);
    }

    static void onEntry(void* ctx, const pcrs_chan_entry* c) noexcept
    {
        auto* b = static_cast<ChannelEntryBuilder*>(ctx);
        ChannelEntry e(*b->feedUrl);
        fill(e, c);
        b->out->push_back(std::move(e));
    }

    static void onError(void* ctx, int32_t lineno) noexcept
    {
        auto* b = static_cast<ChannelEntryBuilder*>(ctx);
        b->errors->push_back(str::format("Parse error at line %d.", (int) lineno));
    }
};

ChannelEntry::ChannelEntry(const std::vector<std::string>& fields, const std::string& aFeedUrl)
    : ChannelEntry(aFeedUrl)
{
    std::vector<pcrs_bytes> v;
    for (auto& f : fields)
        v.push_back({ reinterpret_cast<const uint8_t*>(f.data()), f.size() });
    if (pcrs_chandir_entry(v.data(), v.size(), this, ChannelEntryBuilder::fillOne) != 0)
        throw std::runtime_error("too few fields");
}

std::vector<ChannelEntry>
ChannelEntry::textToChannelEntries(const std::string& text, const std::string& aFeedUrl, std::vector<std::string>& errors)
{
    vector<ChannelEntry> res;
    ChannelEntryBuilder b{ &res, &aFeedUrl, &errors };
    pcrs_chandir_parse(reinterpret_cast<const uint8_t*>(text.data()), text.size(), &b,
                       ChannelEntryBuilder::onEntry, ChannelEntryBuilder::onError);
    return res;
}

static std::string sideUrl(const std::string& feedUrl, const std::string& encodedName, int kind)
{
    return rustbridge::RustBuf(pcrs_chandir_side_url(reinterpret_cast<const uint8_t*>(feedUrl.data()), feedUrl.size(),
                                                     reinterpret_cast<const uint8_t*>(encodedName.data()), encodedName.size(),
                                                     kind)).str();
}

std::string ChannelEntry::chatUrl()
{
    return sideUrl(feedUrl, encodedName, 0);
}

std::string ChannelEntry::statsUrl()
{
    return sideUrl(feedUrl, encodedName, 1);
}
#else
std::vector<ChannelEntry>
ChannelEntry::textToChannelEntries(const std::string& text, const std::string& aFeedUrl, std::vector<std::string>& errors)
{
    istringstream in(text);
    string line;
    vector<ChannelEntry> res;
    int lineno = 0;

    while (getline(in, line)) {
        lineno++;
        vector<string> fields = str::split(line, "<>");
        if (fields.size() != 19) {
            errors.push_back(str::format("Parse error at line %d.", lineno));
        } else {
            res.push_back(ChannelEntry(fields, aFeedUrl));
        }
    }

    return res;
}

std::string ChannelEntry::chatUrl()
{
    if (encodedName.empty())
        return "";

    auto index = feedUrl.rfind('/');
    if (index == std::string::npos)
        return "";
    else
        return feedUrl.substr(0, index) + "/chat.php?cn=" + encodedName;
}

std::string ChannelEntry::statsUrl()
{
    if (encodedName.empty())
        return "";

    auto index = feedUrl.rfind('/');
    if (index == std::string::npos)
        return "";
    else
        return feedUrl.substr(0, index) + "/getgmt.php?cn=" + encodedName;
}
#endif // WITH_RUST_CORE

ChannelDirectory::ChannelDirectory()
    : m_lastUpdate(0)
{
}

int ChannelDirectory::numChannels() const
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);
    return m_channels.size();
}

int ChannelDirectory::numFeeds() const
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);
    return m_feeds.size();
}

// index.txt を指す URL である url からチャンネルリストを読み込み、out
// に格納する。成功した場合は true が返る。エラーが発生した場合は
// false が返る。false を返した場合でも out に読み込めたチャンネル情報
// が入っている場合がある。
static bool getFeed(const std::string& url, std::vector<ChannelEntry>& out)
{
    out.clear();

    try {
        const int serverPort = servMgr->serverHost.port;
        cgi::Query query;
        query.add("host", str::STR("localhost:", serverPort));
        auto body = http::get(url + "?" + query.str());

        std::vector<std::string> errors;
        out = ChannelEntry::textToChannelEntries(body, url, errors);

        for (auto& message : errors) {
            LOG_ERROR("%s", message.c_str());
        }

        return errors.empty();
    } catch (GeneralException& e) {
        LOG_ERROR("%s", e.msg);
        return false;
    }
}

#include "sstream.h"
#include "defer.h"
#include "logbuf.h"

static std::string runProcess(std::function<void(Stream&)> action)
{
    StringStream ss;
    try {
        assert(AUX_LOG_FUNC_VECTOR != nullptr);
        AUX_LOG_FUNC_VECTOR->push_back([&](LogBuffer::TYPE type, const char* msg) -> void
                                      {
                                          if (type == LogBuffer::T_ERROR)
                                              ss.writeString("Error: ");
                                          else if (type == LogBuffer::T_WARN)
                                              ss.writeString("Warning: ");
                                          ss.writeLine(msg);
                                      });
        Defer defer([]() { AUX_LOG_FUNC_VECTOR->pop_back(); });

        action(ss);
    } catch(GeneralException& e) {
        ss.writeLineF("Error: %s\n", e.what());
    }
    return ss.str();
}

bool ChannelDirectory::update(UpdateMode mode)
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);

    const unsigned int coolDownTime = (mode==kUpdateManual) ? 30 : 5 * 60;
    if (sys->getTime() - m_lastUpdate < coolDownTime)
        return false;

    double t0 = sys->getDTime();
    std::vector<std::thread> workers;
    std::mutex mutex; // m_channels を保護するミューテックス。
    m_channels.clear();
    for (auto& feed : m_feeds)
    {
        std::function<void(void)> getChannels =
            [&feed, &mutex, this]
            {
                assert(AUX_LOG_FUNC_VECTOR == nullptr);
                AUX_LOG_FUNC_VECTOR = new std::vector<std::function<void(LogBuffer::TYPE type, const char*)>>();
                assert(AUX_LOG_FUNC_VECTOR != nullptr);
                Defer defer([]()
                            {
                                assert(AUX_LOG_FUNC_VECTOR != nullptr);
                                delete AUX_LOG_FUNC_VECTOR;
                            });

                feed.log = runProcess([&feed, &mutex, this](Stream& s)
                                      {
                                          // print start time
                                          String time;
                                          time.setFromTime(sys->getTime());
                                          s.writeStringF("Start time: %s\n", time.c_str()); // two newlines at the end

                                          std::vector<ChannelEntry> channels;
                                          bool success;
                                          double t1 = sys->getDTime();
                                          success = getFeed(feed.url, channels);
                                          double t2 = sys->getDTime();

                                          LOG_TRACE("Got %zu channels from %s (%.6f seconds)", channels.size(), feed.url.c_str(), t2 - t1);
                                          if (success) {
                                              feed.status = ChannelFeed::Status::kOk;
                                          } else {
                                              feed.status = ChannelFeed::Status::kError;
                                          }

                                          {
                                              std::lock_guard<std::mutex> lock(mutex);
                                              for (auto& c : channels) m_channels.push_back(c);
                                          }
                                      });
            };
        workers.push_back(std::thread(getChannels));
    }

    for (auto& t : workers)
        t.join();

    sort(m_channels.begin(), m_channels.end(),
         [](ChannelEntry& a, ChannelEntry& b)
         {
             return a.numDirects > b.numDirects;
         });
    m_lastUpdate = sys->getTime();
    LOG_INFO("Channel feed update: total of %zu channels in %f sec",
             m_channels.size(),
             sys->getDTime() - t0);
    return true;
}

// index番目のチャンネル詳細のフィールドを出力する。成功したら true を返す。
bool ChannelDirectory::writeChannelVariable(Stream& out, const String& varName, int index)
{
    using namespace std;

    std::lock_guard<std::recursive_mutex> cs(m_lock);

    if (!(index >= 0 && (size_t)index < m_channels.size()))
        return false;

    string buf;
    ChannelEntry& ch = m_channels[index];

    if (varName == "name") {
        buf = ch.name;
    } else if (varName == "id") {
        buf = ch.id.str();
    } else if (varName == "bitrate") {
        buf = to_string(ch.bitrate);
    } else if (varName == "contentTypeStr") {
        buf = ch.contentTypeStr;
    } else if (varName == "desc") {
        buf = ch.desc;
    } else if (varName == "genre") {
        buf = ch.genre;
    } else if (varName == "url") {
        buf = ch.url;
    } else if (varName == "tip") {
        buf = ch.tip;
    } else if (varName == "encodedName") {
        buf = ch.encodedName;
    } else if (varName == "uptime") {
        buf = ch.uptime;
    } else if (varName == "numDirects") {
        buf = to_string(ch.numDirects);
    } else if (varName == "numRelays") {
        buf = to_string(ch.numRelays);
    } else if (varName == "chatUrl") {
        buf = ch.chatUrl();
    } else if (varName == "statsUrl") {
        buf = ch.statsUrl();
    } else if (varName == "isPlayable") {
        buf = to_string(ch.id.isSet());
    } else {
        return false;
    }

    out.writeString(buf);
    return true;
}

#ifdef WITH_RUST_CORE
static std::string directoryUrlOf(const std::string& url)
{
    return rustbridge::RustBuf(pcrs_chandir_directory_url(reinterpret_cast<const uint8_t*>(url.data()), url.size())).str();
}

static std::string formatTime(unsigned int diff)
{
    return rustbridge::RustBuf(pcrs_chandir_format_time(diff)).str();
}
#else
static std::string directoryUrlOf(const std::string& url)
{
    auto matches = Regexp("/[^/]*$").exec(url);
        
    if (matches.size() > 0) {
        return str::replace_suffix(url, matches[0], "/");
    } else {
        return url;
    }
}

static std::string formatTime(unsigned int diff)
{
    auto min = diff / 60;
    auto sec = diff % 60;
    if (min == 0) {
        return str::format("%ds", sec);
    } else {
        return str::format("%dm %ds", min, sec);
    }
}
#endif // WITH_RUST_CORE

amf0::Value ChannelDirectory::getState()
{
    std::vector<amf0::Value> channels;

    for (auto& c : this->channels())
    {
        channels.push_back(amf0::Value::object(
                          {
                              { "name",           c.name },
                              { "id",             c.id.str() },
                              { "tip",            c.tip },
                              { "url",            c.url },
                              { "genre",          c.genre },
                              { "desc",           c.desc },
                              { "numDirects",     c.numDirects },
                              { "numRelays",      c.numRelays },
                              { "bitrate",        c.bitrate },
                              { "contentTypeStr", c.contentTypeStr },
                              { "trackArtist",    c.trackArtist },
                              { "trackAlbum",     c.trackAlbum },
                              { "trackName",      c.trackName },
                              { "trackContact",   c.trackContact },
                              { "encodedName",    c.encodedName },
                              { "uptime",         c.uptime },
                              { "status",         c.status },
                              { "comment",        c.comment },
                              { "direct",         c.direct },
                              { "feedUrl",        c.feedUrl },
                              { "chatUrl",        c.chatUrl() },
                              { "statsUrl",       c.statsUrl() },
                          }));
    }

    std::vector<amf0::Value> feeds;
    for (auto& f : this->feeds())
    {
        int count = std::count_if(channels.begin(), channels.end(), [&](const amf0::Value& v) { return v.object().at("feedUrl")==f.url; });
        feeds.push_back(amf0::Value::object(
                            {
                                {"url", f.url},
                                {"directoryUrl", directoryUrlOf(f.url)},
                                {"status", ChannelFeed::statusToString(f.status) },
                                {"numChannels", count }
                            }));
    }

    return amf0::Value::object(
        {
            {"totalListeners", totalListeners()},
            {"totalRelays", totalRelays()},
            {"lastUpdate", formatTime(sys->getTime() - m_lastUpdate)},
            {"channels", amf0::Value::strictArray(channels) },
            {"feeds", amf0::Value::strictArray(feeds) },
        });
}

int ChannelDirectory::totalListeners() const
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);
    int res = 0;

    for (const ChannelEntry& e : m_channels) {
        res += std::max(0, e.numDirects);
    }
    return res;
}

int ChannelDirectory::totalRelays() const
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);
    int res = 0;

    for (const ChannelEntry& e : m_channels) {
        res += std::max(0, e.numRelays);
    }
    return res;
}

std::vector<ChannelFeed> ChannelDirectory::feeds() const
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);
    return m_feeds;
}

bool ChannelDirectory::addFeed(const std::string& url)
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);

    auto iter = find_if(m_feeds.begin(), m_feeds.end(), [&](ChannelFeed& f) { return f.url == url;});

    if (iter != m_feeds.end()) {
        LOG_ERROR("Already have feed %s", url.c_str());
        return false;
    }

    URI u(url);
    if (!u.isValid() || (u.scheme() != "http" && u.scheme() != "https")) {
        LOG_ERROR("Invalid feed URL %s", url.c_str());
        return false;
    }

    m_feeds.push_back(ChannelFeed(url));
    return true;
}

void ChannelDirectory::clearFeeds()
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);

    m_feeds.clear();
    m_channels.clear();
    m_lastUpdate = 0;
}

std::string ChannelDirectory::findTracker(const GnuID& id) const
{
    for (const ChannelEntry& entry : m_channels)
    {
        if (entry.id.isSame(id))
            return entry.tip;
    }
    return "";
}

std::shared_ptr<ChannelEntry> ChannelDirectory::findEntry(const GnuID& id) const
{
    for (const ChannelEntry& entry : m_channels)
    {
        if (entry.id.isSame(id))
            return std::make_shared<ChannelEntry>(entry);
    }
    return nullptr;
}

std::string ChannelFeed::statusToString(ChannelFeed::Status s)
{
    switch (s) {
    case Status::kUnknown:
        return "UNKNOWN";
    case Status::kOk:
        return "OK";
    case Status::kError:
        return "ERROR";
    }
    throw std::logic_error("should be unreachable");
}

std::vector<ChannelEntry> ChannelDirectory::channels() const
{
    std::lock_guard<std::recursive_mutex> cs(m_lock);
    return m_channels;
}
