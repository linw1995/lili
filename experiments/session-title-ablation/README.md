# Session title runtime ablation

Baseline: `8bbac24964cd3f3e3ba4a71c29396968b1442fb6`.
Platform: arm64 macOS. Application source and the existing PR were not changed by the experiment runs.

## Method

`run.py` archives the pinned revision into a temporary directory, adds four behavior probes, and evaluates each ablation independently. All variants use the same three library suites (`lili-actions`, `lili-app-state`, and `lili-session`) and the same probe assertions. Compilation artifacts are reused from the workspace target directory; source edits occur only in the temporary archive.

The baseline passes 183 tests. The probes cover coalesced null completion, per-Session debounce after a failed lookup, provider separation for identical Session IDs, and avoiding tasks for unmatched sources. Existing regressions cover cache reuse, ordering, notification recreation, persistence, and shutdown/publication.

Negative controls disable one behavior without necessarily removing its supporting data structures. Their line counts must not be interpreted as production refactoring estimates. Test durations include incremental compilation and fixture work and are not performance measurements.

## Results

| Variant | Result | Interpretation |
| --- | --- | --- |
| Baseline | 183 passed; Clippy passed | Valid control |
| Remove action ID from the query key | 183 passed | Candidate: action selection is determined by provider within an immutable supervisor |
| Replace nested optional watch replies | 183 passed | Candidate: watch version updates already distinguish pending from completed-null |
| Replace map plus title-order index with one ordered collection | 183 passed | Candidate with a trade-off: one less index, bounded linear ID lookup |
| Combine the three candidates | 183 passed; Clippy passed | No interaction failure observed in this test set |
| Bypass per-Session debounce | 182 passed, 1 failed | Repeated failures execute again inside the configured debounce window |
| Bypass in-flight coalescing | 182 passed, 1 failed | Concurrent callers no longer share the same successful result under reject admission |
| Remove notification incarnation validation | 182 passed, 1 failed | An old result can update a recreated notification with the same textual ID |
| Stop awaiting publication tasks at shutdown | 182 passed, 1 failed | Shutdown returns while title publication is still blocked |
| Remove the application prefilter | 182 passed, 1 failed | Unmatched sources create transient application tasks; this is a resource assertion, not a demonstrated user-visible title failure |

The first combined run passed behavior tests but failed Clippy on a redundant local binding. Removing that binding and rerunning the single-option and combined variants passed. `results/initial-summary.json` preserves the initial observation; `results/summary.json` contains the latest result for each variant with its harness hash.

## Recommended scope

1. Prioritize the query-key and watch-reply changes. They remove redundant information without changing ownership or public contracts.
2. Review the single-collection change separately. It removes one maintained index but does not reduce formatted production line count in this patch. ID lookup becomes a linear scan bounded by the existing 128-entry configuration limit; no speedup has been established.
3. Keep debounce, coalescing, incarnation checks, child reaping, and tracked publication. The two `pending` collections represent different lifetimes: process execution and notification update/publication.
4. Keep the early prefilter for now. Saving three lines does not justify extra tasks for unmatched notifications without a measured benefit.

The combined measured patch removes six nonblank production lines and one action-order index, reduces the query-key tuple from three strings to two, and eliminates the extra reply-state wrapper. The main benefit is less duplicated state, not a large reduction in source size.

Runtime configuration freezing could enable a larger redesign, because production currently configures actions during initialization. However, configuration replacement is supported by the application-state API and its tests. Removing that contract is a separate design decision, not a behavior-preserving deletion. No ablation here justifies removing request tokens, lifecycle serialization, the shared execution trait, or persistence rollback.

## Reproduce

From the repository root:

```sh
nix develop -c python3 experiments/session-title-ablation/run.py
```

To rerun selected variants:

```sh
nix develop -c python3 experiments/session-title-ablation/run.py single_option combined
```

The runner regenerates patches for every variant and updates results only for selected runs. Raw logs remain under `results/` locally and are ignored by Git. `probes.patch` supplies the added assertions; each variant patch applies on top of that probe baseline. `baseline.patch` is empty after normalization.

## Evidence and limits

- [Latest machine-readable results](results/summary.json)
- [Initial results, including the lint finding](results/initial-summary.json)
- [Combined candidate patch](results/combined.patch)
- [Shared behavior probes](results/probes.patch)
- [Implementation plan](../../openspec/changes/simplify-session-title-runtime/proposal.md)

This is evidence of observed equivalence on the scoped test set, not a proof that all interleavings are equivalent. Variants were not subjected to full-workspace CI, other operating systems, load benchmarks, or packaged UI acceptance. Those checks remain implementation tasks. The experiment runs made no production source changes or remote operations. Subsequent adoption is tracked in the implementation plan.

## Adoption decision

Adopt the smaller query key and single-option watch reply. Retain the existing action map and title-order index for now: the alternative does not reduce production line count and changes lookup/sorting costs without a measured benefit.
