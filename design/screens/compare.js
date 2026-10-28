await open(L("Товары", "Products"));
await send();
click(L("Сравнить…", "Compare…"));
await sleep(500);
click(L("С окружением", "With environment"));
await sleep(300);
click("prod");
await sleep(1800);
