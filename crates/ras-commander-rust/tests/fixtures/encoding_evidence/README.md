# Byte-level metadata encoding evidence

Frozen byte-mutated HDF5 envelopes for `tests/metadata_encoding_conformance.rs`
(`byte_level_metadata_encoding_boundaries`) and `tests/api_harden2_types.rs`
(`attribute_types_come_from_raw_proof`). `malformed-manifest.json` records each file's
SHA-256, and the tests verify those hashes before using a file.

The files are committed so that the tests run in the default CI gate.
`make_malformed.py` is the generator that produced them
(`python -I make_malformed.py ROOT` writes `ROOT/malformed/` and
`ROOT/malformed-manifest.json`). Regenerating with a different h5py/HDF5 version can
change the bytes, so the committed files are authoritative. The `RAS_ENCODING_EVIDENCE`
environment variable overrides this directory.
