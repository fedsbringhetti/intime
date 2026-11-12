# 17. Web Push

> Phase 3, step 4 · PR [#24](https://github.com/fedsbringhetti/intime/pull/24)

## Goal

The service can now deliver a notification to the phone: the server holds a VAPID identity, stores the devices that subscribed, encrypts and signs a message for each and hands it to the browser's push service, and the service worker shows it. A new screen, **Notifiche**, turns notifications on for the device and sends a test. Nothing sends on its own yet: the scheduler comes with step 5, tracking with step 6, the rules screens with step 7.

## Decisions

| Decision | Why |
|----------|-----|
| **`web-push-native` without its `vapid` feature, JWT signed by our code** (decision AK) | The crate does the `aes128gcm` encryption (RFC 8291) and builds the HTTP request. Its `vapid` feature would pull `jwt-simple`, and with it `rsa` 0.9 (RUSTSEC-2023-0071, no fix available): 83 new crates instead of 37, and a vulnerable dependency for a function we use with ES256 only. The JWT is three base64url parts and one ECDSA signature, signed with `p256`. |
| **The `notifications` table waits for step 5** (decision AL) | Nothing but the test sends before the scheduler, and the scheduler decides what a log row holds. The test notification is not recorded. |
| **Only `https` endpoints of known push services** (decision AM) | A subscription's endpoint comes from the client: without a policy, the server could be made to POST to any URL. The host must be, or be a subdomain of, `fcm.googleapis.com`, `push.apple.com`, `push.services.mozilla.com` or `notify.windows.com`. The policy is a value in `AppState`, so integration tests allow the fake push service on `http://127.0.0.1:<port>`. |
| **Default port only, redirects not followed** | Found in review. With a custom port (`https://fcm.googleapis.com:8443`) the host check passes but the service is not the push service; and reqwest follows redirects by default, so a push service host answering 307 or 308 would have the encrypted body re-posted to another host, outside the policy. Now a port is refused when HTTPS is required, and a 3xx counts as `failed`. |
| **`TTL: 900`, `Urgency: high`, JWT valid 1 hour** (decision AN) | A departure reminder is useless after 15 minutes, so the push service drops it for a phone that stays offline. `high` asks for delivery at once, also on a phone in battery saving. The JWT lives one hour (services accept up to 24). |
| **`Notifier` as a trait object returning a boxed future** (decision AO) | The scheduler (step 5) and the API need one thing: "send this". `AppState` holds `Arc<dyn Notifier>`: Web Push with a key, `LogNotifier` without. One device failing never stops the others; 404 and 410 delete the subscription. |
| **The subscription is sent again when the screen opens** (decision AP) | The browser can lose its subscription or the server its row (a 410, a restored backup). On open, with permission granted, the web app POSTs the existing subscription again (upsert by endpoint); if it was made with another key, it unsubscribes and subscribes again, so the server never keeps one it cannot use. |
| **Permission only after a tap** | iOS refuses the permission prompt outside a user gesture, and a screen that asks for permission when it opens is annoying anyway. `enablePush` runs only from the button. |
| **States without a mockup are text-only** (decision AQ) | Blocked, push disabled on the server, browser not supported and network error use the same card as "off" with different text, and the iPhone not opened from the Home Screen gets the yellow warning of the mockup, linking to `/info`. No new mockup. |
| **The device label comes from the `User-Agent`** (decision AR) | `iPhone`, `iPad`, `Android`, `Mac`, `Windows`, or none. It names the device in the future "devices" list without asking the user anything. |
| **Upsert written insert-first** | Found in review. Select-then-insert lets two concurrent subscriptions of one device both take the insert path and one fails on the `UNIQUE` constraint. The statement order below avoids it. |
| **"Invia una prova" returns after 4 s** | The button says "Inviata" so the user sees the result, then goes back to "Invia una prova" by itself: a second test is a normal thing to do. |
| **Public key derived from the private one** | One variable less to keep consistent: `VAPID_PRIVATE_KEY` is enough, and the public key is computed at startup. |

## Files

| File | Role |
|------|------|
| `migrations/0005_push_subscriptions.sql` | the `push_subscriptions` table |
| `src/notify/vapid.rs` | `VapidKey`: parsing, generation, the `Authorization: vapid ...` header with its JWT |
| `src/notify/mod.rs` | `Notification` and its payload, `SendReport`, `NotifyError`, the `Notifier` trait |
| `src/notify/webpush.rs` | `WebPushNotifier`: encryption, headers, sending, 404 and 410 |
| `src/notify/log.rs` | `LogNotifier`, used without a VAPID key |
| `src/push.rs` | `SubscriptionStore`, `decode_keys`, `EndpointPolicy` |
| `src/api/push.rs`, `src/api/notifications.rs` | the four endpoints |
| `src/api/mod.rs`, `src/api/error.rs`, `src/lib.rs`, `src/main.rs` | routes, new `AppState` fields, `push_disabled`, `vapid-keys` and the wiring |
| `src/config.rs`, `Cargo.toml`, `.env.example` | `VAPID_PRIVATE_KEY`, `VAPID_SUBJECT`; `p256`, `base64ct`, `web-push-native` |
| `tests/push.rs`, `tests/common/mod.rs` | integration tests; `Options::vapid_key` and `push_endpoints` |
| `web/src/lib/push.ts`, `pushPayload.ts` (+ tests) | the screen's state, subscribing, reading a payload |
| `web/src/screens/Notifications.svelte` | the Notifiche screen |
| `web/src/sw.ts`, `web/tsconfig.sw.json` | `push` and `notificationclick` |
| `web/src/lib/api.ts`, `router.ts`, `tabs.ts`, `components/TabBar.svelte`, `components/Icon.svelte`, `App.svelte` | API calls, `/notifiche`, the fifth bottom-bar entry, the bell icon |

## How the code works

### The VAPID key and `vapid-keys`

Push services want to know who sends. **VAPID** (RFC 8292) is a P-256 key pair: the browser is told the public key when it subscribes (`applicationServerKey`), and every push is signed with the private one. `VapidKey` wraps `p256`'s signing key and parses the private scalar from text, base64url without padding, 32 bytes:

```rust
impl FromStr for VapidKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let bytes = Base64UrlUnpadded::decode_vec(value.trim())
            .map_err(|_| "the key must be base64url without padding".to_owned())?;
        if bytes.len() != 32 {
            return Err(format!("the key must be 32 bytes, got {}", bytes.len()));
        }
        SigningKey::from_slice(&bytes)
            .map(Self)
            .map_err(|_| "the key is not a valid P-256 private key".to_owned())
    }
}
```

> **Rust concept: `FromStr`.** Implementing the trait gives `"...".parse::<VapidKey>()`, which is what the configuration code calls. The error type is a `String` because the message goes straight into a `ConfigError`. `Debug` is written by hand and prints `VapidKey(***)`, like the API token, so a key cannot end up in a log through `{:?}` of the configuration.

The private key lives in `VAPID_PRIVATE_KEY`. Making one by hand is not reasonable, so the binary has a subcommand. It is handled before `Config::from_env`, because the configuration may be invalid or empty exactly when someone runs it for the first time:

```rust
    // Needs no configuration, which may well be invalid or incomplete when this is run.
    if let ["vapid-keys"] = args.as_slice() {
        return vapid_keys();
    }
```

```rust
fn vapid_keys() -> ExitCode {
    let key = VapidKey::generate();
    println!("VAPID_PRIVATE_KEY={}", key.private_key_base64());
    println!("# public key: {}", key.public_key_base64());
    println!("# also set VAPID_SUBJECT=mailto:you@example.com (or an https: URL)");
    ExitCode::SUCCESS
}
```

The first line can be pasted into `.env`. The public key is printed only for reference: the service derives it from the private one and serves it at `GET /api/push/key`. The configuration (`parse_vapid` in `src/config.rs`) treats an empty or unset key as "no Web Push", an invalid key as an error (without printing it), and requires `VAPID_SUBJECT` to be `mailto:` or `https:` followed by an address whenever a key is set (`mailto:` alone is refused).

### The JWT we sign

The `Authorization` header of a push is `vapid t=<jwt>, k=<public key>`. The JWT has a fixed header, three claims (`aud`, the origin of the push service; `exp`; `sub`, our contact) and an ES256 signature. Instead of a JWT library, a dozen lines:

```rust
        let header = r#"{"typ":"JWT","alg":"ES256"}"#;
        let claims = serde_json::json!({
            "aud": origin(endpoint),
            "exp": (now + JWT_VALIDITY).timestamp(),
            "sub": subject,
        });
        let signing_input = format!(
            "{}.{}",
            Base64UrlUnpadded::encode_string(header.as_bytes()),
            Base64UrlUnpadded::encode_string(claims.to_string().as_bytes()),
        );
        let signature: Signature = self.0.sign(signing_input.as_bytes());
        format!(
            "vapid t={signing_input}.{}, k={}",
            Base64UrlUnpadded::encode_string(&signature.to_bytes()),
            self.public_key_base64()
        )
```

`aud` must be the origin of the endpoint (`https://fcm.googleapis.com`), the port only when it is not the scheme's default. The signature is the 64 bytes `r || s` (what JWT calls ES256), not the DER encoding other libraries default to: `signature.to_bytes()` gives exactly that. `now` comes from the `Clock`, so tests can fix it.

### Encryption and the request

A push message is encrypted so the push service cannot read it: RFC 8291 (`aes128gcm`) mixes the browser's public key (`p256dh`) and secret (`auth`) with a fresh key of ours. `web-push-native` does this and returns an `http::Request`; we add the VAPID header and `Urgency`:

```rust
        let (public_key, auth) = decode_keys(&subscription.p256dh, &subscription.auth)?;
```

```rust
        // `decode_keys` checked the length: `Auth::clone_from_slice` would panic otherwise.
        let auth = Auth::from(auth);
        let mut request = WebPushBuilder::new(uri, public_key, auth)
            .with_valid_duration(TTL)
            .build(payload.to_vec())
            .map_err(|err| format!("cannot encrypt the payload: {err}"))?;
```

```rust
        request
            .headers_mut()
            .insert(reqwest::header::AUTHORIZATION, authorization);
        request
            .headers_mut()
            .insert("Urgency", reqwest::header::HeaderValue::from_static("high"));
        reqwest::Request::try_from(request).map_err(|err| err.to_string())
```

`with_valid_duration(TTL)` sets the `TTL: 900` header (15 minutes). The result is converted to a `reqwest::Request` and sent by a client with a 10-second timeout.

> **Rust concept: `[u8; 16]` and `try_into`.** `decode_keys` turns the decoded bytes (a `Vec<u8>`) into an array with `auth.try_into()`, which fails unless the length is exactly 16. The array type is what proves it to the compiler: `Auth::from([u8; 16])` cannot panic, while the crate's `Auth::clone_from_slice(&[u8])` panics on any other length (see Pitfalls).

### The endpoint policy

`EndpointPolicy::check` runs when a device subscribes. It parses the URL and refuses what is not a push service:

```rust
        // `url.port()` is `None` for the scheme's default port, so any value is explicit.
        if self.require_https && url.port().is_some() {
            return Err("endpoint must not use a custom port".to_owned());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("endpoint must not contain credentials".to_owned());
        }
```

```rust
        let allowed = self
            .hosts
            .iter()
            .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")));
```

The match is the host itself or a subdomain *with the dot*: `evilfcm.googleapis.com` and `fcm.googleapis.com.evil.com` do not pass. An endpoint also has at most 2048 characters. The policy is checked at subscription time, not at send time: what is stored is what was allowed. The client also never follows redirects, so a push service cannot send the encrypted body elsewhere:

```rust
            .redirect(reqwest::redirect::Policy::none())
```

### Subscriptions and the upsert

A browser sends its `PushSubscription.toJSON()`; `SubscriptionStore` keeps one row per endpoint. The same device subscribing again must update the row, not duplicate it or fail. The insert comes first:

```rust
        let mut tx = self.pool.begin().await?;
        let created = sqlx::query(
            "INSERT INTO push_subscriptions (endpoint, p256dh, auth, label, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(endpoint) DO NOTHING",
        )
```

```rust
        let subscription: Subscription = sqlx::query_as(
            "UPDATE push_subscriptions SET p256dh = ?, auth = ?, label = ? WHERE endpoint = ?
             RETURNING id, endpoint, p256dh, auth, label, created_at",
        )
```

Why this order, in a transaction:

- The first statement is a write, so SQLite takes its write lock at once: two concurrent requests for one endpoint run one after the other. A `SELECT` first, then an `INSERT`, lets both see "no row" and both insert; one fails on `UNIQUE`.
- `DO NOTHING` plus `rows_affected() == 1` says exactly whether a row was inserted (`201` or `200`). A single `ON CONFLICT DO UPDATE ... RETURNING` cannot tell the two cases apart.
- The `UPDATE` after it refreshes the keys and label (a browser may rotate them) and leaves `created_at` alone; on a fresh insert it rewrites the same values.

`decode_keys` runs before any of this, at the API, so bad keys are a 400 and not a failure when sending: `p256dh` must be a valid uncompressed P-256 point, `auth` exactly 16 bytes.

### Notifier, WebPushNotifier and LogNotifier

```rust
pub trait Notifier: Send + Sync {
    /// `"webpush"` or `"log"`.
    fn channel(&self) -> &'static str;

    /// Delivers `notification` to every device. One device failing never stops the others.
    fn send<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> BoxFuture<'a, Result<SendReport, NotifyError>>;
}
```

> **Rust concept: `dyn` traits and boxed futures.** `async fn` in a trait makes the trait not *dyn-compatible*: the compiler cannot know the size of each implementation's future, so `Arc<dyn Notifier>` would not compile. Returning `Pin<Box<dyn Future + Send>>` (`BoxFuture`) gives every implementation the same return type, at the cost of one allocation per call, which does not matter for a handful of devices. The lifetime `'a` ties the future to the borrowed `self` and `notification`. The alternatives were a generic `AppState<N: Notifier>` (it spreads through every handler) or the `async-trait` crate (one more dependency for what is written by hand here).

`WebPushNotifier::send` goes through the stored subscriptions one by one and counts what happened in a `SendReport { sent, removed, failed }`. What each response means:

```rust
        if matches!(status.as_u16(), 404 | 410) {
```

2xx is `sent`. **404 and 410** mean the push service no longer knows that subscription (the user uninstalled the app or revoked permission): the row is deleted and counted as `removed`. Everything else (another status, a network error, a timeout, stored keys that no longer decode, a 3xx because redirects are off) is `failed`, logged with `warn` (the status and at most 200 characters of the body, never the keys) and the subscription is kept: a push service having a bad day must not unsubscribe the phone. Devices are served one at a time; for one user there is no need for concurrency.

`LogNotifier` is what runs without a VAPID key: it logs title, body and url at `info` and reports `sent: 1`. The channel name (`"webpush"` or `"log"`) is in the test endpoint's response, so a client can tell. The payload is `{"title", "body", "url", "tag"?}`, at most 3000 bytes (push services accept 4096 after encryption), otherwise `NotifyError::PayloadTooLarge`.

### The API

Four endpoints (documented in [../user/api.md](../user/api.md#push-notifications)). The handlers are small; the interesting parts are the body and the errors. The subscription body is the browser's own JSON, so it must accept `expirationTime` (which browsers send, usually `null`) and refuse other unknown fields:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeRequest {
    endpoint: String,
    /// Browsers send it (usually `null`); we do not use it.
    #[serde(default, rename = "expirationTime")]
    _expiration_time: Option<serde_json::Value>,
    keys: Keys,
}
```

Without a VAPID key, `GET /api/push/key` and `POST /api/push/subscriptions` answer `503 push_disabled`: the screen shows "Notifiche non attive" instead of failing in the browser. `DELETE` answers `204` also for an unknown endpoint, since the browser may have lost track of it. The label comes from the `User-Agent` (decision AR); iPhone and iPad are checked before `Macintosh`, because iPadOS can say `Macintosh`:

```rust
    const DEVICES: [(&str, &str); 5] = [
        ("iPhone", "iPhone"),
        ("iPad", "iPad"),
        ("Android", "Android"),
        ("Macintosh", "Mac"),
        ("Windows", "Windows"),
    ];
```

`POST /api/notifications/test` builds the notification (title "🔔 Notifica di prova", url `/notifiche`, tag `test`), calls `state.notifier.send` and returns `{"channel", "sent", "removed", "failed"}`. In `main.rs`, `config.vapid_private_key` and `vapid_subject` choose the notifier and the public key kept in `AppState`:

```rust
                    Ok(notifier) => {
                        tracing::info!("notifications: Web Push");
                        (Arc::new(notifier), Some(key.public_key_base64().into()))
                    }
```

### The web app: one state, a tap for permission

What the screen shows depends on six things, so `push.ts` reduces them to one value with a pure function (tested without a browser):

```ts
export function pushState(input: PushInputs): PushState {
  // Safari on iOS has no Push API at all outside an installed app, so this comes first:
  // "unsupported" would be the wrong thing to tell someone who only has to install.
  if (input.ios && !input.standalone) return 'ios-home-screen'
  if (!input.supported) return 'unsupported'
  if (!input.serverEnabled) return 'server-disabled'
  if (input.permission === 'denied') return 'denied'
  if (input.permission === 'granted' && input.subscribed) return 'on'
  return 'off'
}
```

The order matters: each state hides the ones after it. `Notifications.svelte` shows a card per state: `off` has the bell, the explanation and "Attiva notifiche"; `on` has "Attive" with "su questo telefono" and the "Invia una prova" button; the others are the text-only variants of decision AQ.

**Permission only after a tap.** The screen's `onMount` calls `loadPush`, which reads state and never asks for permission. Only the button calls `enablePush`, and `requestPermission` is the first thing that runs in it, before any `await`:

```ts
export async function enablePush(key: Uint8Array<ArrayBuffer>): Promise<PushState> {
  const permission = await Notification.requestPermission()
  if (permission === 'denied') return 'denied'
  if (permission !== 'granted') return 'off' // The prompt was dismissed: ask again next time.
  const worker = await registration()
  if (!worker) return 'unsupported'
```

then it subscribes with the server's key (`applicationServerKey`, base64url turned into bytes by `urlBase64ToUint8Array`, `userVisibleOnly: true`) and POSTs the subscription. Browsers (iOS above all) refuse the prompt if the call does not come straight from a user gesture, and a server round trip before it would break that.

**Sending it again, and key rotation (decision AP).** On open, with permission granted and a subscription in the browser, `loadPush` posts it again, and replaces it when it was made with another key:

```ts
    let subscription = await worker.pushManager.getSubscription()
    if (subscription && keyChanged(subscription, key)) {
      try {
        subscription = await replaceSubscription(worker, subscription, key)
      } catch {
        return { state: 'off', key }
      }
    }
    if (subscription) {
      await savePushSubscription(subscriptionBody(subscription.toJSON()))
      subscribed = true
    }
```

The key check compares `subscription.options.applicationServerKey` with the server's key. Some browsers do not report it (`options.applicationServerKey` is `null` or missing): then the subscription is given the benefit of the doubt instead of being replaced at every opening. `replaceSubscription` unsubscribes, tells the server to forget the old endpoint (ignoring failure) and subscribes again. Also, `registration()` waits at most 3 seconds for the service worker: `navigator.serviceWorker.ready` never settles when there is no worker (development builds), and the screen would wait forever.

The test button shows "Inviata" for 4 seconds and goes back to "Invia una prova"; the timer is cleared when the screen closes or a new test starts:

```svelte
        sentTimer = setTimeout(() => (test = 'idle'), 4000)
```

### The service worker

`sw.ts` gets two listeners. `push` reads the payload and shows a notification inside `event.waitUntil`, so the worker stays alive until it is shown:

```ts
self.addEventListener('push', (event) => {
  const { title, body, url, tag } = readPayload(() => event.data?.json())
  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      tag,
      // A replaced notification (same tag) is silent unless it is asked to alert again.
      ...(tag ? { renotify: true } : {}),
      data: { url },
      icon: '/pwa-192x192.png',
      lang: 'it',
    }),
  )
})
```

`renotify: true` with a `tag` makes a second notification with the same tag alert again instead of replacing the first silently. A push with `userVisibleOnly` must always show something, so `readPayload` (in its own module, `pushPayload.ts`, tested without a worker) never throws: a payload that cannot be read gives a generic "InTime", and a `url` that is not a path inside the app (`//evil.com`) becomes `/`. `tsconfig.sw.json` includes this module, since the worker's configuration is separate from the app's.

`notificationclick` closes the notification and opens its `url`: it prefers a window of the app already open (focus it and `navigate`), otherwise `clients.openWindow`. A url outside the app's origin is never followed, and the event never rejects: if nothing works, nothing happens.

## Tests

- **Unit, Rust** (20 new): `vapid.rs` (4): text round trip and key sizes; wrong length, bad base64, a zero scalar and padding refused; `Debug` hides the key; the `Authorization` header is parsed back, with the header, `aud` (with and without a port), `exp` = now + 3600 and `sub` checked and the signature verified with the public key it carries. `push.rs` (10): upsert inserting and updating (keys and label changed, `created_at` kept, `created` flag); two concurrent upserts of one endpoint leave one row and exactly one `created`; list; delete and delete by endpoint; `decode_keys` with a real key, with a bad point and with `auth` of 15 and 17 bytes; the endpoint policy (each push service, subdomains, look-alike hosts, `http`, credentials, a custom port, too long, bad URL; the default port written out is accepted and normalized). `notify/mod.rs` (3): payload shape, no `tag` key when none, too large refused. `log.rs` (1). `api/push.rs` (1): labels from user agents. `config.rs` (1): VAPID unset or empty, valid, invalid key without leaking it, key without a usable subject.
- **Integration** (`tests/push.rs`, 17), with `wiremock` as the push service: the key (200 with a VAPID key, 503 without); subscribing twice (201 then 200, label from the `User-Agent`); no label for an unknown agent; `expirationTime` accepted and unknown fields refused; endpoints that are not push services refused (`http`, other hosts, look-alikes); bad keys refused without panicking; unsubscribing (204, also twice); the test notification reaching two devices with `TTL`, `Urgency`, `Content-Encoding: aes128gcm` and a `vapid` `Authorization`, whose body is decrypted in the test with `web_push_native::decrypt` and the device's own secret to the payload JSON; a 410 removing one subscription while the other still gets the message; a 404 doing the same; a 500 and a network error counted as `failed` with the subscription kept; a redirect not followed and counted as `failed`; stored keys that do not decode counted as `failed` without panicking; the JWT naming the push service's origin and the subject; no VAPID key, channel `log`. `tests/common/mod.rs` gets `Options::vapid_key` (enables Web Push with the subject `mailto:test@example.com`) and `Options::push_endpoints` (which hosts subscriptions may point to; the production list by default), and `TestApp::subscriptions` and `vapid_public_key`.
- **Web** (Vitest): `push.test.ts` (29): each state of `pushState`, `isIos`, the key conversion and comparison, `subscriptionBody`, and `loadPush` and `enablePush` against a fake `navigator`, `Notification` and service worker registration (an existing subscription sent again, a key replaced, a browser that does not report the key, a worker that is not active yet or never comes, no prompt without a tap, a dismissed prompt). `pushPayload.test.ts` (5): full payload, generic fallbacks, bad JSON, a `url` outside the app, fields of the wrong type. `api.test.ts` (4 new): the calls. `router.test.ts`: `/notifiche` and the new tab.

Results: 218 Rust tests (37 new, from 181); 126 web tests (38 new, from 88).

## Check it by hand

```bash
cargo run -- vapid-keys
```

Prints the pair to put in the environment. Then, on a copy of `data/` (see Pitfalls) with the web app built (`cd web && npm run build`):

```bash
DATA_DIR=/tmp/gtt-copy BIND_ADDR=127.0.0.1:8081 VAPID_PRIVATE_KEY=... VAPID_SUBJECT=mailto:you@example.com cargo run
```

Done on 5 November 2026 on a copy of the data folder, with the keys of `vapid-keys`: migration 5 applied and the log said "notifications: Web Push". In Chrome on the Mac, `http://localhost:8081/notifiche` showed "Attiva notifiche"; the tap asked for permission and the card became "Attive su questo telefono". The subscription was stored with an `fcm.googleapis.com` endpoint and the label "Mac". "Invia una prova" delivered "🔔 Notifica di prova" twice in a row, and both alerted; tapping a notification opened `/notifiche`. The built-in browser of the Claude desktop app blocks notifications, which showed the "Notifiche bloccate" state, so this check needs a real browser.

The same through the API (a subscription needs a real browser's keys, so only the test is shown):

```bash
curl -s -X POST localhost:8081/api/notifications/test
```

## Pitfalls

- **`Auth::clone_from_slice` panics.** `web-push-native` builds the browser's `auth` secret from a slice and panics when it is not 16 bytes long (the method is also deprecated). A client sending a 15-byte `auth` could have crashed a request. The keys are checked first by `decode_keys`, and the secret is built with `Auth::from([u8; 16])`, which cannot fail.
- **`jwt-simple` pulls `rsa`.** The crate that comes with `web-push-native`'s `vapid` feature depends on `rsa` 0.9, which has RUSTSEC-2023-0071 and no fix. Signing the JWT ourselves with `p256` avoided both the advisory and about 46 crates (decision AK).
- **The old service worker serves "Pagina non trovata".** After the change, `/notifiche` shows the not-found screen in a browser that already has the app installed, until "Aggiorna" is tapped: the old worker answers every navigation with its cached `index.html` and old code, which does not know the route. It goes away after the new version is activated; it is the update prompt of chapter 12 working as designed.
- **reqwest follows redirects by default.** Found in review: a 307 or 308 from a push service would have re-posted the encrypted body to whatever host it named, past the endpoint allowlist (reqwest drops `Authorization` on a redirect to another host, but not the body). The client now uses `redirect::Policy::none()`, and a test checks it.
- **Select-then-insert races.** Found in review: the first version of the upsert looked the endpoint up and then inserted, so two concurrent subscriptions of one device could both insert and one would fail on `UNIQUE`. The write-first transaction above fixed it.
- **A subscription may lack `options.applicationServerKey`.** Some browsers do not report it. Treating that as "different key" would unsubscribe and subscribe again at every opening, so a missing key counts as unchanged.
- **wiremock splits headers at commas.** `Authorization: vapid t=..., k=...` has a comma, and a header matcher sees it as two values. The tests read the request and check the header themselves instead of using `header()` matchers.
- **Permission before any `await`.** Anything asynchronous before `Notification.requestPermission()` (a fetch of the key, for instance) loses the user gesture on iOS and the prompt does not appear; the key is fetched when the screen opens and kept.
- **A new migration and the usual database.** As in chapters 15 and 16, migration 5 would make `data/gtt.db` unusable by `main`; the check by hand ran on a copy of the folder, with a temporary entry in `.claude/launch.json` removed at the end.

## Later changes

None yet.
