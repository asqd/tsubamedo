# Demo API for the screenshots (design/screens/make.sh): stage on :8771 and prod on :8772,
# the same API with slightly different answers; under /en the same in English (shop-en).
import http.server, json, threading, uuid
DATA = {
    8771: {"version": "1.2.0", "items": [{"id": 1, "name": "Кофе", "price": 120}, {"id": 2, "name": "Чай", "price": 90}], "total": 2},
    8772: {"version": "1.3.0", "items": [{"id": 1, "name": "Кофе", "price": 135}, {"id": 3, "name": "Какао", "price": 110}, {"id": 2, "name": "Чай", "price": 90}], "total": 3},
}
README = {8771: "Магазин API\nверсия 1.2\nскидки: нет\nдоставка: 2 дня\n", 8772: "Магазин API\nверсия 1.3\nскидки: 5%\nдоставка: 2 дня\nсамовывоз: да\n"}
EN_NAMES = {"Кофе": "Coffee", "Чай": "Tea", "Какао": "Cocoa"}
EN_README = {8771: "Shop API\nversion 1.2\ndiscounts: none\ndelivery: 2 days\n", 8772: "Shop API\nversion 1.3\ndiscounts: 5%\ndelivery: 2 days\npickup: yes\n"}

def english(handler):
    """/en/… is the same API in English: the prefix is dropped and True returned."""
    if handler.path.startswith("/en/"):
        handler.path = handler.path[3:]
        return True
    return False
def make(port):
    class H(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            en = english(self)
            if self.path == "/readme":
                b, t = (EN_README if en else README)[port].encode(), "text/plain; charset=utf-8"
            else:
                d = dict(DATA[port]); d["requestId"] = str(uuid.uuid4())
                if en:
                    d["items"] = [dict(i, name=EN_NAMES[i["name"]]) for i in d["items"]]
                b, t = json.dumps(d, ensure_ascii=False).encode(), "application/json"
            self.send_response(200); self.send_header("Content-Type", t); self.send_header("X-Request-Id", str(uuid.uuid4()))
            self.send_header("X-Version", DATA[port]["version"]); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
        def do_POST(self):
            english(self)
            n = int(self.headers.get("Content-Length") or 0)
            order = json.loads(self.rfile.read(n) or b"{}")
            prices = {i["id"]: i["price"] for i in DATA[port]["items"]}
            total = sum(prices.get(i["id"], 0) * i.get("qty", 1) for i in order.get("items", []))
            d = {"id": 1042, "status": "new", "items": order.get("items", []), "total": total, "delivery": order.get("delivery"), "requestId": str(uuid.uuid4())}
            b = json.dumps(d, ensure_ascii=False, indent=2).encode()
            self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
        def log_message(self, *a): pass
    return http.server.ThreadingHTTPServer(("127.0.0.1", port), H)
threading.Thread(target=make(8771).serve_forever, daemon=True).start()
make(8772).serve_forever()
