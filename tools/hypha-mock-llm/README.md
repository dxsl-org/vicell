# Hypha mock LLM proxy

A host-side **plumbing test** for the Hypha `llm-gateway` cell. It is **not** a real LLM — it
isolates the *Cellos side* of the LLM path (cell → net → TLS → HTTP → JSON parse) from the real
provider, so we can verify that path before tackling public DNS / CA trust / the provider's real
API (os-gap **G3**).

## What it does

- Speaks **TLS 1.3** with a self-signed **P-256** cert (auto-generated on first run).
  Cellos's net cell uses embedded-tls `UnsecureProvider` — it does **not** verify the server cert
  (see os-gap **G14**), so a self-signed cert is accepted.
- Answers any `POST /v1/chat/completions` with an OpenAI-compatible JSON whose
  `choices[0].message.content` **echoes the prompt back** — proving the round-trip end to end.

## Run

On the **host** (needs Python 3; `cryptography` is used for cert-gen in TLS mode, else it prints an
`openssl` one-liner):

```sh
# Plaintext (easiest — matches the gateway default USE_TLS=false, port 8080):
python tools/hypha-mock-llm/mock_proxy.py --plain
# [mock-llm] PLAIN HTTP mock LLM listening on 0.0.0.0:8080

# TLS 1.3 (set USE_TLS=true in llm-gateway/src/main.rs, port 8443):
python tools/hypha-mock-llm/mock_proxy.py
# [mock-llm] TLS 1.3 mock LLM listening on 0.0.0.0:8443
```

The gateway defaults to **plaintext** (`USE_TLS = false` in
`cells/apps/hypha/llm-gateway/src/main.rs`) — pair it with `--plain`. Flip both for TLS.

Then boot Cellos with **QEMU user-mode (SLIRP) networking** (the default), where the guest reaches
the host at `10.0.2.2`. In the Cellos shell:

```
/bin/hypha           # P1 interactive chat (spawns llm-gateway)
```

The gateway is pinned to `10.0.2.2:8443` (`PROXY_IP`/`PROXY_PORT` in
`cells/apps/hypha/llm-gateway/src/main.rs`).

## Verified (host side)

`python` TLS-1.3 client → `POST /v1/chat/completions` → `200` → echo JSON. Confirms TLS handshake,
HTTP request/response, and JSON shape. The Cellos-guest → host hop still needs a boot run to confirm
QEMU user-net routing + the gateway's hand-rolled HTTP/JSON against this server.

## Files

- `mock_proxy.py` — the server.
- `cert.pem` / `key.pem` — auto-generated self-signed cert (gitignored; regenerated if missing).
