# Anomaly disposition

| Record | Observation and technical disposition | Dossier restriction |
| --- | --- | --- |
| #4904 | Finite-loop proof semantics remain under active investigation; this baseline's guard tests establish only their listed examples. | No general finite-loop implementation proof credit. Revalidate any future semantic fix on a new binary baseline. |
| #4907 | [Run40053683090](https://github.com/EricSpencer00/Resilient/actions/runs/40053683090), source `cef6ad2d59deef321b434ea2863f3d52cf384b1f`, passed both full serial/eight-thread runs but failed 1/12 GPIO diagnostics and 1/12 hook diagnostics (telemetry127/128). [Run40054122856](https://github.com/EricSpencer00/Resilient/actions/runs/40054122856), source `76519bfa8ddb705d6f3fb77d775637d10b25e79a`, passed both full runs and 24 diagnostics; 26 bundles per run were verified. | Retain both schedules; later passes do not erase intermittent failures. No concurrency/runtime safety claim, and these runtime revisions differ from the compiler binary baseline. |
| #4927 / #4934 | Body-grounded postcondition queries were absent from ordinary saved-query bundles. #4934 repaired their export; three unchanged regressions failed on the test-only baseline and passed on the repaired source. | All four contract replay cases, including the false-postcondition control, passed on this retained binary; this does not establish complete language coverage. |
| Source binding | Legacy signed manifests authenticate their path label/enumeration. The added `--source-root` mode authenticates parsed entry/import content and the frozen source graph; 24 paired cases passed in run40056681094 (then revalidated on the integrated binary in run40060158827). | This profile requires `--source-root` for content identity. Accepted requirements, package resolution configuration, producer/translation correctness and atomic filesystem access remain separately controlled or excluded. |
| Arithmetic model | Existing mathematical integer contract models do not establish machine-width arithmetic equivalence. | Exclude target overflow/representation credit and independently verify consuming output. |
| Evidence lifetime | Actions binary artifact retention is 90 days; source/log records committed here persist independently. | The current run40060158827 downloaded inventory was verified in full, including `.tests.lock` and raw test output. The separately labeled hosted debug archive is prepared for permanent release retention; publication and verified receipt remain tracked by #4954. |

These are disclosed restrictions rather than accepted residual safety risks.
Project safety owners must evaluate failure impact and detection in their own
workflow. Technical preparation does not close project-specific safety anomalies.
