# Авторитетный DNS для проб: отвечает по типу запроса. UDP и TCP на одном адресе, плюс UDP-эхо на 9999.
import socket, struct, sys, threading
ADDR = sys.argv[1]
def name(labels): return b"".join(bytes([len(l)]) + l for l in labels) + b"\0"
def answer(q, tcp):
    end = 12; labels = []
    while q[end] != 0:
        n = q[end]; labels.append(q[end + 1:end + 1 + n]); end += n + 1
    qtype = struct.unpack(">H", q[end + 1:end + 3])[0]
    question = q[12:end + 5]
    first = labels[0].lower()
    def reply(flags, rdata=None, rtype=None):
        an = 0 if rdata is None else 1
        r = q[:2] + struct.pack(">HHHHH", flags, 1, an, 0, 0) + question
        if rdata is not None:
            r += b"\xc0\x0c" + struct.pack(">HHIH", rtype or qtype, 1, 60, len(rdata)) + rdata
        return r
    if first == b"nx": return reply(0x8183)
    if first == b"big" and not tcp: return reply(0x8380)
    if qtype == 1: return reply(0x8180, bytes([198, 51, 100, 2]))
    if qtype == 28: return reply(0x8180, socket.inet_pton(socket.AF_INET6, "2001:db8:2::2"))
    if qtype == 16:
        text = (b"x" * 200) if first == b"big" else b"cm-probe"
        return reply(0x8180, bytes([len(text)]) + text)
    if qtype == 15: return reply(0x8180, struct.pack(">H", 10) + name([b"mail", b"test"]))
    if qtype == 33: return reply(0x8180, struct.pack(">HHH", 0, 0, 8080) + name([b"srv", b"test"]))
    if qtype in (64, 65): return reply(0x8180, struct.pack(">H", 1) + b"\0")
    return reply(0x8180)
def udp():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind((ADDR, 53))
    while True:
        q, a = s.recvfrom(4096)
        try: s.sendto(answer(q, False), a)
        except Exception: pass
def tcp():
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind((ADDR, 53)); s.listen(8)
    while True:
        c, _ = s.accept()
        try:
            n = struct.unpack(">H", c.recv(2))[0]; r = answer(c.recv(n), True); c.sendall(struct.pack(">H", len(r)) + r)
        except Exception: pass
        c.close()
def echo():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind((ADDR, 9999))
    while True:
        d, a = s.recvfrom(2048); s.sendto(b"echo:" + d, a)
for f in (udp, tcp): threading.Thread(target=f, daemon=True).start()
echo()
