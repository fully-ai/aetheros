# Aether Browser — Native HTML/CSS/TLS Web Browser (`BROWSER.md`)

## 1. Architecture & Network Integration

**Aether Browser** (`AppKind::Browser` in `kernel/src/gui/mod.rs` + `net::http_get` in `kernel/src/net/mod.rs`) is a native lightweight web browser integrated directly with AetherOS's RFC 1035 DNS resolver, RFC 9293 TCP state machine, HTTP/1.1 client, and X.509 TLS certificate verifier.

## 2. Features & Security Controls

1. **Interactive Bookmark & Target Bar**:
   - `[aetheros.org]` (`https://aetheros.org` — Official AetherOS Portal)
   - `[Docs]` (`https://docs.aetheros.org` — Architecture & Syscall Handbook)
   - `[Store]` (`https://store.aetheros.org` — Signed AetherPKG Repository Index)
   - `[Example]` (`http://example.com` — Standard RFC Example Domain `93.184.216.34`)
   - `[BadSSL!]` (`https://expired.badssl.com` — Live test of X.509 expired certificate blocking)
   - `[Download]` (Saves the rendered page to `/home/aether/Downloads/page.txt` without execution permissions and triggers a desktop notification).
2. **TLS Security Padlock & Certificate Validation**:
   - Displays a green **`[TLS 1.3 OK]`** badge (`TLS_AES_256_GCM_SHA384 (X.509 Verified)`) for verified HTTPS origins.
   - Displays a red **`[TLS BLOCKED]`** alert and refuses to render untrusted content when an expired (`TlsCertificateExpired`) or untrusted-root (`TlsCertificateUntrusted`) certificate is encountered.
   - Enforces a maximum redirect limit of 5 hops (`NetError::TooManyRedirects`).
3. **HTML DOM & Typography Renderer**:
   - Parses `<title>`, `<h1>`, `<h2>`, `<p>`, `<ul>`, `<li>`, and `<a href="...">`, rendering color-coded headings, bullet lists, and link highlights.
