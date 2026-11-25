# Implementation decisions

Use stable Rust, edition 2024. Cargo resolves compatibility in the workspace lockfile. Axum 0.8 uses brace path parameters and Tower 0.5 services. SQLx 0.8 uses explicit PostgreSQL executors and runtime-bound parameters. Its generated SQL is checked against a disposable database in integration tests.

The public mach crate joins the runtime and procedural macros. mach-core owns configuration, mach-db owns database contracts, and mach-http owns the application boundary. mach-codegen shares schema parsing and SQL generation between mach-macros and mach-cli. mach-guard inspects application source. Runtime request paths do not parse Rust or inspect a schema.

Public entity input and output use separate opt-in fields. Generated endpoints require an explicit resource grant. Source edits operate on parsed declaration spans. A journal stores before/after images and verification failures restore touched files. Ambiguous schema changes must stop for review.

## Milestones

1. Serve and test an HTTP application.
2. Scaffold a buildable application and register modules.
3. Persist typed entities and generate SQL and migrations.
4. Apply and verify reversible source changes.
5. Report actionable compiler and architecture diagnostics.
6. Exercise security boundaries, PostgreSQL behavior and benchmark workloads.
