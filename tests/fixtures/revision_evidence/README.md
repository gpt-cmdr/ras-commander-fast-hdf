# Metadata revision evidence

Frozen revision fixtures for `tests/metadata_encoding_conformance.rs`
(`revision_metadata_boundaries`). `revision-manifest.json` records each file's SHA-256, and
the test verifies those hashes before using a file.

The files are committed so that the test runs in the default CI gate.
`generate_revision.py` is the generator that produced them; it writes into the `revision/`
directory next to the script, so run a copy in a scratch directory rather than
overwriting the committed files. Regenerating with a different h5py/HDF5 version can
change the bytes, so the committed files are authoritative. The `RAS_REVISION_EVIDENCE`
environment variable overrides this directory.
