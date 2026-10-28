// Helpers for the screenshot steps, prepended to each of them by make.sh.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// A text of the window in its language: the steps run on the Russian and the English one.
const L = (ru, en) => (document.documentElement.lang === "en" ? en : ru);
// A button by its text, or the one it starts (tabs carry a ● or a count after it).
function button(text) {
  const all = [...document.querySelectorAll("button")];
  const label = (b) => (b.querySelector(".ell") ?? b).textContent.trim();
  return all.find((b) => label(b) === text) ?? all.find((b) => label(b).startsWith(text));
}
const click = (text) => {
  const b = button(text);
  if (!b) throw new Error(`no button "${text}"`);
  b.click();
};
async function open(name) {
  const row = [...document.querySelectorAll(".item")].find((r) => r.querySelector(".ell")?.textContent.trim() === name);
  if (!row) throw new Error(`no request "${name}"`);
  row.click();
  await sleep(500);
}
async function send() {
  click(L("Отправить", "Send"));
  await sleep(1200);
}
// The language of «Код…» by its name in the list: «requests».
async function language(name) {
  document.querySelector(".language-picker .language").click();
  await sleep(200);
  const item = [...document.querySelectorAll(".language-menu .menu-item")].find((b) => b.textContent.trim().startsWith(name));
  if (!item) throw new Error(`no language "${name}"`);
  item.click();
}
