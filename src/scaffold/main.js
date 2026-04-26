document.addEventListener("DOMContentLoaded", function () {

    // --- Search notes (Vanilo.list) ---
    var searchInput = document.getElementById("search");
    var results = document.getElementById("results");

    function loadNotes(query) {
        var url = "/api/notes";
        if (query) url += "?q=" + encodeURIComponent(query);

        fetch(url)
            .then(function (r) { return r.json(); })
            .then(function (data) {
                if (data.notes && data.notes.length > 0) {
                    Vanilo.list("#results", "Card", data.notes);
                } else if (results) {
                    results.innerHTML = "<p style=\"color:var(--muted)\">No notes yet.</p>";
                }
            })
            .catch(function () {
                if (results) results.innerHTML = "<p style=\"color:var(--muted)\">Could not load notes.</p>";
            });
    }

    if (searchInput) {
        loadNotes();
        var timer;
        searchInput.addEventListener("input", function () {
            clearTimeout(timer);
            timer = setTimeout(function () { loadNotes(searchInput.value); }, 300);
        });
    }

    // --- Add note (POST + Vanilo.put) ---
    var form = document.getElementById("add-form");
    if (form) {
        form.addEventListener("submit", function (e) {
            e.preventDefault();
            var title = form.querySelector("[name=title]").value.trim();
            var description = form.querySelector("[name=description]").value.trim();
            if (!title) return;

            fetch("/api/notes", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ title: title, description: description })
            })
                .then(function (r) { return r.json(); })
                .then(function (data) {
                    if (data.ok) {
                        Vanilo.put("#msg", "Tag", { label: "Saved: " + title });
                        form.reset();
                        loadNotes(searchInput ? searchInput.value : "");
                    }
                })
                .catch(function () {
                    Vanilo.put("#msg", "Tag", { label: "Error — try again" });
                });
        });
    }

    // --- GitHub zen quote (fetch demo) ---
    var zenEl = document.getElementById("zen");
    if (zenEl) {
        fetch("/api/github")
            .then(function (r) { return r.json(); })
            .then(function (data) { if (data.zen) zenEl.textContent = data.zen; })
            .catch(function () {});
    }
});
