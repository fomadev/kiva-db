const { KivaClient } = require("./kiva");

async function main() {
  const db = new KivaClient("127.0.0.1", Number(process.argv[2] || 2911));
  db.onEvent = (event) => console.log("event:", event);

  await db.exec('set product:1 {"name": "Pen", "price": 2}');
  console.log(await db.exec("get product:1.name"));
  console.log(await db.exec("count where product:*.price < 10"));

  await db.exec("subscribe product:*");
  await db.exec('json set product:1 .price 3');
  try {
    await db.exec('update product:1 "not an object"');
  } catch (err) {
    console.log("refused:", err.message);
  }
  await db.exec("del product:1");
  setTimeout(() => db.close(), 200);
}

main();
