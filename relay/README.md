# Blankey relay protocol (v1)

Blankey's MCP server only listens on 127.0.0.1. A relay lets web-based agents (claude.ai custom connectors and other
remote MCP clients) reach it: the agent talks OAuth 2.1 + streamable HTTP MCP to the relay, and the relay forwards each
MCP request over a WebSocket that the desktop app keeps open to it. The app never accepts inbound connections.

The official relay is run by the Blankey project. Anyone can run their own: implement this document and point the app
at it (Connections → "Use custom relay…"). Billing and accounts are up to each relay.

```
agent --OAuth 2.1 + DCR--> relay (authorization server, MCP resource)
agent --POST /mcp (Bearer)--> relay --WebSocket frame--> Blankey app --> local MCP server (in process)
```

## Trust model

- The relay sees MCP traffic: profile and field metadata, templates, example data and request status. Blankey's tools
  never return vault values (see `mcp::tools::tests::no_tool_leaks_vault_values`), so real personal data does not pass
  through it.
- The device has the final say. An agent only gets tokens after the user accepts it in the app (`approve` frame), and
  each of the user's computers asks before it serves an agent for the first time.
- The app keeps its own allowlist of accepted grants and answers 403 to MCP frames for any other grant, so a relay
  cannot attach an agent without the user.
- Remote sessions cannot read local files by path; agents get files through `request_file`, which the user answers
  with a file picker; the agent gets a file id and size, never the file name. Each agent (grant) only sees its own
  requests and files. Long waits are capped (`wait_request` ≤ 50 s per call over a relay).

## Discovery

`GET {relay}/.well-known/blankey-relay` → `200 application/json`

```json
{
  "protocol": 1,
  "issuer": "https://relay.example.com",
  "device_register": "https://relay.example.com/device/register",
  "device_connect": "wss://relay.example.com/device/connect",
  "device_revoke": "https://relay.example.com/device/grants/revoke",
  "mcp_url": "https://relay.example.com/mcp",
  "account_url": "https://relay.example.com/account"
}
```

The app refuses a relay whose `protocol` it does not support, and any URL that is not `https`/`wss` unless the host is
`localhost` or `127.0.0.1` (for development). `account_url` is shown to the user (sign up, billing, devices).

## OAuth

The relay is an OAuth 2.1 authorization server:

- RFC 8414 metadata at `{issuer}/.well-known/oauth-authorization-server`
- RFC 9728 protected-resource metadata at `{issuer}/.well-known/oauth-protected-resource`. The resource is the
  relay's origin (`issuer`), so one token audience covers `mcp_url` and the device endpoints; scopes keep them apart.
  Clients that send `resource` must send that value. `401` responses from `mcp_url` carry
  `WWW-Authenticate: Bearer resource_metadata="<that url>"`; a token with the wrong scope gets `403`
  `insufficient_scope`
- RFC 7591 dynamic client registration; public clients (`token_endpoint_auth_method: none`)
- Authorization code with PKCE (`S256` only), refresh tokens with rotation

Two scopes:

| Scope | Client | Used for |
|---|---|---|
| `device` | the Blankey app | `device_register`, `device_connect`, `device_revoke` |
| `mcp` | agents | `mcp_url` |

A client asks for one scope at a time. Relays should only grant `device` to clients whose redirect URI is a
loopback address, so a web agent cannot obtain a device token.

### Desktop app

1. Registers itself (DCR) with `client_name: "Blankey"` and the redirect URI
   `http://127.0.0.1:{port}/relay/callback`, where `{port}` is the app's local MCP port. It registers again when the
   port changes. Relays should also accept any port on a loopback redirect (RFC 8252 §7.3).
2. Opens the system browser on the authorization endpoint with `scope=device`, PKCE and `state`; the user signs in on
   the relay.
3. Exchanges the code; keeps the refresh token in the OS keychain.
4. `POST device_register` with `Authorization: Bearer <device token>` and `{"name": "<host name>", "platform":
   "macos|windows|linux"}` → `200 {"device_id": "<id>", "account": "<email>"}`. The id is stable; the app stores it and
   sends it again on later registrations (`{"device_id": ...}`) to keep the same device. `account` is optional: the
   signed-in account as the user knows it, shown in the app.

### Agents

The agent registers (DCR) and starts authorization with `scope=mcp`. The relay:

1. signs the user in;
2. may refuse with an explanation page (no subscription etc.);
3. creates a grant id and sends `approve` to the primary device (see below); when no device is online, or it does not
   answer within 120 s, the authorization fails with `access_denied`;
4. on `accepted: true` issues the code. All tokens from this consent are tied to the grant id, which belongs to the
   account, not to one device. A new authorization by the same client may replace that client's earlier grant.

### Primary device

Only one device per account receives agent requests: the primary one. The app claims it when the user unlocks
Blankey (`primary` frame below); the primary device is the connected one that claimed it last, or, when none of the
connected devices has, the one seen most recently. Other devices stay connected and take over when the primary one
goes offline.

The first time a grant reaches a device that has not answered for it, the relay sends that device an `approve` frame
for the existing grant and forwards the request only once it is accepted. The relay remembers each device's answer;
requests that arrive while the question is open wait for the same answer. A device that refused gets `403` with a
JSON-RPC error for that grant from then on.

## Device WebSocket

`GET device_connect?device_id=<id>` with `Authorization: Bearer <device token>`, upgraded to a WebSocket.
`401` means refresh the token; `402` means the account needs attention (the app shows `account_url`); `404` means the
device is unknown (register again). One connection per device: a new one replaces the old one.

Every frame is a JSON text message with `"v": 1` and a `type`. Unknown types are ignored. `id` is a string chosen by
the sender of the request and echoed in the reply.

### Keep-alive

The app sends `{"v":1,"type":"ping"}` every 30 s; the relay replies `{"v":1,"type":"pong"}`. The app reconnects after
a missed pong or a close, with exponential backoff from 1 s to 60 s plus jitter.

### MCP

Relay → app, for each HTTP request the agent sends to `mcp_url`:

```json
{"v": 1, "type": "mcp", "id": "r-17", "grant": "g-abc", "client_name": "Claude",
 "method": "POST", "headers": {"content-type": "application/json", "accept": "application/json, text/event-stream",
 "mcp-session-id": "…", "mcp-protocol-version": "2025-06-18"}, "body": "<JSON-RPC text>"}
```

Forward only `content-type`, `accept`, `mcp-session-id`, `mcp-protocol-version` and `last-event-id`; drop any absent
ones. `method` is `POST`, `GET` or `DELETE`.

App → relay:

```json
{"v": 1, "type": "mcp_result", "id": "r-17", "status": 200,
 "headers": {"content-type": "application/json", "mcp-session-id": "…"}, "body": "<JSON-RPC text>"}
```

The relay returns `status`, `headers` and `body` to the agent as the HTTP response. The app's server runs in JSON
response mode, so bodies are complete JSON, not streams. If the grant is not on the app's allowlist the app answers
`status: 403`. If the device is not connected the relay answers the agent `503` with a JSON-RPC error
(`{"jsonrpc":"2.0","id":<request id or null>,"error":{"code":-32000,"message":"Blankey is not running on your computer"}}`).
The relay waits at least 60 s for an `mcp_result` and then answers `504` with a JSON-RPC error.

### Primary

App → relay when the user unlocks Blankey: `{"v": 1, "type": "primary", "id": "p-1"}`; the relay makes this device
the primary one and echoes the frame back. When the app is not connected at the time, it sends the claim after
connecting, and keeps sending it on each new connection until the relay has echoed it. A reconnect alone does not
claim.

### Approval

Relay → app during agent authorization, and when a grant first reaches a device that has not answered for it:

```json
{"v": 1, "type": "approve", "id": "a-3", "grant": "g-abc", "client_name": "Claude",
 "redirect_host": "claude.ai"}
```

App → relay: `{"v": 1, "type": "approval", "id": "a-3", "accepted": true}`. The app asks the user (and Touch ID
where available) and adds the grant to its allowlist only when accepted.

### Grants

App → relay: `{"v": 1, "type": "grants", "id": "q-1"}`; relay → app:
`{"v": 1, "type": "grants", "id": "q-1", "grants": [{"grant": "g-abc", "client_name": "Claude",
"created_at": "2026-10-12T10:00:00Z"}]}`: the grants this device accepted that the relay still honours. The app shows
them on the Connections page.

## Revoking

`POST device_revoke` with the device token and `{"grant": "g-abc"}` → `204`. The relay must stop accepting tokens of
that grant immediately, on every device of the account. The app removes the grant from its allowlist first, so it is
refused even if this call fails.

## Running your own relay

Any stack works (Workers, Node, Go…) as long as it speaks the above over TLS. Checklist:

- discovery document, OAuth metadata, DCR, PKCE, refresh tokens, `device` and `mcp` scopes
- device registration and one WebSocket per device, `ping`/`pong`
- `primary` claims; agent requests only to the primary device, falling back to another connected one
- `approve` before issuing agent codes, and on a device's first request for a grant; tokens tied to the grant
- `mcp` frames per request, 60 s wait, 503 when no device is online, 403 for a grant the device refused
- revocation

To test: run it locally, set the app's relay to `http://localhost:<port>`, sign in from the app, then connect
[MCP Inspector](https://github.com/modelcontextprotocol/inspector) to `mcp_url`: it registers, you accept it in
Blankey, and `tools/list` should list Blankey's tools. The app's own tests (`src-tauri/src/relay/`) run a stub relay
that exercises the frame handling and can serve as a reference.
