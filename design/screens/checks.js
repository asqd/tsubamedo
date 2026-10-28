await open(L("Товары", "Products"));
click(L("Проверки", "Checks"));
await sleep(300);
await send();
document.querySelector(".check-badge")?.click();
await sleep(300);
