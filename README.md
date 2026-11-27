<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/logo-dark.svg">
    <img src="docs/assets/logo-light.svg" alt="MACH" width="360">
  </picture>
</p>

<p align="center">A Rust backend framework for consistent PostgreSQL APIs.</p>

MACH is a Rust backend framework for building APIs on PostgreSQL. You declare an entity in Rust, choose which fields the API may accept and return, and use the CLI to generate the controller, inspect the SQL and prepare migrations. Axum serves HTTP requests and SQLx runs the parameterized queries.

The same CLI serves people and coding agents. For example, `mach field rename User name full_name --dry-run` shows which declarations and queries will be affected before any code is edited. Generation is deterministic and local, with no dependency on an AI model.

## Try it

With stable Rust, rustfmt and Clippy installed, run this from the repository root. The project was verified with Rust 1.95.

```sh
cargo run -p hello-world
```

In another terminal, `curl http://127.0.0.1:3000/` returns `Hello, MACH!`. This example needs no database. Stop it with Ctrl+C before starting another application on the same port.

## Build a users API

```sh
cargo install --path crates/mach-cli --locked
mach new demo --db postgres
cd demo
mach generate resource users --crud
mach field add User phone:string --optional
mach field rename User name full_name --dry-run
mach field rename User name full_name
mach generate resource posts --crud --owner users
mach field add Post created_at:timestamp --generated --expose
mach generate migration
mach inspect User --sql
mach context --compact
mach check --agent
mach test
mach build --release
```

The generated project references the `mach` crate in this checkout. To use another copy, set `MACH_FRAMEWORK_PATH` before running `mach new`. Commit the generated `Cargo.lock`, and adjust the local dependency if you move the application to another machine.

To run the API against a local PostgreSQL, review the SQL in `migrations/` and run the commands below in the `demo` directory. They create development credentials and grant creating users and reading the user with ID 1.

```sh
export POSTGRES_PASSWORD="$(openssl rand -hex 32)"
export DATABASE_URL="postgres://app:${POSTGRES_PASSWORD}@127.0.0.1:5432/app"
export MACH_TOKEN="$(openssl rand -hex 32)"
export MACH_GRANTS="users:create:collection,users:read:1"
docker compose up -d --wait
mach migrate
mach dev
```

In another terminal with the same `MACH_TOKEN`, create the first user.

```sh
curl -i http://127.0.0.1:3000/users \
  -H "Authorization: Bearer ${MACH_TOKEN}" \
  -H 'Content-Type: application/json' \
  -d '{"email":"ana@example.org","full_name":"Ana"}'
```

On a fresh database the response is `201 Created` and includes the generated ID. The `phone` field stays private until it gets an explicit visibility declaration. Listing requires its own grant. Requests without a credential get 401 and operations without a grant get 403. The API and the database port listen on localhost by default. In a deployed environment, terminate TLS in front of the service.

## How it works

- **Entities and validation.** `#[entity(table = "users")]` declares the table. `#[id] #[generated]` marks the i64 primary key. `#[writable]` adds a field to the public input and `#[expose]` allows it to be serialized. Input rejects unknown fields. Models accept String, i32, i64, bool, `Timestamp` (TIMESTAMPTZ, serialized as RFC 3339) and Option of these. `#[generated]` on a `Timestamp` lets the database fill it with `now()`. PUT replaces the writable fields. The generated `users` resource already declares the email as unique.
- **Queries.** Repositories derive methods with the prefixes `find_by`, `count_by`, `exists_by` and `delete_by`. Predicates support equality, `and`, `is_null`, `greater_than` and `less_than`. The compiler checks field names, ambiguity, parameters and return types. A single lookup returns the first row in primary key order. Listing returns 25 records by default, with a limit of 100 and a maximum offset of one million. `?order=desc` reverses the primary key order.
- **Persistence.** CRUD and queries take the pool or a transaction explicitly. `#[references("other_table.id")]`, `#[unique]` and `#[index]` declare foreign keys and single-column indexes. For custom SQL, use `mach::sqlx` and bind external values as parameters.
- **Record ownership.** `#[owner]` marks an i64 field the client never sends. `create`, `update` and `delete` take the owner explicitly, and writes filter by it: changing another owner's record results in 404. `generate resource posts --crud --owner users` creates the `user_id` field and a controller that uses `s.require(...)?.id()?`, the numeric ID of the authenticated principal. The access policy must return that ID; the default `BearerAccess` represents a single operator and refuses these writes.
- **WebSocket.** Enable the `ws` feature of the `mach` crate to use `mach::axum::extract::ws`.
- **Code and schema changes.** The CLI edits the Rust declarations it can identify, preserves unrelated text and verifies that the project compiles. A journal in `.mach/` restores files after a failure. Conflicting concurrent edits require manual reconciliation. SQL migrations are numbered and versioned. A stable column key lets columns be renamed while preserving data. A required column without a default added to an existing table produces a `review` warning, because existing rows need a value before the migration runs. Type conversions, changes to existing defaults and reworked relationships require reviewed SQL and history reconciliation.
- **MACH Guard.** Combines compiler and Clippy diagnostics with rules for architecture, comments, unsafe code and potentially blocking operations. Results distinguish proven violations from heuristic warnings. `mach fix --safe` applies rustfmt with verification and restores files on failure.

## Command reference

| Command | Purpose |
|---|---|
| `mach generate` | Generates modules, entities, controllers, services, CRUD resources or migrations. |
| `mach field`, `mach index` | Adds, renames, alters or removes fields and declares indexes. Both accept `--dry-run`. |
| `mach migrate` | Applies reviewed migrations. Destructive changes require `--allow-destructive`. |
| `mach context`, `inspect`, `routes`, `dependencies`, `schema`, `api` | Queries metadata, routes, SQL and APIs. Use `--json` for automation. |
| `mach expand` | Shows macro expansion using the optional cargo-expand tool. |
| `mach check --agent`, `mach check --strict` | Produces structured diagnostics or applies the strict comment policy. |
| `mach test --affected`, `mach build --release`, `mach dev` | Runs tests, builds or starts the application through Cargo. |

## Examples and verification

[users-api](examples/users-api) contains the example's entity, controller, repository and migration. [authenticated-api](examples/authenticated-api) reuses those routes and limits reads to the ID tied to the credential. Run it from the users-api directory with the same database, a bearer credential and `MACH_ACCOUNT_ID` set.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p mach-cli --test postgres -- --ignored
python3 -m unittest discover -s benchmarks
```

The integration test uses Docker, PostgreSQL 16 and Python 3. It creates a disposable application and database and checks persistence, per-resource authorization, record ownership, column renames with existing data and recovery after failed edits.

## Benchmarks

The HTTP benchmark compares MACH and Axum with the same limits, credentials, data and database configuration. On Linux, with the PostgreSQL server binaries installed, run `python3 benchmarks/run_http.py --requests 10000 --concurrency 1 16 --repeats 3`. The [local measurement](benchmarks/results/local-2026-11-25.json) covers 64 thousand requests across eight workloads on a shared machine.

[BackendBench](benchmarks/BackendBench.md) defines tasks for comparing development cost with agents. The grader uses real telemetry records. Token savings have not been measured yet.

[Engineering decisions](docs/engineering.md) · [MIT License](LICENSE)
