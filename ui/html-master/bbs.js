// -*- mode: js; js-indent-level: 4 -*-

// メイン処理に開いてほしいコマンドを入れるキュー。
// ['open', URL] スレッド、板を開く。
// ['post', MESSAGE] 投稿。
$queue = [];

// HTML の文字列から文字だけを取り出す。<template> の中身は別の文書に作られるので、
// スクリプトは動かず、画像も読まれない。
function textOf(html) {
    const template = document.createElement('template');
    template.innerHTML = html;
    return template.content.textContent;
}

// 掲示板から届いた HTML (レスの本文) を、改行と http(s) のリンクと文字だけにして parent に足す。
// 掲示板のサーバーは配信者がコンタクト URL で好きに指定できるので、届いた HTML をそのまま
// ページに入れると、スクリプトや onerror などを仕込まれる。
function appendSanitized(parent, node) {
    for (const child of node.childNodes) {
        if (child.nodeType === Node.TEXT_NODE) {
            parent.appendChild(document.createTextNode(child.textContent));
        } else if (child.nodeType === Node.ELEMENT_NODE) {
            const tag = child.tagName;
            if (tag === 'BR') {
                parent.appendChild(document.createElement('br'));
            } else if (tag === 'SCRIPT' || tag === 'STYLE') {
                // 中身ごと捨てる。
            } else if (tag === 'A' && /^https?:\/\//i.test(child.getAttribute('href') || '')) {
                const a = document.createElement('a');
                a.href = child.getAttribute('href');
                a.target = '_blank';
                a.rel = 'noopener noreferrer';
                appendSanitized(a, child);
                parent.appendChild(a);
            } else {
                // タグは捨てて、中身だけを残す。
                appendSanitized(parent, child);
            }
        }
    }
}

function renderPost(p, info) {
    const div = document.createElement('div');
    div.className = 'highlight';
    div.style.padding = '2px';
    div.style.fontSize = '13px';

    const a = document.createElement('a');
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    a.title = textOf([p.name, p.mail, p.date].join(' '));
    a.href = `${threadUrl(info)}/${p.no}`;
    const b = document.createElement('b');
    b.textContent = p.no;
    a.appendChild(b);
    div.appendChild(a);
    div.appendChild(document.createTextNode('：'));

    const body = document.createElement('template');
    body.innerHTML = p.body;
    appendSanitized(div, body.content);
    return div;
}

function boardCgi(info) {
    return ("/cgi-bin/board.cgi?fqdn="+info.fqdn+"&category="+info.category+"&board_num="+info.board_num);
}

function threadCgi(info, id) {
    return ("/cgi-bin/thread.cgi?fqdn="+info.fqdn+"&category="+info.category+"&board_num="+info.board_num+"&id="+id);
}

function boardUrl(info) {
    return (info.protocol+"://"+info.fqdn+"/"+info.category+"/"+info.board_num);
}

function threadUrl(info) {
    return (info.protocol+"://"+info.fqdn+"/"+(info.shitaraba ? "bbs" : "test")+"/read.cgi/"+info.category+(info.shitaraba ? "/"+info.board_num+"/" : "/")+info.thread_id);
}

function scrollViewToBottom() {
    const view = document.getElementById('bbs-view');
    view.scrollTop = view.scrollHeight - view.clientHeight;
}

function setPostFormVisible(visible) {
    for (const form of document.querySelectorAll('.post-form'))
        form.style.display = visible ? '' : 'none';
}

function newPostsCallback(url, thread) {
    const div = document.querySelector('#bbs-view > div.thread');

    for (var i = 0; i < thread.posts.length; i++) {
        div.appendChild(renderPost(thread.posts[i], url));
    }

    // 最下部にスクロール。
    if (thread.posts.length > 0) {
        scrollViewToBottom();
    }
}

// スレッドが開かれたときにDOMを変える。実際のレスは追加しない。
function loadThread(info, thread, board_title) {
    var board_url = boardUrl(info);
    var thread_url = threadUrl(info);
    var naviHtml;
    naviHtml = `<a class='board-link' href='${h(board_url)}'>${h(board_title)}</a>`;
    naviHtml += ' &raquo; ';
    naviHtml += `<b><a target='_blank' rel='noopener noreferrer' href='${h(thread_url)}/l5#form_write'>${h(thread.title)}</a></b>`;
    document.getElementById('bbs-title').innerHTML = naviHtml;

    for (const link of document.querySelectorAll('.board-link')) {
        link.addEventListener('click', function (e) {
            if (e.button === 0) {
                e.preventDefault();
                $queue.push(['open', board_url]);
            }
        });
    }

    document.getElementById('bbs-view').innerHTML = "<div class='thread'></div>";
}

function h(str) {
    var table = {
        '&': "&amp;",
        '<': "&lt;",
        '>': "&gt;",
        '\"': "&quot;",
        '\'': "&#39;",
    };
    return String(str).replace(/[&<>"']/g, function (char) {
        return table[char];
    });
}

function getBoardInfo(url) {
    var match;
    if (match = /^(\w+):\/\/([^\/]+)\/(\w+)(\/(\d+))?\/?$/.exec(url)) {
        var info = {};
        info.protocol = match[1];
        info.fqdn = match[2];
        info.shitaraba = match[2].indexOf("shitaraba") != -1;
        info.category = match[3];
        info.board_num = info.shitaraba ? match[5] : "";
        return info;
    } else {
        return null;
    }
}

function getThreadInfo(url) {
    var match;
    if (match = /^(\w+):\/\/([^\/]+)\/(test|bbs)\/read\.cgi\/(\w+)\/(\d+)(\/(\d+))?(:?|\/.*)$/.exec(url)) {
        var info = {};
        info.protocol = match[1];
        info.fqdn = match[2];
        info.shitaraba = match[2].indexOf("shitaraba") != -1;
        info.category = match[4];
        info.board_num = info.shitaraba ? match[5] : "";
        info.thread_id = info.shitaraba ? match[7] : match[5];
        return info;
    } else {
        return null;
    }
}

// #chat-visibility チェックボックスの状態を見て、#bbs-view-conatiner
// の表示/非表示を切り替える。
function switchChatVisibility() {
    const container = document.getElementById('bbs-view-container');
    if (document.getElementById('chat-visibility').checked) {
        container.style.display = '';

        // 最新レスが見えるように最下部にスクロールする。
        scrollViewToBottom();
    } else {
        container.style.display = 'none';
    }
}

function postMessage(message) {
    $queue.push(['post', message])
}

function delay(ms) {
    return new Promise((resolve, _reject) => setTimeout(resolve, ms));
}

function encodeForm(obj) {
    let buf = ""
    for (var key of Object.keys(obj)) {
        buf += `&${key}=${encodeURIComponent(obj[key])}`;
    }
    return buf.substring(1);
}

// JSON を取得する。200 番台でなければ例外。X-Requested-With を付けると、ログインが切れている
// ときにログインのページではなく 403 が返る。
async function getJSON(url) {
    const response = await fetch(url, { headers: { 'X-Requested-With': 'XMLHttpRequest' } });
    if (!response.ok)
        throw new Error(`${response.status} ${response.statusText}`);
    return response.json();
}

async function doPostMessageAsync(info, body) {
    const name = '', mail = 'sage';
    const query = encodeForm({'fqdn':info.fqdn, 'category':info.category, 'id':info.thread_id, 'board_num':info.board_num}) + '&' + encodeForm({name, mail, body})

    try {
        const result = await getJSON(`/cgi-bin/post.cgi?${query}`);
        if (result.status == 'error') {
            alert(`Error: code = ${result.code}`);
        } else {
            document.getElementById('message-input').value = '';
        }
    } catch (error) {
        alert(error.message);
    }
}

// url を開く。開けないURLならそのまま終了。ユーザーからの入力で別の
// URLを開く。スレッドURLでは取得ループも同時に行う。
async function mainAsync(url) {
    const view = document.getElementById('bbs-view');
    const title = document.getElementById('bbs-title');
    var info;
    if (info = getBoardInfo(url)) {
        // スレッドリストを取得する。
        view.textContent = "掲示板を読み込み中…";
        let board;
        try {
            board = await getJSON(boardCgi(info));
        } catch {
            view.textContent = "エラー: /cgi-bin/board.cgiの実行に失敗しました。";
            return;
        }
        console.log(board);

        var board_url = boardUrl(info);
        title.innerHTML = `<a target='_blank' rel='noopener noreferrer' href='${h(board_url)}'><b>${h(board.title)}</b></a>`;

        if (board.threads.length > 0) {
            var buf = "";
            for (var i = 0; i < board.threads.length; i++) {
                var t = board.threads[i];
                var thread_url = info.protocol+"://"+info.fqdn+"/"+(info.shitaraba ? "bbs" : "test")+"/read.cgi/"+info.category+(info.shitaraba ? "/"+info.board_num+"/" : "/")+t.id+"/l50";
                buf += `<div style="padding:2px;font-size:13px"><a href='${h(thread_url)}' class='thread-link' data-thread-id='${h(t.id)}'>${h(t.title)} (${h(t.last)})</a></div>`;
            }
            view.innerHTML = buf;
        } else {
            view.innerHTML = `掲示板「<a target='_blank' rel='noopener noreferrer' href='${h(board_url)}'>${h(board.title)}</a>」にスレッドはありません。`;
        }

        for (const link of document.querySelectorAll('.thread-link')) {
            link.addEventListener('click', function (e) {
                if (e.button === 0) {
                    $queue.push(['open', this.href]);
                    e.preventDefault();
                }
            });
        }

        setPostFormVisible(false);

        // 移動要求を待つだけのループ。
        while (true) {
            // 移動要求が来たら中断して全部やりなおす。
            if ($queue.length > 0) {
                const [cmd, arg1] = $queue.shift();
                if (cmd == 'open') {
                    return mainAsync(arg1);
                } else if (cmd == 'post') {
                    alert('post not possible');
                }
            }
            await delay(100);
        }
    } else if (info = getThreadInfo(url)) {
        setPostFormVisible(false);

        let board;
        let thread;
        try {
            board = await getJSON(`/cgi-bin/board.cgi?fqdn=${info.fqdn}&category=${info.category}&board_num=${info.board_num}`);
        } catch {
            view.textContent = "エラー: /cgi-bin/board.cgiの実行に失敗しました。";
            return;
        }
        try {
            thread = await getJSON(threadCgi(info, info.thread_id));
        } catch {
            view.textContent = "エラー: /cgi-bin/thread.cgiの実行に失敗しました。";
            return;
        }

        setPostFormVisible(true);

        console.log(thread);
        loadThread(info, thread, board.title);
        newPostsCallback(info, thread);

        while (true) {
            // 合計７秒間スリープする。
            for (let i = 0; i < 70; i++) {
                // 移動要求が来たら中断して全部やりなおす。
                if ($queue.length > 0) {
                    const [cmd, arg1] = $queue.shift();
                    if (cmd == 'open') {
                        return mainAsync(arg1);
                    } else if (cmd == 'post') {
                        await doPostMessageAsync(info, arg1);
                        break;
                    }
                }
                await delay(100);
            }

            // レスを取得して追加。
            thread = await getJSON(threadCgi(info, thread.id)+"&first="+(thread.last+1));
            console.log(thread);
            newPostsCallback(info, thread);
        }
    } else if (url === "") {
        title.textContent = "n/a";
        view.textContent = "コンタクトURLがありません。";
    } else {
        title.textContent = "n/a";
        view.innerHTML = "対応した掲示板のURLではありません:<br>" + h(url);
    }
}

async function tryOpenCurrentThreadAsync(url)
{
    const info = getBoardInfo(url)
    if (info) {
        try {
            board = await getJSON(boardCgi(info))
        } catch {
            console.error("エラー: /cgi-bin/board.cgiの実行に失敗しました。")
            return await mainAsync(url)
        }
        console.log(board);

        for (let i = 0; i < board.threads.length; i++) {
            const t = board.threads[i];
            if (t.last < 1000) {
                const thread_url = info.protocol+"://"+info.fqdn+"/"+(info.shitaraba ? "bbs" : "test")+"/read.cgi/"+info.category+(info.shitaraba ? "/"+info.board_num+"/" : "/")+t.id+"/l50";
                console.log(`Opening ${thread_url} ...`);
                return await mainAsync(thread_url)
            }
        }
    }
    return await mainAsync(url)
}

function handleSubmit() {
    postMessage(document.getElementById('message-input').value);
}

document.addEventListener('DOMContentLoaded', function(){
    switchChatVisibility();

    // チャット表示切り替えチェックボックスの挙動。
    document.getElementById('chat-visibility').addEventListener('change', function() {
        switchChatVisibility();
    });

    // 投稿ボタンが押された。
    document.getElementById('post-button').addEventListener('click', handleSubmit);
    document.getElementById('message-input').addEventListener('keydown',function(ev){
        if (ev.keyCode == 13 && ev.shiftKey) {
            handleSubmit();
            ev.preventDefault();
        }
    });

    tryOpenCurrentThreadAsync(CONTACT_URL);
});
