# Agent Catalog

An ARD-compliant, **presence-aware** catalog of agents. The catalog indexes,
verifies, and shows liveness — it never hosts agents.

Every agent directory today is a phone book of numbers nobody answers: static,
self-asserted, unverifiable. This one is built on three pillars:

1. **Liveness.** Listings carry live presence (online / last seen / dead for a
   month), fed by connectors to real networks — AgentMesh first, whose
   heartbeats make presence native rather than crawled.
2. **Verification.** Claims can be *probed*: a listing's "prove it" record
   shows the agent actually responded, with latency and timestamp. Claims that
   can't be probed are labeled as assertions.
3. **Neutrality.** Sources are pluggable connectors (AgentMesh, A2A cards,
   manual submission); the catalog serves ARD-standard read interfaces. No
   network owns the shelf.

The same code runs the public instance and self-hosted org catalogs. Org
instances can subscribe to the public catalog; nothing internal ever publishes
outward by default.

## Specialty claims

Listings carry namespaced specialty claims — a thin convention, not an
ontology:

- `build:macos`, `build:windows` — situation: what the host can build/test
- `access:hubspot-api`, `access:salesforce-prod` — systems it can reach
- `runtime:coding-assistant` — what kind of runtime it is

Search composes claims with presence: *"who has HubSpot access and is
listening right now."*

## Quickstart (dev)

```
cargo run                 # serves http://localhost:8080
```

That's the whole setup. The catalog runs its own embedded PostgreSQL: the
first run downloads the server binaries and initializes a cluster, both under
one data root (`CATALOG_DATA_DIR`, default `.data/`); every later run starts
in seconds. No Docker, no services, no install.

Have your own Postgres 15+? Set `DATABASE_URL` and the embedded server never
starts — that's also how deployed instances run.

Smoke test:

```
curl http://localhost:8080/healthz
curl -X POST http://localhost:8080/api/listings -H "Content-Type: application/json" \
  -d '{"name":"Echo","description":"Public mesh echo agent","specialties":["runtime:test"],"protocol":"mesh"}'
curl "http://localhost:8080/api/listings?q=echo"
```

## API (v0)

| Route | Method | Purpose |
|---|---|---|
| `/healthz` | GET | liveness + version |
| `/api/listings` | GET | search: `q` (full-text), `specialty`, `protocol`, `source`, `presence`, `limit` |
| `/api/listings/:id` | GET | one listing with presence |
| `/api/listings` | POST | manual submission (upsert on `source_id`) |

## Roadmap

- **AgentMesh connector** — harvest the registry, subscribe to presence
- **Probe runner** — scheduled "prove it" checks backing verification badges
- **ARD read interface** — standard-shaped projection of the same listings
- **A2A card connector** — list anything with an agent card, probe via bridge
- **Semantic search** — pgvector column per listing
- **Federation** — org instances subscribing to the public catalog

## Architecture notes

- One binary (Rust + axum), Postgres storage. Listings are JSONB documents
  with filterable columns lifted out; full-text search is a stored tsvector.
- Presence is a cache, not a record — rebuilt from source networks on restart.
- Probes are append-only.
- Dev runs an embedded PostgreSQL (localhost-only, dev-only committed
  credential); deployed instances read `DATABASE_URL` from the environment.
  No credential anywhere depends on a human's (or an assistant's) memory.
