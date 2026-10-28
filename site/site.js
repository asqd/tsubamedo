// The repository from the address (<owner>.github.io/<repo>/, also <repo>/en/), and the files
// of its latest release: links, sizes and the version come from the GitHub API, so the page
// never needs editing for a release. Without the API the links lead to the release page.
// Every text written here follows the page's <html lang>.
(function () {
  const english = document.documentElement.lang.startsWith("en");
  const text = english
    ? { locale: "en-US", mb: "MB", version: (v, date) => `Version ${v}, released ${date} · `, news: "What’s new" }
    : { locale: "ru-RU", mb: "МБ", version: (v, date) => `Версия ${v} от ${date} · `, news: "Что нового" };

  // The language switcher: the choice is remembered, so the Russian page stops sending this
  // visitor to English, and the section being read is kept.
  for (const a of document.querySelectorAll(".lang a[hreflang]")) {
    a.addEventListener("click", () => {
      try {
        localStorage.setItem("tsubamedo.lang", a.hreflang);
      } catch (e) {
        // Storage blocked: the switch still works, it just isn't remembered.
      }
      a.href = a.getAttribute("href").split("#")[0] + location.hash;
    });
  }

  const fallback = "asqd/tsubamedo";
  const owner = location.hostname.match(/^([^.]+)\.github\.io$/);
  const first = location.pathname.split("/").filter(Boolean)[0];
  const repo = owner && first ? `${owner[1]}/${first}` : fallback;
  const base = `https://github.com/${repo}`;
  const releases = `${base}/releases/latest`;

  for (const a of document.querySelectorAll("a[data-repo]")) a.href = base + a.dataset.repo;
  for (const el of document.querySelectorAll(".repo-url")) el.textContent = base;
  for (const a of document.querySelectorAll("a[data-file]")) a.href = releases;

  // `tsubamedo-*-linux-x86_64.tar.gz`: a pattern of the end of the file's name.
  const matcher = (pattern) => new RegExp(pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*") + "$");
  // «9,2 МБ» or «9.2 MB».
  const mb = (bytes) =>
    `${(bytes / 1048576).toLocaleString(text.locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} ${text.mb}`;

  fetch(`https://api.github.com/repos/${repo}/releases/latest`)
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then((release) => {
      const assets = release.assets.filter((a) => !a.name.endsWith(".sha256"));
      const find = (pattern) => assets.find((a) => matcher(pattern).test(a.name));
      for (const a of document.querySelectorAll("a[data-file]")) {
        const asset = find(a.dataset.file);
        if (!asset) continue;
        a.href = asset.browser_download_url;
        a.title = `${asset.name}, ${mb(asset.size)}`;
      }
      const version = release.tag_name.replace(/^v/, "");
      const date = new Date(release.published_at).toLocaleDateString(text.locale, { day: "numeric", month: "long", year: "numeric" });
      const note = document.getElementById("release");
      note.textContent = text.version(version, date);
      const all = document.createElement("a");
      all.href = release.html_url;
      all.textContent = text.news;
      note.append(all);
      const cli = find("tsubamedo-*-macos-apple-silicon.tar.gz");
      if (cli) document.getElementById("cli-name").textContent = cli.name;
    })
    .catch(() => {
      // No release yet, or the API's hourly limit: the links lead to the release page.
    });
})();
