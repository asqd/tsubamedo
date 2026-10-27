// The repository from the address (<owner>.github.io/<repo>/), and the files of its latest
// release: links, sizes and the version come from the GitHub API, so the page never needs
// editing for a release. Without the API the links lead to the release page.
(function () {
  const fallback = "asqd/tsubamedo";
  const owner = location.hostname.match(/^([^.]+)\.github\.io$/);
  const first = location.pathname.split("/").filter(Boolean)[0];
  const repo = owner && first ? `${owner[1]}/${first}` : fallback;
  const base = `https://github.com/${repo}`;
  const releases = `${base}/releases/latest`;

  for (const a of document.querySelectorAll("a[data-repo]")) a.href = base + a.dataset.repo;
  for (const el of document.querySelectorAll(".repo-url")) el.textContent = base;
  for (const a of document.querySelectorAll("a[data-file]")) a.href = releases;

  // The main button follows the visitor's system; a Mac cannot tell its processor to a page.
  const ua = navigator.userAgent;
  const primary = document.querySelector(".cta .primary");
  const second = document.querySelector('.cta [data-file="macos-intel.dmg"]');
  if (/Windows/.test(ua)) {
    primary.dataset.file = "windows-x64-setup.exe";
    primary.textContent = "Скачать для Windows";
    second.hidden = true;
  } else if (/Linux/.test(ua) && !/Android/.test(ua)) {
    primary.dataset.file = "linux-x86_64.AppImage";
    primary.textContent = "Скачать AppImage для Linux";
    second.hidden = true;
  }

  // `tsubamedo-*-linux-x86_64.tar.gz`: a pattern of the end of the file's name.
  const matcher = (pattern) => new RegExp(pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*") + "$");
  const mb = (bytes) => `${(bytes / 1048576).toFixed(1).replace(".", ",")} МБ`;

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
      const date = new Date(release.published_at).toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: "numeric" });
      const dmg = find("macos-apple-silicon.dmg");
      const note = document.getElementById("release");
      note.textContent = `Версия ${version} от ${date}${dmg ? ` · DMG ${mb(dmg.size)}` : ""} · `;
      const all = document.createElement("a");
      all.href = release.html_url;
      all.textContent = "все файлы и суммы SHA-256";
      note.append(all);
      const cli = find("tsubamedo-*-macos-apple-silicon.tar.gz");
      if (cli) document.getElementById("cli-name").textContent = cli.name;
    })
    .catch(() => {
      // No release yet, or the API's hourly limit: the links lead to the release page.
    });
})();
