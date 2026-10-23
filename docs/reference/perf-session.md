# Session reuse and caching design

This page describes how an engine session (`RasHdfSession`) reuses work between calls:
what metadata is memoized, how long it lives, how thread pools are shared, and which
reader-level caches support it. It also states the rules that any benchmark of these
caches must follow. The Python-facing session semantics are in [sessions.md](sessions.md).

## Metadata memoization

The source must remain immutable for the lifetime of a session. Successful metadata
queries are memoized by the complete method argument tuple, including max_bytes,
expected time rows, family and exact path/name. A smaller byte budget never borrows
a larger-budget verdict. Errors are removed for retry, including host errors.
Per-key OnceLock initialization allows concurrent callers to share one computation;
the map lock is released during computation, so nested validation can use other keys.
Returned values remain owned clones, preserving caller mutation/lifetime semantics.

Cached objects include numeric capability/encoding proof, group members, mesh inventory
and diagnostics, cross sections/profiles, time axes, bounded units, mesh geometry
descriptors, plan capabilities, internal structure inventories, mesh result inventories
and run summary. Caching the internal structure inventory also serves plan capabilities
and the catalogue: those calls reuse the same validated inventory rather than repeating
its identity and variable checks. This is still a semantic catalogue, not a structural
h5py enumeration. Typed result reads still validate the complete family; requested-only
qualification was deliberately deferred to avoid changing sibling-corruption verdicts.

Metadata caches are session-local, without eviction, and retain successful results for
each distinct argument tuple. This trades memory for reuse. Many distinct budgets/paths
can grow memory; numeric result buffers are not placed in the metadata cache. Default
generic data caches retain their existing bounds. Dropping the session releases its
metadata and file context. Returned clones can outlive the session as before.

## Thread pools

Process-wide dedicated Rayon pools are shared per explicit worker count. This preserves
thread_count and existing 1..1024 validation, without reconfiguring Rayon globally.
Pools persist for process lifetime; distinct worker-count requests can create multiple
pools. Numeric reads on one worker and reads below 64 KiB of selected stored values
run serially without pool dispatch. Larger reads with multiple requested workers use
the shared pool. Default thread counts and parallel flags are unchanged.

## Reader-level caches

The vendored generic reader caches old-style symbol-table names and addresses per
file context. A name-to-address map avoids repeated linear scans. Entries never hold
Arc<FileContext>, preventing a reference cycle. Heap name decoding reads a segment
once on first enumeration and validates offsets, UTF-8 and null termination.
Group::member_names classifies child headers without constructing datasets or loading
their attributes. The domain engine uses this for lightweight group discovery.

Raw datatype envelope qualification remains unchanged on a first numeric capability
check; only the proven capability is reused. Reader-wide object/datatype reuse, a
selection-independent chunk-index cache, cache bypass/configuration and decode-into-
output bulk changes remain separate work. Existing bounded deflate already reserves
the known expected size plus the safety byte; no decompressor swap was made.

## Benchmark rules

Benchmark numbers must come from release builds using the same stored queries and
exact little-endian value hashes. Session-cold does not mean OS-cache-cold. Separate
metadata catalogue and h5py structural traversal are not equivalent semantic outputs.
A paired before/after comparison runs before-cold / after-cold / after-reused / before-reused in each
of seven repetitions, followed by h5py and direct-reader controls. Background load
and file-cache differences still limit small timing conclusions. No per-item causal
speedup can be claimed from the combined changes without an ablation measurement.

## Clarifications

Engine `group_members` validates child datasets on its first memoized call, retaining prior corruption verdicts; only vendor member_names remains lightweight. Numeric reads memoize the original error-producing kind proof and preserve error code/path/subject. Sessions with equal worker counts share throughput on the same pool, rather than obtaining independent thread capacity.
