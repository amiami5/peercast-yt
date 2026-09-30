// class="reloader" の要素の中身を、data-interval 秒ごとに data-url から読み直す。
// 読んだ HTML の中の <script> は実行されないので、読み直す部分にはスクリプトを置かないこと。
document.addEventListener('DOMContentLoaded', function () {
    for (const r of document.querySelectorAll('.reloader')) {
        const callback = function () {
            // X-Requested-With を付けると、ログインが切れているときにログインのページではなく 403 が返る。
            fetch(r.dataset.url, { cache: 'no-store', headers: { 'X-Requested-With': 'XMLHttpRequest' } })
                .then(function (response) {
                    if (!response.ok)
                        throw new Error(response.status);
                    return response.text();
                })
                .then(function (data) {
                    r.innerHTML = data;
                })
                .catch(function () {
                    // 読めなければ、次の回にまた試す。
                })
                .finally(function () {
                    setTimeout(callback, r.dataset.interval * 1000);
                });
        };
        setTimeout(callback, r.dataset.interval * 1000);
    }
});
