# BackendBench v1

Compare MACH, raw Axum, NestJS and Spring Boot using the same PostgreSQL version, hardware, agent, model revision, sampling settings, permissions, time limit, context reset policy and dependency-cache state. Record the starting commit and toolchain. Run each task from an identical logical fixture on each stack. Randomize stack order. Run at least three independent repetitions. Separate cold and warm agent sessions.

| Task | Observable acceptance condition |
|---|---|
| Create users | POST accepts email and name, returns 201 and a generated ID. GET retrieves the persisted row. |
| Add optional phone | Existing rows read as null. Public input and output stay restricted to the declared contract. |
| Rename name | An old row retains its value after migration. The new API name works. The old name is rejected. |
| Unique email | Concurrent duplicate inserts produce one persisted row and one conflict. |
| Derived lookup | Exact email lookup returns a matching user or absence. A literal containing an apostrophe round-trips. |
| Authentication | Missing and invalid credentials both yield 401. |
| Authorization | A valid reader for one ID receives 403 for another ID. |
| Pagination | Stable ID order, bounded size and rejection of limits outside 1–100. |
| Fix a wrapper | Remove an argument-forwarding wrapper while preserving callers and behavior. |
| Refactor a module | Move the requested business rule into a service. Existing HTTP and database tests still pass. |

Run the same black-box assertions against each implementation. Record functional and security checks separately, including failures and timeouts. Preserve the prompts, commit diffs, test logs, telemetry source and raw usage record outside the application repository. Remove credentials from retained artifacts. Keep unseen acceptance tests out of agent context.

One JSONL row per run contains framework, task (1–10), run ID, model, functional_pass, security_pass and elapsed_s. Optional nonnegative integer metrics are input_tokens, output_tokens, cached_tokens, tool_calls, files_read, files_modified, compilation_attempts and test_attempts. Token measurements require a usage_source identifying an actual provider record or exported agent telemetry. Omit unavailable metrics. Keep cached tokens separate because providers vary in their accounting.

Run `python3 benchmarks/backendbench.py runs.jsonl` for grouped counts, pass rates and metric medians. Compare like model/task groups and report the denominators for each measurement. Report unsuccessful runs alongside passing runs. The evaluator runs locally. No token-efficiency result exists until telemetry is collected.
