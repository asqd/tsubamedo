// Helpers for the screenshot steps, prepended to each of them by make.sh.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// A button by its text, or the one it starts (tabs carry a ● or a count after it).
function button(text) {
  const all = [...document.querySelectorAll("button")];
  const label = (b) => (b.querySelector(".ell") ?? b).textContent.trim();
  return all.find((b) => label(b) === text) ?? all.find((b) => label(b).startsWith(text));
}
const click = (text) => {
  const b = button(text);
  if (!b) throw new Error(`нет кнопки «${text}»`);
  b.click();
};
async function open(name) {
  const row = [...document.querySelectorAll(".item")].find((r) => r.querySelector(".ell")?.textContent.trim() === name);
  if (!row) throw new Error(`нет запроса «${name}»`);
  row.click();
  await sleep(500);
}
async function send() {
  click("Отправить");
  await sleep(1200);
}
function pick(select, value) {
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(select, value);
  select.dispatchEvent(new Event("change", { bubbles: true }));
}
