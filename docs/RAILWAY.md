# Railway Deploy

Inkwell is ready to run on Railway as a Dockerfile-backed web service with a
Railway Postgres database.

## Project Setup

1. Create a Railway project from `git@github.com:HexSleeves/inkwell.git`.
2. Add a PostgreSQL database service.
3. On the Inkwell web service, add these variables:

| Variable | Value |
| --- | --- |
| `DATABASE_URL` | Reference the PostgreSQL service's `DATABASE_URL`. |
| `INKWELL_API_KEY` | Generate a long random value. Required for write routes. |
| `INKWELL_SITE_URL` | Railway public URL or custom HTTPS domain. |
| `HOST` | `0.0.0.0` |

Do not set `PORT`; Railway injects it and uses it for routing and healthchecks.

## Deploy Behavior

`railway.json` configures Railway to:

- build with the repo `Dockerfile`;
- run `inkwell db migrate` before each deploy;
- start the service with `inkwell serve`;
- wait for `GET /health` before promoting the deployment.

## First Deploy

After the service is connected and variables are set, trigger a deploy from the
Railway dashboard or CLI:

```bash
railway up
```

Generate a public Railway domain from the service Networking tab, then set
`INKWELL_SITE_URL` to that HTTPS URL and redeploy so feed/sitemap metadata uses
the public origin.

## Smoke Check

```bash
BASE=https://your-app.up.railway.app
KEY=<INKWELL_API_KEY>

curl -fsS "$BASE/health"
curl -fsS -X POST "$BASE/documents" \
  -H "x-api-key: $KEY" -H 'content-type: application/json' \
  -d '{"title":"Railway smoke","bodyMarkdown":"# Hello from Railway","tags":["smoke"]}'
curl -fsS -X POST "$BASE/documents/railway-smoke/publish" -H "x-api-key: $KEY"
curl -fsS "$BASE/railway-smoke"
curl -s -o /dev/null -w '%{http_code}\n' -X POST "$BASE/documents" \
  -H 'content-type: application/json' -d '{"title":"nope"}'
```

The final command should print `401`.
