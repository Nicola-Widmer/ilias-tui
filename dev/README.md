# Local test ILIAS

A throwaway ILIAS 10 in Docker, for developing features without touching a real one. It only
listens on `localhost`. Its passwords are public, so never put real data in it.

```bash
docker compose -f dev/docker-compose.yml up -d
```

```bash
./dev/seed.sh
```

The first start takes several minutes (the image, then the ILIAS setup); `seed.sh` waits for
it. Then run the TUI against it, with its own session, cache and saved-files list:

```bash
ILIAS_URL=http://localhost:8080/ cargo run
```

Log in in the Chrome window that opens:

| Login     | Password    |                                 |
| --- | --- | --- |
| `student` | `ilias-dev` | Member of the three test courses |
| `root`    | `ilias-dev` | Administrator                   |

The same logins work in a browser at <http://localhost:8080/>.

## What's in it

`seed.php` creates three courses (`DEMO.101`, `DEMO.102`, `DEMO.100`), nested folders, files of
several types, a forum without posts, and two exercises due in 3 and 10 days, counted from when
you seeded. For anything else, such as forum posts or an edited file, log in as `root` and add
it in the browser.

## Stopping and starting over

Stop it and keep the content:

```bash
docker compose -f dev/docker-compose.yml down
```

Delete everything:

```bash
docker compose -f dev/docker-compose.yml down -v
```

## Differences from a real ILIAS

- **Login:** ILIAS' own login page instead of OpenID Connect and 2FA, so the silent session
  renewal does nothing here: an expired session means logging in again.
- **Look and language:** the default skin and English, without a school's skin or plugins.
  A parser that works here can still fail elsewhere, so check real pages too.
- **Web services:** `seed.sh` turns on SOAP and turns off the password change on first login.
