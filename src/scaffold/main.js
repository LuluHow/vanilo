document.addEventListener("DOMContentLoaded", function () {
    var el = document.getElementById("visit-count");
    if (!el) return;

    fetch("/api/hello")
        .then(function (r) { return r.json(); })
        .then(function (data) { el.textContent = data.visits; })
        .catch(function () { el.textContent = "-"; });
});
