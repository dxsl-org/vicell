#!/usr/bin/env python3
"""Hypha mock LLM proxy — host-side plumbing test for the llm-gateway cell.

Purpose (os-gap G3 de-risk): isolate the *Cellos side* of the LLM path
(cell -> net -> TLS -> HTTP -> JSON parse) from the real provider. This is NOT
a real LLM. It speaks TLS 1.3 with a self-signed P-256 cert (Cellos's net cell
uses embedded-tls `UnsecureProvider`, so it does not verify the cert) and
answers any `POST /v1/chat/completions` with an OpenAI-compatible JSON that
echoes the prompt back — proving the round-trip end to end.

Run on the HOST (the guest reaches it at 10.0.2.2:8443 via QEMU user-net):
    python tools/hypha-mock-llm/mock_proxy.py
Then in the Cellos shell:
    /bin/hypha          # or: /bin/llm-gateway  (P0 standalone, if reverted)

Requires QEMU user-mode (SLIRP) networking, where guest 10.0.2.2 == host.
"""

import datetime
import json
import os
import ssl
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

HOST = "0.0.0.0"
PORT = 8443
HERE = os.path.dirname(os.path.abspath(__file__))
CERT = os.path.join(HERE, "cert.pem")
KEY = os.path.join(HERE, "key.pem")


def ensure_cert():
    """Generate a self-signed P-256 cert if missing. embedded-tls supports
    ecdsa_secp256r1_sha256, so we use a P-256 key signed with SHA-256."""
    if os.path.exists(CERT) and os.path.exists(KEY):
        return
    try:
        from cryptography import x509
        from cryptography.x509.oid import NameOID
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import ec
    except ImportError:
        sys.exit(
            "Missing cert.pem/key.pem and the `cryptography` package is not "
            "installed.\nEither `pip install cryptography` and re-run, or "
            "generate manually:\n"
            "  openssl req -x509 -newkey ec "
            "-pkeyopt ec_paramgen_curve:prime256v1 -nodes "
            f"-keyout {KEY} -out {CERT} -days 3650 -subj /CN=10.0.2.2"
        )
    key = ec.generate_private_key(ec.SECP256R1())
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "10.0.2.2")])
    now = datetime.datetime.now(datetime.timezone.utc)
    cert = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=3650))
        .sign(key, hashes.SHA256())
    )
    with open(KEY, "wb") as f:
        f.write(key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        ))
    with open(CERT, "wb") as f:
        f.write(cert.public_bytes(serialization.Encoding.PEM))
    print(f"[mock-llm] generated self-signed P-256 cert: {CERT}")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print("[mock-llm] " + (fmt % args))

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b""
        user = ""
        try:
            msgs = json.loads(raw or b"{}").get("messages", [])
            if msgs:
                user = msgs[-1].get("content", "")
        except Exception:
            pass

        reply = (
            "Mock LLM here — the Cellos TLS+HTTP+JSON path works. "
            "You sent: " + user[:160].replace("\n", " ")
        )
        body = json.dumps({
            "id": "mock-1",
            "object": "chat.completion",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": reply}}],
        }).encode()

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    plain = "--plain" in sys.argv
    port = 8080 if plain else PORT

    httpd = HTTPServer((HOST, port), Handler)
    if plain:
        print(f"[mock-llm] PLAIN HTTP mock LLM listening on {HOST}:{port}")
        print(f"[mock-llm] guest reaches it at 10.0.2.2:{port} (QEMU user-net)")
    else:
        ensure_cert()
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.minimum_version = ssl.TLSVersion.TLSv1_3  # embedded-tls is TLS 1.3 only
        ctx.load_cert_chain(CERT, KEY)
        httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
        print(f"[mock-llm] TLS 1.3 mock LLM listening on {HOST}:{port}")
        print(f"[mock-llm] guest reaches it at 10.0.2.2:{port} (QEMU user-net)")

    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n[mock-llm] bye")


if __name__ == "__main__":
    main()
