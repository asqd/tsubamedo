await open("Создать заказ");
click("Код…");
await sleep(500);
pick(document.querySelector(".code-dialog select"), "python-requests");
await sleep(800);
