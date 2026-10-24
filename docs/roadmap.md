# Roadmap

Planned work that is recorded but not yet started. Items here have no
committed schedule; the current release work comes first.

## Next: reading HDF files directly from cloud object storage (AWS S3)

**Status:** planned, not started (recorded 2026-10-24).

**Goal.** Read HEC-RAS geometry and plan HDF files directly from AWS S3
buckets (and other S3-compatible object stores) without downloading the
whole file first. Reads should be efficient over the network: HTTP range
requests for only the bytes a read needs, with request coalescing,
read-ahead and caching of HDF5 metadata (superblock, object headers,
B-trees, chunk indexes) so a typical read costs a small, bounded number of
round trips.

**Why it matters.** Federal efforts such as FEMA's Future of Flood Risk Data
(FFRD), and state-level programs, store and process HEC-RAS models on cloud
infrastructure. A native Rust engine that also handles cloud reads could:

- replace FFRD's `rashdf` library outright, or
- become the engine `rashdf` is rebuilt around.

This is the next performance and feature step for the engine, and the one
that makes it usable in those cloud pipelines.

**Requirements to keep from the local-file engine.**

- Same results as local reads: every equivalence suite must give identical
  values for a file read from S3 and the same file read from disk.
- Same refusal discipline: anything the engine cannot prove is refused, never
  guessed. Network errors are reported as errors, never as empty or partial
  results.
- Bounded resources: limits on request count, bytes fetched and cache size,
  in the same spirit as the existing decode and allocation budgets.

**Open questions for when this starts.**

- Object-store client and authentication (credential chain, anonymous public
  buckets, S3-compatible endpoints).
- Range-request strategy and cache design, and how they interact with the
  session pool and the stored-family index.
- How ras-commander exposes cloud paths (for example `s3://` URLs) in the
  `auto | h5py | native` modes, given that h5py needs a separate driver
  (ros3 or fsspec) for the comparison path.
- Benchmarks against `rashdf` and h5py with fsspec on the same S3 objects.
