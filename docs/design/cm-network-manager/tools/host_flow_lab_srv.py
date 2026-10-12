import http.server, socket, sys, threading
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = ("from=" + self.client_address[0] + "\n").encode()
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
def dns(addr):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind((addr, 53))
    log = open(sys.argv[1], "a")
    while True:
        q, a = s.recvfrom(2048)
        log.write(a[0] + "\n"); log.flush()
        # header: id, flags 0x8180, qd=1, an=1; question echoed; answer A 198.51.100.2
        end = 12
        while q[end] != 0: end += q[end] + 1
        end += 5
        r = q[:2] + b"\x81\x80\x00\x01\x00\x01\x00\x00\x00\x00" + q[12:end] + b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04" + bytes([198, 51, 100, 2])
        s.sendto(r, a)
for addr in sys.argv[2:]:
    threading.Thread(target=dns, args=(addr,), daemon=True).start()
class S6(http.server.ThreadingHTTPServer):
    address_family = socket.AF_INET6
threading.Thread(target=lambda: S6(("::", 8086), H).serve_forever(), daemon=True).start()
http.server.ThreadingHTTPServer(("0.0.0.0", 8080), H).serve_forever()
