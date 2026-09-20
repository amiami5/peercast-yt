// ------------------------------------------------
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
// ------------------------------------------------

#include "sslclientsocket.h"
#include <openssl/ssl.h>
#include <openssl/x509v3.h>
#include <sys/types.h>
#ifdef WIN32
#else
#include <sys/socket.h>
#include <arpa/inet.h> // inet_pton
#include "strerror.h"
#endif
#include <unistd.h>
#include "str.h"
#include "stats.h"

using namespace str;

SslClientSocket::SslClientSocket()
    : m_remoteAddr({})
{
    static std::mutex s_mutex;
    static bool s_openssl_initialized = false;
    {
        std::lock_guard<std::mutex> lock(s_mutex);
        if (!s_openssl_initialized)
        {
            LOG_DEBUG("Initializing OpenSSL ...");
            SSL_load_error_strings();
            SSL_library_init();
            OpenSSL_add_ssl_algorithms();
            s_openssl_initialized = true;
        }
    }

    m_socket = -1;
    m_ctx = nullptr;
    m_ssl = nullptr;
}

SslClientSocket::~SslClientSocket()
{
    if (m_ssl)
    {
        // TODO: エラーが起こってる場合は shutdown したらいけないらしい。
        SSL_shutdown(m_ssl);
        SSL_free(m_ssl);
    }

    if (m_ctx)
    {
        SSL_CTX_free(m_ctx);
    }

    if (m_socket != -1)
    {
#ifdef WIN32
        closesocket(m_socket);
#else
        ::close(m_socket);
#endif
    }
}

void SslClientSocket::setTimeoutOptions()
{
#ifdef WIN32
    DWORD timeout;
    timeout = this->readTimeout;
    if (setsockopt(m_socket, SOL_SOCKET, SO_RCVTIMEO, (const char*) &timeout, sizeof timeout) != 0)
        throw SockException("Failed to set read timeout on socket");

    timeout = this->writeTimeout;
    if (setsockopt(m_socket, SOL_SOCKET, SO_SNDTIMEO, (const char*) &timeout, sizeof timeout) != 0)
        throw SockException("Failed to set write timeout on socket");
#else
    struct timeval tv = {};
    tv.tv_sec = this->readTimeout / 1000;
    tv.tv_usec = (this->readTimeout % 1000) * 1000;
    if (setsockopt(m_socket, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv) != 0)
        throw SockException("Failed to set read timeout on socket");

    tv.tv_sec = this->writeTimeout / 1000;
    tv.tv_usec = (this->writeTimeout % 1000) * 1000;
    if (setsockopt(m_socket, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv) != 0)
        throw SockException("Failed to set write timeout on socket");
#endif
}

void SslClientSocket::setHostname(const std::string& hostname)
{
    m_hostname = hostname;
}

void SslClientSocket::open(const Host &rh)
{
    m_socket = socket(AF_INET6, SOCK_STREAM, 0);
    if (m_socket == -1)
	throw SockException("socket open error");

#ifdef WIN32
    // ソケットをデュアルスタックモードに入れる。
    int disable = 0;
    if (setsockopt(m_socket, IPPROTO_IPV6,  IPV6_V6ONLY, (const char*) &disable, sizeof(disable)) == SOCKET_ERROR) {
        throw SockException("Can't reset V6ONLY");
    }
#endif

    setTimeoutOptions();

    m_ctx = SSL_CTX_new(SSLv23_client_method());
    assert( m_ctx != nullptr ); // ライブラリが初期化されているから null は返らない。

#ifndef WIN32
    // サーバー証明書を検証する (Windows のビルドには OS の CA ストアを
    // 読み込む処理がないので、従来どおり検証しない)。検証しないと、通信路上
    // の第三者が YP のフィードなどを改ざんできてしまう。
    if (!m_hostname.empty()) {
        if (SSL_CTX_set_default_verify_paths(m_ctx) != 1)
            throw SockException("Failed to load CA certificates");
        SSL_CTX_set_verify(m_ctx, SSL_VERIFY_PEER, nullptr);
    }
#endif

    m_ssl = SSL_new(m_ctx);
    assert( m_ssl != nullptr );

    if (!m_hostname.empty()) {
        bool isIpAddress = false;
#ifndef WIN32
        unsigned char addr[sizeof(struct in6_addr)];
        isIpAddress = inet_pton(AF_INET, m_hostname.c_str(), addr) == 1 ||
                      inet_pton(AF_INET6, m_hostname.c_str(), addr) == 1;
#endif
        if (!isIpAddress) {
            // SNI。証明書を複数のホストで共用しているサーバーで必要。
            SSL_set_tlsext_host_name(m_ssl, m_hostname.c_str());
        }

#ifndef WIN32
        X509_VERIFY_PARAM* param = SSL_get0_param(m_ssl);
        X509_VERIFY_PARAM_set_hostflags(param, X509_CHECK_FLAG_NO_PARTIAL_WILDCARDS);
        int ok = isIpAddress
            ? X509_VERIFY_PARAM_set1_ip_asc(param, m_hostname.c_str())
            : X509_VERIFY_PARAM_set1_host(param, m_hostname.c_str(), 0);
        if (ok != 1)
            throw SockException("Failed to set the host name to verify");
#endif
    }

    host = rh;

    m_remoteAddr.sin6_family = AF_INET6;
    m_remoteAddr.sin6_port = htons(host.port);
    m_remoteAddr.sin6_addr = host.ip.serialize();
}

void SslClientSocket::bind(const Host &)
{
    throw NotImplementedException("Operation is not supported");
}

void SslClientSocket::connect()
{
    if (::connect(m_socket, (struct sockaddr *) &m_remoteAddr, sizeof(m_remoteAddr)) == -1)
    {
#ifdef WIN32
        throw SockException( format("Can't connect: error = %d", WSAGetLastError()).c_str() );
#else
        throw SockException( format("Can't connect: %s", str::strerror(errno).c_str()).c_str() );
#endif
    }

    if (SSL_set_fd(m_ssl, m_socket) == 0) // error
    {
        throw SockException("SSL_set_fd failed");
    }

    int r;
    r = SSL_connect(m_ssl);
    if (r != 1) {
        const long vr = SSL_get_verify_result(m_ssl);
        if (vr != X509_V_OK)
            throw SockException(format("SSL handshake failed (certificate verification failed: %s)",
                                       X509_verify_cert_error_string(vr)).c_str());
        throw SockException("SSL handshake failed");
    }
}

bool SslClientSocket::active()
{
    return m_socket != -1;
}

std::shared_ptr<ClientSocket> SslClientSocket::accept()
{
    throw NotImplementedException("Operation is not supported");
}

Host SslClientSocket::getLocalHost()
{
    struct sockaddr_in6 localAddr;

    socklen_t len = sizeof(localAddr);
    if (getsockname(m_socket, (sockaddr *)&localAddr, &len) == 0) {
        return Host(IP(localAddr.sin6_addr), 0);
    } else {
        throw SockException("getsockname failed", errno);
    }
}

void SslClientSocket::setBlocking(bool)
{
    throw NotImplementedException("Operation is not supported");
}

int SslClientSocket::read(void *p, int l)
{
    int bytesRead = l;

    while (l > 0) {
        int r = SSL_read(m_ssl, p, l);
        
        if (r <= 0) { // error
            int err = SSL_get_error(m_ssl, r);
            if (err == SSL_ERROR_ZERO_RETURN) {
                throw EOFException("Closed on read");
            } else {
                throw SockException( format("SSL_read failed, error = %d", err).c_str() );
            }
        } else {
            {
                stats.add(Stats::BYTESIN, r);
                if (host.localIP())
                    stats.add(Stats::LOCALBYTESIN, r);
                updateTotals(r, 0);
            }

            l -= r;
            p = (char *)p + r;
        }
    }
    return bytesRead;
}

int SslClientSocket::readUpto(void *p, int l)
{
    int r = SSL_read(m_ssl, p, l);
        
    if (r <= 0) { // error
        int err = SSL_get_error(m_ssl, r);
        if (err == SSL_ERROR_ZERO_RETURN) {
            return 0;
        } else {
            throw SockException( format("SSL_read failed, error = %d", err).c_str() );
        }
    } else {
        return r;
    }
}

#include <openssl/err.h>

void SslClientSocket::write(const void *p, int l)
{
    if (l == 0) // 0バイトの書き込みは常に成功する。
        return;

    int r = SSL_write(m_ssl, p, l);
    if (r <= 0) { // error
        auto err = ERR_get_error();

        char buf[120]; // This is large enough. See ERR_error_string(3).
        ERR_error_string(err, buf);

        throw SockException( format("SSL_write failed: %s", buf).c_str() );
    }

    assert( r == l );

    {
        stats.add(Stats::BYTESOUT, r);
        if (host.localIP())
            stats.add(Stats::LOCALBYTESOUT, r);
        updateTotals(0, r);
    }
}

static std::string s_serverCrtPath = "server.crt";
static std::string s_serverKeyPath = "server.key";

void SslClientSocket::configureServer(const std::string& certificate, const std::string& privatekey)
{
    s_serverCrtPath = certificate;
    s_serverKeyPath = privatekey;
}

std::pair<std::string,std::string> SslClientSocket::getServerConfiguration()
{
    return { s_serverCrtPath, s_serverKeyPath };
}

std::shared_ptr<SslClientSocket> SslClientSocket::upgrade(std::shared_ptr<ClientSocket> rawsock)
{
    SSL_CTX* ctx = SSL_CTX_new(SSLv23_server_method());
    assert( ctx != nullptr );

    SSL_CTX_set_ecdh_auto(ctx, 1);

    if (SSL_CTX_use_certificate_file(ctx, s_serverCrtPath.c_str(), SSL_FILETYPE_PEM) <= 0) {
        throw GeneralException("Certificate file");
    }

    if (SSL_CTX_use_PrivateKey_file(ctx, s_serverKeyPath.c_str(), SSL_FILETYPE_PEM) <= 0) {
        throw GeneralException("Private key file");
    }

    rawsock->setBlocking(true);

    SSL* ssl = SSL_new(ctx);
    assert( ssl != nullptr );
    int ret;
    ret = SSL_set_fd(ssl, rawsock->getDescriptor());
    if (ret != 1) {
        throw StreamException(str::format("%s: SSL_set_fd", __func__));
    }
    if ((ret = SSL_accept(ssl)) <= 0) {
        int code = SSL_get_error(ssl, ret);
        throw StreamException(str::format("%s: SSL_accept: ret = %d, code = %d", __func__, ret, code));
    } else {
        auto sock = std::make_shared<SslClientSocket>();

        sock->m_remoteAddr.sin6_family = AF_INET6;
        sock->m_remoteAddr.sin6_port = htons(rawsock->host.port);
        sock->m_remoteAddr.sin6_addr = rawsock->host.ip.serialize();
        sock->m_socket = rawsock->getDescriptor();
        sock->m_ctx = ctx;
        sock->m_ssl = ssl;
        sock->host = rawsock->host;
        sock->setTimeoutOptions();

        rawsock->detach();

        return sock;
    }
}
