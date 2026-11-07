# Phone API v1

One versioned JSON API (and one WebSocket) that the phone web app uses today and that a later Capacitor, React
Native or native app can use unchanged. All logic lives in the PC app (`packages/app/src-tauri/src/potato/phone/`);
a page only draws what this API returns. Types: `packages/shared/src/phone-api.ts`.

- Base: the phone server's origin, `/api/v1/...`. `/api/...` is the same route (v1 is the only version).
- Bodies are JSON (`Content-Type: application/json`), keys are camelCase, times are epoch milliseconds.
- Errors: `{ "error": "code", "message": "plain sentence" }`. 400 bad input, 401 `unpaired`, 403 `not_allowed` /
  `bad_origin` / `bad_request` / `pc_only`, 404, 421 `bad_host`, 429 `rate_limited`, 503 `not_ready`.
- Every request that changes something carries the header `X-Potato-Phone: 1`.
- Auth: `Authorization: Bearer <device token>` on every route except `health`, `hello`, `pair` and the app files.
  There is no cookie and no password. A 401 means the phone is signed out: forget the token and pair again.

## Finding the PC

The PC can be reachable at several origins (a LAN address, the person's own domain, a tunnel). A client keeps a list and
races them with `hello`. `hello` is open but proves nothing unless the PC knows a secret:

`POST /hello { kind: "pair"|"device", nonce, deviceId? }` returns `{ proof, pcName }` where
`proof = hex(HMAC-SHA256(key, "potato-phone-hello|" + nonce))`. The nonce is 16 to 64 letters or digits, fresh each time.
`key` is `SHA-256(pairing code, uppercase UTF-8)` for `pair`, and `SHA-256(the token's raw bytes)` for `device` (the token
is base64; the bytes are what you get after decoding it). A machine that answers but cannot produce the proof is not the
PC. An unknown code or phone still gets a 64-character answer (under a throwaway key), so the answer tells nothing.

Cross-origin calls (the app on one address talking to another) work only between the server's own addresses: the server
answers CORS for exactly those origins, never `*`.

## Pairing

The PC shows a QR (Settings > Phone). Its address is `<origin>/#/pair/<CODE>?a=<origin>&a=<origin>...`. The code and the
other addresses are in the fragment, so no server or proxy log ever sees them. A code works once and for 10 minutes;
five wrong guesses burn it. `POST /pair` is limited to 10 tries a minute (4 through a tunnel).

`POST /pair { code, deviceName }` returns `{ deviceId, token, pcName, candidates }`. The token is 32 random bytes; the PC
keeps only its SHA-256. A code made with "Re-link" gives an existing phone a new token and keeps its id, name and abilities,
so a changed address never loses a pairing. Nothing in pairing knows an address, so a later stable address needs no change.

Abilities (shown on the PC, per phone): `watch` (read), `type` (send messages), `answer` (cards), `control` (stop,
pause). A phone without one gets 403 `not_allowed`.

## Routes (token required)

| Route | Ability | Answer |
|---|---|---|
| `GET /me` | any | `{ deviceId, name, pcName, caps, groups, paused, candidates }` (use `candidates` to refresh your address list) |
| `DELETE /me` | any | `{ ok }` this phone signs itself out |
| `GET /bots` | watch | `{ rev, paused, bots: [{ id, name, state, stateText, lastLine, lastAt, unread, openCards, typing }], rooms }`. `state`: `working`, `needs_you`, `idle`, `offline`, `problem` |
| `GET /bots/:id?limit=200` | watch | `{ rev, bot, messages, cards, job, work, typing }` |
| `POST /bots/:id/read` | watch | `{ ok }` clears the unread count |
| `POST /bots/:id/send { text, files? }` | type | `{ id, delivered, state }`. `files`: up to 4 `{ name, kind: "image", data: base64 }` |
| `POST /bots/:id/cards/:card/answer { answer }` | answer | `{ ok, answer }` |
| `POST /bots/:id/stop` | control | `{ ok }` ends the bot's job |
| `POST /pause-all`, `POST /resume-all` | control | `{ paused }` |
| `GET /rooms/:id`, `POST /rooms/:id/send { text }` | watch / type | group chat (only when Bot groups is on at the PC) |
| `GET /changes?since=N&wait_ms=20000` | any | `{ rev }` LONG POLL: answers at once when `rev > since`, else when something changes or after `wait_ms` (at most 25 s) |
| `GET /push/key`, `POST /push/subscribe`, `DELETE /push/subscribe`, `POST /push/presence` | watch / any | Web Push, below |

Card answers: an approval takes `allow_once`, `allow_job` or `deny`; a question takes the choice number as a string.
"Always allow" does not exist on the phone. A sign-in card (`login` set) can only be dismissed with its last choice
("Not now"); the first two get 403 `pc_only`. On a connector card (`connector` set) choice 0 opens PC settings, so it is
`pc_only`. No route takes a password, key or token for a bot: secrets never pass through the phone.

## Live updates

Server-Sent Events do not work through Cloudflare quick tunnels, so the web app uses the long poll (`/changes`):
one request waits, then the app re-reads the screen it is showing. A client that prefers a socket can open

`GET /ws` with `Upgrade: websocket` and `Sec-WebSocket-Protocol: pm.v1, pm-bearer.<token>`.

The server answers `pm.v1` (the token is never echoed) and sends text frames: `{"type":"changed","rev":N}` once at once,
then on every change; `{"type":"signed_out"}` and a close when the phone is revoked; a ping every 10 s when quiet.
After `changed`, call the read routes. The socket carries no data of its own.

## Push

The PC sends Web Push itself, with its own VAPID key (no hosted service). `GET /push/key` gives `{ publicKey }`
(base64url, for `applicationServerKey`). `POST /push/subscribe { endpoint, keys: { p256dh, auth } }` stores the
phone's subscription (only known push services are accepted: FCM, Apple, Mozilla, Windows). `POST /push/presence
{ visible }` says whether the app is open (an open app gets no push). A push says "Nimbus needs you" when a card waits
and "Nimbus finished" when a job ends; the body is blank if the phone hides details. Android Chrome and iOS 16.4+ (app on
the Home Screen) both receive it through their own push service.

## Native wrappers

A Capacitor or React Native app keeps the same token (in the secure store), the same list of origins, and the same
calls. Differences: a native app has no Origin and no mixed-content rule, so it can reach `http://` LAN addresses and the
`https://` tunnel without CORS; use `pm-bearer` on the socket. Push there would use the platform's own service (FCM/APNs)
instead of Web Push, which needs a server-side key and is not built.

## Safety rules the server enforces (and tests)

Public peers (not this PC, not a private network) are dropped unread. `Host` must be an IP, `localhost`, `*.local`, this
computer, the person's own address, or the running tunnel. `Origin` must be the server's own or one of its addresses.
Forwarder headers (`Cf-Connecting-Ip`, `X-Forwarded-For`, ...) never earn trust: loopback is not trust, only a valid token is.
They only put a request on a tighter rate limit.
