await open("Товары");
click("Проверки");
await sleep(300);
await send();
document.querySelector(".check-badge")?.click();
await sleep(300);
